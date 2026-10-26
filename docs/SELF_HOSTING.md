# Loomtracer Self-Hosting Guide

This guide covers deploying a self-hosted Loomtracer cell — the server binary
plus its Postgres backend — on infrastructure you control. One
`docker compose up` command brings up the full stack.

---

## Prerequisites

- Docker 24+ and Docker Compose v2 (`docker compose version`)
- Port 8080 available on the host (or set `LOOMTRACER_PORT`). It is published
  on `127.0.0.1` only — see [Exposing the cell deliberately](#exposing-the-cell-deliberately)
  to let other machines reach it

No other dependencies: Postgres runs as a companion container.

---

## Quick start

```bash
git clone https://github.com/alexknips/loomtracer
cd loomtracer

# 1. Create your environment file
cp .env.example .env

# 2. Set a strong admin key and Postgres password
#    (these are the only required secrets)
sed -i "s/change-me-before-production/$(openssl rand -hex 32)/g" .env

# 3. Start the cell (builds the image on first run, ~5 minutes)
docker compose up --build -d

# 4. Wait for healthy status
docker compose ps
```

The server is ready when `loomtracer` shows `healthy`:

```
NAME        STATUS                   PORTS
loomtracer    Up (healthy)             127.0.0.1:8080->8080/tcp
postgres    Up (healthy)
```

Verify:

```bash
curl http://localhost:8080/v1/health
# {"status":"ok"}
curl http://localhost:8080/v1/version
# {"version":"0.8.0+d2d203d1a2b3","cargo_version":"0.8.0","sha":"d2d203d1a2b3","ingest_project":true}
```

`/v1/health` and `/v1/version` are both unauthenticated liveness/build probes.
Like `loomtracer --version` locally, `/v1/version`'s `sha` is what the running
container was actually built from, not what a tag or `:latest` claims.
`ingest_project: true` says the cell files a `POST /v1/ingest` batch under the project it
names; the capture shippers read it before they send a span that names one, and send a cell
without it nothing (`docs/AGENT_DECISION_CAPTURE.md`).

> **Search note:** `GET /v1/decisions/search` and `GET /v1/decisions/recall` work on both
> backends — SQLite goes through FTS5, Postgres goes through a portable in-memory term
> matcher with the same ranking order (`docs/AGENT_FLUENT_QUERYING.md` §1.5). `GET
> /v1/decisions/relevant?topic=<topic>` remains available on both backends too, for exact
> topic-key filtering rather than free-text search.

---

## Auth story (no WorkOS required)

Self-hosted cells do not need WorkOS. A single cell can serve multiple users —
each user gets their own bearer token, and every captured decision is attributed
to that user's identity. The admin provisions users via the API; users connect
their agents with their personal token.

The server supports two auth modes depending on whether `LOOMTRACER_DATABASE_URL`
is set:

### Postgres mode (default in this compose) — multi-user quickstart

When `LOOMTRACER_DATABASE_URL` is set, the server uses per-user bearer tokens.

**Step 1 — provision your first tenant (one-time):**

```bash
# Replace <ADMIN_KEY> with the value of LOOMTRACER_ADMIN_KEY in your .env
curl -s -X POST http://localhost:8080/v1/tenants \
  -H "Authorization: Bearer <ADMIN_KEY>" \
  -H "Content-Type: application/json" \
  -d '{"tenant_id": "myorg", "display_name": "My Org"}' | tee /tmp/tenant.json
```

**Step 2 — add users:**

Each team member gets their own token. The `actor_id` recorded in every
decision they capture is derived from their email at token-creation time and
cannot be overridden by the caller.

```bash
# Add Alice (admin)
curl -s -X POST http://localhost:8080/v1/users \
  -H "Authorization: Bearer <ADMIN_KEY>" \
  -H "Content-Type: application/json" \
  -d '{"email": "alice@example.com", "display_name": "Alice", "role": "admin"}' \
  | tee /tmp/alice.json

# Add Bob (member)
curl -s -X POST http://localhost:8080/v1/users \
  -H "Authorization: Bearer <ADMIN_KEY>" \
  -H "Content-Type: application/json" \
  -d '{"email": "bob@example.com", "display_name": "Bob", "role": "member"}' \
  | tee /tmp/bob.json
```

Response for each user:

```json
{
  "user_id":      "3fa85f64-...",
  "email":        "alice@example.com",
  "display_name": "Alice",
  "role":         "admin",
  "token_id":     "8d3f...",
  "token_secret": "lt_tk_<64-hex>"
}
```

`token_secret` is shown **once only** — save it immediately. The server stores
only its hash.

**Step 3 — each user connects their agent:**

```json
{
  "mcpServers": {
    "loomtracer": {
      "url": "http://localhost:8080/mcp",
      "headers": {
        "Authorization": "Bearer lt_tk_<Alice's token>"
      }
    }
  }
}
```

Decisions captured by Alice are attributed to `human:alice@example.com`; Bob's
to `human:bob@example.com`. Actor identity is locked to the token — the server
ignores any `X-Loomtracer-Actor` header.

**Managing tokens:** the routes that take a `tenant_id` (`POST /v1/users`,
`POST /v1/agent-tokens`, `GET /v1/users`, `POST /v1/users/<USER_ID>/tokens`) answer
`404 unknown_tenant` for one that is not registered, and mint nothing.

```bash
# List all users
curl -s http://localhost:8080/v1/users \
  -H "Authorization: Bearer <ADMIN_KEY>"

# Mint an additional token for an existing user (e.g. new device)
curl -s -X POST "http://localhost:8080/v1/users/<USER_ID>/tokens?label=laptop" \
  -H "Authorization: Bearer <ADMIN_KEY>"

# Revoke a token (e.g. lost device)
curl -s -X DELETE "http://localhost:8080/v1/users/<USER_ID>/tokens/<TOKEN_ID>" \
  -H "Authorization: Bearer <ADMIN_KEY>"
```

**Checking who a token is:** `GET /v1/whoami` takes the same bearer token as every
other route and answers with the identity it carries — the UI's token sign-in uses
it to show "Signed in as …". It opens no ledger and writes no event.

```bash
curl -s http://localhost:8080/v1/whoami \
  -H "Authorization: Bearer lt_tk_<Alice's token>"
# → {"actor_id":"human:alice@example.com","tenant_id":"myorg"}
```

A missing, unknown or revoked token gets `401` with the usual error body. An agent
token answers `agent:<tool>:<name>`. The SQLite-mode shared `LOOMTRACER_API_KEY` and a
tenant's provisioning token from `POST /v1/tenants` are not people, so they answer
`service:api`: writes made with them carry no person's name. (The shared key takes the
actor from an `X-Loomtracer-Actor` header when the caller sends one.)

### SQLite mode (no Postgres)

If you remove the `LOOMTRACER_DATABASE_URL` line from `docker-compose.yml` the
server falls back to SQLite stored in the `loomtracer-data` volume. With
`LOOMTRACER_ADMIN_KEY` set, the same `POST /v1/users` endpoint works for
per-user SQLite tokens (default tenant `local`; omit `tenant_id` in body).

A SQLite-mode cell on v0.8.0 or earlier keeps its ledger in the volume `hivemind-data`, which the
current compose file does not mount: copy it across with the SQLite steps of
[Upgrading a cell from v0.8.0 or earlier](#upgrading-a-cell-from-v080-or-earlier), or the server
starts on an empty ledger.

For a single static key instead:

```bash
# In .env:
LOOMTRACER_API_KEY=your-secret-key
```

When both `LOOMTRACER_API_KEY` and `LOOMTRACER_DATABASE_URL` are unset the server
starts in **development mode** — all requests are accepted without a token.
That is only acceptable on loopback, so `loomtracer serve` refuses to start in
development mode on a non-loopback bind unless you pass
`--allow-unauthenticated-remote` (see
[Exposing the cell deliberately](#exposing-the-cell-deliberately)). In
SQLite mode set `LOOMTRACER_API_KEY` as well: per-user tokens alone do not close
the server to requests that send no token. An empty `LOOMTRACER_API_KEY` or
`LOOMTRACER_ADMIN_KEY` counts as unset.

---

## Configuration reference

All variables are read at startup. Compose reads them from `.env` in the
project root, or from the shell environment.

| Variable | Default | Description |
|---|---|---|
| `LOOMTRACER_DATABASE_URL` | *(unset)* | Postgres connection string. When set enables the multi-tenant Postgres backend. Also accepted as `--database-url` on the CLI and `loomtracer mcp` (flag beats the env var), not only by `serve` — see below. Unset = SQLite at `LOOMTRACER_DIR`. |
| `LOOMTRACER_DIR` | `/data` | Directory for the SQLite ledger (SQLite mode only). Mount a volume here. |
| `LOOMTRACER_PORT` | `8080` | Port the HTTP API listens on inside the container. |
| `LOOMTRACER_BIND` | `127.0.0.1` (`0.0.0.0` in the Docker image) | Address `loomtracer serve` binds (`--bind`). The image sets `0.0.0.0` so the published port can reach the server inside the container; exposure to the network is decided by `LOOMTRACER_PUBLISH_ADDR`, not by this. Not set in `.env` for compose — it is baked into the image. |
| `LOOMTRACER_PUBLISH_ADDR` | `127.0.0.1` | Host address compose publishes `LOOMTRACER_PORT` on (compose only). See [Exposing the cell deliberately](#exposing-the-cell-deliberately). |
| `LOOMTRACER_ADMIN_KEY` | *(unset)* | Bearer token for `POST /v1/tenants`, `POST /v1/users`, `GET /v1/users`, and token revocation. Required before provisioning tenants or users. |
| `LOOMTRACER_API_KEY` | *(unset)* | Static bearer token (SQLite mode only; forwarded by `docker-compose.yml`). With neither this nor `LOOMTRACER_DATABASE_URL` set the server runs in development mode (no auth), which refuses a non-loopback bind. |
| `ANTHROPIC_API_KEY` | *(unset)* | Enables the server-side Layer-3 classifier and scorer (Claude Haiku) on the SQLite backend only. Ignored when `LOOMTRACER_DATABASE_URL` is set, which is the compose default: the server logs one warning at start and classifies nothing. Optional; see [Keyless capture](#keyless-capture--no-anthropic_api_key-required). |
| `LOOMTRACER_CORS_ORIGINS` | *(unset)* | Comma-separated origins allowed for browser cross-origin requests. |
| `POSTGRES_PASSWORD` | `loomtracer` | Password for the bundled Postgres service. Change before production. |
| `LOOMTRACER_TENANT` | `local` | Default tenant. For the CLI and `loomtracer mcp` this selects the Postgres tenant scope when `LOOMTRACER_DATABASE_URL`/`--database-url` is also set. Not used by `serve`, which resolves the tenant per request from the caller's auth token instead. |

### CLI and MCP direct-Postgres mode

`LOOMTRACER_DATABASE_URL`/`--database-url` and `LOOMTRACER_TENANT`/`--tenant` are
read by every `loomtracer` subcommand, not only `serve` — including
`loomtracer mcp`, so a stdio MCP server can talk to this cell's Postgres
directly instead of a local SQLite file. Two facts worth knowing before
using this:

- The CLI and `loomtracer mcp` never provision tenants in the `lt_tenants`
  table that `POST /v1/tenants` manages (see above) — you must provision a
  tenant through the API first, then point `--tenant` at it.
- A binary built without the `shared-backend-postgres` Cargo feature fails
  with an explicit error if a database URL is configured — it never falls
  back to SQLite silently. The Docker image is built with this feature;
  `cargo install` / `make install` is not, so a locally installed `loomtracer`
  needs `--features shared-backend-postgres` to open a Postgres ledger.

See the "Using the CLI / MCP from local agents" section below for how to
reach this cell's Postgres from outside the compose network.

**WorkOS variables** (`WORKOS_DOMAIN`, `WORKOS_JWKS_URL`, `WORKOS_AUDIENCE`) enable
OIDC browser login. Self-hosted cells typically omit these; the token-based auth
above is the supported self-host path. If you do need OIDC, leave connector
config (`CONNECTOR_*`) as a placeholder — that surface is not yet finalised
(see hivemind-ld68).

---

## Keyless capture — no ANTHROPIC_API_KEY required

`ANTHROPIC_API_KEY` in the table above is **optional**. Without it the server
starts and operates correctly — the only thing that doesn't run is the
server-side background classifier (Worker B). Worker B runs only on the SQLite
backend: with `LOOMTRACER_DATABASE_URL` set, as in the default compose stack, it
never starts, a key included, and the server logs one warning saying the key is
ignored. On that stack classification is always Worker A.

**If you omit `ANTHROPIC_API_KEY`**, classification still happens via
**Worker A**: the `loomtracer-capture` plugin ships a
`/loomtracer-capture:classify-queue` command that drains the same pending queue
using your agent's subscription seat. No key, no extra configuration.

### Keyless walkthrough

1. **Start the cell** without an API key (skip that line in `.env`).

2. **Install the capture plugin** from Claude Code:

   ```text
   /plugin marketplace add alexknips/loomtracer
   /plugin install loomtracer-capture@loomtracer
   /reload-plugins
   ```

3. **Capture a decision** in-session:

   ```text
   /loomtracer-capture:capture "Use Postgres for the shared backend" \
     --kind decision \
     --title "Use Postgres for the shared backend" \
     --rationale "SQLite WAL mode does not scale across concurrent writers" \
     --topic-keys infrastructure,storage \
     --options "SQLite,Postgres" \
     --chose "Postgres" \
     --rests-on-assumption "More than one agent writes to the shared ledger at once" \
     --binds "Every agent and the shared ledger move to Postgres" \
     --undo-cost "Moving back means exporting the ledger out of Postgres and replaying it"
   ```

   This writes a **local ledger** (the `loomtracer/` folder of the project Claude Code runs in)
   and does not contact the cell, with `LOOMTRACER_API_URL` set or not. To record the decision in the
   cell, post it to `POST /v1/decisions` with your token: see
   [`KEYLESS_CAPTURE.md` Step 3](KEYLESS_CAPTURE.md#step-3--capture-a-decision-in-session).

4. **Drain the classification queue** after your session:

   ```text
   /loomtracer-capture:classify-queue
   ```

   The command classifies pending batches using your subscription model and
   writes `IngestBatchClassified` events. Pass `--limit N` to cap the run.

5. **Query back**. `loomtracer query` reads a local ledger, so from the project folder step 3
   wrote to it lists that capture:

   ```bash
   loomtracer query recent_decisions --since 1h --limit 5
   ```

   A decision posted to the cell is read from the cell's HTTP API instead
   (`GET /v1/decisions/search`, see [`KEYLESS_CAPTURE.md` Step 5](KEYLESS_CAPTURE.md#step-5--query-it-back)).

See [`docs/KEYLESS_CAPTURE.md`](KEYLESS_CAPTURE.md) for the full walkthrough
and a comparison of Worker A vs Worker B.

---

## E2E verification

Run these checks after provisioning your first tenant to confirm all layers work.

```bash
export HM_URL=http://localhost:8080
export HM_TOKEN="lt_tk_..."   # a user token from POST /v1/users

# 1. Health
curl -s $HM_URL/v1/health
# → {"status":"ok"}

# 2. Capture a decision (actor_id is resolved from your token — no need to supply it)
curl -s -X POST $HM_URL/v1/decisions \
  -H "Authorization: Bearer $HM_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "title": "Use Postgres for shared-backend storage",
    "rationale": "WAL mode does not scale across writers.",
    "topic_keys": ["infrastructure"],
    "chosen_option_label": "postgres",
    "options": [
      {"label": "postgres", "description": "r2d2 connection pool"},
      {"label": "sqlite",   "description": "WAL mode, single writer"}
    ],
    "grounding": [
      {"kind": "evidence", "content": "SQLite allows one writer at a time", "source": "https://www.sqlite.org/lockingv3.html"}
    ],
    "binds": {"level": "something", "statement": "Every agent and the shared ledger move to Postgres"},
    "undo_cost": {"level": "costly", "statement": "Moving back means exporting the ledger out of Postgres and replaying it"}
  }' | tee /tmp/decision.json
# → {"decision_id":"decision-...","option_ids":[...],...}

# 3. Query it back — full-text search/recall work on both backends (see the
# Search note above); topic filtering also works on both:
curl -s "$HM_URL/v1/decisions/relevant?topic=infrastructure" \
  -H "Authorization: Bearer $HM_TOKEN" | python3 -m json.tool | head -20

# 3b. Fluent (no-id) follow-up — "why was this decided?", by description:
curl -s "$HM_URL/v1/decisions/why?description=Postgres%20for%20shared-backend" \
  -H "Authorization: Bearer $HM_TOKEN" | python3 -m json.tool | head -20

# 4. MCP-over-HTTP endpoint reachable
curl -s -o /dev/null -w "%{http_code}" -X POST $HM_URL/mcp \
  -H "Authorization: Bearer $HM_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}'
# → 200
```

**Layer-3 classifier** (SQLite mode with `ANTHROPIC_API_KEY` set; a Postgres
server ignores the key): after step 2, wait a few seconds then re-fetch the
decision — `topic_keys` should be populated automatically.

**Fluent (no-id) routes:** step 3b above is one of four GET routes that
resolve a decision without an id — `situational`, `recall`, `why`, `verify`.
See `docs/DEPLOYMENT.md`'s "Fluent (no-id) read routes" table for the full
param list; all four work identically on both backends. `disagree`/
`supersede` remain `decision_id`-path-only over HTTP (not fluent).

---

## Configuring an agent to use MCP-over-HTTP

Point your Claude Code (or any MCP-compatible agent) at the `/mcp` endpoint:

```json
{
  "mcpServers": {
    "loomtracer": {
      "url": "http://localhost:8080/mcp",
      "headers": {
        "Authorization": "Bearer lt_tk_<your-token-secret>"
      }
    }
  }
}
```

Use the `token_secret` value returned by `POST /v1/users` as the bearer token.
The token format is `lt_tk_<64-hex>` (shown once at creation time).

**Actor identity differs from the REST endpoints above.** `POST /v1/decisions`
and friends always attribute writes to the bearer token's own bound identity
and ignore any client-supplied actor — that is what "Actor identity is locked
to the token" means in the auth story above. The `/mcp` tool-call surface
defaults to the bearer token's actor for an agent token: a write with no
`actor_id` under a token minted at `POST /v1/agent-tokens` is recorded as
`agent:claude:<role>`, whether or not the client sends an `Mcp-Session-Id` header
(Claude Code always does). Under a person's credential (a user token or a WorkOS
JWT) the default is the MCP session's agent, `agent:mcp-http:<session>`, and not
the person: an agent's write must not be recorded as `human:<email>`, where a
chosen option would self-accept as that person. On top of that, `capture_decision`,
`capture_evidence`, `capture_hypothesis`, `disagree_decision`, `supersede_decision`,
`link_supersession`, and `move_decision` all accept an optional `actor_id` argument that, when present,
overrides the default for that one call
(`src/api/mcp_http.rs::mcp_resolve_actor`; verified against
a running cell — a `tools/call capture_decision` with
`"actor_id": "agent:claude:some-session"` produces a `PROPOSED_BY` edge to
that actor, not to the default). This lets one shared per-role
token serve many distinct callers with correct per-caller attribution — e.g.
a fleet of agent sessions sharing one token, each passing its own
`agent:<tool>:<session>` as `actor_id` — without minting a token per session.
A token per agent, with no `actor_id` passed, is the simpler setup when each
agent has its own identity.

This is a convenience, not a security boundary: `actor_id` here is
**caller-asserted, not verified**. Any holder of the token can claim any
actor string, including another human's or agent's identity — the same trust
level as the CLI's free-text `--actor` flag. Only share one token across
callers you already trust not to misattribute their own writes; mint a
personal per-user token (Step 2 above) whenever that trust does not hold.

---

## Using the CLI / MCP from local agents

The compose stack above publishes only port 8080, on `127.0.0.1`
(`docker-compose.yml`); Postgres is reachable only on the compose network. An agent that wants to
run the `loomtracer` CLI or `loomtracer mcp` with `--database-url` /
`LOOMTRACER_DATABASE_URL` against the cell's own Postgres — instead of going
through the HTTP API — needs one of the following.

### Option A — publish Postgres on loopback

Bring the stack up with the local-agents override, which adds a
loopback-only port mapping for the `postgres` service
(`docker-compose.local-agents.yml`; not applied by default):

```bash
docker compose -f docker-compose.yml -f docker-compose.local-agents.yml up -d
```

Then point the CLI or `loomtracer mcp` at it directly:

```bash
LOOMTRACER_DATABASE_URL=postgres://loomtracer:<POSTGRES_PASSWORD>@127.0.0.1:5432/loomtracer \
LOOMTRACER_TENANT=<tenant> \
loomtracer query situational --diff
```

Set `POSTGRES_PORT` in `.env` if `5432` collides with a Postgres instance
already running on the host.

**This puts the database credential on the agent's machine.** The
`POSTGRES_PASSWORD` from `.env` is a full read/write credential to every
tenant's data on the cell, not scoped to one tenant. Only use this option on
a host you trust with that credential. A CLI that talks to the server's HTTP
API instead of the database directly would avoid this, but that is a
separate, larger change — file it only if this credential story doesn't
work for your setup.

### Option B — run the CLI inside the container

No port publishing needed. Exec into the running `loomtracer` service container, which
already has `LOOMTRACER_DATABASE_URL` pointed at the compose network address:

```bash
docker compose exec loomtracer loomtracer --tenant <tenant> query situational --diff
```

This is the safer default — no database credential ever leaves the
container.

---

## Moving an existing ledger into the cell

`loomtracer migrate` copies a local SQLite ledger into a cell event for event. Over
HTTP it needs only the cell's admin key, not its database password:

```bash
# The cell's LOOMTRACER_ADMIN_KEY, in a file only you can read (never on the command line).
install -m 600 /dev/null ~/.config/loomtracer/admin-key
printf '%s' "$LOOMTRACER_ADMIN_KEY" > ~/.config/loomtracer/admin-key

# 1. Dry run: writes nothing, prints how many events would move and how many are already there.
loomtracer migrate --from ./loomtracer --to-url https://cell.example.com \
  --admin-key-file ~/.config/loomtracer/admin-key --to-tenant <tenant> --dry-run

# 2. The move. Drop --dry-run; everything else stays the same.
loomtracer migrate --from ./loomtracer --to-url https://cell.example.com \
  --admin-key-file ~/.config/loomtracer/admin-key --to-tenant <tenant>
```

`--from` defaults to `--loomtracer-dir`, `--tenant` names the source tenant (default `local`),
and `--to-tenant` is always required: a move into the wrong tenant cannot be undone, because
the cell is append-only.

What a move guarantees:

- **Each event keeps what it says.** The uuid, actor, source, source ref, correlation id,
  payload and recorded time are carried over unchanged. Nothing is stamped with the time of
  the move or the identity of whoever ran it.
- **Causation follows the cause.** The cell numbers events itself, so on a tenant that
  already holds events a source event lands at a different `event_id` than it had. Each
  causation link is carried as the *uuid* of its cause and renumbered to wherever the cause
  landed.
- **A draft keeps the events that act on it.** A decision the classifier drafted is named
  `capture:<offset of its batch event>:<index>`, and its acceptance, retitle, score and links
  carry that id. The move sends each such id with the batch event's *uuid* in place of the
  offset and the cell writes the offset where that batch event landed, so the moved draft
  keeps its acceptance and its links and no draft already in the tenant gains them. This
  covers every payload string that is exactly a draft id (or its `:opt:` option node). A
  `suggestion.surfaced` on a draft keeps the `finding_id` it was surfaced under, a hash that
  includes the old id, so that finding can be surfaced again for the moved draft.
- **A re-run writes nothing.** An event is skipped when the tenant already holds its uuid,
  so an interrupted move can simply be run again. An event already in the tenant is left as
  it is, including a causation id an earlier copy wrote.
- **Parity is checked by uuid.** After a real move every source event uuid is looked up in
  the destination; the report's `parity_check` gives `present_in_destination` and `missing`,
  and the command fails if any is missing. Counts alone would prove nothing on a tenant that
  held events before.
- **Refusals happen before the first event is sent:** a causation link whose cause is not an
  earlier event of the source ledger, a draft id whose batch is not an earlier event of the
  source ledger, a draft id sent to the cell by its offset instead of its batch event's uuid
  (the cell cannot tell which batch it meant), a `notification.sent` or `blocker.resolved`
  event whose payload names event ids (those numbers cannot be carried to another ledger), and
  an empty source (a wrong `--from` or `--tenant`). An unknown destination tenant is a 404 and a wrong
  admin key a 401; neither writes anything.

Rehearse against a scratch server first, with the same source, and compare the counts:

```bash
LOOMTRACER_ADMIN_KEY=scratch-key loomtracer serve --loomtracer-dir /tmp/scratch-cell --port 18732 &
printf 'scratch-key' > /tmp/scratch-admin-key
loomtracer migrate --from ./loomtracer --to-url http://127.0.0.1:18732 \
  --admin-key-file /tmp/scratch-admin-key --to-tenant local
```

The client calls `POST /v1/ledger/replay`: `Authorization: Bearer <LOOMTRACER_ADMIN_KEY>`, body
`{"tenant_id": "...", "dry_run": false, "events": [...]}` with at most 1000 events, each
`{event_uuid, type, actor_id, source, source_ref, correlation_id, causation_event_uuid,
payload, ts}` (`ts` is required). The answer is `{received, new_events, already_present}`. A
batch whose causation link names an event that is neither in the tenant nor earlier in the
batch is refused whole.

`loomtracer migrate --to <postgres-url>` writes straight to the database with the same rules,
but it needs the database credential and a binary built with the `shared-backend-postgres`
feature; prefer `--to-url`.

---

## Projects for a city of agents

A cell that serves a Gas City, or any fleet of agents working in several repos,
groups its decisions by project inside one tenant (see
[MULTI_TENANCY.md](MULTI_TENANCY.md#projects-inside-a-tenant)). The registry
lives in that tenant's ledger, so register it against the cell's ledger — with
`--database-url`/`--tenant` (Option A) or from inside the container (Option B):
one project per rig (`loomtracer project register <rig>`), a `city` project every
rig is `part_of` (`loomtracer project link --from <rig> --to city --kind
part_of`), `depends_on` links for the real dependencies between rigs, and a rig
anchor so a session's `GC_RIG` names its project (`loomtracer project anchor
--handle <rig> --kind rig --value <rig>`). Commit a one-line `.loomtracer-project`
file naming the handle at a repo's root (or in a subfolder for a sub-project) to
attach the folder as well; the CLI does not write it. Agents whose CLI or stdio
MCP server runs with `--project-from-context` (the capture plugin does) then file
each capture under its rig's or folder's project and say how, and a capture that
names or finds no project falls into the actor's personal project with a notice.
A session in a rig that no project in the ledger is anchored to is refused
instead: it is pointed at the wrong ledger, and the refusal names the rig and the
ledger it would have written to. Each project's topic keys are declared too
(`loomtracer project declare-topic <rig> --in-use` adopts what a rig's existing
decisions use); a capture under a registered project may only use declared keys
and says so with `--declare-topic` when it adds one.
Those agents read the registry, their working directory, `GC_RIG`, and their git
diff from the machine the CLI runs on, so they need Option A (the CLI on the
agent's own machine, pointed at the cell's Postgres); Option B runs the CLI
inside the container, where none of those belong to the agent, so pass
`--project` there. Over HTTP the server has no view of the caller's working
directory: `/mcp` takes the project as an argument, and the REST
`POST /v1/decisions` route takes none, so it files into the actor's personal
project.

---

## Exposing the cell deliberately

Out of the box the cell is reachable only from the machine it runs on:

- `docker-compose.yml` publishes `127.0.0.1:8080`, not every interface.
- `loomtracer serve` binds `127.0.0.1` unless told otherwise (`--bind` /
  `LOOMTRACER_BIND`). The Docker image sets `LOOMTRACER_BIND=0.0.0.0` *inside* the
  container so the published port can reach it; what the network can see is
  decided by the `ports:` mapping.
- On the Postgres backend, reads and writes of the ledger need a bearer token
  (the Slack routes need a valid Slack signature instead); `/v1/health` and
  `/v1/version` are open probes.

To let other machines in, choose one:

- **Reverse proxy on the same host (recommended).** Keep the loopback publish
  and point Caddy, nginx or Traefik at `127.0.0.1:8080`; terminate TLS there.
- **A private-network address, such as a tailnet IP.** Set
  `LOOMTRACER_PUBLISH_ADDR=<that interface's address>` in `.env` and run
  `docker compose up -d`.
- **Every interface.** `LOOMTRACER_PUBLISH_ADDR=0.0.0.0`. Docker inserts its own
  firewall rules for published ports, which bypass host firewalls such as
  `ufw` — only do this behind a network-level firewall or security group.

Agents on other machines then use that address in place of
`http://localhost:8080` (for example in the `/mcp` config below).

**Unauthenticated servers.** `loomtracer serve` without `LOOMTRACER_API_KEY` and
without `LOOMTRACER_DATABASE_URL` runs in development mode and accepts every
request without a token. It refuses to start that way on a non-loopback
`--bind`. `--allow-unauthenticated-remote` overrides the refusal for the rare
case where the network itself is the boundary; the server then logs a warning
naming the flag on every start. The startup log always states the auth mode
and the address it bound.

---

## Production checklist

- [ ] Set `LOOMTRACER_ADMIN_KEY` to a strong random value (`openssl rand -hex 32`)
- [ ] Set `POSTGRES_PASSWORD` to a strong random value
- [ ] Place a TLS-terminating reverse proxy (Caddy, nginx, Traefik) in front
      of the cell, proxying to `127.0.0.1:8080` — the server speaks plain HTTP
- [ ] Keep `LOOMTRACER_PUBLISH_ADDR` at its `127.0.0.1` default unless you have
      deliberately chosen another address
- [ ] Back up the `postgres-data` Docker volume regularly
- [ ] Monitor `GET /v1/health` with an external uptime checker
- [ ] Set `restart: always` in compose for unattended recovery

---

## Upgrading

```bash
./scripts/cell-update.sh [ref]   # default ref: origin/master
```

Builds the server image from the given ref (default `origin/master`), tags
it by commit sha — never `:latest`, which only moves when a `v*` tag is
pushed (see `.github/workflows/release.yml`) and can otherwise sit behind
master for weeks — restarts the `loomtracer` service with it, and waits for
`/v1/health` before declaring success. Confirm what's actually running with
`curl http://localhost:8080/v1/version`.

**The update stays.** Before the restart, the script writes the tag it built
as `LOOMTRACER_IMAGE=loomtracer:<sha>` in the compose project's `.env`.
Anything that later runs `docker compose up -d` there — a systemd timer or
cron job that keeps the cell up, or you — recreates the container on the
image you deployed, not back on `:latest`. An existing `.env` keeps its mode
and every other line; a new one is created owner-only (`0600`).

The pin is kept only once the new container is healthy and `/v1/version`
reports the sha. If either fails, the script exits non-zero and puts `.env`
back exactly as it found it. The container it started keeps running so you
can look at it, and anything reconciling with `docker compose up -d` returns
the cell to the previous image.

A cell kept in its own directory, with override files, tells the script
which files it is reconciled with:

```bash
LOOMTRACER_COMPOSE_FILE=/srv/cell/docker-compose.yml:/srv/cell/docker-compose.local-agents.yml \
  ./scripts/cell-update.sh
```

`LOOMTRACER_COMPOSE_FILE` lists every compose file, colon-separated as
`COMPOSE_FILE` is (default `docker-compose.yml`); the restart uses all of
them, and `.env` is the one next to the first, where `docker compose` looks
for it. Set `LOOMTRACER_ENV_FILE` to pin in a different file; the script hands
that file to `docker compose --env-file` as well.

To roll back, run the script for the earlier ref (`./scripts/cell-update.sh
<previous sha>`): it rebuilds that image and pins it the same way. To go back
to the release images, delete the `LOOMTRACER_IMAGE` line from `.env`, then
`docker compose pull && docker compose up -d`.

Equivalent by hand, without the health check or sha tagging:

```bash
git pull
docker compose build --no-cache
docker compose up -d
```

Note: with `image:` and `build:` both set in `docker-compose.yml`, a bare
`docker compose up -d` reuses whatever image already carries that tag
locally rather than pulling — so `docker compose build` must run first, or
this silently keeps serving the old container. If `.env` pins
`LOOMTRACER_IMAGE` (the script does), that tag is the pinned one, and
`docker compose build` would overwrite it with the fresh build; delete the
pin before updating by hand.

Postgres schema migrations run automatically at startup.

### Upgrading a cell from v0.8.0 or earlier

The new name reached a cell in two steps. **v0.8.0**, the published release (GitHub's "Latest",
`1296a475`), renamed everything you type: the binary, the `LOOMTRACER_*` settings, the image
`ghcr.io/alexknips/loomtracer`, the `lt_tk_` token prefix. It did not rename what a cell stores:
a cell on v0.8.0 still has the tables `hm_*`, the Postgres database and role `hivemind`, the
compose service `hivemind` and the volume `hivemind-data`. The files of this repository, and the
first release after v0.8.0, rename those too: the tables `hm_*` are `lt_*`, the database and role
are `loomtracer` (the default password too, which a renamed role does not take on its own: step 2
of Postgres mode sets it), the compose service is `loomtracer` and the volume is `loomtracer-data`.

This section is for every cell that still stores the old names, a cell on v0.8.0 as much as one on
v0.7.0 or earlier. If `docker compose ps` in the cell's directory lists a service `hivemind`, it is
yours. Starting the new compose file on such a cell would make a second, empty stack beside it, and
the new server refuses to start on a database that still has the old tables. Rename them first, in
this order, from the cell's directory, with the **old** compose files still in place. Every step
but the last keeps the data where it is, and the rename of the tables and of the project handle can
be rehearsed on a copy first.

The cell's settings differ by release:

- A cell on **v0.8.0** already has its settings under their `LOOMTRACER_*` names
  (`LOOMTRACER_API_KEY`, `LOOMTRACER_ADMIN_KEY`, ...) and pulls `ghcr.io/alexknips/loomtracer`.
  Nothing below renames a setting for it.
- A cell on **v0.7.0 or earlier** has `HIVEMIND_*` settings and pulls `ghcr.io/alexknips/hivemind`.
  It also takes the install steps of the v0.8.0 entry in [CHANGELOG.md](../CHANGELOG.md) (the
  ledger folder, the tokens, its own settings).

**The steps need an image newer than v0.8.0.** `loomtracer migrate-storage` and
`loomtracer project rename` are not in v0.8.0, and a v0.8.0 server does not read the renamed
tables. Release images are published only when a `v*` tag is pushed, so until a release newer than
v0.8.0 exists, `ghcr.io/alexknips/loomtracer:latest` is v0.8.0's image: build the image from the
files you pulled (`docker compose build`, in step 3 below) and do not `docker compose pull`.

Which steps are yours depends on where the cell keeps its ledger. A cell on Postgres (the default
compose file) follows **Postgres mode**. A cell in [SQLite mode](#sqlite-mode-no-postgres), with
the `LOOMTRACER_DATABASE_URL` line removed from `docker-compose.yml`, keeps its whole ledger in
the old `hivemind-data` volume, which the new compose file does not mount: it follows **SQLite
mode**, further down, and does not remove that volume until its last step says so.

**A cell from v0.7.0 or earlier has tokens whose prefix needs changing, and no step below can do
that for you; a cell on v0.8.0 has nothing to change.** A token from `POST /v1/users`,
`POST /v1/agent-tokens` or `POST /v1/tenants` started `hm_tk_` up to v0.7.0 and starts `lt_tk_`
from v0.8.0 on, and a server from v0.8.0 on answers `401` to a token that still starts `hm_tk_`.
So on a cell from v0.7.0 or earlier, whoever holds a token changes `hm_tk_` to `lt_tk_` where they
keep it (a shell variable, the `Authorization: Bearer` line of an MCP config, `LOOMTRACER_API_KEY`
on a machine that captures into the cell). Nothing was revoked or lost: the server keeps a hash of
the part after the prefix, `migrate-storage` carries that row across, and `lt_tk_` followed by the
same 64 hex characters is the same token:

```bash
NEW_TOKEN="lt_tk_${OLD_TOKEN#hm_tk_}"
```

On a cell on v0.8.0 the tokens already start `lt_tk_` (the v0.8.0 install steps changed the older
ones, and the cell minted the rest under the new prefix). Step 6 of each mode checks one either
way. The shared `LOOMTRACER_API_KEY` of SQLite mode is a string you chose, not a token with a
prefix: it only changes its name in `.env`.

#### Postgres mode

1. **Stop the server, keep Postgres, and take a backup.**

   ```bash
   docker compose stop hivemind
   docker compose exec -T postgres pg_dump -U hivemind -d hivemind > cell-before-storage-rename.sql
   ```

2. **Rename the database and the role.** A role cannot rename the session it is logged in on, so
   a temporary superuser does it (Postgres in the container trusts its own socket, so no password
   is asked):

   ```bash
   docker compose exec -T postgres psql -U hivemind -d postgres -v ON_ERROR_STOP=1 \
     -c 'CREATE ROLE lt_rename SUPERUSER LOGIN'
   docker compose exec -T postgres psql -U lt_rename -d postgres -v ON_ERROR_STOP=1 \
     -c 'ALTER DATABASE hivemind RENAME TO loomtracer' \
     -c 'ALTER ROLE hivemind RENAME TO loomtracer'
   docker compose exec -T postgres psql -U loomtracer -d postgres -v ON_ERROR_STOP=1 \
     -c 'DROP ROLE lt_rename'
   ```

   If the second command prints `MD5 password cleared because of role rename`, set the password
   again with `\password loomtracer` in `psql`. A SCRAM password (the default since Postgres 14)
   survives the rename.

   **A cell whose `.env` has no `POSTGRES_PASSWORD` needs one more command.** It has been running
   on the password v0.8.0's compose file defaults to, `hivemind`, and the renamed role keeps it.
   The new compose file connects as `loomtracer` with `${POSTGRES_PASSWORD:-loomtracer}`, so
   without a change `migrate-storage` (step 4) and the server (step 6) are refused by Postgres.
   Give the role the password the new file sends (or one you choose, then put the same in `.env` as
   `POSTGRES_PASSWORD=...`):

   ```bash
   docker compose exec -T postgres psql -U loomtracer -d postgres -v ON_ERROR_STOP=1 \
     -c "ALTER ROLE loomtracer PASSWORD 'loomtracer'"
   ```

   A cell that sets `POSTGRES_PASSWORD` in `.env` keeps it, and has nothing to do here. If a later
   step ends with `password authentication failed for user "loomtracer"`, the role's password is
   not the one `.env` (or the default) gives the server: run this command with the password you
   mean. Putting `POSTGRES_PASSWORD=hivemind` in `.env` instead also works.

3. **Switch to the new compose files** (`git pull`, or copy the new `docker-compose.yml` and keep
   your overrides, renaming the `hivemind` service in them to `loomtracer`), then build the image
   they run with `docker compose build`: the next step needs a `loomtracer` that has
   `migrate-storage`, which v0.8.0's image does not. Compose recreates the `postgres` container
   because its settings changed; its data is in the `postgres-data` volume and stays. The
   `hivemind-data` volume holds nothing in Postgres mode (check:
   `docker run --rm -v <project>_hivemind-data:/d:ro alpine ls -A /d` prints nothing), so the new
   `loomtracer-data` volume starts empty. If that check lists `ledger.sqlite`, the cell is in
   SQLite mode and the volume holds its ledger: stop here and follow
   [SQLite mode](#sqlite-mode-a-ledger-in-the-hivemind-data-volume) below instead.

4. **Rename the tables**, keeping every row. `--dry-run` runs it and rolls it back, printing the
   tables and the row counts a real run would produce:

   ```bash
   docker compose run --rm --no-deps loomtracer migrate-storage --dry-run
   docker compose run --rm --no-deps loomtracer migrate-storage
   ```

   It is one transaction that checks every table holds the same number of rows before and after
   and changes nothing if one does not. Running it again says there is nothing to rename.

5. **Rename the project handle** if a project is still registered under `hivemind`, once per
   tenant. This registers the new handle, moves the old project's anchors and links to it,
   declares its topic keys and moves each of its decisions, all as recorded events (a
   `decision.moved` per decision), so nothing is rewritten and the same command with the two
   handles swapped undoes it. Put the same handle in the `.loomtracer-project` file of every
   repository that used the old one:

   ```bash
   docker compose run --rm --no-deps loomtracer --actor human:you --tenant local \
     project rename hivemind loomtracer --display-name Loomtracer
   ```

   The old handle stays registered with nothing left under it.

6. **Start the cell** and remove what the old stack left behind:

   ```bash
   docker compose up -d --remove-orphans
   curl http://localhost:8080/v1/health
   ```

   Check that `/v1/version` and a known query answer as before. In Postgres mode the static key
   is ignored, so a query is asked with a token: use a user, agent or tenant token issued before
   the upgrade (on a cell from v0.7.0 or earlier with its prefix changed to `lt_tk_`, see above),
   and it answers who it is:

   ```bash
   curl -s http://localhost:8080/v1/whoami -H "Authorization: Bearer lt_tk_<64-hex of an old token>"
   ```

   A token with `hm_tk_` still in front answers `401 invalid or missing bearer token`: its holder
   has not changed the prefix yet. Then remove the old volume, which step 3 showed empty (never in
   SQLite mode, where it holds the ledger):

   ```bash
   docker volume rm <project>_hivemind-data
   ```

   To go back, restore `cell-before-storage-rename.sql` into a fresh Postgres and run the old image.

#### SQLite mode: a ledger in the `hivemind-data` volume

The ledger is the file `ledger.sqlite` in the `hivemind-data` volume (beside it the `slack-app`
folder, if the cell runs the Slack app). The new compose file mounts `loomtracer-data` at `/data`
instead, so a server started on it, without these steps, answers on a new, empty ledger: the graph
is empty and recall finds nothing, while the real ledger sits untouched in the old volume. Carry
it across, rename its tables (`hm_users` and `hm_tokens`), and remove the old volume last. Run it
from the cell's directory, the first step with the **old** compose files still in place:

1. **Count the decisions, stop the server and take a backup.** The count is what the new server
   has to answer in step 6. `LOOMTRACER_API_KEY` below is the key the old server runs with: set it
   in your shell from the cell's `.env`, where a v0.8.0 cell holds it as `LOOMTRACER_API_KEY` and a
   cell on v0.7.0 or earlier as `HIVEMIND_API_KEY`:

   ```bash
   curl -s -H "Authorization: Bearer $LOOMTRACER_API_KEY" http://localhost:8080/v1/graph \
     | jq '(.data // .).decisions | length'
   docker compose stop hivemind
   docker run --rm -v <project>_hivemind-data:/d:ro -v "$PWD":/backup alpine \
     tar czf /backup/cell-before-storage-rename.tgz -C /d .
   ```

2. **Switch to the new compose files** (`git pull`, or copy the new `docker-compose.yml` and keep
   your overrides, renaming the `hivemind` service in them to `loomtracer`), and remove the
   `LOOMTRACER_DATABASE_URL` line again, as for [SQLite mode](#sqlite-mode-no-postgres). Then
   build the image they run with `docker compose build`: step 4 needs a `loomtracer` that has
   `migrate-storage`, which v0.8.0's image does not. `.env` must hold `LOOMTRACER_API_KEY`: SQLite
   mode without the key refuses to start. A v0.8.0 cell has it already. A cell on v0.7.0 or
   earlier renames `HIVEMIND_API_KEY` to it (and every other `HIVEMIND_*` setting to its
   `LOOMTRACER_*` name).

3. **Carry the ledger into the new volume.** This copies and leaves `hivemind-data` as it was.
   Compose makes `<project>_loomtracer-data` on the way, and the server is not started:

   ```bash
   docker compose run --rm --no-deps -v <project>_hivemind-data:/old:ro \
     --entrypoint cp loomtracer -a /old/. /data/
   docker compose run --rm --no-deps --entrypoint ls loomtracer -A /data   # lists ledger.sqlite
   ```

4. **Rename the tables**, keeping every row (the same command as step 4 above; it works on the
   ledger under `/data`):

   ```bash
   docker compose run --rm --no-deps loomtracer migrate-storage --dry-run
   docker compose run --rm --no-deps loomtracer migrate-storage
   ```

   Run before step 3 it stops with "there is none under /data": nothing has been copied yet, so
   there is no ledger to rename, and it created none.

5. **Rename the project handle** if a project is still registered under `hivemind`: the command of
   step 5 above, with `--tenant local`.

6. **Start the cell and check the ledger came with it,** then remove the old volume:

   ```bash
   docker compose up -d --remove-orphans
   curl http://localhost:8080/v1/health
   curl -s -H "Authorization: Bearer $LOOMTRACER_API_KEY" http://localhost:8080/v1/graph \
     | jq '(.data // .).decisions | length'
   ```

   The count is the one from step 1, and a known query (`loomtracer query recall`, or
   `GET /v1/decisions/recall?q=`) answers as before. That key is not a user's token, so it does
   not show whether the users' and agents' tokens came across: ask `GET /v1/whoami` with one
   issued before the upgrade (on a cell from v0.7.0 or earlier with its prefix changed to
   `lt_tk_`, see above), and it answers who it is (with `hm_tk_` still in front it answers `401`):

   ```bash
   curl -s http://localhost:8080/v1/whoami -H "Authorization: Bearer lt_tk_<64-hex of an old token>"
   ```

   Only then:

   ```bash
   docker volume rm <project>_hivemind-data
   ```

   If the new server answers 0 decisions and `docker run --rm -v <project>_hivemind-data:/d:ro
   alpine ls -A /d` still lists `ledger.sqlite`, it started on an empty ledger: step 3 was skipped
   or failed. Take the server down (`docker compose down`), keep a copy of
   `<project>_loomtracer-data` if anything was captured on it meanwhile, remove that volume
   (`docker volume rm <project>_loomtracer-data`) and run steps 3 to 6 again, so the old ledger
   lands on a fresh volume and not beside the files the empty one left. To go back, restore
   `cell-before-storage-rename.tgz` into a volume and run the old image.

Outside a container, a SQLite ledger holds only `hm_users` and `hm_tokens` to rename: run
`loomtracer migrate-storage` in the project (or with `--loomtracer-dir`), then
`loomtracer project rename` for a registered handle. The server refuses to start on a ledger that
still has the old tables (exit 4, naming `loomtracer migrate-storage`), so an upgrade run in the
wrong order does not start on empty tables and answer every token with `401`. The storage rename
revokes and mints no user, agent or tenant token: one that starts `lt_tk_` keeps working, and one
that still starts `hm_tk_` (a cell from v0.7.0 or earlier whose holders have not changed it, see
above) is refused.

---

## Troubleshooting

**`loomtracer` container exits immediately**: check logs with
`docker compose logs loomtracer`. Common cause: Postgres not yet healthy when
the server starts (the `depends_on: condition: service_healthy` guard handles
this; if it races, restart with `docker compose restart loomtracer`).

**The cell answers 0 decisions after you moved to the new compose files (SQLite mode)**: the
server started on the new, empty `loomtracer-data` volume while the ledger is still in the old one.
That happens to a cell on v0.8.0 as much as to one from before it. Check with
`docker run --rm -v <project>_hivemind-data:/d:ro alpine ls -A /d`: if it lists `ledger.sqlite`,
do not remove that volume; follow the SQLite steps of
[Upgrading a cell from v0.8.0 or earlier](#upgrading-a-cell-from-v080-or-earlier).

**Every user and agent token answers 401 after a cell from v0.7.0 or earlier is upgraded**: the
tokens still start `hm_tk_`, which v0.8.0 and every server after it refuse. Change that to `lt_tk_`
and keep the 64 hex characters after it: the same secret works, because nothing was revoked. See
the tokens paragraph of
[Upgrading a cell from v0.8.0 or earlier](#upgrading-a-cell-from-v080-or-earlier).

**`POST /v1/tenants` returns 500 "LOOMTRACER_ADMIN_KEY not configured"**: set
`LOOMTRACER_ADMIN_KEY` in your `.env` and restart the stack.

**Auth errors on every request in Postgres mode**: you must provision a tenant
first and use its `token_secret` as the bearer. The static `LOOMTRACER_API_KEY`
variable is ignored in Postgres mode.
