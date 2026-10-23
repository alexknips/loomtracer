# Loomtracer Capture Plugin

This directory is both the Codex capture plugin and the Claude Code
`loomtracer-capture` plugin. The Claude package installs:

- `/loomtracer-capture:capture` for explicit writes through the deterministic
  capture helper. Use `--kind decision`, `--kind evidence`, or
  `--kind hypothesis` when the kind is known. It writes a local ledger and never
  contacts a server: with `LOOMTRACER_API_URL` set it refuses when the project
  holds no ledger, and otherwise records there and says on stderr that the cell
  did not get it.
- `/loomtracer-capture:capture-decision` as the legacy decision-only wrapper for
  `emit decision.capture`.
- `/loomtracer-capture:query-decisions` for free-text `query recall` reads of the
  local ledger (it never contacts a server; it refuses, naming the directory,
  when the directory holds no ledger, and with `LOOMTRACER_API_URL` set it says
  so) — "what did we decide about X?", never a decision id. For single-decision
  follow-up (why, still-holds, disagree, supersede), install the
  `loomtracer-context` plugin instead.
- `/loomtracer-capture:classify-queue` (Worker A) — drains the pending
  classification work queue using the agent's subscription seat. Run after a
  session to classify batches that the server-side classifier has not yet
  processed. See [Queue drain](#queue-drain-worker-a) below.
- A `SessionStart` hook that tells the agent to check the ledger before it acts
  and to record what is worth keeping (rules learned from failures, design
  choices later work must follow, reversals), not routine status. See
  [Session directive](#session-directive).
- Hooks on Claude Code's `AskUserQuestion` tool: the question is recorded as an
  ask when the agent puts it to you, and your answer as a decision that answers
  it. See [AskUserQuestion hooks](#askuserquestion-hooks).
- A `loomtracer` MCP stdio server wired to `loomtracer mcp`.
- The `loomtracer-capture` skill for capture boundaries and provenance rules.
- The Claude `active-capture` skill, which nudges `/capture <text>
  [--kind decision|evidence|hypothesis]` during live durable-decision
  moments while avoiding synthetic test data, routing chatter, choices that bind
  nothing and are cheap to undo, and requests (a request goes to your tracker).
- The `citation` skill, which guides the agent to pin URL versions (commit
  hash, revision ID, or access date) when a link appears in a capture context.

The shared helper defaults to `source=agent` and derives
`actor_id=agent:<tool>:<name>`. The name is stable only where something stable
names it: `LOOMTRACER_AGENT_SESSION` is checked first, then `GC_AGENT` (mirrored
in `GC_ALIAS`), since it names a fixed crew/polecat/refinery slot that survives
process restarts. With neither it falls back to a raw per-run session id
(`CODEX_THREAD_ID`, `CODEX_SESSION_ID`, or `CODEX_TASK_ID` under Codex;
`CLAUDE_SESSION_ID` or `CLAUDE_CODE_SESSION_ID` under Claude Code), then Gas
City's session-instance variables (`GC_SESSION_ID`/`GC_SESSION_NAME`), so each
session is its own actor. To keep one actor outside Gas City, export
`LOOMTRACER_AGENT_SESSION` (say `alice-laptop`) before starting the client, or
pass `--agent-session <name>` (or `--actor-id`) to the helper; the bundled MCP
server reads the same variable.

## Install

Install the Loomtracer CLI first so `loomtracer` is on `PATH`.

From Claude Code, add this repository as a marketplace and install the plugin:

```text
/plugin marketplace add alexknips/loomtracer
/plugin install loomtracer-capture@loomtracer
/reload-plugins
```

For local development from this checkout:

```text
/plugin marketplace add .
/plugin install loomtracer-capture@loomtracer
/reload-plugins
```

Claude Code can also test the plugin directory directly:

```bash
claude --plugin-dir ./plugins/loomtracer-capture
```

For rig-local dogfooding, copy the active capture skill into the checkout's
project skills directory:

```bash
plugins/loomtracer-capture/scripts/install-active-capture-skill.sh --project-dir .
```

### The plugin and the CLI version

The marketplace serves this plugin from `master`, while people install the latest
release binary of `loomtracer`, so the plugin always runs ahead of the CLI under
it. Anything the plugin passes that only `master` has is therefore passed only
when the installed CLI lists it: the MCP server config, `capture.sh` and the
`AskUserQuestion` hooks each ask `--help` first. On a CLI older than the flag the
plugin works without it (for `--project-from-context`, decisions are filed under
the personal project). CI runs the plugin's MCP servers, capture helper and hooks
against the latest release binary
(`scripts/check_plugin_against_release.py`), so a plugin change that needs a
newer CLI fails there instead of in a stranger's install.

The default ledger is the project-local `./loomtracer/` directory. The bundled MCP
server descriptor pins that location so agents launched from this checkout use
the same ledger without per-session setup. To use another ledger for slash
commands, set the plugin option `loomtracer_dir`, export `LOOMTRACER_DIR`, or pass
`--loomtracer-dir`.

## Provenance Defaults

Claude plugin writes default to `actor_id=agent:claude:<name>` and
`source=agent`. The slash command prefers Gas City's stable `GC_AGENT`/
`GC_ALIAS` slot identity, falling back to `CLAUDE_SESSION_ID` or
`CLAUDE_CODE_SESSION_ID` when neither is set; the bundled MCP server starts
with `--agent-tool claude` and uses the same environment resolution for write
tools when `actor_id` is omitted.

Codex skill writes use the same convention with `actor_id=agent:codex:<name>`,
preferring `GC_AGENT`/`GC_ALIAS` and falling back to `CODEX_SESSION_ID`,
`CODEX_TASK_ID`, or `LOOMTRACER_CODEX_SESSION`. The Codex bundle has its own MCP
config, `.codex-plugin/mcp.json`, which starts the server with
`--agent-tool codex`; each surface names its own tool. With none named, the
MCP server uses the client's own name from its handshake and a CLI write uses
`unknown`.

Bare terminal writes such as `loomtracer emit decision.proposed ...` or
`loomtracer emit decision.capture ...` default to
`actor_id=human:<git config user.email>` and `source=human` when `--actor` is
not supplied and no agent is present in the environment. A typed
`--actor human:<name>` is recorded as given.

## Projects

Every decision capture says which project it belongs to and how that was
determined. The capture helper passes `--project-from-context` (so does the
bundled MCP server, and the Codex skill's direct `loomtracer ... emit
decision.capture` form), so the CLI works the project out from where the agent
is running. First match wins:

1. `--project HANDLE`, when the caller names one. An unregistered handle is
   refused with the register command, never guessed.
2. The `.loomtracer-project` files of the folders the uncommitted change touches
   (working-tree diff plus the staged set); when none of those files sits under
   one, the nearest `.loomtracer-project` walking up from the working directory.
   The file holds one project handle; a marker nested inside an attached folder
   is a sub-project and, being nearer, wins.
3. The project anchored to the Gas City rig (`GC_RIG`).
4. The current project set with `loomtracer project use`. It is a per-machine
   setting kept for the CLI's `--actor` (by default the person at the terminal),
   not a ledger fact.

A change that touches folders of several projects is one decision, recorded for
the nearest project they are all part of, and the CLI says so on its stderr
(`recorded for platform: this change spans auth and billing`; the helper passes
that line through rather than folding it into the one-line confirmation). With no
project in common it is saved to the actor's personal project and the CLI names
the projects it spans and how to move it. A spanning change is never refused and
never dropped.

When none applies, the decision is saved to the actor's personal project. The
confirmation line says so and how to attach the folder:

```text
Captured Loomtracer decision decision-... in ./loomtracer.
project: personal:agent:claude (personal_fallback) — saved to your personal project; ...
this folder is not attached to a project yet; register the project (loomtracer project register <handle>) and commit a .loomtracer-project file holding the handle in this folder
```

The line is the whole recipe: register the project (`loomtracer project register
billing`) and commit a `.loomtracer-project` file containing `billing` in the
folder. `loomtracer project anchor --kind folder` only records a fact about the
project and does not attach the folder; the marker file is what a capture reads,
and `--kind rig` with the rig's name is what binds a Gas City rig.

A session in a Gas City rig that no project in the ledger is anchored to is
refused instead of filed under the personal project: it is writing to the wrong
ledger, and the refusal names the rig, the ledger, and the ways out. A folder
marker, a rig anchor, or a current project is tried first.

Topic keys are declared per project. A capture under a registered project may use
only the keys that project declared; pass `--declare-topic KEY` (each also in
`--topic-keys`) to add a new one, and the confirmation lists what was declared.
`loomtracer project show billing` lists a project's keys. A personal project has no
vocabulary.

A capture from an attached folder confirms with the project and how it was
found instead, for example `project: billing (folder_marker)`. Evidence and
hypothesis captures carry no project. `/loomtracer-context:supersede` works the
same way for the replacement decision; with none found it stays in the project
of the decision it replaces. A wrong project is fixed with `loomtracer move
"<description>" --to <handle>`. The `--project-from-context` flag needs a
`loomtracer` CLI newer than v0.7.0; the helper and the MCP server leave it out on an
older CLI, and its captures are then filed under the personal project. Nothing
here applies over HTTP: MCP-over-HTTP takes the
project as an argument and REST takes none (its decisions land in the personal
project), and only the CLI and the stdio MCP server this plugin ships fill it in
from context. See
[`docs/AGENT_DECISION_CAPTURE.md`](../../docs/AGENT_DECISION_CAPTURE.md#which-project-a-capture-lands-in)
and [`docs/MULTI_TENANCY.md`](../../docs/MULTI_TENANCY.md#projects-inside-a-tenant).

## Session directive

An agent only uses a tool it has been told about. With the plugin installed and the MCP
server connected, agents that nobody had told to use Loomtracer used it 0 times in 31
headless benchmark sessions, because the other hooks fire on `AskUserQuestion`, which an
autonomous agent never calls. So the plugin's `SessionStart` hook
(`scripts/session-start-hook.sh`) adds this to every new session's context:

> Loomtracer ledger: before you act, call the `loomtracer` MCP tool `recall_decisions` (q =
> three or four key words about the task) and follow the earlier decisions that still
> hold. Call `capture_decision` (title, rationale = the why, options, grounding, binds,
> undo_cost) for: a rule learned from something that failed or cost you (save it the
> first time); a design or interface choice later work must follow; a reversal of an
> earlier decision (use `supersede_decision`). Not for routine status, session
> summaries, plans that hold only for this session, or a choice binding nothing and
> cheap to undo.

It says what is worth saving, because told only to record "decisions" the agents in the
benchmarks saved a summary of each session and missed the rules that cost them something.

- **It is a fixed text of about 595 characters**, which the tests hold under a 150-token
  budget (4 characters per token). The text it replaces measured 143 input tokens on the
  wire (19,310 against 19,167 on the same prompt with the hook on and off), so expect the
  wire cost to run above that estimate. The hook reads no ledger and decides nothing; recall
  and capture stay the agent's own calls through the MCP server. A test keeps this quotation
  equal to the text the hook prints.
- **It fails open.** With no `loomtracer` binary (the one on `PATH`, or `LOOMTRACER_CAPTURE_BIN`)
  it prints nothing and exits 0, so the session starts as if the plugin had no hook. A
  ledger directory that does not exist yet still gets the directive: the first session in a
  project is where it is needed, and the MCP server creates the ledger on the first capture.
- **It runs on startup, resume, clear and compact**, which is when Claude Code runs
  `SessionStart` hooks.

To turn it off, set `LOOMTRACER_DIRECTIVE_DISABLE` to any non-empty value. For one project,
put it in that project's `.claude/settings.json`:

```json
{ "env": { "LOOMTRACER_DIRECTIVE_DISABLE": "1" } }
```

For everything you run, export it in your shell or set the same `env` in
`~/.claude/settings.json`. The AskUserQuestion hooks below are separate and stay on;
`LOOMTRACER_ASK_HOOK_DISABLE=1` turns those off.

**Codex.** The Codex package reads the same `hooks/hooks.json`, so the directive ships there
too (checked on Codex 0.159.3: a plugin's `SessionStart` hook reaches the model as context,
and with no `loomtracer` binary the agent sees nothing). Install the plugin with
`codex plugin add loomtracer-capture@loomtracer-plugins`. Codex runs a hook only once it is
trusted, and until then skips it without an error; the trust is kept per hook as a
`trusted_hash` under `[hooks.state."<key>"]` in `~/.codex/config.toml`, and
`codex exec --dangerously-bypass-hook-trust` skips the check for unattended runs.
`LOOMTRACER_DIRECTIVE_DISABLE` turns it off there as well.

The hook needs a `loomtracer` that serves `recall_decisions` and `capture_decision` (v0.7.0
and later do); CI checks that against the latest release
(`scripts/check_plugin_against_release.py`). The text names the tools by their MCP names,
so an agent that has the plugin's MCP server finds them as
`mcp__plugin_loomtracer-capture_loomtracer__recall_decisions` and
`mcp__plugin_loomtracer-capture_loomtracer__capture_decision`.

## AskUserQuestion hooks

An agent asking you a question is an act with a time. The plugin's hooks on Claude
Code's `AskUserQuestion` tool (`hooks/hooks.json`) record it, so the ledger holds
when a question was asked and when it was answered:

| Hook | When | Writes |
|---|---|---|
| `PreToolUse` | the agent calls `AskUserQuestion` | one ask per question (MCP `request_decision`): the question, who asked, the time of the call |
| `PostToolUse` | you have answered | one decision per answered question (MCP `capture_decision`) that answers the same question |

The ask and the decision name the same question, so `loomtracer query why` on the decision
shows `asked_at` and `answered_at`, and the request leaves the waiting list
(`loomtracer query get_waiting_requests`). The two hooks share no state.

What the decision carries:

- **Decided by** `human:<your git email>` (set `LOOMTRACER_HUMAN_ACTOR=human:<name>`
  to change it), **recorded by** the agent (`agent:claude:<name>`).
- **Options** are the ones the agent offered, with their descriptions; **chosen** is
  the one you picked. Several picks in a multi-select become one combined option
  (`Search + Export`); what you picked into it is not listed as turned down. Your own
  words ("Other") are quoted verbatim. Words, or a note with nothing picked, choose an
  offered option when they lead with its name and name no other offered option
  ("Five times, because the jobs are cheap" chooses `Five times`). Words that name no
  single option ("Redis or In-process", "Not Redis, the other one") are the answer
  themselves: the chosen option is what you said, no offered option is listed beside it,
  and none reads as turned down. A note-only answer never quotes Claude Code's
  `(notes only)` placeholder as your words, and one with no note records nothing.
- **Title** is `<header>: <chosen>`. **Rationale** is the note you wrote on your
  pick, word for word. A note too short to read on its own is quoted instead, and
  the rationale says a note is attached. Only when you gave neither a note nor
  words of your own is it a fixed sentence saying no reasons were given. **Rests
  on** is a bet with nothing declared, because nothing was stated with the answer;
  add what it rests on later with `loomtracer ground`.
- **Project** is worked out from where the agent runs, as for any capture
  (`LOOMTRACER_PROJECT` names one outright).
- **Topic** is the slug of the question's header (`Database` becomes `database`). A
  registered project accepts only topic keys it has declared, so when the project has
  not declared that one, the hook files the answer under the fixed key
  `claude-code-question` instead and declares only that key. The hook retries once,
  and only for that refusal (both attempts are in `hook.log`; any other refusal is
  logged, not retried). A project's vocabulary grows by at most that one key from
  these hooks, never one per question. In a personal or unregistered checkout there
  is no vocabulary: the header's key is used and nothing is declared.

What it does not do:

- It records only real asks. The tool call is the act and its time is the ask's
  time. An agent deciding alone, you deciding unprompted and imports write no ask,
  and nothing is back-dated. A question you decline or leave unanswered stays
  waiting; nothing is recorded for it.
- It keeps no averages and no per-person figures: the waiting list is a to-do list,
  and each decision carries its own two times.
- It never gets in the agent's way. A failed write is logged (to stderr and to
  `hook.log`, readable by you alone, under `${CLAUDE_PLUGIN_DATA}/ask-hook/`, else
  `~/.local/state/loomtracer-ask-hook/`) and the question still appears; the hook
  exits 0 with nothing on stdout. When the ask could not be recorded, the answer is
  still captured, without `asked_at`.

Where it writes, and the settings it reads:

| Setting | Effect |
|---|---|
| none | the local ledger (`LOOMTRACER_DIR`, else the plugin's `loomtracer_dir` option, else `<repo root>/loomtracer`, the same rule as `/loomtracer-capture:capture`), through `loomtracer mcp` |
| `LOOMTRACER_API_URL`, `LOOMTRACER_API_KEY` | that server's `POST /mcp` with the key as the bearer token; the token decides who is writing |
| `LOOMTRACER_CAPTURE_BIN` | the `loomtracer` binary to run (default: `loomtracer` on `PATH`) |
| `LOOMTRACER_HUMAN_ACTOR`, `LOOMTRACER_PROJECT` | who answers, and the project to file under |
| `LOOMTRACER_ASK_HOOK_DISABLE=1` | do nothing |

It needs `python3` (without it nothing is recorded, the question is unaffected) and
a `loomtracer` CLI or server that has the ask verb (`loomtracer ask`, MCP
`request_decision`). A server without it refuses the ask (logged) and the answer is
still sent as a decision, without `asked_at`. Upgrade the server before relying on
the asks.

## Verify

Capture one decision:

```text
/loomtracer-capture:capture "Use the Claude plugin for local capture" --kind decision --title "Use the Claude plugin for local capture" --rationale "The plugin installs commands, skill guidance, and MCP without project-local setup" --topic-keys agents,claude,distribution --options "Claude plugin,Manual MCP setup" --chose "Claude plugin" --rests-on-assumption "Contributors install Claude Code plugins from the repository marketplace" --binds "The README and the marketplace listing point contributors at the plugin" --cheap-to-undo
```

Answer two questions before you write a decision. The first looks back:

- **What does it rest on?** `--rests-on-decision` (a decision we already made),
  `--rests-on-evidence` with `--evidence-source` (something observed),
  `--rests-on-assumption`, or `--bet` (nothing yet). A capture that names none is
  refused and writes nothing. The decider's own words are not a grounding; they go
  in `--quote` with `--question`.

The second looks forward, and both halves are required (a capture that leaves
either out is refused and writes nothing):

- **What does it bind, and what would undoing it cost?** `--binds "<what>"` (other
  people, other code or a later choice depend on it) or `--binds-nothing`, and
  `--undo-cost "<what>"` or `--cheap-to-undo`.

A choice is a decision when it binds something or is costly to undo. Typing
"apple" rather than "Apple" in one comment binds nothing and is cheap to undo:
not a decision. Merging tonight despite a failing check binds nothing later but
is costly to undo: a decision. A capture that answers `--binds-nothing
--cheap-to-undo` is still recorded, and the reply says it is below the decision
floor. A request ("make this a bead") is not a decision: a request goes to your
tracker. The four answer flags need a `loomtracer` newer than v0.7.0: on an older
CLI the helper leaves them out and says so on stderr, a v0.7.0 MCP server ignores
the `binds` and `undo_cost` arguments, and either way the answers are not
recorded. See the `loomtracer-capture` skill.

`--options` splits on every comma. A label that holds a comma goes on its own
`--option "Rename later, after the comparison"`: the flag takes one label whole,
repeats, and `--chose` repeats the label exactly. It needs a `loomtracer` newer
than v0.7.0; the helper refuses (exit 2, nothing written) when the CLI lacks it.

Capture one evidence item:

```text
/loomtracer-capture:capture "The plugin smoke test wrote a decision and queried it back" --kind evidence
```

The command prints a one-line confirmation and a query suggestion. Run the
suggested query or use:

```text
/loomtracer-capture:query-decisions --actor-id agent:claude:<name> --source agent --limit 10
```

The MCP server appears as `loomtracer` in Claude Code's MCP tool list after the
plugin is loaded.

## Queue drain (Worker A)

Loomtracer accumulates unclassified ingest batches in a work queue. Worker A
drains this queue using the agent's subscription seat — no API key required.

### Run manually

```text
/loomtracer-capture:classify-queue
```

The command lists pending batches, classifies each one using your subscription
model, and writes `IngestBatchClassified` events. Report at the end: batches
processed and total captures written.

Pass `--limit N` to cap the number of batches processed per run (default 20):

```text
/loomtracer-capture:classify-queue --limit 5
```

### Check queue depth

`classify-queue list --json` answers with the same object on a local ledger and against a
server-backed cell (`LOOMTRACER_API_URL` set): `batches` (at most `--limit`, 20 by default),
`pending_total` (the whole queue's depth), `truncated` (the page holds fewer batches than are
pending) and `budget`:

```bash
loomtracer classify-queue list --json --limit 1 | jq .pending_total
```

That is the reply of builds after the v0.7.0 release. The v0.7.0 release prints the bare list of
batches for a local ledger (`--limit` 20 by default, so count it with a `--limit` past the queue:
`jq length`) and `{batches, budget}`, with no `pending_total`, from a server. The session hook
below reads all three shapes.

### Session hook (optional)

Add `scripts/check-classify-queue.sh` as a `Stop` hook in `.claude/settings.json`
to see a nudge when unclassified batches are waiting:

```json
{
  "hooks": {
    "Stop": [{
      "matcher": "",
      "command": "plugins/loomtracer-capture/scripts/check-classify-queue.sh"
    }]
  }
}
```

The hook only prints a one-line notification; it does not drain autonomously. In a project with
no ledger (none under `./loomtracer/`, or under `LOOMTRACER_DIR`) it prints nothing and creates
nothing, so it is safe to install for every project. It asks as usual when `LOOMTRACER_API_URL` or
`LOOMTRACER_DATABASE_URL` is set, since neither names a folder.

### Bounds

- Each batch = one model invocation (subscription-seat bound).
- Large backlogs drain across multiple `/classify-queue` runs — the queue persists.
- The server-side classifier (Worker B) and Worker A share the same queue;
  concurrent classification is idempotent (last-writer-wins per batch).

## Uninstall

```text
/plugin uninstall loomtracer-capture@loomtracer --prune
```

Remove the marketplace if no other Loomtracer plugins are installed:

```text
/plugin marketplace remove loomtracer
```
