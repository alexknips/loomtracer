# Jeff's UI on the HiveMind API: what maps and what does not

The UI began on Supabase. It now reads a HiveMind server over its HTTP API, and nothing else.
This is one row per page and API route of the original app (`TheChairmanMiao/neurofilament`
at `5c0a0278`), saying what became of it and which HiveMind endpoint it uses. Where there is
none, the last column says what the server would need.

- **Rewired**: works against a local `hivemind serve`.
- **Kept**: static or local; needs no server.
- **Left out**: removed from the app and its navigation. No dead buttons.

All reads go through the UI's own server (`lib/hivemind/client.ts`) with
`HIVEMIND_API_URL` and a bearer token from `HIVEMIND_API_KEY`. A browser never calls HiveMind.

## Pages

| Original page | Now | HiveMind endpoint | What the server would need |
|---|---|---|---|
| `/` landing | **Kept.** The "Request access" form and the "Sign in" link are gone; the full app links to the workspace instead. | none | Nothing. The page's copy still says "invite-only pilot" and "Request access": that is a decision for Alex and Jeff, not made here. |
| `/demo` | **Kept**, unchanged (fictional data in the page). | none | Nothing. |
| `/graph` (reasoning workspace) | **Rewired.** One project's decisions as a timeline, trace map and sources. | `GET /v1/graph`, `GET /v1/whoami` | See "Notes for the server" below: a per-project graph, and the fields the graph leaves out. |
| `/inspect` (local file inspector) | **Kept.** Opens an exported bundle in the browser; nothing is uploaded. | none | Nothing. |
| `/review` (redirected to `/graph`) | **Rewired**, as a read-only queue: proposed, contested, unanswered questions, decisions with no stated ground. | `GET /v1/decisions/search?status=proposed`, `GET /v1/attention/contested`, `/waiting`, `/open-why` | **Accept and reject.** There is no REST endpoint to accept a decision or to reject one outright (`decision.accepted` and `decision.rejected` are CLI events). Disagreement exists (`POST /v1/decisions/{id}/disagreements`) and could back a "reject with a reason" button once the UI can say who is acting. |
| `/projects` | **Rewired**, read-only: projects, hierarchy, decisions by status. | `GET /v1/graph` (Project nodes, `PART_OF` edges, each decision's `project`) | **List, create, move and link projects over HTTP.** Today only the CLI does (`hivemind project list/register/link/...`). `GET /v1/projects` and `POST /v1/projects` would replace reading the whole graph. |
| `/login` | **Left out.** | none | A login for people. `docs/AUTH_MODEL.md` plans OIDC-backed sessions; the server accepts bearer tokens and WorkOS JWTs, but the UI has no sign-in. Until then the UI has no login of its own. |
| `/auth/callback`, `/auth/signout` | **Left out.** | none | The same. |
| `/join` (accept an invitation) | **Left out.** | none | An invitation model: members, roles, invitations. |
| `/onboarding` | **Left out.** | none | Project creation over HTTP (see `/projects`) and a way to mint an agent token for the signed-in person. |
| `/oauth/consent` | **Left out.** (Jeff's own OAuth consent screen for MCP clients.) | none | Nothing for the UI: the server publishes its own OAuth discovery (`/.well-known/oauth-*`) and its MCP endpoint is `POST /mcp`. |
| `/settings/connections` (agent connections) | **Left out.** | partial: `POST /v1/agent-tokens` | Listing and revoking agent connections for a person. The mint endpoint needs the admin key, which the UI must not hold. |
| `/settings/tokens` | **Left out.** (Retired in the original too: it answered 410.) | partial: `POST /v1/users/{id}/tokens`, `DELETE /v1/users/{id}/tokens/{token_id}` (admin key) | A person-scoped token endpoint that does not need the admin key. |
| `/settings/team` | **Left out.** | partial: `GET /v1/users`, `POST /v1/users` (admin key) | Roles (owner, reviewer), removing a member, and a non-admin way to read the team. |
| `/settings/invitations` | **Left out.** | none | An invitation model. |
| `/settings/access-requests` | **Left out.** | none | An access-request queue (the landing page's form fed it). |
| `/.well-known/oauth-protected-resource` and `.../api/mcp` | **Left out.** | none | Nothing: see `/oauth/consent`. |

## API routes

| Original route | Now | HiveMind endpoint | What the server would need |
|---|---|---|---|
| `GET /api/reasoning` | **Rewired.** Projects; with `project_ref`, that project's bundle. Read-only. | `GET /v1/graph`, `GET /v1/whoami` | As for `/graph`. |
| `POST /api/reasoning` (save a bundle) | **Left out.** | none | HiveMind stores decisions, not bundles. Its writes are `POST /v1/decisions`, `/v1/evidence`, `/v1/hypotheses` and `/v1/ingest`; none takes a free-form "contribution". |
| `POST /api/reasoning/commitments` (adopt, revise, withdraw) | **Left out.** | none | Accepting a decision over HTTP (above). HiveMind has no "revise a commitment" or "withdraw" action; replacing is `POST /v1/decisions/{id}/supersessions`. |
| `POST`, `DELETE /api/reasoning/grants` (agent authority) | **Left out.** | none | HiveMind has no grant object: an agent's delegation is the `delegated_by` marker on a capture. A standing grant would be a new server concept. |
| `GET /api/health` | **Rewired.** 200 when the server answers for the key; 503 naming why. | `GET /v1/whoami` | Nothing. |
| `GET /api/session` | **Left out.** The workspace header shows the tenant and actor instead. | `GET /v1/whoami` | Nothing. |
| `GET`, `POST /api/projects` | **Left out.** The page lists; it does not create. | `GET /v1/graph` | `GET`, `POST /v1/projects`. |
| `PATCH /api/projects/[id]` (rename, move) | **Left out.** | none | Update and move over HTTP. |
| `GET /api/search` (retired in the original) | **Replaced** by the `/search` page. | `GET /v1/decisions/search` | A `project` filter: the handler reads none (`project: None`). |
| `GET /api/graph`, `PATCH /api/nodes/[id]/organization`, `POST /api/review/[id]/ratify`, `POST /api/review/[id]/reject`, `GET /api/export` | **Not imported.** Retired node-graph code (Jeff's own bead `nf-w56`); nothing called them. | `GET /v1/graph`, `POST /v1/decisions/{id}/disagreements`; `hivemind export` (CLI) | Ratify and reject: see `/review`. Export over HTTP: the CLI writes a Markdown tree; no endpoint does. |
| `GET`, `POST`, `DELETE /api/mcp` | **Left out.** | `POST /mcp` (HiveMind's own MCP endpoint) | Nothing: the server already is an MCP server. |
| `DELETE /api/connections/[id]` | **Left out.** | none | See `/settings/connections`. |
| `POST /api/oauth/consent` | **Left out.** | none | See `/oauth/consent`. |
| `GET`, `POST /api/tokens`, `DELETE /api/tokens/[id]` | **Left out.** | partial, see `/settings/tokens` | As there. |
| `GET /api/team`, `POST /api/team/invitations`, `DELETE /api/team/invitations/[id]`, `DELETE /api/team/members/[id]` | **Left out.** | partial: `GET /v1/users` | See `/settings/team` and `/settings/invitations`. |
| `GET`, `POST /api/invitations`, `DELETE /api/invitations/[id]`, `POST /api/invitations/prepare` | **Left out.** | none | An invitation model. |
| `POST /api/access-requests`, `GET /api/admin/access-requests`, `PATCH /api/admin/access-requests/[id]`, `PATCH /api/accounts/[id]` (suspend an account) | **Left out.** | none | An access-request queue and account status. |

## Inside `/graph`

| Original feature | Now | What the server would need |
|---|---|---|
| Project picker | **Rewired**, with each project's decision count. | A project list endpoint (above). |
| Saved-history picker, "Reload saved history" | **Left out**, one history per project. "Reload from the ledger" re-reads it. | Nothing. |
| "Capture contribution" composer and "Save changes" | **Left out.** | A capture that takes a free-form contribution, or the UI composing a `POST /v1/decisions` (title, rationale, options, what it rests on, what it binds, what undoing it costs). |
| "Commitments and authority" panel, "Agent commitment grants" | **Left out.** | See `/api/reasoning/commitments` and `/grants`. |
| Review notes in a project's history | **Left out** for a project read from HiveMind: nothing stores them. They still work, and export, in `/inspect`. | A place to keep a note against a decision. |
| Timeline, trace map, sources, inspector, cutoff slider, actor and role filters, "Export history" | **Kept**, over the rendered project. | Nothing. |

## Notes for the server

What building this against the API showed. None of it was changed here.

1. **`GET /v1/graph` repeats edges.** On a small ledger every `HAS_OPTION` edge and most others
   appear twice, identically. The UI dedupes by (relation, source, target).
2. **`GET /v1/graph` returns the whole tenant's graph, unbounded.** The UI builds one project's
   view from it. At tens of thousands of decisions this wants a per-project graph with a limit
   and a cursor.
3. **No project endpoints over HTTP.** The UI derives projects from the graph.
4. **The graph leaves out what a history view could use**: an `event_origin` and a time on
   options, evidence, hypotheses and questions (the UI orders them by the decision that
   introduced them); an option's description; a hypothesis's kind and status (a refuted one shows
   only through its `REFUTES` edge); where evidence was seen.
5. **`GET /v1/decisions/{id}/compact-view` answers for the head of the supersession chain**, not
   for the decision asked about: asked about a superseded decision it describes its replacement.
   That is what "compact" means, but a page about one decision wants `GET /v1/decisions/{id}`,
   which the UI uses.
6. **Reading is by bearer token only.** There is no person behind a UI request, so everything
   the token can read is what the UI shows.
