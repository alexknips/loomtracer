# Loomtracer web UI

The front end for HiveMind: a Next.js app that reads a HiveMind server over its HTTP API. It
began as Jeff Miao's Loomtracer UI (`TheChairmanMiao/neurofilament`, imported with its history)
and ran on Supabase. The HiveMind server is now its only backend.

It reads. It does not write: the HiveMind HTTP API has no endpoint for several things the
original UI did (sign-in, teams, tokens, accepting or rejecting a decision). Those features are
left out rather than shown dead. [`HIVEMIND_API_MAP.md`](HIVEMIND_API_MAP.md) lists each one and
what the server would need.

## Run it

You need Node 20.9 or later and a running HiveMind server.

```bash
# 1. A HiveMind server. A scratch one is enough to look around:
hivemind --hivemind-dir /tmp/scratch-ledger emit decision.capture \
  --title "Price per seat" --rationale "A fixed seat count is easy to budget for" \
  --options "Per seat,Usage based" --chose "Per seat" \
  --rests-on-assumption "Teams prefer a bill they can predict" \
  --binds "The first pricing experiment" --cheap-to-undo --topic-keys pricing
hivemind --hivemind-dir /tmp/scratch-ledger serve --port 18736

# 2. The UI, from the repository root:
npm ci
HIVEMIND_API_URL=http://127.0.0.1:18736 npm run dev --workspace=web
```

Open <http://127.0.0.1:3000/graph>. Against a server that has a `HIVEMIND_API_KEY`, set the same
value as `HIVEMIND_API_KEY` for the UI. `.env.example` lists every variable. The token is read
on the server side only; a test (`lib/hivemind/boundary.test.ts`) fails if a client component
could reach it.

**The UI has no login of its own.** Whoever can open it sees what its token can read. `npm run
dev` and `npm run start` therefore listen on `127.0.0.1` only (`next -H 127.0.0.1`); keep it that
way, or put your own access control in front before you bind another address. Do not put it on
the open internet with a token for a ledger you would not publish.

## Pages

| Page | Reads | What it shows |
|---|---|---|
| `/graph` | `GET /v1/projects`, `GET /v1/graph?project=`, `GET /v1/whoami` | A project's newest decisions (200, or up to 1000 on request) as a timeline, a trace map and the records behind them. The cutoff slider replays them in ledger order. Status (`accepted`, `contested`, `superseded`...) is a word on every decision. |
| `/search` | `GET /v1/decisions/search`, `GET /v1/projects` | Decisions by their words, filterable by status and project. |
| `/decisions/<id>` | `GET /v1/decisions/{id}`, `.../timeline`, `GET /v1/graph` | One decision: why, what it weighed, what it rests on, who accepted or rejected it, what replaced it. |
| `/review` | `GET /v1/decisions/search?status=proposed`, `GET /v1/attention/*` | What needs a person: proposed, contested, unanswered questions, decisions with no stated ground. |
| `/projects` | `GET /v1/projects` | Projects, their hierarchy and their decisions by status. |
| `/inspect` | none | Opens an exported reasoning bundle in the browser. Nothing is uploaded. |
| `/`, `/demo` | none | Jeff's landing page and fictional demo. |

## How `/graph` reads HiveMind

The history views read a *reasoning bundle*: records with their source text, and connections
between them. `lib/hivemind/bundle.ts` renders one project of the HiveMind graph as such a bundle,
as a pure function of `GET /v1/graph?project=<ref>` (that project's newest decisions and what hangs
off them; the server returns each edge once and bounds the page with `limit` and a cursor):

- each Decision, Option, Evidence, Hypothesis and Question node is one record, whose source is
  the node's own words (a decision's title and rationale, plus its quote, binds and undo cost);
- each graph edge between two such records is one connection, in the direction the graph stores
  it, worded with the graph contract's labels (`docs/GRAPH_CONTRACT.md` in the repository root);
- an edge to a decision of another project stays a pointer to it, not a copy of its text;
- records are ordered by the decision that introduced them, in ledger order.

It states only what the graph holds. Requests, blockers, notifications, reaffirmations and
project links are not drawn there.

## Tests

```bash
npm ci
npm test                       # unit tests: the adapter, the client, the route boundary
npm run build --workspace=web  # type check and production build
npm run lint --workspace=web
npm run test:e2e --workspace=web
```

The end-to-end tests start their own scratch `hivemind serve` on port 18736 (or
`HIVEMIND_E2E_PORT`, which must be 18736 or higher), in a temporary directory, seeded with a
fictional pricing study, and stop it by pidfile. They never touch another ledger, and they refuse
`:8080` and `:18732`. They need `hivemind` on `PATH` (or `HIVEMIND_BIN`) and a Playwright
Chromium (`npx playwright install chromium`). Set `LOOMTRACER_SCREENSHOTS=<dir>` to also write a
screenshot of each page, and `LOOMTRACER_E2E_BUILT=1` to test the production build instead of
`next dev`.

## Layout

- `app/`: pages and the two API routes (`/api/reasoning`, `/api/health`).
- `lib/hivemind/`: the HiveMind client, the graph-to-bundle adapter, project and decision reads.
- `components/reasoning/`: Jeff's history views.
- `../packages/reasoning/`: `@loomtracer/reasoning`, the bundle format and its validation.
- `e2e/`: Playwright tests and the scratch-server support code.

## Licence

AGPL-3.0-only, like the rest of this repository. The bundled fonts (Fraunces, Source Sans 3)
keep their own licences, next to the font files in `app/fonts/`.
