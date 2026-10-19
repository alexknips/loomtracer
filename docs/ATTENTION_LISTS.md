# Attention lists and the decision timeline

Four lists say what needs a person's attention without anyone having to go looking, and a
per-decision timeline says when each thing happened. Everything on this page is read from the
ledger and the graph at read time, so a decision that gets answered, replaced or resolved leaves
the list on the next call. Nothing is ranked, scored or inferred, and nothing is written.

## Rules

- **Facts and durations only.** The lists show what happened and when. There are no averages, no
  per-person figures and no grading of anyone. The one duration is the time from a decision's
  first explicit ask to the decision, for that one decision.
- **Consistency and flexibility are read, not judged.** "The same question got different answers"
  and "a decision was revised" are facts the ledger holds; no list says whether either was good.
- **Nothing is guessed.** A decision nobody asked about has no ask time and no duration. An event
  with no timestamp shows `ts: null` (or `undated` in a summary) rather than a made-up time.
- **Nothing is hidden.** Every list says when it was cut short (`truncated`) and how to continue
  (`next_cursor`).

## Where to read them

| List | CLI | MCP tool | HTTP |
|---|---|---|---|
| Waiting | `hivemind query get_waiting_requests` | `get_waiting_requests` | `GET /v1/attention/waiting` |
| Contested | `hivemind query get_contested_decisions` | `get_contested_decisions` | `GET /v1/attention/contested` |
| Open why | `hivemind query get_open_why_decisions` | `get_open_why_decisions` | `GET /v1/attention/open-why` |
| Changed | `hivemind query get_changed_decisions` | `get_changed_decisions` | `GET /v1/attention/changed` |
| Timeline of one decision | `hivemind query why` (JSON `data.timeline`, text `timeline:` block) | `get_decision_neighborhood` (`data.timeline`) | `GET /v1/decisions/{id}/timeline`, and `GET /v1/decisions/why` (`data.timeline`) |
| Status events of one decision | not yet | not yet | `GET /v1/decisions/{id}/status-events` |

The CLI prints JSON; add `--summary` for text. The stdio server, the HTTP endpoint and the CLI
share one core per tool, so the three answer the same. Every list pages with `limit` and `cursor`
and answers in the usual envelope (`result_count`, `truncated`, `latency_ms`, `data`), with
`data.next_cursor` and `data.total_matches`.

## Waiting

Open requests: an explicit ask (`hivemind ask`, MCP `request_decision`) whose question no
decision answers yet. Oldest first. Each item has the `request_id` a later capture links to
(`capture --answers`), the question text, `asked_at` (the ask's own time) and `requested_by` (who
asked). A request leaves the list when a decision answers its question.

An ask records who asked, not whom; there is no addressee on it, so the list cannot say who is
being waited on.

## Contested

Decisions in contest, oldest first, each with why it is listed:

- `disagreement`: at least one actor accepted the decision and at least one rejected it, and
  nobody superseded it. This is the `contested` status. `accepted_by` and `rejected_by` name the
  sides.
- `conflicting_answers`: the decision is accepted and current, and another accepted, current
  decision answers the same question with a different chosen option. Both decisions are listed,
  each naming the other in `conflicts_with`. This is the `conflicting_answer` reason of
  `still_holds`.

Each item also carries `asked_at` (when the question the decision answers was first explicitly
asked, absent when nobody asked) and `decided_at` (when the decision was recorded). Nothing picks a
side or closes the disagreement. Superseding a decision takes it off the list.

## Open why

Binding decisions whose why is still to answer, oldest first: decisions in force that clear the
decision floor and rest on nothing but what their own capture brought, so nobody has tied them to
anything outside the call that recorded them. It is an open state, never a refusal: every listed
decision was written, and nothing here suggests or links a parent (finding one is the analysis
layer's work, not the write path's and not this read's).

A decision is listed when all of these hold:

- **In force.** Not superseded and not rejected. A contested or merely proposed decision is listed.
- **Binding.** Its floor verdict is `cleared`: its capturer said it binds something or is costly
  to undo. A decision below the floor is not a decision that needs a parent, and a decision whose
  answers were never asked (it predates the floor, or came by a path that never asks) has no
  verdict. Neither is listed; `left_out.below_floor` and `left_out.floor_not_asked` count the
  in-force decisions left off for that reason, so the list is never mistaken for every decision.
- **No earlier decision.** It follows from no other decision and replaces none. Naming the
  decision it overturns is the reference a pivot has.
- **No person-declared root.** No person accepted it (`human:` actor) and it records no
  delegation from a person. A ruling a person made is the root of its own why. An agent that
  scribed it is the recorder (`recorded_by`), never the decider (`deciders`).
- **Only its own capture's grounding.** Every observation, assumption and bet it rests on is its
  own: recorded before the decision's proposal, as part of the call that made it, and rested on by
  no other decision. A node a second decision rests on is reused, which is a link between
  decisions. A node recorded after the proposal was attributed later by someone else. A decision
  that declares nothing at all passes this vacuously and is listed with an empty `rests_on`.

Each item has the decision's `title`, `project`, `status`, `captured_at`, `deciders` (empty while
nobody has accepted it), `recorded_by`, the two floor answers (`binds`, `undo_cost`, and
`floor: cleared`) and `rests_on`: the observations, assumptions and bets it rests on, each with
its `kind`, `id`, `label` and, for an observation, its `source`.

A decision leaves the list the moment it follows from another decision (`hivemind ground`), a
person decides it, a second decision shares its evidence, someone attributes more grounding to it,
or it is superseded. `next_cursor` is a position in the order (the ledger offset and id of the last
item shown), so a decision recorded or answered between two pages never makes the next page skip or
repeat one. `total_matches` counts every listed decision, all pages together.

What it cannot see: the graph does not record which call recorded an observation. "Its own" is read
from what the graph holds, one decision resting on it and recorded before the proposal. An
observation recorded on its own earlier and cited by id by exactly one decision therefore reads as
that decision's own, and so does one attributed later by an edge alone (`ground` naming an existing
node). Both err toward listing the decision: a gap shown that a reader can close, never one hidden.

## Changed

Decisions that were revised or superseded, or whose premise stopped standing, inside a window,
most recently changed first. The window is `since` (default: the last seven days) to `until`,
and the list echoes the window it used in `data.since` and `data.until`. In the CLI `--since`
takes an RFC3339 timestamp, a `YYYY-MM-DD` date or a duration such as `7d`; over MCP and HTTP it
is RFC3339. For the whole ledger, pass an early date.

Each item is one decision with its `asked_at` and `decided_at` (as in the contested list) and its
`changes` in the window, oldest first. A change is one of:
`superseded` (by which decision), `retitled` (from, to), `moved` (from, to), `premise_refuted`
(which hypothesis, by which evidence), `premise_superseded` or `premise_rejected` (which
decision). Each carries its time, its actor and the ledger event that holds it (`event_origin`,
`citation_id` = `event:<offset>`). Accepting, rejecting and recording are not changes; a rejection
shows in the contested list.

`next_cursor` (`<ledger offset>:<decision id>` of the last item shown) continues with older
changes: pass it back as `cursor`. Decisions can share one offset (a superseded premise and every
decision that follows from it), and none of them is skipped at a page boundary. A change written
between pages does not shift the next page. `total_matches` counts every decision that changed in
the window.

An event with no timestamp is always in the window, undated, so nothing is hidden.

## The timeline of one decision

`timeline.entries` is the decision's dated story, oldest first in ledger order, each entry cited
by its ledger event:

| `kind` | What it says |
|---|---|
| `asked` | The question the decision answers was explicitly asked (`request_id`, `question_id`). |
| `recorded` | The decision was recorded. |
| `accepted`, `rejected` | Someone accepted or rejected it (`reason` when given). |
| `superseded` | A newer decision replaced this one (`by_id`). |
| `supersedes` | This decision replaced an older one (`replaces_id`). |
| `retitled`, `moved` | Its title or project changed (`from`, `to`, `reason`). |
| `premise_refuted` | A hypothesis it premised on was refuted (`hypothesis_id`, `evidence_id`). |
| `premise_superseded`, `premise_rejected` | A decision it follows from was superseded or rejected. |

The header carries `asked_at` (the earliest explicit ask), `decided_at` (when the decision was
recorded, the time `why` shows as `answered_at`) and `asked_to_decided_seconds`. The last two are
absent when nobody asked first, or when the only ask came after the decision.

Propagation is written when it happened and only to the decisions that already rested on the
premise then: a `premise_*` entry appears at the moment a premise stopped standing, for the
decisions that already premised on the hypothesis or followed from the decision. A decision that
started resting on a refuted premise later was stale from the start; its `still_holds` says so.

A classified capture can state positions inside itself (`accepted_by`, `rejected_by`,
`supersedes_id`, `premised_on_ids`, and `refutes_ids` on an evidence capture) and has no event of
its own for any of them. The timeline and the changed list read those from the graph and treat them
as the entries they stand for, by the same rules. A decision's own positions share the capture's
offset and are dated when the decision was said (the source turn's time when the capture names one,
else the batch's), as in the status events below. A refutation is dated by the classification
event, since evidence carries no time of its own; its `actor_id` is the classification's recorder.
A position the ledger also holds as its own event is listed once, as that event.

Entries follow the ledger's order. A decision imported with its source's own time can show an
entry whose `ts` is earlier than the one before it; both are true.

## The status events of one decision

`GET /v1/decisions/{id}/status-events` lists only the events that set the decision's status, newest
first, so a reader can tell the status of a past moment from the status now. The status is derived,
never stored, so nothing is edited: this reads the log.

```json
{
  "decision_id": "decision-...",
  "status": "superseded",
  "events": [
    {
      "event": "superseded",
      "occurred_at": "2026-09-24T10:02:11Z",
      "offset": 412,
      "actor": {"id": "human:alex", "kind": "human"},
      "superseded_by": "decision-...",
      "status_after": "superseded"
    }
  ]
}
```

| Field | Meaning |
|---|---|
| `event` | `proposed`, `accepted`, `rejected` or `superseded`. |
| `occurred_at` | The event's own ledger time; `null` when the event carries none. |
| `offset` | The ledger offset (`event_origin`) of the event that did it. |
| `actor` | `{id, kind}` of who proposed, accepted, rejected or superseded it; `kind` is `human`, `agent` or `unknown`. `null` only when the graph names no proposer. |
| `delegated_by` | On `accepted`, when the event says an agent decided within a human's delegated scope. |
| `superseded_by` | On `superseded`: the newer decision. |
| `status_after` | The status the one status rule gives after this entry and every entry before it. |

`status` is the status now and equals the newest entry's `status_after`. One entry per event: a
second acceptance by the same actor is its own entry and leaves `status_after` unchanged;
concurrent supersessions each get an entry. `truncated` is always `false`: a decision's status
events are all returned.

A decision whose capture states the acceptance, rejection or replaced decision itself (a classified
capture's `accepted_by`, `rejected_by`, `supersedes_id`) has no event of its own for it. Those
entries share the capture's offset and are dated when the decision was recorded. A replacement
made this way is cited at the replacing decision's own offset, dated when that decision was
recorded and credited to its proposer. A decision proposed and accepted in one capture shows both
at the same time.

## What it costs

The dated facts come from the ledger events themselves, because the graph keeps no per-edge
times. `get_changed_decisions` and the timeline read the ledger's events once per call, the same
read `get_decisions_changed_since` does, plus the graph edges a classified capture states (its
acceptances, rejections, replaced decisions, premises and refutations); the waiting and contested
lists read the graph only. The status events read only the ledger's accept, reject and supersede
events, plus the decision's own edges in the graph.
