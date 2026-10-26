# Loomtracer

[![CI](https://github.com/alexknips/loomtracer/actions/workflows/ci.yml/badge.svg)](https://github.com/alexknips/loomtracer/actions/workflows/ci.yml)
[![Kuzu nightly](https://github.com/alexknips/loomtracer/actions/workflows/kuzu-nightly.yml/badge.svg?branch=master)](https://github.com/alexknips/loomtracer/actions/workflows/kuzu-nightly.yml)

Loomtracer is memory for decisions. You and your agents record what was decided,
which options lost and why, and who decided, you or the agent. The next session
does not reintroduce what you dropped or rewrite what you agreed. When a
decision is contested or replaced, its status changes; nothing goes quietly
stale. One person and their agents, local-first, AGPL-3.0-only, free to self-host.

It is not only for engineering. A decision record holds a title, a rationale,
the options weighed, the evidence it rests on and who decided; nothing in it is
specific to code, so it fits a research design, a launch plan or a purchase as
well as an architecture choice. The examples below are engineering ones because
a developer's setup is what can run Loomtracer today: a local ledger or a
self-hosted cell, the MCP server, and the Claude Code and Codex plugins.

It is deliberately a decision graph, not a chat archive, notes app, or task
tracker. Humans and agents are both represented as actors, disagreement is
preserved as first-class state, and decision status is derived from graph
relations instead of being silently overwritten.

See [VISION.md](VISION.md) for why it exists,
[PRINCIPLES.md](PRINCIPLES.md) for the load-bearing constraints, and
[STRATEGY.md](STRATEGY.md) for the active investment fronts.

## Quickstart

The fastest first run uses an isolated temporary ledger, captures one decision,
and immediately queries it back:

```bash
loomtracer --actor human:alice quickstart
```

From a fresh clone before installing the binary:

```bash
cargo run -- --actor human:alice quickstart
```

Every write carries actor provenance. Use `--actor` to override the actor with
a stable human id such as `human:alice` or an agent id such as
`agent:codex:<name>` (a name that stays the same from one session to the next).
If `--actor` is omitted, the CLI falls back to
`LOOMTRACER_ACTOR`, then `human:<git config user.email>`, then a local user name. When
the environment shows an agent (an agent tool or session variable, or a Gas City
identity), the fallback is that agent's `agent:<tool>:<name>` instead of the git
user, so an agent's `ground`, `retitle`, `disagree` and `supersede` are recorded as
the agent that ran them; a `LOOMTRACER_ACTOR` that already names an agent is kept.

To capture and query the first real decision in the current directory:

```bash
loomtracer --actor human:alice --loomtracer-dir ./loomtracer emit decision.capture \
  --title "Use Loomtracer for architecture decisions" \
  --rationale "We need a durable record of what was decided, why, and by whom" \
  --topic-keys onboarding,architecture \
  --options "Loomtracer,Notes" \
  --chose "Loomtracer" \
  --rests-on-assumption "Architecture choices are revisited often enough to need a record" \
  --binds "Later architecture decisions are recorded here, not left in chat" \
  --cheap-to-undo

loomtracer --loomtracer-dir ./loomtracer query search_decisions \
  --topic onboarding \
  --limit 5
```

The three answers at the end (what it rests on, what it binds, what undoing it
costs) are required; [CLI Shape](#cli-shape) says why.

## What Loomtracer Does Today

- **Write.** A typed event ledger records decision, evidence, hypothesis,
  option, and relation events. The `commands` module validates invariants and
  appends to the ledger; both CLI and MCP write through the same internal
  functions.
- **Project.** Ledger events are projected into a graph view of `Decision`,
  `Option`, `Evidence`, `Hypothesis` and `Question` nodes, plus the `Actor`s
  who acted and the `Project`s decisions are filed under. Status (`proposed`,
  `accepted`, `rejected`, `contested`, `superseded`) is derived from explicit
  graph edges, not stored.
- **Read.** Query code derives status, walks supersession chains, and surfaces
  contested decisions through deterministic graph reads. Search includes
  topic/status filters and FTS-backed full-text search; `recent_decisions`
  exposes bounded recent decision reads, and `disagree`/`supersede` are
  first-class CLI verbs mirrored by MCP tools.
- **Capture from agents.** Installable plugins for Claude Code and Codex,
  plus the bundled MCP stdio server, let agents capture decisions in-flow
  with auto-populated actor and provenance defaults.

## Decision Quality — Where Loomtracer is Heading

*The quality profile, attention findings and failure-mode attribution below are shipped; the organizational feedback loop is planned.*

Recording decisions is the foundation. The next layer is measuring whether
they held up — and feeding that signal back so organizations can improve how
they make decisions.

**Quality profile (shipped).** For any decision, `score_decision` (MCP, and
`loomtracer query score_decision`) returns seven dimensions: framing, alternatives,
information, reasoning, values and tradeoffs, bias exposure and calibration. Each is
either assessed (a level of none, partial or solid, with the reasons and the ids of the
records behind it) or *not assessed*, with the reason. The levels come from what the
record states (options recorded, evidence and prior decisions it rests on, a rationale, a
declared confidence), never from a judgement of whether it is sound, and nothing recorded
after the decision raises them. There is no composite number, no tier and no grade. No LLM
required; works self-hosted.

**Attention findings (shipped).** `scan_decision_quality` lists the decisions that need a
look now: a bet past its check date, a decision whose premise was superseded, rejected or
refuted, evidence nobody has re-checked. Each finding carries a stable id, the reason in
words and the dimensions it bears on. `get_suggestions` returns the same page without the
findings someone has acknowledged (`exclude_acknowledged`, true by default);
`acknowledge_suggestion` records that a finding was looked at, as an attributed event. What happened to
a decision afterwards (superseded, contested, refuted premise) is shown as outcome, status or
staleness, not folded into a quality grade. See
[`docs/DECISION_SCORING.md`](docs/DECISION_SCORING.md).

**Failure-mode attribution (shipped).** `analyze_failure_modes` (MCP, stdio and
HTTP) asks which conditions go with decisions that did not hold up, and answers
in aggregate. Each group gives its size, how many failed, its failure rate and
how far that is from the whole ledger's rate, with a confidence flag that
follows the group's size. The groups are authorship (human-authored, agent
proposed and human accepted, agent only), review (unreviewed, self-accepted,
peer-reviewed, disputed, rejected), whether an agent acted within a human's
delegation or alone, capture source, whether options and evidence were
recorded, and each quality dimension at each level. A decision failed when it
was superseded, rejected or contested, or a premise it rests on no longer
stands; how it was made is a condition to group by, never a failure itself. It
reads every decision, returns no per-person ranking, does not group by model,
and needs no LLM.

**Organizational self-improvement (planned).**
The goal is understanding *how* mistakes happen in an organization — by
condition and pattern, not by assigning blame to individuals. Engineers are
becoming decision engineers; Loomtracer aims to give that discipline measurable
metrics and a structured feedback loop. The metrics above ship; the loop that
carries them back into how an organization decides is planned. External
consumers — dashboards, automation, factory loops — pull profiles, findings and
signals via MCP.

The default CLI stores events in SQLite at `./loomtracer/ledger.sqlite` or the
directory passed with `--loomtracer-dir`. A write creates the ledger on first use;
a read (`query`, `digest`, `dump`, `export`, `map`, `reexamine`, `tui`,
`restatements propose`, `classify-queue list`, `project list`, `show` and
`decisions`) aimed at a folder
that holds none refuses (exit 2, naming the folder) and creates nothing, so a
mistyped folder is never read as "no decisions". So does a verb that acts on a
decision or project the ledger already holds (`disagree`, `supersede`, `move`,
`retitle`, `ground`, `reaffirm`, `review`, `quality-scan`, `restatements apply`,
`classify-queue submit`,
and `project link`, `unlink`, `anchor`, `declare-topic` and `use`): there is
nothing for it to act on, and a mistyped folder is never read as a missing
decision. Queries and DOT dumps replay the
ledger into an in-memory graph view by default. A Kuzu-backed projection is
available behind the optional `graph-kuzu` feature.

SQLite is a deliberate short-term choice; the long-term storage backend is
open. See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for the current state
model and what is marked temporary.

Out of scope today: compactification, similarity search, semantic ranking,
LLM-backed query, federation. The first four belong to the agentic layer above
the ledger; federation is a later step on the shared-backend front —
[`STRATEGY.md`](STRATEGY.md) tracks where each one stands.

## Self-hosting

Run a full Loomtracer cell on your own infrastructure — your data stays on your
machine, nothing phones home, no API key required to get started.

```bash
git clone https://github.com/alexknips/loomtracer && cd loomtracer
cp .env.example .env
sed -i "s/change-me-before-production/$(openssl rand -hex 32)/g" .env
docker compose up --build -d
```

A cell is a shared backend: every request is authenticated and scoped to one
tenant (see [`docs/MULTI_TENANCY.md`](docs/MULTI_TENANCY.md)).

Then install the capture plugin and start capturing decisions — see
[`docs/SELF_HOSTING.md`](docs/SELF_HOSTING.md) for the full runbook (auth,
multi-user, production checklist, troubleshooting) and
[`docs/KEYLESS_CAPTURE.md`](docs/KEYLESS_CAPTURE.md) for a zero-to-first-decision
walkthrough that needs no `ANTHROPIC_API_KEY`.

## Install

Loomtracer is self-hosted: you install it and run it yourself, and the ledger
stays on your own machine or infrastructure.

Install the default CLI directly from Git:

```bash
cargo install --git https://github.com/alexknips/loomtracer --locked loomtracer
```

Tagged releases publish prebuilt tarballs for Linux (x86_64 and ARM64) and
macOS (ARM64 / Apple Silicon). On Intel macOS, install from source with the
`cargo install --git` command above. The installer selects the matching
asset and verifies its SHA-256 checksum before copying `loomtracer` into
`~/.local/bin`:

```bash
curl -fsSL https://raw.githubusercontent.com/alexknips/loomtracer/master/scripts/install.sh | sh
```

Set `LOOMTRACER_VERSION=v0.7.0` to install a specific release tag or
`LOOMTRACER_INSTALL_DIR=/usr/local/bin` to choose another destination.

Verify the installed binary:

```bash
loomtracer --version
loomtracer --help
```

The release asset names are:

| Platform | Asset |
| --- | --- |
| Linux x86_64 | `loomtracer-linux-x86_64.tar.gz` |
| Linux ARM64 | `loomtracer-linux-arm64.tar.gz` |
| macOS ARM64 | `loomtracer-macos-arm64.tar.gz` |

Kuzu support and the terminal UI are optional and not part of the default
binary:

```bash
cargo install --git https://github.com/alexknips/loomtracer --locked --features graph-kuzu loomtracer
cargo install --git https://github.com/alexknips/loomtracer --locked --features tui loomtracer
```

The crate is prepared for packaging but is not yet published to crates.io;
`Cargo.toml` keeps `publish = false` until the project explicitly reserves
the name.

## Build And Test

```bash
cargo build
cargo test
```

To build from this checkout and install `loomtracer` onto `$PATH` for local
dogfooding, run `make install` instead of copying `target/debug/loomtracer` by
hand — see [`docs/DOGFOOD.md`](docs/DOGFOOD.md#starting-an-agent).

Kuzu support is optional and may compile bundled native C++ code:

```bash
cargo test --features graph-kuzu kuzu -- --nocapture
```

Only run the Kuzu feature tests when changing the Kuzu adapter or explicit Kuzu
CLI path. CI does not build Kuzu per change; the `Kuzu nightly` workflow builds
and tests it on `master` every night. See `docs/ARCHITECTURE.md` for native
dependency notes.

Additional test harnesses:

```bash
cargo test --test golden
cargo test --test golden -- --bless
cargo test --test local_capture_demo -- --nocapture
cargo test --test slack_app -- --nocapture
cargo test --test seed -- --include-ignored
cargo test --test seed replay_smoke -- --nocapture
```

`cargo test --test local_capture_demo -- --nocapture` demonstrates the local
Slack-style plus agent capture prototype against a temp ledger. It ingests a
fake Slack thread, records Codex and Claude decisions through `decision.capture`,
and verifies topic/status queries expose distinct Slack and agent provenance.

`cargo test --test slack_app -- --nocapture` exercises the real local-first
Slack app shell: manifest generation, workspace install state, queued thread
capture, queue drain into the Loomtracer ledger, `/loomtracer query`, and
`/loomtracer show` responses with source citations.

`cargo test --test seed -- --include-ignored` writes a deterministic demo ledger
under `./loomtracer/` unless `LOOMTRACER_SEED_DIR` points somewhere else. The replay
smoke test is warning-only for query drift and exits non-zero only for setup,
ledger, projection, or query failures.

## CLI Shape

Use `--actor` to override the human or agent taking the action. Bare terminal
writes default to `human:<git config user.email>` with `source=human`. Use
`--loomtracer-dir` to choose the ledger directory.

```bash
loomtracer --actor alice emit evidence.recorded \
  --content "SQLite WAL is sufficient for current local writes"

loomtracer --actor alice emit hypothesis.recorded \
  --statement "Embedded storage keeps onboarding under five minutes"
```

Decisions go through `decision.capture`. Agents can use it noninteractively: run
by an agent with no `--actor` typed, it records a stable `agent:<tool>:<name>`
and writes events with `source=agent`; a typed `--actor` is recorded as typed:

```bash
loomtracer --loomtracer-dir ./loomtracer/ emit decision.capture \
  --title "Use direct CLI capture for agent decisions" \
  --rationale "The local command is deterministic and does not depend on hooks" \
  --topic-keys agents,capture \
  --options "Direct CLI,MCP server" \
  --chose "Direct CLI" \
  --rests-on-assumption "Agents already have shell access to the local ledger" \
  --binds "Plugins and docs teach the CLI path before MCP" \
  --cheap-to-undo
```

Options are short human labels (`"Direct CLI"`), never slugs or letter codes:
`why` and `verify` print them as written, and `--chose` alone says which one won.
A slug that slips through still reads as words, with what was recorded beside it
(`chose: Postgres (recorded as: pick-a-postgres)`).

Every capture answers two questions before it is written. What does it rest on
(a decision we already made, something observed, something assumed, or a declared
`--bet`)? And what does it bind beyond the edit in front of you, and what would
undoing it cost (`--binds` or `--binds-nothing`, `--undo-cost` or
`--cheap-to-undo`)? A choice that binds nothing and is cheap to undo is not a
decision: it is still recorded, marked below the decision floor, and a request
goes to your tracker. A capture that leaves either question out is refused and
nothing is written.

Humans can review recent agent decisions in a guided terminal flow:

```bash
loomtracer --actor human:lead --loomtracer-dir ./loomtracer review \
  --actor 'agent:*' \
  --since 7d \
  --unreviewed-only
```

The review flow reads candidates through the deterministic `query recent`
path, then records explicit write-layer events: approve appends
`decision.accepted`, disagree appends `decision.rejected` with the reason, and
supersede proposes a replacement decision plus `decision.superseded`.
Loomtracer does not create a separate review event. Reviewed/unreviewed state is
derived from reviewer-authored accept, reject, or supersede events, so replaying
the ledger reproduces the same reviewed state.

### Agent Capture Plugins

The repository ships installable capture bundles for Claude Code and Codex. Both
packages teach agents when to preserve a decision and how to call the same
`decision.capture` CLI shown above. The direct CLI remains the critical write
path; MCP is bundled as a supplemental tool transport.

Claude Code uses the marketplace at `.claude-plugin/marketplace.json` and the
plugin in `plugins/loomtracer-capture`. Install it from Claude Code:

```text
/plugin marketplace add alexknips/loomtracer
/plugin install loomtracer-capture@loomtracer
/reload-plugins
```

In this repository, `.claude/settings.json` advertises the marketplace and
enables `loomtracer-capture@loomtracer` so trusted Claude Code sessions are prompted
to install it. The plugin provides:

- `/loomtracer-capture:capture` — capture a decision, evidence, or hypothesis
- `/loomtracer-capture:capture-decision` — legacy decision-only shorthand
- `/loomtracer-capture:query-decisions` — free-text recall: "what did we decide about X?"
- `/loomtracer-capture:batch-capture` — classify the recent conversation with a
  Haiku subagent in your own session and capture the durable decisions in it
  (keyless)
- `/loomtracer-capture:classify-queue` — drain the pending classification queue
  using your subscription seat (**no `ANTHROPIC_API_KEY` required**)
- Hooks on Claude Code's `AskUserQuestion` tool: the question an agent puts to
  you is recorded as an ask, and your answer as a decision that answers it
- A `loomtracer` MCP server wired to `loomtracer mcp`
- A session-start directive: every session is told (about 595 characters) to call
  `recall_decisions` before acting and `capture_decision` for a rule learned from a
  failure, a design choice later work must follow, or a reversal, not routine status,
  so an agent nobody briefed still checks and fills the ledger.
  Turn it off with `LOOMTRACER_DIRECTIVE_DISABLE=1`.

See `plugins/loomtracer-capture/README.md` for uninstall and verification steps.

A second plugin from the same marketplace, `plugins/loomtracer-context`, is the
read and contest side: it consults, verifies and pushes back on decisions by
describing them, never by id, through the local CLI (it ships no MCP server).
Install it next to the capture plugin:

```text
/plugin install loomtracer-context@loomtracer
```

- `/loomtracer-context:situational` — "what should I know before I touch this?",
  from the current diff, paths or branch
- `/loomtracer-context:recall` — "what did we decide about X?"
- `/loomtracer-context:why` and `/loomtracer-context:verify` — a decision's
  rationale and neighborhood, and whether it held up
- `/loomtracer-context:importance` — which decisions carry the most impact, and
  who decided them
- `/loomtracer-context:disagree` and `/loomtracer-context:supersede` — contest or
  replace a decision
- `/loomtracer-context:ground`, `/loomtracer-context:reexamine` and
  `/loomtracer-context:reaffirm` — say what a decision rests on, list the
  decisions whose premise changed, and say one still holds

See `plugins/loomtracer-context/README.md` for the full list and the install
steps.

**Keyless capture:** `ANTHROPIC_API_KEY` is optional. Without it the server
runs without a background classifier; run `/loomtracer-capture:classify-queue`
after a session to classify pending batches using your agent's subscription
seat. See [`docs/KEYLESS_CAPTURE.md`](docs/KEYLESS_CAPTURE.md) for the full
zero-to-first-decision walkthrough.

From a local Loomtracer checkout, start Codex in this repository, open `/plugins`,
choose `Loomtracer Plugins`, and install `Loomtracer Capture`. From another
checkout or machine, add this repository as a marketplace first:

```bash
codex plugin marketplace add https://github.com/alexknips/loomtracer.git
```

Users who only want the instruction bundle can copy
`plugins/loomtracer-capture/skills/loomtracer-capture` into `$HOME/.agents/skills/`
and invoke `$loomtracer-capture` directly. Local capture defaults to
`./loomtracer/`; set `LOOMTRACER_DIR` or pass `--loomtracer-dir` to point Codex at a
shared ledger.

### MCP Server

Any MCP-aware client (Claude Desktop, Claude Code, Cursor, Codex with MCP
support, etc.) can capture and query decisions through the bundled MCP stdio
server. The server is a thin transport: capture tools call into the same
`Commands` API the CLI uses, query tools call into the same `queries` API.
Smart behavior stays out — see the "What Loomtracer Does Today" section above.

```bash
loomtracer --loomtracer-dir ./loomtracer/ mcp
```

Once installed, point your MCP client at the same binary. Example config for
Claude Desktop (`~/Library/Application Support/Claude/claude_desktop_config.json`
on macOS):

```json
{
  "mcpServers": {
    "loomtracer": {
      "command": "loomtracer",
      "args": ["mcp"],
      "env": { "LOOMTRACER_DIR": "/absolute/path/to/your/loomtracer/dir" }
    }
  }
}
```

Cursor uses the same shape under `mcp.servers`. The server exposes these tools:

| Tool | Layer | Wraps |
| --- | --- | --- |
| `capture_decision` | write | `emit decision.capture` |
| `capture_evidence` | write | `emit evidence.recorded` |
| `capture_hypothesis` | write | `emit hypothesis.recorded` |
| `disagree_decision` | write | `disagree` |
| `supersede_decision` | write | `supersede` |
| `link_supersession` | write | `emit decision.superseded` |
| `move_decision` | write | `move` |
| `retitle_decision` | write | `retitle` |
| `ground_decision` | write | `ground` |
| `reaffirm_decision` | write | `reaffirm` |
| `acknowledge_suggestion` | write | (MCP only) |
| `request_decision` | write | `ask` |
| `classify_queue_submit` | write | `classify-queue submit` (MCP over HTTP only) |
| `get_decision` | read | `query get_decision` |
| `get_relevant_decisions` | read | `query get_relevant_decisions` |
| `get_situational_decisions` | read | `query situational` |
| `get_supersession_chain` | read | `query get_supersession_chain` |
| `get_decision_neighborhood` | read | `query get_decision_neighborhood` |
| `get_decision_outcome` | read | `query get_decision_outcome` |
| `search_decisions` | read | `query search_decisions` |
| `recent_decisions` | read | `query recent_decisions` |
| `get_waiting_requests` | read | `query get_waiting_requests` |
| `get_contested_decisions` | read | `query get_contested_decisions` |
| `get_open_why_decisions` | read | `query get_open_why_decisions` |
| `get_reexamine_items` | read | `reexamine` |
| `get_changed_decisions` | read | `query get_changed_decisions` |
| `score_decision` | read | `query score_decision` |
| `scan_decision_quality` | read | `query scan_decision_quality` |
| `get_suggestions` | read | `query get_suggestions` |
| `rank_decisions_by_importance` | read | `query rank_decisions_by_importance` |
| `scan_misfiled_decisions` | read | `query scan_misfiled_decisions` |
| `get_decision_context` | read | (MCP only) |
| `decision_context_candidates` | read | (MCP only) |
| `decision_quality_candidates` | read | (MCP only) |
| `analyze_failure_modes` | read | (MCP only) |
| `classify_queue_list` | read | `classify-queue list` (MCP over HTTP only) |
| `dump_graph` | read | `dump --format dot` |
| `recall_decisions` | layer-3 | search + ranked digest |
| `loomtracer_compact_view` | layer-3 | compact subgraph projection |
| `summarize_decisions` | layer-3 | decision digest |

`actor_id` is optional on every capture tool. Left out, it defaults to
`agent:<tool>:<name>`, where the tool is `--agent-tool` if given, else taken from
the environment, else from the MCP client's own name (`claude` for Claude Code,
`codex` for Codex). The `<name>` stays the same across sessions only when
something names it, first match wins: `--session-id`, `LOOMTRACER_AGENT_SESSION`,
or a Gas City agent's `GC_AGENT`/`GC_ALIAS`. With none of those it is the
client's own session id (`CLAUDE_CODE_SESSION_ID` under Claude Code), or a fresh
`mcp-<uuid>` for each server process when the client passes none, so every
session records under a new actor and one person's agent shows up as many. To
keep one actor outside Gas City, start your client with `LOOMTRACER_AGENT_SESSION`
set (say `alice-laptop`), or pass `actor_id` on every call. Pass `actor_id` to
record a different actor. The server records `source=agent` for every write.
Anonymous writes are still impossible: the underlying `Commands` API requires a
non-empty actor.

`LOOMTRACER_DIR` selects the ledger directory if `--loomtracer-dir` is not set.

Local markdown or text decision notes can be imported without network access.
All imported decisions land in the ledger as **unreviewed** (proposed, not
auto-accepted) and flow into `loomtracer review --unreviewed-only`.

Auto-detection: files containing explicit `Decision:` blocks use the
deterministic block parser. Files without them are treated as prose and the
extractor supplied via `--extractor-command` extracts candidate decisions
inline. Re-importing identical input is a no-op. Conflicts are reported by
default; resolve with `--on-conflict keep_existing`, `supersede`, `contest`,
or `add_context`:

```bash
loomtracer --actor alice --json import documents --file ./notes/decision.md
loomtracer --actor alice --json import documents --on-conflict supersede ./notes/
# Prose extraction via extractor command:
loomtracer --actor alice import documents --extractor-command loomtracer-document-extractor ./notes/memo.txt
```

`--extractor-command` and `--llm-response` are alternatives, and `--extractor-arg` needs
`--extractor-command`. A command line that breaks either rule exits 2 before the ledger is
opened, so nothing is recorded, not even the `Decision:` blocks in the input.

PDFs and OCR-backed text are prepared before import. The preparation command
does not write ledger events; it materializes reviewable text with source/page
metadata, then the reviewed output can be imported through the same document
importer:

```bash
loomtracer --json import prepare-documents ./sources/decision.pdf \
  --output-dir ./prepared
loomtracer --actor alice --json import documents ./prepared
```

OCR text files such as `scan.ocr.txt` are marked `review_required`, and that
uncertainty is preserved in the final imported source reference.

The emit commands print the new entity id or event id. Add `--json` for a
structured output envelope.

Query commands return JSON:

```bash
loomtracer query get_decision --id decision-001
loomtracer query get_relevant_decisions --topic architecture
loomtracer query get_relevant_decisions --topic architecture --status accepted
loomtracer query get_supersession_chain --id decision-001
loomtracer query recent_decisions --since 1h --limit 5
```

With a binary installed using `--features graph-kuzu`, use the persistent local
Kuzu projection explicitly:

```bash
loomtracer --graph-backend kuzu query get_relevant_decisions \
  --topic architecture
```

Export the current projected graph as deterministic Graphviz DOT:

```bash
loomtracer dump --format dot > graph.dot
```

With a binary installed using `--features tui`, run the read-only decision
search TUI:

```bash
loomtracer tui --q queue --topic architecture \
  --status accepted --dot-output focused-neighborhood.dot
```

The TUI uses the same query APIs as the CLI, supports keyboard search and graph
navigation, and exports the focused one-hop neighborhood as DOT with `x`.

## Read More

Level-1 guidance (read these first):

- [`VISION.md`](VISION.md) — why Loomtracer exists and who it's for.
- [`PRINCIPLES.md`](PRINCIPLES.md) — the constraints Loomtracer cannot trade
  away.
- [`STRATEGY.md`](STRATEGY.md) — active investment fronts; the filter a bead
  is judged against.

Site and reference docs:

- [alexknips.github.io/loomtracer-site](https://alexknips.github.io/loomtracer-site/)
  — guides, concepts, the CLI and MCP reference, and the live demo. The site
  lives in its own repo,
  [`alexknips/loomtracer-site`](https://github.com/alexknips/loomtracer-site).
- [`alexknips/loomtracer-web`](https://github.com/alexknips/loomtracer-web) — the
  web UI (a Next.js app that reads a Loomtracer server over its HTTP API). Its
  `web/LOOMTRACER_API_MAP.md` is the contract between the two repositories: a
  change to both sides is two pull requests, this one landing and being released
  first, then the UI's raising the release it is pinned to.

Architecture and surfaces:

- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — three-layer architecture,
  surface uniformity, state model, storage, and other architectural decisions
  (some marked temporary).
- [`docs/MULTI_TENANCY.md`](docs/MULTI_TENANCY.md) — multi-tenant model for
  the MCP/API surface.
- [`docs/SEARCH_DESIGN.md`](docs/SEARCH_DESIGN.md) — storage-agnostic search
  surface design.
- [`docs/REMOTE_DB.md`](docs/REMOTE_DB.md) — long-term shared-backend
  direction.

Per-feature docs:

- [`docs/AGENT_DECISION_CAPTURE.md`](docs/AGENT_DECISION_CAPTURE.md) — the
  Claude/Codex capture path (CLI, HTTP API, hooks, MCP, and keyless Worker-A).
- [`docs/KEYLESS_CAPTURE.md`](docs/KEYLESS_CAPTURE.md) — zero-to-first-decision
  walkthrough without an `ANTHROPIC_API_KEY`.
- [`docs/SELF_HOSTING.md`](docs/SELF_HOSTING.md) — self-hosting runbook (auth,
  users, production checklist, troubleshooting).
- [`docs/DOGFOOD.md`](docs/DOGFOOD.md) — operating contract for the repo-local
  shared dogfood ledger and agent capture loop.
- [`docs/LOCAL_CAPTURE_DEMO.md`](docs/LOCAL_CAPTURE_DEMO.md) — local Slack plus
  agent capture demo.
- [`docs/SLACK_APP.md`](docs/SLACK_APP.md) — local-first Slack app install,
  queue, and query surface.
- [`docs/TEXT_IMPORT_AND_DIFF_SEMANTICS.md`](docs/TEXT_IMPORT_AND_DIFF_SEMANTICS.md)
  — local document import and temporal decision diff semantics.
- [`docs/M2_WEEKLY_DIFF_DEMO.md`](docs/M2_WEEKLY_DIFF_DEMO.md) — import
  documents locally, then ask which decisions were added since last week.
- [`AGENTS.md`](AGENTS.md) — project standards and non-goals for
  contributors.
- [`tests/seed/README.md`](tests/seed/README.md) — deterministic seed
  dataset, replay smoke test, and golden snapshot workflow.
