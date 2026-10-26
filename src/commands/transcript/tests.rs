// Parent module gates this file with #[cfg(test)]; repeat the marker so UBS can filter test-only assertions.
#[cfg(test)]
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::commands::{
    personal_project_handle, ClassifiedBatchRecorded, CommandContext, Commands, FloorInput,
    GroundingPlan, SupersedeInput,
};
use crate::events::{
    CaptureItem, Event, EventProvenance, EventType, IngestBatchReceivedPayload, TenantId,
};
use crate::ledger::{EventLedger, InMemoryEventLedger};
use crate::projector::{memory::MemoryGraph, rebuild_graph_for_tenant};
use crate::queries::{
    get_decision_brief, get_waiting_requests, DecisionBrief, DecisionStatus, WaitingRequestsRequest,
};

const SUBMITTER: &str = "agent:claude:hook";
const RECORDER: &str = "agent:loomtracer:classifier";

/// A received batch and the classifier's captures for it, as the transcript fixtures hold them.
#[derive(Deserialize)]
struct Fixture {
    batch: IngestBatchReceivedPayload,
    captures: Vec<CaptureItem>,
}

const DECISION_IN_ITS_OWN_TURN: &str =
    include_str!("../../../tests/fixtures/transcripts/decision_in_its_own_turn.json");
const REQUEST_THEN_ANSWER: &str =
    include_str!("../../../tests/fixtures/transcripts/request_then_answer.json");
const AGENT_DECIDES_ALONE: &str =
    include_str!("../../../tests/fixtures/transcripts/agent_decides_alone.json");
const USER_ASKS_UNNAMED: &str =
    include_str!("../../../tests/fixtures/transcripts/user_asks_unnamed.json");
const PERSON_DECIDES: &str =
    include_str!("../../../tests/fixtures/transcripts/person_decides.json");
const PROPOSED_ONLY: &str = include_str!("../../../tests/fixtures/transcripts/proposed_only.json");
const CALL_REPORTED_BY_ANOTHER: &str =
    include_str!("../../../tests/fixtures/transcripts/call_reported_by_another.json");
const CALL_RELAYED_UNATTRIBUTED: &str =
    include_str!("../../../tests/fixtures/transcripts/call_relayed_unattributed.json");
const AGENT_CARRIES_OUT_PERSONS_CALL: &str =
    include_str!("../../../tests/fixtures/transcripts/agent_carries_out_persons_call.json");

fn moment(timestamp: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(timestamp)
        .expect("test time parses")
        .with_timezone(&Utc)
}

fn commands(ledger: &InMemoryEventLedger) -> Commands<'_, InMemoryEventLedger> {
    Commands::new_with_context(
        ledger,
        CommandContext::new(TenantId::local(), EventProvenance::agent(RECORDER)),
    )
}

/// Receives the fixture's batch, then records the classification of it under `batch_id`.
fn record_as(
    ledger: &InMemoryEventLedger,
    fixture: &str,
    batch_id: &str,
) -> crate::Result<ClassifiedBatchRecorded> {
    record_submitted_by(ledger, fixture, batch_id, SUBMITTER)
}

/// [`record_as`] for a batch that `submitter` shipped.
fn record_submitted_by(
    ledger: &InMemoryEventLedger,
    fixture: &str,
    batch_id: &str,
    submitter: &str,
) -> crate::Result<ClassifiedBatchRecorded> {
    let fixture: Fixture = serde_json::from_str(fixture).expect("fixture parses");
    let commands = commands(ledger);
    commands.record_ingest_batch(
        submitter,
        batch_id,
        &fixture.batch.agent_tool,
        &fixture.batch.session_id,
        fixture.batch.turns,
    )?;
    commands.record_ingest_batch_classified(
        RECORDER,
        &[batch_id.to_owned()],
        "test-model",
        "2",
        fixture.captures,
        None,
    )
}

fn record(ledger: &InMemoryEventLedger, fixture: &str) -> ClassifiedBatchRecorded {
    let batch_id = serde_json::from_str::<Fixture>(fixture)
        .expect("fixture parses")
        .batch
        .batch_id;
    record_as(ledger, fixture, &batch_id).expect("classification is recorded")
}

/// A fixture re-keyed by hand: the same batch with its captures replaced by `captures`.
fn fixture_with(fixture: &str, captures: &Value) -> String {
    let mut fixture: Value = serde_json::from_str(fixture).expect("fixture parses");
    fixture["captures"] = captures.clone();
    fixture.to_string()
}

fn events_of(ledger: &InMemoryEventLedger, event_type: EventType) -> Vec<Event> {
    ledger
        .read(0, 1000)
        .expect("ledger reads")
        .into_iter()
        .filter(|event| event.event_type == event_type)
        .collect()
}

fn answers_relations(ledger: &InMemoryEventLedger) -> Vec<Event> {
    events_of(ledger, EventType::RelationAdded)
        .into_iter()
        .filter(|event| event.payload["relation"] == "ANSWERS")
        .collect()
}

fn graph_of(ledger: &InMemoryEventLedger) -> MemoryGraph {
    let graph = MemoryGraph::default();
    rebuild_graph_for_tenant(ledger, &TenantId::local(), &graph).expect("graph rebuilds");
    graph
}

fn stored_capture(
    ledger: &InMemoryEventLedger,
    recorded: &ClassifiedBatchRecorded,
    index: usize,
) -> Value {
    let event = ledger
        .read(recorded.event_id - 1, 1)
        .expect("ledger reads")
        .remove(0);
    event.payload["captures"][index].clone()
}

/// (a) A decision the classifier derived is recorded at the time of the turn it came from, not
/// when it was classified.
#[test]
fn a_derived_decision_carries_the_time_of_its_own_turn() {
    let ledger = InMemoryEventLedger::new();
    let recorded = record(&ledger, DECISION_IN_ITS_OWN_TURN);

    let turn_time = moment("2026-09-01T09:05:00Z");
    let capture = stored_capture(&ledger, &recorded, 0);
    assert_eq!(capture["source_turn_id"], "t-2");
    assert_eq!(capture["source_ts"], "2026-09-01T09:05:00Z");

    let decision_id = format!("capture:{}:0", recorded.event_id);
    let brief = get_decision_brief(&graph_of(&ledger), &decision_id)
        .expect("brief reads")
        .data
        .expect("decision is in the graph");
    assert_eq!(brief.occurred_at, Some(turn_time));
    // Not the import time: the classification event itself was written just now.
    let classified = ledger
        .read(recorded.event_id - 1, 1)
        .expect("ledger reads")
        .remove(0);
    assert!(classified.ts.expect("event carries a time") > turn_time);
}

/// With no turn time, nothing is guessed: the decision stays at the classification's time.
#[test]
fn a_decision_whose_turn_has_no_time_stays_at_the_classification_time() {
    let ledger = InMemoryEventLedger::new();
    let mut fixture: Value = serde_json::from_str(DECISION_IN_ITS_OWN_TURN).expect("parses");
    for turn in fixture["batch"]["turns"].as_array_mut().expect("turns") {
        turn.as_object_mut().expect("turn").remove("ts");
    }
    let recorded = record_as(&ledger, &fixture.to_string(), "batch-no-times")
        .expect("classification recorded");

    let capture = stored_capture(&ledger, &recorded, 0);
    assert_eq!(capture["source_turn_id"], "t-2");
    assert!(capture.get("source_ts").is_none());

    let classified = ledger
        .read(recorded.event_id - 1, 1)
        .expect("ledger reads")
        .remove(0);
    let decision_id = format!("capture:{}:0", recorded.event_id);
    let brief = get_decision_brief(&graph_of(&ledger), &decision_id)
        .expect("brief reads")
        .data
        .expect("decision is in the graph");
    assert_eq!(brief.occurred_at, classified.ts);
}

/// A capture that names no turn is recorded with no source time, and is read at the newest turn
/// time of its batch (hivemind-ohik): the batch's turns carry times, so the classification's own
/// time, written days later, is not when it was said.
#[test]
fn a_capture_that_names_no_turn_reads_at_its_batchs_newest_turn_time() {
    let ledger = InMemoryEventLedger::new();
    let mut fixture: Value = serde_json::from_str(DECISION_IN_ITS_OWN_TURN).expect("parses");
    fixture["captures"][0]
        .as_object_mut()
        .expect("capture")
        .remove("source_turn_id");
    let recorded =
        record_as(&ledger, &fixture.to_string(), "batch-no-turn").expect("classification recorded");

    let capture = stored_capture(&ledger, &recorded, 0);
    assert!(capture.get("source_turn_id").is_none());
    assert!(capture.get("source_ts").is_none());

    let newest_turn_time = moment("2026-09-01T09:20:00Z");
    let classified = ledger
        .read(recorded.event_id - 1, 1)
        .expect("ledger reads")
        .remove(0);
    assert!(classified.ts.expect("event carries a time") > newest_turn_time);
    let decision_id = format!("capture:{}:0", recorded.event_id);
    let brief = get_decision_brief(&graph_of(&ledger), &decision_id)
        .expect("brief reads")
        .data
        .expect("decision is in the graph");
    assert_eq!(brief.occurred_at, Some(newest_turn_time));
}

/// The batch's turn times are read from the ledger when the graph is built, so rebuilding the
/// graph from the same ledger gives the same time.
#[test]
fn a_decision_time_from_the_batch_is_the_same_on_every_rebuild() {
    let ledger = InMemoryEventLedger::new();
    let mut fixture: Value = serde_json::from_str(DECISION_IN_ITS_OWN_TURN).expect("parses");
    fixture["captures"][0]
        .as_object_mut()
        .expect("capture")
        .remove("source_turn_id");
    let recorded =
        record_as(&ledger, &fixture.to_string(), "batch-rebuilt").expect("classification recorded");

    let decision_id = format!("capture:{}:0", recorded.event_id);
    let times: Vec<_> = (0..2)
        .map(|_| {
            get_decision_brief(&graph_of(&ledger), &decision_id)
                .expect("brief reads")
                .data
                .expect("decision is in the graph")
                .occurred_at
        })
        .collect();
    assert_eq!(times[0], Some(moment("2026-09-01T09:20:00Z")));
    assert_eq!(times[0], times[1]);
}

/// Only a received turn's own time counts: a `source_ts` the submission carries is replaced.
#[test]
fn a_source_time_the_submission_carries_is_replaced_by_the_turns_own() {
    let ledger = InMemoryEventLedger::new();
    let mut fixture: Value = serde_json::from_str(DECISION_IN_ITS_OWN_TURN).expect("parses");
    fixture["captures"][0]["source_ts"] = json!("2000-01-01T00:00:00Z");
    let recorded =
        record_as(&ledger, &fixture.to_string(), "batch-forged").expect("classification recorded");
    let capture = stored_capture(&ledger, &recorded, 0);
    assert_eq!(capture["source_ts"], "2026-09-01T09:05:00Z");

    // And with no turn named there is no time to take, so the forged one is dropped.
    fixture["captures"][0]
        .as_object_mut()
        .expect("capture")
        .remove("source_turn_id");
    let recorded = record_as(&ledger, &fixture.to_string(), "batch-forged-no-turn")
        .expect("classification recorded");
    assert!(stored_capture(&ledger, &recorded, 0)
        .get("source_ts")
        .is_none());
}

/// (b) A request turn followed by an answering turn yields one `question.asked` at the request
/// turn's time, and a decision linked to the same question.
#[test]
fn a_request_turn_then_an_answering_turn_is_one_ask_and_a_linked_decision() {
    let ledger = InMemoryEventLedger::new();
    let recorded = record(&ledger, REQUEST_THEN_ANSWER);

    let asks = events_of(&ledger, EventType::QuestionAsked);
    assert_eq!(asks.len(), 1, "one request turn is one ask");
    let ask = &asks[0];
    assert_eq!(ask.ts, Some(moment("2026-09-02T10:00:00Z")));
    assert_eq!(ask.actor_id, "agent:claude:builder");
    assert_eq!(
        ask.payload["text"],
        "Which database should the hosted MVP use?"
    );

    // One question node, shared by the ask and the answer: the wording differs in case and
    // trailing punctuation only.
    assert_eq!(events_of(&ledger, EventType::QuestionRecorded).len(), 1);
    let answers = answers_relations(&ledger);
    assert_eq!(answers.len(), 1);
    assert_eq!(answers[0].payload["to_id"], ask.payload["question_id"]);
    let decision_id = format!("capture:{}:1", recorded.event_id);
    assert_eq!(answers[0].payload["from_id"], decision_id.as_str());
    assert_eq!(answers[0].causation_event_id, Some(recorded.event_id));
    assert_eq!(answers[0].ts, Some(moment("2026-09-02T10:30:00Z")));
    assert_eq!(answers[0].actor_id, RECORDER);

    // The ledger reads ask, then the classification, then the answer.
    assert!(ask.event_id.expect("ask has an offset") < recorded.event_id);
    assert!(answers[0].event_id.expect("answer has an offset") > recorded.event_id);

    // A reader sees both times, and the question is no longer waiting.
    let graph = graph_of(&ledger);
    let brief = get_decision_brief(&graph, &decision_id)
        .expect("brief reads")
        .data
        .expect("decision is in the graph");
    assert_eq!(brief.asked_at, Some(moment("2026-09-02T10:00:00Z")));
    assert_eq!(brief.occurred_at, Some(moment("2026-09-02T10:30:00Z")));
    let waiting = get_waiting_requests(
        &graph,
        &WaitingRequestsRequest {
            limit: 10,
            cursor: None,
        },
    )
    .expect("waiting reads")
    .data;
    assert!(waiting.items.is_empty());
}

/// A request nobody has answered is waiting, asked at the request turn's own time.
#[test]
fn a_request_turn_nobody_answered_is_waiting() {
    let ledger = InMemoryEventLedger::new();
    let mut fixture: Value = serde_json::from_str(REQUEST_THEN_ANSWER).expect("parses");
    fixture["captures"]
        .as_array_mut()
        .expect("captures")
        .truncate(1);
    record_as(&ledger, &fixture.to_string(), "batch-unanswered").expect("classification recorded");

    let waiting = get_waiting_requests(
        &graph_of(&ledger),
        &WaitingRequestsRequest {
            limit: 10,
            cursor: None,
        },
    )
    .expect("waiting reads")
    .data;
    assert_eq!(waiting.items.len(), 1);
    assert_eq!(waiting.items[0].asked_at, moment("2026-09-02T10:00:00Z"));
    assert_eq!(
        waiting.items[0].requested_by.as_deref(),
        Some("agent:claude:builder")
    );
}

/// The same transcript ingested again adds no second ask: it is the same moment seen again.
#[test]
fn a_re_ingested_request_turn_is_not_asked_twice() {
    let ledger = InMemoryEventLedger::new();
    record(&ledger, REQUEST_THEN_ANSWER);
    record_as(&ledger, REQUEST_THEN_ANSWER, "batch-request-answer-again")
        .expect("classification recorded");

    assert_eq!(events_of(&ledger, EventType::QuestionAsked).len(), 1);
    assert_eq!(events_of(&ledger, EventType::QuestionRecorded).len(), 1);
}

/// The request fixture with the actor the classifier named on the request left out.
fn request_with_no_named_asker() -> Value {
    let mut fixture: Value = serde_json::from_str(REQUEST_THEN_ANSWER).expect("parses");
    fixture["captures"][0]
        .as_object_mut()
        .expect("capture")
        .remove("actor_id");
    fixture
}

/// Without an actor named on the request, a request an assistant's turn made is credited to the
/// batch's submitter: that agent spoke the turn, which the batch records.
#[test]
fn an_assistant_turn_request_with_no_named_asker_is_by_the_batch_submitter() {
    let ledger = InMemoryEventLedger::new();
    let fixture = request_with_no_named_asker();
    assert_eq!(fixture["batch"]["turns"][0]["role"], "assistant");
    record_as(&ledger, &fixture.to_string(), "batch-unnamed-asker").expect("recorded");

    let asks = events_of(&ledger, EventType::QuestionAsked);
    assert_eq!(asks.len(), 1);
    assert_eq!(asks[0].actor_id, SUBMITTER);
    assert_eq!(asks[0].ts, Some(moment("2026-09-02T10:00:00Z")));
}

/// Without an actor named on the request, a request a user's turn made writes no ask: the batch
/// records no human, and a human's question is never credited to the agent that submitted it.
/// The decision that answers it is still recorded, at its own turn's time, and still linked to
/// the question; the timeline reads "asked at: not recorded" and nothing waits.
#[test]
fn a_user_turn_request_with_no_named_asker_writes_no_ask_but_the_decision_is_recorded() {
    let ledger = InMemoryEventLedger::new();
    let recorded = record(&ledger, USER_ASKS_UNNAMED);

    assert!(events_of(&ledger, EventType::QuestionAsked).is_empty());
    assert_eq!(events_of(&ledger, EventType::QuestionRecorded).len(), 1);
    assert_eq!(answers_relations(&ledger).len(), 1);

    let graph = graph_of(&ledger);
    let decision_id = format!("capture:{}:1", recorded.event_id);
    let brief = get_decision_brief(&graph, &decision_id)
        .expect("brief reads")
        .data
        .expect("decision is in the graph");
    assert_eq!(brief.occurred_at, Some(moment("2026-09-04T09:05:00Z")));
    assert_eq!(brief.asked_at, None);
    let waiting = get_waiting_requests(
        &graph,
        &WaitingRequestsRequest {
            limit: 10,
            cursor: None,
        },
    )
    .expect("waiting reads")
    .data;
    assert!(waiting.items.is_empty());
}

/// A role that does not say an agent spoke the turn (a system note, a tool summary, none at all)
/// credits nobody: only an assistant's turn lets the submitter stand in for the asker.
#[test]
fn a_request_turn_of_any_other_role_with_no_named_asker_writes_no_ask() {
    for role in ["system", "tool-summary", "", "Assistant"] {
        let ledger = InMemoryEventLedger::new();
        let mut fixture = request_with_no_named_asker();
        fixture["batch"]["turns"][0]["role"] = json!(role);
        record_as(&ledger, &fixture.to_string(), "batch-other-role").expect("recorded");

        assert!(
            events_of(&ledger, EventType::QuestionAsked).is_empty(),
            "a request in a {role:?} turn with no named asker writes no ask"
        );
    }
}

/// A named actor wins on either role: the classifier said who asked, so the ask is theirs.
#[test]
fn a_named_asker_is_credited_whatever_the_role_of_the_request_turn() {
    for role in ["assistant", "user"] {
        let ledger = InMemoryEventLedger::new();
        let mut fixture: Value = serde_json::from_str(REQUEST_THEN_ANSWER).expect("parses");
        fixture["batch"]["turns"][0]["role"] = json!(role);
        fixture["captures"][0]["actor_id"] = json!("human:alex");
        record_as(&ledger, &fixture.to_string(), "batch-named-asker").expect("recorded");

        let asks = events_of(&ledger, EventType::QuestionAsked);
        assert_eq!(
            asks.len(),
            1,
            "a named asker in a {role} turn makes the ask"
        );
        assert_eq!(asks[0].actor_id, "human:alex");
    }
}

/// (c) An agent deciding alone mid-task makes no ask.
#[test]
fn an_agent_deciding_alone_writes_no_ask() {
    let ledger = InMemoryEventLedger::new();
    let recorded = record(&ledger, AGENT_DECIDES_ALONE);

    assert!(events_of(&ledger, EventType::QuestionAsked).is_empty());
    assert!(events_of(&ledger, EventType::QuestionRecorded).is_empty());
    assert!(answers_relations(&ledger).is_empty());
    // Its own turn's time is still the decision's.
    let decision_id = format!("capture:{}:0", recorded.event_id);
    let brief = get_decision_brief(&graph_of(&ledger), &decision_id)
        .expect("brief reads")
        .data
        .expect("decision is in the graph");
    assert_eq!(brief.occurred_at, Some(moment("2026-09-03T11:00:00Z")));
    assert_eq!(brief.asked_at, None);
}

/// A decision that states the question it answers is linked to it, but with no request turn
/// before it nobody asked: no ask, and the timeline reads "asked at: not recorded".
#[test]
fn a_decision_stating_its_question_with_no_request_turn_is_linked_but_not_asked() {
    let ledger = InMemoryEventLedger::new();
    let mut captures =
        serde_json::from_str::<Value>(AGENT_DECIDES_ALONE).expect("parses")["captures"].clone();
    captures[0]["question"] = json!("How many times should the queue retry a failed job?");
    let recorded = record_as(
        &ledger,
        &fixture_with(AGENT_DECIDES_ALONE, &captures),
        "batch-states-question",
    )
    .expect("classification recorded");

    assert!(events_of(&ledger, EventType::QuestionAsked).is_empty());
    assert_eq!(events_of(&ledger, EventType::QuestionRecorded).len(), 1);
    assert_eq!(answers_relations(&ledger).len(), 1);
    let decision_id = format!("capture:{}:0", recorded.event_id);
    let brief = get_decision_brief(&graph_of(&ledger), &decision_id)
        .expect("brief reads")
        .data
        .expect("decision is in the graph");
    assert_eq!(brief.asked_at, None);
}

/// A request turn with no time would be dated by when it was classified, so it writes no ask.
#[test]
fn a_request_turn_with_no_time_writes_no_ask() {
    let ledger = InMemoryEventLedger::new();
    let mut fixture: Value = serde_json::from_str(REQUEST_THEN_ANSWER).expect("parses");
    fixture["batch"]["turns"][0]
        .as_object_mut()
        .expect("turn")
        .remove("ts");
    record_as(&ledger, &fixture.to_string(), "batch-request-no-time").expect("recorded");

    assert!(events_of(&ledger, EventType::QuestionAsked).is_empty());
    // The answer still links to its question, at its own turn's time.
    assert_eq!(answers_relations(&ledger).len(), 1);
}

/// A request that states no question has no words to ask: no ask is written.
#[test]
fn a_request_with_no_question_writes_no_ask() {
    let ledger = InMemoryEventLedger::new();
    let mut fixture: Value = serde_json::from_str(REQUEST_THEN_ANSWER).expect("parses");
    fixture["captures"][0]
        .as_object_mut()
        .expect("capture")
        .remove("question");
    record_as(&ledger, &fixture.to_string(), "batch-request-no-question").expect("recorded");

    assert!(events_of(&ledger, EventType::QuestionAsked).is_empty());
}

/// Every refusal leaves nothing behind but the received batch.
fn assert_refused_with_nothing_written(fixture: &Value, expected: &str) {
    let ledger = InMemoryEventLedger::new();
    let error = record_as(&ledger, &fixture.to_string(), "batch-refused")
        .expect_err("classification is refused");
    assert!(
        error.to_string().contains(expected),
        "refusal should say {expected:?}, got: {error}"
    );
    let events = ledger.read(0, 100).expect("ledger reads");
    assert_eq!(events.len(), 1, "only the received batch is on the ledger");
    assert_eq!(events[0].event_type, EventType::IngestBatchReceived);
}

#[test]
fn a_turn_the_batch_does_not_hold_is_refused() {
    let mut fixture: Value = serde_json::from_str(REQUEST_THEN_ANSWER).expect("parses");
    fixture["captures"][1]["source_turn_id"] = json!("t-99");
    assert_refused_with_nothing_written(&fixture, "names turn t-99");
}

#[test]
fn a_blank_turn_id_is_refused() {
    let mut fixture: Value = serde_json::from_str(REQUEST_THEN_ANSWER).expect("parses");
    fixture["captures"][1]["source_turn_id"] = json!("  ");
    assert_refused_with_nothing_written(&fixture, "source_turn_id must not be empty");
}

#[test]
fn a_question_on_a_kind_that_takes_none_is_refused() {
    let mut fixture: Value = serde_json::from_str(REQUEST_THEN_ANSWER).expect("parses");
    fixture["captures"][1]["kind"] = json!("evidence");
    assert_refused_with_nothing_written(&fixture, "only a decision or a decision-request");
}

#[test]
fn a_question_with_no_words_is_refused() {
    let mut fixture: Value = serde_json::from_str(REQUEST_THEN_ANSWER).expect("parses");
    fixture["captures"][0]["question"] = json!("?");
    assert_refused_with_nothing_written(&fixture, "no words in it");
}

/// A decision the same moment seen again is dropped as a restatement and writes no second link.
#[test]
fn a_decision_dropped_as_the_same_moment_seen_again_writes_no_link() {
    let ledger = InMemoryEventLedger::new();
    let first = record(&ledger, REQUEST_THEN_ANSWER);
    let restated_id = format!("capture:{}:1", first.event_id);

    let mut fixture: Value = serde_json::from_str(REQUEST_THEN_ANSWER).expect("parses");
    fixture["captures"][1]["restates_id"] = json!(restated_id);
    record_as(&ledger, &fixture.to_string(), "batch-request-answer-again").expect("recorded");

    assert_eq!(answers_relations(&ledger).len(), 1);
    assert_eq!(events_of(&ledger, EventType::QuestionAsked).len(), 1);
}

// --- who decided (hivemind-u70b, hivemind-zdsh.23) ---
//
// A classified decision is accepted only when the capture names an acceptor, and the classifier
// names one only when the text shows that actor deciding, or the session agent making and
// carrying out a call of its own (`CLASSIFIER_PROMPT` and
// `plugins/loomtracer-capture/commands/classify-queue.md` carry the rule). Each fixture below is
// a transcript with the answer that rule gives; replaying it through the write path and the
// status read shows the status and decider it lands in.

/// A recorded fixture's one decision as `why` and `verify` read it: its status, who decided it,
/// and who the capture credits as its proposer.
fn who_decided(fixture: &str) -> (DecisionStatus, Vec<String>, Option<String>) {
    let ledger = InMemoryEventLedger::new();
    let recorded = record(&ledger, fixture);
    let decision_id = format!("capture:{}:0", recorded.event_id);
    let brief = get_decision_brief(&graph_of(&ledger), &decision_id)
        .expect("brief reads")
        .data
        .expect("decision is in the graph");
    (
        brief.status,
        brief.decided_by.decider_ids,
        brief.decided_by.proposer_id,
    )
}

/// A person states the choice: they are its decider, and it reads accepted.
#[test]
fn who_decided_a_person_who_states_the_choice_is_its_decider() {
    let (status, deciders, proposer) = who_decided(PERSON_DECIDES);

    assert_eq!(status, DecisionStatus::Accepted);
    assert_eq!(deciders, vec!["human:dana".to_owned()]);
    assert_eq!(proposer.as_deref(), Some("human:dana"));
}

/// The session agent makes a call mid-task and carries it out: the call was its own to make, so
/// it is the decider, as the submitter of the batch (`Session agent:` on the classifier's input).
#[test]
fn who_decided_the_session_agent_deciding_alone_is_its_own_decider() {
    let (status, deciders, proposer) = who_decided(AGENT_DECIDES_ALONE);

    assert_eq!(status, DecisionStatus::Accepted);
    assert_eq!(deciders, vec![SUBMITTER.to_owned()]);
    assert_eq!(proposer, None, "who proposed it is still not inferred");
}

/// A person makes the call and the session agent carries it out: the person is the decider, and
/// the agent that implemented it is not made one.
#[test]
fn who_decided_an_agent_carrying_out_a_persons_call_leaves_the_person_its_decider() {
    let (status, deciders, proposer) = who_decided(AGENT_CARRIES_OUT_PERSONS_CALL);

    assert_eq!(status, DecisionStatus::Accepted);
    assert_eq!(deciders, vec!["human:sam".to_owned()]);
    assert_eq!(proposer.as_deref(), Some("human:sam"));
}

/// An agent puts a call up for review: it proposed it, nobody has decided it.
#[test]
fn who_decided_an_agents_proposal_awaiting_review_stays_proposed() {
    let (status, deciders, proposer) = who_decided(PROPOSED_ONLY);

    assert_eq!(status, DecisionStatus::Proposed);
    assert!(
        deciders.is_empty(),
        "nobody is shown deciding: {deciders:?}"
    );
    assert_eq!(proposer.as_deref(), Some("agent:claude:planner"));
}

/// One actor reports a call another made: the one the text shows approving is the decider, and
/// the reporter is only credited with having recorded it.
#[test]
fn who_decided_a_call_reported_by_another_is_decided_by_who_made_it() {
    let (status, deciders, proposer) = who_decided(CALL_REPORTED_BY_ANOTHER);

    assert_eq!(status, DecisionStatus::Accepted);
    assert_eq!(deciders, vec!["human:priya".to_owned()]);
    assert_eq!(proposer.as_deref(), Some("agent:claude:release"));
}

/// A call passed on without saying who made it has no decider, however sure the person passing it
/// on sounds: it stays proposed, and the one relaying it is not made its decider.
#[test]
fn who_decided_a_call_relayed_without_its_decider_stays_proposed() {
    let (status, deciders, proposer) = who_decided(CALL_RELAYED_UNATTRIBUTED);

    assert_eq!(status, DecisionStatus::Proposed);
    assert!(
        deciders.is_empty(),
        "nobody is shown deciding: {deciders:?}"
    );
    assert_eq!(proposer.as_deref(), Some("human:sam"));
}

/// The decision the first capture of `recorded` became, as the graph reads it.
fn draft_brief(ledger: &InMemoryEventLedger, recorded: &ClassifiedBatchRecorded) -> DecisionBrief {
    let decision_id = format!("capture:{}:0", recorded.event_id);
    get_decision_brief(&graph_of(ledger), &decision_id)
        .expect("brief reads")
        .data
        .expect("decision is in the graph")
}

/// hivemind-6td5: a draft submitted through the queue carries nothing about who the session was
/// held with in its own words; it names the submitter of the batch the ledger received, the agent
/// that ran beside a human, and the session.
#[test]
fn a_draft_names_the_session_and_who_it_was_held_with_from_the_received_batch() {
    let ledger = InMemoryEventLedger::new();
    let recorded = record_submitted_by(
        &ledger,
        DECISION_IN_ITS_OWN_TURN,
        "batch-human",
        "human:alex@example.com",
    )
    .expect("classification recorded");

    let brief = draft_brief(&ledger, &recorded);

    let drafted = brief
        .decided_by
        .drafted_from
        .expect("a classifier draft says where it was drafted");
    assert_eq!(drafted.session_ids, ["session-own-turn"]);
    assert_eq!(
        drafted.initiated_by.as_deref(),
        Some("human:alex@example.com")
    );
    assert_eq!(
        drafted.participants,
        ["agent:claude:hook", "human:alex@example.com"]
    );
    // Nobody is credited as the proposer or the decider: the transcript named neither.
    assert_eq!(brief.decided_by.proposer_id, None);
    assert!(brief.decided_by.decider_ids.is_empty());
}

/// Who a session was held with is not a submission's to say: a worker that sends names there has
/// them dropped from the stored capture and from the graph.
#[test]
fn names_a_submission_carries_for_its_session_are_dropped() {
    let ledger = InMemoryEventLedger::new();
    let mut fixture: Value = serde_json::from_str(DECISION_IN_ITS_OWN_TURN).expect("parses");
    fixture["captures"][0]["participants"] = json!(["human:mallory@example.com"]);
    fixture["captures"][0]["session_initiator"] = json!("human:mallory@example.com");

    let recorded = record_as(&ledger, &fixture.to_string(), "batch-forged-session")
        .expect("classification recorded");

    let capture = stored_capture(&ledger, &recorded, 0);
    assert!(capture.get("participants").is_none(), "{capture}");
    assert!(capture.get("session_initiator").is_none(), "{capture}");
    let drafted = draft_brief(&ledger, &recorded)
        .decided_by
        .drafted_from
        .expect("a classifier draft");
    assert_eq!(drafted.initiated_by.as_deref(), Some(SUBMITTER));
    assert_eq!(drafted.participants, [SUBMITTER]);
}

/// A classification of a batch the ledger never received (a `loomtracer emit` capture) names a
/// draft with an empty conversation: nothing is guessed, and it is no error.
#[test]
fn a_draft_of_an_unreceived_batch_names_no_session_and_nobody() {
    let ledger = InMemoryEventLedger::new();
    let mut fixture: Fixture =
        serde_json::from_str(DECISION_IN_ITS_OWN_TURN).expect("fixture parses");
    for capture in &mut fixture.captures {
        capture.source_turn_id = None;
    }
    let recorded = commands(&ledger)
        .record_ingest_batch_classified(
            RECORDER,
            &["batch-never-received".to_owned()],
            "test-model",
            "2",
            fixture.captures,
            None,
        )
        .expect("classification recorded");

    let drafted = draft_brief(&ledger, &recorded)
        .decided_by
        .drafted_from
        .expect("a classifier draft");

    assert!(drafted.session_ids.is_empty());
    assert_eq!(drafted.initiated_by, None);
    assert!(drafted.participants.is_empty());
}

// --- a person decides a draft (hivemind-r7d73) ---
//
// A draft the classifier recorded has no `decision.proposed`: it is `capture:<event>:<index>`
// in the batch that holds it. The person who made the call writes the same events a decision
// recorded any other way takes, and the status read then says what they did.

const ALEX: &str = "human:alex@example.com";
const BOB: &str = "human:bob@example.com";

/// A draft nobody has decided: its id and the ledger it is in.
fn undecided_draft() -> (InMemoryEventLedger, String) {
    let ledger = InMemoryEventLedger::new();
    let recorded = record(&ledger, PROPOSED_ONLY);
    let draft = draft_brief(&ledger, &recorded);
    assert_eq!(draft.status, DecisionStatus::Proposed);
    assert!(draft.decided_by.decider_ids.is_empty());
    (ledger, draft.decision_id)
}

fn brief_of(ledger: &InMemoryEventLedger, decision_id: &str) -> DecisionBrief {
    get_decision_brief(&graph_of(ledger), decision_id)
        .expect("brief reads")
        .data
        .expect("decision is in the graph")
}

#[test]
fn a_person_accepts_a_draft_and_verify_reads_it_accepted_by_them() {
    let (ledger, id) = undecided_draft();

    let event_id = commands(&ledger)
        .accept_decision(&id, ALEX)
        .expect("a person can accept a draft");

    let accepted = &events_of(&ledger, EventType::DecisionAccepted)[0];
    assert_eq!(accepted.event_id, Some(event_id));
    assert_eq!(accepted.actor_id, ALEX);
    let brief = brief_of(&ledger, &id);
    assert_eq!(brief.status, DecisionStatus::Accepted);
    assert_eq!(brief.decided_by.decider_ids, [ALEX]);
}

#[test]
fn a_person_rejects_a_draft() {
    let (ledger, id) = undecided_draft();

    commands(&ledger)
        .reject_decision(&id, ALEX)
        .expect("a person can reject a draft");

    let brief = brief_of(&ledger, &id);
    assert_eq!(brief.status, DecisionStatus::Rejected);
    assert_eq!(brief.decided_by.rejecter_ids, [ALEX]);
}

#[test]
fn a_person_who_disagrees_with_an_accepted_draft_makes_it_contested() {
    let (ledger, id) = undecided_draft();
    let commands = commands(&ledger);
    commands.accept_decision(&id, ALEX).expect("accepted");

    commands
        .disagree(BOB, &id, "the pinned clock is the real fix")
        .expect("a person can disagree with a draft");

    let rejected = &events_of(&ledger, EventType::DecisionRejected)[0];
    assert_eq!(rejected.actor_id, BOB);
    assert_eq!(
        rejected.payload["reason"],
        "the pinned clock is the real fix"
    );
    assert_eq!(brief_of(&ledger, &id).status, DecisionStatus::Contested);
}

/// A retitle starts from the title the draft carries and each later one from the last.
#[test]
fn a_person_retitles_a_draft_again_and_again() {
    let (ledger, id) = undecided_draft();
    let commands = commands(&ledger);

    let first = commands
        .retitle_decision_to(ALEX, &id, "Retry the gate once", None)
        .expect("a person can retitle a draft");
    let second = commands
        .retitle_decision_to(ALEX, &id, "Retry the nightly gate a single time", None)
        .expect("and again");

    assert_eq!(first.from, "Retry the nightly gate once before failing it");
    assert_eq!(second.from, "Retry the gate once");
    assert_eq!(
        brief_of(&ledger, &id).title,
        "Retry the nightly gate a single time"
    );
}

/// A draft is filed in the personal project of whoever recorded the classification; a person
/// moves it to their own, and a second move starts from there.
#[test]
fn a_person_moves_a_draft_out_of_the_classifiers_personal_project() {
    let (ledger, id) = undecided_draft();
    let commands = commands(&ledger);
    let mine = personal_project_handle(ALEX);

    let moved = commands
        .move_decision_to(ALEX, &id, &mine, Some("this one is mine"))
        .expect("a person can move a draft");

    assert_eq!(moved.from, personal_project_handle(RECORDER));
    assert_eq!(moved.to, mine);
    assert_eq!(
        brief_of(&ledger, &id).project.as_deref(),
        Some(mine.as_str())
    );
    let back = commands.move_decision_to(ALEX, &id, &personal_project_handle(RECORDER), None);
    assert!(
        back.is_err(),
        "another actor's personal project is not one to move into"
    );
}

#[test]
fn a_person_says_what_a_draft_rests_on() {
    let (ledger, id) = undecided_draft();
    let commands = commands(&ledger);

    let grounded = commands
        .ground_and_answer(
            ALEX,
            &id,
            &GroundingPlan {
                new_assumptions: vec!["the flake is the test clock".to_owned()],
                ..GroundingPlan::default()
            },
            None,
        )
        .expect("a person can ground a draft");
    let evidence = commands
        .record_evidence(ALEX, "the gate passed on every retry last week")
        .expect("evidence");
    commands
        .attach_evidence(&id, &evidence, ALEX)
        .expect("a person can attach evidence to a draft");

    assert_eq!(grounded.rests_on.len(), 1);
    let assumes = events_of(&ledger, EventType::RelationAdded)
        .into_iter()
        .find(|event| event.payload["relation"] == "ASSUMES")
        .expect("the assumption is linked");
    assert_eq!(assumes.payload["from_id"], id);
    assert_eq!(assumes.actor_id, ALEX);
    brief_of(&ledger, &id);
}

#[test]
fn a_person_replaces_a_draft_and_the_draft_reads_superseded() {
    let (ledger, id) = undecided_draft();

    let replacement = commands(&ledger)
        .supersede(SupersedeInput {
            actor_id: ALEX,
            old_decision_id: &id,
            new_title: "Pin the nightly gate clock",
            new_rationale: "A retry hides the flake; a pinned clock removes it.",
            topic_keys: &[],
            option_labels: &["Pin the clock".to_owned(), "Retry once".to_owned()],
            chosen_option_label: Some("Pin the clock"),
            still_proposed: false,
            hypothesis_ids: &[],
            evidence_ids: &[],
            project: None,
            grounding: None,
            expressed_confidence: None,
            floor: FloorInput::NotAsked,
        })
        .expect("a person can replace a draft");

    let proposed = events_of(&ledger, EventType::DecisionProposed)
        .into_iter()
        .find(|event| event.payload["decision_id"] == replacement.new_decision_id)
        .expect("the replacement is proposed");
    assert_eq!(proposed.actor_id, ALEX);
    assert_eq!(
        proposed.payload["topic_keys"],
        json!(["ci"]),
        "the replacement keeps the draft's topic"
    );
    assert_eq!(brief_of(&ledger, &id).status, DecisionStatus::Superseded);
}

/// An id that is not a decision of a classified batch of this ledger is still no decision.
#[test]
fn an_id_that_is_no_draft_is_still_refused_and_writes_nothing() {
    let (ledger, id) = undecided_draft();
    let batch = id
        .strip_prefix("capture:")
        .and_then(|rest| rest.split(':').next())
        .expect("a capture id");
    let before = ledger.latest_offset().expect("latest offset");

    for missing in [
        format!("capture:{batch}:9"), // past the end of the batch
        "capture:1:0".to_owned(),     // event 1 is a received batch, not a classification
        "capture:999:0".to_owned(),   // no such event
        "capture:x:y".to_owned(),
    ] {
        let error = commands(&ledger)
            .accept_decision(&missing, ALEX)
            .expect_err("no draft there");
        assert!(
            error.to_string().contains("decision does not exist"),
            "{missing}: {error}"
        );
    }
    assert_eq!(ledger.latest_offset().expect("latest offset"), before);
}

// ---------------------------------------------------------------------------------------------
// hivemind-odmnc: a submit names only batches the tenant received
// ---------------------------------------------------------------------------------------------

/// A received batch with no turns, the way `POST /v1/ingest` leaves one.
fn receive_batch(ledger: &InMemoryEventLedger, batch_id: &str) {
    commands(ledger)
        .record_ingest_batch(SUBMITTER, batch_id, "claude", "session-x", Vec::new())
        .expect("the batch is received");
}

fn batch_ids(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

#[test]
fn batches_the_tenant_received_are_known_and_a_classified_one_still_is() {
    let ledger = InMemoryEventLedger::new();
    receive_batch(&ledger, "s:0-1");
    receive_batch(&ledger, "s:1-2");
    commands(&ledger)
        .record_ingest_batch_classified(
            RECORDER,
            &batch_ids(&["s:0-1"]),
            "test-model",
            "2",
            Vec::new(),
            None,
        )
        .expect("the first batch is classified");

    commands(&ledger)
        .require_received_batches(&batch_ids(&["s:0-1", "s:1-2"]))
        .expect("both were received; sending the classified one again is not refused");
}

#[test]
fn a_batch_nobody_received_is_refused_by_name_and_writes_nothing() {
    use crate::error::{CommandError, LoomtracerError};

    let ledger = InMemoryEventLedger::new();
    receive_batch(&ledger, "s:0-1");
    let before = ledger.latest_offset().expect("latest offset");

    let error = commands(&ledger)
        .require_received_batches(&batch_ids(&["s:0-1", "typo:0-1", "other:0-1", "typo:0-1"]))
        .expect_err("two of the ids were never received");

    // Named in the order given, each once, and the received one is not among them.
    assert!(
        matches!(&error, LoomtracerError::Command(CommandError::Invariant(message))
            if message.contains("ingest batch does not exist: typo:0-1, other:0-1. ")),
        "{error}"
    );
    assert!(
        error.to_string().contains("nothing was recorded"),
        "{error}"
    );
    assert_eq!(ledger.latest_offset().expect("latest offset"), before);
}

#[test]
fn a_batch_with_no_batch_at_all_in_the_ledger_is_refused() {
    let ledger = InMemoryEventLedger::new();

    let error = commands(&ledger)
        .require_received_batches(&batch_ids(&["s:0-1"]))
        .expect_err("an empty ledger received nothing");

    assert!(
        error
            .to_string()
            .contains("ingest batch does not exist: s:0-1"),
        "{error}"
    );
}

#[test]
fn another_tenants_batch_is_not_this_tenants_to_classify() {
    let ledger = InMemoryEventLedger::new();
    let acme = Commands::new_with_context(
        &ledger,
        CommandContext::new(
            TenantId::new("acme").expect("a tenant id"),
            EventProvenance::agent(RECORDER),
        ),
    );
    acme.record_ingest_batch(SUBMITTER, "acme:0-1", "claude", "session-x", Vec::new())
        .expect("acme receives the batch");

    acme.require_received_batches(&batch_ids(&["acme:0-1"]))
        .expect("acme received it");
    let error = commands(&ledger)
        .require_received_batches(&batch_ids(&["acme:0-1"]))
        .expect_err("the local tenant never received it");
    assert!(
        error
            .to_string()
            .contains("ingest batch does not exist: acme:0-1"),
        "{error}"
    );
}
