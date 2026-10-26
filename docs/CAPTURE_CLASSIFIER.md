# Capture Classifier Design

Status: design for bead `hivemind-m1-5-hidden-capture-jtkmje.3`.

This document specifies the batch capture classifier. It reads a small
text-only batch of recent agent activity and returns capture candidates, or an
empty result. It is a layer-3 suggestion component: it does not write ledger
events, query the graph, rank existing decisions, deduplicate, or resolve
disagreement.

Three workers run it today: the optional server-side worker
(`src/classifier.rs`, which needs `ANTHROPIC_API_KEY` and a SQLite server), the agent-seat
`/loomtracer-capture:classify-queue` command, and the keyless
`/loomtracer-capture:batch-capture` command (a Haiku subagent inside the agent's
own session). The canonical write path remains the commands layer: each worker
hands its result to it.

**This is not the classifier behind `/capture` with `--kind` omitted.**
`/loomtracer-capture:capture` does not call this classifier, and no
`loomtracer-classifier` subagent ships. `capture.sh` classifies the one text it was
given with its own, smaller prompt into four kinds (`decision`, `evidence`,
`hypothesis` or `none`; it has no capture path for blocker or notification). Which
classifier it runs, the environment variables that replace it, and what it does
when none is available are in
[`plugins/loomtracer-capture/commands/capture.md`](../plugins/loomtracer-capture/commands/capture.md).
The five kinds in this document are the batch classifier's.

## Model Choice

Use Claude Haiku 4.5 as the default classifier model. Prefer the pinned API
model id `claude-haiku-4-5-20251001`. The scorer (see
[`DECISION_SCORING.md`](DECISION_SCORING.md)) makes the same default choice
for the same reasons.

An operator override is implemented, not aspirational: set
`LOOMTRACER_CLASSIFIER_MODEL` (and, independently, `LOOMTRACER_SCORER_MODEL` for
the scorer) to any Anthropic model id to replace the default for that worker.
An empty or unset value falls back to the pinned default. This is a single
flat override per worker, not multi-model routing — there is no per-batch or
per-decision model selection, and the classifier and scorer are configured
independently of one another. The model id actually used for a given
classification or scoring call is recorded in that event's payload
(`classifier_model` on `ingest.batch_classified`, `model` on `decision.scored`,
beside its `prompt_version`), so a changed override is visible in the ledger, not
just in process environment.

As of 2026-09-07, Anthropic's model overview lists the current production
lineup as Claude Fable 5.1, Claude Opus 5, Claude Sonnet 5, and Claude
Haiku 4.5. Haiku 4.5 remains the fastest model in the lineup, with
near-frontier intelligence, a 200k context window, 64k max output, and
$1 / $5 per million input/output tokens. Sonnet 5 is the next tier up: 1M
context, 128k max output, and $2 / $10 pricing. Opus 5 and Fable 5.1 are
priced and positioned for complex agentic coding and demanding long-horizon
reasoning respectively, well beyond what this classifier needs.

Haiku 4.5 is still the right tier because the classifier task is small,
latency-sensitive, mostly qualitative, and conservative misses are acceptable.
The prompt asks for a handful of node-kind decisions over 3 to 5 recent turns,
not long-horizon planning or full-transcript reasoning. Sonnet 5 is the
fallback choice — via the `LOOMTRACER_CLASSIFIER_MODEL` override above — if
real-session tuning shows Haiku missing durable decisions that humans and
Sonnet consistently catch, but that should be proven with false negative
samples before paying the extra latency and cost.

Use Claude structured outputs for the API-backed implementation. Anthropic
documents structured outputs as generally available for Haiku 4.5, and the
feature guarantees valid JSON against a supplied schema in ordinary non-refusal,
non-token-limit cases.

References:

- Anthropic models overview:
  https://platform.claude.com/docs/en/about-claude/models/overview
- Claude Haiku 4.5 model page:
  https://www.anthropic.com/claude/haiku
- Claude structured outputs:
  https://platform.claude.com/docs/en/build-with-claude/structured-outputs
- Claude Sonnet 5 model page:
  https://platform.claude.com/docs/en/models/sonnet-5/overview

## Input

The classifier input is a batch of recent session turns, rendered as text. Do
not include raw tool-call payloads, full command output, file diffs, screenshots,
or binary/media content. If the caller summarizes a tool result, label it as a
summary and keep it short.

Recommended batch envelope:

```json
{
  "batch_id": "session-id:offset-range",
  "agent_tool": "claude|codex|other",
  "session_id": "...",
  "turns": [
    {
      "turn_id": "...",
      "role": "user|assistant|system|tool-summary",
      "text": "...",
      "truncated": false,
      "ts": "2026-06-16T06:58:10Z"
    }
  ]
}
```

Start with the most recent 3 to 5 turns. Four turns is the default. The Stop
hook should submit the latest small batch in the background after the agent
finishes a turn. The user's next turn must not wait for this classifier.

Truncation must be explicit. If a turn is shortened before classification, set
`truncated: true` and include a clear marker in the text. Do not silently trim a
batch and present it as complete.

`ts` is optional and is this turn's own time in the source transcript —
Claude Code and Codex JSONL logs carry one per turn — not when the batch was
submitted or classified. `record_ingest_batch` preserves it verbatim on the
`ingest.batch_received` event; omit it and it stays absent, never defaulted to
submission time.

Classification reads it through `source_turn_id` (see
[Source turn and question](#source-turn-and-question)): a derived decision is
recorded at the time of the turn it came from. A turn with no `ts` is never
given one.

## Batching Parameters

- Batch size: start with 4 recent turns; permit 3 to 5 through configuration.
- Input content: text only, no raw tool payloads.
- Per-turn budget: keep each turn concise enough that the whole request remains
  comfortably below 8k input tokens.
- Output budget: cap at roughly 1200 tokens; most valid outputs should be under
  300 tokens.
- Sampling: low temperature, no extended thinking.
- Latency target: warmed p95 under 2 seconds per batch on Haiku 4.5.
- Timeout: if the background classifier exceeds 5 seconds, drop the attempt and
  log the timeout for later tuning.
- Retry policy: no synchronous retry from the Stop hook. Retry only in offline
  evaluation tooling. The server worker does not retry either: a batch whose call or
  write failed (a timeout, a reply cut off at the output budget, a refused write) is
  logged once and left until the next UTC day. A reply the model stopped at `max_tokens`
  is reported as that.

Structured-output schema compilation may add first-use latency. The 2 second
target applies to the warmed schema path.

## Output Schema

The classifier returns exactly one JSON object:

```json
{
  "captures": [
    {
      "kind": "decision",
      "title": "Use Haiku 4.5 for capture classification",
      "rationale": "The task is small, latency-sensitive, and tolerant of conservative misses.",
      "topic_keys": ["capture", "classifier", "agents"],
      "evidence_ids": [],
      "options": ["haiku", "sonnet"],
      "chosen_option": "haiku",
      "confidence": 0.86
    }
  ]
}
```

`captures` may be empty, and an empty array is the expected result for most
batches. There is no `"none"` kind.

Allowed `kind` values:

- `decision`
- `evidence`
- `hypothesis`
- `blocker`
- `notification`

The classifier no longer produces `decision-request` (hivemind-bbnw.12): a
request goes to your tracker, not into decision memory. The kind stays in the
`ingest.batch_classified` schema and in the graph, so a classification recorded
before this change replays exactly as it did (see
[Recorded `decision-request` captures](#recorded-decision-request-captures)).

Field rules:

- `title`: concise, durable label for the thing worth capturing.
- `rationale`: why this should become organizational memory, not a transcript
  summary.
- `topic_keys`: 1 to 5 lowercase search keys.
- `evidence_ids`: only IDs already present in the input batch; never invent
  graph IDs.
- `options`: for decisions, list the meaningful alternatives if visible;
  otherwise `null`.
- `chosen_option`: for decisions, the chosen path if visible; otherwise `null`.
- `confidence`: classifier self-estimate from 0.0 to 1.0 for offline prompt
  tuning. It is not graph provenance, not a query ranking score, and must not be
  presented as authoritative confidence unless stored with the classifier model,
  prompt version, schema version, and source batch id.

### Source turn and question

Two more fields tie a capture to the transcript it came from. Both are
`null` unless the text says so; neither is inferred.

- `source_turn_id`: the id of the one turn the capture came from, copied from
  that turn's header (`[user turn <id>]`, how the batch is rendered). For a
  decision it is the turn in which the choice was made or stated.
- `question`: on a `decision`, the question it answers, when the text states
  one, in the words it was asked in. `null` on every other kind.

The classifier only judges which turn and which words. The write path checks
that the turn is one of the batches being classified (refusing the whole
classification otherwise), reads that turn's own `ts` into the capture's
`source_ts`, and applies the rules below. It never judges, and it never takes a
time from the classifier.

- A decision is recorded at its turn's time (`occurred_at`). With no turn, or a
  turn with no `ts`, it reads at the newest turn time of its batch (hivemind-ohik),
  taken from the received batch when the graph is built, since the classifier
  can run days after the session. A batch whose turns carry no `ts` at all keeps
  the classification's own time.
- A `decision` with a `question` links to the question with `ANSWERS`, matched
  on the normalized text exactly as `capture --question` is: a decision that
  states the same question as an ask already on the record (up to case, spacing
  and trailing punctuation) shares its `Question` node, so the ask reads as
  answered.
- No ask is written for an agent deciding alone: a decision records its
  question and nothing more. No "first raised" time is inferred from earlier
  mentions. An ask comes from `loomtracer ask` or the `AskUserQuestion` hook, at
  the moment the question was put.

A decision answering an ask from an earlier session links only if it states the
question in the same words; the classifier is not shown earlier asks, so an
answer in other words is recorded unlinked rather than guessed.

### Recorded `decision-request` captures

Classifications recorded before hivemind-bbnw.12 may hold `decision-request`
captures, and `emit ingest.batch_classified` still accepts the kind, so they
replay unchanged:

- A `decision-request` with a `question` and a turn that has a `ts` writes one
  `question.asked` at that turn's time, by the actor named on the request. With
  no actor named, the turn's role decides: a request in an `assistant` turn is
  credited to whoever submitted the batch (that agent spoke it), and a request
  in a `user` turn (or any other role) writes no ask, because the batch does not
  record which human spoke and a human's question is never credited to an agent.
  A request with no question, or whose turn has no `ts`, writes no ask.
- The same transcript ingested again writes the same ask once.

Nothing here teaches the kind: neither classify worker offers it, and the
recorded type is not removed from the graph.

### Who decided

`actor_id` is who proposed, made or reported the item; `accepted_by` and
`rejected_by` are who decided it. They are different people more often than not,
and a decision's status follows only the second: a capture that names an acceptor
reads `accepted`, one that names none reads `proposed`. The projector never
derives an acceptance from `actor_id` (hivemind-s0ra), so the judgement is the
classifier's, and both workers (the server-side classifier in `src/classifier.rs`
and the agent-seat `classify-queue` command) apply the same rule:

- List an actor in `accepted_by` or `rejected_by` only when the text shows that
  actor deciding: stating the choice themselves ("we'll go with X"), approving or
  accepting it ("approved"), or rejecting it.
- When the text reports someone's call ("Priya approved X"), the acceptor is
  Priya, and the one reporting it is `actor_id` at most.
- Proposing, recommending, reporting or relaying a choice is not deciding it, and
  neither is carrying out a call somebody else made. A proposal nobody has
  accepted, a call passed on without saying who made it, and a speaker the text
  does not name all leave `accepted_by` empty, so the decision stays `proposed`.
- One agent decides for itself: the session agent. A batch's text opens with
  `Session agent: <id>` (the agent the projector credits for the session: the
  batch's submitter when it is an `agent:` id, else `agent:<agent_tool>:hook`;
  `classify-queue list` shows the line as the first line of each `batch_text`).
  When the session agent's own turn shows it making a call that nobody else made,
  and carrying it out ("I'm keeping the retry count at three", "went with X and
  pushed it"), the call was its to make: that id, copied exactly, goes in
  `accepted_by`, and the decision reads `accepted`, decided by that agent. A call a
  person or another actor made is theirs however the agent carried it out: they are
  listed, never the session agent. A recommendation, a suggestion or a proposal that
  waits for someone ("I recommend X, your call", "should we use X?", "put on a
  branch for review") stays empty, as does a call the session agent only reports
  from someone else, and another agent named in the text that only narrates or
  carries out its own choice is proposing it. This is the whole of the exception (hivemind-zdsh.23
  reverses what hivemind-u70b ruled for agents): the projector still derives nothing
  from `actor_id`, the rule for people is unchanged, and a decision recorded before
  the rule keeps what it was recorded with.
- The session agent's own call reads the same whether or not `actor_id` also names it:
  `review` is `self_accepted` and `why` prints one `decided by:` line. A draft that names
  no `actor_id` keeps `proposer_id` empty (nothing infers a proposer); its review reads
  the session agent as the only actor on record, because it is the sole acceptor and took
  part in the session (`PARTICIPATED_BY`). Another acceptor, a person who accepts the draft
  later, or an agent that was not in the session makes it `peer_reviewed`
  (hivemind-26br4).

`tests/fixtures/transcripts/` holds one transcript for each case (decided,
the session agent deciding alone, the agent carrying out a person's call, proposed
only, reported by someone else, relayed) with the answer the rule gives and the status it lands in;
`cargo test who_decided` replays each answer through the write path and the
status read. The fixtures are the answers the rule gives, not what a model
answers: a change to the wording is probed against a model before it ships.

For API structured outputs, use a JSON Schema equivalent to the object above
with `additionalProperties: false`. Keep all fields required; nullable values
should be represented explicitly.

## Kind Guidelines

Capture a `decision` when the batch contains a chosen path among plausible
alternatives and the choice passes the decision test below. It may be proposed
rather than accepted, but it must have a selection and a reason. If the batch
only asks someone to choose later, or asks for work to be done, it is a request,
not a decision: a request goes to your tracker, so capture nothing for it.

The decision test has two legs, and a choice needs only one of them:

1. **It binds something.** The choice reaches beyond the edit in front of its
   author: other people, other code, or a later choice depend on it.
2. **Undoing it is costly.** Reversing it is more than an ordinary edit.

Choosing "apple" over "Apple" in one comment binds nothing and is cheap to undo:
not a decision. Merging tonight despite a failing check binds nothing later but
is costly to undo: a decision. A rename that users' scripts call binds them: a
decision. The capture verbs ask the same two questions of every capture (what
does it bind, what would undoing it cost; see
[`AGENT_DECISION_CAPTURE.md`](AGENT_DECISION_CAPTURE.md#what-the-decision-binds-and-what-undoing-it-costs)),
and record a choice that answers "nothing" and "cheap" marked below the decision
floor. The classifier's own captures are not asked, so the test is in its
prompt: it omits what fails both legs.

Capture `evidence` when the batch contains an observation with a referent that
can support or refute later choices: a test result, production symptom, external
document fact, measured latency, user research finding, or verified constraint.
Do not capture raw command output unless the observation has been interpreted.

Capture a `hypothesis` when the batch states a proposition that is being tested
or may later be supported/refuted. Prefer hypothesis over evidence when the
claim is uncertain.

Capture a `blocker` when progress is materially blocked by a dependency,
permission issue, missing artifact, failing gate, unavailable service, or
unresolved external decision. Routine "I am still working" status is not a
blocker.

Capture a `notification` only for durable announcements another actor would
need after session restart: handoff state, merge-ready status, rejected
integration with proof, or a completed verification result. Do not capture
ordinary gc/br claim mechanics.

Do not capture:

- synthetic seed data, fake demo decisions, or fixture content unless the user
  is explicitly designing the fixture itself;
- gc/br plumbing chatter, branch naming, routine claim/drain/merge mechanics, or
  gate output with no product implication;
- raw tool-result noise, stack traces, logs, or diffs without a summarized
  observation;
- private scratch notes, plans, TODO lists, or task tracking;
- requests for work, or asks for someone to choose: a request goes to your
  tracker;
- repeated statements already captured in the same batch;
- a borderline choice that fails both legs of the decision test: it binds
  nothing and is cheap to undo. Omit it and tune later if humans identify a real
  miss.

## Classifier Prompt

This is the design baseline of the prompt the batch classifier runs, with the
output schema enforced by structured outputs where available. No
`.claude/agents/loomtracer-classifier.md` ships. The shipped prompts live in
`CLASSIFIER_PROMPT` in `src/classifier.rs`, in `SKILL.md` and in
`commands/batch-capture.md`; a test holds their decision-test paragraph identical
to the one below, and they add fields this baseline leaves out (who decided; the
server-side prompt also has the relational ids).

```markdown
You are the Loomtracer capture classifier.

Loomtracer stores organizational decision memory: durable decisions, evidence,
hypotheses, blockers, and notifications with provenance. It does not store chat
history, task tracking, private scratch notes, or raw tool logs.

Read the batch of recent agent activity. Return only JSON matching the capture
schema. Most batches should return {"captures":[]}.

Capture a decision only when the text shows a chosen path among plausible
alternatives, gives or implies a reason, and passes the decision test: the
choice binds something beyond the edit in front of its author (other people,
other code, a later choice), or undoing it would be costly. Choosing "apple"
over "Apple" in one comment binds nothing and is cheap to undo, so it is not a
decision. Merging tonight despite a failing check binds nothing later but is
costly to undo, so it is. Text that asks for work to be done, or only asks
someone to choose, is a request, not a decision: a request goes to your
tracker, so capture nothing for it.

Capture evidence only when there is an observation with a referent that could
support or refute a later decision, such as a test result, production symptom,
verified external fact, measured latency, or explicit user research finding.

Capture a hypothesis only when the text states a proposition being tested or a
claim that may later be supported or refuted.

Capture a blocker only when progress is materially stopped by a dependency,
permission issue, unavailable service, missing artifact, failing gate, or
unresolved external decision.

Capture a notification only when another actor would need the announcement
after restart, such as handoff state, merge readiness with proof, rejection
state, or completed verification.

Do not capture synthetic test data, fixture/demo content, gc/br plumbing,
branch-name mechanics, routine gate chatter, raw command output, stack traces,
file diffs, TODO lists, status narration, generic plans, or requests for work.
Settle a borderline choice with the decision test above: if it binds nothing and
is cheap to undo, omit it.

Never invent evidence ids. Use only ids present in the input. Keep titles short.
Use 1 to 5 lowercase topic keys. Confidence is your self-estimate for offline
tuning, not authoritative truth.

Write `title` as one plain sentence a stranger understands, and keep
`rationale` readable without the tracker. If a tracker or ticket id (a bead,
Jira, GitHub issue) appears in the source text, leave it out of the title and
rationale — it is not part of the captured content.
```

## Evaluation And Tuning

Tune against real session logs after the implementation bead lands. Review false
positives first because noisy capture damages trust faster than conservative
omission. Then review false negatives for durable design decisions missed by
Haiku.

Keep the first tuning loop prompt-only. Do not add hard thresholds, similarity
dedupe, graph reads, or write-path inference to compensate for classifier
mistakes. If the classifier is too noisy, tighten the prompt and examples. If it
misses real decisions that Sonnet catches consistently, record that evidence and
consider a separate bead to revisit the model tier.

## Dogfood Recording

This design choice is captured as `decision.proposed` in the rig ledger:

- Event id: `decision-74dfd81b-7c84-4591-89e7-20732b449fe7`
- Captured at: `2026-06-07T03:19:23Z`
- Actor: `agent:codex:019e9fcd-566f-7851-9932-8ba41f5a5c30`
- Ledger: `/data/projects/hivemind/loomtracer/ledger.sqlite`
- Verification: `query get_decision --id
  decision-74dfd81b-7c84-4591-89e7-20732b449fe7 --summary` returned
  `proposed` with title `Use Haiku 4.5 for capture classification` and topics
  `capture,classifier,agents`.
