# Graph Contract: Arrows Run Newer → Older

Every arrow a reader is shown points from the node recorded later to the node recorded
earlier, for every `RelationKind`. The ledger is append-only, so a reference always points
back at something that already exists; the arrows say so.

This document is the rule. The `arrow_*` tests in `src/projector/tests.rs` enforce it for
every kind on projected data, and the table below is checked against the code row by row.

## The rule

- **Stored: one semantic direction per kind.** `RelationKind::endpoints()` names one
  `(source node kind, target node kind)` pair per kind, and the source is the side making
  the claim: a decision is `BASED_ON` its evidence, evidence `SUPPORTS` a hypothesis. The
  Kuzu rel tables are bound to that pair, the Postgres projection stores
  `(relation, from, to)` exactly as projected, and every query walks it. A query never has
  to look for a relation in two places, which is what keeps staleness propagation (a
  refuted hypothesis marking every decision premised on it) complete.
- **Shown: an arrow, oriented by time.** `projector::arrow::orient` turns one stored edge
  into the arrow a reader sees. If the stored source was recorded at or after the stored
  target, the arrow runs as stored. If the stored target was recorded after the stored
  source, the arrow runs the other way and carries the relation's *reversed* label. So the
  arrow always points at the older node.
- **Time is `event_origin`.** A node's time is the ledger offset of the event that created
  it, and nothing moves it afterwards: a score, a blocker resolution, a notification
  acknowledgement or a project anchor annotates a node without changing its `event_origin`.
  A forward reference (see [Forward references](#forward-references)) is created as a
  placeholder at the referencing event; the event that really records it fills the
  placeholder in and sets the offset to that event.
- **Ties and unknowns keep the stored direction.** Nodes made by the same event are the
  same age, and a node without a recorded time cannot be ordered, so neither reverses an
  edge.
- **Actors are identities, not records.** An actor has no place on the time axis and counts
  as older than every record, so an edge to an actor always points at the actor. This is
  why 14 of the 31 kinds never reverse.
- **No inverse kinds.** The server has no "superseded by" relation. To ask what supersedes
  a decision, read `SUPERSEDES` edges *into* it; to ask what it supersedes, read them *out
  of* it. A reversed label appears only when the stored direction cannot point backward in
  time (see the table).

## What readers get

`GET /v1/graph` edges, the neighborhood edges (`loomtracer query why`, MCP
`get_decision_neighborhood`, `GET /v1/decisions/why`), the CLI text summary, and the DOT
exports (CLI `dump`, MCP `dump_graph`, the TUI) all draw the same arrow:

| Field | Meaning |
|---|---|
| `from`, `to` | The arrow's ends: `from` is the newer node, `to` the older one. |
| `relation` | What the edge means, the stored kind. Unchanged whichever way the arrow runs. |
| `label` | The relation read along the arrow, an active phrase (`based on`, `informs`). |
| `reversed` | `true` when the arrow runs against the stored direction. |

A consumer that needs the side making the claim reads `from`/`to` when `reversed` is
`false` and the other way round when it is `true`. Rust readers use
`NeighborEdge::stored_source` / `stored_target`. A UI draws `from → to` with `label` and needs no
time logic of its own.

## What `GET /v1/graph` says about its nodes

The response is `{decisions, nodes, edges}`; the edges are the arrows above. Two more things
a reader needs without reconstructing them from edges:

- **`decisions[]`**: one entry per Decision node, its stored row (`id`, `title`, `rationale`,
  `topic_keys`) plus three derived fields.
  - `status`: `proposed`, `accepted`, `rejected`, `contested` or `superseded`, the word every
    other reader of a decision uses. Any incoming `SUPERSEDES` makes it `superseded`; else
    `ACCEPTED_BY` and `REJECTED_BY` together make it `contested`; else accepted only is
    `accepted`, rejected only is `rejected`, neither is `proposed`. A UI's "active" is
    `proposed` or `accepted`.
  - `deciders`: the `ACCEPTED_BY` actors as `{id, kind}` (`kind` is `human`, `agent` or
    `unknown`, from the `human:` / `agent:` id prefix), sorted by id, `[]` until someone
    accepts. Distinct from the proposer (`PROPOSED_BY`, whoever recorded it) and the
    participants (`PARTICIPATED_BY`). On a contested decision these are the acceptors; the
    rejecters are the `REJECTED_BY` edges. A decision the classifier captured from a
    transcript (`capture:<event>:<index>`) has the acceptors and rejecters the classifier named
    and no others: its `actor_id` is who proposed, made or reported it, so it is never read as
    the decider, and a capture that names no acceptor stays `proposed` until a person decides it.
    Who the conversation was held with is read from the batches the classification covers, never
    from the submission: the decision is `INITIATED_BY` the submitter of the first batch the
    classification names and `PARTICIPATED_BY` each batch's submitter and, beside a human, the
    agent tool that ran (`agent:<tool>:hook`; an agent token is already the agent). A
    classification that covers no received batch names nobody.
  - `decided_at`: the `decision.proposed` capture event's timestamp, ISO-8601 UTC (the same
    value the query layer calls `occurred_at`, e.g. `DecisionBrief`). A classified capture
    carries the time of the transcript turn it came from when it names one that has a time
    (`source_ts`, read from the received turn); otherwise the newest turn time of the batch it
    was classified from, when that batch's turns carry times (read from the ledger's received
    batch when the graph is built, so a session classified days later still reads as when it
    was said); otherwise the classified-batch event's time. `null` only for a decision from an
    event predating the ledger's timestamp backfill.
  - `slug`: the decision's stable link segment (`/decisions/<slug>`) — its title, kebab-cased
    (apostrophes dropped, so `reader's` is `readers`) and cut at the last whole word that
    fits in 60 characters (a first word longer than that is cut at 60), with a growing
    id-tail suffix (`-a1b2`, then longer) for a later decision whose title
    slugs to the same thing. Assigned once, when the decision is first projected — its
    `decision.proposed`, or the `ingest.batch_classified` that recorded it as a draft from the
    classify queue (ledger order, so replay always assigns the same slugs); nothing projected
    after that touches it, so a retitle (or any other annotation) does not move a decision's
    link, and neither does projecting the same `decision.proposed` or classified draft a second
    time. `null` only for a decision from an event predating this field.
- **`nodes[]` with `kind: "Decision"`** carry `decided_at` and `slug` the same way, so a
  whole-graph view can show when each decision was made, and link to it, without a second
  lookup into `decisions[]`.
- **`nodes[]` with `kind: "Option"`** carry `title`: the option's own label, else its id, so an
  option always has something to show. `label` stays as it was (absent when no label was ever
  recorded). A label recorded as a slug or a lettered code (`pick-a-postgres`) is read as words
  (`Postgres`) when the ledger is replayed; the node then also carries `recorded_as`, the text the
  capture recorded, so a page that shows the words can show what the record says beside them. It
  is absent when the label reads as it was recorded. Every `CHOSE` edge ends at such a node. A
  classifier draft can choose a label it did not weigh, or name a choice and no options: that
  option is a node labelled with the recorded words, with a `CHOSE` edge and no `HAS_OPTION`.
- **`nodes[]` with `kind: "Question"`, `"Ask"`, `"Blocker"` or `"DecisionRequest"`** carry a
  `label`: the words that were asked for a question and an ask (`text`), the one-line `reason`
  for a blocker and a decision request. A `Reaffirmation` and an `AttributionDispute` read as their
  `reason` the same way.
  None of these has a title, so without the label a page can only call the node untitled.

Both are pure reads: three bulk edge scans (`SUPERSEDES`, `ACCEPTED_BY`, `REJECTED_BY`), no
per-decision queries and no inference.

**Each edge once.** The graph holds one edge per `(relation, from, to)`, the first write's, on
every backend (Postgres `ON CONFLICT DO NOTHING`, Kuzu, and the in-memory graph the API serves
from). A capture asserts each of its pairs twice, in the `decision.proposed` event and again in
the `relation.added` that follows it; the second assertion is not a second edge, so `edges[]`
never repeats a `(relation, from, to)`.

## One project's page: `GET /v1/graph?project=`

The whole-tenant graph grows with the ledger, so a reader that shows one project asks for that
project. `project` is a registered handle or a personal address (`personal:agent:claude`);
`limit` (default 200, at most 1000) and `cursor` page it. The response has the same
`{decisions, nodes, edges}` shape as above and these fields besides:

| Field | Meaning |
|---|---|
| `project` | The address the project's decisions are filed under (a personal address in its canonical form). |
| `limit` | How many of the project's decisions the page holds at most. |
| `total_matches` | Every decision the project holds, on this page or not. |
| `truncated` | `true` when older decisions remain. |
| `next_cursor` | Where the next, older page starts (`null` when `truncated` is `false`). Pass it back as `cursor`. |

A page holds the project's newest decisions first, by ledger offset, and `next_cursor` continues
to older ones, so a reader can walk the whole project and a short page never reads as a complete
one. What hangs off the page's decisions comes with them:

- every edge that has a page decision at either end, whatever its relation, and the node at its
  other end (an actor, an option, evidence, a hypothesis, a question, a request, a blocker, or
  another decision, which may belong to another project);
- from an option reached that way, what it is premised on (`PREMISED_ON`);
- from evidence and hypotheses reached that way, one hop of `SUPPORTS` and `REFUTES`;
- a reaffirmation whose line ends at a node reached that way.

Nodes at the far end are named, not followed: a decision of another project is a `nodes[]` and
`decisions[]` entry and shows no edges of its own beyond those to the page. Every edge in the
page has both of its ends in `nodes[]`, each edge appears once, and arrows run newer to older
like every arrow. A project nobody registered is 404 (a typo never reads as an empty project);
a personal address always resolves and may be empty. `limit` or `cursor` without `project`
is 400. Without `project` the whole tenant is returned, unpaged, as before.

## One decision's graph: `GET /v1/graph?decision=`

A page about one decision needs that decision and what it is linked to, not a project's page or
the tenant. `decision` is the decision's id. The response has the same `{decisions, nodes,
edges}` shape as above and no paging fields, because nothing is cut off: it holds the
decision and what hangs off it, chosen by the same rules as a project's page (every edge with
the decision at either end and the node at the other end, what its options are premised on, one
hop of `SUPPORTS` and `REFUTES` from the evidence and hypotheses reached, and a reaffirmation
whose line ends at a node reached). A decision at the far end of an edge (the one it
supersedes, the one that supersedes it, a premise) is a `nodes[]` and `decisions[]` entry with
its own `status` and `deciders`, and shows no edges of its own beyond those to this decision.
Each edge appears once and has both of its ends in `nodes[]`; every entry reads exactly as it
does in the whole graph. The size follows what the decision is linked to, not the ledger. A
decision the graph does not hold is 404; `project`, `limit` or `cursor` together with
`decision` is 400, as is an empty `decision`.

## Projects: `GET /v1/projects`

A picker or a tree needs the projects and how many decisions each holds, not the graph. The
response is the query envelope; `data` holds `items`, one per project with a registration or a
decision in it, by handle, and `limit`, `cursor`, `next_cursor`, `total_matches` as every page:

| Field | Meaning |
|---|---|
| `handle` | The address decisions are filed under. |
| `name` | The display name it was registered with, else its handle; for a personal address, the person or agent tool it belongs to. |
| `personal` | `true` for a personal address (`personal:...`), which appears once a decision is filed in it. |
| `parent` | The handle of the project it is part of now (`PART_OF`), `null` when it is part of none. A retracted link does not show. |
| `decisions` | `{total, proposed, accepted, contested, rejected, superseded}`: its decisions by the status above. |

`data.unfiled_decisions` counts decisions recorded under no project, which are in no item. The
endpoint is read only: registering, renaming and moving a project stay with the CLI.

## Kinds

Source → Target is the stored direction. The arrow label is used when the arrow runs as
stored; the reversed label when the stored target was recorded after the stored source.
`never` marks a kind whose target is an actor.

| Relation | Source → Target | Arrow label | Reversed label |
|---|---|---|---|
| `PROPOSED_BY` | Decision → Actor | proposed by | never |
| `DECISION_REQUESTED_BY` | DecisionRequest → Actor | requested by | never |
| `DECISION_REQUEST_FOR_DECISION` | DecisionRequest → Decision | asks about | answers |
| `DECISION_REQUEST_REQUIRED_OWNER` | DecisionRequest → Actor | waits on | never |
| `ACCEPTED_BY` | Decision → Actor | accepted by | never |
| `REJECTED_BY` | Decision → Actor | rejected by | never |
| `REQUEST_PROPOSED_BY` | DecisionRequest → Actor | proposed by | never |
| `REQUEST_ACCEPTED_BY` | DecisionRequest → Actor | accepted by | never |
| `REQUEST_REJECTED_BY` | DecisionRequest → Actor | rejected by | never |
| `SUPERSEDES` | Decision → Decision | supersedes | is superseded by |
| `BLOCKED_ACTOR` | Blocker → Actor | blocks | never |
| `BLOCKER_FOR_DECISION` | Blocker → Decision | waits on | unblocks |
| `BLOCKER_REQUIRED_OWNER` | Blocker → Actor | waits on | never |
| `NOTIFICATION_FOR_BLOCKER` | Notification → Blocker | is about | is announced by |
| `NOTIFICATION_RECIPIENT` | Notification → Actor | is sent to | never |
| `BASED_ON` | Decision → Evidence | based on | informs |
| `HAS_OPTION` | Decision → Option | weighs | is weighed in |
| `CHOSE` | Decision → Option | chose | is chosen in |
| `PREMISED_ON` | Option → Hypothesis | rests on | underpins |
| `PREMISED_ON_DIRECT` | Decision → Hypothesis | rests on | underpins |
| `SUPPORTS` | Evidence → Hypothesis | supports | draws on |
| `REFUTES` | Evidence → Hypothesis | refutes | is refuted by |
| `SAME_AS` | Decision → Decision | is the same as | is the same as |
| `PARTICIPATED_BY` | Decision → Actor | captured with | never |
| `INITIATED_BY` | Decision → Actor | initiated by | never |
| `PART_OF` | Project → Project | is part of | contains |
| `DEPENDS_ON` | Project → Project | depends on | is needed by |
| `FOLLOWS_FROM` | Decision → Decision | follows from | underlies |
| `ANSWERS` | Decision → Question | answers | is answered by |
| `ASK_FOR` | Ask → Question | asks | is asked by |
| `ASKED_BY` | Ask → Actor | asked by | never |

`SAME_AS` is a link between two records of one decision, never a merge: both nodes stay
as recorded. It is written by `relation.added`, and by a classified `decision` capture that
names the decision it restates (`restates_id`): the capture's node points at the restated
decision, newer to older. `recall` and the description resolvers fold linked records into
one answer (see `AGENT_FLUENT_QUERYING.md`).

Why these reverse. Most edges are written by the event that creates their source, so the
target already exists and the arrow runs as stored. The 17 kinds with a reversed label can
also be written later, by `relation.added`, `decision.superseded` or `project.linked`, or
name a placeholder that a later event fills in:

- Evidence recorded after the decision it is attached to, or a hypothesis registered after
  the evidence or option that bears on it: `BASED_ON`, `PREMISED_ON`, `PREMISED_ON_DIRECT`,
  `SUPPORTS`, `REFUTES`.
- A decision that answers a request, or unblocks a blocker, proposed after the request or
  blocker: `DECISION_REQUEST_FOR_DECISION`, `BLOCKER_FOR_DECISION`. A blocker reported
  after the notification that names it: `NOTIFICATION_FOR_BLOCKER`.
- A premise decision, a linked decision, an option, a superseded decision or a parent
  project recorded after the source: `FOLLOWS_FROM`, `SAME_AS`, `HAS_OPTION`, `CHOSE`,
  `SUPERSEDES`, `PART_OF`, `DEPENDS_ON`. A `SUPERSEDES` reverses only when the decision
  named as the superseder was recorded before the decision it supersedes.
- A question recorded after the decision that answers it: `ANSWERS`. A capture appends
  `question.recorded` after the proposal it is caused by, so the first answer to a question
  is older than its question; a later answer to the same question is newer, and its arrow
  runs as stored.
- An ask names a question: `ASK_FOR`. `loomtracer ask` always ensures the `Question` node
  exists before the `Ask` node that names it (in the same call when the question is new), so
  in practice this arrow never reverses; it carries a real reversed label anyway because its
  target is not an `Actor`.

A `Reaffirmation` node is created by `decision.reaffirmed` ("still holds"): its id is the event's
uuid, and it carries `decision_id` (the dependent), `premise_id` (a decision or a hypothesis),
`reason`, `reaffirmed_by` and `reaffirmed_at`, plus the usual `event_origin`. It has no edges, and
that is the rule, not an omission. An edge from the dependent to the premise is unique per
(kind, source, target) on Postgres and Kuzu, so a second reaffirmation of one pair after the
premise changed again would collapse into the first; a node per event keeps every one. The
premise is a decision or a hypothesis, which one Kuzu relation table, bound to a single
source-and-target pair, cannot span; and every relation kind costs the bulk readers one more
graph query per call. A reader takes the nodes in bulk and matches `decision_id` and
`premise_id`. Nothing else is written or moved by the event.

A view that draws edges would show the node alone, so the two graph views (`GET /v1/graph`
and the DOT export, both through `queries::reaffirmation_arrows`) add two *derived* lines read
from the node's own properties: `REAFFIRMS` ("reaffirms") to the dependent decision and
`REAFFIRMED_AFTER` ("after the change to") to the premise, which is a decision or a hypothesis.
A reaffirmation is recorded after both ends exist, so each line runs from it, newer to older,
like every arrow. `GET /v1/graph` marks them `"derived": true`; a stored edge carries no such
key. They are not edges: no query returns them, `RelationKind` has no variant for them, and a
line is left out when the node it names is not in the graph.

An `AttributionDispute` node is created by `decision.attribution_disputed` ("I never decided
this"): its id is the event's uuid, and it carries `decision_id`, `reason`, `disputed_by` (the
event's actor, always one the decision names as a decider), `disputed_at` and `said_in` (where the
person said it, null when not stated), plus the usual `event_origin`, `source` and `source_ref`.
When an agent recorded the dispute for the person (hivemind-f9xgu) the node's `source` is `agent`
and its `source_ref` is the relaying agent, and `reason` is the person's verbatim quote;
`decided_by.attribution_disputes` reads that as `relayed_by`. It has no edges, for the reason a `Reaffirmation` has none, and one more: the
acceptance it disputes keeps its `ACCEPTED_BY` edge and its actor exactly as recorded, so the
graph holds the attribution and the dispute of it side by side and nothing is rewritten. The
decision's status is untouched (a dispute is not a rejection). A reader takes the nodes in bulk
and matches `decision_id`; `decided_by.attribution_disputes` of a decision brief is the latest
node of each `disputed_by`. The graph views draw the node without a line to its decision.

A `Notification` node is created by `notification.sent` (about a blocker, with a
`NOTIFICATION_FOR_BLOCKER` edge) or by `suggestion.surfaced` (about a finding: it carries
`finding_id` and `decision_id` and has only the `NOTIFICATION_RECIPIENT` edge, never a
blocker edge). `notification.acknowledged` annotates either with `ack_at`, `snooze_until`
and `action`.

## Forward references

`decision.requested`, `blocker.reported`, `notification.sent` and classified captures
may name a `Decision`, `Blocker`, `Hypothesis` or `Evidence` id that no event has
recorded yet. The projector creates a placeholder node for it at that event, so the edge
never dangles. The event that really records the node fills the placeholder in later and
becomes the node's `event_origin`, so an arrow that first ran as stored turns around once
the node it names is really recorded.

## Changing a direction or a label

The stored direction of a kind, and the two labels it carries, are part of the wire
contract: the direction changes the Kuzu rel tables, the projected Postgres rows and every
query, and a label is what a UI shows. Either needs the owner's sign-off recorded on the
tracking bead before the code lands. Any change here also has to keep
`RelationKind::arrow_labels` and this table in step; the doc test fails otherwise.
