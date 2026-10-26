#!/usr/bin/env python3
"""Unit tests for capture/core.py."""

import json
import os
import sys
import tempfile
import unittest
import urllib.error
from unittest import mock

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from core import (
    _extract_turn,
    _require_cell_files_projects,
    _write_cursor,
    cursor_path_for_session,
    jsonl_path_for_session,
    project_for_session,
    ship,
)


class TestJsonlPathForSession(unittest.TestCase):
    def test_derives_path_from_project_dir(self):
        path = jsonl_path_for_session("session-abc", "/data/projects/hivemind")
        home = os.path.expanduser("~")
        self.assertEqual(
            path,
            f"{home}/.claude/projects/-data-projects-hivemind/session-abc.jsonl",
        )

    def test_uses_cwd_when_project_dir_omitted(self):
        original_cwd = os.getcwd()
        try:
            os.chdir(tempfile.gettempdir())
            path = jsonl_path_for_session("s1")
            self.assertIn("s1.jsonl", path)
        finally:
            os.chdir(original_cwd)

    def test_mangles_every_non_alnum_character_not_just_slash(self):
        """Claude Code's real mangling replaces dots/underscores too, not just
        '/'. A naive '/' -> '-' replace misses cwds like gc's own agent
        worktrees (/home/ubuntu/gc/.gc/agents/mayor)."""
        path = jsonl_path_for_session(
            "session-abc", "/home/ubuntu/gc/.gc/agents/mayor"
        )
        home = os.path.expanduser("~")
        self.assertEqual(
            path,
            f"{home}/.claude/projects/-home-ubuntu-gc--gc-agents-mayor/session-abc.jsonl",
        )


class TestExtractTurn(unittest.TestCase):
    def _make_record(self, type_, role, content):
        return {
            "type": type_,
            "uuid": "uuid-1",
            "message": {"role": role, "content": content},
        }

    def test_extracts_user_text_turn(self):
        record = self._make_record(
            "user", "user", [{"type": "text", "text": "Should we use REST or RPC?"}]
        )
        turn = _extract_turn(record)
        self.assertIsNotNone(turn)
        self.assertEqual(turn["role"], "user")
        self.assertEqual(turn["text"], "Should we use REST or RPC?")
        self.assertFalse(turn["truncated"])

    def test_extracts_assistant_text_turn(self):
        record = self._make_record(
            "assistant",
            "assistant",
            [{"type": "text", "text": "REST is better for ergonomics."}],
        )
        turn = _extract_turn(record)
        self.assertIsNotNone(turn)
        self.assertEqual(turn["role"], "assistant")
        self.assertEqual(turn["text"], "REST is better for ergonomics.")

    def test_ships_the_records_own_time_as_the_turns_ts(self):
        record = self._make_record("user", "user", [{"type": "text", "text": "Go with REST."}])
        record["timestamp"] = "2026-09-27T14:03:11.123Z"
        self.assertEqual(_extract_turn(record)["ts"], "2026-09-27T14:03:11.123Z")

    def test_a_record_without_a_readable_time_ships_no_ts(self):
        for timestamp in (None, "", "yesterday", "2026-09-27T14:03:11", 1790000000):
            record = self._make_record(
                "user", "user", [{"type": "text", "text": "Go with REST."}]
            )
            if timestamp is not None:
                record["timestamp"] = timestamp
            self.assertNotIn("ts", _extract_turn(record), msg=repr(timestamp))

    def test_skips_non_user_assistant_types(self):
        record = {"type": "file-history-snapshot", "uuid": "u", "message": {}}
        self.assertIsNone(_extract_turn(record))

    def test_summarises_tool_use(self):
        record = self._make_record(
            "assistant",
            "assistant",
            [
                {"type": "text", "text": "Running a check."},
                {"type": "tool_use", "name": "Bash", "id": "t1", "input": {}},
            ],
        )
        turn = _extract_turn(record)
        self.assertIsNotNone(turn)
        self.assertIn("[Tool: Bash]", turn["text"])
        self.assertTrue(turn["truncated"])

    def test_skips_thinking_blocks(self):
        record = self._make_record(
            "assistant",
            "assistant",
            [
                {"type": "thinking", "thinking": "private reasoning"},
                {"type": "text", "text": "My answer."},
            ],
        )
        turn = _extract_turn(record)
        self.assertIsNotNone(turn)
        self.assertNotIn("private reasoning", turn["text"])
        self.assertEqual(turn["text"], "My answer.")

    def test_truncates_long_text(self):
        long_text = "x" * 3000
        record = self._make_record(
            "user", "user", [{"type": "text", "text": long_text}]
        )
        turn = _extract_turn(record)
        self.assertTrue(turn["truncated"])
        self.assertLessEqual(len(turn["text"]), 2010)

    def test_returns_none_for_empty_text(self):
        record = self._make_record(
            "assistant", "assistant", [{"type": "thinking", "thinking": "nope"}]
        )
        self.assertIsNone(_extract_turn(record))

    def test_handles_plain_string_content(self):
        record = self._make_record("user", "user", ["Hello from a plain string"])
        turn = _extract_turn(record)
        self.assertIsNotNone(turn)
        self.assertIn("Hello from a plain string", turn["text"])

    def test_typed_prompt_str_content_extracts_to_exact_text(self):
        """Claude Code stores a typed prompt as message.content = str."""
        prompt = "Go with option B, the shared Postgres cell."
        turn = _extract_turn(self._make_record("user", "user", prompt))
        self.assertIsNotNone(turn)
        self.assertEqual(turn["text"], prompt)
        self.assertEqual(turn["role"], "user")
        self.assertFalse(turn["truncated"])

    def test_str_content_keeps_multiline_prompt_intact(self):
        prompt = "Decision:\n- use Postgres\n- drop SQLite"
        turn = _extract_turn(self._make_record("user", "user", prompt))
        self.assertEqual(turn["text"], prompt)

    def test_blank_str_content_is_not_a_turn(self):
        self.assertIsNone(_extract_turn(self._make_record("user", "user", "   ")))

    def test_non_str_non_list_content_is_skipped_not_raised(self):
        """A malformed record must not raise: an exception in extraction would
        wedge the cursor on that line forever."""
        self.assertIsNone(_extract_turn(self._make_record("user", "user", None)))


class TestCursorFlow(unittest.TestCase):
    def _write_jsonl(self, path, records):
        with open(path, "w", encoding="utf-8") as fh:
            for record in records:
                fh.write(json.dumps(record) + "\n")

    def _append_jsonl(self, path, records):
        with open(path, "a", encoding="utf-8") as fh:
            for record in records:
                fh.write(json.dumps(record) + "\n")

    def setUp(self):
        self.tmpdir = tempfile.mkdtemp()
        self.session_id = "test-session-cursor"
        self.jsonl_path = os.path.join(self.tmpdir, f"{self.session_id}.jsonl")
        self.cursor_dir = os.path.join(self.tmpdir, "cursors")

        # Patch cursor_path to use a temp directory.
        import core as core_module
        self._orig_cursor_path = core_module.cursor_path_for_session

        def patched_cursor_path(sid):
            return os.path.join(self.cursor_dir, f"{sid}.offset")

        core_module.cursor_path_for_session = patched_cursor_path

    def tearDown(self):
        import core as core_module
        core_module.cursor_path_for_session = self._orig_cursor_path
        import shutil
        shutil.rmtree(self.tmpdir, ignore_errors=True)

    def test_first_run_sets_cursor_to_eof_and_ships_nothing(self):
        self._write_jsonl(
            self.jsonl_path,
            [
                {"type": "user", "uuid": "u1", "message": {"role": "user", "content": [{"type": "text", "text": "old turn"}]}}
            ],
        )

        posted = []

        import core as core_module
        orig_post = core_module._post
        core_module._post = lambda url, key, env: posted.append(env)

        try:
            ship(self.session_id, self.jsonl_path, "http://localhost:8080", "")
            self.assertEqual(posted, [], "first run must not ship existing content")

            # Cursor file should exist now.
            cursor_file = os.path.join(self.cursor_dir, f"{self.session_id}.offset")
            self.assertTrue(os.path.exists(cursor_file))
            with open(cursor_file, encoding="utf-8") as fh:
                offset = int(fh.read())
            file_size = os.path.getsize(self.jsonl_path)
            self.assertEqual(offset, file_size)
        finally:
            core_module._post = orig_post

    def test_second_run_ships_only_new_turns(self):
        # Write initial content and init cursor.
        self._write_jsonl(
            self.jsonl_path,
            [
                {"type": "user", "uuid": "old", "message": {"role": "user", "content": [{"type": "text", "text": "old"}]}}
            ],
        )

        import core as core_module
        posted = []
        orig_post = core_module._post
        core_module._post = lambda url, key, env: posted.append(env)

        try:
            # First call: init cursor to EOF.
            ship(self.session_id, self.jsonl_path, "http://localhost:8080", "")
            self.assertEqual(posted, [])

            # Append a new turn.
            self._append_jsonl(
                self.jsonl_path,
                [
                    {"type": "assistant", "uuid": "new1", "message": {"role": "assistant", "content": [{"type": "text", "text": "new answer"}]}}
                ],
            )

            # Second call: should ship only the new turn.
            ship(self.session_id, self.jsonl_path, "http://localhost:8080", "")
            self.assertEqual(len(posted), 1)
            self.assertEqual(len(posted[0]["turns"]), 1)
            self.assertEqual(posted[0]["turns"][0]["turn_id"], "new1")
            self.assertEqual(posted[0]["session_id"], self.session_id)
        finally:
            core_module._post = orig_post

    def _user_turns(self, count):
        return [
            {"type": "user", "uuid": f"new{i}", "message": {"role": "user", "content": [{"type": "text", "text": f"new turn {i}"}]}}
            for i in range(count)
        ]

    def _cursor_offset(self):
        with open(os.path.join(self.cursor_dir, f"{self.session_id}.offset")) as fh:
            return int(fh.read())

    def _init_cursor_at_eof(self):
        """Write one old record and run ship() so the cursor sits at EOF."""
        import core as core_module
        self._write_jsonl(
            self.jsonl_path,
            [{"type": "user", "uuid": "old0", "message": {"role": "user", "content": [{"type": "text", "text": "old"}]}}],
        )
        orig_post = core_module._post
        core_module._post = lambda url, key, env: None
        try:
            ship(self.session_id, self.jsonl_path, "http://localhost:8080", "")
        finally:
            core_module._post = orig_post

    def test_span_larger_than_max_batch_ships_every_turn_in_chunks(self):
        """A span of more than _MAX_BATCH_TURNS turns ships all of them, oldest
        first, split across batches with distinct ids; the cursor ends at EOF."""
        import core as core_module
        self._init_cursor_at_eof()
        posted = []
        orig_post = core_module._post
        core_module._post = lambda url, key, env: posted.append(env)

        try:
            self._append_jsonl(self.jsonl_path, self._user_turns(10))

            ship(self.session_id, self.jsonl_path, "http://localhost:8080", "")

            self.assertEqual([len(env["turns"]) for env in posted], [4, 4, 2])
            shipped_ids = [t["turn_id"] for env in posted for t in env["turns"]]
            self.assertEqual(shipped_ids, [f"new{i}" for i in range(10)])
            batch_ids = [env["batch_id"] for env in posted]
            self.assertEqual(len(set(batch_ids)), 3, "each chunk needs its own batch_id")
            self.assertEqual(self._cursor_offset(), os.path.getsize(self.jsonl_path))
        finally:
            core_module._post = orig_post

    def test_failed_post_mid_span_leaves_cursor_and_resends_whole_span(self):
        """If a later chunk fails, the cursor stays put so the next run resends
        every turn of the span rather than only its tail."""
        import core as core_module
        self._init_cursor_at_eof()
        offset_before = self._cursor_offset()
        self._append_jsonl(self.jsonl_path, self._user_turns(10))
        orig_post = core_module._post

        posted = []

        def _fail_on_second_chunk(url, key, env):
            if posted:
                raise OSError("network down")
            posted.append(env)

        core_module._post = _fail_on_second_chunk
        try:
            # ship() must swallow the error.
            ship(self.session_id, self.jsonl_path, "http://localhost:8080", "")
            self.assertEqual(len(posted), 1, "first chunk posted before the failure")
            self.assertEqual(self._cursor_offset(), offset_before)

            # Recovery: the retry ships all 10 turns and then advances.
            retried = []
            core_module._post = lambda url, key, env: retried.append(env)
            ship(self.session_id, self.jsonl_path, "http://localhost:8080", "")
            shipped_ids = [t["turn_id"] for env in retried for t in env["turns"]]
            self.assertEqual(shipped_ids, [f"new{i}" for i in range(10)])
            self.assertEqual(self._cursor_offset(), os.path.getsize(self.jsonl_path))
        finally:
            core_module._post = orig_post

    def test_typed_prompt_string_content_ships_verbatim(self):
        """A typed user prompt (content is a str) reaches the ingest payload as
        the exact original text, not one character per token."""
        import core as core_module
        self._init_cursor_at_eof()
        posted = []
        orig_post = core_module._post
        core_module._post = lambda url, key, env: posted.append(env)

        try:
            prompt = "Go with option B, the shared Postgres cell."
            self._append_jsonl(
                self.jsonl_path,
                [{"type": "user", "uuid": "u1", "message": {"role": "user", "content": prompt}}],
            )

            ship(self.session_id, self.jsonl_path, "http://localhost:8080", "")

            self.assertEqual(len(posted), 1)
            self.assertEqual(posted[0]["turns"][0]["text"], prompt)
        finally:
            core_module._post = orig_post

    def test_cursor_does_not_advance_when_post_fails(self):
        """A failed POST must not lose the batch: the cursor stays put so the
        next run resends the same turns instead of silently dropping them."""
        self._write_jsonl(
            self.jsonl_path,
            [
                {"type": "user", "uuid": "u0", "message": {"role": "user", "content": [{"type": "text", "text": "turn zero"}]}}
            ],
        )

        import core as core_module

        # Init cursor.
        orig_post = core_module._post
        core_module._post = lambda url, key, env: None
        ship(self.session_id, self.jsonl_path, "http://localhost:8080", "")

        # Append new turn.
        self._append_jsonl(
            self.jsonl_path,
            [{"type": "user", "uuid": "u1", "message": {"role": "user", "content": [{"type": "text", "text": "new"}]}}],
        )

        # Make POST raise.
        def _fail(*_args, **_kwargs):
            raise OSError("network down")

        core_module._post = _fail

        cursor_file = os.path.join(self.cursor_dir, f"{self.session_id}.offset")
        with open(cursor_file) as fh:
            offset_before = int(fh.read())

        # ship() should not raise; it prints to stderr.
        ship(self.session_id, self.jsonl_path, "http://localhost:8080", "")

        with open(cursor_file) as fh:
            offset_after = int(fh.read())
        self.assertEqual(offset_after, offset_before, "cursor must not advance when POST fails")

        # Next run resends: same new turn arrives again, not skipped.
        core_module._post = orig_post
        posted = []
        core_module._post = lambda url, key, env: posted.append(env)
        ship(self.session_id, self.jsonl_path, "http://localhost:8080", "")
        self.assertEqual(len(posted), 1)
        self.assertEqual(posted[0]["turns"][0]["turn_id"], "u1")

        core_module._post = orig_post


class TestProjectOfBatches(unittest.TestCase):
    """Each batch names the project of the session, so the decisions classified
    from it are filed under that project and not the agent's personal one."""

    def setUp(self):
        self.tmpdir = tempfile.mkdtemp()
        self.session_id = "test-session-project"
        self.jsonl_path = os.path.join(self.tmpdir, f"{self.session_id}.jsonl")
        self.cursor_dir = os.path.join(self.tmpdir, "cursors")
        # The folder the session runs in, with a nested one beneath it.
        self.repo = os.path.join(self.tmpdir, "repo")
        self.nested = os.path.join(self.repo, "services", "billing")
        os.makedirs(self.nested)

        import core as core_module
        self._orig_cursor_path = core_module.cursor_path_for_session
        core_module.cursor_path_for_session = lambda sid: os.path.join(
            self.cursor_dir, f"{sid}.offset"
        )
        # No cell here: the one that is asked whether it files projects says yes.
        self._orig_probe = core_module._require_cell_files_projects
        self.asked_cell = []
        core_module._require_cell_files_projects = self.asked_cell.append
        self._orig_env = os.environ.pop("LOOMTRACER_PROJECT", None)

    def tearDown(self):
        import core as core_module
        core_module.cursor_path_for_session = self._orig_cursor_path
        core_module._require_cell_files_projects = self._orig_probe
        os.environ.pop("LOOMTRACER_PROJECT", None)
        if self._orig_env is not None:
            os.environ["LOOMTRACER_PROJECT"] = self._orig_env
        import shutil
        shutil.rmtree(self.tmpdir, ignore_errors=True)

    def _marker(self, folder, text):
        with open(os.path.join(folder, ".loomtracer-project"), "w", encoding="utf-8") as fh:
            fh.write(text)

    def _record(self, uuid, **extra):
        record = {
            "type": "user",
            "uuid": uuid,
            "message": {"role": "user", "content": [{"type": "text", "text": f"turn {uuid}"}]},
        }
        record.update(extra)
        return record

    def _write_jsonl(self, records, mode="w"):
        with open(self.jsonl_path, mode, encoding="utf-8") as fh:
            for record in records:
                fh.write(json.dumps(record) + "\n")

    def _ship_new_turn(self, session_dir):
        """Run ship() past the first-run cursor init, with one new turn, and
        return the envelopes it posted (a refused span leaves the list empty)."""
        import core as core_module
        self._write_jsonl([self._record("old0")])
        posted = []
        orig_post = core_module._post
        core_module._post = lambda url, key, env: posted.append(env)
        try:
            ship(self.session_id, self.jsonl_path, "http://localhost:8080", "")
            self._write_jsonl([self._record("new0")], mode="a")
            ship(
                self.session_id,
                self.jsonl_path,
                "http://localhost:8080",
                "",
                session_dir=session_dir,
            )
        finally:
            core_module._post = orig_post
        return posted

    def _cursor_offset(self):
        with open(os.path.join(self.cursor_dir, f"{self.session_id}.offset")) as fh:
            return int(fh.read())

    def test_the_marker_of_the_session_folder_names_the_project(self):
        self._marker(self.repo, "loomtracer\n")
        posted = self._ship_new_turn(self.repo)
        self.assertEqual(len(posted), 1)
        self.assertEqual(posted[0]["project"], "loomtracer")
        self.assertEqual(posted[0]["project_source"], "folder_marker")

    def test_the_nearest_marker_above_the_folder_wins(self):
        self._marker(self.repo, "platform")
        self._marker(os.path.join(self.repo, "services"), "billing")
        self.assertEqual(
            project_for_session(self.nested, self.jsonl_path), ("billing", "folder_marker")
        )
        self.assertEqual(
            project_for_session(self.repo, self.jsonl_path), ("platform", "folder_marker")
        )

    def test_the_setting_names_the_project_when_no_marker_is_above(self):
        os.environ["LOOMTRACER_PROJECT"] = " hivemind-company "
        posted = self._ship_new_turn(self.repo)
        self.assertEqual(posted[0]["project"], "hivemind-company")
        self.assertEqual(posted[0]["project_source"], "stated")

    def test_the_marker_wins_over_the_setting(self):
        self._marker(self.repo, "loomtracer")
        os.environ["LOOMTRACER_PROJECT"] = "hivemind-company"
        self.assertEqual(
            project_for_session(self.repo, self.jsonl_path), ("loomtracer", "folder_marker")
        )

    def test_neither_marker_nor_setting_names_no_project(self):
        posted = self._ship_new_turn(self.repo)
        self.assertEqual(len(posted), 1)
        self.assertNotIn("project", posted[0])
        self.assertNotIn("project_source", posted[0])

    def test_without_a_session_folder_the_transcripts_own_cwd_is_used(self):
        """The sidecar has no hook payload: the records name the folder."""
        self._marker(self.repo, "loomtracer")
        self._write_jsonl([self._record("old0", cwd=self.nested)])
        self.assertEqual(
            project_for_session(None, self.jsonl_path), ("loomtracer", "folder_marker")
        )

    def test_a_broken_marker_ships_nothing_and_leaves_the_cursor(self):
        """A marker that is not one handle on one line is never walked past: the
        span is held back (cursor untouched) until the marker is fixed."""
        self._marker(self.repo, "two handles\n")
        posted = self._ship_new_turn(self.repo)
        self.assertEqual(posted, [])
        # The cursor sits where the first run left it, before the new turn.
        self.assertLess(self._cursor_offset(), os.path.getsize(self.jsonl_path))

        self._marker(self.repo, "loomtracer")
        import core as core_module
        orig_post = core_module._post
        core_module._post = lambda url, key, env: posted.append(env)
        try:
            ship(
                self.session_id,
                self.jsonl_path,
                "http://localhost:8080",
                "",
                session_dir=self.repo,
            )
        finally:
            core_module._post = orig_post
        self.assertEqual([env["project"] for env in posted], ["loomtracer"])
        self.assertEqual(self._cursor_offset(), os.path.getsize(self.jsonl_path))

    def test_the_cell_is_asked_once_before_a_span_that_names_a_project(self):
        self._marker(self.repo, "loomtracer")
        posted = self._ship_new_turn(self.repo)
        self.assertEqual(len(posted), 1)
        self.assertEqual(self.asked_cell, ["http://localhost:8080"])

    def test_a_span_naming_no_project_does_not_ask_the_cell(self):
        posted = self._ship_new_turn(self.repo)
        self.assertEqual(len(posted), 1)
        self.assertEqual(self.asked_cell, [])

    def test_a_cell_that_does_not_file_projects_is_sent_nothing(self):
        """The cell is asked before the first POST: a cell that predates project
        filing records the batch and drops the project, so nothing is sent to it and
        the cursor stays where the span begins (hivemind-8fbto)."""
        import core as core_module

        def refuse(api_url):
            raise RuntimeError("the cell does not file batches under a project")

        core_module._require_cell_files_projects = refuse
        self._marker(self.repo, "loomtracer")
        posted = self._ship_new_turn(self.repo)
        self.assertEqual(posted, [])
        self.assertLess(self._cursor_offset(), os.path.getsize(self.jsonl_path))

    def test_an_empty_marker_is_refused_too(self):
        self._marker(self.repo, "  \n")
        with self.assertRaises(ValueError):
            project_for_session(self.repo, self.jsonl_path)


def _version_reply(body):
    """What urlopen returns for GET /v1/version: a context manager over the reply."""
    reply = mock.MagicMock()
    reply.__enter__.return_value.read.return_value = body
    return reply


class TestCellFilesProjects(unittest.TestCase):
    """The cell is asked, on /v1/version, whether it files a batch under its project."""

    def test_a_cell_that_says_it_files_projects_is_accepted(self):
        with mock.patch(
            "urllib.request.urlopen",
            return_value=_version_reply(b'{"sha": "abc", "ingest_project": true}'),
        ) as urlopen:
            _require_cell_files_projects("http://cell.example:8080")
        self.assertEqual(
            urlopen.call_args.args[0], "http://cell.example:8080/v1/version"
        )

    def test_a_cell_that_does_not_say_so_is_refused(self):
        for reply in (
            b'{"sha": "abc", "version": "0.8.0+abc"}',
            b'{"ingest_project": false}',
            b'{"ingest_project": "yes"}',
            b"{}",
            b"",
            b"not json",
            b"[]",
        ):
            with self.subTest(reply=reply):
                with mock.patch(
                    "urllib.request.urlopen", return_value=_version_reply(reply)
                ):
                    with self.assertRaisesRegex(RuntimeError, "predates project filing"):
                        _require_cell_files_projects("http://cell.example:8080")

    def test_a_cell_with_no_version_answer_is_refused_naming_the_status(self):
        for code in (404, 503):
            with self.subTest(code=code):
                error = urllib.error.HTTPError(
                    "http://cell.example:8080/v1/version", code, "no", {}, None
                )
                with mock.patch("urllib.request.urlopen", side_effect=error):
                    with self.assertRaisesRegex(RuntimeError, f"HTTP {code}"):
                        _require_cell_files_projects("http://cell.example:8080")

    def test_a_cell_that_cannot_be_reached_raises_like_a_failed_post(self):
        with mock.patch(
            "urllib.request.urlopen", side_effect=urllib.error.URLError("refused")
        ):
            with self.assertRaises(urllib.error.URLError):
                _require_cell_files_projects("http://cell.example:8080")


class TestProjectEcho(unittest.TestCase):
    """A batch that names a project is only done once the cell says it recorded it."""

    def test_the_echoed_project_is_accepted(self):
        from core import _require_project_recorded

        _require_project_recorded({"project": "billing"}, b'{"project": "billing"}')

    def test_a_reply_without_the_project_is_refused(self):
        from core import _require_project_recorded

        for reply in (b"{}", b"", b"not json", b"[]", b'{"project": "other"}'):
            with self.assertRaises(RuntimeError, msg=reply):
                _require_project_recorded({"project": "billing"}, reply)

    def test_a_batch_naming_no_project_needs_no_echo(self):
        from core import _require_project_recorded

        _require_project_recorded({"batch_id": "b"}, b"{}")
        _require_project_recorded({"batch_id": "b"}, b"")


class TestPostHttpError(unittest.TestCase):
    """_post should surface HTTP error status and response body in the exception."""

    def test_http_error_message_includes_status_and_body(self):
        import io
        import urllib.error
        import urllib.request

        import core as core_module

        fake_response_body = b'{"error":{"code":"validation_error","message":"batch_id must not be empty"}}'

        original_urlopen = urllib.request.urlopen

        def mock_urlopen(req, timeout=None):
            raise urllib.error.HTTPError(
                req.full_url,
                400,
                "Bad Request",
                {},
                io.BytesIO(fake_response_body),
            )

        urllib.request.urlopen = mock_urlopen
        try:
            with self.assertRaises(RuntimeError) as cm:
                core_module._post("http://localhost:8080", "", {"batch_id": ""})
            msg = str(cm.exception)
            self.assertIn("400", msg)
            self.assertIn("validation_error", msg)
        finally:
            urllib.request.urlopen = original_urlopen


if __name__ == "__main__":
    unittest.main()
