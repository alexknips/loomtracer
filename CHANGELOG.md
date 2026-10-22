# Changelog

All notable changes to HiveMind are documented here.
This project adheres to [Semantic Versioning](https://semver.org/).

## Unreleased

### Breaking changes

- **An MCP tool call that sends an argument the tool does not take is refused.** A tool used to
  read the keys it knew and skip the rest, so `recall_decisions` with `query` (it takes `q`) ran
  unfiltered and `link_supersession` with `decided_by` succeeded without carrying it, while
  `POST /v1/decisions/{id}/supersession-links` refused the same field with 400. Over stdio and
  `POST /mcp` alike, a `tools/call` whose top-level `arguments` hold a key outside the tool's
  `inputSchema.properties` now answers a tool error (`isError: true`) that names the key and
  lists the tool's real arguments (`unknown argument `query`; recall_decisions takes: ...`), and
  nothing is read or written. A client that sent a stray key and got a result must drop it.
  (hivemind-smxrq)
- **Every capture says what the decision binds and what undoing it would cost.** The CLI
  `emit decision.capture` and `supersede` take `--binds <what>` or `--binds-nothing`, and
  `--undo-cost <what>` or `--cheap-to-undo`: one of each pair is required. The MCP tools
  `capture_decision` and `supersede_decision` and REST `POST /v1/decisions` and
  `POST /v1/decisions/{id}/supersessions` take `binds: {level: nothing|something, statement?}`
  and `undo_cost: {level: cheap|costly, statement?}`, both required, a sentence with
  `something` / `costly` and none otherwise. A call that leaves either out is refused and
  nothing is written (CLI exit 2, MCP tool error, REST 400); clients that send neither are
  refused once a server runs this release. Nothing is refused for being below the floor: a
  decision that binds nothing and is cheap to undo is recorded, and the reply says so
  (`floor: {binds, undo_cost, cleared}`, plus `below_floor: true` and a note when it is below).
  `decision.proposed` gains optional `binds` and `undo_cost`; the decision node carries
  `binds_level`, `binds`, `undo_cost_level` and `undo_cost` on every backend. Raw
  `emit decision.proposed`, classifier ingest, document import, Slack, and every decision
  recorded before this release carry neither and read "not asked"; nothing is backfilled. The
  server must run this release before any client sends the fields. (hivemind-bbnw.10)
- **`score_decision` and `scan_decision_quality` return the quality profile and attention
  findings; the score, the tier and the five deductions are gone.** `score_decision` now
  returns the decision's seven dimensions (framing, alternatives, information, reasoning,
  values_tradeoffs, bias_exposure, calibration), each `assessed` (a level of `none`,
  `partial` or `solid`, the reasons and the node ids behind it) or `not_assessed` (and why),
  plus attention lines and a provenance object whose `line` reads "not yet reviewed by a
  human" when an agent authored the decision and no human accepted it. `scan_decision_quality`
  returns one page of attention findings (a bet past its check date, a premise that was
  superseded or rejected, an assumption or bet that was refuted, evidence nobody re-checked),
  each with a stable `finding_id`, the reason, the node ids and the dimensions it bears on;
  `truncated` and `data.next_cursor` page through them. Both work self-hosted, with no model.
  Removed with no alias: the `score` and `tier` fields, `reasons[].deduction`,
  `contributing_ids`, the `min_tier` argument and `--min-tier` flag, and the `since_event_origin`
  argument on the scan (pass `kinds`, `evidence_window_days`, `limit` and `cursor`, or
  `--kind`, `--evidence-window-days`, `--limit` and `--cursor`). Superseded, premise-stale,
  contested, agent-only and thin-structure signals are no longer graded as quality: they are
  outcome, staleness, status, provenance and Alternatives / Information, shown as such. The
  stdio server, the HTTP endpoint and the CLI share one core, so the three answer the same.
  `hivemind quality-scan` files one ticket per finding (title `[HiveMind] <kind>: <decision>`),
  without a score or tier; `--min-tier` and `--since-event-origin` are gone, `--kind` is new.
  `docs/DECISION_SCORING.md` describes what exists and marks Composite, Confidence and
  Reputation deferred. (hivemind-qo11.5)
- **The "did it hold up" view stops carrying quality signals.** `verify`, `why`
  (`get_decision_neighborhood`), `decision_quality_candidates` and the Markdown decision-log
  export no longer list a `thin_structure` reason: how a decision was made (options weighed,
  what it rests on) is quality, and `score_decision` reports it as Alternatives and
  Information. What stays is what happened to the decision: superseded (by which decision;
  nothing weighs how quickly), a premise that no longer stands, contested. Removed with no
  alias: the `thin_structure` reason kind and its `no_options` / `nothing_declared` fields, the
  `has_options`, `has_evidence` and `grounding_state` fields on each `decision_quality_candidates`
  record, and `signal_breakdown.thin_structure_count` on `analyze_failure_modes`. A decision
  with no options and nothing it rests on now reads "still holds: yes" with no reasons; its
  `rests_on` still says "nothing declared". `analyze_failure_modes` gains `by_condition`:
  failure rates for each quality dimension at each level (`none`, `partial`, `solid`,
  `not_assessed`). A level only sorts decisions into groups; it is never itself a failure.
  (hivemind-qo11.7)
- **The Markdown decision-log export shows each decision's quality profile, and its Outcome
  no longer lists thin structure; `quality-scan` tickets carry the dimension lines.** Every
  decision file from `hivemind export --format markdown` gains a `## Quality profile` section
  between `## Outcome` and `## Provenance`: the seven dimensions of `score_decision`, each as
  a bullet with its level and, beneath it, its reasons with the node ids they rest on, or
  "not assessed" and why, plus the attention lines when there are any. The Outcome section's
  `Reasons` no longer includes `Thin structure: ...` (a decision with no options or nothing
  declared it rests on reads as its Alternatives and Information lines instead); it lists only
  what happened to the decision. Exports written before this hold the old shape until
  re-exported. A `quality-scan` ticket body now names the finding id (`Finding ID`) and lists
  the dimensions the finding bears on (`Dimensions it bears on`), each with its level,
  reasons and ids, or why it was not assessed; the reason heading reads `Why it needs a look`.
  Neither the export nor a ticket contains a score or a tier. Library callers:
  `queries::export_decision_log` takes a fourth argument, `Option<ProfileSection>`, that
  returns each exported decision's section (pass `None` to omit it; the CLI passes
  `quality_profile::decision_log_section`), and `linear::format_issue_title` and
  `format_issue_description` take the scan's `ScanFinding`. (hivemind-qo11.6)
- **A capture under a registered project may use only the topic keys that project has
  declared.** A topic says what a decision is about; left free, every capture invents its
  own keys and recall by topic becomes a lottery. Keys are lowercase kebab (as before), and
  a project's vocabulary grows only when someone says so: a capture's `--declare-topic`
  (MCP `declare_topics`) for a key it uses, or `hivemind project declare-topic <handle>
  <key>...`. The reply lists what a capture declared; a capture that uses an undeclared key
  is refused before anything is written, naming the keys, what the project has and how to
  declare. `hivemind project show` lists a project's keys. A project that already has
  decisions adopts what they use with `hivemind project declare-topic <handle> --in-use`.
  Personal projects and captures with no project have no vocabulary and are unchanged. A new
  ledger event, `project.topic_declared`, records each declaration (upgrade the server
  before writing one); no existing event changes. Captures into a registered project that
  omit `--declare-topic` for a new key are refused, so callers that invent keys need the
  flag. (hivemind-zywz)
- **A capture from a rig the ledger has no project for is refused, not filed under your
  personal project.** With `--project-from-context` (the capture plugins, `hivemind mcp
  --project-from-context`), a session in a Gas City rig that no project in the ledger is
  anchored to is writing to the wrong ledger, which is how a rig's decisions ended up in
  another ledger. The refusal names the rig, the ledger (tenant and local directory or shared
  database, never a credential) and the ways out. A folder marker, a rig anchor or a current
  project still wins first; a supersede inherits the replaced decision's project and is not
  refused. (hivemind-zywz)
- **`hivemind migrate` renumbers causation by uuid and checks parity by uuid; its report
  changes.** The old `--to <postgres-url>` copied `causation_event_id` as the source number,
  but the destination numbers events itself, so on a tenant that already held events every
  link pointed at an unrelated event; and its parity check was `destination count >= migrated`,
  which proves nothing on such a tenant. Now each link travels as its cause's uuid and is
  renumbered to where the cause landed, an event whose uuid the destination holds is skipped,
  and parity means every source uuid is present in the destination. The report drops
  `events_migrated` and `parity_check.destination_event_count`; it gains `destination`,
  `source_event_count`, `new_events`, `already_present` and `parity_check.present_in_destination`
  / `missing`. `--to` is now optional (one of `--to` and `--to-url` is required), and a dry run
  connects to the destination to count what is already there. A source with no events, a
  causation link to nothing, and a `notification.sent` or `blocker.resolved` event whose
  payload names event ids are refused before anything is sent. (hivemind-jawy)

- **`GET /v1/decisions/why` and `/v1/decisions/verify` answer a description that matches
  nothing with HTTP 200 and `data.outcome: "not_found"`, not 404.** A miss is an answer, and
  the CLI, the plugin's `why.sh` / `verify.sh` and MCP already gave it as one; HTTP was the one
  surface that answered with a 404 and a `not_found` error body. The reply is now the same
  envelope the other three print (`{result_count, truncated, latency_ms, data: {outcome:
  "not_found"}}`), and `tests/read_surface_parity.rs` asks the same no-match question of `why`
  and `verify` through all four. A 404 stays for what does not exist: an `id` that names no
  decision, and an unknown route. A client that branched on the 404 for a description now reads
  `data.outcome`. (hivemind-996m)
- **A new ledger event, `decision.reaffirmed` ("still holds"): upgrade every reader before any
  client records one.** It says a decision that rests on a premise stood after the premise
  changed: `{decision_id, premise_id, reason}`, attributed to the actor who looked again, a
  one-line reason required, a human or an agent alike. `premise_id` is a decision or a
  hypothesis. Readers hard-fail on an event type they do not know, so a server or CLI older
  than this release stops replaying a ledger that holds one: run this release on the server
  (the cell image) first, then on every client. Nothing records the event yet; the verbs that
  do (`reaffirm`, `reexamine`, MCP `reaffirm_decision`) land after this is live, and no existing
  event changes, so an existing ledger replays unchanged. The graph gains one node kind,
  `Reaffirmation` (id: the event's uuid; `decision_id`, `premise_id`, `reason`, `reaffirmed_by`,
  `reaffirmed_at`), and no new relation kind; `GET /v1/graph` draws it with the decision it is
  about (see the surfaces entry below). `Commands::reaffirm_decision` and
  `reaffirm_decision_with_uuid` (library callers) append exactly one event and touch nothing
  else; its invariants are written in the `commands` module header.
  (hivemind-ottw.1)

### Added

- **`GET /v1/projects` lists the projects of a ledger, and `GET /v1/graph?project=<ref>` reads
  one of them.** `/v1/projects` (the query envelope; `limit` and `cursor`) answers each project's
  `handle`, `name`, `personal`, `parent` (the project it is `part_of`) and `decisions` by status
  (`total`, `proposed`, `accepted`, `contested`, `rejected`, `superseded`), plus
  `unfiled_decisions` for those filed under no project. It lists registered projects and the
  personal projects (`personal:agent:claude`) that hold a decision; it is read only, and
  registering, renaming and moving projects stay `hivemind project`. `/v1/graph?project=<ref>`
  answers that project's newest decisions and what hangs off them (every edge that touches one
  of them, with the node at its far end, which may be a decision of another project) in the
  shape `GET /v1/graph` has, with `limit` (default 200,
  at most 1000), `total_matches`, `truncated` and `next_cursor` added; pass `next_cursor` back as
  `cursor` for the older decisions. An unregistered handle answers 404. Without `project` the
  graph is as before, and `limit` or `cursor` without `project` answers 400. On a copy of the
  cell's ledger (157,563 events, 1,845 decisions; a debug build on a loaded host) the whole graph
  was 4.50 MB in 1.6 to 2.6 s of server time, and the page of its largest project (1,788
  decisions, the default 200) is 0.47 MB in about 0.6 s, with the project list another 1.8 KB.
  (hivemind-d87pp)
- **`GET /v1/decisions/search` takes `project`**: the project's own decisions, those of the
  projects it is part of, and those it depends on, each labelled by `scope`. A handle that is not
  registered answers 404. The CLI and MCP scope a question by project through recall, not search:
  `hivemind query recall --project <ref>` and the `recall_decisions` tool's `project` scope the
  same way, with the same `scope` labels; `hivemind query search` and the `search_decisions` tool
  take no `project`.
  (hivemind-d87pp)
- **`GET /v1/graph?decision=<id>` reads one decision and what hangs off it, and the web UI's
  decision page reads that instead of the whole tenant graph.** It answers `decisions`, `nodes`
  and `edges` in the shape `GET /v1/graph` has, for the decision and every node it is linked
  to (its options, the evidence and hypotheses behind them, who proposed and decided it, what it
  follows from, replaced and was replaced by), each edge once, with no paging fields: the answer
  is bounded by the decision's own links. An id the graph does not hold answers 404; a blank
  `decision`, or one sent with `project`, `limit` or `cursor`, answers 400. The server still
  walks the graph's edges to find them, so the saving is in what is sent and drawn. On a copy of
  the cell's ledger (157,563 events, 1,845 decisions; a debug build on a loaded host), the three
  reads of one decision page (the most-linked decision, 14 edges) went from 4.35 MB in 3.5 to
  3.7 s to 18 KB in 0.7 to 0.9 s, the graph read itself from 4.35 MB to 15 KB; the timeline read
  (about 1 s) is now most of what the page waits for. (hivemind-d2886)
- **A web UI in `web/` reads a HiveMind server over its HTTP API.** It is Jeff Miao's Loomtracer
  UI (a Next.js app, imported with its history from `TheChairmanMiao/neurofilament`), moved off
  Supabase: `HIVEMIND_API_URL` and a bearer token in `HIVEMIND_API_KEY`, read on the server side
  only, are all it needs. Pages: `/graph` (a project's decisions as a timeline, a trace map and
  their records, replayed in ledger order, with status on every decision and `contested` its own),
  `/search`, `/decisions/<id>`, `/review` (proposed, contested, unanswered, no stated ground) and
  `/projects`, plus Jeff's local bundle inspector and landing page. It reads and does not write;
  sign-in, teams, tokens and accepting or rejecting a decision are left out because the HTTP API
  has no endpoint for them (`web/HIVEMIND_API_MAP.md` lists each, and what the server would need).
  It has no login of its own: run it on a loopback address or behind your own access control.
  `npm ci` at the repository root, then `npm run dev --workspace=web`; `web/README.md` has the
  rest, and CI builds and tests the workspace. The Rust server is unchanged. (hivemind-hqbd9)
- **A cell can link two decisions that are already recorded as superseded-by, so a person's
  replacement is not recorded as the agent's.** The MCP tool `link_supersession`
  (`old_decision_id`, `new_decision_id`; stdio and `POST /mcp`) and REST
  `POST /v1/decisions/{id}/supersession-links` with `{"supersedes_id": "<older decision>"}`
  (`{id}` is the newer one) append the same `decision.superseded` event the local
  `emit decision.superseded --old A --new B` does, attributed to the caller, and create
  nothing: capture the newer decision first with its real `decided_by`, then link. Afterwards
  `get_decision` and `why` show the old decision as superseded, the new one keeps its decider,
  and `get_supersession_chain` walks from one to the other. An id that is not a recorded
  decision, the same id twice, and a link that would close a loop of supersessions (the old
  decision already supersedes the new one, directly or through others) are refused (MCP
  invalid-argument, REST 400) and nothing is written. A pair already linked, directly or
  through other decisions, writes nothing and replies `linked: false` with
  `already_linked_through`, so a retry after a timeout is safe. `supersede_decision` is
  unchanged: it still records a new replacement decided by the caller. (hivemind-4p0xd)
- **A supersede whose option label looks like a slug says so in its reply, like a capture.**
  `supersede_decision`, `POST /v1/decisions/{id}/supersessions` and `supersede --json` list each
  such label (`direct-cli`) under `warnings`, with the same `kind`, `option_label`, `suggestion`
  (`Direct CLI`) and `message` a capture returns, from the same check (text mode prints one
  `warning:` line per label on stderr). The replacement is recorded with the label as sent,
  nothing is refused, and a supersede with no such label replies exactly as before, with no
  `warnings` key. Raw `emit decision.proposed` stays unwarned: it is the low-level event path.
  (hivemind-1iowm)
- **A capture whose option label looks like a slug says so in its reply.** When a label is
  lowercase words joined by `-` or `_` (`direct-cli`, `name-a-upheld`), the reply of
  `capture_decision`, `POST /v1/decisions` and `emit decision.capture --json` lists it under
  `warnings`, each entry with `kind` (`option_label_looks_like_a_slug`), the `option_label` as
  sent, the `suggestion` (the words every reader shows for it, `Direct CLI`) and a `message`
  (text mode prints one `warning:` line per label on stderr, so the id on stdout stays what scripts
  read). It is a shape check on the input: the capture is recorded with the label as sent,
  nothing is refused and nothing is rewritten, and a capture with no such label replies exactly
  as before, with no `warnings` key. A lone lowercase word (`sqlite`) is a fine label and says
  nothing. (hivemind-ggoi)
- **The attention list says when a superseded premise was never followed up, and stops flagging
  decisions someone has said still hold.** A new finding kind, `propagation_unfinished`
  (`scan_decision_quality`, `get_suggestions`, `hivemind quality-scan --kind`), is flagged on a
  decision that was superseded when a decision resting on it is still open for re-examination
  more than `propagation_window_days` (14) after the supersession: "it was superseded by 'X' on
  2026-08-01; more than 14 days later, 2 of 5 decisions resting on it are not yet recorded as
  still holding". It counts, closes and ages the same way the `[UNFINISHED]` line on `verify`
  does, names no person, bears on no dimension, and moves no floor, tier or score. Only a
  supersession ages: a rejected or contested premise has no change time in the graph, and a
  refuted hypothesis is not a decision, so none of them fires it (their dependents keep their own
  findings and re-examine items). Its `finding_id` leaves out the counts, so an acknowledgement
  holds until the last dependent is looked at. And a dependent whose re-examine item is closed by
  a reaffirmation is no longer flagged for that premise by `premise_superseded`,
  `premise_rejected`, `assumption_refuted` or `bet_failed`: the same item state that stops
  `held_up` counting it on `verify`; a reaffirmation from before the change, or one the ledger
  cannot place, closes nothing. No new event or read; an existing ledger shows it on the next
  scan. `FindingKind` gains `PropagationUnfinished` (library callers with an exhaustive match),
  and `dimensions_for` answers an empty list for it. (hivemind-ottw.6)
- **A disagreement or a replacement now says what the decision rests on, so it can be taken
  upstream.** The reply of `disagree` (CLI, MCP `disagree_decision`, REST
  `POST /v1/decisions/{id}/disagreements`) gains `rests_on`: each thing the decision rests on, as
  `verify` shows it (kind, id, label, state, when it was added), and for a prior decision
  `dependents_count`, how many decisions rest on it. The replies of `disagree` and `supersede`
  gain `raise_upstream`: each prior decision the target rests on that still holds, as
  `{premise_id, label, other_dependents}` (the decision acted on, and the replacement, are not
  counted among the others); an empty list means there is nothing to raise. On the CLI the text
  reply adds one `raise_upstream:` line per premise, and `--json` carries both fields. A premise
  that is already superseded, rejected or contested is shown in `rests_on` and not offered, and an
  assumption or a bet is answered with evidence, so neither is listed. It is a hint and changes
  nothing: the disagreement or the replacement is recorded exactly as before, never dropped or
  swapped for the premise's, and nothing is written for the premise. A supersede reply's
  `rests_on` is still what the replacement rests on. No model, no new event or node, no
  migration. The `hivemind-context` skill, its `disagree` and `supersede` commands and scripts,
  the Codex prompts and the `hivemind-capture` skill now say to run `verify` first and ask
  whether the disagreement is with the decision or with a premise it rests on. A client on an
  older hivemind simply gets no such fields. (hivemind-ottw.5)
- **`hivemind reaffirm` and `hivemind reexamine`: say a decision still holds after something it
  rests on changed, and list what is waiting for that. Needs the server and every client on a
  release that knows `decision.reaffirmed`.** `reaffirm "<decision>" --because "<reason>"`
  (MCP `reaffirm_decision`, REST `POST /v1/decisions/{id}/reaffirmations` with `premise_id` and
  `reason`, and the hivemind-context plugin's `reaffirm` command) records one "still holds",
  attributed to whoever runs it, a person or an agent. It closes only an open re-examine item:
  with one open item the premise is implied, with several the open items are listed (CLI text
  and `--json`, MCP `{outcome: "ambiguous", field: "premise", items}` in the reply's `data`) and nothing is
  written until `--premise` / `premise_id` names one (an id, or words from its title). Nothing
  to re-examine is refused with the reason and nothing written (CLI exit 2, MCP tool error,
  REST 400): the premise has not changed, the decision does not rest on it, the decision itself
  was superseded or rejected, or the item is already closed, in which case the refusal names
  who reaffirmed it, when, the event's ledger number (what the first reply called `event_id`)
  and its uuid, so a retried call sees what happened. The same check runs on every transport
  (`reaffirmation::reaffirm`), because `Commands::reaffirm_decision` itself does not look at
  the graph. The reply names the decision, the premise and `covers`: the change of the premise
  it answered, by ledger offset. An ambiguous or unmatched description is a successful
  `ambiguous` / `not_found` answer, nothing written, like the other fluent verbs.
  `reexamine` (MCP `get_reexamine_items`) lists the open items, each naming the decision, the
  premise and how it changed (`superseded`, `rejected`, `contested`, `refuted`) and when;
  `--all` / `include_closed` adds the closed ones, `--decision` narrows to one decision,
  `--project` asks from one project (the rest are counted in `not_shown`, never named), and a
  long list stops at `--limit` and says so with a cursor. Both tools are registered on the
  stdio and the HTTP transport (39 tools). With `HIVEMIND_API_URL` set, both CLI verbs refuse
  when no local ledger exists and otherwise name the ledger they used, like the other verbs
  that open one. `hivemind-context` gains `reaffirm` and `reexamine` commands and scripts; an
  older CLI says it has no such command. (hivemind-ottw.4)
- **The places that show a decision now say when what it rests on changed, and whether anyone
  looked again.** `verify` (and `brief`, and the `why` brief) prints the open or closed
  re-examine entries under `still holds:` (`NO: premise superseded; re-examine open since
  2026-09-12`, or `yes — … someone said it still holds (below)` with who, when and the reason
  they gave), and `still_holds.reexamine` carries the same entries in JSON. A decision that
  itself changed (superseded, rejected, contested) gains `propagation`: `3 of 5 dependents
  re-examined; 2 open since 2026-09-12 [UNFINISHED]` — counts, never an actor. `recall`,
  `search` and the topic lookup `get_relevant_decisions` gain a marker on each match
  (`graph_context.reexamine` / `reexamine`: `{stale, open, reaffirmed}`; `stale=yes`,
  `reexamine_open=N` and `reaffirmed=N` in `--summary`), and the recall digest lists the stale
  matches, which closes a gap: recall used to carry no outcome, so a decision whose premise had
  been superseded read exactly like one that stands. The `situational` STALE cell reads
  `STALE(premise superseded, re-examine open)`, `holds(re-examined)` or `holds(re-examine open)`.
  The compact view carries `reexamine` on each premise and refuted hypothesis and `propagation`
  on a rejected or contested decision. The history rows (`recent_activity`,
  `decisions_changed_since`) give `decision.reaffirmed` its own kind `reaffirmed` with
  `reaffirmed_premise: {premise_id, reason}`, where it was a `context_change` for a moment, and
  `why`/`chain` timelines get a `reaffirmed` fact. The Markdown decision log writes a
  "Re-examination" list and a "Propagation" line in the Outcome section (without the UNFINISHED
  mark, which depends on today's date, so the same ledger exports the same file), and the TUI
  flags `!stale re-examine:N` on a result and prints the entries on the open decision.
  `GET /v1/graph` and `dump --format dot` no longer show a `Reaffirmation` alone: it is
  labelled with its reason and tied to the decision it concerns (`REAFFIRMS`) and the premise
  that changed (`REAFFIRMED_AFTER`) by two lines read from its own properties, marked
  `"derived": true` in the JSON because the graph stores no such edge. Default-on, no flag.
  A page of matches reads the grounding facts in bulk once (13 reads, however many decisions
  there are) and a request with no matches reads nothing extra; a brief of a changed decision
  reads a handful. `HistoryChangeKind` gains `Reaffirmed` and `TimelineFact` gains `Reaffirmed`
  (library callers with an exhaustive match); `get_relevant_decisions` returns
  `RelevantDecision` (the decision plus its marker, serialized flat). (hivemind-ottw.3)
- **A decision that rests on something that changed now says whether anyone looked again, and a
  "still holds" counts.** When a premise a decision rests on changes (a prior decision it
  follows from is superseded, rejected or contested; an assumption or bet it rests on is
  refuted), `verify` (`get_decision_outcome`) gains `still_holds.reexamine`: one entry per
  changed premise with the change (who, the ledger offset, the time where the graph states one)
  and its state:
  `open`, `reaffirmed` (each reaffirmation with its actor, time and reason), or the decision was
  itself superseded or rejected afterwards. A reaffirmation closes it only if its ledger offset is
  after the change, so a premise that changes again reopens it and the earlier word stays
  visible. `held_up` now ignores a stale premise once its change is reaffirmed for that decision
  (the reason stays in `reasons`); a contested premise opens an entry and, as before, never flips
  it. A dependent that was already superseded or rejected before the change has no entry. The
  library reads behind it: `queries::get_reexamine_items` (a keyset-paged list that says
  `truncated` and gives a cursor, scoped to a project one hop with the rest counted and never
  named) and `queries::get_propagation` ("N of M dependents re-examined", and `unfinished` once
  an item has been open more than `propagation_window_days`, 14 by default, a new field of
  `AttentionConfig`). No new event or node; an existing ledger shows its items on the first read.
  The CLI and MCP verbs that list and record them are `reexamine` and `reaffirm` (below). `GROUNDING_FACT_READS` is 13: one
  more bulk read, for the reaffirmations. (hivemind-ottw.2)
- **The answer to "what did we decide and why" carries the quality profile, and says how many
  days it has held, in the same response.** `verify` (`get_decision_outcome`) gains `data.quality`
  and `why` (`get_decision_neighborhood`) gains `data.root.quality`: the seven-dimension profile
  exactly as `score_decision` returns it (framing, alternatives, information, reasoning,
  values_tradeoffs, bias_exposure, calibration; each `assessed` with a `level` of `none`,
  `partial` or `solid`, its `reasons` and the `node_ids` they rest on, or `not_assessed` and
  why), less the `provenance` line, which stays on `score_decision`. There is no composite, tier
  or score. Both answers also gain `still_holds.days_held`: the whole days nothing in the graph
  replaced the decision, counted to now, or to the superseding decision once it was superseded.
  It is not evidence the decision was carried out, and it is absent, never `0`, when a date is
  not recorded. The same on MCP over stdio and HTTP, the native HTTP routes and the CLI; a
  not-found or ambiguous answer carries no `quality`, and `get_decision` stays the plain record.
  `--summary` prints `still holds: yes — held N days; …`, a `quality` block with one line per
  dimension, and, for a superseded decision, `held N days until it was superseded`. The
  `rejected:` line now names which rejected options share the decision's rationale, read from the
  Alternatives floor, and says nothing for an option that carries a description of its own; it no
  longer claims "no distinct per-option reason is recorded" for every decision. (hivemind-bbnw.2)
- **A list of binding decisions whose why is still to answer.** `hivemind query
  get_open_why_decisions`, the MCP tool `get_open_why_decisions` and `GET
  /v1/attention/open-why` list, oldest first, the decisions in force that clear the decision floor
  (they bind something or are costly to undo) and rest on nothing but what their own capture
  brought: they follow from no earlier decision and replace none, no person accepted them and they
  record no person's delegation, and every observation, assumption and bet they rest on was
  recorded with them and is rested on by no other decision. Each item names its `deciders`, its
  `recorded_by`, `captured_at`, both floor answers and `rests_on`. It is an open state, never a
  refusal: every listed decision was written, and nothing is suggested or linked. `left_out`
  counts the in-force decisions left off because they are below the floor or were never asked.
  `truncated` and `data.next_cursor` page through it by position. See `docs/ATTENTION_LISTS.md`.
  (hivemind-lapcm)
- **Every read of a decision shows what it binds and what undoing it costs.** `get_decision`,
  search and recall results, `get_decision_brief`, the digest and the decision-log export carry
  `binds`, `undo_cost` (each `{level, statement?}`, absent when the decision was never asked) and
  `floor`: `cleared`, `below` (it binds nothing and is cheap to undo, by the capturer's own
  answers) or `not_asked` (a decision recorded before the questions existed, or captured by a
  path that does not ask). The text forms print `binds:` and `undo cost:` lines, `not asked` where
  there is no answer, and a below-floor decision reads `floor: below the decision floor`; a
  list row ends in `floor=below`. In the export the front matter gains `floor`, each decision
  file a "Binds and undo cost" section, and a below-floor decision's index status reads
  `<status>, below the decision floor`. Nothing is hidden: search and recall return a
  below-floor decision like any other. The background scorer now sends both answers to its model
  as `Recorded answer` lines to cite as the basis of any reach or irreversibility explanation,
  and says they are not quotable text; its prompt version is now `assessment-v2`. No score
  changes and no new weights. (hivemind-bbnw.11)
- **A capture whose rationale names an earlier decision by id, but does not rest on it, says so
  in its reply.** When the rationale holds a `decision-...` id (the full id, or its first eight
  hex digits when exactly one decision starts with them) that exists and is not among what the
  capture rests on, the reply of `capture_decision`, `POST /v1/decisions` and
  `emit decision.capture --json` lists it under `cited_not_linked` (text mode prints one line
  per id on stderr, so the id on stdout stays what scripts read). It is a hint: the capture is
  recorded either way, nothing is refused and nothing is linked for the caller;
  `hivemind ground --id <decision id> --rests-on-decision <cited id>` adds the link. The
  `hivemind-capture` skill and the `capture_decision` tool text now say to record a decision the
  rationale names as `--rests-on-decision`, and to record a person's ruling as that person's
  decision (`--decided-by`) that the agent's decision rests on, not as the agent's evidence.
  (hivemind-dy6b8)
- **Which decisions carry the most impact, and who decided them.** A new read lists the
  decisions most impactful first, each with who decided it (a person, an agent, an agent
  within a person's delegation, or no one recorded): `rank_decisions_by_importance
  {include_not_in_force?, limit?, cursor?}` on the stdio MCP server and the HTTP MCP endpoint,
  `GET /v1/decisions/importance`, `hivemind query rank_decisions_by_importance`, and
  `/hivemind-context:importance` in the plugin, all one core. A decision is ranked when the
  record gives a reason it matters: other decisions follow from it, counted directly and
  through chains (five hops; `chain_capped` says when more lie beyond), each row naming up to
  ten of them. More resting on it ranks higher, then the newest. A decision with no such basis
  follows, unranked, reading `not_assessed`: never a score of zero, because nothing following
  from a decision says nothing about it. The actor that only recorded a decision is never
  shown as its decider (`recorded_by` is its own field), and a decision nobody accepted reads
  `none_recorded`. A model's own judgement of stakes, irreversibility and actionability, stored
  until now and read by nothing, is shown on the row as `model_judged` and never moves the rank,
  so the list is the same on a ledger with no model. Superseded and rejected decisions are left
  out and counted in `left_out` unless `include_not_in_force`. Importance stays a separate axis:
  no quality score, no composite, and the tally of who decided the ranked decisions is by kind
  only, never per person or agent. A page holds at most 50 rows with `truncated` and
  `data.next_cursor`; `tests/importance_surface_parity.rs` requires every surface to answer
  alike. `docs/DECISION_SCORING.md` describes it and no longer lists Importance as deferred.
  (hivemind-bbnw.9)
- **`hivemind migrate --to-url` moves a SQLite ledger into a cell over HTTP, keeping each
  event's uuid, actor, source and time; nobody needs the database password.** It sends the
  events in batches to the new admin-key-gated `POST /v1/ledger/replay` (`tenant_id`,
  `dry_run`, up to 1000 `events`; answers `received`, `new_events`, `already_present`) with the
  key read from `--admin-key-file`. The cell dedups by event uuid, so a re-run writes nothing;
  renumbers each causation link by its cause's uuid; and refuses a batch whose link names an
  event that is nowhere, writing none of it. `--dry-run` prints what would move and writes
  nothing; `--to-tenant` is always required. `hivemind migrate` is available in every build
  (`--to <postgres-url>` still needs `shared-backend-postgres`). `docs/SELF_HOSTING.md` has
  the walkthrough. (hivemind-jawy)
- **A cell's restatements are listed and linked over HTTP with the caller's token, so nobody
  needs the database password to curate a cell.** `GET /v1/restatements/proposals[?project=]
  [&limit=][&cursor=]` returns what `hivemind restatements propose` prints (each pair, their
  overlap and the title words they share), a page at a time with `truncated` and
  `data.next_cursor`, and writes nothing. `POST /v1/decisions/{id}/restatements` with
  `{"restates_id": "<earlier decision>"}` records one `SAME_AS` link, later to earlier, as the
  bearer token's actor, with the rules of `restatements apply --link LATER=EARLIER`: a pair
  already linked, either way round, writes nothing (`linked: false`), nothing is deleted or
  rewritten, and an id that is not a recorded decision is refused. There is no route that links
  every proposal: a wrong link cannot be undone for reads, so each is named. A link no longer
  reads every event of the ledger: it reads the `decision.proposed` and `relation.added` events
  alone, so on a ledger of mostly transcripts it costs what its decisions cost, not a scan of
  the transcripts: on a 108k-event copy of the town ledger a new link took ~53 s through the
  CLI before and takes 1 to 2 s over HTTP now (a debug build). (hivemind-h4kr)
- **The `hivemind-capture` plugin tells every session to check the ledger before it acts
  and to record what is worth keeping, so an agent nobody briefed uses HiveMind and saves the
  right things.** With the plugin installed and its MCP server connected, agents given no
  instruction recalled and captured 0 times in 31 headless benchmark sessions: the plugin's
  only hooks fire on `AskUserQuestion`, which an autonomous agent never calls. A `SessionStart`
  hook (`plugins/hivemind-capture/hooks/hooks.json`, `scripts/session-start-hook.sh`) now adds
  this to the session's context:

  > HiveMind ledger: before you act, call the `hivemind` MCP tool `recall_decisions` (q =
  > three or four key words about the task) and follow the earlier decisions that still
  > hold. Call `capture_decision` (title, rationale = the why, options, grounding, binds,
  > undo_cost) for: a rule learned from something that failed or cost you (save it the
  > first time); a design or interface choice later work must follow; a reversal of an
  > earlier decision (use `supersede_decision`). Not for routine status, session
  > summaries, plans that hold only for this session, or a choice binding nothing and
  > cheap to undo.

  The first version said only to record each durable decision, and benchmark agents read that
  as "summarise what I did": one capture per session of the month's routine, the rule that
  avoided a costly client saved late or never, and design choices made and never saved. The
  text now names what is worth saving and what is not. It is a fixed text of about 595
  characters, held under a 150-token budget by a test (the shorter text before it measured
  143 input tokens on the wire: 19,310 against 19,167 with the hook on and off), reads no
  ledger, and fails open: with no `hivemind` binary it prints nothing and exits 0. Turn it off
  with `HIVEMIND_DIRECTIVE_DISABLE=1`, in the environment or under `env` in
  `.claude/settings.json`. Needs a `hivemind` that serves `recall_decisions`, `capture_decision`
  and `supersede_decision` (v0.7.0 and later); CI checks the hook against the latest release.
  The Codex package reads the same hook file, so it ships there too (Codex runs a hook only
  after you trust it; the plugin README says how). A test keeps the quotations in the plugin
  README, `docs/AGENT_DECISION_CAPTURE.md` and this file equal to the hook's text. See
  `plugins/hivemind-capture/README.md`.
- **`GET /v1/decisions/{id}/status-events` lists what changed one decision's status, newest
  first.** One entry per proposed, accepted, rejected or superseded event: `event`,
  `occurred_at` (the event's own ledger time, `null` when it carries none), `offset`, `actor`
  (`{id, kind}`), `delegated_by` on an acceptance that carries it, `superseded_by` on a
  supersession, and `status_after`, the one status rule applied up to that entry. `status` is
  the status now and equals the newest entry's `status_after`. A decision whose capture states
  the acceptance, rejection or replaced decision itself shows those entries at the capture's
  offset, dated when the decision was recorded. Read-only: no event type, nothing written, the
  timeline unchanged. HTTP only for now. See `docs/ATTENTION_LISTS.md`.
- **`GET /v1/classify-queue/sessions` summarises the sessions that still have pending batches.**
  One row per (session, actor): `session_id`, `actor_id`, `batch_count`, `oldest_submitted_at`
  and `newest_submitted_at`, most recently active first, plus `session_total`, `batch_total`,
  `truncated` and the day's classification `budget`; `?limit=` (default 200) bounds the rows.
  No turn text, so a runner that classifies one session at a time picks a session here and
  then fetches only that session's batches with `GET /v1/classify-queue?session_id=`.
  (hivemind-t15t)
- **`GET /v1/decisions/{id}/possibly-related`: decisions that may be related, labelled inferred,
  never a recorded relation.** Most decisions have no recorded relation to another decision, yet
  every capture carries its topic keys and, for a classifier capture, the capture session it was
  recorded out of. The endpoint offers, for one decision, the decisions recorded out of the same
  conversation first, then decisions that share a topic key only a few
  decisions carry. A topic key carried by more than `max(10, 2% of decisions)` decisions is an
  area tag ("refinery", "ci") and links nothing; the asked decision's keys that were set aside
  come back in `ignored_topic_keys` with how many decisions carry each, so the filter is never
  silent. Items are ranked by same conversation, then by the sum of `1 / (carried_by - 1)` over
  the shared keys, then by id, and each item lists those keys with `carried_by`, so the order
  can be recomputed from the answer. Decisions already joined to it by a recorded `SUPERSEDES`,
  `FOLLOWS_FROM` or `SAME_AS` are left out. The answer carries `layer: "inferred"` and a note
  saying so; it pages with `limit` and `cursor` and says when it is `truncated`. Nothing is
  written, the graph and `GET /v1/graph` gain no edge, and no ledger event changes. "Same
  conversation" is the same capture session, the `session_id` of the received batches a
  classification names, so a session the classifier recorded in several events is one group;
  where a classification names no batch the ledger received (a `hivemind emit` capture), its own
  event is the group. Each classified decision node gains a `session_ids` property, set while
  the ledger is projected: a server builds its in-memory graph from the ledger on first use, so
  existing ledger data is covered by the restart that deploys this, with nothing to migrate.
  HTTP only for now; there is no MCP tool or CLI verb yet. Library callers:
  `possibly_related::possibly_related` (Layer 3) over the new `queries::decision_capture_facts`
  and `queries::recorded_decision_links`. (hivemind-xarm, hivemind-266t)
- **Attention lists and a per-decision timeline: what is waiting, contested or changed, and
  when each thing happened.** Three lists say what needs a person without anyone going looking,
  each paged with `limit`/`cursor` and saying when it is `truncated`: **waiting**
  (`get_waiting_requests`, now also an MCP tool and `GET /v1/attention/waiting`) lists open asks
  with no answering decision, oldest first, with `asked_at` and who asked; **contested**
  (`hivemind query get_contested_decisions`, MCP `get_contested_decisions`,
  `GET /v1/attention/contested`) lists decisions accepted and rejected by different actors
  (naming both sides) and accepted decisions whose answers to one question choose differently
  (each naming the other), oldest first, each with `asked_at` and `decided_at`; **changed** (`hivemind query get_changed_decisions`,
  MCP `get_changed_decisions`, `GET /v1/attention/changed`) lists decisions superseded,
  retitled, moved or left without a premise in a window (`--since`, default the last 7 days,
  echoed in the reply), most recently changed first, each with `asked_at`, `decided_at` and its
  dated changes cited by ledger event. `hivemind why`, MCP `get_decision_neighborhood` and `GET /v1/decisions/why` gain
  `data.timeline` (and `GET /v1/decisions/{id}/timeline` reads it alone): the decision's dated
  story, oldest first in ledger order, asked, recorded, accepted or rejected, superseded,
  retitled, moved, and the moment a premise it rested on stopped standing (a refuted hypothesis,
  a superseded or rejected decision it follows from, written only for decisions that already
  rested on it then), with `asked_at`, `decided_at` and, when someone asked first,
  `asked_to_decided_seconds`; `why --summary` prints it as a `timeline:` block. Speed is the
  waiting list plus that one duration per decision: no averages, no per-person figures, no
  grading; a decision nobody asked about has no ask time. An ask records who asked, not whom, so
  the waiting list cannot say who is being waited on. No ledger event changes and nothing needs
  migrating; `docs/ATTENTION_LISTS.md` holds the rules. Library callers: `DecisionStandings`
  also reads who rejected a decision (`rejecters_of`); `NeighborhoodView` gains an optional
  `timeline` that only a caller holding the ledger fills. (hivemind-bbnw.7)
- **`hivemind ask` records that you are explicitly asking a question, before any decision
  answers it.** `hivemind ask "<question text>"` and MCP `request_decision` write a new
  `question.asked` event (envelope `ts`/`actor_id`/`source_ref` are the ask's time, asker and
  where — never back-dated), resolving the question exactly like a capture's `--question`: a
  match on the normalized text (lowercase, spaces collapsed, trailing punctuation dropped) is
  reused, otherwise `question.recorded` creates the node. Unlike answering, asking is never
  suppressed as a duplicate: the same question can be asked more than once, each its own
  outstanding request (new `Ask` node, `ASK_FOR`/`ASKED_BY` relations — see
  `docs/GRAPH_CONTRACT.md`). `emit decision.capture --answers <request id>` (MCP
  `capture_decision` with `answers`) resolves an existing request to its question and links the
  decision to it exactly as `--question <text>` would, without repeating the words; refused
  when the request does not exist, or together with `--question`. Distinct from `ground
  --answers`, which takes the question's own text, not a request id. `hivemind query
  get_waiting_requests` lists open requests (no answering decision yet) oldest first: request
  id, question, asked_at, who asked. `hivemind why` (`DecisionBrief`, the graph query layer,
  and the MCP/HTTP `get_decision_neighborhood` it feeds) on an answered decision now also
  carries `asked_at` (the earliest explicit ask for its question, when there was one) beside the
  existing `occurred_at` (the decision's own timestamp — unchanged, still the JSON field name);
  the CLI text summary labels the two lines `asked_at:` and `answered_at:` (was `when:`).
  `verify` is unaffected — asked/answered times are a `why` feature.
  Deliberately out of scope for this pass, left for a follow-up: `--project` on `ask` (the
  underlying `Question` node has never been project-scoped, matching hivemind-zdsh.16), an MCP
  tool for the waiting list (the CLI query is the only required surface), and the full
  waiting/contested/changed attention lists with per-decision timelines (hivemind-bbnw.7). This
  reuses none of `decision.requested`/`DecisionRequest`/`NodeKind::DecisionRequest`: that
  existing, unused scaffolding belongs to a different, still-undecided design
  (`docs/HUMAN_DECISION_BLOCKER_NOTIFICATIONS.md`) with its own required fields (`priority`,
  `authority_class`, `client_request_id`) that do not fit an explicit ask; overloading it would
  have conflated two concepts under one event type. (hivemind-bbnw.4)
- **The Claude Code plugin records an agent's question to you as an ask, and your answer as a
  decision that answers it.** `hivemind-capture` now ships hooks on the `AskUserQuestion` tool
  (`plugins/hivemind-capture/hooks/hooks.json`). `PreToolUse` writes one ask per question at the
  moment of the tool call (MCP `request_decision`; the event's own time is `asked_at`, never a
  guess). `PostToolUse` writes one decision per answered question: recorded by the agent,
  decided by you (`human:<git email>`, or `HIVEMIND_HUMAN_ACTOR`), the offered options, the one
  you picked, and any words of your own quoted verbatim. Several picks in a multi-select become one
  combined option (`Search + Export`) and what you picked into it is not listed as turned down. The decision names the same question as the ask,
  so `hivemind why` shows `asked_at` beside `answered_at` and the request leaves
  `get_waiting_requests`; the hooks share no state. The note you write on your pick is the
  rationale, word for word (a note too short to read on its own is quoted instead); only with
  neither a note nor words of your own does it say no reasons were given. It rests on a bet with
  nothing declared, because nothing was stated with the answer. The answer is filed under the
  slug of the question's header; when a registered project has not declared that topic (a
  project accepts only declared ones), the hook writes it once more under the fixed key
  `claude-code-question` and declares only that key, so a project's vocabulary grows by at most
  one key from these hooks, never one per question, and your answer is recorded in every
  checkout (a personal or unregistered one has no vocabulary and declares nothing). Only that
  refusal is retried; both attempts are in `hook.log`. A question you decline
  or never answer stays waiting; nothing is recorded when nobody asked (an agent deciding alone,
  you deciding unprompted, an import). Writes go to the local ledger through `hivemind mcp`, or
  to `POST /mcp` of the server named by `HIVEMIND_API_URL` (bearer token `HIVEMIND_API_KEY`), so
  a hosted deployment works the same way. The hooks never get in the agent's way: a failed write
  is logged (`hook.log`) and the question still appears; `HIVEMIND_ASK_HOOK_DISABLE=1` turns
  them off. Needs `python3` and a CLI or server with the ask verb. Tests replay the hook
  payloads of a recorded Claude Code session
  (`tests/fixtures/claude_code/ask_user_question/`). The `hivemind-capture` skill tells agents
  not to capture the same answer again. (hivemind-bbnw.5)
- **`GET /v1/graph` says when each decision was made.** Every `decisions[]` entry and every
  `nodes[]` entry of `kind: "Decision"` now carries `decided_at`, the `decision.proposed`
  capture event's timestamp in ISO-8601 UTC (`null` only for an event predating the ledger's
  timestamp backfill), so a whole-graph view can show a decision's date without a second
  lookup. Additive: no existing key changes. Documented in `docs/GRAPH_CONTRACT.md`.
  (hivemind-zbd0)
- **A decision's link no longer moves.** Every `decisions[]` entry, every `nodes[]` entry of
  `kind: "Decision"`, and the decision brief (`GET /v1/decisions/verify`) now carry `slug`: the
  decision's title, kebab-cased, with a growing id-tail suffix for a later decision whose title
  slugs to the same thing. The server assigns it once, when `decision.proposed` is first
  projected (ledger order, so replay is deterministic); nothing projected afterwards touches
  it, so a retitle or any other annotation leaves a decision's link unchanged. Additive: no
  existing key changes, no new event. Documented in `docs/GRAPH_CONTRACT.md`. (hivemind-nidp)
- **Importers keep the source's own time instead of import time, and record an explicit ask as
  `question.asked`.** Every `Commands`-driven write can carry an explicit event `ts`
  (`CommandContext::event_ts`) in place of wall-clock "now". The Slack thread importer stamps
  the events it writes with the deciding message's own time (the `Chosen:`/`Chose:` message,
  else the `Decision:` message), and the local document importer honors an optional
  `ts:`/`decided:`/`decided-at:` block marker, falling back to the file's last-modified time when
  the block names none. That fallback is "last modified", not "decided": a git checkout or a copy
  resets it. The same block parser reads a `Decision:` block pasted from a git commit trailer or
  a tracker/beads ticket body, so a `ts:` in that block carries the commit's or ticket's own
  time; none of these is a separate importer. An ask has one home, the `question.asked` event
  from `hivemind ask`: the Slack thread importer writes one, at the root message's own time and
  by the root's author, only when the root is a different message from the one carrying the
  decision **and** is recognisably a question by a deterministic test (it ends in `?`, or
  carries a `Question:`, `Ask:` or `Decision needed:` marker); the decision then answers it, as
  `capture --answers` does. A decision posted top-level, a root that states the decision and a
  root that does not visibly ask get their own time and no ask, so the timeline reads "asked at:
  not recorded" rather than a guess. Live Slack app captures see only their own message, so they
  write no ask; documents never do. `ingest.batch_received` turns can carry an optional `ts`
  (Claude Code and Codex JSONL logs already have one); classification does not consume it yet.
  `docs/TEXT_IMPORT_AND_DIFF_SEMANTICS.md`, `docs/SLACK_APP.md` and
  `docs/CAPTURE_CLASSIFIER.md` describe the rule per source. (hivemind-bbnw.6)
- **A decision the transcript classifier derives is recorded at its own turn's time, and an
  explicit decision-request turn writes `question.asked`.** A classified capture can name the
  turn it came from (`source_turn_id`, the turn id the classifier now sees in each turn's
  header) and, on a `decision` or `decision-request`, the question in the words it was asked in
  (`question`). The write path checks the turn is one of the batches the classification covers
  (refusing the classification otherwise), stores that turn's own time on the capture as
  `source_ts` (never taken from the caller), and the projector records the decision at that
  time, so `why` shows when it was decided instead of when the transcript was classified. A
  capture that names no turn, or whose turn carries no time, keeps the classification's time.
  A `decision-request` that states a question and names a turn with a time writes one
  `question.asked` at that turn's time, by the actor the classifier named on the request (with
  none named, a request in an assistant's turn is credited to whoever submitted the batch, that
  agent having spoken it, and a request in a user's turn writes no ask, since the batch records
  no human and a human's question is never credited to an agent); a later decision that states
  the same question links to it with
  `ANSWERS`, matched on the normalized text exactly as `capture --question` is. No ask is
  written for an agent deciding alone, for a request with no question, or for a request turn
  with no time (the timeline reads "asked at: not recorded"), and a re-ingested transcript
  writes the same ask once. `source_turn_id`, `source_ts` and `question` are new optional
  fields of `ingest.batch_classified` captures: a plugin that sends them to an older server is
  refused, so upgrade the server first. The shipped hooks already send each turn's `ts`.
  `docs/CAPTURE_CLASSIFIER.md` and `docs/AGENT_DECISION_CAPTURE.md` describe the rule.
  (hivemind-bbnw.8)
- **A model's assessment of a decision can be recorded and is shown beside the quality
  floors.** `decision.scored` gains a second payload version (`schema_version: 2`), keyed by
  the decision's id (a proposed decision or a classified capture): all seven dimensions, each
  either assessed (a level of `none`, `partial` or `solid`, an explanation and, at `partial`
  or `solid`, the verbatim passage of the decision's own text it rests on; a `none` answer may
  leave the quote out, since an absence cannot be quoted) or not assessed (and why), with the
  model and prompt version. The write path refuses, and records nothing, when a quote does not
  occur in the decision's recorded text or a `partial` or `solid` answer gives none.
  `score_decision` adds `model_assessment` beside the floors
  (only when one exists; no floor changes because of it), and the CLI summary and the Markdown
  export show it. Existing `decision.scored` events (the float scores) are unchanged in the
  ledger and keep replaying; the profile never shows them. Nothing produces a version-2
  assessment yet. (hivemind-qo11.8)
- **Two producers now write a model's assessment; the version-1 write path is retired.** The
  background scorer (`src/scorer.rs`, `ANTHROPIC_API_KEY`) and the keyless `emit decision.scored`
  path write `decision.scored` schema version 2 exclusively, through the same
  `Commands::record_decision_assessed` validator. The scorer now also assesses decisions
  captured directly (`decision.proposed`/`decision.capture`), not only classifier-extracted
  captures, and no longer double-assesses a decision that already has a version-2 event. Its
  prompt asks the model to assess Framing and Values / Tradeoffs — the two dimensions with no
  floor beyond "a question was recorded" — and to enrich the other five only where it has real
  grounds, leaving the rest `not_assessed`; there is no 0.5 placeholder. `emit decision.scored`
  takes the scores file `{"dimensions": {...}}` (the same shape) and names its target by
  `--decision-id` (a decision captured directly) or `--batch-id`/`--capture-index` (a classifier
  capture); `--scorer-model`/`--weight-version` are gone, replaced by `--model` and the required
  `--prompt-version`. Nothing changes when `ANTHROPIC_API_KEY` is absent: the worker does not
  start, and the profile stays complete on floors alone. Version-1 `decision.scored` events
  already in a ledger keep validating, replaying and projecting unchanged; nothing new writes
  that shape. (hivemind-qo11.9)
- **`get_suggestions` returns what needs a look and has not been dealt with.** A new MCP tool
  on the stdio server and the HTTP endpoint, and `hivemind query get_suggestions`: the
  attention findings of `scan_decision_quality`, less the ones that have been acknowledged
  (matched by `finding_id`). `exclude_acknowledged` (`--exclude-acknowledged` on the CLI) is
  true by default; false returns every finding. The page is filled from the findings that
  remain, so it holds `limit` findings whenever that many remain and `truncated` is exact. A
  finding whose basis changed has a new `finding_id`, so an old acknowledgement never hides
  it. Same arguments, response shape and refusals as `scan_decision_quality`; no score, no
  tier. (hivemind-m306.4.1)
- **`acknowledge_suggestion` records that a finding was looked at, and `get_suggestions` then
  leaves it out.** A new MCP tool on the stdio server and the HTTP endpoint: pass the
  finding's `finding_id` and `decision_id` (and optionally `action`: `seen`, `acted` or
  `dismissed`, default `seen`; `channel`, default `mcp`). It appends two attributed events: a
  new event type `suggestion.surfaced` (this finding, shown to the acting actor over this
  channel) and a `notification.acknowledged` naming it. `get_suggestions` with
  `exclude_acknowledged` (the default) leaves the finding out until its basis changes and its
  `finding_id` changes with it; `exclude_acknowledged: false` and `scan_decision_quality` still
  show it. Acknowledgement is by finding, not by actor, and an acknowledgement with a
  `snooze_until` hides the finding only until then. A surfaced finding projects to a
  `Notification` node with no blocker, so the blocker queries do not see it.
  **Schema:** `notification.acknowledged` gains an optional `action` (`seen`, `acted`,
  `dismissed`) and `suggestion.surfaced` is a new `schemas/v0` event. Both are additive: no
  migration, every acknowledgement already in a ledger stays valid and replays unchanged, and
  Kuzu graphs are rebuilt from the ledger. **Upgrade the server before any client writes the
  new event:** a binary from before this change cannot read a ledger that holds a
  `suggestion.surfaced`. (hivemind-m306.4.2)
- **A decision answers a question, and decisions that answer the same one are findable.**
  `--question "<one line>"` on `emit decision.capture` (MCP `question`) now names the
  question the decision answers, and it no longer needs a `--quote`: only a quote needs its
  question. Two captures whose question is equal after lowercasing, collapsing spaces and
  dropping trailing punctuation share one question node (a new `question.recorded` event and an
  `ANSWERS` relation; the capture reply gains `question_id`). The match is exact, with no ranking
  and no model. `query verify` / `why` print `answers: <question>` and `also answered by: <title>
  [status]`; two accepted, non-superseded decisions that chose different options are flagged on
  both as `conflicting_answer` (attention, not staleness: neither is marked stale, nothing is
  resolved); `query situational` shows a newer accepted answer to the question of a decision it
  matched. `hivemind ground "<decision>" --answers "<question>"` (MCP `ground_decision` with
  `answers`) links a decision captured without one, attributed to whoever runs it. Existing
  ledgers replay unchanged: a decision recorded with question text and no node keeps the text and
  gets no node. (hivemind-zdsh.16)
- **`GET /v1/graph` says who decided what and whether it held up.** Every `decisions[]` entry
  carries its derived `status` (`proposed`, `accepted`, `rejected`, `contested`, `superseded`)
  and `deciders` (the actors who accepted it, each with `kind` `human` or `agent`; empty until
  someone accepts), and every Option node carries a `title` (its label, else its id).
  Additive: no existing key changes. Documented in `docs/GRAPH_CONTRACT.md`. (hivemind-pkw0)
- **`GET /v1/whoami` says who a token is.** It takes the same bearer credential as every other
  `/v1` route (user token, agent token, WorkOS sign-in, the shared `HIVEMIND_API_KEY`, a
  Postgres tenant token) and answers `{"actor_id", "tenant_id"}` for it, or `401` with the usual
  error body for a missing, empty, unknown or revoked token. The shared key and a tenant token
  answer `service:api`, so a client can warn that writes made with them carry no person's name.
  Opens no ledger and writes no event; there is no MCP tool for it. Documented in
  `docs/SELF_HOSTING.md`. (hivemind-jro0)
- **A decision captured before the title cap can be given a short name.** `hivemind retitle`
  (MCP `retitle_decision`), found by description or `--decision`, appends a new
  `decision.retitled` event instead of rewriting anything: the projection shows the new title,
  the node keeps `former_title`, and the event carries `from`/`to` and an optional `reason`.
  The new title obeys the same cap capture enforces; an unknown decision, a stale `from`, or a
  title that is already current writes nothing, and no event is appended. Reversible: retitling
  back is just another recorded retitle. `recent_decisions` / `decisions_added_since` /
  `decisions_changed_since` gain their own `title_changed` history line (mirrors
  `decision.moved`'s `project_moved`), and `why` is unaffected. (hivemind-ydmp)

### Changed

- **The capture plugins, the skill, the Codex bundle and the classifier teach what makes a
  choice a decision, and the classifier stops producing `decision-request`.** The capture flow
  is "answer before you write": what does it rest on (looks back, unchanged and first), then
  what does it bind and what would undoing it cost (looks forward). A choice is a decision when
  it binds something beyond the edit in front of you or is costly to undo; "apple" for "Apple"
  in one comment is not one, merging tonight despite a failing check is, and a request goes to
  your tracker. The `hivemind-capture` skill, its `capture`, `capture-decision`,
  `batch-capture` and `classify-queue` commands, both `active-capture` skill copies (kept
  byte-identical), the Codex bundle (`openai.yaml`, manifest prompts), the plugin README, the
  `hivemind-context` `supersede` text, `docs/AGENT_DECISION_CAPTURE.md` and
  `docs/CAPTURE_CLASSIFIER.md` all say so, and the session directive now names `binds` and
  `undo_cost` in its `capture_decision` call (still under its 150-token budget). The
  classifier prompt's "if the material is borderline, omit it" becomes the same test, and its
  three copies (`src/classifier.rs`, `docs/CAPTURE_CLASSIFIER.md`, and the copy in the plugin
  skill and `batch-capture` that `scripts/e2e_smoke.sh` cuts out by sentinel) carry the same
  decision paragraph; a test holds them equal. `decision-request` leaves the classifier's output
  schema, the `classify-queue` and `batch-capture` kind lists and `capture.sh`'s kind list
  (`--kind decision-request` now reads as an unsupported kind). The recorded type stays:
  `ingest.batch_classified` and the graph still accept it, so a classification recorded earlier
  replays unchanged. The prompt change moves fidelity on the classifier corpus, and a
  re-baseline needs a metered model key, so it is not run here. The four answer flags
  (`--binds`, `--binds-nothing`, `--undo-cost`, `--cheap-to-undo`) are newer than the latest
  release, whose CLI refuses a capture that carries any of them, so `capture.sh` leaves them
  out on a CLI that does not list them and says so on stderr, the skill and README say the same
  for the direct CLI form, and a v0.7.0 MCP server ignores the extra `binds` and `undo_cost`
  arguments instead of refusing. The "Plugins against the latest release" check follows the
  same fallback. (hivemind-bbnw.12)
- **`scan_misfiled_decisions` says where each decision is filed and where it belongs.** Each
  candidate carries its `project`; `--project` limits the report to decisions filed exactly
  under one project (an unregistered handle is refused, never an empty report) and
  `--move-to <project>` names the destination, skips decisions already there and gives each
  text row the `hivemind move --decision <id> --to <project>` that moves it. Still a report:
  it moves nothing. (hivemind-zywz)
- **Docs.** The README and VISION's "Who it's for" now say HiveMind is for one person and their
  agents in any field, not only engineering, and say plainly that a developer's setup (a local
  ledger or a self-hosted cell, the MCP server, the Claude Code and Codex plugins) is what runs
  it today. No capability changed. (hivemind-48zc)
- **Docs.** The capture command no longer says an omitted `--kind` goes to a `hivemind-classifier`
  subagent: none ships. It now says what `capture.sh` runs (`$HIVEMIND_CAPTURE_CLASSIFIER_JSON`,
  else `$HIVEMIND_CAPTURE_CLASSIFIER_CMD`, else `claude --print` on `$HIVEMIND_CLASSIFIER_MODEL`,
  default `haiku`), that it answers `decision`, `evidence`, `hypothesis` or `none`, and what
  happens when no classifier is available. `docs/CAPTURE_CLASSIFIER.md` now says it specifies the
  batch classifier (server-side worker, `classify-queue`, `batch-capture`), not the one behind
  `/capture`. No capability changed. (hivemind-ptqk7)

### Fixed

- **`why` no longer answers a close match that holds only a few of the question's words when a
  decision it left out says more of them, a word the question says in two forms is counted
  once, and a peer does not come before a decision that says as much of the question.** The
  first two come from the older rule that admits a decision below the half-of-the-words bar
  (hivemind-wdxx0 left them as known costs). That rule counted "job" and "jobs" as two words a
  decision shares with the question when the decision says "job" once, so a session note whose
  title says "job" and "run" weighed as much as the decision about the thing asked, in the
  pair rule, in how much of the question a title carries (`About`, which orders `recall` and
  `why`) and in the peer rule that lists decisions whose titles say as many words as the
  answer's. Each stem is now counted once in all of them. And an admitted decision is no longer
  named alone when a decision left out below the bar holds two of the question's words, one in
  its title or topic keys, and is strictly more about the question: `why` lists them instead,
  with the labels of any close match. And a decision listed with the answer because its title
  says two of the question's words side by side (a peer) no longer comes before the decision the
  answer was found by when that decision is about the question at least nine tenths as much: two
  drafts that said "get" and "new" in their titles weighed 8% more than the decision titled for
  the new name, because "get" is held by fewer decisions than "name", and came before it though
  it holds four of the question's words and they hold two. A peer that weighs clearly more keeps
  its place. A share of the question held does not tell the right
  answers from the wrong ones (right ones go down to a weight share of 0.27), so none is used.
  What the question says is the only thing counted. Through the debug build of this change on a
  copy of the crew ledger (534 events, 69 decisions), 690 questions: two questions differ from
  before. "should the sorting step use a model on the person's own machine unless they switch
  it?" has a decision that is not in the ledger and stops answering "The company weekly's
  headline number is ..." as a close match lacking five words, and lists two decisions; "when
  a decision mixes lettered and unlabelled choices, which one goes first?" goes from the
  second of a list to the first. The checker's 439 stay at 355 in the recall top three (first
  294 to 295), no question loses its place in it, the `why` answers that are a decision other
  than the recorded one go from 20 to 19, the controls are unchanged. On a copy of the cell's
  ledger (about 150,000 events, 1,845 decisions), the same 694 questions: the checker's 439 stay
  at 305 in the recall top three (first 200 to 202), no question loses its place in it, `why`'s confident wrong answers go from
  42 to 38 across the batteries (36 to 33 in the checker's) and its right answers from 243 to
  244, and the controls are unchanged. Per question: "when the forecast runs out of finished
  examples, how far out does the survival curve put the leftover chance?" (twice) and "why did
  the drawings in the animation stop using a fixed-size canvas that got shrunk on phones?"
  stop answering a decision they are not about ("beadline forecaster: one shared schedule per
  run ..." holding three of eleven words; "Hidden Flow tab stops the R3F frame loop instead of
  unmounting the Canvas"), and list them instead, where the decision asked about is not in the
  ledger or is still not found; "how
  does beadline handle the time that is left over for the jobs we have no finish times for?"
  goes from the first of a list to the answer; "how does beadline draw the time a long job
  still needs for jobs that run longer than anything we have seen finish" puts the forecast
  decision first and answers it as a close match, where a note from a session came first;
  "if just one choice lacks a letter, how are the choices sorted on the page?" stops answering
  a wrong decision and lists the right one second (recall 3 to 2); "how does the page match each
  choice with its reason when the choices have no letters?" goes from a list to the first of one
  (recall 2 to 1); "I want to add a topic filter to the decision list page; was anything decided
  about filters?" has its decision at 5 in `recall`, from 7; "does the command-line binary get a
  new name along with the new company?" puts the decision titled for the new name first, from
  third behind two drafts; "how do we know the demo's sample data was not edited by hand?" puts
  its decision first, from third. The wdxx0 wins hold: the link
  question answers no wrong decision and lists the decision about the missing link third, the
  lint question puts the decision about it first. Not fixed: the decision about the leftover
  mass, which the survival curve question asks for, is still not listed.
  `docs/AGENT_FLUENT_QUERYING.md` states both rules. (hivemind-otruj)

- **The web UI's landing page no longer promises an invite-only pilot.** Its "Connect your agents
  today." section said "The invite-only pilot includes setup for Codex and Claude Code", but the
  hqbd9 merge removed the "Request access" form and the "Sign in" link because the server has no
  accounts or access requests, so there was no pilot a visitor could join. The sentence is gone
  and the next one, which tells a visitor to connect an agent through MCP, stands on its own; no
  other copy changed. `web/HIVEMIND_API_MAP.md` records the removal. The Rust server is
  unchanged. (hivemind-21yg5)

- **The web UI's decision page names the choice of a draft that chose words it never weighed as an
  option.** A classifier draft can choose a label that is not one of the options it listed (or
  name a choice and no options). The page marked the chosen option among "What it weighed", so
  for such a draft it marked none (`capture:105186:3` listed seven options, none chosen) or, when
  the draft listed no options, showed no options section at all (`capture:106622:0` never said
  "rebase in scratch worktree"), while the CLI's `why` and the `/graph` page named it. On the
  cell's ledger that was 24 drafts. The page now adds a "What it chose" section with the words
  the capture recorded; the options it weighed stay as they were, none marked, as the choice is
  not one of them. A draft whose choice was one of its options marks it there as before, and a
  decision with options and no choice shows them as open. The Rust server is unchanged.
  (hivemind-89vgq)

- **`recall` and `why` list the decisions whose titles say two of a long question's words side by
  side, when one of the words is held by a third of the ledger, and `why` no longer answers the
  decision that says more of the words in a rationale.** "Does the graph draw anyone in it at
  all, or do we tell you who made the call?" left out the decision titled "hu-8oa: the graph
  draws no people ..." and `why` answered "Graph list view kept and made readable ..." as a close
  match; "if somebody follows a link to the graph but it does not point at any one thing we
  chose, what do they get told when it opens?" left out "A Graph link naming no decision says so
  ...". Both titles hold two words of the question, but "graph" is held by 29 of the crew
  ledger's 69 decisions, so the words weigh at most 2.4 where the pair test asks 5.3
  (`1.25 * ln(searched + 1)`), and the decision whose title says "graph" and "made" lacked fewer
  words, so it was named. Two things change. A decision below the bar is now also listed, with
  the answer, when it is a peer of the answer's best-titled decision: its title or topic keys
  hold as many of the question's words (two at least), two of them words the question says side
  by side, and it is about the question (`About`) at least 0.9 times as much; at most three, most
  about the question first, and only when the answer has a decision titled for one of the words
  (an answer found by a rare word in a rationale has none). A peer is listed and never named
  alone: `why` does not answer a peer. And `why` no longer answers a close match when the next
  close match says as much of the question in its title and topic keys and is about it nearly as
  much; it lists both, the way it lists two close matches that lack the same number of words.
  The two-words rule for a title (hivemind-pobj2) now also needs the two words side by side in
  the question, or five of the question's words held in all: on the cell's ledger "is there a
  lint that checks nobody makes up their own colours and sizes?" had "lint" and "sizes" in the
  title "Box the large Payload enum variant to fix clippy size lint" and put it ahead of the
  lint decision that was asked about. Through the debug build of this change on a copy of the
  crew ledger (534 events, 69 decisions), 690 questions: both questions above and their two
  longer forms put the decisions asked about in the top three (`why` lists them, and does not
  answer the graph decision), the controls ("do we need a licence to play music in the
  office?" among them) stay empty, the checker's 439 go from 352 to 355 in the recall top
  three and no question loses its place in it, and the `why` answers that are a decision other
  than the recorded one fall from 23 to 18. Known cost: seven `why` answers that were right
  become the first of a list (a peer is listed with the answer: "how does the browser turn the
  server's graph into the drawing, and what about two arrows between the same pair?" among them)
  and five recorded decisions move from first to second in `recall`, behind a peer that is as
  much about the question. On a copy of the cell's ledger (about 150,000 events, 1,845
  decisions), the same 694 questions: the checker's 439 stay at 305 in the recall top three
  (first 199 to 200), seven recorded decisions leave the top three and nine enter it, and
  `why`'s right answers go from 156 to 141 (first of a list from 50 to 67, a list from 123 to
  141) while its confident wrong answers fall from 55 to 36; the controls are unchanged. The
  long question about a link that points at nothing no longer answers "The UI graph sizes a
  ranked decision ..." and lists the decision about the missing link third, and the question
  about a lint that checks colours and sizes puts the decision about it first again. Known
  cost on the cell: "why did the drawings in the animation stop using a fixed-size canvas that
  got shrunk on phones?", whose decision neither build finds, listed two other decisions and now
  answers one of them as a close match lacking six words, because the stricter two-words rule
  drops the other.
  Not fixed here, as they come from an older rule that admits a decision holding three of the
  words and counts "job" and "jobs" twice: on the cell's ledger the decision about what the
  graph draws is still not in the top three of the long question (a decision about the same
  graph comes first), "how does beadline draw the time a long job still needs for jobs that
  run longer than anything we have seen finish" still puts a note from a session first, and
  "when the forecast runs out of finished examples, how far out does the survival curve put the
  leftover chance?" still answers "beadline forecaster: one shared schedule per run ..." as a
  close match lacking eight words, where the decision about the leftover mass is not listed.
  (The entry above, hivemind-otruj, changes the last two.)
  `docs/AGENT_FLUENT_QUERYING.md` and the `recall_decisions` description state the rules.
  (hivemind-wdxx0)

- **On a ledger of the cell's size, `recall`, `why` and `verify` name the decision filed under the
  question's rare word, and a rare word no longer outweighs two words in a title.** Both rules
  below were measured on 69 decisions only. On 1,845 (a copy of the cell's ledger, 157,563
  events) "which shelf do we want people to put us on?" still missed "HiveMind's public shelf is
  agent memory": several decisions say "shelf" in their title and tie on how much of the question
  they carry, and the subject rule broke the tie by the lower id, which put a proposal about the
  shelf (`capture:...`) ahead of the decision filed under `public-shelf`. The
  tie now goes as the order of close candidates already has it: of two equally about the
  question, the one that names a word it holds in both its title and its topic keys, then the
  lower id. And "does the command-line binary get a new name along with the new company?" lost
  its answer: "along", which seven decisions hold, weighs more than the two words ("new",
  "name") in the title of the decision about the new name, so a merge note that says "along"
  was added ahead of it and `why` listed four instead of answering. A subject is now added only
  when its title or topic keys say at least as many of the question's words as any decision in
  the answer does; two words in a title are no chance meeting. Through the debug build of this
  change on a copy of the cell's ledger, 643 questions plus four for this fix: both questions
  and the shelf question with "when they first hear what we make" put the shelf decision first
  and `why` lists it first; the rename question puts the rename decision first and `why`
  answers it; "what shelf do we put the product on publicly?" still answers it. The checker's
  439 go from 304 to 305 in the recall top three (first 196 to 199), `why` right answers
  from 154 to 156 and first-listed from 49 to 50; the other published sets and the licence and
  music controls are unchanged, and no question loses a right answer. Two `why` answers
  change to a decision that is not the one the question recorded: the rename question answers
  the new-name decision where the set recorded the decision that keeps the tool's name
  (second in `recall`), and "when the forecast runs out of finished examples, how far out does
  the survival curve put the leftover chance?", which neither build answers right, listed the
  forecaster decision with a benchmark note about a "threshold curve" and now answers the
  forecaster decision alone as a close match that names the eight words it lacks. Known
  cost: "will renaming the database break backups?" still puts "Registered Dolt backup for new
  gas-city-setup rig manually" first, as it did before. It holds "backup", a form of "backups",
  in its title, so it is shaped like the shelf decision, and no count of the ledger separates the
  two. On the crew ledger (534 events, 69 decisions) the two questions this rule moves back to
  first are the ones the subject rule below moved to second: "how do we get the graph to draw a
  decision's options A, B, C in order ..." (`why` answers it again) and "how are features that
  are built but not yet released drawn on the marketing site's graphs?" (`why` lists it first
  again); the other two stay second, and the checker's recall top three stays at 352 of 439.
  `docs/AGENT_FLUENT_QUERYING.md` states both rules. (hivemind-1p3tk)

- **`recall`, `why` and `verify` find the decision whose title holds two words of a long question,
  when everyday words outnumber them.** "For jobs that run longer than anything we have seen
  finish, how does beadline draw the time they still need?" listed three decisions that say
  "run", "finish", "time" or "need" in a rationale and left out the one titled "beadline ADR-2:
  Kaplan-Meier leftover mass draws beyond longest completed duration ...", which "how does
  beadline draw the time a long job still needs" finds first; `why` answered the first of the
  three as a close match. Every decision was below the bar of five of the ten words. The titled
  one carried 0.198 of the question's weight (two words nobody holds weigh in the total) where
  the pair test asks 0.2, and the three-decision cap ranks by the weight of every word held, so
  the rationales outweighed it. A decision below the bar is now also answered when its title or
  topic keys hold two of the question's words, the words it holds weigh at least 1.25 times
  `ln(searched + 1)` as the pair test asks, and it is more about the question than every
  decision in the answer; it is tried when no subject (the shelf rule below) is found, at most
  one is added, and an empty answer adds nothing. It comes where its title puts it, first
  here, and carries `missing_terms` like any close match. Through the debug build of this
  change on a copy of the crew ledger (534 events, 69 decisions), 643 questions plus 35 written
  blind for this fix: the checker's 439 go from 350 to 352 in the recall top three (first 293
  to 295) and `why` from 210 to 211 right answers, one question listing the right decision
  first where it missed; the bead's question moves from absent to first, and `why` lists it
  first instead of answering the wrong decision; the shelf question and "what order does the
  main list of decisions show, and which filters are left?" stay first, every unrelated
  control answers as before ("do we need a licence to play music in the office?" among them),
  and the other published sets are unchanged. No question loses a right answer or moves one
  from its place, and no wrong answer is added. `docs/AGENT_FLUENT_QUERYING.md` states the
  rule. (hivemind-pobj2)

- **`recall`, `why` and `verify` find the decision titled for the one rare word a question names,
  when the question adds everyday words that other decisions hold.** "Which shelf do we want
  people to put us on?" listed three decisions that say "people" and "put" and left out the one
  titled "HiveMind's public shelf is agent memory", which `recall "shelf"` finds first; `why`
  with the same words listed those three as ambiguous and the target was absent. A decision
  below the bar is now also answered when it is the question's subject: its title or topic keys
  hold the rarest word that any decision holds, no decision in the answer holds that word in its
  title or topic keys, it is more about the question than every decision in the answer, and the
  words some decision holds carry at least two thirds of the weight of the question (a question
  mostly about what nobody holds, such as "did we decide to move the UI from React to Svelte?",
  has no subject). At most one decision is added this way. It comes where its title puts it,
  usually first, and carries `missing_terms` like any close match. Through the debug build of
  this change on a copy of the crew ledger (534 events, 69 decisions), 643 questions: the
  checker's 439 go from 348 to 350 in the recall top three (the shelf question and "what order
  does the main list of decisions show, and which filters are left?" are first) and `why`
  lists the shelf decision first; no question loses a right answer from its top three,
  every unrelated control answers as before ("do we need a licence to play music in the
  office?" among them), and the other published sets are unchanged. Four right answers move
  from first to second behind the new decision, where it is the only decision to hold a word
  in its title (`why` lists them instead of answering with the first): "how do we get the graph
  to draw a decision's options A, B, C in order ...", "how are features that are built but not
  yet released drawn on the marketing site's graphs?", "when the auto-classifier sees a ruling it
  has already logged, does it write a duplicate?" and "since joining forces with a designer, what
  brand does the homepage carry?". `docs/AGENT_FLUENT_QUERYING.md` states the rule.
  (hivemind-oq2ae)
- **The web UI opens a classifier draft's decision page.** A decision the classifier read out of
  a session has an id with colons in it (`capture:105186:3`), and a result of `/search` linked to
  it as `/decisions/capture%3A105186%3A3`. The page took that segment as it came, still
  percent-encoded, and the server client escaped it a second time, so the server was asked for
  `capture%253A105186%253A3`, answered "decision not found" and the page showed "This page could
  not be found". On a ledger like the cell's, where most decisions are drafts, that was most
  results. The page now decodes the id once, so the server is asked for
  `/v1/decisions/capture%3A105186%3A3`, and `/decisions/capture:105186:3` written out with its
  colons opens the same page. A `decision-<uuid>` page is unchanged. The Rust server is
  unchanged. (hivemind-4ruxj)
- **A classifier draft's chosen option is always in the graph, so a page edge never ends at a node
  `nodes[]` lacks.** A draft whose chosen label was not one of the options it weighed (or that
  named a choice and no options) was projected with a `CHOSE` edge to an Option node nobody
  created. On the cell that left 34 `CHOSE` edges of `GET /v1/graph` ending at nothing, so the
  page of a project (`?project=`), which promises both ends of every edge in `nodes[]`, broke the
  promise, and the web UI dropped those edges: a draft like `capture:106622:0` showed no chosen
  option. The projector now creates the chosen Option node, labelled with the words the capture
  recorded; it is not `HAS_OPTION`, as nobody weighed it. The server's graph is rebuilt from the
  ledger on start, so a restart carries existing drafts across; nothing is rewritten.
  (hivemind-yeb7a)
- **`GET /v1/graph` returns each edge once.** A capture asserts its option, chosen and assumption
  edges twice in the ledger (inside `decision.proposed` and again as `relation.added`), and the
  in-memory graph the server answers from kept both, so a ledger of 1,845 decisions answered 796
  repeats among 9,881 edges (the Postgres and Kuzu graphs always kept one per relation, source and
  target, the first write). The in-memory graph now keeps the first write of each pair too, so
  `query`, MCP and the REST reads agree with the shared backends, and a reader no longer has to
  collapse repeats. The capture still writes the pair twice; the ledger is not rewritten.
  (hivemind-d87pp)
- **A question with few content words finds its decision through `search`, as the help says.**
  `query search`, MCP `search_decisions` and `GET /v1/decisions/search` (the web UI's search page)
  ask again the way `recall` asks when no decision has every word of a phrase of three or more
  words, but counted the words after the question words ("where does the", "how should ... be")
  were dropped. "where does the website live" and "how should interviews be framed" are five
  words each and have two content words, so they stayed on the all-words path and answered 0
  while `recall` ranked the decision first with nothing missing. The words are now counted as
  they were written; a query of fewer words stays exact, and a phrase of nothing but question
  words ("what did we decide") still lists nothing instead of every decision. The help, the MCP
  tool text and `docs/SEARCH_DESIGN.md` say so. (hivemind-vpf4q)
- **What changed since a date now lists the decisions passive capture recorded after it.** A
  classified capture's decision has no `decision.proposed` event, so `query
  get_decisions_changed_since` and `get_recent_activity` listed its classification as a
  `context_change` that named no decision, and `situational --since-ts` (and the MCP
  `get_situational_decisions` `since_timestamp`) read the decision as `since=unchanged` /
  `changed_since=false`: on the cell, 124 of 130 captured decisions from the last month read
  unchanged, against 22 of 22 recorded by hand as changed. Each decision a classified capture
  records is now a `new_decision` row naming it, cited by the classification event that
  recorded it (`event_origin`, `citation_id`, `ts` and actor are the event's own), and the
  classification keeps its own row. A window holds the events recorded in it: a decision is new
  when its classification is, whenever the turn it records was said. `get_decisions_added_since`
  is unchanged, since it already listed a capture's draft as added. (hivemind-svqxz)
- **`HIVEMIND_AGENT_SESSION` keeps one actor through the capture helpers and the skill's direct
  form too, as the docs said.** The README, `docs/DOGFOOD.md` and `docs/AGENT_DECISION_CAPTURE.md`
  tell a person outside Gas City to set it, and the MCP server and bare `emit` honoured it, but
  `/hivemind-capture:capture-decision`, `plugins/hivemind-capture/scripts/capture.sh`, the
  repo-local `.claude/scripts/capture-decision.sh --source agent` and the `$hivemind-capture`
  skill's direct `hivemind emit` form (and the `hivemind-context` plugin's write verbs) named the
  session from `GC_AGENT`/`GC_ALIAS` and the client's own per-run id and ignored it, so with `HIVEMIND_AGENT_SESSION=alice-laptop` set
  every session still recorded under a new `agent:claude:<uuid>` (or `agent:claude:manual-session`).
  All of them now take the variable first, then `GC_AGENT`/`GC_ALIAS`, then the client's session
  id, the same order as the CLI and the MCP server; the helper still records the per-run id in
  `source_ref`. The skill no longer says the CLI files an unnamed tool as `codex` (it is
  `unknown`), and `hivemind mcp --help` describes `--session-id` with the order the README gives
  instead of "a generated id". (hivemind-9v7vu)
- **The server's graph build no longer decodes every transcript it replays.** A cell ledger is
  97% `ingest.batch_received` events, and the build decoded, validated and noted each transcript
  whole to keep the four small facts a classification reads from it (which batch it was, which
  session shipped it, with which agent tool, and when its newest turn was said), then dropped
  the rest. The build now reads those four facts per batch straight from the stored payload,
  without parsing the turn text, and projects whole only the newest received batch of each
  actor, which is what gives the actor its node. A batch's facts are known before the
  classification that names it, and every other event replays as before. On a copy of the cell
  ledger (150,681 events, 146,742 of them received batches) a cold start to listening fell from
  3.6-4.2 s to 1.8-2.1 s on SQLite (peak memory 108 to 93 MB) and from 4.1-4.7 s to 2.6-3.7 s
  on Postgres (peak memory 271 to 107 MB), and `GET /v1/graph` came back byte-identical on
  both. A received batch is no longer checked against the batch shape while the graph is built
  unless it is the newest of its actor, so one batch of another shape no longer stops a build.
  (hivemind-v79o2)
- **`verify`, `why` and `--summary` count the days a superseded decision held to the link, not
  to the replacement's own date.** `still_holds.days_held` ended at the superseding decision's
  `occurred_at`, so linking a decision decided on 24 September as the replacement of one decided
  on 22 September (`emit decision.superseded --old A --new B`, run on 20 October) said it "held
  2 days until it was superseded" while the same answer said the change was open since
  20 October. The re-examine items, the propagation line and the attention findings already date
  a change by the link. The days now run to the time the supersession was recorded (the earliest
  link when concurrent supersessions both survive): 28 days in that example. A supersession
  recorded without a time keeps the replacement's date; a link recorded before the decision it
  replaces still leaves the figure out. (hivemind-ot2gf)
- **One tenant's graph refresh no longer stalls every other tenant's reads, and a SQLite
  refresh no longer scans the whole tenant for one new event.** The server kept every tenant's
  graph behind one lock, and a refresh after a write cloned the tenant's graph, replayed into
  it and freed the old one with that lock held, so on a cell-sized ledger about a quarter of
  another tenant's reads waited 2 ms or more, up to 99 ms. Each tenant now has its own refresh
  lock: the clone and the replay run with no cache lock held, the cache is locked only to swap
  the new graph in, and the replaced one is freed after the locks are released. Two refreshes
  of one tenant still run one after the other, so no event is applied twice. On SQLite the read
  of the events after the last one applied, and the read of the latest offset, planned as a scan
  of the tenant's whole type index and a sort (22 ms on 150,000 events on a quiet host); a new
  index on `(tenant_id, event_id)` makes both a search. The index is built the first time a
  ledger is opened by this release; no event changes. (hivemind-cfiff)
- **Nine MCP tools are written once, so stdio and `POST /mcp` refuse a bad argument in the
  same words, and stdio no longer lists tools it cannot run.** `capture_evidence`,
  `capture_hypothesis`, `get_decision`, `get_relevant_decisions`, `search_decisions`,
  `dump_graph`, `summarize_decisions`, `classify_queue_list` and `classify_queue_submit` had a
  copy per transport, and the copies had drifted: over HTTP, `search_decisions` quietly used
  its default limit of 25 for a `limit` that was a string or negative, and
  `get_relevant_decisions` returned every decision for a `status` that was not a string, where
  stdio refused both. Both now refuse, with one message, on both transports. stdio's
  `tools/list` no longer offers `classify_queue_list` and `classify_queue_submit`, which only
  `POST /mcp` serves; a stdio call to either still answers "unknown tool". (hivemind-q3rpv)
- **`query get_decisions_changed_since`, `get_recent_activity` and `get_decisions_added_since`
  now say a passive capture replaced or rejected a decision.** A classified capture's
  `supersedes_id` and `rejected_by` are edges of the graph and no `decision.superseded` or
  `decision.rejected` event, so the feeds, which list events, showed the classification as a
  context change that named no decision: a decision a capture replaced stayed unchanged in them,
  the decisions that rested on it were not told, and the same supersession recorded by hand was
  listed. Each such position is now a `supersession` or `status_change` row, and a
  `stale_premise` row for the decisions that followed the replaced one, cited by the
  classification event that recorded it (`event_origin`, `citation_id`, `ts` and actor are the
  event's own); `get_decisions_added_since` lists the replaced decision as changed. `--status`
  filters on these three queries read where the graph says a decision stands, so a decision a
  capture replaced is found by `--status superseded` and one a capture accepted by `--status
  accepted`, and `--topic` finds a capture's decision by the topics its node holds. The three
  queries now read the graph beside the ledger: `query` builds it as it does for the other
  history reads (`get_recent_decisions`, `get_changed_decisions`). A hypothesis a capture refutes
  is still not a feed row; `why` and `get_changed_decisions` show it. (hivemind-pf6xi)
- **`hivemind map --alpha 0.0,0.5` now works, and `hivemind review --help` says who the
  reviewer is.** `map --help` told you to pass `0.0,0.5` to get both maps, but the flag took one
  value per use and refused the comma form with an invalid-float error (only `--alpha 0.0 --alpha
  0.5` worked). It takes the comma list now, as the server's map route always did, and still
  takes the repeated flag. `review --help` said each verdict is written "from `--actor`" and that
  `--unreviewed-only` skips decisions "someone" already ruled on. The reviewer is the global
  `--actor`, which goes before `review`, and `--unreviewed-only` skips only the decisions that
  reviewer already accepted, rejected or superseded. `review --actor` is the glob filter on the
  actors who made the decisions, and its help now says so. (hivemind-piwlc)
- **`hivemind import --help` no longer offers a Confluence connector, and the import docs no
  longer teach commands that do not exist.** `import` and `import connector` said they read the
  history of a "Google Doc, Confluence page or git-tracked file", but `import connector run`
  registers only the git-file and Google Docs connectors and `connector auth` accepts only
  `gdocs`. `TEXT_IMPORT_AND_DIFF_SEMANTICS.md` taught the retired `hivemind suggest
  document-candidates` and `materialize-document-candidates`, said extraction writes no events, and
  said any IANA timezone is accepted. It now describes `import documents --extractor-command` and
  `--llm-response`, which write each candidate as a `proposed` decision, and says only UTC is
  accepted. `INGESTION_CONNECTORS.md` no longer shows `connector auth confluence` or presents the
  `POST /v1/import/connector` route as built, and `ARCHITECTURE.md` no longer names the `suggest`
  namespace. (hivemind-htomw)
- **The background scorer no longer offers its model text the write path refuses, and no longer
  pays again for a decision it keeps failing.** For a classifier capture it listed the expressed
  confidence, the topic keys and the options joined on one line as quotable fields, but a quote
  is accepted only from the title, rationale, chosen option and each option alone, so a quote from
  any of the others was refused and the decision stayed unassessed. Each option now has its own
  line, and the confidence and topic keys ride in as `Recorded context` lines the prompt says never
  to quote (prompt version `assessment-v3`). A decision with no assessment was also sent to the
  model on every 30 s pass, one metered call each, with no limit. A decision whose call fails or
  whose answer the write path refuses is now tried at most 3 times, 5 minutes apart, then left
  unassessed until the server restarts, with a warning when the worker gives up. Only a server
  with `ANTHROPIC_API_KEY` set on the SQLite backend runs the scorer. `CAPTURE_CLASSIFIER.md` now
  says the scorer's model is the `model` field of `decision.scored` (and the classifier's the
  `classifier_model` field of `ingest.batch_classified`), not provenance. (hivemind-m88ju)
- **`hivemind import connector` on a git file that was renamed no longer drops its history.** The
  git connector listed versions with `git log --follow --reverse`, and git applies `--follow`
  only while walking from the newest commit, so for a file moved into `adr/` or `docs/` the
  list lost every commit before the move and the import went ahead on the rest with nothing
  said. Each version now carries the path the file had at that commit, and `git show` reads it
  under that name, so both names are walked. A commit or Drive revision whose date cannot be
  read is now an error; before, it was stamped with the time of the import. (hivemind-jfvdh)
- **A Postgres server no longer starts a classifier and scorer that read an empty SQLite file.**
  `hivemind serve` started both Layer-3 workers whenever `ANTHROPIC_API_KEY` was set, whatever the
  backend. They open `ledger.sqlite` under `HIVEMIND_DIR` and work the `local` tenant, so with
  `HIVEMIND_DATABASE_URL` set (the compose default and the Fly recipe) they created an empty
  `ledger.sqlite`, saw no batch and no decision, and classified and scored nothing without a log
  line saying so. They now start only on the SQLite backend. On Postgres a key that is set is
  ignored and the server logs one warning at start; classify keylessly with `classify-queue`. The
  docs that promised the key switches the classifier on there now say it does not. (hivemind-tbu78)
- **The server classifier (Worker B) keeps to the daily cap and does not resend a failed batch
  every 10 seconds.** A pass made one paid call per pending batch whatever
  `HIVEMIND_CLASSIFY_DAILY_CAP` said (only `classify-queue submit` honoured it), and a batch
  whose call or write failed stayed pending and was sent again on the next pass, for ever. It
  now checks the cap before each call, counting the classifications any worker recorded today
  and its own failed calls, and stops for the UTC day when the cap is spent. A batch whose call
  or write failed is logged once and left until the next UTC day (it stays pending). A reply
  the model stopped at `max_tokens` is reported as that, in the classifier, scorer and
  restatement judge alike, not as a JSON parse error. Pending batches are found with the heads-only
  reads `classify-queue list` uses, not by reading the whole ledger with its payloads every pass.
  (hivemind-49ddt)
- **`hivemind migrate --from <typo>` no longer leaves an empty ledger at the typo.** It opened
  the source before checking anything, and opening makes a ledger, so a mistyped `--from` ended
  with "nothing to migrate" and a `ledger.sqlite` behind it; every later read of that folder
  (`hivemind --hivemind-dir ./hivemnd query ...`) passed the no-ledger check and answered "0
  decisions". A source folder that holds no ledger is now refused before it is opened (exit 2,
  naming the folder), whether or not the folder exists, and nothing is created. A ledger that
  exists but holds no events for the tenant still gets "nothing to migrate". (hivemind-eapb1)
- **The admin token routes refuse a tenant that is not registered.** `POST /v1/agent-tokens`,
  `POST /v1/users` and `POST /v1/users/{id}/tokens` took `tenant_id` from the body or query
  (default `local`) and never checked it. On SQLite a typo'd tenant got `201` and a token that
  `/v1/whoami` reported as that tenant, so the UI's token sign-in looked fine, and then every
  other call failed `404 unknown_tenant`; on Postgres the foreign key refused it but the answer
  was a `500 internal_error` carrying the raw database message. `GET /v1/users?tenant_id=` answered
  a missing tenant with an empty user list. All four now answer `404 unknown_tenant`, naming the
  tenant, on both backends, and write nothing; an empty `tenant_id` is a `400`. A Postgres server
  where `local` was never provisioned answers the same when `tenant_id` is left out.
  (hivemind-on7rr)
- **`hivemind --json quality-scan` prints its report when there is nothing to file.** It printed
  the JSON report when a finding was waiting and a plain sentence when none was, so a scan on a
  schedule (`hivemind --json --actor agent:x quality-scan --dry-run | jq .filed`) worked until
  the first time the ledger was clean, which is the normal state. With `--json` the clean answer
  is the same report with `filed: 0` and `issues: []`; beside a cell (`HIVEMIND_API_URL` set) the
  notice that the local ledger was used is its `notice` key, not a trailing text line. Without
  `--json` the sentence stays, and a scan with findings prints the report indented, as other
  commands do, instead of on one line. (hivemind-r2076)
- **`classify-queue` over `HIVEMIND_API_URL` now sends `--tenant` (and `HIVEMIND_TENANT`) and
  `--actor`.** It sent only the bearer token, so a SQLite `hivemind serve` with a static key or
  none answered for tenant `local` as `service:api` whatever the flags said:
  `hivemind --tenant acme classify-queue list` showed local's queue with no warning, and `submit`
  either refused acme's batch ids (exit 3) or recorded the classification in local under the wrong
  name. The CLI now sends `X-HiveMind-Tenant` and `X-HiveMind-Actor` on every call, so a local
  ledger and a SQLite server agree. A server that knows the caller by token still takes tenant and
  actor from the token and ignores both headers, and `classify-queue --help` now says so.
  (hivemind-vx9ma)
- **A truncated `recall` gives the cursor to read the rest.** `query recall` and the
  `recall_decisions` tool took a `--cursor` / `cursor` and answered `truncated: true` when more
  decisions matched than `--limit` (at most 10), but never said what to pass as the cursor; only
  a caller who guessed that it is a count of records could page, and a count of the items shown
  is wrong once records linked as one decision fold into one item. Now the answer carries
  `ranked.next_cursor` (CLI `--json`, MCP, `GET /v1/decisions/recall`), which counts matched
  records, and the `--summary` footer reads `truncated=true next_cursor=<n>`. Pass it back as
  `--cursor` for the next page; the last page has `next_cursor: null`. A decision recorded twice
  whose records fall on two pages is shown on both, naming its other record in
  `also_recorded_as`. (hivemind-v3lrl)
- **`recent_decisions`, `get_decisions_added_since`, `get_recent_activity`,
  `get_decisions_changed_since`, `hivemind review`, project-scoped reads and the read notice no
  longer read every transcript the ledger holds.** Each of them walked the whole ledger and
  parsed every event, and on a ledger that is mostly received transcripts
  (`ingest.batch_received`) that cost seconds per call, growing with the transcripts rather
  than with the decisions. The history reads now fetch only the event types they report, plus
  the classification event a drafted decision names; a project-scoped read (`--project` on
  `recall`, `situational`, `reexamine`, `misfiled`, `get_project`, the decision-log
  export) fetches only the six `project.*` event types; and the line the CLI and the stdio MCP
  tools print when annotation rows could not be read fetches only the annotation rows. What the
  reads answer is unchanged, with one exception: `get_recent_activity` and
  `get_decisions_changed_since` no longer list a row for `ingest.batch_received`, the raw
  transcript, which names no decision. The classification row that follows it stays, and every
  other event type still has its row. (hivemind-gxypz)
- **`hivemind digest` keeps the newest decisions of a window and lets you read the rest.** A
  window that held more than 50 decisions showed an arbitrary 50, in decision-id order (a random
  uuid, with classifier drafts, `capture:<n>:<i>`, ahead of every other), and the rest could not
  be reached: the note said to raise `--limit`, which was already at its cap of 50. Now the
  digest orders its window newest decided first (ties by decision id), shows the first
  `--limit` of them (default and cap 50), and a window with more says which decisions it shows
  and gives a `next_cursor` (`--cursor` reads the next page; the JSON carries `next_cursor` and
  each entry its `decided_at`). A cursor past the end of the window is refused, not answered
  as an empty window. (hivemind-4focm)
- **`import documents` keeps a whole rationale, and refuses a field it does not know.** A line
  inside a `Decision:` block was read as a `key: value` field first, so a rationale line holding
  a colon (`See https://example.com/adr`, `Benchmark: p95 40ms vs 120ms`) was dropped as an
  unknown key together with every rationale line after it, and the import succeeded with a cut
  rationale and no warning. `rationale: first words` followed by indented lines kept only the
  first line. Now every line indented deeper than the `rationale:` key belongs to the rationale,
  colons or not, whether the text starts on the key line or the line after. A misspelled key
  (`evidnce:`, `chose_option:`) used to be dropped the same way; an unknown field indented under
  the `Decision:` marker, or an indented line that is no field, list item or rationale
  continuation, is now reported as a validation error naming the line, and that block writes
  nothing. Text that is not indented under the marker, such as a heading, prose after the block
  or the `# hivemind-source-ref:` line of the next prepared page, is still ignored.
  (hivemind-d4fbt)
- **Linking an existing decision as the superseder (or existing evidence as the refutation) no
  longer reads as a change that happened long ago.** `SUPERSEDES` and `REFUTES` edges carried no
  time of their own, so the attention findings and the re-examine items ordered a change by the
  edge's ledger offset but aged it by a node's creation time: the superseding decision's
  `occurred_at`, or the refuting evidence's `recorded_at`. `emit decision.superseded --old A
  --new B` on a `B` captured months earlier, `emit relation.added --kind refutes` from old
  evidence, and a classifier draft whose `supersedes_id` was said days before it was classified
  all dated the change at the newer node's birth, so `propagation_unfinished` fired the same day
  on `A`, `verify` showed `[UNFINISHED]`, and a dependent's `premise_superseded` carried that
  old `basis_at`, although nobody could yet have reaffirmed anything. Both edges now carry
  `added_at`, the time of the event that recorded them (memory, Postgres and Kuzu), and the
  14-day window, the unfinished check and `basis_at` run from the earliest such edge; a node's
  time is used only for an edge with none. The ledger is unchanged; an edge projected by an
  earlier build gets its time when the graph is rebuilt from the ledger. (hivemind-xhoie)
- **Moving a ledger into a cell no longer attaches a classifier draft's acceptance, retitle,
  score or link to a different draft.** A draft's id is `capture:<offset of its batch
  event>:<index>`, and `hivemind migrate` / `POST /v1/ledger/replay` renumber every event in the
  destination, but the events that act on a draft (`decision.accepted`, `decision.rejected`,
  `decision.retitled`, `decision.moved`, `decision.reaffirmed`, `decision.scored`,
  `relation.added`, `suggestion.surfaced`, a later classification that supersedes or restates it,
  and the rest) were copied with the old id. On a destination that already held events, the
  acceptance landed on whatever draft sat at that offset there and the moved draft read
  `proposed`, while the move reported success. The source now sends each draft id as
  `capture:<uuid of the batch event>:<index>` and the destination writes it as
  `capture:<where that batch event landed>:<index>`; an id sent in the offset form is refused,
  and nothing is written, because the destination cannot tell which batch it meant. A draft id
  that names no earlier event of the source ledger is refused before the first event is sent.
  Replay clients that send draft ids themselves must use the uuid form. A `suggestion.surfaced`
  on a draft keeps its `finding_id`, a hash of the old id, so the moved draft's finding reads as
  not yet surfaced and can be surfaced again. (hivemind-sh1ic)
- **`hivemind review` shows a decision's options by their labels, so the reviewer can see what
  was chosen.** The prompt listed each option by its node id (`option-3f1c…`, with `(chosen)`
  after one of them), which told a person nothing about an agent's choice. It now reads
  `Async (chosen)` and `Sync`, and a label that was recorded as a slug shows what was recorded
  beside it (`Upheld (recorded as: name-a-upheld)`). `query recent` / `recent_decisions` carry
  the same labels: each item's `option_ids` list is replaced by `options`, a list of
  `{option_id, label, recorded_as?}` (`chosen_option_id` still names the choice). Options list
  in option-id order, as before. (hivemind-7v77s)
- **`hivemind_compact_view` and chain-mode `summarize_decisions` no longer fail on a decision
  that was superseded twice.** Once a decision had two superseders, or superseded two decisions,
  both answered "supersession chain branched" for it and for everything older in its chain (MCP,
  REST, CLI and TUI), though the write side allows both and concurrent supersessions are meant to
  both stand. The compact view now ends the line at the fork and lists it under
  `supersession_chain.forks` (`at`, `side` older or newer, `candidates`); nothing is picked as the
  current decision, and the decision shown reads `superseded`. A decision that superseded several
  counts them all in `elided.superseded_decision_count`. The chain summary names each branch's
  decisions, cites them, and reads "Current status: none" when no single decision is current.
  The same pair superseded twice (a retried supersede) was also read as a branch on the in-memory
  graph, which the cell's graph cache holds; the walk now counts it once. `chain`
  (`get_supersession_chain`) still answers with an error at a real fork. (hivemind-pydlc)
- **A node in the DOT graph export now draws on two lines instead of showing a literal `\n`.**
  `hivemind dump --format dot`, the MCP `dump_graph` tool, the HTTP `/mcp` dump and the TUI
  `--dot-output` export all escaped the line break they had just added, so Graphviz drew
  `Use SQLite\nstatus: accepted` on one line. The title, statement or reason is escaped first and
  the break is added after, for decisions, hypotheses, decision requests, blockers,
  reaffirmations, notifications and the TUI neighborhood. A quote or backslash in the text is
  still escaped. Nothing is stored differently; export again to get the fixed file.
  (hivemind-2xdyr)
- **`situational` and `get_situational_decisions` now match a multi-word topic key to the files
  being touched.** A decision with the topic `project-scope` never surfaced for
  `src/queries/project_scope.rs` or the branch `feature/project-scope`: the situation is split
  into words (`queries`, `project`, `scope`) and the key was compared whole, so a kebab key could
  not equal any of them and only a one-word key such as `auth` ever matched. A topic key now
  matches when every one of its parts is a word of the situation, and the `topic_key` reason
  names the whole key (`project-scope`). All parts are needed: `project-scope` does not surface
  for `src/queries/project.rs` alone, and two touched paths that each hold one part together hold
  both. A part the situation never holds (`test`, a one-letter part) is not asked for, and a key
  made of nothing else matches nothing. A one-word key matches as before. The score counts the
  situation words the key covers. Nothing is rewritten or migrated. (hivemind-1huzw)
- **Topic filters now find a decision however the topic is typed.** Capture stores every topic
  key as lowercase kebab (`Release Plan` is `release-plan`, `MCP` is `mcp`), but the reads compared
  the typed text, so `hivemind query search --topic "Release Plan"`, `get_relevant_decisions`
  with `Release Plan`, `hivemind export --topic MCP`, the `--topic` hint of `why` and
  `disagree`, active blockers and the recent, added-since and changed-since reads all answered
  "nothing recorded on this topic" with exit 0. Every one of them now normalizes the topic it is
  given the way capture does (`Release Plan`, `release_plan` and `RELEASE-PLAN` find
  `release-plan`); a topic with no letter or digit names no key and is dropped from a filter
  (`get_relevant_decisions` refuses it). The stored side is normalized at comparison time too, so
  a key that reached the ledger un-normalized (a classifier draft, a blocker report, an event
  appended without capture, e.g. `org.incident`) is found by `org-incident` and `org.incident`
  alike. Decisions, blockers and requests keep their topic keys as recorded and every read
  returns them that way; nothing is rewritten or migrated. (hivemind-vwoau)
- **`analyze_failure_modes` and `scan_misfiled_decisions` now read every decision, not the first
  1,000.** On a ledger of about 1,500 decisions, `analyze_failure_modes` reported
  `corpus_stats.total_decisions: 1000` and every breakdown left out the 500 newest decisions,
  and `scan_misfiled_decisions` looked only at the 1,000 alphabetically first decision ids, so a
  misfiled decision whose id sorted later was never flagged and a re-run after moving the
  flagged ones read clean. Neither reply said it had stopped early. The failure-mode report now
  pages its outcome and context reads to the end and counts the whole corpus
  (`total_decisions` is the decision count). The misfiled scan searches each foreign key across
  the whole tenant, and its `truncated` / `next_cursor` page through every match, however many
  decisions the tenant holds. (hivemind-11adr)
- **`decision_quality_candidates` with `only_with_signals` no longer repeats decisions across
  pages or ends the walk early.** A page examines `limit` decisions and keeps the ones that carry
  a signal, but `next_cursor` moved on by the number kept. On a ledger of about 1,500 decisions
  at `limit` 25, a page that kept two returned `next_cursor: "2"`, and the next page read the same
  two again; a window of 25 with no signal returned `next_cursor: null` while `truncated` was
  still `true`, so the walk stopped with every newer decision unread. The cursor now counts the
  decisions examined, so each decision with a signal comes back once and a truncated page always
  has a cursor. A page can hold fewer than `limit` decisions, or none, and still be truncated:
  keep following `next_cursor` until `truncated` is `false`. (hivemind-nwfw6)
- **MCP `recent_decisions`, `get_changed_decisions` and the dated timeline in
  `get_decision_neighborhood` now read the caller's tenant, not `local`.** Over stdio
  (`hivemind --tenant <id> mcp`) and over `POST /mcp` (a tenant header under the static key, or an
  agent token minted for a tenant), these three tools read the `local` tenant's ledger whatever
  tenant the call was opened for. A caller in tenant `acme` got `local`'s decisions (titles,
  rationales, quotes, actors) from `recent_decisions` and none of its own, an empty
  `get_changed_decisions`, and a `why` whose graph was `acme`'s but whose timeline was not. Only
  the `local` tenant was ever read so far (the city cell has no other), so no wrong answer has been
  seen; it would have been a cross-tenant read as soon as a second tenant existed. The ledger
  value a call is opened with now carries its tenant, and `EventLedger::bound_tenant` has no
  default: a ledger must say which tenant its plain `append`, `read`, `replay_from` and
  `latest_offset` address, so none can quietly mean `local`. The REST routes, which pinned the
  tenant themselves, are unchanged. (hivemind-xm9by)
- **A decision a passive capture accepted, rejected or replaced now shows it in its timeline and
  the changed list, and `recent` lists a proposal a capture replaced as `superseded`.** A
  classified capture can state positions inside itself (`accepted_by`, `rejected_by`,
  `supersedes_id`, `premised_on_ids`, and `refutes_ids` on evidence). The graph, `status`, `why`
  and re-examine saw them; the timeline (`why`'s `timeline`, `GET /v1/decisions/{id}/timeline`) and
  `get_changed_decisions` read only the ledger's fact events and did not. A capture accepted at
  capture had a timeline of just `recorded`; a decision replaced through a capture's
  `supersedes_id` had no `superseded` entry and never reached the changed list, and the
  decisions that followed from it got no `premise_superseded`. These now read the capture's
  positions as the entries they stand for: `accepted` and `rejected` at the capture's offset and
  the time the decision was said, `superseded` and `supersedes` for `supersedes_id`,
  `premise_superseded` and `premise_rejected` for the followers, and `premise_refuted` for the
  decisions that rested on a hypothesis an evidence capture refutes (dated by the classification
  event, as evidence carries no time of its own). An acceptance, rejection, supersession or
  refutation the ledger also holds as its own event is listed once, as that event. Separately,
  `get_recent_decisions` and `get_decisions_added_since` took a proposal's status from the
  ledger's events alone, so a `decision.proposed` decision a capture replaced was listed as
  `accepted` or `proposed` and `--status superseded` did not find it; they now take it from the
  graph, as classified drafts already did. (hivemind-u46nq)
- **MCP `capture_decision` and `supersede_decision` no longer read the whole ledger, so a write
  answers in seconds instead of minutes and the client no longer times out on a capture that was
  recorded.** On the city cell (about 130,000 events, 127,000 of them raw transcripts) a capture
  took about two minutes and the agent's client gave up first, believed the capture had failed,
  and retried into a duplicate. The write itself was fast; the time went into the work around it.
  Each lookup the command layer made (the topic vocabulary, the question and restatement checks,
  the transcript and grounding checks) paged through every event of every type, and naming a
  premise decision rebuilt the whole decision graph from event 0. One capture made about 15 full
  passes, and a realistic one about 33. Every such lookup now asks the ledger only for the event
  types it needs, through the existing type index, and the HTTP server hands the write its cached
  graph (which catches up from its last offset) instead of replaying the ledger. REST
  `POST /v1/decisions` and `/supersessions` resolve premises from the same cache. The same fix
  applies to `disagree_decision`, `move_decision`, `retitle_decision`, `ground_decision`,
  `reaffirm_decision`, `get_decision_outcome`, `get_supersession_chain`,
  `get_decision_neighborhood` and `hivemind_compact_view` over HTTP, which each rebuilt the graph
  per call. Measured on a private Postgres copy of the cell ledger (130,581 events), a minimal
  `capture_decision` fell from 59.9 s and 67.3 s to 1.2 s, a realistic one (registered project,
  premise decision, new evidence, question, cited ids, declared topic) from 207.7 s to 1.8 s, and
  `supersede_decision` with a premise and new evidence took 2.1 s; Postgres page reads for one
  capture fell from 1,926 and 4,251 to about 150 for all three writes together. A guard test
  (`src/mcp/write_cost_tests.rs`) counts the ledger reads of a write and fails if it reads a
  transcript or if the reads grow with the number of transcripts in the ledger. (hivemind-f38ka)
- **`classify-queue list` and `submit` aimed at a folder with no ledger now refuse, and `submit`
  refuses a batch id nobody received.** With `HIVEMIND_API_URL` unset, `classify-queue list` under
  a mistyped or fresh `--hivemind-dir` made an empty `ledger.sqlite` there and exited 0 with an
  empty queue. That mattered most for the capture plugin's optional `Stop` hook
  (`scripts/check-classify-queue.sh`), which runs it at the end of a session: every project
  without `./hivemind/` got a stray ledger just because the hook was installed. `list` now
  refuses like the other read verbs and `submit` like the verbs that act on a ledger: exit 2,
  nothing printed, nothing created, and the message names the verb and the folder. With
  `HIVEMIND_API_URL` set they ask that server as before. `list` now also reads the database named
  by `--database-url`; it always read local SQLite, even where `submit` wrote to the database.
  `submit` with a batch id the ledger never received used to append an `ingest.batch_classified`
  event that covered nothing and exit 0. It now refuses naming the id, writes nothing and exits 3;
  `POST /v1/classify-queue/submit` answers 404 and the MCP tool `classify_queue_submit` returns a
  tool error, because the check is the same on all three. A batch that is already classified
  is still accepted again. The hook script now looks for a ledger first and prints nothing and
  creates nothing in a project without one, whichever hivemind is installed. `--help`, the README
  and the capture docs say the same. (hivemind-odmnc)
- **The slug-label warning no longer fires on ordinary hyphenated words.** A capture or supersede
  that sent an option label like `e-mail`, `opt-in`, `read-only`, `follow-up`, `real-time`,
  `built-in` or `self-hosted` was told the label "looks like a slug" and was offered words that
  were worse than the label (`E mail`). The warning now fires only on a shape ordinary words do
  not have: an underscore (`use_postgres`), three or more parts joined by `-` or `_`
  (`name-a-upheld`), or a part that is an acronym in lowercase (`direct-cli`, `mcp-server`; prose
  writes `CLI`). Any other two-part hyphenated label, `direct-store` and `read-only` alike, gets
  no warning, because no shape tells them apart and a missed slug costs less than a warning on a
  good label. A suggestion that would hold a one-letter fragment of a split word (`E mail`, `K
  means clustering`) is not offered at all; `a` and `I` are words and stay. The label is recorded
  as sent either way, and readers show labels as before. (hivemind-lwhfk)
- **`classify-queue submit` with `HIVEMIND_API_URL` set now exits 3 for a batch id the server never
  received, as it does on a local ledger.** The server's 404 used to come back as exit 2 and
  "invalid command line input: server returned 404 Not Found: ...", although the input was fine
  and `--help` and the capture docs say exit 3. It now exits 3 with the server's own words
  (`error: invariant violated: ingest batch does not exist: <id>. ...`), the same text a local
  ledger prints, and nothing is recorded either way. A 404 that does not carry the server's
  `not_found` error (a proxy, the wrong host) is still the generic failed request, exit 2.
  (hivemind-d6tya)
- **`classify-queue list` and `submit` with `HIVEMIND_API_URL` set now exit 4, "unknown tenant",
  for a token whose tenant the server does not hold, as a local ledger does — not exit 3 "invariant
  violated".** The server answered an unregistered tenant with the same `404` `not_found` it uses
  for an id that does not exist, so the previous fix read it as a refused batch id (exit 3), and an
  agent that skips a refused id would have skipped every batch for one configuration reason. The
  server now answers an unregistered tenant with `404` and its own `error.code`, `unknown_tenant`
  (message `unknown tenant '<id>': ...`, no longer prefixed `ledger storage error:`), on every
  `/v1/*` route (`POST /mcp` carries the same text in its tool error); the CLI prints it exactly as
  a local ledger does
  (`error: ledger storage error: unknown tenant '<id>': ...`, exit 4). An unknown batch id is
  unchanged: exit 3, in the server's words. A REST client that matched `not_found` to detect an
  unregistered tenant must match `unknown_tenant` instead. `docs/DEPLOYMENT.md`, the capture docs
  and the plugin's `classify-queue` command say the same. (hivemind-vf0iz)
- **A verb that acts on an existing decision or project, aimed at a folder with no ledger, now
  refuses instead of making an empty ledger and answering not_found.** `disagree`, `supersede`,
  `move`, `retitle`, `ground`, `reaffirm`, `review`, `quality-scan`, `restatements apply`, and
  `project link`, `unlink`, `anchor`, `declare-topic` and `use <handle>` can never succeed on a
  ledger that does not exist, because there is nothing for them to act on. Under a mistyped
  `--hivemind-dir` they still created an empty `ledger.sqlite` there and answered "no decision
  matches that description" (or "project does not exist"), which left a stray ledger behind and
  sent the user looking for a missing decision instead of a wrong folder. They now refuse like
  the read verbs do (hivemind-i6ply): exit 2, nothing printed, nothing created (no folder, no
  ledger file), and the message names the verb and the folder. A folder that holds a ledger, even
  one with no decision in it, answers as before. The verbs that record a ledger's first event
  keep making it (`emit`, `ask`, `import`, `project register`, `ingest slack-thread`,
  `tenant create`), a configured `--database-url` names no folder and is unchanged, and so is the
  refusal with `HIVEMIND_API_URL` set. `hivemind mcp` and `serve` over a missing folder are
  unchanged: a server legitimately starts before the first capture. `--help` for these verbs,
  the README and the capture and context docs say the same. (hivemind-5t1uk)
- **A read aimed at a folder with no ledger now refuses instead of making an empty ledger and
  answering "No decision found".** With `HIVEMIND_API_URL` unset, `--hivemind-dir` pointing at a
  mistyped or fresh folder, and a read verb (`query why`, `verify`, `recall`, `search`,
  `situational` and the rest of `query`), the CLI created an empty `ledger.sqlite` there, answered
  that nothing matched and exited 0, which reads the same as a real ledger with no match; a
  checker took it for a real answer twice. The read verbs now refuse (exit 2, nothing printed,
  nothing created: no folder, no ledger file, nothing under `export --out`) and the message names
  the folder it looked at and the verb: `query`, `digest`, `dump`, `export`, `map`, `reexamine`,
  `tui` (before the terminal UI starts), `restatements propose`, `project list`, `project show`
  and `project decisions`. This is the case hivemind-rmdi1, 3wism, nrk9f and wp0n7 left open;
  they refused it with the cell URL set. A folder that holds a ledger, even one with no decision
  in it, answers as before. Writes keep making the ledger on their first run (`emit`, `ask`,
  `import`, `project register`, `ingest slack-thread`, `tenant create`); the verbs that act on a
  decision or project a ledger already holds are refused too (see the next entry). A configured
  `--database-url` names no folder and is unchanged. Which verbs only read is now one exhaustive match on the verb, with no catch-all
  case, so a verb added later does not compile until someone decides. `--help` for these verbs,
  the README and the capture and context docs say the same. (hivemind-i6ply)
- **A decision that revises its premise is no longer asked to re-examine itself.** A decision B
  captured with both `--follows A` and `--supersedes A`, and the only decision to supersede A, is
  the change itself. `verify A` counted it as a dependent of A, `hivemind reexamine` and
  `get_reexamine_items` listed an open item asking whether B still holds now that A was superseded
  (by B), every re-examine marker showed it, and B's own `verify` read "does not hold" because its
  premise had gone stale. B now has no item, is not counted in A's "N of M dependents
  re-examined", shows no marker, and holds. A decision that is one of several superseders of A
  keeps its item, because it is told about the others. `verify`'s propagation line and the
  `propagation_unfinished` finding now count the same dependents. No new event; an existing ledger
  reads right on the next call. (hivemind-tfmfn)
- **`reaffirm` on a decision that revised its premise says why it is refused, and the history no
  longer lists the revision as made stale by its own supersession.** `hivemind reaffirm B --premise A`
  (and the MCP `reaffirm_decision` tool and REST `POST /v1/decisions/{id}/reaffirmations`), where B
  is the only decision to supersede A, was refused with "A has not changed". A had changed: B
  superseded it. The refusal now reads "B is the decision that superseded A, so there is nothing for
  it to re-examine about it"; with no premise named, the same sentence names every premise B alone
  superseded. A premise nobody changed keeps "has not changed". In the recent-activity feed, the
  changed-since listing and the added-since listing, the stale-premise row for A's supersession by
  B no longer names B among the decisions made stale; with several superseders, each still appears
  on the others' rows. Nothing is refused that was not refused before, and no event is written or
  changed. (hivemind-851kw)
- **A below-floor decision now ends its row in `floor=below` in every list, not only in search,
  relevant and situational.** The list a person asks for first ("what did we decide today?",
  `recent_decisions`) showed an Apple-vs-apple decision with no marker, and so did
  `get_decisions_added_since`, `rank_decisions_by_importance`, `get_contested_decisions`,
  `get_changed_decisions`, `recall` and `project decisions`, although the entry above says a list
  row ends in `floor=below`. Each of those rows now ends in it, and `situational` moves its marker
  from the middle of the row to the end. The JSON items of the same lists carry `binds`,
  `undo_cost` and `floor` like every other read of a decision. The marker is information, not a
  filter: the ranking, the windows and the order are unchanged. (hivemind-ekmp4)
- **`scan_misfiled_decisions` ends a below-floor decision's row in `floor=below` too.** The
  ekmp4 entry above says a list row ends in the marker, but the misfiled report was left out: an
  Apple-vs-apple decision in it showed no marker, and its JSON and MCP items carried no `binds`,
  `undo_cost` or `floor`. They do now, like every other read of a decision. A row that names a
  decision by id alone (`get_supersession_chain`) carries none, and the docs say so. The report
  is still only a report: nothing is filtered, reordered or moved. (hivemind-h2we6)
- **`hivemind query` no longer answers "0 decisions" from a ledger it made up when
  `HIVEMIND_API_URL` points at a cell.** Only `classify-queue` honours that variable; `query`
  (and the plugin's `/hivemind-capture:query-decisions`) reads a local ledger and never contacts
  the server, so a person who followed `docs/KEYLESS_CAPTURE.md` to a cell and ran Step 5 from a
  fresh folder got an empty `./hivemind/ledger.sqlite` created and an empty answer, and from a
  folder that held some other ledger an answer from it, with nothing saying either was not the
  cell's. With `HIVEMIND_API_URL` set, `query` now refuses when `--hivemind-dir` holds no ledger
  (and creates none), and otherwise answers with a `notice` that the answer is the local ledger's.
  A configured `--database-url` is a direct read of the shared data and is unchanged. Step 5 now
  names the HTTP reads that do reach a cell (`GET /v1/decisions/search`, `/relevant`) and says the
  CLI reads a local ledger. (hivemind-rmdi1)
- **`hivemind emit`, `digest`, `review`, `export` and `dump` no longer use a ledger silently when
  `HIVEMIND_API_URL` points at a cell.** The same gap as `query` above, and worse for `emit`:
  with the variable set, `emit decision.capture` from a fresh folder recorded the decision into a
  `./hivemind/ledger.sqlite` it created, exited 0 and named no path, so the cell never got it and
  nothing said so; the four reads created an empty ledger and answered from it. All five now follow
  the `query` rule. With `HIVEMIND_API_URL` set and no `--database-url`, a fresh folder is
  refused (exit 2, nothing created, nothing recorded, and the message says the verb uses a local
  ledger and points at the cell's HTTP routes), and a folder that holds a ledger works and says
  which ledger it used: a `notice` key under `--json` or a `notice:` line after a text answer,
  and on stderr for `emit`'s bare id and `dump`'s DOT, so `$(hivemind emit ...)` and `| dot` keep
  reading a clean stdout. `--help` for these verbs and for `query` says the same. A configured
  `--database-url` is a direct use of the shared data and is unchanged. (hivemind-3wism)
- **`hivemind disagree`, `supersede`, `move`, `retitle`, `ground`, `ask`, `import`,
  `restatements`, `map` and `quality-scan` no longer use a ledger silently when
  `HIVEMIND_API_URL` points at a cell.** The ten verbs the entry above left out. With the variable
  set, `ask` recorded the question and `disagree`, `supersede`, `ground`, `move` and `retitle`
  wrote their event into a `./hivemind/ledger.sqlite` they created, and `map`, `restatements
  propose` and `quality-scan` answered from an empty one, exiting 0 with no word that the cell
  never saw any of it. They now follow the `query` rule: with `HIVEMIND_API_URL` set and no
  `--database-url`, a fresh folder is refused (exit 2, nothing created, nothing recorded, and the
  message names the cell's HTTP route or MCP tool that does the same), and a folder that holds a
  ledger works and says which ledger it used, as a `notice` key under `--json` or a `notice:` line
  after a text answer (on stderr where the answer is a JSON array, as for `map` with several
  `--alpha` and `restatements apply --json`). `import prepare-documents` opens no ledger and is
  unchanged. `--help` for these verbs says the same. A
  configured `--database-url` is a direct use of the shared data and is unchanged.
  (hivemind-nrk9f)
- **`hivemind project`, `ingest slack-thread`, `tui` and `tenant create` no longer use a ledger
  silently when `HIVEMIND_API_URL` points at a cell.** The verbs the two entries above left out.
  With the variable set, `tenant create` put a tenant into a `./hivemind/ledger.sqlite` it created
  (so a person following the cell docs had a tenant in a made-up local ledger, not the cell's),
  `project register`, `link`, `unlink`, `anchor` and `declare-topic` and `ingest slack-thread`
  wrote into one, and `project list`, `show`, `decisions`, `use <handle>` and `tui` answered from
  an empty one, all exiting 0 with no word that the cell never saw any of it. They now follow the
  `query` rule: with `HIVEMIND_API_URL` set and no `--database-url`, a fresh folder is refused
  (exit 2, nothing created, nothing recorded, and the message names the cell's HTTP route or MCP
  tool that does the same, or says the cell has none, as for the project registry), and a folder
  that holds a ledger works and says which ledger it used, as a `notice` key under `--json` or a
  `notice:` line after a text answer. `tui` is refused before the terminal UI starts and prints
  its notice on stderr once it has exited. `project show --current` and `project use --clear`
  open no ledger (only the current-project file) and are unchanged, as are `quickstart`,
  `migrate`, `serve`, `mcp`, `slack-app`, `connector`, `classify-queue` and `import
  prepare-documents`. The guard now names every verb and subcommand with no catch-all case, so a
  verb added later does not compile until someone decides whether it needs the guard. `--help`
  for these verbs says the same. A configured `--database-url` is a direct use of the shared data
  and is unchanged. (hivemind-wp0n7)
- **The capture skill and command no longer offer `--kind blocker` or `--kind notification`.**
  `capture.sh` refuses both (exit 2, nothing recorded: "kind blocker is known but does not yet
  have a canonical CLI capture path"), yet the active-capture skill taught `/capture ... --kind
  blocker` and the capture command's argument-hint listed both kinds, so an agent that followed
  them lost the capture. Both copies of the skill, the argument-hint, the plugin README and
  `capture.sh --help` now offer `decision`, `evidence` and `hypothesis`, and say the other two
  have no capture path yet (planned). A test pins that neither the skill nor the hint teaches a
  kind `capture.sh` refuses. (hivemind-udvza)
- **`capture.sh "<text>"` with no `--kind` no longer ends in the blocker/notification refusal.**
  The classifier it asks was offered `blocker` and `notification`, and when it picked one
  `capture.sh` exited 2 ("kind blocker is known but does not yet have a canonical CLI capture
  path") and recorded nothing, for a kind the caller never chose. The classifier is now offered
  only decision, evidence, hypothesis and none, and any other answer (those two, or a kind it
  made up) ends as "No HiveMind capture emitted: classifier returned kind=<kind>, which has no
  capture path." and exit 0: nothing is recorded, and it is not forced into the nearest recordable
  kind, since a blocker is not evidence. An explicit `--kind blocker` or `--kind notification`
  still exits 2. (hivemind-uwslm)
- **Disagreeing with a decision you accepted yourself says so and points to `supersede`.**
  An agent that captured a decision (a capture accepts itself) and then ran `disagree`,
  `emit decision.rejected` or the `disagree_decision` tool was refused with "invariant violated:
  actor ... cannot accept and reject decision ...", which read like a bug. The refusal and its
  exit code (3, and the same HTTP status) are unchanged. The message now reads "<actor> already
  accepted decision <id>, so it cannot also reject it", then names `hivemind supersede --old <id>
  --title ... --rationale ... --chose ...` to record a changed choice, and `hivemind retitle` or
  `hivemind move` to fix only the wording or the project. Accepting a decision the same actor
  rejected reads "already rejected ..., so it cannot also accept it". (hivemind-09j6r)
- **`recall`, `why` and `verify` answer a question that opens with `I'm`, `we're`, `let's` or
  a like contraction exactly as they answer it without.** `I'm`, `I've`, `I'd`, `I'll`, the same
  four for `we`, `you` and `they`, and `let's` (with a straight or a typographic apostrophe)
  counted as words a decision had to hold: `recall "I'm changing how the frontend turns the graph
  API response into edges; what did we settle?"` found nothing although the same question
  without `I'm` finds the decision page's brief, and `why "I'm editing the public demo data"`
  answered as a close match that "lacks i'm" where `why "editing the public demo data"` was a
  full one. They are question words now, like the `I` and `we` inside them; `let's` is read as
  "let us", not as a possessive of `let`, and a bare `let` stays a word to look for. On a copy of
  the crew ledger (534 events, 69 decisions), the checker's 223 questions: 220 answer
  byte for byte as before, no right answer is lost in the recall top three (187 before and
  after), and the three that open with `I'm` change: the frontend question goes from nothing to
  the brief decision, `why` for the forecaster question no longer lacks `i'm`, and the
  new-button question lists the close decisions the same question lists without `I'm`. Not
  fixed: a question whose words the decision spells differently (synonyms) still finds nothing.
  (hivemind-e9340)
- **A decision the classify queue drafted appears in every time window that covers the day it
  was said.** `recent_decisions --since`, `digest --window`, `get_decisions_added_since`,
  `search_decisions --since` / `--until` (CLI, MCP and `GET /v1/decisions/search?since=`),
  windowed `recall` and `review --since` left every `capture:<event>:<index>` draft out,
  although a topic lookup or a search with no window found it: they dated a decision from its
  `decision.proposed` event, and a draft has none. On the city cell, "what did I and my agents
  decide in the last 7 days" listed 20 of the 332 decisions `GET /v1/graph` dates in those
  days, and the `recent_decisions --since 1h` that `docs/KEYLESS_CAPTURE.md` Step 5 tells you to
  run right after classifying came back empty. A decision is now windowed by the time the
  graph dates it, the `decided_at` of `GET /v1/graph`: its proposal's time, or for a draft the
  time of the turn it was said in, which can be days before the classification that recorded
  it. So a conversation from last week sits in last week's window however late it was
  classified, and one the window does not cover stays out even if it was classified inside it.
  `recent_decisions` and `get_decisions_added_since` list the drafts beside the proposals,
  with their real status, topics and actors, and gain `decided_at` on each item (`creation` is
  still the event that recorded it, which for a draft is the classification); a hand-recorded
  decision keeps its place in each window. A draft is added in `get_decisions_added_since`
  when its classification is inside the offset bounds and its `decided_at` inside the
  timestamp bounds. `hivemind query recent_decisions`, `get_decisions_added_since` and `review`,
  and the `recent_decisions` MCP tool, now read the graph as well as the ledger, so a local
  `query` replays the ledger into a graph first, as the other queries do. (hivemind-23xw6)
- **`recall`, `why` and `verify` find a decision from a question that adds everyday words to
  the one rare word the decision holds, when the decision holds a second rare word or spells
  two of the question's words as one.** "How far does the forecast's leftover tail reach?"
  found nothing although the decision titled "... leftover mass draws beyond longest completed
  duration ..." says "tail" in its rationale and `recall "leftover"` finds it first; "how
  does the file store throw away expired entries?" listed only a competitor-check note,
  although the decision titled "FileStore retention deletes ..." is about it. A decision
  below the half bar is now also listed when it holds two of the question's words, one in its
  title or topic keys, that no more than two decisions hold each (and that weigh what the
  existing pair test asks), and when its title or topic keys spell two consecutive words of
  the question together ("file store" is `FileStore`, "data base" is `database`) and few
  decisions do. A rationale that spells them together does not count. "File store" now counts as
  two words of the title, so the decision comes first among the ones below the bar. The story
  the user asks about is also the chapters the Flow view draws (`story` and `chapter` join the
  stand-in words), so "why did the names in the story pictures shrink to almost nothing on a
  phone?" lists the SVG-chapters decision third instead of sixth. Through the debug build of
  this change on a copy of the crew ledger (534 events, 69 decisions), 480 questions: the
  checker's 203 go from 171 to 174 in the recall top three (the leftover and file-store
  questions are first, the story-pictures question third) and `why` lists the right decision
  for 173 instead of 171; 40 fresh questions of the same shape go from 33 to 36; the
  other 237 lose no right answer, and one lists one more decision below an unchanged first;
  the 95 unrelated controls answer exactly as before, "do we need a licence to play music
  in the office?" among them. Not fixed: "what stops someone hard-coding a colour in a
  component?" still finds nothing. The decision about colour holds one word of the five, in its
  rationale, and six other decisions hold one word of the same question each (two of them
  "colour" too, the others "stops", "someone" and "component"), as rare as "colour"; no count
  of words tells them from an unrelated question. `docs/AGENT_FLUENT_QUERYING.md` and the `recall_decisions` tool description state
  the rule. (hivemind-9wk9e)
- **The first `why`, `verify` or `recall` after the server starts answers as fast as every later
  one.** The server built its graph from the whole ledger inside the first request it got, so
  the first question after a roll waited for that: 13.4 s on the city cell, whose ledger holds
  about 115,000 ingest transcripts beside its 1,084 decisions, against the 10 s a `why` should
  answer in. The server now builds the graph of every tenant before it opens its port, so
  `/v1/health` and the container health check pass only once reads are warm, and the startup
  log says `graph cache warmed` with the tenant count and the time it took. A tenant whose
  graph cannot be built is logged and skipped, and its first read builds it as before. The
  price is the same wait moved to startup: on a copy of the cell's decisions padded with
  115,864 synthetic transcripts (SQLite on this host, not the cell's Postgres), a fresh start
  of the previous build answered its first `why` in 19.6 s and 26.8 s (two runs) and the
  new build in 0.42 s and 0.71 s, after warming for 21.8 s and 33.2 s before it listened.
  (hivemind-7jon8)
- **`why`, `verify` and `recall` by description, and a plain `search` with a status or time
  window, answer in a fraction of a second on a ledger of a thousand decisions instead of
  timing out.** On the city cell (1,084 decisions) they took 20-30 s and mostly ended in
  HTTP 408 at the server's 30 s limit, while a lookup by id answered in 1.4 s. Two causes. The
  in-memory graph copied every node and edge on each read, and a search read a decision's
  status with three such reads per decision, so one question cost thousands of full copies; a
  read now looks at the graph where it is, and a search reads every status and every premise's
  standing once. And `why` read every event of the ledger to date one decision, and a
  `since`/`until` window read every event to date the proposals, ingest transcripts and all
  (about 115,000 on the cell); each now reads only the seven decision event types, or only the
  proposals. Nothing about an answer changes: through the real binary on a copy of the cell's
  2,493 non-transcript events, the checker's 17 questions on `why`, `verify` and `recall`
  (51 answers) are identical to the previous build's, and the server answers all 51 with HTTP
  200 (median 0.14 s, the first, which builds the graph, 3.7 s) where the previous build gave
  408 at 30 s. Not measured here: the Postgres read of the full ledger itself, which the copy
  does not carry. A time-window search no longer fails on a malformed event of a type it does
  not read. (hivemind-7jon8)
- **`why`, `verify`, `chain` and `compact_view` list the decisions instead of naming one when
  four or more hold every word of a short question and none of them is told apart by a word
  of its own.** "What order is the decision list in?" answered the decision on the order of
  the listings, and "what does the decision page show first?" answered the one on the first
  five suggestions, in one call and with no mention of the other seven or eight decisions
  that hold the same words. With four or more decisions holding every word, the first is now
  named alone only if its title or topic keys say a word of the question that no other
  decision holding every word says in its own, in any form; otherwise they come back in the
  `ambiguous` shape, the one that is most about the question first, none picked. A question
  that names its subject in a word only one headline carries, and a decision the question
  names outright (id, title or the question it records), keep their one-call answer; with
  three decisions holding every word or fewer nothing changes, and a verb that writes lists
  several full matches as before. Through the real binary on a copy of the crew ledger (534
  events), 891 questions (the checker's 143, the earlier batteries, 414 generated and 40
  new ones of this shape): wrong one-call answers go from 18 to 15, and four right
  one-call answers (the decision asked about) now come back listed first; the 74 unrelated
  controls answer as before. On the 455-event prefix of the same ledger wrong answers go
  from 36 to 32. Not fixed: a leader that says a word of its own in its headline, or a
  question that fewer than four decisions answer in full, is still named when it is not the
  decision asked about. `docs/AGENT_FLUENT_QUERYING.md` and the four tool descriptions
  state the rule. (hivemind-bnces)
- **`recall`, `why` and `verify` list the decision whose title is about the one word the
  question is about, even when the question's other words sit in other decisions'
  rationales.** "What licence is the code under?" returned four decisions that say "code" and
  "under" in long rationales, each lacking "licence", and left out the one titled "No
  commercial-licence line in the README; AGPL-3.0 only ...", although `recall "licence"`
  found it first; "what shelf do we put the product on publicly?" left out "HiveMind's
  public shelf is agent memory ...". A decision below the half bar is now also listed when
  the words its title or topic keys hold weigh at least 0.6 times `ln(decisions + 1)` (a word
  few decisions hold) and two fifths of the weight of all the question's words, with the
  ledger's own counts and no word list. A rare word that sits only in a rationale does not
  name a decision, and a question that adds to the rare word several that nobody holds ("do we
  need a licence to play music in the office?") still finds nothing. A word ending in `ly`
  is also read as the word it comes from (`publicly` is `public`), so a question's adverb
  meets the title that says the adjective. Through the real binary on a copy of the crew
  ledger (534 events), 437 questions: the checker's 143 go from 118 to 120 in the recall top
  three (both new answers are first), and `why` lists both decisions first; the 294 others
  lose no right answer, and two of them list one more decision below an unchanged first;
  the 74 unrelated controls keep their 0 answered. 414 generated questions keep their 361
  right, 27 first-listed and 20 listed answers and their 387 first and 407 top-three
  recalls (four list one more decision below the right one). On the 455-event prefix of the
  ledger, "what shelf do we put the product on publicly?" goes from a ten-decision list to
  naming the public-shelf decision alone, and the licence question lists its decision first.
  Not fixed: a decision that says the question's rare word only in its rationale, and a
  synonym of a word the title says. The `recall_decisions` tool description and
  `docs/AGENT_FLUENT_QUERYING.md` state the rule. (hivemind-t4g9u)
- **An agent's `ground`, `retitle`, `disagree`, `supersede` and `move` from the CLI are
  recorded as the agent, not as the person whose git email is configured.** `emit
  decision.capture` already filed an agent's capture under the agent, but the follow-up
  verbs with no `--actor` fell back to `human:<git user.email>`: a `disagree` an agent ran
  turned a decision `contested` with that person named as the one who rejected it, a
  `ground` read "attributed later by human:...", and a `supersede` was accepted as the person
  and filed in their personal project. That includes the `hivemind ground --id ...
  --rests-on-decision ...` command the cited-not-linked hint prints. With an agent in the
  environment (a tool or session variable, or a Gas City identity) and no `--actor`, every
  CLI write now records `agent:<tool>:<name>`, the same actor the capture derives. A plain
  terminal with no agent still records the person, a typed `--actor` still wins, and a
  `HIVEMIND_ACTOR` that already names an agent is kept. The current-project setting a
  person made with `project use` is still read under the person's key. (hivemind-jglb7)
- **`hivemind --help` names the actor an untyped write is recorded as in the session that
  reads it.** The `--actor` line printed `[default: human:<git user.email>]` in every
  environment, so an agent checking who its `ground`, `disagree` or `supersede` would be
  recorded as was told the person, while the write was recorded as the agent. The line now
  reads "Here that is the agent, agent:<tool>:<name>; a plain terminal records
  human:<git user.email> instead" in an agent session, and names the git user (or
  `HIVEMIND_ACTOR`) at a plain terminal. Help and the recorded actor come from one rule. A
  typed `--actor` is still recorded as typed. (hivemind-iynfm)
- **A rationale that quotes a full decision id is no longer refused as a "bare list item"
  about one time in five.** The capture check for a reference into a chat list (`1a`, `2. a`)
  read a four-digit uuid group of three digits and a letter, such as the `612a` in
  `decision-0d40fa67-612a-4f3b-8c1d-0123456789ab`, as a citation, so quoting an earlier
  decision's full id (which the capture guidance says to link) was refused or accepted
  depending on the uuid, not the text. A full id (`decision-`, `evidence-`, `hypothesis-` or a
  bare uuid) is now skipped whole; `1a`, `2c` and `2. a` beside it are still refused unless
  `quote` and `question` carry the verbatim words. (hivemind-kra19)
- **A decision someone rejected and nobody accepted no longer reads "still holds: yes".**
  `why`, `verify`, `GET /v1/decisions/verify` and `get_decision_outcome` answered `[rejected]`
  with `held_up: true` and no reason, while a decision resting on it read "premise rejected".
  The rejected decision now reads `held_up: false` with a `rejected` reason that names who
  rejected it (`{"kind": "rejected", "by": ["human:carol"]}`; text: "still holds: NO: rejected").
  A contested decision (accepted and rejected) and an accepted one read as before. The
  failure-attribution corpus counts these decisions as not holding and its `signal_breakdown`
  gains `rejected_count`; the bulk `get_decision_quality_candidates` outcome gains `rejected`.
- **A write that runs long is no longer answered "408 request timed out" after it has been
  recorded.** The server cut every request off after 30 seconds, but a write's ledger work does
  not stop when the answer is dropped: it ran on and committed. The client was told the write
  failed when it was recorded, and a client that retried recorded it twice (on the city cell, one
  `capture_decision` retried three times after a 408 left four copies of one decision). Only reads
  (`GET`, `HEAD`, `OPTIONS`) are cut off at 30 seconds now; every `POST` runs to the end and
  answers with what it did, however long that takes, except a read tool called through
  `POST /mcp` (see the next entry). A client should still treat a dropped
  connection on a write as "outcome unknown" and look before it retries. An idempotency key,
  so that a retry is recognised, is a separate design and is not part of this change.
  (hivemind-s9cx)
- **A client that stalls mid-request no longer holds one of the server's request slots, and a
  read tool over `POST /mcp` is cut off at 30 seconds again.** The server read a request body
  before any deadline, so a client that sent its headers and then went quiet kept one of the 200
  concurrent-request slots until it disconnected. A request body that delivers nothing for 30
  seconds is now abandoned with `408 request body timed out`; no handler has run by then, so
  nothing is recorded. The 30 seconds are between bytes, not for the whole body, so a large
  ledger replay batch on a slow link keeps going while it keeps arriving. A client that stalls
  before its headers are complete holds a connection but no slot, and is not covered. Since the
  previous entry a read tool called through `POST /mcp` (`recall_decisions`, `search_decisions`,
  `dump_graph`, ...) had no time limit, and every town agent reaches the cell that way. A read
  tool now answers a tool error (`isError: true`, "request timed out after 30s") at 30 seconds;
  a write tool (`capture_decision`, `supersede_decision`, `classify_queue_submit`, ...) still
  runs to its result. Which tool is which is declared once, in `TOOL_KINDS`; a tool added
  without a kind fails a test. (hivemind-g3vd0)

- **An option label that holds a comma now counts as one option in the text answers.** Since
  `--option` records a label such as `Rename after the comparison, before the first listing`
  whole, `why --summary`, `verify --summary` and `digest --summary` still joined every label
  with `, `, so a three-option decision read as four and "before the first listing" read as an
  option of its own. A label that holds a comma, a semicolon or a double quote is now set apart
  in double quotes (`rejected: "Rename after the comparison, before the first listing", Rename
  now`); every other label reads as before. The same goes for the `Options:` line of
  `summarize_decisions`. The JSON answers are unchanged: each label stays whole and unquoted.
  (hivemind-2gbo)

- **A decision nobody has decided yet no longer lists every option as rejected.** A decision
  captured with its options and no choice (an open question, or an MCP capture whose chosen
  option did not arrive) read as one that rejected everything: `why --summary` printed
  `rejected: Weekly, Daily`, and `verify`, `GET /v1/decisions/verify`, `get_decision_outcome`
  and `why` put every option in `rejected_options` beside `chosen_option: null`. An option is
  rejected only against a recorded choice. With none, the options come back in a new
  `open_options` list, `rejected_options` is empty, and `why --summary` prints
  `options: Daily, Weekly (open — no choice is recorded)`. `open_options` is left out of the
  JSON when a choice is recorded, so a decision with a choice reads exactly as before.
  (hivemind-d6ar)

- **`disagree`, `retitle`, `move`, `supersede` and `ground` no longer write to one of two
  decisions that a description matches in full.** Their help says a description that matches
  more than one decision lists the candidates and writes nothing, but since the ordering by
  where the words sit (hivemind-eral) a writer was answered with whichever full match carried
  more of the question in its title and topic keys. On a copy of today's ledger (534 events),
  `disagree "the packaging decision loose end"` recorded a rejection on the pricing decision
  (event 535) while the decision on the licence line holds every word too, and `retitle` and
  `ground` landed on it the same way; the plugin scripts and MCP `disagree_decision` did the
  same. A rejection, a new title or a new premise cannot be taken back, so a verb that writes
  now lists every decision that holds every word, the one about the question first, and writes
  nothing, unless the description names one outright (its id or title is the description, or it
  records the question as asked) or only one decision holds every word. Through the real
  binary on that copy, the three verbs list both decisions and leave the ledger at 534 events;
  a description one decision alone holds (`commercial licence README`) still writes in one call;
  `why` still answers with the decision about the question. `why`, `verify`, `chain` and
  `compact-view` are unchanged: a wrong pick there shows the wrong decision and writes nothing.
  Unlike before the ordering, two full matches no longer resolve for a writer because one has a
  word in its title and the other has none; `--pick N`, `#N`, `--id` or `--topic` settle it.
  (hivemind-293q)

- **A write over `/mcp` under an agent token, without an `actor_id`, is recorded as the token's
  actor, not as `agent:mcp-http:<session>`.** Claude Code's HTTP transport always sends
  `Mcp-Session-Id`, and the server preferred that id over the token, so a decision captured
  under an agent token (`agent:claude:<role>`) was filed under a fresh id per connection that
  named no one. An agent token's own actor now wins over the session id; an explicit `actor_id`
  argument still wins over both. Under a person's credential (a user token or a WorkOS JWT) a
  write without `actor_id` is still recorded as the MCP session's agent
  (`agent:mcp-http:<session>`), never as the person: with a chosen option it would otherwise
  self-accept as that person. The `actor_id` text on the capture, disagree, supersede, move,
  retitle, acknowledge, ground and request tools says what it defaults to. Decisions already
  recorded under an `agent:mcp-http:<session>` actor stay as they are: the ledger is
  append-only. (hivemind-zbjc)

- **An option label with a comma is no longer cut in two by `--options`, and a piece that
  starts with a space is refused instead of recorded.** `emit decision.capture --options
  "Rename now,Rename after the comparison, before the first listing,Pause"` recorded the
  third label as two options, "Rename after the comparison" and " before the first listing",
  both read as rejected, and the record is append-only, so the only repair was a supersede
  that falsely read as the decider changing their mind. A label that contains a comma now
  goes on its own `--option` flag, which takes one label whole and repeats (`--options` pieces
  first, then each `--option`); `--chose` repeats the label exactly. `--options` still splits
  on every comma, but a piece that is empty or starts or ends with whitespace is refused with
  that hint, because the space after a comma is the mark of a comma inside a label. The same
  two flags work on `emit decision.proposed`, `supersede` and `slack-app enqueue-capture`. MCP
  `capture_decision`, `supersede_decision` and REST `POST /v1/decisions` take options as an
  array and never split on a comma.
- **The capture plugin's `capture.sh` passes `--option` on, so a label with a comma can be
  recorded through it.** The CLI's refusal for a space after a comma tells you to give the label
  to `--option`, but the helper did not know the flag: it forwarded it alone and took its value
  for the capture text, so the capture failed with "a value is required for '--option'" and the
  label could not be recorded at all through the plugin. `capture.sh ... --options "Rename
  now,Pause" --option "Rename after the comparison, before the first listing"` now records the
  label whole, and the capture skill, the `/hivemind-capture:capture` command and the plugin
  README say how. On a `hivemind` that has no `--option` (v0.7.0 and older) the helper refuses
  with exit 2 and writes nothing, telling you to upgrade or reword the label, instead of handing
  you the CLI's "a similar argument exists: `--options`", which would split the label.
- **A draft submitted through the classify queue names the session it came from and who it was
  held with, and `verify` says why nobody has decided it.** The queue's submit path wrote a
  worker's captures as sent, so a draft carried no session initiator and no participants: on the
  city cell 87% of 646 drafts named no one, and the graph held no `INITIATED_BY` or
  `PARTICIPATED_BY` edge at all. The projector now reads both from the received batches a
  classification covers: `INITIATED_BY` the submitter of the first batch it names, and
  `PARTICIPATED_BY` each submitter plus the agent that ran beside a human (`agent:<tool>:hook`;
  an agent token is already the agent), so a replay names them for drafts already in the ledger.
  A classification that covers no received batch (a `hivemind emit` capture) names nobody. The
  write path drops any `participants` and `session_initiator` a submission carries, and the
  built-in classifier no longer sets them. `verify` and the brief (`decided_by.drafted_from`)
  print `drafted in: session <id>, started by <actor>, with <actors>`, and, while nobody has
  accepted a draft, `undecided: a classifier draft is never self-accepted, and the transcript
  names no one who accepted it`, also when it names a chosen option. A decision someone rejected
  with nobody accepting it no longer reads as unreviewed: its review shape is the new `rejected`,
  and `verify` prints `rejected by: <actors>` (`decided_by.rejecter_ids`) instead of "not yet
  decided". An agent-authored decision a human rejected is no longer marked "not yet reviewed by
  a human". (hivemind-6td5)
- **The capture plugin's queue-depth hook counts the queue instead of the reply's keys.**
  `scripts/check-classify-queue.sh` ran `classify-queue list --json | jq length`: against a
  server-backed cell that printed "4 batch(es) pending" for an empty queue (the reply is an
  object with four keys) and for a queue of 37, and on a local ledger it never said more than 20
  (`--limit` defaults to 20). The one-liner in the script's header fired on every stop for the
  same reason. `classify-queue list --json` on a local ledger now prints the same object a
  server-backed cell does, `{batches, pending_total, truncated, budget}`, instead of the bare
  list of batches, and the hook, the plugin README, `commands/classify-queue.md` and
  `docs/KEYLESS_CAPTURE.md` read `pending_total` through `--limit 1`: nothing is said at 0 and the
  true count otherwise, locally and against a served copy. The hook and its one-liner also read
  what the v0.7.0 release prints (a bare list locally, `{batches, budget}` with no `pending_total`
  from a server) and count its batches, so they warn on that binary too. A script that read the
  local list with `jq '.[]'` reads `.batches[]` now. (hivemind-wjvf)
- **A question with "not" or "doesn't" in it is no longer answered by whatever decision has
  "no" or "not" in its title, nor sent past the decision it asks about.** Since the negation
  rule shipped, `why`, `verify` and the write verbs resolved a negated question only to a
  decision whose title held any negation word, about anything: "why does the decision page
  not show when a decision was accepted or superseded?" was answered by a competitor check
  titled "... no change to positioning ...", and "why didn't we keep title slugs for decision
  links?" by the decision that kept them ("... not slugs remembered per browser"). Plain
  questions with a "not" in a side clause ("... and why not the change days?", "... so it
  doesn't cover the node names") lost their one-call answer to a list, the decision asked about
  marked "polarity: question is negated; this decision is not". A decision is now the opposite
  of a negated question only when its own title says every word asked about outright, outside
  any negation ("Adopt Kafka" for "why didn't we adopt Kafka"); a negation in a title reaches
  the rest of its clause. A title that leaves a word out, or denies it, competes like any other
  decision, on how much of the question it carries, and the opposite decision is still listed
  with `polarity_mismatch` and never answered with. (hivemind-z05v)

- **A decision's link is cut at a whole word, and an apostrophe no longer splits a word.** The
  `slug` a proposed decision gets from its title was cut at 60 characters wherever that fell
  (`…-so-none-of-it-is-ever-overwritte`), and `reader's` became `reader-s`. The slug now ends
  at the last whole word that fits in 60 characters (one word longer than that is cut at 60),
  and apostrophes (straight or curly) are dropped: `readers`. The length budget and the room
  for a collision suffix are unchanged. A title that has neither problem slugs as it did. A
  slug is worked out from the title each time the ledger is replayed, so a decision whose
  title was cut mid-word or had an apostrophe gets its new slug when the graph is next
  rebuilt (on a server restart); its old link stops resolving. (hivemind-q9l1)

- **`why` and `verify` no longer answer, in one call, a decision that lacks the word the
  question is about.** When no decision held every word, `why` answered with the one that
  lacked the fewest, and a decision with a long rationale and long evidence lacks few words of
  anything: "why did we choose Postgres for the hosted cell" was answered with the hosted-plan
  decision (it has no "postgres"), "why do we charge per seat" with the pricing decision (no
  "seat"), "why is the demo's data checked into the UI repo instead of read from hivemind" with
  the product-naming decision. A close candidate is now answered with only when its own title
  and topic keys hold at least two of the question's words as words; one word in a title is
  what any decision on the subject has, and the question a decision records (it can be a
  paragraph long) orders it but does not count toward that. Otherwise `why`, `verify`, `chain` and
  `compact-view` list it (`outcome: ambiguous`), with `missing_terms` saying what each lacks,
  and nothing is answered with. The close candidates are also listed in a new order: the one
  whose title, topic keys and recorded question carry more of the question comes first (each
  word weighed by how few decisions hold it, as for a decision that holds every word), and only
  among candidates that are equally about it the one that lacks fewer words. A decision that
  lacks one word fewer but holds the rest only in its rationale no longer leads a decision whose
  title says them. `recall` orders its close matches the same way, so it and `why` name the same
  decision first. A question that matches no decision's headline is ordered as before. MCP tool
  descriptions for `chain`, `why`, `compact_view`, `verify` and `recall_decisions` say so.
  (hivemind-tfde)

- **`why`, `verify` and `recall` put the decision that says the question in other words
  first, and the site's arrow is the app's edge.** "What word do the site's diagrams put on
  the arrow from a new decision to the one it replaced?" listed the decision asked about,
  "Site graphs label the supersedes edge 'replaces'", third, behind a decision on the site's
  refutation chain (its title says "one" and "replacement") and the product-naming decision;
  against the 455 events the checker ran on it was answered with the naming decision. What a
  question is about counted a word only when a title or topic key held the word itself, so
  "graphs" for "diagrams" counted for nothing and a generic word in a title beat a title that
  says the question twice over in other words. A title or topic key that says a word only
  through a stand-in now counts for half of what the word would, weighed by how few decisions
  say it in any of those ways, and `edge` and `arrow` are stand-ins for each other. The arrow
  question is answered with the decision it asks about (it lacks "put" and "new", and says so
  in `close_match`); two more questions of the checker's set, one about Flow on a phone and one
  about a refuted premise on the site, now answer or list the decision they ask about first.
  Through the real binary on a copy of today's ledger, over the checker's 55 questions: right
  one-call answers 30 -> 32, the decision listed first 3 -> 4, wrong one-call answers 6 -> 6,
  `recall` top-1 33 -> 36; on the 455-event copy right 31 -> 33, wrong 4 -> 3, `recall` top-1
  37 -> 39. The 14 hard paraphrases, the 41 hold-out questions, the 34 of hivemind-eral, 32
  fresh ones and 54 unrelated controls (none answered) are unchanged, and over 414 / 366
  generated questions right one-call answers are 363 -> 363 / 317 -> 317 and wrong ones
  5 -> 4 / 6 -> 6.
  (hivemind-ctok)

- **`why` and `verify` are asked the same questions through the CLI, the plugin's scripts,
  HTTP and MCP, and the replies are compared.** The four share one resolver but each builds its
  own reply, and nothing proved they agree. `tests/read_surface_parity.rs` puts the questions
  hivemind-tfde and hivemind-3lko were filed on to a ledger shaped like the one they were asked
  of and requires the same envelope (the decision answered with, the list and its order,
  `close_match`) from all four. The one difference it found: a description that matches
  nothing is HTTP 404 with a `not_found` error body, where the CLI and MCP print a successful
  `not_found` outcome. `docs/DEPLOYMENT.md` said HTTP answered 200 with that outcome; it now
  says what the route does. (hivemind-ctok)

- **`why`, `verify`, `chain` and `compact-view` no longer name a decision alone because it
  leads the decisions that all hold every word by one word.** "Why doesn't the decision page
  show when a decision was accepted or superseded?" was answered with a decision about how a
  contested, superseded decision is noted (it says "superseded" in its title and files
  "decision-page" under a topic key), while the decisions about the status history, which the
  question is about, hold every word as well and were not offered. The negation was not the
  cause: the question without it was answered the same way. Among decisions that hold every
  word, the one whose title and topic keys carry the most of the question leads, and a lead of
  one word was enough to be named alone, with two of four words in its headline and the rest in
  a long rationale. When other decisions hold every word, the leader is now named alone only if
  its own title and topic keys hold more than half of the question's words; otherwise the full
  matches come back as an `outcome: ambiguous` list, the most about the question first and
  nothing in `missing_terms`. The only decision that holds every word, and a decision the
  question names outright (its id or title is the question, or it records the question as
  asked), are named as before. A verb that writes (`disagree`, `supersede`, ...) is held to the
  same rule, so it asks for `--pick` or `--id` instead of writing to the leader. Through the
  real binary on a copy of today's ledger (534 events) and its 455-event prefix: the
  "not", "doesn't" and plain forms of that question list seven decisions with the two about
  the status history among them (#2 and #4); the checker's 55 questions go from 6 wrong
  one-call answers to 5 and the 34 of hivemind-eral from 3 to 2; over 414 / 366 generated
  questions wrong one-call answers go 4 -> 3 / 6 -> 5; right one-call answers (32, and 363 /
  317 generated), the 14 hard paraphrases, the 41 hold-out questions, 32 fresh ones and 54
  unrelated controls (none answered) are unchanged. Not fixed: a 3-word paraphrase of that
  question ("when was a decision accepted or superseded on the decision page") still names the
  same decision, since two of its three words are in that decision's headline. MCP tool
  descriptions for `chain`, `why`, `compact_view` and `verify` say so. (hivemind-5ctc)

- **`why`, `verify`, `chain` and `compact-view` no longer name a decision alone for a short
  question that the decision only holds in its rationale.** Two shapes of the same mistake were
  left by the fix above. (1) "When was a decision accepted or superseded on the decision page?"
  and "why does the decision page not show when a decision was superseded?" were still answered
  with the decision about how a contested, superseded decision is noted: it says "superseded" in
  its title and files "decision-page" under a topic key, two of the three words, which is more
  than half, while the six to ten other decisions that hold all three words, the status
  history among them, were not offered. When two or more of the other decisions that hold every
  word say one of the words in their own title or topic keys, a leader of a three-word question
  is now named alone only if all three words are in its title and topic keys; otherwise the full
  matches come back as an `outcome: ambiguous` list, as before for longer questions. (2) "If I
  rename a decision, will old links to it break?" and "does every acceptance or rejection get its
  own line in a decision's history?" were answered with the one decision that holds every word,
  the website rename that says "rename" in its title and the rest in its rationale, and a status
  history decision that says "history" in its title and the other six words in its rationale,
  while the decision the question is about lacked a word or two and was never offered. A
  decision that alone holds every word and says fewer than two of the question's words in its
  title and topic keys (in any form; all of them in a question of one or two words) is now
  listed first, with the decisions that lack some of the words after it, each naming what it
  lacks (`outcome: ambiguous`), when such decisions exist. With none to list it among, it is
  answered with as before, and so is a decision the question names outright. A verb that writes
  keeps its rule for the only decision that holds every word. Through the real binary on a copy
  of today's ledger (534 events): the four questions above list the decision they are about among
  the first three (the status history #3, #2 and #2, the decision links #3), and the
  not / doesn't / plain forms of the question of hivemind-5ctc still list; the checker's 55 questions go
  from 5 wrong one-call answers to 4 with the 32 right ones unchanged; the 34 of hivemind-eral,
  14 hard, 41 hold-out, 32 fresh and 54 unrelated controls (none answered) are unchanged; of 20
  fresh paraphrases of the same questions wrong one-call answers go 6 -> 3, and two right
  one-call answers ("will a decision's link change if I retitle it?" and its slug sibling) are
  now listed first instead of named; over 414 generated questions wrong one-call answers go
  3 -> 1 and right ones 363 -> 362. Not fixed: a close candidate that leads on two generic
  words of its title ("after a disputed decision is replaced, do we still show who pushed
  back?") still names a decision about something else. MCP tool descriptions for `chain`, `why`,
  `compact_view` and `verify` say so. (hivemind-jis8)

- **`why` and `verify` no longer name a promoted close candidate alone on two generic words of its
  title, and a list puts the decisions that hold every word where they are seen.** "Does the
  decision page show supersession dates?" was answered with the contested-then-superseded note
  ("supersession" and "page" in its title, "dates" missing) as a close match, while the status
  history decision the question is about, one of four that hold all four words, sat ninth in the
  list behind a dozen close candidates promoted over a weak full match. A close candidate that
  is promoted over the decisions holding every word is now named alone only when its title and
  topic keys hold three of the question's words (all of them, in a shorter question), the bar
  of a full match that leads with rivals; with fewer, the call lists it and the full matches
  (`outcome: ambiguous`). In a question of three words two stay enough while no more than one
  other decision holds every word and says one of the words in its headline, as for a full
  match since hivemind-jis8. A list shows at most two promoted close candidates before its
  first full match. Only the order of a list and this one gate change; nothing in the write
  path, in recall or in search. Through the real
  binary on a copy of today's ledger (534 events), 406 questions: the checker's 123 keep their
  54 right one-call answers and go from 9 wrong one-call
  answers to 8 ("does the decision page show supersession dates?" lists 6d2c7a02 third);
  the 34 of hivemind-eral, 14 hard, 41 hold-out, 32 fresh, 55 published and 30 blind
  questions lose no right answer; of 15 unrelated controls the answered ones go from 6 to 5
  ("will renaming the repo break CI?" is listed). On a copy of the live cell's 2302 events,
  "does every acceptance or rejection get its own line in a decision's history?" and "does the
  decision page show supersession dates?" now list the status history decision second and
  third instead of naming an unrelated close match. Not fixed: a close candidate that leads
  on two generic words of its title with no promotion over anything ("after a disputed decision
  is replaced, do we still show who pushed back?", "does the command-line binary get a new name
  along with the new company?") and one promoted on a hair of weight through single-letter terms
  ("why could a decision page show its options as A, C, B, ...") still name a decision about
  something else: no feature of the candidate separates them from the right close answers
  ("is anyone working on swapping the product name right now?"), and every gate that lists them
  lists 13 to 16 of those too. Dropping single-letter terms and a promotion margin were
  measured and lose right answers. MCP tool descriptions for `chain`, `why`, `compact_view`
  and `verify` say so. (hivemind-vecg4)

- **A decision captured from a session that was classified days later says when it was said,
  not when the classifier ran.** A decision the classifier captured without naming a source
  turn was dated at the classification's own time, so `why` and `verify` showed a session said
  on 27 September and classified on 1 October (the weekly usage limit had held the queue) as
  decided four days late. When the batch the capture came from carries turn times, such a
  decision now reads at the newest turn time of that batch (across all the batches a grouped
  classification covers), the same value the restatement same-moment rule uses. A capture that
  names its turn keeps that turn's own time, and a batch with no turn times at all (everything
  shipped before turns carried a time) keeps the classification's time, as before. Nothing is
  rewritten: the time is read from the received batch when the graph is built, so decisions
  already recorded read correctly after the upgrade, and a rebuilt or extended graph gives the
  same answer. (hivemind-ohik)

- **`why`, `verify` and `recall` no longer answer a different decision, with no warning,
  just because its long rationale happens to hold every word of the question.** A few
  decisions have rationales and evidence of two thousand characters or more, and between them
  they hold nearly every common word, so a plain question matched one of them in full ("is
  the product still called HiveMind" was answered with a decision on a triage gate), and a
  full match dropped every close candidate: the decision the question is about, whose title
  and topic keys carry it but which lacks one word, was not even offered. Where the words sit
  is now compared before whether every word is somewhere. A decision's title, topic keys and
  recorded question count for the question's words they hold, each weighed by how few
  decisions hold it, and between equal weights the one whose own title holds more is the one
  about it. A close candidate that carries more of the question than every decision that
  holds all of its words is listed first, and `why`, `verify` and the other verbs that only
  read answer with it, saying what it lacks in `close_match`; a verb that writes gets it at
  the head of the `Ambiguous` list and never has it picked. Among the decisions that hold
  every word, the one that carries more of them comes first (two that each had a word in
  their title used to be equals). A negated question ranks a close candidate that says what
  was asked ahead of an opposite-polarity decision that merely holds every word. `recall`
  orders its list the same way, so it and `why` name the same decision first. The decision
  whose id or title is the question, or that records it as asked, is still first, and a
  question with nothing in any title or topic is ordered as before. The `close match:` line
  of `--summary` output no longer claims that no decision has every word, since one may.
  (hivemind-eral)

- **A rationale holding a figure like `28k`, `167h`, `3d` or `10x` is no longer refused as a
  "bare list item".** The capture check for a reference into a chat list (`1a`, `2. a`) also
  matched any number followed by one freestanding letter, so an ordinary quantity was refused:
  in a YC-Bench run, 28 of an agent's 85 captures failed this way (27 for a figure: 21 hour
  counts such as `167h`, 5 amounts such as `28k` or `78M`, one `4x`), and the agent rewrote the
  decision as vaguer prose without the numbers. A number followed by `k`, `m`, `s`, `h`, `d`,
  `w`, `y` or `x` (either case) is now read as a figure, including `$4.7k`. `1a`, `1b`, `2c` and
  `2. a` are still refused unless `quote` and `question` carry the verbatim words, and the
  minimum length and word count are unchanged. Over MCP, `capture_decision` and
  `supersede_decision` also take `topic_keys` as one string for one topic (the array form still
  works); a string that is an array written out as text, such as `["a", "b"]`, is refused with a
  message that says to pass the array itself, rather than filed under one mashed topic.
  (hivemind-ukd8)
- **Asking the exact question a decision answers now finds it.** A decision records the question
  it answers (`--question`, `capture --answers`, `ground --answers`, `request_decision`, and every
  AskUserQuestion answer the ask hooks write), but `why`, `recall` and `search` never read it:
  asking "Which typeface do printed receipts use?" said "no decision matches that description",
  and `recall` listed the target as a close match missing "typeface" and "use". The decisions the
  ask hooks write were hit hardest, because their title is "<header>: <choice>" and their
  rationale is a fixed sentence, so the question is the only text a person would ask with. The
  question is now a searchable field, `decision.question`, ranked with the rationale, so `why`,
  `verify`, `recall` and `search --q "<words only in the question>"` find the decision over the
  CLI, MCP and HTTP. A description that is the recorded question itself, in any case and with or
  without its closing "?", is an exact match (rank 0, like an exact title); when several decisions
  answer one question, `why` lists them all instead of picking one. A question linked after the
  capture (`ground --answers`) is searched the same way. (hivemind-pyy4)
- **An importer whose proposal is refused no longer leaves an ask, evidence or hypotheses
  behind.** The Slack thread import wrote the thread's ask (`question.asked`) and its evidence
  before it proposed the decision, and the document import wrote a block's evidence and
  hypotheses first, so a proposal refused after that (an unreadable rationale, an overlong option
  label, a title past 120 characters, no topic key, a decider that is no actor) left them in
  the ledger with no decision: an ask in the waiting requests that no answer would ever land
  on, and each importer retry added another. Both importers now run every refusal
  `propose_decision` can raise before their first write, so a refused thread or block writes
  nothing and retrying the same input adds nothing. Live Slack app captures still write no ask.
  (hivemind-poum)
- **An answer to an agent's question in your own words, or as a note only, is no longer recorded
  with the option you chose listed as turned down.** The `AskUserQuestion` hook filed every
  answer that was not a click on an offered option under a made-up choice, "Other (own words)",
  with every offered option rejected beside it, so "Five times, because the jobs are cheap" was
  recorded as turning down "Five times". A note-only answer (Claude Code's `n to add notes`, then
  Enter) also had Claude Code's placeholder `(notes only)` quoted as your words. Now your words
  (or your note, when you picked nothing) choose an offered option when they lead with its name
  and name no other offered option: `Retries: Five times`, the other option turned down, your
  words quoted (a note is the rationale, word for word). Words that name no single option, such
  as "Redis or In-process" or "Not Redis, the other one", are the answer: the chosen option is
  what you said and no offered option is listed beside it, so none reads as turned down. A
  multi-select with words of your own records the options you ticked as the choice and quotes
  the words (`Features: Search + Export`). The placeholder is never quoted, and a notes-only
  answer with no note records nothing. (hivemind-2kzf)
- **`GET /v1/classify-queue` no longer takes ~25 seconds whatever the limit, and a worker can
  choose a session without fetching the queue.** On the city cell every request read the whole
  ledger (about 90,000 received batches, each with its turn text) twice, once for the list and
  once for the daily budget, and only then dropped the classified ones, so even `limit=200`
  cost ~25 s and a larger limit ran into the 30 s request timeout (HTTP 408): nothing new was
  classified. The list and the budget now read the classification events without their
  captures and each received batch as an id, a time, an actor and two strings (its batch id and
  session id), in one query per 10,000 events, and read the turn text of only the batches the
  reply carries, by id. The reply gains `pending_total` (how many batches match, whatever the
  limit) and `truncated`, over HTTP and MCP. The Postgres ledger filters by event type and
  extracts those fields in the query, SQLite filters in the query, and both gain a
  `(tenant_id, event_type, event_id)` index, built the first time the new server opens the
  ledger (the Postgres `events_tenant_type_idx` it supersedes is dropped). (hivemind-t15t)
- **`hivemind quality-scan` files each finding once, however often it runs.** It filed a Linear
  ticket for every finding above the threshold on every run, so a scheduled run against decisions
  that had not changed filed the same tickets again each time. It now reads `get_suggestions`
  (the findings nobody has acknowledged) and acknowledges each finding as soon as its ticket
  exists (`acknowledge_suggestion`, action `acted`, channel `linear`, under `--actor`), so a
  repeated run files nothing and a finding whose basis changed is filed as a new ticket. A run
  stops at the first ticket Linear refuses and the next run files the rest. Each filed finding
  now writes a `suggestion.surfaced` and a `notification.acknowledged` to the ledger, so
  **upgrade the server before running it against a shared ledger** (see `acknowledge_suggestion`
  above). The "nothing to file" line now reads `quality-scan: no unacknowledged finding —
  nothing to file`. The context plugin's skill says how an agent routes suggestions to a tracker,
  a `CLAUDE.md` rule or a cron of its own. (hivemind-m306.4.3)
- **The capture plugin can answer a question that was asked.** `capture.sh --answers <request
  id>` (a request `hivemind ask` wrote, listed by `query get_waiting_requests`) was refused with
  "a value is required for '--answers'" and wrote nothing: the script did not know the flag takes
  a value, so it sent the flag alone and put the request id into the capture text. It now passes
  the request id through, so the decision is linked to the request, the request leaves the waiting
  list and `why` shows `asked_at` and `answered_at`, exactly as `emit decision.capture --answers`
  does. The capture skill and `/hivemind-capture:capture` now say how to answer an asked
  question. A CLI older than the flag (the 0.7.0 release) refuses `--answers` with its own
  unexpected-argument error. (hivemind-ig9i)
- **`recall` finds a decision when the question uses another form or a synonym of its
  words.** "why doesn't the decision page show when a decision was accepted or superseded?"
  left out the decision whose text says "acceptance, rejection or supersession", because
  `superseded` and `supersession` shared no stem; "how does the interface refer to an agent
  that made a decision?" never found "The UI names every agent 'an agent'". `recall` (and
  `why`, `verify`, `chain` and `compact-view`, which name what `recall` names) now also match
  the noun or verb of a word (`superseded` ~ `supersession`, `accepted` ~ `acceptance`,
  `decide` ~ `decision`, `refuted` ~ `refutation`) and a short fixed list of stand-in words
  from the product's own vocabulary (`interface` ~ `UI`, `assumption` ~ `hypothesis`, `picture`
  ~ `graph`, `replace` ~ `supersede`, `site` ~ `website`, `link` ~ `url`, ...). Among decisions
  that match equally well, the one that has the word asked for comes before one that only has a
  stand-in. A possessive ("the website's picture") is the word it belongs to, and "made a
  decision" / "make the decision" is question framing, like "pick" (alone, "make" stays a
  word). The verbs that write keep matching a word and its inflections only, so a synonym never
  picks what a write lands on. `search` is still literal. (hivemind-md0y)
- **One rule for every negation in a question.** "not", "no", "never", "without", "cannot" and
  every contraction ending in "n't" (doesn't, don't, isn't, can't, …) are never missing words:
  `recall "why does the decision page not show when a decision was accepted or superseded?"`
  no longer lists every candidate as "missing: not" and leads with the decision asked about.
  A negated question also never resolves to the opposite decision: `why`, `verify` and the
  write verbs resolve it only to a decision whose own title is negated, so "don't adopt Kafka",
  "didn't we adopt Kafka" and "do not adopt Kafka" no longer answer with "Adopt Kafka". It is
  listed as a close candidate with `polarity_mismatch: true` (`polarity: question is negated;
  this decision is not` in `--summary` output) instead of `missing: not`. `recall` ranks as if
  the negation were absent and uses polarity only to break ties. (hivemind-g889)
- **A decision that several sessions restate is one decision to `recall` and `why`, not
  three records that make `why` ambiguous.** The classifier now compares each decision it
  extracts with the closest recorded decisions and, when it states one again, records it with
  `restates_id`; the capture projects a `SAME_AS` link to the decision it restates. `recall`
  and the description resolvers behind `why`, `verify`, `chain` and `compact-view` show linked
  records as one: the earliest record that matches the words, with the others as
  `also_recorded_as` (`{decision_id, title}`; an `also recorded as:` line in `--summary`).
  Records with no link between them stay separate, so a description matching two is still
  ambiguous. A restating capture classified from turns with the same newest source time as
  the decision it restates (a transcript sent twice) is not recorded a second time; the
  reply's `restated` lists each restating capture as `linked` or `deduplicated`. The capture
  hook now sends each turn's own time (`ts`) so this can tell a re-sent transcript from a
  decision made again. For decisions recorded before this, `hivemind restatements propose`
  lists the links to approve (title-word overlap, with the shared words) and
  `restatements apply [--all | --link LATER=EARLIER]` records them; nothing is deleted or
  rewritten. `restates_id` is a new field of `ingest.batch_classified` captures: upgrade the
  server before a client that sends it. (hivemind-83cj)
- **One malformed model-assessment row no longer makes every read of the ledger fail.** A
  `decision.scored` event of schema version 2 whose payload cannot be read (a hand-injected
  row, a buggy or old producer, a partial import) used to abort the projection, so `query
  recall`, `why`, `verify`, the CLI summary, the Markdown export, the MCP stdio server and
  `/mcp` all refused to answer. Readers now skip an assessment (and a `decision.scored` or
  `decision.metadata_derived` row of any shape) that fails validation: the decision it names
  shows what its own events say (the floors of `score_decision`, no `model_assessment`), and
  a readable assessment before or after it still lands. The skip is never silent: `query`,
  `digest`, `export` and `quality-scan` on the CLI, and every read tool on the MCP stdio
  server and `/mcp`, carry a one-line notice naming the rows, for example `1 assessment row
  could not be read and was skipped (ledger event 57: payload does not match event type
  DecisionScored: ...); answers leave it out`: a `notice` key beside `data` in `--json` and in
  the MCP result, a final `notice:` line in `--summary` output. The server logs one warning per
  replay with the count and the ledger offsets. An unreadable assessment does not count as
  assessed either, so the background scorer still assesses that decision. The write path is
  unchanged: it still refuses a malformed assessment before anything is appended, and a
  malformed event of any other kind
  (the ones the graph is built from) still fails the read. There is no `doctor` command; the
  notice is where the bad rows are listed. (hivemind-qo11.10)
- **The Claude Code and Codex capture plugins work with the latest release binary.** The
  marketplace serves the plugins from `master` while people install the release binary, so a
  plugin change that passed a flag only `master` has (`--project-from-context`) broke every
  stranger's install: the MCP server closed at start and `capture.sh` exited 2 with nothing
  written. The MCP server config, `capture.sh`, the AskUserQuestion hooks and the
  hivemind-context `supersede` verb now pass such a flag only when the installed CLI lists it
  in its `--help` (an older CLI files the decision under the personal project), the hook
  writes an answer without its question for a release that takes a question only beside a
  quote, and hivemind-context `ground` says the release has no such command. CI runs the
  plugins against the latest release binary (`scripts/check_plugin_against_release.py`), so a
  plugin change that needs a newer CLI fails there. Each surface also names its own tool: the
  Codex bundle starts its own MCP config (`.codex-plugin/mcp.json`, `--agent-tool codex`)
  instead of the Claude plugin's, which filed Codex MCP captures as `agent:claude`, and the
  shared skill's direct CLI form names `claude` or `codex` instead of always `codex`. The
  plugin manifests are 0.1.1 so installed copies update. (hivemind-cxqd)
- **A decision the classifier captured from a transcript no longer reads with no time.** Every
  classified decision now carries the time of its batch as `occurred_at`, so `why`, `verify`,
  `GET /v1/decisions/why` and `decided_at` on `GET /v1/graph` say when. Who decided is still
  only what the classifier named as acceptor or rejecter: a capture that names none stays
  `proposed`, even when a human is credited as the one who proposed it. The change is in how a
  classified batch projects, so a ledger already holding captures reads right after the graph
  is rebuilt from it; no event is rewritten. (hivemind-s0ra)
- **The classifier names a decision's acceptor only when the text shows that actor deciding.**
  Both classify workers, the server-side classifier and the `classify-queue` command, now
  separate who proposed, made or reported a call (`actor_id`) from who decided it
  (`accepted_by`, `rejected_by`). A person who states the choice, or an actor the text shows
  approving it, is listed; a proposer, a reporter, someone relaying a call without its decider,
  an agent narrating or carrying out its own choice, and a speaker the text does not name are
  not. A decision with no acceptor reads `proposed`. The rule applies to classifications made
  from now on; captures already on the ledger are unchanged. (hivemind-u70b)
- **`why`, `verify`, `chain` and `compact-view` answer a question `recall` answers, in one
  call.** `recall` returned a decision for "how do we keep links to a decision page stable
  across browsers?" while `why` said "no decision matches that description", and `verify` on
  "is the product still called Upheld" stopped at a "close" list of one that needed a second
  call with `--pick`. These verbs only read, so they now match at the bar `recall` uses (at
  least half of the words, not more than half), and when the close candidates that are left
  are not tied, the closest one is the answer, with a `close match:` line ahead of
  `--summary` output and `close_match: {decision_id, title, missing_terms}` beside `data` in
  `--json`, the HTTP `why`/`verify` routes and the MCP `get_decision_neighborhood`,
  `get_decision_outcome`, `get_supersession_chain` and `get_compact_view`. The closest is the
  one that lacks the fewest words; between candidates that lack the same number, it is the
  one whose title or topic keys carry more of the words it did match, so the decision named
  "Name the product Upheld" beats a decision that only says "product" and "called" somewhere
  in a long rationale for "is the product still called Upheld". `recall` orders its close
  matches the same way, so it and `why` name the same decision first. Candidates equally
  close on both counts are still listed, and a decision that shares only one word with the
  question is listed, never answered with. Verbs that write
  (`disagree`, `supersede`, `move`, `retitle`, `ground`, and a grounding premise named by
  description) are unchanged: more than half of the words, at least two, and a close
  candidate is listed, never picked. (hivemind-3lko)

- **An attention finding names every decision, premise, bet, assumption and evidence item it
  is about in words, not by a bare id.** `scan_decision_quality`, `get_suggestions` and
  `hivemind quality-scan` named findings only by id: `reason` read "the decision it follows
  from, decision-9455…, was superseded by decision-72e8…", and a finding carried no title at
  all (`finding_id`, `decision_id`, `kind`, `basis_at`, `node_ids`, `reason` were the whole
  shape), so a reader had to look each id up before knowing what the finding was about. Every
  finding now carries `decision_title` (the flagged decision's own title, falling back to its
  id when the record has none), and `reason` quotes the title, statement or content of every
  other node it names — a prior decision by its title, a bet or assumption by its statement, an
  evidence item by its content, clipped to 200 characters — instead of the raw id: "the decision
  it follows from, 'Use the old queue', was superseded by 'Use the new queue' on 2026-08-01".
  The ids (`decision_id`, `node_ids`) stay in the finding as handles for a follow-up call, never
  as the only way to say what the finding is about. The stdio server, the HTTP endpoint and the
  CLI carry the change identically. A `quality-scan` ticket body now opens with the decision's
  title, not only its id, so it reads without looking anything up; `linear::format_issue_title`
  drops its now-redundant `decision_title: Option<&str>` argument and reads the finding's own.
  Small fix alongside: `evidence_not_rechecked`'s reason read "more than 1 days ago" for a
  one-day window; it now reads "more than 1 day ago". (hivemind-uag0)
- **`recall` answers a plain question in the asker's own words.** It said "No decisions found"
  to questions like "how does the decision page get the brief it shows?" because every word
  that was not a question word had to appear in a decision exactly as written (`shows`, not
  `show`), while `why` found the same decision. `recall` now matches like `why` (a word also
  matches its inflections) and keeps a decision that matches at least half of the words, after
  the ones that match them all, fewest missing words first. Each such close match carries
  `missing_terms` (`ranked.items[].missing_terms`; `missing=` in `--summary`; a `Close matches`
  line in the digest) so a partial match never reads as a full one. "What did we decide about
  sign-in and pricing" now finds the sign-in decision and the pricing decision. A question
  that no decision shares half its words with is still an empty answer, and `search` still
  needs every word. The stdio server, the HTTP endpoint and the CLI share the change.
  (hivemind-j1q3)
- **A replacement of a moved decision is filed where the decision was moved, not in the
  replacer's personal project.** `hivemind supersede`, `supersede_decision`, the REST supersede
  and the `review` supersede inherit the old decision's project when none is named, but read it
  from the decision's first proposal and so ignored a later `hivemind move`. A decision moved
  to a shared project (say `ui`) was replaced into the replacer's personal project, against
  what `--help` and the plugin README say. The replacement is now filed where the old decision
  is now, and the same holds when `--project-from-context` finds nothing. It records
  `project_source` `inherited`, a new value: the replacement took its project from the decision
  it replaces, so it is neither `stated` nor a move. (A replacement used to copy the old
  decision's own source, `rig` or `folder_marker`, which said how the old decision was filed,
  not the replacement.) A caller cannot claim `inherited` next to a `--project`, as with `moved`
  and `personal_fallback`. A decision that sits in a personal project is replaced into the
  replacer's own personal project, since a personal address belongs to one actor and cannot be
  stated or moved into by anyone else; `--help` and the plugin README now say so. Replacements
  already on the ledger are not moved: `hivemind move` fixes one. (hivemind-9lzi)
- **`why`, `verify`, the recall digest and the export show options as words, not slugs and
  letter codes.** An option recorded as `name-a-upheld` read `chose: name-a-upheld` and
  `rejected: name-b-standing, name-c-decisis`, with the answer letter inside the label, and
  options from before labels existed read as the raw id or the id's slug. Options are now shown
  as `Upheld`, `Standing` and `Decisis`: a label that is a slug (lowercase words joined by `-` or
  `_`) is turned into words, acronyms keep their capitals (`direct-cli` reads `Direct CLI`; MCP,
  CLI, API, ADR, UI, URL, HTTP, JSON, SQL and CI), and when every option of a decision is lettered
  `a`, `b`, `c`... the shared stem and the letter are dropped. A label that is already words
  (`Direct CLI`, `sqlite`) is shown as recorded, and an option whose only record is an id with no
  words in it stays as that id. The words are a reading of the record, and the record stays in
  view: the Option node keeps the recorded text as `recorded_label`, and where it reads
  differently `why` prints `chose: Upheld (recorded as: name-a-upheld)`, `why --json` and the
  brief carry `recorded_as` on each option, the decision-log export writes `recorded as:` beside
  the option, and `GET /v1/graph` Option nodes carry `recorded_as` for a page to show. The ledger
  is unchanged: the words are derived each time it is replayed, so existing ledgers read this way
  with no migration step. The capture skill, its slash commands, the README, the CLI `--options`
  help and the MCP `capture_decision` schema now teach short human labels (`--options "Direct
  CLI,MCP server" --chose "Direct CLI"`) instead of `--options direct-cli,mcp`, and `hivemind
  quickstart` records `Local ledger` and `Spreadsheet`. New captures that still use slugs are not
  refused; they read as words. (hivemind-hk5z)
- **A capture refused for its project leaves nothing behind.** `decision.capture` and
  `capture_decision` checked the stated project (registered, no reserved `personal:` prefix, a
  source a caller may claim) only after they had recorded the evidence, assumptions and bet named
  on the same call, so a refusal left those as unattached nodes and every retry added more. The
  project is now checked before the first write, and a refused capture appends nothing. Events
  already written that way stay in the ledger; they are unattached nodes, not part of any
  decision. (hivemind-s15q.20)
- **A supersede that says which option was chosen no longer leaves the replacement "not yet
  decided".** `hivemind supersede --chose <option>`, `supersede_decision` with
  `chosen_option_label`, the REST supersede and the interactive `review` supersede recorded the
  replacement as a proposal nobody had decided, so the only live answer on the question read
  `proposed`, and every decision that followed from the old one was told it "was superseded by"
  something undecided. A chosen option now means the replacement was already decided, as it
  does for `emit decision.capture`: it is accepted from the acting actor right after the
  supersession is recorded, and `why` shows who decided. `--still-proposed` (`still_proposed`
  on MCP and REST) keeps a genuine open recommendation at `proposed`. A supersede that names no
  chosen option decides nothing and still leaves the replacement `proposed`. Supersedes already
  on the ledger are not rewritten: no acceptance is invented for a person after the fact.
  (hivemind-k7o9)
- **Six tools that `POST /mcp` listed now run there.** `recent_decisions`,
  `decision_quality_candidates`, `get_decision_context`, `decision_context_candidates`,
  `scan_misfiled_decisions` and `analyze_failure_modes` were in the HTTP endpoint's `tools/list`
  but answered a call with an `unknown tool` error; only the stdio server ran them. They now
  run through the same core as the stdio server, so a call takes the same arguments, is refused
  with the same messages and returns the same response on both. Nothing changes on stdio.
  (hivemind-imp1.1)
- **"Why did we pick / choose / go with X?" answers in one step.** The verbs people ask about a
  decision with (pick, choose, decide, "go with", "settle on", "opt for", with their past and
  `-s` forms) and the adverbs they put in a why-question (still, again, ever, even, really,
  actually, now, anymore, currently) are now question words. Asking `why did we pick shadcn for
  the design system` or `why is the courtroom demo still on the site` resolves to the decision
  and its rationale instead of listing it as a close candidate that lacks "pick" or "still".
  The list is fixed and applies to the question only, so a decision titled "Pick the cheapest
  vendor" still matches on "pick", and "go" alone stays a search term. `recall` drops the same
  words. (hivemind-36vt)
- **`/hivemind-capture:query-decisions "some words"` no longer fails on the quoted form its own
  argument-hint documents.** The command wrapped what you typed in a second pair of quotes, so
  `"current-project setting"` reached the CLI as two words (`error: unexpected argument
  'setting' found`) and any flag typed after the query was swallowed into it. It now passes
  your arguments through as typed and joins the leading free-text words into one query, so
  the quoted and unquoted forms both work. (hivemind-f4ng)
- **`recall`, `why`, `verify`, `disagree` and `supersede` in the hivemind-context plugin print
  their usage when called with no arguments**, instead of dying on macOS's bash 3.2 with an
  unbound-variable error. (hivemind-f4ng)
- **`--graph-backend kuzu` works again.** It is only there in a binary built from source with
  `--features graph-kuzu`, and building its graph from a ledger stopped at the first event: the
  Kuzu graph had no column for an actor's kind and no table for the `SAME_AS`, `PARTICIPATED_BY`
  and `INITIATED_BY` links, nor a column for an evidence capture's topic keys or a decision's
  score, and it could not store a blocker resolved with a reason but no event id. A
  `graph.kuzu` left by an earlier build needs nothing from you: every run rebuilds it from the
  ledger. (hivemind-cbab)
- **`scripts/cell-update.sh` keeps the update.** It restarted the cell on the sha-tagged image
  but named that image only for its own `docker compose up`, so a systemd timer or cron job that
  keeps the cell up with `docker compose up -d` recreated the container on `:latest` at its next
  run, and the update quietly undid itself. The script now writes `HIVEMIND_IMAGE=hivemind:<sha>`
  into the compose project's `.env` before the restart and keeps it once the new container is
  healthy and `/v1/version` reports the sha; a failed update puts the file back as it was, so a
  reconciler returns the cell to the previous image. An existing `.env` keeps its mode and every
  other line, and a new one is created owner-only. `HIVEMIND_COMPOSE_FILE` now takes several
  compose files, colon-separated, so a cell with an override file is restarted with all of them,
  and `HIVEMIND_ENV_FILE` names an env file other than the `.env` next to the first compose
  file. Documented in `docs/SELF_HOSTING.md`. (hivemind-2mgj)
- **A Slack workspace installed by hand without its tenant is refused up front, and a capture
  that cannot be written shows in the server log.** `docs/SLACK_APP.md` says a hand
  `slack-app install` needs `hivemind tenant create <team_id>` first and that the routes 404
  without it, but only the slash command did: the message shortcut opened its modal, the
  submission, the reaction (Slack was asked for the message) and a marker-bearing mention were
  all acknowledged and queued, and the background drain then failed every one of them every 15
  seconds with nothing but a `last_error` in `slack-app/queue.jsonl`. The shortcut, the modal
  submission, a marker-bearing mention or message and a reaction on the capture emoji now answer
  `404` before anything is queued or Slack is called, with a `WARN` naming the workspace and the
  `hivemind tenant create` that fixes it; an event that is no capture is still acknowledged.
  Every drain pass that still fails a capture now logs a `WARN` with its workspace, queue item,
  attempt count and reason (it was an `info` line with counts only, below the default level).
  `slack-app drain` and `slack-app command` now use the tenant named by the capture's or the
  command's team id, as the server does, instead of the global `--tenant` (default `local`), so
  the same queue no longer lands in `local` from the CLI and in the team's tenant from the
  server; a team whose tenant was never registered fails with the fix in its `last_error`.
  Register the tenant (`hivemind tenant create <team_id>`) before draining a hand install from
  the CLI; the drain report's failed items gain a `team_id`. (hivemind-62jy)

## v0.7.0 — 2026-09-25 — M6: Fluent verbs and grounded capture

You can now consult HiveMind without holding a decision id. Describe a decision — or just
ask "why did we…?" or "did that hold up?" — and `why`, `verify`, `disagree` and `supersede`
find it, refusing to guess when the description is ambiguous; `recall` and `situational`
answer from a question or from the files you are touching. `why` now answers *why*: rationale,
the options chosen and rejected, who decided and whether it still holds, not a bare list of
ids. And every captured decision must now say what it rests on; when a premise is later
superseded, the decisions that followed from it say so.

This release has breaking changes, several of which reject input v0.6.0 accepted. Read
**Breaking changes** before upgrading, and upgrade every reader of a shared ledger before any
v0.7.0 client writes to it.

### Breaking changes

- **Capture and supersede require grounding.** Every capture answers "what does this decision
  rest on?" and is refused — nothing written — when the answer is empty. There are four ways to
  answer: a decision already made, something observed (and where), something assumed, or, when
  there is nothing yet, a declared **bet** ("nothing yet" is a bet, optionally with what would
  change your mind and a check-by date). An existing evidence or hypothesis id also counts. The
  decider's own words are not a grounding; they go in `--quote` / `--question`.
  (hivemind-gwhr.1, hivemind-gwhr.2)
  - CLI: `emit decision.capture` and `supersede` take `--rests-on-decision
    <description|#N|decision-id>`, `--rests-on-evidence <observation>` with `--evidence-source`,
    `--rests-on-assumption`, `--bet [statement]` with `--would-change-if` / `--check-by`, and
    `--confidence low|medium|high`. A capture that names none exits **2** with the four ways to
    answer. A premise description that is ambiguous or matches nothing also exits 2 and writes
    nothing; ambiguous ones list numbered candidates to re-run with `--rests-on-decision '#N'`.
  - MCP (stdio and HTTP): `capture_decision` and `supersede_decision` require a `grounding`
    array of typed items (`decision`, `evidence`, `assumption`, `bet`). An empty one returns a
    tool result with `isError: true` and the four ways to answer. `hypothesis_ids` /
    `evidence_ids` remain as deprecated aliases and count as grounding. An ambiguous or unmatched
    premise is a *successful* `{outcome: "ambiguous" | "not_found", field: "grounding[i]"}`
    result with no event appended.
  - HTTP: `POST /v1/decisions` and `POST /v1/decisions/{id}/supersessions` take the same
    `grounding` array; an empty one is **HTTP 400** (`validation_error`) with the four ways to
    answer.
  - Not asked: raw `emit decision.proposed`, classifier ingest, document import, Slack import and
    the interactive `review` supersede.
  - **The capture plugins do not ask the question yet.** The hivemind-capture skill, its
    commands and the Codex bundle were not updated in this release (hivemind-gwhr.5): until that
    plugin update ships, an agent capturing through them meets the refusal text — which does
    name the four ways to answer — instead of being prompted for grounding up front.
- **Capture rejects input v0.6.0 accepted.** Rejected outright, never truncated or repaired:
  - a rationale under 20 characters or 4 words, or one that leans on a bare reference into a
    list that isn't in it (`1a`, `2. a`) — pair `--quote` with `--question` instead
    (hivemind-763i). `--quote` and `--question` must be given together (hivemind-zdsh.13);
  - a title over 120 characters, of more than one sentence, or that is a numbered list
    (hivemind-zdsh.12);
  - an option label over 80 characters, containing "chosen", reading as a "rejected options"
    bucket, or bundling several numbered answers. Option ids are now opaque
    (`option-<uuid>`) rather than a slug of the label; existing slug ids display a readable
    label (hivemind-zdsh.10);
  - a bare UUID as an actor id — use `human:<name>` or `agent:<tool>:<name>`. Agent ids are now
    derived from a durable identity (`GC_AGENT` / `GC_ALIAS`) before falling back to session
    environment variables, so one agent keeps one actor id across restarts; the raw session id
    moves to `source_ref` (hivemind-zdsh.9).

  Ledger replay of events captured earlier is unaffected.
- **A chosen option now self-accepts.** A capture that names a chosen option is `accepted`
  immediately — by `--decided-by` when given, otherwise by the recording actor — instead of
  staying `proposed` forever. Pass `--still-proposed` (`still_proposed` over MCP and HTTP) to keep
  a genuine open recommendation at `proposed`. `supersede` and document import are unaffected.
  (hivemind-zdsh.8)
- **A ledger holding v0.7.0 writes cannot be read by v0.6.0.** v0.6.0 refuses the new fields —
  `unknown field 'option_descriptions'` on `decision.proposed`, `unknown field 'kind'` on
  `hypothesis.recorded` — so a single v0.7.0 capture is enough. Upgrade every server, CLI and
  plugin that reads a ledger *before* any v0.7.0 client writes to it. Ledgers written by v0.6.0
  replay unchanged. New event
  types: `project.registered`, `project.linked`, `project.unlinked`, `project.anchored`,
  `project.unanchored` and `decision.moved`; new optional fields on `decision.proposed`,
  `decision.accepted`, `hypothesis.recorded` and `ingest.batch_classified`; a new relation kind,
  `FOLLOWS_FROM`. Postgres projections add the new columns automatically.
- **`hivemind serve` binds loopback by default.** v0.6.0 bound `0.0.0.0` and, with no
  `HIVEMIND_API_KEY`, started with no authentication — an open server on every interface. It now
  binds `127.0.0.1` (`--bind` / `HIVEMIND_BIND`), and in that development mode (no
  `HIVEMIND_API_KEY`, no `HIVEMIND_DATABASE_URL`) it refuses to start on a non-loopback bind
  unless `--allow-unauthenticated-remote` is passed. An empty `HIVEMIND_API_KEY` or
  `HIVEMIND_ADMIN_KEY` now counts as unset (see Security under Fixed). In Docker the image binds
  `0.0.0.0` inside the container and `docker-compose.yml` publishes on `127.0.0.1`
  (`HIVEMIND_PUBLISH_ADDR` to expose it deliberately); SQLite mode in compose refuses to start
  without `HIVEMIND_API_KEY`. The image no longer bundles the website. (hivemind-4dur)
- **An unregistered tenant is an error.** An unknown `--tenant` / `X-HiveMind-Tenant` now fails
  reads and writes on both backends (`unknown tenant 'acme': run hivemind tenant create acme`)
  instead of silently opening an empty scope. `local` is always registered. On SQLite,
  `hivemind tenant create <id>` registers one — a v0.6.0 ledger that used a non-default tenant
  needs that once before use. On Postgres, tenants are provisioned by the server's provisioning
  route. (hivemind-rkbf.1)
- **Every arrow the server shows points newer → older.** `GET /v1/graph` edges, the
  neighborhood edges (`hivemind query why`, MCP `get_decision_neighborhood`, HTTP
  `/v1/decisions/why`), the CLI text summary and the DOT exports now draw each edge from the
  node recorded later to the node recorded earlier, for all 28 relation kinds. `from`/`to`
  are the arrow's ends, `relation` still names the meaning, `label` reads the relation along
  the arrow (`based on`, `informs`, `answers`) and `reversed` marks an arrow that runs against
  the relation's stored direction. Storage and queries are unchanged, so nothing needs
  migrating: graphs rebuild from the ledger. The rule and the per-kind table are in
  `docs/GRAPH_CONTRACT.md`. (hivemind-ku1x)
- **Follow-up events no longer overwrite `event_origin`.** `decision.scored`,
  `blocker.resolved`, `notification.acknowledged` and `project.anchored` leave the
  `event_origin` of the node they annotate at the offset of the event that created it. A
  persistent Postgres projection needs the usual rebuild to pick that up. (hivemind-ku1x)
- **Other wire changes.**
  - `GET /v1/decisions/{id}` returns 404 `not_found` for a missing decision (or another
    tenant's) instead of 200 with `data: null`.
  - `hivemind --version` and the MCP `serverInfo.version` read `<semver>+<commit>` (the
    commit's first 12 characters), not bare semver; `GET /v1/version` returns the same.
  - Capture and supersede replies gain `rests_on` and `premise_stale`, and on the CLI's JSON
    output and MCP `project` and `project_source`. Text mode still prints the bare decision id on
    stdout; the project line goes to stderr and each stale premise adds a `premise_stale` line.
  - `hivemind digest` text shows option titles rather than option ids, gains a `rests on:` line
    per decision and no longer ends with a `Cited:` line (the JSON keeps `cited_decision_ids`).
- **The website and docs moved; the hosted open beta is gone.** They now live in
  `alexknips/hivemind-site`; `website/` left this repo and the old GitHub Pages URL only
  redirects. The managed MCP server is down and nothing points at it: self-hosting — the release
  tarball or the ghcr image — is the only install path. (hivemind-se2a, hivemind-v5lv)

### Added

#### Fluent verbs: describe a decision, don't quote its id
- **`disagree`, `supersede`, `query chain`, `query why`, `query verify` and `query compact-view`
  take a description.** A positional description resolves to one decision; `--pick N`, `--topic`
  and a bare `#N` (from the previous ambiguous result) narrow it. A description that does not
  resolve to exactly one decision writes nothing and lists numbered candidates; `--id` /
  `--decision` / `--old` remain. The `#N` state is a local file scoped by `$HIVEMIND_SESSION`,
  never part of the ledger. (hivemind-tenv.1)
- **`why` answers why.** The neighborhood root carries the decision's brief — title, rationale,
  chosen and rejected options, who decided, whether it still holds — and every non-actor node
  carries a label. `verify` ("did it hold up?", `query get_decision_outcome`) leads with the same
  brief and shows the recorder and the decider as separate lines, saying "not yet decided" when
  nobody accepted. Natural questions resolve: question words are dropped, negations are kept so
  "do not adopt X" never resolves to the decision that adopted X, and when nothing matches every
  word, close candidates are listed with their `missing_terms` instead of `not_found`.
  (hivemind-5gwg)
- **`recall` takes its documented question form.** `recall "what did we decide about projects"
  --topic projects` drops the question words, reports them as `ignored_words`, and lets `--topic`
  decide when nothing else is left. (hivemind-5gwg)
- **`query situational` — "what should I know before I touch this?"** Decisions bearing on the
  working situation (touched paths, a diff, the current branch, the cwd) with no question needed;
  it defaults to the current git diff, and can be asked from one project (see Projects).
  (hivemind-tenv.2)
- **Over MCP and HTTP too.** The same verbs are HTTP routes (`GET /v1/decisions/recall`,
  `/why`, `/verify`, `/situational`) and MCP tools. New MCP tools on both transports:
  `get_decision_neighborhood`, `get_situational_decisions`, `scan_misfiled_decisions`,
  `classify_queue_list` and `classify_queue_submit`. `disagree_decision`,
  `get_decision_outcome`, `get_supersession_chain` and `hivemind_compact_view` accept
  `description` (+ `topic`) in place of `decision_id`, and `supersede_decision` in place of
  `old_decision_id`. (hivemind-ot72)
- **`hivemind-context` plugin.** A CLI-only Claude Code and Codex plugin — `situational`,
  `recall`, `why`, `verify`, `disagree`, `supersede` — that never asks the agent for a decision
  id. (hivemind-tenv.3)

#### What a decision rests on
- **Answers show what a decision rests on.** Every decision view gives each premise with its
  state — a decision that holds, was superseded, rejected or is contested; evidence with where it
  was seen; an assumption that is open, supported or refuted; a bet that is open, held or failed
  — and whether it was named at capture or attributed later, plus the expressed confidence and
  how many decisions rest on it. A superseded or rejected premise makes the decision **stale**
  in `verify`, `compact-view`, `situational`, `digest`, recent-activity history and the scorer;
  a contested premise is shown, not stale; an overdue bet is reported `unchecked` and does not
  flip "still holds". A decision captured before this release reads "nothing declared (never
  asked)". (hivemind-gwhr.3)
- **`hypothesis.recorded` gains `kind` (`assumption` | `bet`), `check_by` and `would_change_if`;
  a decision can `FOLLOWS_FROM` another decision** (`emit relation.added --kind follows-from`).
  (hivemind-gwhr.1)

#### Projects
- **Every decision carries its project and how that was determined.** `--project` /
  `--project-source` on `emit decision.capture`, `emit decision.proposed` and `supersede` (MCP:
  `project`, `project_source`). HiveMind checks the handle and never infers it: an unregistered
  handle is refused, and with none the decision is saved to the recorder's personal project and
  the reply says so (`personal_fallback`). `supersede` inherits the old decision's project.
  (hivemind-s15q.1–.4)
- **Every answer names its project.** Each decision that `get_decision`, `search`, `recall`,
  `situational`, `why`, `verify`, `compact-view`, `recent` and `digest` return, and the
  decision log (`hivemind export`), carries `project` (the address) and `project_label` (what a
  person calls it): the display name it was registered with, else the handle; for a personal
  project, "alex's personal project" or "claude agents' personal project". A moved decision
  names the project it moved to. A decision that a request or blocker named before any proposal recorded
  it has no project: `project` is `null` and the label reads "no project recorded", so it is
  visible rather than blank or guessed. The text output gains a trailing `project=<label>` field
  on tab-separated rows and a `project: <label>` line on paragraph-shaped ones (`verify`,
  `compact-view`, `digest`, the decision log, the TUI and Slack answers). A decision captured
  from a classified batch is filed under the recording actor's personal project.
  (hivemind-s15q.5)
- **`hivemind move` — move a decision to another project by describing it.** `hivemind move
  "<description>" --to <handle> [--pick N] [--topic T] [--reason R]`, or `--decision <id>`,
  resolves the decision the way `disagree` and `supersede` do: a description that matches more
  than one decision lists numbered candidates and writes nothing, and one that matches nothing
  is a successful `not_found` answer. Where the decision is now is read from the ledger, never
  typed; `--to` must be a registered project or your own personal project, and a decision
  already there is refused. Each move is recorded (who, when, from, to, why), keeps the
  decision's original capture provenance, and shows in `get_recent_activity` and
  `get_decisions_changed_since` as a `project_moved` change carrying `project_move`
  (`from`, `to`, `reason`); it is reversed by moving the decision back. Over MCP it is
  `move_decision` on both transports, taking `decision_id` or `description` (+ `topic`), `to` and
  `reason`, and replying `{decision_id, event_id, from, to, reason?}` like `move --json`.
  (hivemind-s15q.11)
- **"What should I know" asks from one project.** `hivemind query situational --project <handle>`
  (MCP `get_situational_decisions`, argument `project`) takes a registered handle or a personal
  address such as `personal:human:alex`. Matches come from that project first, then from the
  project it is `part_of` (an inherited constraint, labelled "from Platform; Billing is part of
  it"), then from the projects it `depends_on` (one hop, "from Auth; Billing depends on it");
  each group keeps the usual score order and paging. Every match carries a `scope` (`relation`,
  `label`), and the answer carries a `scope` note — on an empty answer too — naming the projects
  it looked in and how many more levels up the `part_of` chain and how many more linked projects
  were not followed, so a short answer never reads as a complete one. Decisions filed under any
  other project, and decisions with no project recorded, are not in a scoped answer. Superseded
  and refuted labels apply across the hop unchanged. An unregistered handle is refused with the
  `hivemind project register` hint, never answered as an empty result. `--summary` adds a `scope`
  line and a `scope=<label>` cell on each row. Links are read from the ledger, so a retracted
  link is not followed. Without `--project` the answer is unchanged, and `GET
  /v1/decisions/situational` does not take `project`. (hivemind-s15q.6)
- **`hivemind project`** — `register`, `link` / `unlink` (`part_of`, `depends_on`), `anchor`
  (folder, rig, Jira, Linear, GitHub, channel), `list`, `show`, `use` (a per-machine current
  project) and `decisions <handle>` (paged, with `truncated` and a cursor). (hivemind-s15q.2,
  .9, .13)
- **`hivemind export --format markdown --out <dir>`** writes the decision log as Markdown
  grouped per project — an `INDEX.md`, then one directory per project; `--project`, `--since`
  and `--topic` filter. The export owns its tree and prunes stale files. (hivemind-xw61)
- **`scan_misfiled_decisions`** (`query scan_misfiled_decisions`, MCP) reports decisions carrying
  a topic key you name as foreign. It only reports; to relocate one, use `hivemind move`.
  (hivemind-zdsh.14)

#### Attribution
- **`--decided-by`** records the human who decided when an agent captures: the decision is
  accepted by them and `verify` shows both. **`--delegated-by human:<name>`** marks an agent
  deciding within a human's delegation, so it reads differently from an agent deciding alone;
  `digest`, `verify` and the Markdown decision-log export show it and failure-mode attribution
  gains a `delegation` dimension. (hivemind-zdsh.3, hivemind-zdsh.6, hivemind-o7p2)
- **`POST /v1/agent-tokens`** (admin) mints a token bound to `agent:<tool>:<name>`, so a shared
  per-role token attributes its writes to an agent rather than a human. (hivemind-zdsh.19)

#### Backends and operation
- **Backend selection for the CLI and stdio MCP:** `--database-url` / `HIVEMIND_DATABASE_URL`
  points them at the shared Postgres backend, as `hivemind serve` already could. It needs a build
  with the `shared-backend-postgres` feature — the ghcr image has it, the release tarballs do
  not (they refuse with a clear error). `docker-compose.local-agents.yml` publishes Postgres on
  `127.0.0.1` for agents outside the compose network. (hivemind-ot72.2, hivemind-ot72.15)
- **Slack front door.** `POST /v1/slack/events`, `POST /v1/slack/commands` and
  `GET /v1/slack/oauth/callback`, with request-signature verification; a workspace maps to one
  tenant. Limits: SQLite backend only, `reaction_added` is verified and acknowledged but does not
  capture, and there is no modal. (hivemind-s5rs)
- **Classification over HTTP and MCP.** `classify-queue list` / `submit` use `GET /v1/classify-queue`
  and `POST /v1/classify-queue/submit` when `HIVEMIND_API_URL` is set. One submission can cover
  several batches of a session (`--batch-id a,b`, `--session-id`), and a shared daily cap
  (`HIVEMIND_CLASSIFY_DAILY_CAP`, default 150) leaves the rest pending for the next UTC day.
  (hivemind-zdsh.18)
- **`emit decision.scored`** submits a quality score computed at the edge, so a plugin can score
  without `ANTHROPIC_API_KEY` on the server. (hivemind-wi3u)
- **A build stamp.** `hivemind --version`, `GET /v1/version` and the MCP handshake report the
  commit. `make install` now installs to `~/.local/bin`, where `scripts/install.sh` puts
  releases; `scripts/install-local.sh`, `scripts/cell-update.sh` and `scripts/check-freshness.sh`
  build, roll and check a self-hosted cell. (hivemind-zdsh.7)

### Fixed

- **Security: an empty admin key no longer opens the admin routes.** A v0.6.0 server started
  with `HIVEMIND_ADMIN_KEY=""` — which the shipped `docker-compose.yml` passes when the variable
  is unset — let a request with no token, or an empty bearer, create users and mint tokens
  through `POST /v1/users`. An empty key now counts as unset and those routes refuse. If such a
  server was reachable by anyone you do not trust, review the users and tokens it holds.
  (hivemind-4dur)
- **"Did it hold up?" works.** `get_decision_outcome` and `get_decision_context` failed with
  `graph projection error: unsupported query` on the default in-memory graph — the backend the
  MCP server always uses — and on Postgres. Both are fixed. (hivemind-tenv.1, hivemind-kj0i,
  hivemind-ookw)
- **Attribution is honest.** `supersede_decision` and `disagree_decision` over MCP no longer
  label an agent `agent:codex:…` whatever its tool; the CLI's `disagree` and `supersede` record
  `source=agent` for an agent actor instead of `human`; `verify` no longer prints the recorder as
  the decider; the classifier no longer folds a per-run session id into an actor id.
  (hivemind-zdsh.9, hivemind-xm93, hivemind-zdsh.19)
- **Passive capture no longer drops the session.** The capture hook reads `session_id` and
  `transcript_path` from the hook's stdin, where Claude Code provides them, instead of returning
  silently when `CLAUDE_SESSION_ID` is empty, and resolves transcript paths containing dots and
  underscores. It advances its cursor only after a successful POST, ships every turn of a span
  oldest-first, and ships a typed prompt verbatim rather than character by character.
  (hivemind-zdsh.17, hivemind-8h7m)
- **Plugin scripts.** `capture.sh` no longer drops `--decided-by`'s value; the
  `hivemind-context` verbs join unquoted multi-word text; `query-decisions.sh` has honest
  defaults and quotes its arguments; the capture skills say to `recall` before capturing so a
  restarted session does not record a decision twice. (hivemind-zdsh.3, hivemind-tenv.3,
  hivemind-tenv.4, hivemind-zdsh.11)
- **Postgres.** Concurrent first starts against a fresh database no longer race on schema
  creation (advisory lock); `HIVEMIND_POSTGRES_POOL_SIZE` overrides the connection pool size.
  (hivemind-g9kv, hivemind-ot72.3)
- **Classifier.** A capture that names a sibling capture in the same batch by title now resolves
  to that node's real id; duplicate-title resolution warns and is counted.
  (hivemind-o4l6, hivemind-11nl)
- **Situational.** A decision with two `BASED_ON` edges to the same evidence no longer lists the
  match twice. (hivemind-nw9v)

### Known issues

- `verify` on a decision captured without grounding suggests `hivemind ground …`. That verb is
  not in this release; to attach a premise after the fact use
  `emit relation.added --kind follows-from --from <decision-id> --to <premise-decision-id>`.

---

## v0.6.0 — 2026-09-07 — M7: Decision-quality layer

HiveMind now derives whether decisions held up and scores them explainably — entirely
from graph structure, no LLM, no external calls required. An engineer and their agents
can see which decisions didn't hold up, why, and what conditions predict failure — without
any per-person ranking or blame. Scores always ship with their reasons and contributing
decision IDs. External consumers (factory loops, dashboards) pull the same signal via MCP.

### Added

#### Decision outcome signals (Mechanism foundation)
- **Decision outcome model.** Layer-2 query (`src/queries/outcome.rs`) derives
  four per-decision quality signals from existing graph edges — `superseded_fast`
  (replaced within one sprint), `premised_on_refuted` (rested on a hypothesis
  later contradicted by evidence), `contested_unresolved` (active disagreement with
  no resolution), and `thin_structure` (no options considered or evidence attached).
  No LLM, no external calls; works self-hosted.
  MCP tools: `get_decision_outcome`, `decision_quality_candidates`.
  (hivemind-he9a.1, 6085596)
- **Decision context features.** Layer-2 query (`src/queries/context.rs`) derives
  the independent-variable side of the causal pair: authorship shape (human/agent/joint),
  source, model, review depth, evidence and hypothesis counts, rationale richness proxies.
  MCP tools: `get_decision_context`, `decision_context_candidates`.
  (hivemind-he9a.2, a779f60)

#### In-house explainable scorer
- **Decision-quality scorer.** Layer-3 scorer (`src/queries/inhouse_scorer.rs`) combines
  outcome signals and context features into a `[0,1]` quality score plus a tier label,
  always with contributing reasons and decision IDs attached — never a bare number.
  Deterministic; no LLM; self-hosted and hosted. Scores decisions and interaction patterns,
  never individual people or agents.
  MCP tools: `score_decision`, `scan_decision_quality`.
  CLI: `hivemind query scan_decision_quality`.
  (hivemind-he9a.3, b006942)
- **Failure-mode attribution.** Layer-2 query (`src/queries/attribution.rs`) reports
  aggregate condition patterns (model, context sufficiency, review depth, evidence
  thinness, authorship shape) with effect sizes and honest confidence intervals.
  Output is aggregate-only — no per-person rankings, no individual identifiers.
  MCP tool: `analyze_failure_modes`.
  (hivemind-he9a.4, ba5178a)

#### MCP exposure — Mechanism A
- **Quality scores over MCP.** External consumers (factory loops, dashboards, other agents)
  pull decision scores, signals, context features, and failure-mode analysis via the existing
  MCP auth surface (hosted + self-hosted). Low-effort pull model; no new infrastructure.
  Adds hosted-mode routing in `src/api.rs` and CLI wiring.
  (hivemind-he9a.6, 14ae063)

#### Scheduled scan → Linear tickets — Mechanism B
- **Quality-scan CLI + Linear connector.** `hivemind quality-scan` schedules periodic scans
  of recent decisions and files Linear tickets for human review on decisions above a
  configurable badness threshold. Precision-biased (files only on strong signals);
  human-in-the-loop mandatory; opt-in (requires `HIVEMIND_LINEAR_API_KEY`). Reuses
  the in-house scorer — no duplicate scoring logic.
  (`src/linear.rs`, hivemind-he9a.7, be93351)

#### E2E test suite
- **SQLite smoke suite + CI job.** Compose-based e2e smoke test (SQLite backend) runs
  in CI on every push. (hivemind-fabq.1, 465277c)
- **Postgres smoke suite + CI job.** Compose-based e2e smoke test (Postgres backend)
  with tenant provisioning and bearer-token auth runs in CI. (hivemind-fabq.2, 2024c3e)

### Changed
- **`docs/DECISION_SCORING.md`** status updated to reflect shipped implementation.
- **`STRATEGY.md`** gains an 8th active front: Decision quality.
- **README install snippet.** `HIVEMIND_VERSION` example updated to `v0.6.0`.

---

## v0.5.0 — 2026-08-21 — Import unification: prose extraction and clean CLI

`hivemind import` now works end-to-end for real documents. The command
auto-detects whether input is raw prose or a pre-structured block list and
extracts decision candidates inline — no pre-processing step required.
Imported decisions land as **unreviewed** (status `proposed`) and are
surfaced by `hivemind review --unreviewed-only`, closing the review loop.
The experimental `suggest document-candidates` and `materialize-document-candidates`
sub-commands, superseded by this flow, are retired.

### Added
- **Prose-aware import.** `hivemind import` auto-detects prose vs block input
  and runs inline extraction when prose is supplied, eliminating the separate
  `suggest → materialize` step. (`src/import.rs`, 21a2368, f90992c)
- **Unreviewed landing.** All imported decisions land with status `proposed`
  (unreviewed), surfaced by `hivemind review --unreviewed-only`.
  (21a2368)

### Removed
- **`hivemind suggest document-candidates`** — superseded by auto-detection in
  `hivemind import`. (2268b1e)
- **`hivemind materialize-document-candidates`** — superseded by inline
  extraction in `hivemind import`. (2268b1e)

### Changed
- **README install snippet.** `HIVEMIND_VERSION` example updated to `v0.5.0`.
- **Docs.** Import/review flow docs updated to reflect the new single-command
  path; stale `suggest`/`materialize` references removed. (7f05a18)

### Improved
- **Hosted server hardening.** Per-request timeouts, concurrency limits, and
  JWKS refresh robustness. (hivemind-u6uu)
- **Graph caching.** Projected graph cached in `AppState` with incremental
  offset-keyed invalidation — reduces per-request rebuild latency.
  (94717be)
- **`premised_on` edge origin.** Phase 2: `PREMISED_ON` edges now originate
  from the chosen `Option` node rather than the `Decision`, matching the
  semantic intent. (hivemind-1ysb)
- **Website.** Added "Use cases" (Shipped vs Planned) and "Why HiveMind"
  narrative pages. (96f911a, cd6103e)

## v0.4.1 — 2026-07-21 — Self-host server image on ghcr

A packaging release that completes the self-hosting deployment path. No
functional changes to the `hivemind` binary since v0.4.0.

### Added
- **Self-host server image on ghcr.** Tagged releases now build and publish
  the multi-tenant server image to `ghcr.io/alexknips/hivemind` (tags
  `:latest`, `:0.4`, `:0.4.1`), built with the `shared-backend-postgres`
  feature. `docker-compose.yml` now pulls this prebuilt image, with the local
  `build:` retained as a fallback — so `docker compose up` no longer requires
  a ~5-minute local Rust build on first run. (hivemind-4qj7)

### Changed
- **README install snippet.** `HIVEMIND_VERSION` example updated to `v0.4.1`.

## v0.4.0 — 2026-07-20 — M4: Self-hosting GA, keyless capture, and Ingestion v1

Organizational self-hosting exits beta: per-user bearer tokens, a
compose-first deployment cell, WorkOS AuthKit OAuth for the hosted MCP
gateway, and self-host polish bring the self-hosted path to GA readiness.
Keyless capture (no `ANTHROPIC_API_KEY` on the server) ships via a
subscription-seat Worker A path. Ingestion v1 lands as **experimental**
connectors: a `Connector` trait, `GitFileConnector`, `GoogleDocsConnector`
(OAuth), and a same-as/dedup layer with same-as review commands. Layer-3
gains a 2-axis decision scorer and a semantic spectral decision map; the CLI
adds `recall` and `digest` subcommands; the MCP gateway adds HTTP Streamable
transport and a `recall_decisions` tool. Fidelity measurement matures via a
36-case gold corpus, A/B harness, and Tier 1.5 synthetic dispersion cases.

Prebuilt binaries ship for **linux-x86_64, linux-arm64, and macos-arm64**.
Intel macOS (`x86_64-apple-darwin`) is not currently published as a prebuilt
— the ONNX runtime that backs the spectral map ships no prebuilt for that
target — so Intel Mac users install via `cargo install --git`.

### Added

#### Self-hosting and auth
- **Per-user bearer tokens** for self-hosted cells. Each provisioned user
  receives an `hm_tk_<64-hex>` token validated against a per-cell token
  store. (`src/ledger/postgres/tenant_store.rs`, `src/ledger/sqlite/mod.rs`,
  hivemind-iioq, f57dd30)
- **Self-hosted cell compose wiring.** `docker-compose.yml` ships a
  production-ready cell configuration — env surface, volume mounts, and
  health-check; `docs/SELF_HOSTING.md` is the authoritative runbook.
  (hivemind-ppw3, 5780e7a)
- **WorkOS AuthKit OAuth.** The hosted MCP gateway accepts WorkOS-issued
  JWTs, enabling browser-based GitHub/Google login for MCP clients.
  (`src/api.rs`, hivemind-k6hv, 74d7ec0)
- **CORS layer**, opt-in. Browser SPA clients can call `/v1/` directly when
  origins are set via `HIVEMIND_CORS_ORIGINS`; off by default, so
  self-hosters are unaffected unless they enable it. (hivemind-2l8h, 936a3d7)
- **`hivemind digest` command.** Generates a weekly decision-digest summary
  for a given time window. (`src/cli/mod.rs`, `src/summarize.rs`,
  hivemind-k125, d798df7)
- **`hivemind migrate` CLI**, behind the `shared-backend-postgres` build
  feature (off by default). Migrates a local SQLite ledger to a remote
  Postgres-backed shared backend. (hivemind-uuq9.8, d4b7e7b)
- **SPA static serving + `GET /v1/graph`.** The server serves the SPA demo
  bundle and exposes `GET /v1/graph`, which returns the full decision graph
  as JSON (nodes + edges, no coordinates). The 2-D spectral *layout* is a
  separate endpoint — see the decision map below. (98fb8a7)

#### Keyless capture (no server-side ANTHROPIC_API_KEY)
- **Worker A — subscription-seat keyless path.** `hivemind classify-queue`
  CLI and capture plugin drain unclassified batches using the subscription
  seat's API key, not the server's. Enables zero-API-key self-hosted
  deployments. (hivemind-v8im, 731d367)
- **Haiku edge classifier subagent.** The capture path (`hivemind emit`
  plus the capture plugin/skill) can classify graph edges (actor, evidence,
  option relationships) at capture time via a local Haiku call rather than
  the server-side classifier. (hivemind-mfc7, 97f869d)
- **Keyless capture docs and onboarding.** `docs/KEYLESS_CAPTURE.md` —
  zero-to-first-decision walkthrough. The self-hosting funnel (README,
  `docs/SELF_HOSTING.md`, `docs/AGENT_DECISION_CAPTURE.md`) updated to
  surface the keyless path. (hivemind-kj84, 0268e8b)

#### Ingestion v1 connectors (experimental)
> These connectors are **experimental** — the `Connector` trait and
> ingestion API are subject to change before a stabilization milestone.

- **`Connector` trait + version-walk pipeline.** Common interface for
  pull-based ingestion; `GitFileConnector` walks git history and extracts
  decision candidates per version. (`src/connector/`, hivemind-2tcl.1,
  edd2af4)
- **`GoogleDocsConnector`.** Ingests Google Docs via Drive API OAuth2 flow
  with credentials and token caching. (hivemind-2tcl.2, ced6ed9)
- **Same-as / dedup layer.** Deduplicates connector output across ingestion
  runs; `hivemind import connector same-as-candidates` /
  `confirm-same-as` / `retract-same-as` for manual review of same-as
  candidates. (hivemind-2tcl.3, 7d7269a)

#### Layer-3 intelligence
- **2-axis decision scorer.** Layer-3 scorer annotates decisions (via
  `decision.scored` events) on two axes: **Quality** [0,1], a weighted
  composite over 7 dimensions, and **Importance** (Stakes × Irreversibility
  × Actionability). Swappable — query and write layers are unaffected. Note:
  scoring calls a model and requires a server-side `ANTHROPIC_API_KEY`; it is
  distinct from the keyless *capture* path. (`src/scorer.rs`,
  hivemind-uuq9.19, 7b64b8c)
- **2-D spectral decision map.** `GET /v1/decisions/map` returns a spectral
  layout (x=time, y=Fiedler eigenmap) backed by `fastembed-rs`
  BGE-small-en-v1.5 semantic embeddings. **Currently SQLite-backend only** —
  the endpoint returns a validation error on the Postgres backend, so the map
  is not yet available on the hosted deployment. (hivemind-plvq, 36519f6 /
  0c34846)
- **`hivemind query recall` CLI.** Wraps search + summarize in one command
  for fast on-demand recall. (M3/ynt5.2, 80d74c0)
- **`recall_decisions` MCP tool.** One-call search + summarize over the MCP
  gateway. (M3/ynt5.1, 825dd9e)
- **TUI summary and compact-graph views.** `'s'` opens a summary panel;
  `'v'` toggles compact-graph mode in `hivemind query`. (hivemind-0o9e,
  4d528ec)

#### MCP and transport
- **HTTP Streamable MCP transport.** MCP tools available over the HTTP
  Streamable transport in addition to stdio, enabling web-based MCP clients.
  (hivemind-iwz9, 8448868)

#### Measurement (M4)
- **Capture-fidelity evaluator and gold corpus.** `benchmarks/fidelity/` —
  36-case gold corpus, schema-ceiling scorecard, and `--ceiling` mode.
  (hivemind-21zi, hivemind-0d2v, 7dc29e2)
- **A/B uplift harness.** `ab-eval` binary compares two classifier
  strategies on the same corpus; Phase 1 scorecard published at
  `benchmarks/fidelity/ab-eval-scorecard-v1-phase1-2026-07-13.txt`.
  (ee1798d / df894c2)
- **Tier 1.5 synthetic corpus** (G1–G3): dispersion-graded cases for
  evaluating classifier robustness across signal density.
  (hivemind-92fs, d56f799)

### Fixed
- Self-host GA polish: removed broken `--agent hivemind-classifier` call in
  `capture.sh`, added batch-capture slash command, corrected non-existent
  remote MCP flags in docs, fixed `hm_sk_live_` → `hm_tk_` token-prefix
  mismatch, added Postgres FTS note in quickstart.
  (hivemind-669v, c87a819)
- Postgres AppState build/startup panic; `extract_ctx` Postgres lookups
  wrapped in `spawn_blocking`. (hivemind-noc9, hivemind-e8zp)
- Release pipeline could publish an incomplete release. `continue-on-error`
  on the build matrix plus an `if: !cancelled()` publish gate let a release
  publish with platform artifacts missing. Publish is now gated on all
  builds succeeding, with an explicit guard that refuses to publish unless
  every expected platform tarball and checksum is present. linux-arm64
  cross-linking (`cannot find -lstdc++`) is fixed by installing the aarch64
  C++ toolchain. (hivemind-ehwd, 375f81e)
- Three `cargo-audit` advisories resolved: crossbeam-epoch, quinn-proto,
  lopdf. (hivemind-7g5o)
- Docker base image bumped to Debian Trixie (glibc 2.40) for `ort`
  pre-built ONNX compatibility. (hivemind-plvq)

### Changed
- **README install snippet.** `HIVEMIND_VERSION` example updated to
  `v0.4.0`.

## v0.3.0 — 2026-06-18 — M3: Layer-3 recall tools over the MCP gateway

HiveMind gains its first Layer-3 read tools for on-demand recall: decision
summarization and graph compactification, alongside richer search. All are
exposed as read-only tools over the authenticated, tenant-isolated MCP gateway,
so agents can recall and condense organizational decision memory on demand
without a local ledger. Layer-3 stays swappable — the write and query layers
remain fully functional without it.

### Added
- **Decision summarization** (`summarize_decisions` MCP tool, Layer-3). Produces
  a text summary of the decisions matching a query/filter, built purely from the
  read layer — no writes, no inference beyond explicit status derivation.
  (`src/summarize.rs`)
- **Graph compactification** (`hivemind_compact_view` MCP tool, Layer-3). A
  `CompactView` query that collapses safely-redundant detail while preserving
  main decisions and their rationale, per the signal/noise semantics in the
  spec. (`src/queries/compact_view.rs`)
- **Compactification specification.** `docs/COMPACTIFICATION_SPEC.md` defines the
  signal/noise rules for graph compaction — what detail is safely droppable
  versus what must always be preserved.
- **Search filter improvements.** HTTP decision search now honors the `source`
  filter and comma-separated `actor_id` values. (`src/api.rs`)
- **MCP gateway end-to-end tests.** `tests/mcp_stdio_e2e.rs` exercises the stdio
  MCP gateway across search, summarize, compact-view, and tenant isolation.

### Deferred to follow-up
- **hivemind-yfbq** (P3): tighten the UBS warning baseline now that
  assertion-heavy integration-test files under `tests/` are exempted via
  `.ubsignore`. Criticals remain gated everywhere; only the warning-count
  baseline is affected.

## v0.2.0 — 2026-06-17 — M2: Shared multi-tenant backend

HiveMind now runs as a hosted multi-tenant service. A single Postgres-backed
process serves multiple tenants with cryptographic token auth and row-level
isolation, reachable over a REST API from CLI, MCP, or any HTTP client.

### Added
- **Postgres ledger + projection.** `PostgresEventLedger` and
  `PostgresGraphView` back the write and read layers in Postgres. Schema
  migrations run at startup. Projection is still rebuildable from the ledger.
  Enabled via the `shared-backend-postgres` feature flag.
- **HTTP REST API** (`/v1/`). A third transport over the same internal commands
  and query functions: `POST /v1/decisions`, `GET /v1/decisions/{id}`,
  `GET /v1/decisions/search`, `GET /v1/decisions/relevant`,
  `POST /v1/ingest`, `POST /v1/tenants`, `GET /v1/health`.
  Bearer token auth is enforced when an API key is configured.
- **Tenant provisioning + Postgres RLS isolation.** `POST /v1/tenants`
  creates a tenant and issues a `hm_tk_<64-hex>` bearer token. Token
  resolution uses a SHA-256 hash stored in `hm_tokens`. Row-level security
  in Postgres prevents cross-tenant event reads at the database layer.
  SQLite dev mode provides header-based tenant scoping for local development.
- **Capture clients: hook + sidecar.** Two autonomous capture surfaces built
  over a shared `ingest-client` core: a commit-hook shipper and a sidecar
  daemon. Both carry actor provenance and route to the same `/v1/ingest`
  endpoint.
- **Haiku classifier for ingest batches.** `POST /v1/ingest` with the
  `shared-backend-postgres` feature routes accepted batches through a
  Claude Haiku call that extracts decision candidates, topic, and confidence
  from conversation turns. Classifier output is stored per-batch.
- **MCP read gateway.** A thin MCP server that wraps the HiveMind HTTP API,
  exposing `search_decisions` and `get_decision` tools to MCP clients
  (Claude Code, Codex, IDE plugins) without a local SQLite dependency.
- **Docker deploy.** `Dockerfile` and `docker-compose.yml` for production
  deployment. `/v1/health` endpoint and Docker healthcheck. Deployment guide
  in `docs/DEPLOYMENT.md`.
- **Multi-tenant test fixtures and integration tests.** Dedicated
  `tests/multi_tenant.rs` suite covering three isolated tenants. HTTP
  integration tests in `tests/api.rs` covering auth, RLS, ingest,
  capture+query round-trip, and supersession.
- **Decision-scoring model locked** (design only; not yet implemented).
  Layer-3, post-PoC 2-axis scoring design recorded in
  `docs/DECISION_SCORING.md` with research basis in
  `docs/DECISION_QUALITY_LITERATURE.md`.

### Architecture docs updated
- `docs/ARCHITECTURE.md` — updated to reflect M2 transport surface and
  Postgres state model.
- `docs/REMOTE_DB.md` — updated to M2 shipped state.
- `docs/AGENT_DECISION_CAPTURE.md` — updated capture flow for HTTP service.
- `docs/MCP_SERVICE_SPLIT.md` — documents MCP gateway over HTTP API.
- `docs/M2_VERIFICATION.md` — e2e verification matrix, all steps passed.

### Deferred to follow-up
- **uuq9.8**: `hivemind migrate` CLI for local-to-remote ledger migration.
  Step 5 of M2 verification is explicitly out of scope for this release.
- **uuq9.19**: Full 2-axis decision scorer (Layer-3, post-PoC). Design is
  locked and documented; implementation is not authorized until after the
  hosted MVP.

## v0.1.0 — 2026-06-15 — M1: Dogfood loop ships

First milestone release. HiveMind — an event-sourced, graph-projected decision
ledger for human governance of agentic decision-making — is now usable end to
end inside its own repository.

### Added
- **Event-sourced decision ledger.** Typed events (decision, option, evidence,
  hypothesis, relation) appended to an authoritative SQLite ledger and projected
  into a graph (Decision, Actor, Evidence, Option, Hypothesis). Status
  (proposed / accepted / contested / superseded) is derived from edges, never
  stored; the ledger is the trust boundary and smart behavior stays out of the
  write path.
- **Capture surfaces.** CLI (`hivemind emit decision.capture`), an MCP server,
  and Claude Code / Codex capture plugins — all thin wrappers over the same
  internal commands, carrying actor provenance (`agent:<tool>:<session>` /
  `human:<id>`) and `source=agent|human`.
- **Deterministic query layer.** `search_decisions` with filters (topic, status,
  actor, source, time, evidence, supersession), full-text search, bounded pages,
  and explicit rank basis. Semantic/vector ranking is intentionally kept out of
  the query path (a layer-3 concern, above the ledger).
- **Dogfood loop (M1).** Concurrent multi-agent writes to a shared ledger,
  repo-local MCP config, plugin actor-prefix conventions, the `docs/DOGFOOD.md`
  operations guide, and foundational VISION / PRINCIPLES / STRATEGY decisions
  seeded into the ledger.
- **Verification recorded in the ledger itself.** The dogfood loop is marked
  operational by two distinct agents capturing real decisions, verified end to
  end — HiveMind used by the team building it.

### Notes
- Storage is local-first SQLite today. A Postgres-backed multi-tenant service is
  the M2 direction (see `docs/REMOTE_DB.md`, `docs/MULTI_TENANCY.md`).
