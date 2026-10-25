#!/usr/bin/env python3
"""Run the plugins' capture and MCP invocations against the latest RELEASE `loomtracer` binary.

The marketplace serves the plugins from master, but people install the latest release binary, so
every plugin change runs ahead of the CLI under it. A flag or verb only master has breaks a
stranger's install while every master-built test stays green (hivemind-cxqd). This is that check:

  * both MCP configs (Claude Code's `.mcp.json`, the Codex bundle's `.codex-plugin/mcp.json`) are
    started the way a client starts them, handshake, and take one grounded capture that lands
    attributed to their own tool (agent:claude / agent:codex);
  * `capture.sh` writes a decision, evidence and a hypothesis, under Claude Code's and Codex's
    environment, and `query-decisions.sh` reads them back;
  * a decision captured without saying what it binds and what undoing it costs is refused, through
    MCP and through `capture.sh`, with a message that names both questions;
  * the Codex skill's direct CLI form works as the skill says to use it on an older CLI;
  * the AskUserQuestion hooks never get in the way and still record the human's answer;
  * the SessionStart directive names only tools the release's MCP server serves;
  * the loomtracer-context scripts run, and a verb the release lacks is refused with that said.

Everything runs under a throwaway HOME, with a scrubbed environment and the release binary alone on
PATH. Python standard library only.

  scripts/check_plugin_against_release.py                 download the latest release (needs gh)
  scripts/check_plugin_against_release.py --bin PATH      use this binary as "the release"
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import threading
from pathlib import Path
from typing import Any, Dict, List, Optional

ROOT = Path(__file__).resolve().parent.parent
CAPTURE = ROOT / "plugins" / "loomtracer-capture"
CONTEXT = ROOT / "plugins" / "loomtracer-context"
HOOK_FIXTURES = ROOT / "tests" / "fixtures" / "claude_code" / "ask_user_question"

RPC_TIMEOUT_S = 60

# What each tool puts in the environment of the processes it starts. Claude Code names the project
# folder to its plugins; Codex does not, and CLAUDE_PROJECT_DIR alone makes capture.sh say claude.
SESSIONS = {
    "claude": {"CLAUDE_CODE_SESSION_ID": "check-claude-session"},
    "codex": {"CODEX_THREAD_ID": "check-codex-thread"},
}

# The decision floor (hivemind-bbnw.10): a captured decision says what it binds and what undoing it
# would cost, or the release refuses it and writes nothing. The same two answers as CLI flags (for
# capture.sh and `supersede.sh`) and as MCP objects, and the two questions every refusal names.
FLOOR_FLAGS = ["--binds", "Operators see an outage after three attempts", "--cheap-to-undo"]
FLOOR_WIRE = {
    "binds": {"level": "something", "statement": "Operators see an outage after three attempts"},
    "undo_cost": {"level": "cheap"},
}
FLOOR_QUESTIONS = ("what it binds", "what undoing it would cost")

FAILURES: List[str] = []


def names_floor_questions(refusal: str) -> bool:
    """True when a refusal says that nothing was written and names both floor questions."""
    return "nothing was written" in refusal and all(q in refusal for q in FLOOR_QUESTIONS)


def check(name: str, ok: bool, detail: str = "") -> bool:
    print(f"{'PASS' if ok else 'FAIL'}  {name}")
    if not ok:
        FAILURES.append(name)
        if detail.strip():
            for line in detail.strip().splitlines()[-12:]:
                print(f"        {line}")
    return ok


# --------------------------------------------------------------------------------------------
# The release binary
# --------------------------------------------------------------------------------------------


def release_asset() -> str:
    system, machine = platform.system(), platform.machine().lower()
    arch = "arm64" if machine in ("arm64", "aarch64") else "x86_64"
    if system == "Linux":
        return f"loomtracer-linux-{arch}.tar.gz"
    if system == "Darwin" and arch == "arm64":
        return "loomtracer-macos-arm64.tar.gz"
    raise SystemExit(f"no release binary for {system} {machine}; pass --bin")


class ReleasePredatesRename(Exception):
    """The latest release was published as `hivemind`, before the rename to Loomtracer (0.8.0)."""


def release_predates_rename(repo: Optional[str]) -> bool:
    """True when the latest release has only `hivemind-*` assets: no binary these plugins can run."""
    command = ["gh", "release", "view", "--json", "assets", "--jq", ".assets[].name"]
    if repo:
        command += ["--repo", repo]
    done = subprocess.run(command, capture_output=True, text=True, timeout=120)
    names = done.stdout.split()
    return done.returncode == 0 and any(n.startswith("hivemind-") for n in names) and not any(
        n.startswith("loomtracer-") for n in names
    )


def download_latest_release(repo: Optional[str], into: Path) -> Path:
    asset = release_asset()
    command = ["gh", "release", "download", "--pattern", f"{asset}*", "--dir", str(into)]
    if repo:
        command += ["--repo", repo]
    try:
        subprocess.run(command, check=True, timeout=300)
    except subprocess.CalledProcessError:
        if release_predates_rename(repo):
            raise ReleasePredatesRename() from None
        raise
    archive = into / asset
    expected = (into / f"{asset}.sha256").read_text().split()[0]
    actual = hashlib.sha256(archive.read_bytes()).hexdigest()
    if actual != expected:
        raise SystemExit(f"{asset}: sha256 {actual} does not match the published {expected}")
    with tarfile.open(archive) as tar:
        if hasattr(tarfile, "data_filter"):
            tar.extractall(into, filter="data")
        else:
            tar.extractall(into)
    binary = into / "loomtracer"
    binary.chmod(0o755)
    return binary


# --------------------------------------------------------------------------------------------
# A throwaway world: HOME, project folder, scrubbed environment
# --------------------------------------------------------------------------------------------


class World:
    def __init__(self, root: Path, binary: Path) -> None:
        self.root = root
        self.home = root / "home"
        self.bin = root / "bin"
        self.home.mkdir()
        self.bin.mkdir()
        (self.bin / "loomtracer").symlink_to(binary.resolve())
        self.n = 0

    def project(self, label: str) -> Path:
        """A fresh project folder with its own ledger directory: `<project>/loomtracer`."""
        self.n += 1
        folder = self.root / f"{self.n:02d}-{label}"
        folder.mkdir()
        return folder

    def env(self, project: Path, tool: Optional[str] = None, **extra: str) -> Dict[str, str]:
        """What `tool` hands a plugin process: nothing of this machine's agent identity."""
        env = {
            "HOME": str(self.home),
            "PATH": f"{self.bin}:/usr/local/bin:/usr/bin:/bin",
            "LANG": "C.UTF-8",
            "TMPDIR": str(self.root),
            "XDG_STATE_HOME": str(self.home / "state"),
        }
        if tool:
            env.update(SESSIONS[tool])
        if tool == "claude":
            env["CLAUDE_PROJECT_DIR"] = str(project)
        env.update(extra)
        return env

    def loomtracer(self, project: Path, *args: str) -> subprocess.CompletedProcess:
        return run(
            ["loomtracer", "--loomtracer-dir", str(project / "loomtracer"), "--json", *args],
            cwd=project,
            env=self.env(project),
        )

    def actors_for(self, project: Path, words: str) -> List[str]:
        """The actor ids on the decisions a search for `words` finds."""
        done = self.loomtracer(project, "query", "search_decisions", "--q", words, "--limit", "10")
        if done.returncode != 0:
            return []
        try:
            items = json.loads(done.stdout)["data"]["items"]
        except (ValueError, KeyError):
            return []
        return [actor for item in items for actor in item["graph_context"]["actor_ids"]]


def run(
    argv: List[str], cwd: Path, env: Dict[str, str], stdin: str = "", timeout: int = 120
) -> subprocess.CompletedProcess:
    return subprocess.run(
        argv,
        cwd=cwd,
        env=env,
        input=stdin,
        capture_output=True,
        text=True,
        timeout=timeout,
    )


def tail(done: subprocess.CompletedProcess) -> str:
    return f"exit {done.returncode}\n{done.stdout[-600:]}\n{done.stderr[-600:]}"


# --------------------------------------------------------------------------------------------
# MCP: start the server exactly as the plugin's config says, over stdio
# --------------------------------------------------------------------------------------------


class McpClient:
    def __init__(self, argv: List[str], cwd: Path, env: Dict[str, str]) -> None:
        self.proc = subprocess.Popen(
            argv,
            cwd=cwd,
            env=env,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        self.next_id = 0
        self.watchdog = threading.Timer(RPC_TIMEOUT_S, self.proc.kill)
        self.watchdog.start()

    def request(self, method: str, params: Dict[str, Any]) -> Dict[str, Any]:
        self.next_id += 1
        assert self.proc.stdin and self.proc.stdout
        message = {"jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params}
        self.proc.stdin.write(json.dumps(message) + "\n")
        self.proc.stdin.flush()
        line = self.proc.stdout.readline()
        if not line:
            raise RuntimeError("the server closed without answering")
        return json.loads(line)

    def close(self) -> str:
        self.watchdog.cancel()
        if self.proc.stdin:
            self.proc.stdin.close()
        try:
            self.proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.proc.kill()
        return self.proc.stderr.read() if self.proc.stderr else ""


def tool_result(response: Dict[str, Any]) -> Dict[str, Any]:
    result = response.get("result") or {}
    structured = result.get("structuredContent")
    if isinstance(structured, dict):
        return structured
    text = "".join(part.get("text", "") for part in result.get("content", []))
    try:
        parsed = json.loads(text)
    except ValueError:
        return {"error": text or str(response.get("error"))}
    return parsed if isinstance(parsed, dict) else {"error": text}


def check_mcp(world: World, label: str, config: Path, tool: str, has_floor: bool) -> None:
    server = json.loads(config.read_text())["mcpServers"]["loomtracer"]
    project = world.project(f"mcp-{label}")
    argv = [server["command"], *server["args"]]
    env = world.env(project, **server.get("env", {}))
    title = f"Serve the {label} plugin at the release it was built against"
    client = McpClient(argv, project, env)
    stderr = ""
    try:
        init = client.request(
            "initialize",
            {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "check", "version": "0"}},
        )
        connected = check(
            f"{label} MCP server connects", "serverInfo" in (init.get("result") or {}), json.dumps(init)
        )
        if not connected:
            return
        listed = client.request("tools/list", {})
        names = [t["name"] for t in (listed.get("result") or {}).get("tools", [])]
        check(f"{label} MCP server lists capture_decision", "capture_decision" in names, str(names))
        arguments: Dict[str, Any] = {
            "title": title,
            "rationale": "A plugin that runs ahead of the CLI under it breaks a stranger's install",
            "topic_keys": ["plugin-compat"],
            "options": [
                {"label": "Match the release", "description": "Pass only flags the CLI lists"},
                {"label": "Match master", "description": "Pass every flag master has"},
            ],
            "chosen_option_label": "Match the release",
            "grounding": [{"kind": "assumption", "statement": "People install the latest release binary"}],
        }
        # A release from before the floor has no `binds` / `undo_cost`; it takes the capture without.
        answered = {**arguments, **FLOOR_WIRE} if has_floor else arguments
        reply = tool_result(client.request("tools/call", {"name": "capture_decision", "arguments": answered}))
        check(
            f"{label} MCP capture_decision is accepted",
            not reply.get("error") and "decision_id" in json.dumps(reply),
            json.dumps(reply)[:600],
        )
        if has_floor:
            # What an agent that forgets the two answers is told.
            reply = tool_result(
                client.request("tools/call", {"name": "capture_decision", "arguments": arguments})
            )
            error = str(reply.get("error") or "")
            check(
                f"{label} MCP capture_decision without binds/undo answers is refused",
                names_floor_questions(error),
                json.dumps(reply)[:600],
            )
    except (RuntimeError, ValueError, OSError) as error:
        check(f"{label} MCP session", False, str(error))
    finally:
        stderr = client.close()
    actors = world.actors_for(project, "release plugin built")
    check(
        f"{label} MCP capture is attributed to agent:{tool}",
        bool(actors) and all(a.startswith(f"agent:{tool}:") for a in actors),
        f"actors={actors} stderr={stderr[-300:]}",
    )


# --------------------------------------------------------------------------------------------
# capture.sh / query-decisions.sh, under each tool's environment
# --------------------------------------------------------------------------------------------

DECISION_FLAGS = [
    "--kind", "decision",
    "--title", "Keep the retry budget at three attempts",
    "--rationale", "More retries hide an outage from the operator instead of surfacing it clearly",
    "--topic-keys", "retries",
    "--options", "Keep 3 attempts,Raise to 5 attempts",
    "--chose", "Keep 3 attempts",
    "--rests-on-assumption", "Operators read the retry dashboard",
]  # fmt: skip


def check_capture_script(world: World, label: str, tool: str, has_floor: bool) -> Path:
    project = world.project(f"capture-{label}")
    script = str(CAPTURE / "scripts" / "capture.sh")
    env = world.env(project, tool)
    # The skill and the commands teach the two answers, so they are always sent; a release CLI may
    # lack the flags, and the helper then leaves them out rather than have the CLI refuse the whole
    # capture.
    done = run(["bash", script, *DECISION_FLAGS, *FLOOR_FLAGS], cwd=project, env=env)
    check(
        f"capture.sh decision ({label})",
        done.returncode == 0 and "Captured Loomtracer decision" in done.stdout and "unexpected argument" not in done.stderr,
        tail(done),
    )
    if has_floor:
        # What an agent that forgets the two answers is told, and that the release wrote nothing.
        bare = world.project(f"bare-{label}")
        done = run(["bash", script, *DECISION_FLAGS], cwd=bare, env=world.env(bare, tool))
        check(
            f"capture.sh decision without binds/undo answers is refused ({label})",
            done.returncode != 0
            and names_floor_questions(done.stderr)
            and not world.actors_for(bare, "retry budget attempts"),
            tail(done),
        )
    done = run(["bash", script, "Retry dashboards were green all week", "--kind", "evidence"], cwd=project, env=env)
    check(f"capture.sh evidence ({label})", done.returncode == 0 and "Captured Loomtracer evidence" in done.stdout, tail(done))
    done = run(["bash", script, "Operators check the dashboard daily", "--kind", "hypothesis"], cwd=project, env=env)
    check(f"capture.sh hypothesis ({label})", done.returncode == 0 and "Captured Loomtracer hypothesis" in done.stdout, tail(done))
    actors = world.actors_for(project, "retry budget attempts")
    check(
        f"capture.sh decision ({label}) is attributed to agent:{tool}",
        bool(actors) and all(a.startswith(f"agent:{tool}:") for a in actors),
        f"actors={actors}",
    )
    done = run(["bash", str(CAPTURE / "scripts" / "query-decisions.sh"), "retry budget"], cwd=project, env=env)
    check(f"query-decisions.sh finds it ({label})", done.returncode == 0 and "retry budget" in done.stdout.lower(), tail(done))
    return project


# --------------------------------------------------------------------------------------------
# The Codex skill's direct CLI form, used the way the skill says to on an older CLI
# --------------------------------------------------------------------------------------------


def skill_direct_form() -> str:
    skill = (CAPTURE / "skills" / "loomtracer-capture" / "SKILL.md").read_text()
    blocks = re.findall(r"```bash\n(.*?)```", skill, flags=re.S)
    form = next(b for b in blocks if "emit decision.capture" in b and "--project-from-context" in b)
    return "\n".join(line[3:] if line.startswith("   ") else line for line in form.splitlines()) + "\n"


# Flags master's CLI has and a release may lack. The skill tells an agent that meets "unexpected
# argument '<flag>'" for one of these to re-run without it, the four answers going together, so
# the direct-form check does the same (hivemind-bbnw.12).
NEWER_FLAG_GROUPS = [
    ("--project-from-context",),
    ("--binds", "--binds-nothing", "--undo-cost", "--cheap-to-undo"),
]


def without_flags(form: str, flags: Any) -> str:
    """The direct form with the lines that start with one of `flags` removed."""
    lines = [l for l in form.splitlines() if (l.split() or [""])[0] not in flags]
    if lines and lines[-1].rstrip().endswith("\\"):
        lines[-1] = lines[-1].rstrip()[:-1].rstrip()  # the last line no longer continues
    return "\n".join(lines) + "\n"


def check_skill_direct_form(world: World, label: str, tool: str) -> None:
    project = world.project(f"skill-{label}")
    form = skill_direct_form()
    env = world.env(project, tool, LOOMTRACER_DIR=str(project / "loomtracer"))
    done = run(["bash", "-c", form], cwd=project, env=env)
    for _ in NEWER_FLAG_GROUPS:
        group = next(
            (
                g
                for g in NEWER_FLAG_GROUPS
                if done.returncode != 0 and any(f"unexpected argument '{f}'" in done.stderr for f in g)
            ),
            None,
        )
        if group is None:
            break
        form = without_flags(form, group)
        done = run(["bash", "-c", form], cwd=project, env=env)
    check(f"skill direct form captures ({label})", done.returncode == 0, tail(done))
    actors = world.actors_for(project, "Prefer direct CLI capture before MCP")
    check(
        f"skill direct form ({label}) is attributed to agent:{tool}",
        bool(actors) and all(a.startswith(f"agent:{tool}:") for a in actors),
        f"actors={actors}",
    )


# --------------------------------------------------------------------------------------------
# The AskUserQuestion hooks
# --------------------------------------------------------------------------------------------


def check_ask_hooks(world: World) -> None:
    project = world.project("ask-hooks")
    state = world.root / "hook-state"
    env = world.env(
        project,
        "claude",
        LOOMTRACER_DIR=str(project / "loomtracer"),
        LOOMTRACER_HUMAN_ACTOR="human:alice",
        CLAUDE_PLUGIN_DATA=str(state),
    )
    hook = str(CAPTURE / "scripts" / "ask-hook.sh")
    for phase in ("pre", "post"):
        payload = json.loads((HOOK_FIXTURES / f"{phase}.json").read_text())
        payload["cwd"] = str(project)
        done = run(["sh", hook, phase], cwd=project, env=env, stdin=json.dumps(payload))
        check(
            f"ask hook {phase} stays out of the agent's way",
            done.returncode == 0 and not done.stdout,
            tail(done),
        )
    log = state / "ask-hook" / "hook.log"
    text = log.read_text() if log.exists() else ""
    check(
        "ask hooks never start the CLI with a flag it lacks",
        "unexpected argument" not in text and "exited without answering" not in text,
        text,
    )
    actors = world.actors_for(project, "Postgres hosted")
    check("ask hook post records the human's answer", bool(actors), f"actors={actors}\n{text}")


# --------------------------------------------------------------------------------------------
# The SessionStart directive
# --------------------------------------------------------------------------------------------


def check_session_start_hook(world: World) -> None:
    """The directive names tools the release serves, and the opt-out makes it silent."""
    project = world.project("session-start")
    hook = str(CAPTURE / "scripts" / "session-start-hook.sh")
    done = run(["sh", hook], cwd=project, env=world.env(project, "claude"))
    try:
        context = json.loads(done.stdout)["hookSpecificOutput"]["additionalContext"]
    except (ValueError, KeyError, TypeError):
        context = ""
    check("session start hook adds the directive", done.returncode == 0 and bool(context), tail(done))
    named = [name for name in re.findall(r"`([a-z_]+)`", context) if name != "loomtracer"]

    server = json.loads((CAPTURE / ".mcp.json").read_text())["mcpServers"]["loomtracer"]
    client = McpClient([server["command"], *server["args"]], project, world.env(project, **server.get("env", {})))
    try:
        client.request(
            "initialize",
            {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "check", "version": "0"}},
        )
        listed = client.request("tools/list", {})
        served = [t["name"] for t in (listed.get("result") or {}).get("tools", [])]
    finally:
        client.close()
    missing = [name for name in named if name not in served]
    check(
        "session start directive names only tools the release serves",
        bool(named) and not missing,
        f"named={named} missing={missing}",
    )

    done = run(["sh", hook], cwd=project, env=world.env(project, "claude", LOOMTRACER_DIRECTIVE_DISABLE="1"))
    check(
        "session start hook is silent when disabled",
        done.returncode == 0 and not done.stdout and not done.stderr,
        tail(done),
    )


# --------------------------------------------------------------------------------------------
# loomtracer-context
# --------------------------------------------------------------------------------------------


def check_context_scripts(
    world: World,
    project: Path,
    binary_has_ground: bool,
    binary_has_importance: bool,
    binary_has_reaffirm: bool,
    binary_has_reexamine: bool,
    binary_has_floor: bool,
) -> None:
    scripts = CONTEXT / "scripts"

    def verb(name: str, *args: str, **extra_env: str) -> subprocess.CompletedProcess:
        env = world.env(project, "claude", **extra_env)
        return run(["bash", str(scripts / f"{name}.sh"), *args], cwd=project, env=env)

    for name, args in [
        ("recall", ("retry budget",)),
        ("why", ("retry budget",)),
        ("verify", ("retry budget",)),
        ("situational", ("--paths", "src/retry.rs")),
    ]:
        done = verb(name, *args)
        check(f"loomtracer-context {name}", done.returncode == 0, tail(done))
    # An actor cannot accept and reject the same decision, so someone else disagrees.
    done = verb(
        "disagree", "retry budget", "--reason", "Three attempts is too few for a flaky upstream",
        GC_AGENT="check-reviewer",
    )  # fmt: skip
    check("loomtracer-context disagree", done.returncode == 0, tail(done))
    done = verb(
        "supersede", "retry budget",
        "--title", "Raise the retry budget to five attempts",
        "--rationale", "The flaky upstream needs more attempts than three to succeed reliably",
        "--topic-keys", "retries", "--options", "Keep 3 attempts,Raise to 5 attempts",
        "--chose", "Raise to 5 attempts", "--rests-on-assumption", "The upstream stays flaky",
        *(FLOOR_FLAGS if binary_has_floor else []),
    )  # fmt: skip
    check("loomtracer-context supersede", done.returncode == 0, tail(done))
    done = verb("importance")
    if binary_has_importance:
        check("loomtracer-context importance", done.returncode == 0, tail(done))
    else:
        check(
            "loomtracer-context importance names the verb the release lacks",
            done.returncode == 2 and "has no `query rank_decisions_by_importance` command" in done.stderr,
            tail(done),
        )
    done = verb("ground", "retry budget", "--rests-on-assumption", "The upstream stays flaky")
    if binary_has_ground:
        check("loomtracer-context ground", done.returncode == 0, tail(done))
    else:
        check(
            "loomtracer-context ground names the verb the release lacks",
            done.returncode == 2 and "has no `ground` command" in done.stderr,
            tail(done),
        )
    done = verb("reexamine")
    if binary_has_reexamine:
        check("loomtracer-context reexamine", done.returncode == 0, tail(done))
    else:
        check(
            "loomtracer-context reexamine names the verb the release lacks",
            done.returncode == 2 and "has no `reexamine` command" in done.stderr,
            tail(done),
        )
    # By now the decision has been disagreed with and superseded, so a release that has the verb may
    # list candidates (exit 0) or refuse for lack of anything to re-examine (exit 2): either is the
    # verb answering. What it must not do is not know the verb without saying so.
    done = verb("reaffirm", "retry budget", "--because", "Checked against the release")
    if binary_has_reaffirm:
        check(
            "loomtracer-context reaffirm",
            done.returncode in (0, 2) and "has no `reaffirm` command" not in done.stderr,
            tail(done),
        )
    else:
        check(
            "loomtracer-context reaffirm names the verb the release lacks",
            done.returncode == 2 and "has no `reaffirm` command" in done.stderr,
            tail(done),
        )


# --------------------------------------------------------------------------------------------


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    parser.add_argument("--bin", type=Path, help="the loomtracer binary to treat as the latest release")
    parser.add_argument("--repo", help="owner/name to download the latest release from (default: gh's)")
    args = parser.parse_args()

    with tempfile.TemporaryDirectory(prefix="loomtracer-plugin-release-") as tmp:
        root = Path(tmp)
        (root / "release").mkdir()
        (root / "world").mkdir()
        try:
            binary = args.bin or download_latest_release(args.repo, root / "release")
        except ReleasePredatesRename:
            # Only until the first Loomtracer release (0.8.0) is published: the plugins now start
            # `loomtracer`, and the latest release ships `hivemind`, so there is nothing to run.
            print("SKIP  the latest release predates the rename to Loomtracer (it ships hivemind-* assets).")
            print("      Nothing was checked; this runs for real once a loomtracer-* release exists.")
            return 0
        version = subprocess.run([str(binary), "--version"], capture_output=True, text=True).stdout.strip()
        print(f"release binary: {version}")
        world = World(root / "world", binary)
        has_ground = subprocess.run([str(binary), "ground", "--help"], capture_output=True).returncode == 0
        has_importance = (
            subprocess.run([str(binary), "query", "rank_decisions_by_importance", "--help"], capture_output=True).returncode == 0
        )

        has_floor = (
            "--binds-nothing"
            in subprocess.run(
                [str(binary), "emit", "decision.capture", "--help"], capture_output=True, text=True
            ).stdout
        )

        check_mcp(world, "Claude Code", CAPTURE / ".mcp.json", "claude", has_floor)
        check_mcp(world, "Codex", CAPTURE / ".codex-plugin" / "mcp.json", "codex", has_floor)
        claude_project = check_capture_script(world, "Claude Code", "claude", has_floor)
        check_capture_script(world, "Codex", "codex", has_floor)
        check_skill_direct_form(world, "Claude Code", "claude")
        check_skill_direct_form(world, "Codex", "codex")
        check_ask_hooks(world)
        check_session_start_hook(world)
        has_reaffirm = subprocess.run([str(binary), "reaffirm", "--help"], capture_output=True).returncode == 0
        has_reexamine = subprocess.run([str(binary), "reexamine", "--help"], capture_output=True).returncode == 0
        check_context_scripts(
            world, claude_project, has_ground, has_importance, has_reaffirm, has_reexamine, has_floor
        )

    print()
    if FAILURES:
        print(f"{len(FAILURES)} check(s) failed against {version}:")
        for name in FAILURES:
            print(f"  - {name}")
        return 1
    print(f"all checks passed against {version}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
