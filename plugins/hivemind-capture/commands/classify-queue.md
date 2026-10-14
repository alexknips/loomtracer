---
name: classify-queue
description: Drain the HiveMind classification work queue using the agent's subscription seat (Worker A)
argument-hint: '[--limit N]'
allowed-tools: Bash(hivemind classify-queue list:*), Bash(hivemind classify-queue submit:*)
---

# Classify pending HiveMind batches (Worker A — subscription seat)

Drain the pending classification queue up to `$ARGUMENTS` batches (default 20).

Run this once per session end (e.g. from a `Stop` hook), not per batch and not
on an hourly timer — Worker A spends the agent's own subscription-seat model
call to classify, and a shared daily cap (~150 classification calls city-wide;
see `HIVEMIND_CLASSIFY_DAILY_CAP`) protects against runaway spend across every
session in the city. Pass `--session-id` to scope listing to just the calling
session's own batches, so concurrent sessions never redundantly process each
other's queue.

On a server-backed cell, set `HIVEMIND_API_URL` (and `HIVEMIND_API_KEY` if the
server requires auth) so `classify-queue list`/`submit` talk to the server
over HTTP instead of opening a local ledger — no database credential in the
agent's environment (Option B, hivemind-zdsh.2). Leave both unset for the
local `--hivemind-dir` SQLite path.

## Workflow

**Step 1: List pending batches**

```bash
hivemind classify-queue list --json --limit ${HIVEMIND_CQ_LIMIT:-20} \
  ${HIVEMIND_CQ_SESSION_ID:+--session-id "$HIVEMIND_CQ_SESSION_ID"}
```

The reply is an object: `batches` holds the pending batches (at most `--limit`) and `pending_total`
the whole queue's depth. (The v0.7.0 release prints the bare list of batches for a local ledger, and
no `pending_total` from a server: read the list as `batches`.) If `batches` is empty, report "No
pending batches" and stop. If the request
fails with a daily-cap message, report it and stop — the batches stay pending
and will be picked up by a future run once the cap resets (UTC midnight). If
it refuses because `--hivemind-dir` holds no ledger (exit 2, naming the folder),
report that this project has no ledger, so nothing is queued here, and stop.

**Step 2: Classify each batch**

For each batch returned in step 1, read the `batch_id` and `turn_count`.
Do NOT fetch the raw turns text — it is embedded in the batch record's `batch_text`
field via the list command. Use the classification rules below.

**Classification rules (same as the server-side classifier):**

Capture an item ONLY when the conversation text shows:
- **decision**: a chosen or rejected path among plausible alternatives, with a reason,
  that passes the decision test: the choice binds something beyond the edit in front
  of its author (other people, other code, a later choice), or undoing it would be
  costly. Choosing "apple" over "Apple" in one comment binds nothing and is cheap to
  undo, so it is not a decision. Merging tonight despite a failing check binds nothing
  later but is costly to undo, so it is. Text that asks for work to be done, or only
  asks someone to choose, is a request, not a decision: a request goes to your
  tracker, so capture nothing for it.
- **evidence**: a real observation (test result, measurement, verified fact) that
  supports or refutes a decision or hypothesis.
- **hypothesis**: an explicit assumption or prediction being relied on before verification.
- **blocker**: progress is materially stopped (dependency, approval, unavailable service,
  unresolved decision).
- **notification**: a handoff, merge-ready signal, or rejection state another actor
  needs after restart.

**Do NOT capture:** synthetic test data, fixture/demo content, routing chatter (gc/br
plumbing), stack traces, raw command output, status narration, todos, requests for work,
or private scratch notes. Settle a borderline choice with the decision test: if it binds
nothing and is cheap to undo, leave it out. When in doubt, return an empty captures array.

**Who decided (`accepted_by` / `rejected_by`)**

These two fields name who DECIDED, not who spoke. List an actor only when the text
shows that actor deciding: a person stating the choice themselves ("we'll go with X",
"I'm going with X"), or any actor approving, accepting or rejecting a choice
("approved", "yes, do it" in answer to a proposal). When the text reports someone's
call ("Priya approved X"), list Priya, not whoever reports it. Proposing,
recommending, carrying out, reporting or relaying a choice is not deciding it: an
actor who only suggests X ("I suggest X", "we could use X"), only implements it, only
says it was decided, or passes on a call without saying who made it is not listed, and
a proposal nobody has accepted stays empty however firmly it is worded. An agent
narrating or carrying out its own choice is proposing it, so the decision stays
unaccepted until a person is shown deciding it. A speaker the text does not name gives
no one to list. Never infer an acceptor, and never copy `actor_id` into `accepted_by`;
list every actor the text shows deciding. A decision left with no acceptor is recorded
as proposed, which is the honest state until someone is shown deciding it.

**Step 2b: Check each decision against what is already recorded**

A decision that was already recorded (by an earlier session, or by this one) must
not be recorded again as a second decision. For each `decision` you are about to
submit, look it up by its own title:

```bash
hivemind query recall "<the decision's title>"
```

If one of the results is the same decision (the same question, answered the same
way, made again or relayed once more), put that result's `decision_id` in the
capture's `restates_id`. Keep your capture: it records that the decision was made
again, and `recall` and `why` then show the two as one decision. A decision that
only concerns the same topic, answers a different question, chooses differently,
or replaces the earlier one is not a restatement; leave `restates_id` out. When
unsure, leave it out: two records of one decision cost little, and two different
decisions called one cost more.

**Step 3: Submit classifications, grouped by session**

Batches from the same `session_id` came from one conversation — submit them
together in a single call so one write covers the whole session's captures
(pass `--batch-id` once per batch, comma-separated):

```bash
hivemind classify-queue submit \
  --batch-id <batch_id_1>,<batch_id_2> \
  --captures '<json array of CaptureItem objects covering all listed batches>'
```

A batch with no session (`session_id` empty in the list output) submits alone.

Take every `--batch-id` from the list output. An id the ledger never received is
refused (exit 3, naming the id, whether the ledger is local or behind `HIVEMIND_API_URL`) and
nothing is recorded, so a mistyped id never leaves the real batches in the queue under a
classification that covers nothing.

Each CaptureItem JSON object must include:
- `kind`: "decision" | "evidence" | "hypothesis" | "blocker" | "notification"
- `title`: one plain sentence a stranger understands (required). If a tracker
  or ticket id (a bead, Jira, GitHub issue) appears in the source text, leave
  it out of the title — it is not part of the captured content.
- `rationale`: the why, in the words of the text, readable without the
  tracker (required)
- `topic_keys`: array of topic strings (required, may be empty array)
- `evidence_ids`: array of existing evidence IDs referenced (required, usually empty array)
- `options`: array of option strings or null
- `chosen_option`: string or null
- `extraction_confidence`: float in [0,1] — your confidence this item is genuinely durable

Optional fields (omit rather than null unless needed):
- `expressed_confidence`: "low" | "medium" | "high" — only when stated in the text
- `supersedes_id`: ID of a decision this supersedes, only if named in the text
- `restates_id`: ID of a recorded decision this decision states again (step 2b), only on
  a `decision`. The id must be a decision already recorded; submit refuses an unknown one,
  naming it. A decision restated from the very same moment (the same transcript submitted
  twice) is not recorded a second time, and the reply's `restated` list says so. If submit
  is refused because `restates_id` is an unknown field, the cell runs an older hivemind
  than this plugin: submit the same captures again without it.
- `source_turn_id`: the id of the one turn this capture came from, copied exactly from that
  turn's header in `batch_text` (`[<role> turn <id>]`): for a decision the turn in which the
  choice was made or stated. Leave it out when you cannot tell which single turn. It must be a
  turn of the batches you submit; submit refuses an unknown one, naming it. The decision is
  then recorded at that turn's own time rather than at submit time.
- `question`: only on a `decision` (the question it answers, when the text states it), in the
  words it was asked in. Never write a question the text does not contain. A decision that
  states the same question as an ask already on the record (case and trailing punctuation do
  not matter) is linked to it. If
  submit is refused because `source_turn_id`, `source_ts` or `question` is an unknown field,
  the cell runs an older hivemind than this plugin: submit the same captures again without them.
- `actor_id`: who proposed, made or reported the item, only if named in the text. It does
  not say who decided: it is never an acceptance
- `accepted_by`: array of the actors the text shows accepting or choosing it (see "Who
  decided" above); empty when none is shown
- `rejected_by`: array of the actors the text shows rejecting it; empty when none is shown

**Process all batches in sequence.** Report a summary: batches processed, total captures
written, any errors.

## Bounds

- Worker A is subscription-seat bound: each batch consumes one model invocation.
- Large backlogs drain across multiple `/classify-queue` runs — the queue is persistent.
- Queue depth is visible via `hivemind classify-queue list --json --limit 1 | jq .pending_total`,
  on a local ledger and a server-backed cell alike (the reply says `truncated` when more are
  pending than it lists). The v0.7.0 release has no `pending_total`; `scripts/check-classify-queue.sh`
  counts the queue on it too.
- The server-side classifier (Worker B) and Worker A drain the same queue; both can run;
  last-writer-wins per batch_id is idempotent.
