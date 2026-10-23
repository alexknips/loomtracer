# Loomtracer Deployment Guide

This guide covers running Loomtracer as a multi-tenant HTTP service using the
provided Docker image.

## Prerequisites

- Docker 24+ and Docker Compose v2
- (Optional) A Postgres 15+ instance if using the shared-backend-postgres feature

## Quick start: `docker compose up`

```bash
# Clone the repository
git clone https://github.com/alexknips/loomtracer
cd loomtracer

# (Optional) Static API key for the SQLite backend. The bundled compose file
# uses Postgres, where tenant/user tokens are issued instead.
export LOOMTRACER_API_KEY=your-secret-key

# Start Loomtracer + Postgres
docker compose up --build
```

The HTTP API is available at `http://localhost:8080`. Compose publishes it on
`127.0.0.1` only; see [SELF_HOSTING.md](SELF_HOSTING.md#exposing-the-cell-deliberately)
for exposing it to other machines.

Verify it is healthy:

```bash
curl http://localhost:8080/v1/health
# {"status":"ok"}
```

Data is persisted in a Docker named volume (`loomtracer-data`). The Postgres
service is included for the `shared-backend-postgres` feature; Loomtracer
defaults to SQLite stored in the same volume.

## Configuration reference

All configuration is passed through environment variables. Docker Compose
reads these from the shell or from an `.env` file in the project root.

| Variable | Default | Description |
|---|---|---|
| `LOOMTRACER_DATABASE_URL` | *(unset)* | Postgres connection string. Also accepted as `--database-url` on the CLI and `loomtracer mcp` (flag beats the env var), not only by `serve`. When set, the process connects directly to Postgres instead of the local SQLite ledger. |
| `LOOMTRACER_DIR` | `/data` | Directory where the SQLite ledger is stored. Mount a volume here for persistence. |
| `LOOMTRACER_BIND` | `127.0.0.1` (`0.0.0.0` in the Docker image) | Address the HTTP API binds. Also accepted as `--bind` on the CLI. Development mode (no `LOOMTRACER_API_KEY`, no `LOOMTRACER_DATABASE_URL`) refuses a non-loopback bind unless `--allow-unauthenticated-remote` is passed. |
| `LOOMTRACER_PORT` | `8080` | Port the HTTP API listens on. Also accepted as `--port` / `-p` on the CLI. |
| `LOOMTRACER_API_KEY` | *(unset)* | Static bearer token (SQLite mode only). Unset (or empty) with no `LOOMTRACER_DATABASE_URL` means development mode: no auth, loopback bind only. |
| `LOOMTRACER_ADMIN_KEY` | *(unset)* | Bearer token for `POST /v1/tenants` (Postgres mode). Required before provisioning tenants. |
| `LOOMTRACER_TENANT` | `local` | Tenant to open. For the CLI and `loomtracer mcp` this selects the Postgres tenant scope when `LOOMTRACER_DATABASE_URL`/`--database-url` is also set (see below for what happens on an unrecognized value). Not used by `serve`, which resolves the tenant per request from the caller's auth instead. |
| `LOOMTRACER_CORS_ORIGINS` | *(unset)* | Comma-separated origins for browser cross-origin requests. |
| `ANTHROPIC_API_KEY` | *(unset)* | Enables the server-side Layer-3 classifier and scorer (Claude Haiku) on the SQLite backend only. Ignored when `LOOMTRACER_DATABASE_URL` is set: the server logs one warning at start and classifies nothing. Optional. |
| `POSTGRES_PASSWORD` | `loomtracer` | Password for the bundled Postgres service (compose only). |

### CLI and MCP direct-Postgres mode

`LOOMTRACER_DATABASE_URL` (or `--database-url`) and `LOOMTRACER_TENANT` (or
`--tenant`) are read by every `loomtracer` subcommand, not only `serve` — this
includes `loomtracer mcp`, so a stdio MCP server can talk to the shared Postgres
backend directly instead of the local SQLite file.

- The CLI and `loomtracer mcp` never provision tenants in the `lt_tenants`
  table that `POST /v1/tenants` manages (see
  [SELF_HOSTING.md](SELF_HOSTING.md)), but they do check against it: an
  unrecognized or typo'd `--tenant` on a Postgres-backed CLI/mcp invocation is
  refused with `unknown tenant '<id>'` on read and write alike, never opened
  as a fresh empty scope. The HTTP server applies the same check to every
  request and answers an unregistered tenant with `404` and `error.code`
  `unknown_tenant` (message `unknown tenant '<id>': ...`), never the
  `not_found` of an id that does not exist; the CLI prints it as the local
  ledger does (exit 4). (On SQLite the tenants come from `loomtracer tenant create <id>`, and
  `local` is always registered.) Create the tenant through the provisioning
  route first, then point `--tenant` at it.
- A binary built without the `shared-backend-postgres` Cargo feature fails
  with an explicit error if a database URL is configured — it never falls
  back to SQLite silently. The Docker image is built with this feature;
  `cargo install` / `make install` (the path most local dev setups use) is
  not, so a locally installed `loomtracer` cannot open a Postgres ledger
  without rebuilding with `--features shared-backend-postgres`.

For a self-hosted cell (server + Postgres, one command), see [SELF_HOSTING.md](SELF_HOSTING.md).

### Example `.env` file

```dotenv
LOOMTRACER_ADMIN_KEY=change-me-before-production
LOOMTRACER_PORT=8080
LOOMTRACER_TENANT=acme-corp
POSTGRES_PASSWORD=s3cr3t
```

## Healthcheck endpoint

`GET /v1/health` returns HTTP 200 when the database is reachable:

```json
{"status": "ok"}
```

Returns HTTP 503 when the database cannot be opened:

```json
{"status": "error", "message": "..."}
```

The Docker image configures this as the container HEALTHCHECK. Docker Compose
waits for it before considering the service ready.

The server builds the decision graph of every tenant before it opens its port,
so `/v1/health` is not answered (the connection is refused) until the first
read after a start is as fast as every later one. On a large ledger that is a
few seconds to a few tens of seconds after the process starts; the startup log
line `graph cache warmed` reports the tenants and the time it took. A tenant
whose graph cannot be built is logged and skipped, and its first read builds it
as before.

## Graceful shutdown

The server handles `SIGINT` and `SIGTERM`. In-flight requests complete before
the process exits. Docker sends `SIGTERM` on `docker compose stop` or
`docker stop`, so running containers drain cleanly.

## Making API requests

See [ARCHITECTURE.md](ARCHITECTURE.md) for the full endpoint list. A quick
example:

```bash
# Capture a decision
curl -s -X POST http://localhost:8080/v1/decisions \
  -H "Authorization: Bearer $LOOMTRACER_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "title": "Use Postgres for shared-backend storage",
    "rationale": "SQLite WAL mode does not scale across multiple writer processes on shared storage.",
    "topic_keys": ["infrastructure", "storage"],
    "options": [
      {"label": "postgres", "description": "Postgres with r2d2 connection pool"},
      {"label": "sqlite",   "description": "SQLite with WAL mode"}
    ],
    "chosen_option_label": "postgres",
    "grounding": [
      {"kind": "evidence", "content": "SQLite allows one writer at a time", "source": "https://www.sqlite.org/lockingv3.html"}
    ],
    "binds": {"level": "something", "statement": "Every agent and the shared ledger move to Postgres"},
    "undo_cost": {"level": "costly", "statement": "Moving back means exporting the ledger out of Postgres and replaying it"}
  }'

# Query decisions
curl -s "http://localhost:8080/v1/decisions/search?q=storage" \
  -H "Authorization: Bearer $LOOMTRACER_API_KEY"
```

When `LOOMTRACER_API_KEY` is unset, omit the `Authorization` header entirely.

### Fluent (no-id) read routes

Four GET routes resolve a decision without an id, the HTTP equivalent of the
CLI's `why`/`verify`/`recall`/`situational` verbs
(`docs/AGENT_FLUENT_QUERYING.md`):

| Route | Query params | Equivalent |
|---|---|---|
| `GET /v1/decisions/situational` | `paths` (required, comma-separated), `since_offset`, `since_ts`, `limit`, `cursor` | `loomtracer query situational` |
| `GET /v1/decisions/recall` | `q`, `topic`, `status`, `actor_id`, `source`, `since`, `until`, `limit`, `cursor` | `loomtracer query recall` |
| `GET /v1/decisions/why` | `id` or `description` (+ optional `topic`) | `loomtracer query why` |
| `GET /v1/decisions/verify` | `id` or `description` (+ optional `topic`) | `loomtracer query verify` |

```bash
curl -s "http://localhost:8080/v1/decisions/why?description=storage" \
  -H "Authorization: Bearer $LOOMTRACER_API_KEY"
```

`why`/`verify` resolve `description` the same deterministic way `search`
does (term match + topic + recency, no LLM — `docs/AGENT_FLUENT_QUERYING.md`
§1). A description matching more than one decision at the best rank tier is
**not an error** — it comes back as HTTP 200 with `data.outcome` set to `"ambiguous"`
(with a `candidates` list). A description matching none is not an error either: it is
HTTP 200 with `data.outcome` set to `"not_found"`, the same reply the CLI and MCP print. A 404
(`error.code` `not_found`) is for what does not exist: an `id` that names no decision, or an
unknown route. A 404 with `error.code` `unknown_tenant` is a different refusal: the token or
`X-Loomtracer-Tenant` names a tenant the server holds no registry row for, so every call fails the
same way and none is "an id to skip". Every reply is the same envelope (with `close_match` beside `data` when the
decision lacks some of the words asked) from the CLI, the plugin's `why.sh`/`verify.sh`, HTTP
and MCP, and `tests/read_surface_parity.rs` asks the same questions through all four and
compares them. There is no `--pick`/`#N` continuation over HTTP (the API is stateless): re-call
with `id=<decision_id>` from the candidate list.

`disagree`/`supersede` are **not** fluent over HTTP today — `POST
/v1/decisions/{id}/disagreements` and `/v1/decisions/{id}/supersessions`
still take `decision_id` only as a path parameter, unlike their MCP
equivalents (`docs/AGENT_FLUENT_QUERYING.md` §3.4).

### Importance: which decisions carry the most impact

| Route | Query params | Equivalent |
|---|---|---|
| `GET /v1/decisions/importance` | `limit` (1–50, default 10), `cursor`, `include_not_in_force` | `loomtracer query rank_decisions_by_importance`, MCP `rank_decisions_by_importance` |

```bash
curl -s "http://localhost:8080/v1/decisions/importance?limit=5" \
  -H "Authorization: Bearer $LOOMTRACER_API_KEY"
```

The decisions with others resting on them, most first, each with who decided it (a person,
an agent, an agent within a person's delegation, or no one recorded). A decision the record
gives no reason to rank reads `importance: "not_assessed"`, never a number; `truncated` and
`data.next_cursor` page through the list. The same envelope comes from the CLI, the plugin's
`importance.sh`, HTTP and both MCP transports, and `tests/importance_surface_parity.rs`
compares them. `docs/DECISION_SCORING.md` says what a rank is read from.

### Restatements: linking a decision recorded twice

A decision the ledger holds twice (one per session that relayed it) is linked, not
merged, and `recall` and `why` then show one decision naming the other record. The two
routes carry `loomtracer restatements propose|apply --link` over HTTP, so a cell is curated
with a token and nobody needs the database password:

| Route | Does |
|---|---|
| `GET /v1/restatements/proposals` (`project`, `limit`, `cursor`) | Lists pairs that look like one decision recorded twice, each with the title words they share and their overlap; `truncated` and `data.next_cursor` page through them. Writes nothing. |
| `POST /v1/decisions/{id}/restatements` | Body `{"restates_id": "<earlier decision>"}`. Records one `SAME_AS` link from `{id}` (recorded later) to `restates_id`, as the token's actor. `linked: false` and no `event_id` when the pair was already linked either way round; an id that is not a recorded decision is refused. |

```bash
curl -s "http://localhost:8080/v1/restatements/proposals?limit=5" \
  -H "Authorization: Bearer $LOOMTRACER_API_KEY"

curl -s -X POST "http://localhost:8080/v1/decisions/capture:38419:0/restatements" \
  -H "Authorization: Bearer $LOOMTRACER_API_KEY" -H "Content-Type: application/json" \
  -d '{"restates_id": "capture:37652:0"}'
```

A proposal is title words, not a finding, and a wrong link cannot be undone for reads, so
there is no route that links every proposal: read each pair and link the ones that are
unmistakably the same decision.

### Superseding with a decision that is already recorded

`POST /v1/decisions/{id}/supersessions` records a NEW replacement for `{id}`, decided by the
caller. When the replacement is someone else's own answer, capture it first with its real
`decided_by`, then link the two; the MCP tool `link_supersession` does the same over `POST /mcp`.

| Route | Does |
|---|---|
| `POST /v1/decisions/{id}/supersession-links` | Body `{"supersedes_id": "<older decision>"}`. Records that `{id}` (the newer decision) supersedes `supersedes_id`, as `decision.superseded` attributed to the token's actor. It creates nothing and `{id}` keeps whoever decided it. `linked: false` and no `event_id` when `{id}` already superseded it, directly or through other decisions. An id that is not a recorded decision, the same id twice, and a link that would close a loop are refused with 400, and nothing is written. |

```bash
curl -s -X POST "http://localhost:8080/v1/decisions/decision-b/supersession-links" \
  -H "Authorization: Bearer $LOOMTRACER_API_KEY" -H "Content-Type: application/json" \
  -d '{"supersedes_id": "decision-a"}'
```

Here `{id}` is the newer decision; on `/supersessions` it is the old one.

## Multi-tenant usage

Each request carries a tenant identifier via the `X-Loomtracer-Tenant` header.
Tenant ledgers are isolated — decisions written for `tenant-a` are invisible
to `tenant-b` queries.

```bash
curl -s http://localhost:8080/v1/decisions/search \
  -H "Authorization: Bearer $LOOMTRACER_API_KEY" \
  -H "X-Loomtracer-Tenant: acme-corp" \
  -H "X-Loomtracer-Actor: alice@acme.com"
```

See [MULTI_TENANCY.md](MULTI_TENANCY.md) for the full multi-tenant design.

## Production checklist

- [ ] Set `LOOMTRACER_API_KEY` to a strong random value
- [ ] Mount a durable volume at `LOOMTRACER_DIR` (or back up the named volume)
- [ ] Place a TLS-terminating reverse proxy (nginx, Caddy, Traefik) in front
      of the container — the server speaks plain HTTP
- [ ] Set `restart: always` in compose (or equivalent in your orchestrator)
- [ ] Monitor `/v1/health` with an external uptime checker

---

## Hosted deployment: Fly.io

The cheapest production path is a scale-to-zero Fly.io machine backed by a
tiny Fly Postgres cluster. The app machine costs ~$0 while idle (Fly stops it
automatically); the Postgres cluster stays running at ~$2/mo.

### Prerequisites

Install flyctl: <https://fly.io/docs/flyctl/install/>

```bash
fly auth login
```

### One-time setup

```bash
# 1. Create the Fly app (choose a name and the region nearest to you)
fly launch --name loomtracer-api --region sea --no-deploy

# 2. Provision the smallest Fly Postgres cluster (~$2/mo, stays running)
fly postgres create \
  --name loomtracer-pg \
  --region sea \
  --vm-size shared-cpu-1x \
  --volume-size 1

# Attach Postgres — injects LOOMTRACER_DATABASE_URL as a Fly secret automatically
fly postgres attach loomtracer-pg --app loomtracer-api

# 3. Set the API key secret (all clients must send this as a Bearer token)
fly secrets set LOOMTRACER_API_KEY="$(openssl rand -hex 32)" --app loomtracer-api

# 4. Deploy
fly deploy --app loomtracer-api
```

### Verify

```bash
fly status --app loomtracer-api
curl https://loomtracer-api.fly.dev/v1/health
# {"status":"ok"}
```

### Cost estimate

| Resource | Config | Monthly cost |
|---|---|---|
| App machine | shared-cpu-1x, 256 MB, scale-to-zero | ~$0 idle / ~$1.94 if running 24/7 |
| Fly Postgres | shared-cpu-1x, 256 MB, 1 GB volume | ~$2.09 (always running) |
| Bandwidth | First 100 GB | $0 |
| **Total** | | **~$2–4/mo** |

### Secrets reference

Set these via `fly secrets set --app <your-app-name>`. Never commit them.

| Secret | Required | Description |
|---|---|---|
| `LOOMTRACER_DATABASE_URL` | Yes (Postgres mode) | Set automatically by `fly postgres attach`. Enables multi-tenant Postgres backend. |
| `LOOMTRACER_API_KEY` | Yes (production) | Bearer token all clients must send. Omit only in dev mode. |
| `LOOMTRACER_ADMIN_KEY` | Recommended | Bearer token for the `POST /v1/tenants` provisioning endpoint. |
| `ANTHROPIC_API_KEY` | Not used | Ignored on Postgres: the server-side Layer-3 classifier and scorer start only on the SQLite backend. Classify keylessly with `/loomtracer-capture:classify-queue`. |

### Continuous deployment

The repository ships a GitHub Actions workflow at
`.github/workflows/deploy-hosted.yml` that deploys automatically on every
push to `master` that touches `src/**`, `Dockerfile`, `fly.toml`, or the
`Cargo.*` lock files.

Add your Fly API token as a repository secret:

```
GitHub → Settings → Secrets → Actions → New repository secret
Name:  FLY_API_TOKEN
Value: (output of `fly auth token`)
```

Manual deploys are also supported via the **Actions → Deploy to Fly.io →
Run workflow** button.
