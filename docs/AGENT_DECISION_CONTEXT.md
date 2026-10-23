# Agent Decision Context

Status: shipped. Bead `hivemind-tenv.3`, parent epic `hivemind-tenv`
("Agent-fluent decision interaction").

`docs/AGENT_DECISION_CAPTURE.md` documents the write path: how an agent
records a new decision. This document covers the other half — how an agent
consults, verifies, and contests decisions that already exist — and is
honest about the fact that Loomtracer ships **two different interaction
models** for that, not one:

1. **MCP** (`src/mcp.rs`, native stdio and `POST /mcp`) — tools like
   `get_relevant_decisions`, `get_decision_context`, `get_supersession_chain`,
   `disagree_decision`, `supersede_decision`. The tools that act on one
   decision — `get_decision_neighborhood` (why), `get_decision_outcome`
   (verify), `get_supersession_chain`, `loomtracer_compact_view`,
   `disagree_decision`, `supersede_decision`, `move_decision`,
   `ground_decision` and `reaffirm_decision` — take a free-text `description`
   (+ optional `topic`) in place of an id, the same resolve-by-description
   primitive the CLI uses (see `docs/AGENT_FLUENT_QUERYING.md` §3.4).
   `get_decision`, `get_decision_context` and `score_decision` take only a
   `decision_id`: they are meant to be called once a decision is already
   resolved. MCP has no `--pick`/`#N` continuation (stateless per call): an
   ambiguous or not-found description comes back as a normal success reply
   with `data.outcome` set to `"ambiguous"` or `"not_found"`, never a JSON-RPC
   error, and the caller re-calls with `decision_id` once it has one. Pick MCP
   when an id is already known or the runtime prefers structured tool calls.
2. **The `loomtracer-context` Claude/Codex plugin** (this document,
   `plugins/loomtracer-context/`) — thin `scripts/*.sh` wrappers around the
   `hivemind-tenv.1`/`hivemind-tenv.2` CLI verbs
   (`query situational`, `query recall`, `query why`, `query verify`,
   `disagree`, `supersede`, `ground`, `reaffirm`, `reexamine`). None of them
   require a `decision_id` as primary input; free text (or a numbered `#N`
   handle from a prior ambiguous result) is the interface. It ships **no MCP
   server** — every command execs the `loomtracer` binary directly, on purpose,
   so it stays a genuinely different interaction model from MCP rather than
   an MCP client wearing a CLI costume.

Neither model is a strict subset of the other. MCP exposes a broader tool
surface (scoring, compaction, quality scanning) that the CLI-only plugin does
not wrap. The CLI-only plugin's `--pick N` / `#N` cross-invocation
continuation has no MCP equivalent (MCP's ambiguous replies still carry the
full candidate list in the same call — there is just no shorthand to
re-select one in a later call). Pick MCP when an id is already known or the runtime wants structured
tool calls; pick `loomtracer-context` when the agent knows only what it is
about to change or what it wants to ask, not an id, or wants `--pick`/`#N`
continuation across calls.

The HTTP REST API sits between the two: `GET /v1/decisions/{situational,
recall,why,verify}` are fluent (id-optional, `hivemind-ot72.4`) the same way
`why`/`verify` are over MCP, but `POST /v1/decisions/{id}/disagreements` and
`/{id}/supersessions` are still `decision_id`-path-only — HTTP has no fluent
write path yet, unlike MCP. See `docs/DEPLOYMENT.md`/`docs/SELF_HOSTING.md`
for the HTTP route list.

## The Agent's Lens

An agent about to change code does not know a decision's id. It knows:

- what it is about to touch (files, a diff, a branch) — **situational
  first**;
- a question in its own words — **recall**;
- a decision already in front of it that it wants to know more about —
  **why**, resolved by description;
- whether an old decision it is about to depend on still holds —
  **verify**, resolved by description;
- that it disagrees, or that a decision needs replacing — **disagree** /
  **supersede**, both resolved by description, both write verbs with a
  strict ambiguity gate (no write on an unresolved match). When the decision
  rests on a prior decision, the agent first runs **verify** and asks whether
  the disagreement is with the decision or with that premise; the reply then
  lists `raise_upstream`, the premises that still hold and how many other
  decisions rest on them (`docs/AGENT_FLUENT_QUERYING.md`);
- what an existing decision rests on, when it reads `nothing declared` —
  **ground**, resolved by description, a write verb with the same strict
  ambiguity gate, attributed to whoever adds it.

`loomtracer-context` maps each of these onto exactly one CLI verb. See
`docs/AGENT_FLUENT_QUERYING.md` for the resolver's design (deterministic
term-match + topic + recency ranking, no LLM — AGENTS.md Principles 1/7)
and `hivemind-tenv.2`'s situational-query design for the "what should I
know before I touch this" surface.

## Claude

This repository ships a Claude Code marketplace at
`.claude-plugin/marketplace.json` listing `loomtracer-context@loomtracer`
alongside `loomtracer-capture@loomtracer`.

Install from Claude Code:

```text
/plugin marketplace add alexknips/loomtracer
/plugin install loomtracer-context@loomtracer
/reload-plugins
```

The plugin includes:

- `/loomtracer-context:situational`, `/loomtracer-context:recall`,
  `/loomtracer-context:why`, `/loomtracer-context:verify`,
  `/loomtracer-context:disagree`, `/loomtracer-context:supersede`,
  `/loomtracer-context:ground`, `/loomtracer-context:reaffirm`,
  `/loomtracer-context:reexamine`, `/loomtracer-context:importance` — one slash
  command per CLI verb.
- The `loomtracer-context` skill, which teaches an agent when to consult
  Loomtracer before changing code, how to read an ambiguous result, and
  forbids inventing a `decision_id`.

No `mcpServers` key is declared — this plugin intentionally does not also
wire the MCP server. An agent that wants both interaction models installs
`loomtracer-capture` (which does wire MCP) alongside `loomtracer-context`.

## Codex

This repository ships `plugins/loomtracer-context`, exposed through
`.agents/plugins/marketplace.json` next to `loomtracer-capture`. Install from
a Loomtracer checkout by starting Codex in the repository, opening
`/plugins`, choosing `Loomtracer Plugins`, and installing `Loomtracer Context`.
Install from another machine by adding the repository marketplace first:

```bash
codex plugin marketplace add https://github.com/alexknips/loomtracer.git
```

For instruction-only use, copy `plugins/loomtracer-context/skills/loomtracer-context`
to `$HOME/.agents/skills/` and invoke `$loomtracer-context`.

## Backend Selection

Both interaction models — MCP and the `loomtracer-context` plugin's CLI
wrappers — read two independent settings from whatever environment launches
the agent; neither model sets either itself:

- **Ledger location** (SQLite mode): `./loomtracer/` by default, or
  `LOOMTRACER_DIR`/`--loomtracer-dir`/the plugin's `loomtracer_dir` option for
  another directory, including one on shared storage. A read verb (`recall`,
  `why`, `verify`, `situational`, `reexamine` and the rest) aimed at a directory
  that holds no ledger refuses (exit 2, naming the directory) and creates none,
  so a mistyped path is never read as "no decision found"; so does a verb that
  acts on a decision (`disagree`, `supersede`, `reaffirm` and the rest), which
  would otherwise make an empty ledger there and answer "no decision matches". A
  capture creates the ledger on first use.
- **Backend** (SQLite vs. Postgres): unset `LOOMTRACER_DATABASE_URL` uses the
  SQLite ledger above; setting it (with `LOOMTRACER_TENANT` to pick the
  tenant) makes `loomtracer mcp` and every `loomtracer query`/`disagree`/
  `supersede` invocation the plugin shells out to connect directly to that
  Postgres backend instead — `--loomtracer-dir`/`loomtracer_dir` is then
  ignored. Verb names, actor format, and the ambiguity gate are unaffected
  by this choice.
