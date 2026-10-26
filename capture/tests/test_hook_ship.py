#!/usr/bin/env python3
"""Integration tests for capture/hook_ship.py against a real stub HTTP server.

Drives hook_ship.py as a subprocess the same way Claude Code does: JSON hook
payload on stdin, environment variables for the Loomtracer server location.
Exercises the real hook contract (session_id/transcript_path from stdin, a
cwd containing dots) end to end, rather than monkeypatching internals.
"""

import http.server
import json
import os
import subprocess
import sys
import tempfile
import threading
import unittest

HOOK_SHIP = os.path.join(os.path.dirname(__file__), "..", "hook_ship.py")


class _StubIngestHandler(http.server.BaseHTTPRequestHandler):
    """Responds to POST /v1/ingest with whatever status the test queued."""

    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(length)
        request = json.loads(body)
        self.server.requests.append(request)

        status = self.server.next_status_fn()
        # A cell that records the project echoes it back; one that predates project
        # filing (echo_project False) accepts the batch and says nothing of it.
        reply = {}
        if self.server.echo_project and request.get("project"):
            reply["project"] = request["project"]
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(json.dumps(reply).encode())

    def do_GET(self):
        self.server.version_requests += 1
        if self.path != "/v1/version" or not self.server.serves_version:
            self.send_response(404)
            self.end_headers()
            return
        reply = {"version": "0.8.0+abc", "cargo_version": "0.8.0", "sha": "abc"}
        # Only a cell that files a batch under its project says so.
        if self.server.ingest_project:
            reply["ingest_project"] = True
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(json.dumps(reply).encode())

    def log_message(self, *args):  # silence default request logging
        pass


class _StubIngestServer:
    """Background HTTP server for /v1/ingest, with a scriptable status queue."""

    def __init__(self):
        self.httpd = http.server.HTTPServer(("127.0.0.1", 0), _StubIngestHandler)
        self.httpd.requests = []
        self.httpd.echo_project = True
        self.httpd.ingest_project = True
        self.httpd.serves_version = True
        self.httpd.version_requests = 0
        self._statuses = [202]
        self.httpd.next_status_fn = self._next_status
        self.thread = threading.Thread(target=self.httpd.serve_forever, daemon=True)
        self.thread.start()

    def _next_status(self):
        if len(self._statuses) > 1:
            return self._statuses.pop(0)
        return self._statuses[0]

    def set_statuses(self, *statuses):
        self._statuses = list(statuses)

    def set_echo_project(self, echo):
        self.httpd.echo_project = echo

    def set_cell(self, files_projects, serves_version=True):
        """files_projects False is a cell that predates project filing: it accepts a batch
        and drops its project, and says nothing of it on /v1/version (or, with
        serves_version False, has no /v1/version at all)."""
        self.httpd.echo_project = files_projects
        self.httpd.ingest_project = files_projects
        self.httpd.serves_version = serves_version

    @property
    def version_requests(self):
        return self.httpd.version_requests

    @property
    def url(self):
        return f"http://127.0.0.1:{self.httpd.server_port}"

    @property
    def requests(self):
        return self.httpd.requests

    def stop(self):
        self.httpd.shutdown()
        self.thread.join(timeout=5)
        self.httpd.server_close()


class TestHookShipStdinContract(unittest.TestCase):
    """hook_ship.py must take session_id/transcript_path from stdin JSON,
    matching the documented Claude Code hook contract, not env vars."""

    def setUp(self):
        self.tmpdir = tempfile.mkdtemp()
        # A cwd with a dot, like every Gas City session
        # (/home/ubuntu/gc/.gc/agents/mayor) — the naive '/' -> '-' mangling
        # bug this bead fixes would miss transcripts under such a path.
        self.session_id = "session-with-dot-cwd"
        self.transcript_path = os.path.join(self.tmpdir, f"{self.session_id}.jsonl")
        self.cursor_home = os.path.join(self.tmpdir, "home")
        os.makedirs(self.cursor_home, exist_ok=True)

        self.server = _StubIngestServer()

    def tearDown(self):
        self.server.stop()

    def _write_transcript(self, records):
        with open(self.transcript_path, "w", encoding="utf-8") as fh:
            for record in records:
                fh.write(json.dumps(record) + "\n")

    def _run_hook(self, payload):
        env = dict(os.environ)
        env["LOOMTRACER_API_URL"] = self.server.url
        env["HOME"] = self.cursor_home  # isolate ~/.loomtracer/cursors
        result = subprocess.run(
            [sys.executable, HOOK_SHIP],
            input=json.dumps(payload).encode(),
            env=env,
            capture_output=True,
            timeout=15,
        )
        return result

    def test_ships_new_turns_via_stdin_payload(self):
        """A real-shaped hook stdin payload for a dotted cwd, against a stub
        /v1/ingest returning 202: new turns arrive and the cursor advances."""
        self._write_transcript(
            [
                {
                    "type": "user",
                    "uuid": "u1",
                    "message": {
                        "role": "user",
                        "content": [{"type": "text", "text": "hello"}],
                    },
                }
            ]
        )
        payload = {
            "session_id": self.session_id,
            "transcript_path": self.transcript_path,
            "cwd": "/home/ubuntu/gc/.gc/agents/mayor",
            "hook_event_name": "Stop",
        }

        self.server.set_statuses(202)

        # First invocation: cursor initialises to EOF, first-run policy
        # skips the existing content (matches sidecar/core semantics).
        result = self._run_hook(payload)
        self.assertEqual(result.returncode, 0)
        self.assertEqual(self.server.requests, [])

        # Append a new turn and run again: it should ship.
        with open(self.transcript_path, "a", encoding="utf-8") as fh:
            fh.write(
                json.dumps(
                    {
                        "type": "assistant",
                        "uuid": "a1",
                        "message": {
                            "role": "assistant",
                            "content": [{"type": "text", "text": "hi back"}],
                        },
                    }
                )
                + "\n"
            )

        result = self._run_hook(payload)
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(self.server.requests), 1)
        self.assertEqual(self.server.requests[0]["session_id"], self.session_id)
        self.assertEqual(self.server.requests[0]["turns"][0]["turn_id"], "a1")

        cursor_file = os.path.join(
            self.cursor_home, ".loomtracer", "cursors", f"{self.session_id}.offset"
        )
        self.assertTrue(os.path.exists(cursor_file))
        with open(cursor_file, encoding="utf-8") as fh:
            offset = int(fh.read())
        self.assertEqual(offset, os.path.getsize(self.transcript_path))

    def test_failed_posts_do_not_advance_cursor_and_hook_never_blocks(self):
        """401, then 500, then an unreachable server: the cursor must not
        advance across any of them, the hook must still exit 0, and the
        next successful run must resend the same batch."""
        self._write_transcript(
            [
                {
                    "type": "user",
                    "uuid": "u0",
                    "message": {
                        "role": "user",
                        "content": [{"type": "text", "text": "seed"}],
                    },
                }
            ]
        )
        payload = {
            "session_id": self.session_id,
            "transcript_path": self.transcript_path,
            "cwd": "/home/ubuntu/gc/.gc/agents/mayor",
            "hook_event_name": "Stop",
        }

        # Init cursor to EOF (first run).
        result = self._run_hook(payload)
        self.assertEqual(result.returncode, 0)

        with open(self.transcript_path, "a", encoding="utf-8") as fh:
            fh.write(
                json.dumps(
                    {
                        "type": "assistant",
                        "uuid": "a1",
                        "message": {
                            "role": "assistant",
                            "content": [{"type": "text", "text": "reply"}],
                        },
                    }
                )
                + "\n"
            )

        cursor_file = os.path.join(
            self.cursor_home, ".loomtracer", "cursors", f"{self.session_id}.offset"
        )
        with open(cursor_file, encoding="utf-8") as fh:
            offset_before = int(fh.read())

        # 401
        self.server.set_statuses(401)
        result = self._run_hook(payload)
        self.assertEqual(result.returncode, 0, "hook must exit 0 even on 401")
        with open(cursor_file, encoding="utf-8") as fh:
            self.assertEqual(int(fh.read()), offset_before)

        # 500
        self.server.set_statuses(500)
        result = self._run_hook(payload)
        self.assertEqual(result.returncode, 0, "hook must exit 0 even on 500")
        with open(cursor_file, encoding="utf-8") as fh:
            self.assertEqual(int(fh.read()), offset_before)

        # Unreachable server.
        env = dict(os.environ)
        env["LOOMTRACER_API_URL"] = "http://127.0.0.1:1"  # nothing listens here
        env["HOME"] = self.cursor_home
        result = subprocess.run(
            [sys.executable, HOOK_SHIP],
            input=json.dumps(payload).encode(),
            env=env,
            capture_output=True,
            timeout=15,
        )
        self.assertEqual(result.returncode, 0, "hook must exit 0 even when unreachable")
        with open(cursor_file, encoding="utf-8") as fh:
            self.assertEqual(int(fh.read()), offset_before)

        # Now succeed: the same batch (turn a1) must resend, not be lost.
        # (requests[] already holds the 401 and 500 attempts that reached
        # the stub; the unreachable-server attempt never did.)
        self.server.set_statuses(202)
        result = self._run_hook(payload)
        self.assertEqual(result.returncode, 0)
        self.assertEqual(self.server.requests[-1]["turns"][0]["turn_id"], "a1")
        with open(cursor_file, encoding="utf-8") as fh:
            self.assertGreater(int(fh.read()), offset_before)

    def test_batches_name_the_project_of_the_hooks_cwd(self):
        """The hook's cwd sits under a .loomtracer-project: the batch names that
        project (folder_marker), so its decisions are filed under it. A cell that
        refuses the handle (400) leaves the cursor, and the span is sent again."""
        repo = os.path.join(self.tmpdir, "repo")
        os.makedirs(os.path.join(repo, "src"))
        with open(os.path.join(repo, ".loomtracer-project"), "w", encoding="utf-8") as fh:
            fh.write("loomtracer\n")

        def record(uuid, role):
            return {
                "type": role,
                "uuid": uuid,
                "message": {"role": role, "content": [{"type": "text", "text": uuid}]},
            }

        self._write_transcript([record("u0", "user")])
        payload = {
            "session_id": self.session_id,
            "transcript_path": self.transcript_path,
            "cwd": os.path.join(repo, "src"),
            "hook_event_name": "Stop",
        }
        self.assertEqual(self._run_hook(payload).returncode, 0)  # cursor to EOF
        with open(self.transcript_path, "a", encoding="utf-8") as fh:
            fh.write(json.dumps(record("a1", "assistant")) + "\n")
        cursor_file = os.path.join(
            self.cursor_home, ".loomtracer", "cursors", f"{self.session_id}.offset"
        )
        with open(cursor_file, encoding="utf-8") as fh:
            offset_before = int(fh.read())

        self.server.set_statuses(400, 202)
        self.assertEqual(self._run_hook(payload).returncode, 0)
        with open(cursor_file, encoding="utf-8") as fh:
            self.assertEqual(int(fh.read()), offset_before)

        self.assertEqual(self._run_hook(payload).returncode, 0)
        self.assertEqual(len(self.server.requests), 2)
        for request in self.server.requests:
            self.assertEqual(request["project"], "loomtracer")
            self.assertEqual(request["project_source"], "folder_marker")
            self.assertEqual(request["turns"][0]["turn_id"], "a1")
        with open(cursor_file, encoding="utf-8") as fh:
            self.assertGreater(int(fh.read()), offset_before)

    def _marked_session(self):
        """A session in a folder marked `loomtracer`, its cursor at the end of one turn.

        Returns (payload, cursor_file, append): append(uuid) adds an assistant turn."""
        repo = os.path.join(self.tmpdir, "repo")
        os.makedirs(repo)
        with open(os.path.join(repo, ".loomtracer-project"), "w", encoding="utf-8") as fh:
            fh.write("loomtracer\n")

        def record(uuid, role):
            return {
                "type": role,
                "uuid": uuid,
                "message": {"role": role, "content": [{"type": "text", "text": uuid}]},
            }

        def append(uuid):
            with open(self.transcript_path, "a", encoding="utf-8") as fh:
                fh.write(json.dumps(record(uuid, "assistant")) + "\n")

        self._write_transcript([record("u0", "user")])
        payload = {
            "session_id": self.session_id,
            "transcript_path": self.transcript_path,
            "cwd": repo,
            "hook_event_name": "Stop",
        }
        self.assertEqual(self._run_hook(payload).returncode, 0)  # cursor to EOF
        cursor_file = os.path.join(
            self.cursor_home, ".loomtracer", "cursors", f"{self.session_id}.offset"
        )
        return payload, cursor_file, append

    def _cursor(self, cursor_file):
        with open(cursor_file, encoding="utf-8") as fh:
            return int(fh.read())

    def test_a_cell_that_predates_project_filing_is_sent_nothing(self):
        """A cell that predates project filing accepts the batch (202), drops `project`
        and queues it with no project, and would queue the same span again under a new
        batch_id on every run, to be filed in the agent's personal project. The client
        asks the cell first (/v1/version): an old cell is sent nothing at all, the
        cursor stays, and once the cell is updated the span is sent once
        (hivemind-8fbto)."""
        payload, cursor_file, append = self._marked_session()
        offset_before = self._cursor(cursor_file)

        self.server.set_cell(files_projects=False)
        for uuid in ("a1", "a2", "a3"):  # a hook run per tool use, the transcript growing
            append(uuid)
            result = self._run_hook(payload)
            self.assertEqual(result.returncode, 0, "the hook never fails")
            self.assertIn(b"predates project filing", result.stderr)
            self.assertEqual(self._cursor(cursor_file), offset_before)
        self.assertEqual(self.server.requests, [], "an old cell queues every batch it gets")
        self.assertEqual(self.server.version_requests, 3)

        self.server.set_cell(files_projects=True)
        self.assertEqual(self._run_hook(payload).returncode, 0)
        self.assertEqual(len(self.server.requests), 1, "the span is sent once")
        request = self.server.requests[0]
        self.assertEqual(request["project"], "loomtracer")
        self.assertEqual([t["turn_id"] for t in request["turns"]], ["a1", "a2", "a3"])
        self.assertGreater(self._cursor(cursor_file), offset_before)

    def test_a_cell_with_no_version_endpoint_is_sent_nothing(self):
        """A cell older than /v1/version answers 404: that is no answer either, so the
        span is held back before anything is sent."""
        payload, cursor_file, append = self._marked_session()
        offset_before = self._cursor(cursor_file)
        append("a1")

        self.server.set_cell(files_projects=False, serves_version=False)
        result = self._run_hook(payload)
        self.assertEqual(result.returncode, 0, "the hook never fails")
        self.assertIn(b"HTTP 404", result.stderr)
        self.assertEqual(self.server.requests, [])
        self.assertEqual(self._cursor(cursor_file), offset_before)

    def test_a_cell_that_files_projects_but_does_not_echo_one_holds_the_span_back(self):
        """The reply is checked as well: a cell that says it files projects and then
        answers a batch without the project (a proxy in front of an older cell) leaves
        the cursor, and the span is sent again."""
        payload, cursor_file, append = self._marked_session()
        offset_before = self._cursor(cursor_file)
        append("a1")

        self.server.set_echo_project(False)
        result = self._run_hook(payload)
        self.assertEqual(result.returncode, 0, "the hook never fails")
        self.assertIn(b"without recording project loomtracer", result.stderr)
        self.assertEqual(self._cursor(cursor_file), offset_before)

        self.server.set_echo_project(True)
        self.assertEqual(self._run_hook(payload).returncode, 0)
        self.assertEqual(len(self.server.requests), 2, "the same span is sent again")
        self.assertGreater(self._cursor(cursor_file), offset_before)

    def test_falls_back_to_env_when_stdin_has_no_payload(self):
        """No stdin payload (manual invocation): falls back to
        CLAUDE_CODE_SESSION_ID and CLAUDE_PROJECT_DIR / cwd mangling."""
        self._write_transcript(
            [
                {
                    "type": "user",
                    "uuid": "u1",
                    "message": {
                        "role": "user",
                        "content": [{"type": "text", "text": "hi"}],
                    },
                }
            ]
        )

        # Place the transcript where jsonl_path_for_session's mangling of
        # CLAUDE_PROJECT_DIR would find it.
        project_dir = "/proj.with.dots"
        mangled = "-proj-with-dots"
        claude_home = os.path.join(self.cursor_home, ".claude", "projects", mangled)
        os.makedirs(claude_home, exist_ok=True)
        fallback_path = os.path.join(claude_home, f"{self.session_id}.jsonl")
        os.replace(self.transcript_path, fallback_path)

        env = dict(os.environ)
        env["LOOMTRACER_API_URL"] = self.server.url
        env["HOME"] = self.cursor_home
        env["CLAUDE_CODE_SESSION_ID"] = self.session_id
        env["CLAUDE_PROJECT_DIR"] = project_dir

        # No stdin content at all.
        result = subprocess.run(
            [sys.executable, HOOK_SHIP],
            input=b"",
            env=env,
            capture_output=True,
            timeout=15,
        )
        self.assertEqual(result.returncode, 0)

        cursor_file = os.path.join(
            self.cursor_home, ".loomtracer", "cursors", f"{self.session_id}.offset"
        )
        self.assertTrue(
            os.path.exists(cursor_file),
            "env-var fallback must still locate and process the transcript",
        )


if __name__ == "__main__":
    unittest.main()
