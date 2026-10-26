# AGENTS.md — Loomtracer High Bar

This document defines the **standard of excellence** that any agent (human or AI) contributing to Loomtracer is expected to meet. Direction and active investments live in [`VISION.md`](VISION.md), [`PRINCIPLES.md`](PRINCIPLES.md), and [`STRATEGY.md`](STRATEGY.md); planning lives in beads. This document is the bar.

Loomtracer is **memory for decisions**: what was decided, which options lost and why, and who decided, for one person and their agents ([`README.md`](README.md)). It must be designed and built to a standard that justifies people and autonomous agents trusting it as the source of truth for *what was decided, why, by whom, and what it depends on*.

---

## Build and test

A Rust crate on the one toolchain `rust-toolchain.toml` pins: install it once with `rustup toolchain install` in the repository (rustup also installs it on the first `cargo` run, but warns that this is deprecated), and CI and the Docker image build with the same version, so do not override it with `+nightly` or a different default. Before submitting a code diff, run what CI runs (`.github/workflows/ci.yml`):

```bash
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

- **One test:** `cargo test --locked --test <file> <test_name>`. Every file in `tests/` is its own binary; unit tests sit beside the code (`cargo test --locked --lib <name>`).
- **Golden snapshots** (`tests/snapshots/golden`, fixtures in `tests/fixtures`): `cargo test --test golden`; after an intended change, `cargo test --test golden -- --bless`.
- **Feature-gated code** is skipped by the commands above. `tests/migrate.rs` and the Postgres tests need `--features shared-backend-postgres` and `LOOMTRACER_TEST_POSTGRES_URL` pointing at a Postgres, with `LOOMTRACER_POSTGRES_POOL_SIZE=4` (the `rust-postgres` CI job). Without the URL most of them self-skip, so a green run proves nothing about Postgres. `--features tui` is a CI leg too. Kuzu (`graph-kuzu`) compiles C++ and runs only in the nightly workflow.
- **Python capture shipper:** `python3 -m unittest discover -s capture/tests`. `cargo test` runs it too (`tests/capture_shipper_python.rs`), and skips it with a printed reason when `python3` is not on `PATH`.
- **Install the CLI:** `make install` puts `loomtracer` in `~/.local/bin`, not `~/.cargo/bin`; check with `loomtracer --version`.

[`docs/QUALITY_GATES.md`](docs/QUALITY_GATES.md) is the full gate contract: which gates a diff owes, and the proof lines.

---

## Module map

- **Write layer:** `src/commands` (validates invariants, appends events), `src/events.rs`, `src/ingest.rs`, `src/ledger` (`sqlite`, `postgres`, `memory`).
- **Read layer:** `src/queries` (pure reads), `src/projector` (replays the ledger into the graph).
- **Layer 3:** `src/summarize.rs`, `src/scorer.rs`, `src/classifier.rs`, `src/quality_profile`, `src/importance.rs`, `src/possibly_related.rs`.
- **Surfaces:** `src/cli` (`args.rs` defines every flag, `run/` holds the handlers), `src/mcp.rs` (`tool_definitions()` is the one tool list; stdio leaves out `HTTP_ONLY_TOOLS`) with `src/mcp/core.rs`, where every tool's argument parser and logic live once and both transports (`src/mcp.rs` stdio, `src/api/mcp_http.rs` for `POST /mcp`) only adapt them, `src/api` (REST).
- **Outside `src/`:** `plugins/loomtracer-capture` and `plugins/loomtracer-context` are shell scripts and skills, exercised by `tests/*.rs`; `capture/` is the Python shipper. The CLI and MCP reference pages are regenerated in the site repo (`docs/QUALITY_GATES.md`, Reference Docs).

## Day-one gotchas

- `--actor` is a top-level flag and goes before the subcommand (`loomtracer --actor human:alice emit ...`); after it, clap rejects it. `--json`, `--loomtracer-dir`, `--tenant` and `--database-url` are global and work anywhere.
- A read, or a verb that acts on something already recorded, aimed at a folder with no ledger exits 2 and creates nothing. Check `--loomtracer-dir` first (README: "The default CLI stores events in SQLite").
- `LOOMTRACER_API_URL` (with `LOOMTRACER_API_KEY`, see `docs/KEYLESS_CAPTURE.md`) does not turn the CLI into a cell client: only `classify-queue` talks to the server. The verbs that open a local ledger (`query`, `emit`, ...) still use it, say so in their output, and exit 2 when the folder has none.

---

## MVP phase: break things freely, migrate the data (for now)

Loomtracer is in MVP and not deployed to users. **For now, prefer the correct design over compatibility.** Breaking changes to APIs, wire formats, schemas, and storage are fine. Do not add compatibility shims, dual-read paths, or deprecation periods to soften them.

The one hard rule: **existing ledger data is migrated, never lost.** A breaking change to the event or storage shape ships with the migration that carries existing data across, either a ledger replay or an explicit migration step. The ledger is the record of what was decided (§1, §4), so a change that strands or drops it is a defect, not a trade-off.

This holds for now, until the first real users.

---

## 1. What Loomtracer must do (the bar, not the feature list)

1. **Capture organizational decision memory** — not personal notes, not chat history, not task tracking. The unit of value is a recoverable, defensible decision with full provenance.
2. **Treat humans and agents as first-class peers.** Every action is taken by an `Actor`. Agents propose, accept, reject, supersede, and contest decisions on the same footing as humans. No second-class API.
3. **Survive disagreement.** Real organizations contain dissent. `contested` is a real status, not an error. Two actors disagreeing must be preserved, queryable, and resolvable — never silently dropped, overwritten, or auto-resolved.
4. **Preserve the why.** A decision without its rationale, considered options, and underlying evidence is worthless six months later. Capturing the *reasoning context* is non-negotiable.
5. **Make the past auditable.** Every node and edge in the graph traces to a specific event in the immutable ledger. Anyone — human or agent, internal or external — must be able to answer "how did we arrive at this?" deterministically.
6. **Make staleness visible.** When a hypothesis is refuted or a decision is superseded, every dependent decision must surface that fact in queries. Silent staleness is the worst failure mode.
7. **Forget responsibly.** Compactification removes noise, never signal. Main decisions and their why are preserved; only details that are safely redundant — or already captured elsewhere — are dropped. Forgetting is itself a tracked, reversible event.

---

## 2. Non-goals (do not let scope creep here)

Loomtracer is **not**:

- A project management tool, task tracker, or ticketing system.
- A chat archive or conversational memory store.
- An agent's private working memory.
- A general-purpose knowledge graph.
- A documentation system.
- A code-review tool.

If a proposed feature pulls toward any of the above, it is out of scope. The discipline is to stay a **decision graph**, not let it drift into "everything graph."

---

## 3. Architectural standards

The three-layer separation is **load-bearing** and not optional:

1. **Write/ingest layer** — validates invariants, appends events. Dumb-but-correct.
2. **Query/read layer** — pure reads. No LLMs, no ranking, no inference beyond explicit status derivation.
3. **Agentic suggestion/analysis layer** — all intelligence (compactification, similarity, ranking, suggestions) lives here and only here. It is built later and is **swappable** — the rest of the system must remain functional and correct without it.

A function that crosses layers is a bug. A query that calls an LLM is a bug. A write path that "helpfully" runs similarity to deduplicate is a bug. Smart features stay in layer 3 where they can be A/B-tested, replaced, or removed without touching ingest or queries.

---

## 4. Provenance and trust standards

- Every event has an `actor_id`. Anonymous writes are not allowed.
- Every node and edge in the graph carries `event_origin` — the ledger offset that created it.
- Optional cryptographic signing (Ed25519) is a real commitment for the shared-backend / multi-org direction, not a vague aspiration. When a multi-organization deployment lands, signing becomes mandatory.
- `contested` decisions are surfaced as a top-level status — never hidden in flags.
- A `Hypothesis` flipping to `refuted` propagates to every `Decision` that `PREMISED_ON` it. This propagation is visible in queries by default.

---

## 5. Scaling standards

Loomtracer must scale past the single-developer prototype to:

- **Multi-actor concurrent writes** without losing causality. Optimistic concurrency on supersessions; concurrent supersessions of the same target both survive and are flagged as such.
- **Multi-host queries** with bounded response sizes, pagination, and cycle protection. No query ever returns the whole graph.
- **Cross-organization federation** eventually. The ledger format and identity model must not preclude federation, even if it's not built day one.
- **Tens of thousands of decisions per organization** without query latency degrading beyond agent usability (p95 < 250ms target at multi-org scale).

The current local prototype is intentionally smaller, but no design choice today may foreclose any of the above. If a choice is convenient now but blocks scaling later, it's wrong.

---

## 6. Honesty standards

These apply to the system itself and to its contributors:

- **No silent truncation.** Queries that hit limits return `truncated: true` and a way to continue. Never a partial result that looks complete.
- **No silent disagreement collapse.** `contested` is a status, not a fallback.
- **No silent staleness.** Refuted hypotheses and superseded decisions are visible in queries, not hidden.
- **No invented confidence.** If layer 3 ranks or scores, the score's basis must be traceable. "Because the model said so" is not acceptable provenance.
- **No undocumented invariants.** Every rule that the commands layer enforces is written down and tested. Tribal knowledge is a bug.

---

## 7. Onboarding standard

A coding agent should be able to start using Loomtracer in **under five minutes**: install the CLI, run `loomtracer quickstart` (an isolated temporary ledger, one capture, queried back), then capture for real with `loomtracer emit decision.capture` and query it back. A capture names what the decision rests on and answers both floor questions (`--binds` or `--binds-nothing`, `--undo-cost` or `--cheap-to-undo`); the README Quickstart has a working call. If it takes longer than five minutes, the API is wrong.

Codex users can install the Loomtracer capture bundle from
`.agents/plugins/marketplace.json` or copy
`plugins/loomtracer-capture/skills/loomtracer-capture` into `$HOME/.agents/skills/`.
The bundle points Codex at `loomtracer emit decision.capture` and preserves the
actor format `agent:codex:<name>` (the name is stable under Gas City or
`LOOMTRACER_AGENT_SESSION`, otherwise the session id).

A new human contributor should be able to read this file, [`VISION.md`](VISION.md), [`PRINCIPLES.md`](PRINCIPLES.md), [`STRATEGY.md`](STRATEGY.md), and the currently-ready beads, and understand the project's direction and architecture in **under an hour**. If they can't, the documentation is wrong.

---

## 8. The contributor's commitment

Whether you're a human engineer or an AI agent contributing to Loomtracer, you commit to:

- Respect the three-layer boundary.
- Cut speculative features rather than add them.
- Make every change traceable to a decision (eventually recorded *in Loomtracer itself*).
- Surface disagreement and staleness, never suppress them.
- Run the mandatory [quality gate contract](docs/QUALITY_GATES.md) before
  submitting work, with explicit proof for each gate.
- Build the smallest correct thing that meets the bar, then extend.

This is the standard. Anything less and we are not building organizational memory — we are building yet another note-taking app.

---

## 9. Documentation boundary

Work tracking belongs in Beads, not ordinary repo files. Planning lives in
beads exclusively; the level-1 docs (`VISION.md`, `PRINCIPLES.md`,
`STRATEGY.md`) are the compass beads are judged against.

- Use Beads for task breakdowns, dependencies, follow-ups, routing, status, and implementation queues.
- Keep ordinary docs and source comments focused on stable product behavior, architecture, commands, contributor rules, and validation guidance.
- Do not add planning sections to docs as a substitute for beads. When a planning detail must be preserved, update or create the relevant bead or bead note instead.
- Beads live in the city's Dolt server, not in the repo: `.beads/issues.jsonl` is a stale export, never the live tracker. One branch per bead (`polecat/<bead-id>`, cut from `master`), a pull request against `master`, and the bead id in the commit message trailer, e.g. `(loomtracer-abc12)`. [`CONTRIBUTING.md`](CONTRIBUTING.md) has the steps.
