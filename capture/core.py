#!/usr/bin/env python3
"""Shared ingest-client core.

Reads new turns from a Claude Code JSONL transcript file, builds the
capture-classifier envelope, and ships it to POST /v1/ingest (fire-and-forget).

Both the hook shipper and the sidecar shipper call ship() from this module.
They behave identically — same payload schema, same cursor semantics, same
/v1/ingest call. Only the trigger differs.

Each batch names the project the session belongs to, so the decisions classified
from it are filed under that project instead of the agent's personal one: the
`.loomtracer-project` marker of the session's folder (the nearest one walking up,
`project_source: folder_marker`), else the LOOMTRACER_PROJECT setting
(`project_source: stated`), else none. The cell refuses a handle nobody
registered; the cursor stays put on any refusal, so the span is sent again once
the project exists and nothing is filed anywhere else meanwhile.

Dependencies: stdlib only (json, os, urllib.request).
"""

import json
import os
import re
import urllib.error
import urllib.request
from datetime import datetime
from typing import Optional, Tuple

# Per-turn text cap before we mark truncated=True.
_MAX_TURN_TEXT = 2000
# Maximum turns to include in a single batch.
_MAX_BATCH_TURNS = 4
# HTTP POST timeout in seconds.
_POST_TIMEOUT = 3

# The one-line file that attaches a folder (and everything beneath it, until a
# nearer one) to a project; its whole content is the project handle. The CLI reads
# the same file the same way (`loomtracer emit --project-from-context`).
PROJECT_MARKER_FILE_NAME = ".loomtracer-project"

# Claude Code mangles a project directory into its transcript folder name by
# replacing every character that isn't alphanumeric with '-' (so a cwd with
# dots or underscores, e.g. /home/ubuntu/gc/.gc/agents/mayor, does not simply
# become slash-for-dash).
_NON_ALNUM_RE = re.compile(r"[^A-Za-z0-9]")


# ---------------------------------------------------------------------------
# Public API
# ---------------------------------------------------------------------------


def ship(
    session_id: str,
    jsonl_path: str,
    api_url: str,
    api_key: str,
    agent_tool: str = "claude",
    session_dir: Optional[str] = None,
) -> None:
    """Read new turns from jsonl_path since the last cursor and POST to /v1/ingest.

    `session_dir` is the folder the session runs in (the hook's `cwd`); without
    it the folder is the one the transcript's own records name. It decides the
    project each batch names (see the module docstring).

    Always returns None. Exceptions are caught and printed to stderr so the
    caller (hook or sidecar) is never blocked.
    """
    try:
        _ship_impl(session_id, jsonl_path, api_url, api_key, agent_tool, session_dir)
    except Exception as exc:
        import sys
        print(f"loomtracer-ingest: error for {session_id}: {exc}", file=sys.stderr)


def jsonl_path_for_session(session_id: str, project_dir: Optional[str] = None) -> str:
    """Return the JSONL transcript path for a session.

    Claude Code stores transcripts at:
      ~/.claude/projects/<project-hash>/<session_id>.jsonl

    where <project-hash> is the project directory path with every non-
    alphanumeric character replaced by '-'. This is a fallback used only
    when the caller has no transcript_path from the hook's stdin payload;
    prefer that path when available since it is authoritative.
    """
    if project_dir is None:
        project_dir = os.getcwd()
    project_hash = _NON_ALNUM_RE.sub("-", project_dir)
    return os.path.expanduser(f"~/.claude/projects/{project_hash}/{session_id}.jsonl")


def cursor_path_for_session(session_id: str) -> str:
    """Return the cursor state file path for a session."""
    return os.path.expanduser(f"~/.loomtracer/cursors/{session_id}.offset")


def project_for_session(
    session_dir: Optional[str], jsonl_path: str
) -> Optional[Tuple[str, str]]:
    """The project a session's batches name: (handle, project_source), or None.

    1. the nearest `.loomtracer-project` at or above the session's folder
       -> "folder_marker";
    2. else the LOOMTRACER_PROJECT setting -> "stated";
    3. else None: the batch names no project and its decisions stay in the
       agent's personal project.

    A marker that cannot be read, or does not hold exactly one handle on one
    line, raises ValueError instead of being walked past: skipping it would file
    the session under some other project than its folder says. ship() reports
    the error and leaves the cursor, so the span is sent again once the marker is
    fixed.
    """
    folder = session_dir or _transcript_cwd(jsonl_path)
    if folder:
        handle = _nearest_marker_handle(folder)
        if handle:
            return handle, "folder_marker"
    stated = os.environ.get("LOOMTRACER_PROJECT", "").strip()
    if stated:
        return stated, "stated"
    return None


# ---------------------------------------------------------------------------
# Internal helpers
# ---------------------------------------------------------------------------


def _nearest_marker_handle(start: str) -> Optional[str]:
    """The handle in the nearest `.loomtracer-project` at or above `start`."""
    folder = os.path.abspath(os.path.expanduser(start))
    while True:
        marker = os.path.join(folder, PROJECT_MARKER_FILE_NAME)
        if os.path.isfile(marker):
            try:
                with open(marker, encoding="utf-8") as fh:
                    contents = fh.read()
            except (OSError, UnicodeDecodeError) as exc:
                raise ValueError(f"cannot read {marker}: {exc}") from exc
            handle = contents.strip()
            if not handle or any(ch.isspace() for ch in handle):
                raise ValueError(
                    f"{marker} must contain exactly one project handle on a "
                    "single line, for example `billing`"
                )
            return handle
        parent = os.path.dirname(folder)
        if parent == folder:
            return None
        folder = parent


# How many leading transcript records to look through for the session's folder.
_CWD_SCAN_LINES = 50


def _transcript_cwd(jsonl_path: str) -> Optional[str]:
    """The folder the session runs in, as the transcript's own records name it
    (every Claude Code record carries a `cwd`), None when it names none."""
    try:
        with open(jsonl_path, "rb") as fh:
            for _, raw_line in zip(range(_CWD_SCAN_LINES), fh):
                try:
                    obj = json.loads(raw_line)
                except json.JSONDecodeError:
                    continue
                cwd = obj.get("cwd") if isinstance(obj, dict) else None
                if isinstance(cwd, str) and cwd.strip():
                    return cwd.strip()
    except OSError:
        return None
    return None


def _ship_impl(
    session_id: str,
    jsonl_path: str,
    api_url: str,
    api_key: str,
    agent_tool: str,
    session_dir: Optional[str],
) -> None:
    cursor_file = cursor_path_for_session(session_id)

    # Load cursor. On first run: initialise to EOF and return (skip history).
    try:
        with open(cursor_file, encoding="utf-8") as fh:
            cursor = int(fh.read().strip())
    except (OSError, ValueError):
        _init_cursor(cursor_file, jsonl_path)
        return

    # Check if there is new content.
    try:
        file_size = os.path.getsize(jsonl_path)
    except OSError:
        return
    if file_size <= cursor:
        return

    # Read new lines from cursor forward.
    turns = []
    new_cursor = cursor
    try:
        with open(jsonl_path, "rb") as fh:
            fh.seek(cursor)
            for raw_line in fh:
                new_cursor += len(raw_line)
                try:
                    obj = json.loads(raw_line)
                except json.JSONDecodeError:
                    continue
                turn = _extract_turn(obj)
                if turn is not None:
                    turns.append(turn)
    except OSError:
        return

    if not turns:
        # Nothing worth shipping in this span (e.g. only non-turn records);
        # advance past it since there is no batch to retry.
        _write_cursor(cursor_file, new_cursor)
        return

    # Ship every turn in the span, oldest first, in chunks of at most
    # _MAX_BATCH_TURNS. Each chunk is its own batch with its own batch_id.
    # Dropping the head of the span would lose the human prompt, which is
    # the earliest turn.
    api_url = api_url.rstrip("/")
    # Worked out before anything is sent, so a broken marker stops the whole span
    # (cursor untouched) rather than shipping some of it under no project.
    project = project_for_session(session_dir, jsonl_path)
    for n, start in enumerate(range(0, len(turns), _MAX_BATCH_TURNS)):
        envelope = {
            "batch_id": f"{session_id}:{cursor}-{new_cursor}:{n}",
            "agent_tool": agent_tool,
            "session_id": session_id,
            "turns": turns[start : start + _MAX_BATCH_TURNS],
        }
        if project is not None:
            envelope["project"], envelope["project_source"] = project
        # _post() raises on HTTP errors and network failures; letting that
        # propagate (ship() catches it) stops before the cursor moves, so
        # the next run resends the span instead of silently losing it.
        _post(api_url, api_key, envelope)

    # Only advance the cursor once every chunk has been accepted.
    _write_cursor(cursor_file, new_cursor)


def _extract_turn(obj: dict) -> Optional[dict]:
    """Extract a turn dict from a JSONL record, or None if not a user/assistant turn."""
    turn_type = obj.get("type")
    if turn_type not in ("user", "assistant"):
        return None

    msg = obj.get("message")
    if not isinstance(msg, dict):
        return None

    role = msg.get("role", turn_type)
    content = msg.get("content", [])
    # Claude Code stores a typed user prompt as a plain string, not a list of
    # blocks. Iterating that string would split it into single characters.
    if isinstance(content, str):
        content = [content]
    elif not isinstance(content, list):
        return None
    uuid = obj.get("uuid", "")

    text_parts = []
    truncated = False

    for item in content:
        if isinstance(item, str):
            if item.strip():
                text_parts.append(item.strip())
        elif isinstance(item, dict):
            item_type = item.get("type", "")
            if item_type == "text":
                text = item.get("text", "").strip()
                if text:
                    text_parts.append(text)
            elif item_type == "tool_use":
                name = item.get("name", "unknown")
                text_parts.append(f"[Tool: {name}]")
                truncated = True
            elif item_type == "tool_result":
                text_parts.append("[Tool result]")
                truncated = True
            # thinking and other types are skipped

    text = " ".join(text_parts).strip()
    if not text:
        return None

    if len(text) > _MAX_TURN_TEXT:
        text = text[:_MAX_TURN_TEXT] + "…"
        truncated = True

    turn = {
        "turn_id": uuid,
        "role": role,
        "text": text,
        "truncated": truncated,
    }
    ts = _turn_timestamp(obj)
    if ts is not None:
        turn["ts"] = ts
    return turn


def _turn_timestamp(obj: dict) -> Optional[str]:
    """The record's own time in the transcript, as the string the transcript holds.

    Shipped as the turn's `ts` so the server can tell a transcript sent twice (the same turn
    times) from a decision made again later. Only a time that carries a UTC offset is sent, since
    the server refuses the whole batch for one it cannot read; anything else is left out, and the
    turn simply has no source time.
    """
    value = obj.get("timestamp")
    if not isinstance(value, str):
        return None
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        return None
    return value if parsed.tzinfo is not None else None


def _post(api_url: str, api_key: str, envelope: dict) -> None:
    """POST envelope to /v1/ingest. Raises on HTTP errors."""
    url = f"{api_url}/v1/ingest"
    body = json.dumps(envelope).encode()
    headers = {
        "Content-Type": "application/json",
        "X-Loomtracer-Actor": "agent:claude:hook",
    }
    if api_key:
        headers["Authorization"] = f"Bearer {api_key}"

    req = urllib.request.Request(url, data=body, headers=headers, method="POST")
    try:
        with urllib.request.urlopen(req, timeout=_POST_TIMEOUT) as resp:
            reply = resp.read()
    except urllib.error.HTTPError as exc:
        try:
            detail = exc.read(200).decode("utf-8", errors="replace").strip()
        except Exception:
            detail = ""
        raise RuntimeError(
            f"HTTP {exc.code} {exc.reason}" + (f": {detail}" if detail else "")
        ) from exc
    _require_project_recorded(envelope, reply)


def _require_project_recorded(envelope: dict, reply: bytes) -> None:
    """Raise unless the cell says it recorded the project the batch names.

    A cell that predates project filing accepts the batch (202) and drops the
    field, so its decisions would land in the agent's personal project with
    nothing said. A cell that records it echoes the handle back; anything else
    holds the span back (cursor untouched), and the span is sent again once the
    cell is updated, when the batch counts under its project.
    """
    project = envelope.get("project")
    if not project:
        return
    try:
        recorded = json.loads(reply).get("project")
    except (ValueError, AttributeError):
        recorded = None
    if recorded != project:
        raise RuntimeError(
            f"the cell accepted the batch without recording project {project}: "
            "it predates project filing, so the span is held back and sent "
            "again once the cell is updated"
        )


def _init_cursor(cursor_file: str, jsonl_path: str) -> None:
    """Set cursor to current EOF so we only ship future turns."""
    try:
        offset = os.path.getsize(jsonl_path)
    except OSError:
        offset = 0
    _write_cursor(cursor_file, offset)


def _write_cursor(cursor_file: str, offset: int) -> None:
    os.makedirs(os.path.dirname(cursor_file), exist_ok=True)
    with open(cursor_file, "w", encoding="utf-8") as fh:
        fh.write(str(offset))
