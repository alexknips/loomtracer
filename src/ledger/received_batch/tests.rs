use serde_json::{json, Value};

use super::*;
use crate::events::{EventSource, TenantId};

fn received(payload: Value) -> Event {
    Event {
        tenant_id: TenantId::local(),
        event_id: Some(7),
        event_uuid: uuid::Uuid::new_v4(),
        correlation_id: None,
        causation_event_id: None,
        event_type: EventType::IngestBatchReceived,
        actor_id: "agent:claude:hook".to_owned(),
        source: EventSource::Cli,
        source_ref: None,
        payload,
        ts: None,
    }
}

/// Payloads of the shapes a reader meets: well-formed, partial, the wrong kind of value at a
/// key, a key that repeats, escapes and unicode, a turn that is not an object, a time that is
/// not a time, and a payload that is not an object at all. The text of each is what the SQL
/// ledgers hold.
fn payload_texts() -> Vec<String> {
    let mut texts: Vec<String> = [
        json!({"batch_id": "b1", "session_id": "s1", "agent_tool": "claude",
               "turns": [{"ts": "2026-09-18T06:00:00Z", "text": "hi"},
                         {"ts": "2026-09-19T08:00:00+02:00", "role": "agent"},
                         {"ts": "2026-09-17T08:00:00Z"}]}),
        json!({"batch_id": "b2"}),
        json!({"batch_id": "b3", "turns": []}),
        json!({"batch_id": "b4", "turns": [{"text": "no time"}, {"ts": null}, {"ts": 7},
                                           {"ts": "not a time"}, "text", 4, null, [1, 2],
                                           {"ts": ["2026-09-18T06:00:00Z"]}]}),
        json!({"batch_id": "b5", "turns": {"ts": "2026-09-18T06:00:00Z"}}),
        json!({"batch_id": "b6", "turns": "2026-09-18T06:00:00Z"}),
        json!({"batch_id": "b7", "turns": [[{"ts": "2026-09-18T06:00:00Z"}]]}),
        json!({"batch_id": 42, "session_id": 1, "agent_tool": ["claude"], "turns": null}),
        json!({"batch_id": null, "session_id": {"id": "s"}, "agent_tool": true}),
        json!({"batch_id": "", "session_id": "", "agent_tool": ""}),
        json!({"batch_id": "b8", "session_id": "   ", "agent_tool": "codex"}),
        json!({"batch_id": "ünï \"quoted\" \\ 🙂\n", "session_id": "s\u{0}z",
               "turns": [{"ts": "2026-09-18T06:00:00.123456789Z", "n": 1.5e3}]}),
        json!({"nested": {"batch_id": "not top level", "turns": [{"ts": "2026-09-18T06:00:00Z"}]}}),
        json!({"turns": [{"ts": "2026-09-18T06:00:00Z", "ts_other": "2026-10-01T00:00:00Z"}]}),
        json!({}),
        json!([]),
        json!([{"batch_id": "in a list"}]),
        json!("a string"),
        json!(12),
        json!(null),
        json!(true),
    ]
    .iter()
    .map(Value::to_string)
    .collect();
    // Keys that repeat: a decoded Value keeps the last, so must the lean reading.
    texts.push(
        r#"{"batch_id":"first","batch_id":"last","session_id":"s","session_id":7,
            "turns":[{"ts":"2026-09-18T06:00:00Z"}],"turns":[{"ts":"2026-09-01T00:00:00Z"}]}"#
            .to_owned(),
    );
    texts.push(
        r#"{"turns":[{"ts":"2026-09-18T06:00:00Z","ts":"2026-09-01T00:00:00Z"},
                     {"ts":"2026-09-02T00:00:00Z","ts":1}]}"#
            .to_owned(),
    );
    texts
}

#[test]
fn the_lean_reading_of_a_payload_agrees_with_the_decoded_one() {
    for text in payload_texts() {
        let decoded: Value = serde_json::from_str(&text).expect("valid JSON");
        let expected = ReceivedBatchFacts::from_event(&received(decoded)).expect("an event");
        let lean = ReceivedBatchFacts::from_payload_text(7, "agent:claude:hook".to_owned(), &text)
            .expect("a payload that is JSON");
        assert_eq!(lean, expected, "payload {text}");
    }
}

#[test]
fn facts_say_what_a_well_formed_batch_holds() {
    let text = json!({
        "batch_id": "b1", "session_id": "s1", "agent_tool": "claude",
        "turns": [{"ts": "2026-09-18T06:00:00Z"}, {"ts": "2026-09-19T08:00:00+02:00"}],
    })
    .to_string();
    let facts = ReceivedBatchFacts::from_payload_text(7, "human:alice".to_owned(), &text)
        .expect("a payload that is JSON");
    assert_eq!(facts.batch_id.as_deref(), Some("b1"));
    assert_eq!(facts.session_id.as_deref(), Some("s1"));
    assert_eq!(facts.agent_tool, "claude");
    assert_eq!(
        facts.newest_turn_time,
        Some(
            DateTime::parse_from_rfc3339("2026-09-19T06:00:00Z")
                .expect("a time")
                .with_timezone(&Utc)
        )
    );
}

#[test]
fn a_payload_that_is_not_json_is_refused() {
    let refused = ReceivedBatchFacts::from_payload_text(7, "a".to_owned(), "{not json");
    assert!(refused.is_err());
}

#[test]
fn only_a_received_batch_has_facts() {
    let mut event = received(json!({"batch_id": "b"}));
    event.event_type = EventType::EvidenceRecorded;
    assert_eq!(ReceivedBatchFacts::from_event(&event), None);
}
