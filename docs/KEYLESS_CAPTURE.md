# Keyless Capture — Zero to First Decision

Capture and classify your first decision without an `ANTHROPIC_API_KEY`.
The server-side classifier (Worker B) is optional — Worker A drains the
same queue using your agent's subscription seat.

---

## Step 1 — Start a Loomtracer cell

Clone and start the cell in three commands:

```bash
git clone https://github.com/alexknips/loomtracer && cd loomtracer
cp .env.example .env
# Linux:
sed -i "s/change-me-before-production/$(openssl rand -hex 32)/g" .env
# macOS:
# sed -i '' "s/change-me-before-production/$(openssl rand -hex 32)/g" .env
docker compose up --build -d
```

No `ANTHROPIC_API_KEY` needed in `.env` — leave that line empty or omit it.

Provision your first token:

```bash
ADMIN_KEY=$(grep LOOMTRACER_ADMIN_KEY .env | cut -d= -f2)
TOKEN=$(curl -s -X POST http://localhost:8080/v1/tenants \
  -H "Authorization: Bearer $ADMIN_KEY" \
  -H "Content-Type: application/json" \
  -d '{"tenant_id": "me", "display_name": "Me"}' | jq -r '.token_secret')
echo "$TOKEN"   # lt_tk_...
```

---

## Step 2 — Install the loomtracer-capture plugin

From Claude Code:

```text
/plugin marketplace add alexknips/loomtracer
/plugin install loomtracer-capture@loomtracer
/reload-plugins
```

The plugin adds `/loomtracer-capture:capture`, `/loomtracer-capture:query-decisions`,
`/loomtracer-capture:classify-queue`, a bundled MCP server, and a session-start
directive that tells each session to recall earlier decisions before acting and
capture rules learned from failures, design choices later work must follow and
reversals, not routine status (`LOOMTRACER_DIRECTIVE_DISABLE=1` turns it off).

For Codex, install from the repository:

```bash
codex plugin marketplace add https://github.com/alexknips/loomtracer.git
```

Then open `/plugins → Loomtracer Plugins → Loomtracer Capture`.

---

## Step 3 — Capture a decision in-session

From Claude Code, call the `/capture` command during a live session:

```text
/loomtracer-capture:capture "Switch to Postgres for the shared backend" \
  --kind decision \
  --title "Switch to Postgres for the shared backend" \
  --rationale "SQLite WAL mode does not scale across concurrent writers" \
  --topic-keys infrastructure,storage \
  --options "SQLite,Postgres" \
  --chose "Postgres" \
  --rests-on-assumption "More than one agent writes to the shared ledger at once" \
  --binds "Every agent and the shared ledger move to Postgres" \
  --undo-cost "Moving back means exporting the ledger out of Postgres and replaying it"
```

The command writes to a **local ledger**: the `loomtracer/` folder of the project Claude Code runs
in (the repository root, or the current folder outside a repository), created there if it is
missing, or the folder `LOOMTRACER_DIR` or the plugin's `loomtracer_dir` option names. It does not
contact the cell from Step 1, with `LOOMTRACER_API_URL` set or not, and
neither does the plugin's bundled MCP server, which opens the same local ledger. It prints where
it wrote:

```
Captured Loomtracer decision decision-<id> in <folder>/loomtracer.
```

That decision does not enter the classification queue: it already carries its topic keys. The
queue holds session transcripts sent to the cell's `POST /v1/ingest`, and Step 4 drains those.

With `LOOMTRACER_API_URL` set the command does not quietly record to the wrong place either: from a
folder with no ledger it refuses (exit 2, nothing recorded, nothing created), and from a folder
that holds one it records there and says on stderr which ledger took it, and that the cell did
not.

**To record the decision in the cell instead**, post it to the cell's HTTP API with the token
from Step 1:

```bash
curl -s -X POST http://localhost:8080/v1/decisions \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{
    "title": "Switch to Postgres for the shared backend",
    "rationale": "SQLite WAL mode does not scale across concurrent writers",
    "topic_keys": ["infrastructure", "storage"],
    "options": [{"label": "SQLite"}, {"label": "Postgres"}],
    "chosen_option_label": "Postgres",
    "grounding": [{"kind": "assumption", "statement": "More than one agent writes to the shared ledger at once"}],
    "binds": {"level": "something", "statement": "Every agent and the shared ledger move to Postgres"},
    "undo_cost": {"level": "costly", "statement": "Moving back means exporting the ledger out of Postgres and replaying it"}
  }'
```

The reply carries the new `decision_id`, and the cell reads in Step 5 list it. An agent that
should capture into the cell on its own points its MCP client at the cell's `/mcp` endpoint
instead ([Configuring an agent to use MCP-over-HTTP](SELF_HOSTING.md#configuring-an-agent-to-use-mcp-over-http))
and calls `capture_decision`.

---

## Step 4 — Drain the queue with Worker A (no API key)

Run the classify-queue command after your session:

```text
/loomtracer-capture:classify-queue
```

This uses your subscription model seat — no `ANTHROPIC_API_KEY` required.
The command:

1. Lists pending batches in the classification queue
2. Classifies each batch using your agent's model
3. Writes `IngestBatchClassified` events to the ledger
4. Reports batches processed and total captures written

Pass `--limit N` to cap the number of batches per run (default 20):

```text
/loomtracer-capture:classify-queue --limit 5
```

Check queue depth at any time. On a local ledger and against a server-backed cell
(`LOOMTRACER_API_URL` set) alike, the answer is an object whose `pending_total` is the whole
queue's depth and whose `truncated` says the page holds fewer batches than are pending:

```bash
loomtracer classify-queue list --json --limit 1 | jq .pending_total
```

That is the reply of builds after the v0.7.0 release. The v0.7.0 release prints the bare list of
batches for a local ledger (`--limit` 20 by default, so count it with a `--limit` past the queue:
`jq length`) and `{batches, budget}`, with no `pending_total`, from a server. The plugin's
`scripts/check-classify-queue.sh` reads all three shapes.

---

## Step 5 — Query it back

`loomtracer query` and `/loomtracer-capture:query-decisions` read a **local ledger**. Neither contacts
a server; only `classify-queue` does. Where you read depends on where Steps 3-4 wrote.

**A local ledger** (`LOOMTRACER_API_URL` unset):

```text
/loomtracer-capture:query-decisions --limit 5
```

Or via CLI:

```bash
loomtracer query recent_decisions --since 1h --limit 5
loomtracer query get_relevant_decisions --topic infrastructure
loomtracer query search_decisions --q "postgres"
```

**The cell from Step 1** (`LOOMTRACER_API_URL` set): ask its HTTP API with the token from Step 1.
Each call returns the same decisions the CLI lists for a local ledger.

```bash
export LOOMTRACER_API_URL=http://localhost:8080
SINCE=$(date -u -d '1 hour ago' +%Y-%m-%dT%H:%M:%SZ)   # macOS: date -u -v-1H +%Y-%m-%dT%H:%M:%SZ
curl -s -H "Authorization: Bearer $TOKEN" "$LOOMTRACER_API_URL/v1/decisions/search?since=$SINCE&limit=5"
curl -s -H "Authorization: Bearer $TOKEN" "$LOOMTRACER_API_URL/v1/decisions/relevant?topic=infrastructure"
curl -s -H "Authorization: Bearer $TOKEN" "$LOOMTRACER_API_URL/v1/decisions/search?q=postgres"
```

With `LOOMTRACER_API_URL` set, `loomtracer query` does not quietly read the wrong place. From a folder
with no ledger it refuses and creates nothing; from a folder that holds one it answers from that
ledger and adds a `notice` saying the answer is the local ledger's, not the server's. Unset
`LOOMTRACER_API_URL` to read a local ledger without the notice. `loomtracer digest`, `review`,
`export` and `dump` follow the same rule (`dump` and `emit` say it on stderr, so their stdout stays
a plain DOT graph or id). So do the verbs that act on a decision (`disagree`, `supersede`, `move`,
`retitle`, `ground`), `ask`, `import`, `restatements`, `map` and `quality-scan`: each one refuses
from a folder with no ledger, and from a folder that holds one says which ledger it used. The
refusal names the cell's route for the same thing. So do the `project` verbs, `ingest
slack-thread`, `tui` and `tenant create` (a tenant made in a ledger the cell never sees is no
tenant of the cell's; the cell makes one at `POST /v1/tenants` with its admin key). The cell has
no route for the project registry, and the refusal says so. `project show --current` and `project
use --clear` open no ledger and are unaffected.

A decision drafted this way is listed in a time window (`recent_decisions --since`, `digest`,
`search_decisions --since`, `get_decisions_added_since`, `review --since`) by the time of the
turn it was said in, not by when the classification ran. So `--since 1h` right after classifying
a session lists its decisions when that conversation took place in the last hour, and a
conversation from last week sits in last week's window however late it was classified.

---

## How the two classifiers relate

| | Worker A | Worker B |
|---|---|---|
| **Where** | Agent-side (`/classify-queue`) | Server-side (`src/classifier.rs`) |
| **Key required** | No — uses subscription seat | Yes — `ANTHROPIC_API_KEY`, and a SQLite server |
| **When it runs** | On demand after your session | Automatically in background |
| **Queue** | Same shared work queue | Same shared work queue |
| **Result** | Identical `IngestBatchClassified` events | Identical `IngestBatchClassified` events |

Both workers write to the same queue and the result is the same — concurrent
classification is idempotent (last writer wins per batch).

If you later add `ANTHROPIC_API_KEY` to a SQLite server, Worker B starts
classifying automatically. A Postgres server (`LOOMTRACER_DATABASE_URL` set: the
default compose stack, Fly) never starts Worker B: it reads a SQLite ledger and
the captures are in Postgres, so the server ignores the key and logs one warning
saying so. There Worker A is the only classifier. Worker A is also available for
on-demand draining on a SQLite server, or when the server key is absent.

---

## See also

- [`docs/SELF_HOSTING.md`](SELF_HOSTING.md) — full self-host runbook
- [`plugins/loomtracer-capture/README.md`](../plugins/loomtracer-capture/README.md) — plugin install, commands, and session hooks
- [`docs/AGENT_DECISION_CAPTURE.md`](AGENT_DECISION_CAPTURE.md) — all capture paths (CLI, HTTP API, hooks, MCP)
- [`docs/CAPTURE_CLASSIFIER.md`](CAPTURE_CLASSIFIER.md) — classifier design and queue mechanics
