// Parent module gates this file with #[cfg(test)]; repeat the marker so UBS can filter test-only assertions.
#[cfg(test)]
use super::*;

use std::cell::Cell;
use std::collections::HashSet;

use chrono::TimeZone;

use crate::events::Event;
use crate::ledger::InMemoryEventLedger;

fn batch_event(turns: serde_json::Value) -> crate::events::Event {
    crate::events::Event {
        tenant_id: Default::default(),
        event_id: None,
        event_uuid: uuid::Uuid::nil(),
        correlation_id: None,
        causation_event_id: None,
        event_type: crate::events::EventType::IngestBatchReceived,
        actor_id: "actor:test".to_owned(),
        source: Default::default(),
        source_ref: None,
        payload: serde_json::json!({ "batch_id": "b1", "turns": turns }),
        ts: None,
    }
}

// --- render_batch_text ---

#[test]
fn render_batch_text_no_turns_is_empty() {
    let event = batch_event(serde_json::json!([]));
    assert_eq!(render_batch_text(&event), "");
}

#[test]
fn render_batch_text_missing_turns_key_is_empty() {
    let event = crate::events::Event {
        tenant_id: Default::default(),
        event_id: None,
        event_uuid: uuid::Uuid::nil(),
        correlation_id: None,
        causation_event_id: None,
        event_type: crate::events::EventType::IngestBatchReceived,
        actor_id: "actor:test".to_owned(),
        source: Default::default(),
        source_ref: None,
        payload: serde_json::json!({ "batch_id": "b1" }),
        ts: None,
    };
    assert_eq!(render_batch_text(&event), "");
}

#[test]
fn render_batch_text_single_turn() {
    let event = batch_event(serde_json::json!([
        { "role": "user", "text": "hello", "truncated": false }
    ]));
    assert_eq!(render_batch_text(&event), "[user] hello\n");
}

#[test]
fn render_batch_text_truncated_appends_marker() {
    let event = batch_event(serde_json::json!([
        { "role": "assistant", "text": "partial response", "truncated": true }
    ]));
    assert_eq!(
        render_batch_text(&event),
        "[assistant] partial response [TRUNCATED]\n"
    );
}

#[test]
fn render_batch_text_multiple_turns_preserves_order() {
    let event = batch_event(serde_json::json!([
        { "role": "user", "text": "first", "truncated": false },
        { "role": "assistant", "text": "second", "truncated": false },
        { "role": "user", "text": "third", "truncated": false },
    ]));
    assert_eq!(
        render_batch_text(&event),
        "[user] first\n[assistant] second\n[user] third\n"
    );
}

#[test]
fn render_batch_text_names_each_turn_by_its_id() {
    let event = batch_event(serde_json::json!([
        { "turn_id": "t-1", "role": "assistant", "text": "which database?", "truncated": false },
        { "turn_id": "t-2", "role": "user", "text": "partial", "truncated": true },
    ]));
    assert_eq!(
        render_batch_text(&event),
        "[assistant turn t-1] which database?\n[user turn t-2] partial [TRUNCATED]\n"
    );
    assert_eq!(turn_ids(&event), vec!["t-1", "t-2"]);
}

#[test]
fn render_batch_text_unknown_role_defaults_to_unknown() {
    let event = batch_event(serde_json::json!([
        { "text": "no role field" }
    ]));
    assert_eq!(render_batch_text(&event), "[unknown] no role field\n");
}

/// A received batch from `actor_id`, shipped by `agent_tool`, with one user turn.
fn batch_event_from(actor_id: &str, agent_tool: &str, turns: serde_json::Value) -> Event {
    let mut event = batch_event(turns);
    event.actor_id = actor_id.to_owned();
    event.payload["agent_tool"] = serde_json::json!(agent_tool);
    event
}

#[test]
fn render_batch_text_opens_with_the_session_agent_the_projector_credits() {
    let turns = serde_json::json!([{ "role": "user", "text": "hello", "truncated": false }]);

    let own_identity = batch_event_from("agent:claude:gastown.furiosa", "claude", turns.clone());
    assert_eq!(
        render_batch_text(&own_identity),
        "Session agent: agent:claude:gastown.furiosa\n\n[user] hello\n"
    );

    let beside_a_human = batch_event_from("human:alex@example.com", "claude", turns.clone());
    assert_eq!(
        render_batch_text(&beside_a_human),
        "Session agent: agent:claude:hook\n\n[user] hello\n"
    );

    let no_agent_named = batch_event_from("human:alex@example.com", "", turns);
    assert_eq!(render_batch_text(&no_agent_named), "[user] hello\n");
}

#[test]
fn render_batch_text_of_a_batch_with_no_turns_names_no_session_agent() {
    let event = batch_event_from(
        "agent:claude:gastown.furiosa",
        "claude",
        serde_json::json!([]),
    );
    assert_eq!(render_batch_text(&event), "");
}

// --- pending-batch filter ---

#[test]
fn pending_batch_filter_excludes_classified_ids() {
    let received = vec![
        PendingBatch {
            batch_id: "a".to_owned(),
            submitted_at: None,
            actor_id: "actor:test".to_owned(),
            turn_count: 1,
            session_id: String::new(),
            agent_tool: String::new(),
            batch_text: String::new(),
        },
        PendingBatch {
            batch_id: "b".to_owned(),
            submitted_at: None,
            actor_id: "actor:test".to_owned(),
            turn_count: 2,
            session_id: String::new(),
            agent_tool: String::new(),
            batch_text: String::new(),
        },
        PendingBatch {
            batch_id: "c".to_owned(),
            submitted_at: None,
            actor_id: "actor:test".to_owned(),
            turn_count: 3,
            session_id: String::new(),
            agent_tool: String::new(),
            batch_text: String::new(),
        },
    ];
    let mut classified: HashSet<String> = HashSet::new();
    classified.insert("b".to_owned());

    let pending: Vec<_> = received
        .into_iter()
        .filter(|b| !classified.contains(&b.batch_id))
        .collect();

    assert_eq!(pending.len(), 2);
    assert_eq!(pending[0].batch_id, "a");
    assert_eq!(pending[1].batch_id, "c");
}

#[test]
fn pending_batch_filter_all_classified_yields_empty() {
    let received = vec![PendingBatch {
        batch_id: "x".to_owned(),
        submitted_at: None,
        actor_id: "actor:test".to_owned(),
        turn_count: 0,
        session_id: String::new(),
        agent_tool: String::new(),
        batch_text: String::new(),
    }];
    let mut classified: HashSet<String> = HashSet::new();
    classified.insert("x".to_owned());

    let pending: Vec<_> = received
        .into_iter()
        .filter(|b| !classified.contains(&b.batch_id))
        .collect();

    assert!(pending.is_empty());
}

// --- classifier model override ---

#[test]
fn resolve_classifier_model_defaults_when_no_override() {
    assert_eq!(resolve_classifier_model(None), CLASSIFIER_MODEL);
}

#[test]
fn resolve_classifier_model_uses_override_when_present() {
    assert_eq!(
        resolve_classifier_model(Some("claude-sonnet-5".to_owned())),
        "claude-sonnet-5"
    );
}

#[test]
fn pending_batch_filter_no_classified_returns_all() {
    let received = vec![
        PendingBatch {
            batch_id: "p".to_owned(),
            submitted_at: None,
            actor_id: "actor:test".to_owned(),
            turn_count: 5,
            session_id: String::new(),
            agent_tool: String::new(),
            batch_text: String::new(),
        },
        PendingBatch {
            batch_id: "q".to_owned(),
            submitted_at: None,
            actor_id: "actor:test".to_owned(),
            turn_count: 5,
            session_id: String::new(),
            agent_tool: String::new(),
            batch_text: String::new(),
        },
    ];
    let classified: HashSet<String> = HashSet::new();

    let pending: Vec<_> = received
        .into_iter()
        .filter(|b| !classified.contains(&b.batch_id))
        .collect();

    assert_eq!(pending.len(), 2);
}

// --- source turn and question (hivemind-bbnw.8) ---

fn parsed_capture(extra: serde_json::Value) -> CaptureItem {
    let mut capture = serde_json::json!({
        "kind": "decision",
        "title": "Use Postgres for the hosted MVP",
        "rationale": "Several friends write at once",
        "topic_keys": ["hosted-mvp"],
        "evidence_ids": [],
        "options": null,
        "chosen_option": null,
        "extraction_confidence": 0.9,
        "expressed_confidence": null,
        "supersedes_id": null,
        "actor_id": null,
        "blocked_actor_id": null,
        "decision_id": null,
    });
    capture
        .as_object_mut()
        .expect("capture is an object")
        .extend(extra.as_object().expect("extra is an object").clone());
    let response = serde_json::json!({ "captures": [capture] }).to_string();
    parse_capture_response(&response)
        .expect("response parses")
        .remove(0)
}

#[test]
fn the_schema_asks_for_a_source_turn_and_a_question_on_every_capture() {
    let schema = capture_schema();
    let items = &schema["properties"]["captures"]["items"];
    for field in ["source_turn_id", "question"] {
        assert!(
            items["properties"].get(field).is_some(),
            "{field} is a property"
        );
        assert!(
            items["required"]
                .as_array()
                .expect("required list")
                .iter()
                .any(|name| name == field),
            "{field} is required (nullable) under structured outputs"
        );
    }
}

#[test]
fn a_response_without_the_new_fields_still_parses() {
    let capture = parsed_capture(serde_json::json!({}));
    assert_eq!(capture.source_turn_id, None);
    assert_eq!(capture.question, None);
}

#[test]
fn the_evaluator_path_keeps_the_question_but_no_turn() {
    // No batch, so no turn the model could honestly name.
    let capture = parsed_capture(serde_json::json!({
        "source_turn_id": "t-1",
        "question": "Which database should the hosted MVP use?"
    }));
    assert_eq!(capture.source_turn_id, None);
    assert_eq!(
        capture.question.as_deref(),
        Some("Which database should the hosted MVP use?")
    );
}

#[test]
fn a_turn_the_batch_does_not_hold_is_dropped_before_submission() {
    let mut named = parsed_capture(serde_json::json!({}));
    named.source_turn_id = Some("t-2".to_owned());
    let mut invented = parsed_capture(serde_json::json!({}));
    invented.source_turn_id = Some("t-99".to_owned());
    let mut captures = vec![named, invented];

    conform_to_batch(&mut captures, &["t-1".to_owned(), "t-2".to_owned()]);

    assert_eq!(captures[0].source_turn_id.as_deref(), Some("t-2"));
    assert_eq!(captures[1].source_turn_id, None);
}

#[test]
fn a_question_the_write_path_would_refuse_is_dropped_before_submission() {
    let mut kept = parsed_capture(serde_json::json!({}));
    kept.question = Some("Which database should the hosted MVP use?".to_owned());
    let mut wordless = parsed_capture(serde_json::json!({}));
    wordless.question = Some(" ? ".to_owned());
    let mut on_evidence = parsed_capture(serde_json::json!({}));
    on_evidence.kind = "evidence".to_owned();
    on_evidence.question = Some("Which database?".to_owned());
    let mut captures = vec![kept, wordless, on_evidence];

    conform_to_batch(&mut captures, &[]);

    assert!(captures[0].question.is_some());
    assert_eq!(captures[1].question, None);
    assert_eq!(captures[2].question, None);
}

// --- who decided (hivemind-u70b, hivemind-zdsh.23) ---

/// The words the two classify workers share about who decided: the server-side prompt and the
/// agent-seat command (Worker A), which is read from the plugin so a drift in either shows here.
/// The town runner of the cell (`hivemind-classify-runner.py`) takes the same words by hand.
const DECIDER_RULE: &[&str] = &[
    "a person stating the choice themselves",
    "list priya, not whoever reports it",
    "a proposal nobody has accepted stays empty however firmly it is worded",
    "the one agent that decides for itself is the session agent",
    "making a call that nobody else made",
    "list that id, copied exactly,",
    "a call a person or another actor made is theirs, not the session agent's",
    "a recommendation, a suggestion or a proposal that waits for someone",
    "only narrates or carries out its own choice is proposing it",
    "session agent: <id>",
];

fn one_line(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[test]
fn both_classify_workers_carry_the_same_rule_for_who_decided() {
    let command = include_str!("../../plugins/loomtracer-capture/commands/classify-queue.md");
    for (worker, text) in [
        ("the server-side prompt", one_line(CLASSIFIER_PROMPT)),
        ("the classify-queue command", one_line(command)),
    ] {
        for phrase in DECIDER_RULE {
            assert!(text.contains(phrase), "{worker} lost {phrase:?}");
        }
    }
}

// --- the decision test and the retired decision-request (hivemind-bbnw.12) ---

/// The paragraph that opens "Capture a decision only when", up to the next blank line, with
/// whitespace and case flattened so the same words wrapped or indented differently compare equal.
fn decision_test_paragraph(prompt: &str) -> String {
    let mut lines = prompt
        .lines()
        .skip_while(|line| !line.contains("Capture a decision only when"));
    let paragraph: Vec<&str> = lines
        .by_ref()
        .take_while(|line| !line.trim().is_empty())
        .collect();
    assert!(!paragraph.is_empty(), "no decision paragraph in the prompt");
    one_line(&paragraph.join(" "))
}

/// What `scripts/e2e_smoke.sh` cuts out of SKILL.md with awk: from the line that ends the
/// classifier's opening sentence to the line that ends its output rule.
fn cut_by_the_e2e_sentinels(skill: &str) -> String {
    let mut cut = Vec::new();
    let mut inside = false;
    for line in skill.lines() {
        if line.ends_with("You are the Loomtracer capture classifier.") {
            inside = true;
        }
        if inside {
            cut.push(line);
        }
        if line.ends_with("Return only the JSON array, no other text.") {
            break;
        }
    }
    cut.join("\n")
}

#[test]
fn the_decision_test_reads_the_same_in_every_copy_of_the_prompt() {
    let skill = include_str!("../../plugins/loomtracer-capture/skills/loomtracer-capture/SKILL.md");
    let batch_capture = include_str!("../../plugins/loomtracer-capture/commands/batch-capture.md");
    let doc = include_str!("../../docs/CAPTURE_CLASSIFIER.md");
    let e2e = include_str!("../../scripts/e2e_smoke.sh");
    assert!(
        e2e.contains("You are the Loomtracer capture classifier\\.$")
            && e2e.contains("Return only the JSON array, no other text\\.$"),
        "scripts/e2e_smoke.sh cuts the prompt out of SKILL.md by these two sentinels"
    );

    let server = decision_test_paragraph(CLASSIFIER_PROMPT);
    for needle in [
        "binds something beyond the edit",
        "\"apple\" over \"apple\"",
        "merging tonight",
        "a request goes to your tracker",
    ] {
        assert!(server.contains(needle), "the server prompt lost {needle:?}");
    }
    for (copy, text) in [
        ("docs/CAPTURE_CLASSIFIER.md", doc.to_owned()),
        (
            "SKILL.md as the e2e smoke cuts it",
            cut_by_the_e2e_sentinels(skill),
        ),
        ("commands/batch-capture.md", batch_capture.to_owned()),
    ] {
        assert_eq!(
            decision_test_paragraph(&text),
            server,
            "{copy} drifted from the server-side prompt"
        );
    }
}

#[test]
fn the_classifier_no_longer_produces_a_decision_request() {
    let schema = capture_schema();
    let kinds = schema["properties"]["captures"]["items"]["properties"]["kind"]["enum"]
        .as_array()
        .expect("kind enum");
    assert!(kinds.iter().all(|kind| kind != "decision-request"));
    assert_eq!(
        kinds.len(),
        5,
        "decision, evidence, hypothesis, blocker, notification"
    );

    let prompt = one_line(CLASSIFIER_PROMPT);
    assert!(!prompt.contains("decision-request") && !prompt.contains("decision request"));

    // Every place that names the kinds an agent may submit agrees: the plugin copies, the
    // helper's classifier enum and the keyless commands.
    for (copy, text) in [
        (
            "SKILL.md",
            include_str!("../../plugins/loomtracer-capture/skills/loomtracer-capture/SKILL.md"),
        ),
        (
            "batch-capture.md",
            include_str!("../../plugins/loomtracer-capture/commands/batch-capture.md"),
        ),
        (
            "classify-queue.md",
            include_str!("../../plugins/loomtracer-capture/commands/classify-queue.md"),
        ),
        (
            "capture.sh",
            include_str!("../../plugins/loomtracer-capture/scripts/capture.sh"),
        ),
    ] {
        assert!(
            !text.contains("\"decision-request\"") && !text.contains("decision-request,"),
            "{copy} still offers decision-request as a kind to produce"
        );
    }
}

// --- the pending queue's reads (hivemind-t15t) ---

fn queue_event(event_type: EventType, payload: serde_json::Value, second: u32) -> Event {
    Event {
        tenant_id: Default::default(),
        event_id: None,
        event_uuid: uuid::Uuid::new_v4(),
        correlation_id: None,
        causation_event_id: None,
        event_type,
        actor_id: "agent:test:queue".to_owned(),
        source: Default::default(),
        source_ref: None,
        payload,
        ts: chrono::Utc
            .with_ymd_and_hms(2026, 10, 1, 12, second / 60, second % 60)
            .single(),
    }
}

fn append_received(
    ledger: &impl EventLedger,
    batch_id: &str,
    session_id: &str,
    actor_id: &str,
    second: u32,
) {
    let mut event = queue_event(
        EventType::IngestBatchReceived,
        serde_json::json!({
            "batch_id": batch_id,
            "session_id": session_id,
            "agent_tool": "claude",
            "turns": [{ "turn_id": "t1", "role": "user", "text": format!("text of {batch_id}"),
                        "truncated": false }],
        }),
        second,
    );
    event.actor_id = actor_id.to_owned();
    ledger
        .append_for_tenant(&TenantId::local(), event)
        .expect("append received batch");
}

fn append_classified(ledger: &impl EventLedger, batch_ids: &[String], second: u32) {
    let event = queue_event(
        EventType::IngestBatchClassified,
        serde_json::json!({
            "batch_id": batch_ids.first(),
            "batch_ids": batch_ids,
            "captures": [{ "kind": "decision", "title": "a capture the queue never reads" }],
        }),
        second,
    );
    ledger
        .append_for_tenant(&TenantId::local(), event)
        .expect("append classification");
}

/// A ledger that counts what the queue's reads cost: how many reads, and how many events came
/// back still carrying their turns.
struct CountingLedger {
    inner: InMemoryEventLedger,
    reads: Cell<usize>,
    bodies_read: Cell<usize>,
}

impl CountingLedger {
    fn new() -> Self {
        Self {
            inner: InMemoryEventLedger::new(),
            reads: Cell::new(0),
            bodies_read: Cell::new(0),
        }
    }

    fn note(&self, events: &[Event]) {
        self.reads.set(self.reads.get() + 1);
        self.bodies_read.set(
            self.bodies_read.get()
                + events
                    .iter()
                    .filter(|event| event.payload.get("turns").is_some())
                    .count(),
        );
    }

    fn reset(&self) {
        self.reads.set(0);
        self.bodies_read.set(0);
    }
}

impl EventLedger for CountingLedger {
    fn bound_tenant(&self) -> TenantId {
        self.inner.bound_tenant()
    }

    fn append_for_tenant(&self, tenant_id: &TenantId, event: Event) -> crate::Result<EventId> {
        self.inner.append_for_tenant(tenant_id, event)
    }

    fn read_for_tenant(
        &self,
        tenant_id: &TenantId,
        offset: EventId,
        limit: usize,
    ) -> crate::Result<Vec<Event>> {
        let events = self.inner.read_for_tenant(tenant_id, offset, limit)?;
        self.note(&events);
        Ok(events)
    }

    fn replay_from_for_tenant(
        &self,
        tenant_id: &TenantId,
        offset: EventId,
        callback: &mut dyn FnMut(&Event) -> crate::Result<()>,
    ) -> crate::Result<()> {
        self.inner
            .replay_from_for_tenant(tenant_id, offset, callback)
    }

    fn latest_offset_for_tenant(&self, tenant_id: &TenantId) -> crate::Result<EventId> {
        self.inner.latest_offset_for_tenant(tenant_id)
    }

    fn read_types_for_tenant(
        &self,
        tenant_id: &TenantId,
        types: &[EventType],
        omit_payload_keys: &[&str],
        offset: EventId,
        limit: usize,
    ) -> crate::Result<Vec<Event>> {
        let events =
            self.inner
                .read_types_for_tenant(tenant_id, types, omit_payload_keys, offset, limit)?;
        self.note(&events);
        Ok(events)
    }

    fn read_fields_for_tenant(
        &self,
        tenant_id: &TenantId,
        event_type: EventType,
        payload_keys: &[&str],
        offset: EventId,
        limit: usize,
    ) -> crate::Result<Vec<crate::ledger::EventFields>> {
        self.reads.set(self.reads.get() + 1);
        self.inner
            .read_fields_for_tenant(tenant_id, event_type, payload_keys, offset, limit)
    }

    fn read_ids_for_tenant(
        &self,
        tenant_id: &TenantId,
        event_ids: &[EventId],
    ) -> crate::Result<Vec<Event>> {
        let events = self.inner.read_ids_for_tenant(tenant_id, event_ids)?;
        self.note(&events);
        Ok(events)
    }
}

/// `received` batches over 20 sessions, the first `classified` of them covered by classifications
/// of 100 batches each: `received - classified` stay pending.
fn queue_with_history(received: usize, classified: usize) -> CountingLedger {
    let ledger = CountingLedger::new();
    let ids: Vec<String> = (0..received).map(|n| format!("batch-{n}")).collect();
    for (n, batch_id) in ids.iter().enumerate() {
        append_received(
            &ledger,
            batch_id,
            &format!("session-{}", n % 20),
            "agent:test:queue",
            u32::try_from(n % 3000).unwrap_or(0),
        );
    }
    for covered in ids
        .iter()
        .take(classified)
        .cloned()
        .collect::<Vec<_>>()
        .chunks(100)
    {
        append_classified(&ledger, covered, 0);
    }
    ledger
}

#[test]
fn listing_the_pending_queue_never_reads_the_text_of_classified_history() {
    let measure = |received: usize, classified: usize| {
        let ledger = queue_with_history(received, classified);
        ledger.reset();
        let page = list_pending_batches_for_ledger(&ledger, &TenantId::local(), None, 10)
            .expect("list pending");
        (
            page.pending_total,
            page.batches.len(),
            ledger.reads.get(),
            ledger.bodies_read.get(),
        )
    };

    // Ten times the history, the same cost: three reads (classifications, received heads, the
    // page's bodies), and the turns of only the ten batches handed back.
    assert_eq!(measure(300, 250), (50, 10, 3, 10));
    assert_eq!(measure(3000, 2950), (50, 10, 3, 10));
}

#[test]
fn a_queue_longer_than_one_read_page_is_read_whole() {
    let ledger = InMemoryEventLedger::new();
    let received = QUEUE_READ_PAGE + 50;
    for n in 0..received {
        append_received(
            &ledger,
            &format!("batch-{n}"),
            "session-x",
            "agent:test:queue",
            0,
        );
    }
    // One classification per batch: more classifications than one read page, too.
    for n in 0..(received - 30) {
        append_classified(&ledger, &[format!("batch-{n}")], 0);
    }

    let page = list_pending_batches_for_ledger(&ledger, &TenantId::local(), None, 100)
        .expect("list pending");

    assert_eq!(page.pending_total, 30);
    assert!(!page.truncated);
    let ids: Vec<&str> = page.batches.iter().map(|b| b.batch_id.as_str()).collect();
    let expected: Vec<String> = ((received - 30)..received)
        .map(|n| format!("batch-{n}"))
        .collect();
    assert_eq!(ids, expected.iter().map(String::as_str).collect::<Vec<_>>());
}

#[test]
fn listing_pending_batches_filters_by_session_and_says_when_it_stopped_short() {
    let ledger = InMemoryEventLedger::new();
    append_received(&ledger, "a1", "session-a", "agent:test:queue", 1);
    append_received(&ledger, "b1", "session-b", "agent:test:queue", 2);
    append_received(&ledger, "a2", "session-a", "agent:test:queue", 3);
    append_received(&ledger, "a3", "session-a", "agent:test:queue", 4);
    append_classified(&ledger, &["a2".to_owned()], 5);

    let everything =
        list_pending_batches_for_ledger(&ledger, &TenantId::local(), None, 20).expect("list");
    let ids: Vec<&str> = everything
        .batches
        .iter()
        .map(|b| b.batch_id.as_str())
        .collect();
    assert_eq!(ids, ["a1", "b1", "a3"]);
    assert_eq!(everything.pending_total, 3);
    assert!(!everything.truncated);
    assert!(everything.batches[0].batch_text.contains("text of a1"));
    assert_eq!(everything.batches[0].turn_count, 1);
    assert_eq!(everything.batches[0].session_id, "session-a");
    assert_eq!(everything.batches[0].agent_tool, "claude");

    let session_a =
        list_pending_batches_for_ledger(&ledger, &TenantId::local(), Some("session-a"), 1)
            .expect("list session a");
    let ids: Vec<&str> = session_a
        .batches
        .iter()
        .map(|b| b.batch_id.as_str())
        .collect();
    assert_eq!(ids, ["a1"], "the oldest of the session comes first");
    assert_eq!(
        session_a.pending_total, 2,
        "the total counts the session's batches, not the limit"
    );
    assert!(session_a.truncated);

    let none = list_pending_batches_for_ledger(&ledger, &TenantId::local(), Some("session-z"), 20)
        .expect("list unknown session");
    assert!(none.batches.is_empty());
    assert_eq!(none.pending_total, 0);
    assert!(!none.truncated);
}

#[test]
fn pending_sessions_are_summarised_newest_first_without_turn_text() {
    let ledger = InMemoryEventLedger::new();
    append_received(&ledger, "a1", "session-a", "agent:test:one", 10);
    append_received(&ledger, "a2", "session-a", "agent:test:one", 30);
    append_received(&ledger, "a3", "session-a", "agent:test:one", 20);
    append_received(&ledger, "b1", "session-b", "agent:test:two", 40);
    append_received(&ledger, "c1", "session-c", "agent:test:three", 5);
    append_received(&ledger, "c2", "session-c", "agent:test:three", 6);
    append_received(&ledger, "d1", "session-d", "agent:test:four", 50);
    // session-c has one batch left; session-d has none.
    append_classified(&ledger, &["c1".to_owned(), "d1".to_owned()], 60);

    let page = list_pending_sessions_for_ledger(&ledger, &TenantId::local(), 10).expect("sessions");

    assert_eq!(page.batch_total, 5);
    assert_eq!(page.session_total, 3);
    assert!(!page.truncated);
    let summary: Vec<(&str, &str, usize)> = page
        .sessions
        .iter()
        .map(|s| (s.session_id.as_str(), s.actor_id.as_str(), s.batch_count))
        .collect();
    assert_eq!(
        summary,
        [
            ("session-b", "agent:test:two", 1),
            ("session-a", "agent:test:one", 3),
            ("session-c", "agent:test:three", 1),
        ]
    );
    let session_a = &page.sessions[1];
    assert_eq!(
        session_a.oldest_submitted_at,
        queue_event(EventType::IngestBatchReceived, serde_json::json!({}), 10).ts
    );
    assert_eq!(
        session_a.newest_submitted_at,
        queue_event(EventType::IngestBatchReceived, serde_json::json!({}), 30).ts
    );

    let json = serde_json::to_value(&page).expect("page serialises");
    assert!(
        !json.to_string().contains("text of"),
        "a session summary carries no turn text: {json}"
    );

    let first_only =
        list_pending_sessions_for_ledger(&ledger, &TenantId::local(), 1).expect("limit 1");
    assert_eq!(first_only.sessions.len(), 1);
    assert_eq!(first_only.session_total, 3);
    assert!(first_only.truncated);
}

#[test]
fn the_daily_budget_counts_only_todays_classifications() {
    let ledger = InMemoryEventLedger::new();
    let mut yesterday = queue_event(
        EventType::IngestBatchClassified,
        serde_json::json!({ "batch_ids": ["old"], "captures": [] }),
        0,
    );
    yesterday.ts = Some(chrono::Utc::now() - chrono::Duration::days(1));
    ledger
        .append_for_tenant(&TenantId::local(), yesterday)
        .expect("append yesterday's classification");
    let mut today = queue_event(
        EventType::IngestBatchClassified,
        serde_json::json!({ "batch_ids": ["new"], "captures": [] }),
        0,
    );
    today.ts = Some(chrono::Utc::now());
    ledger
        .append_for_tenant(&TenantId::local(), today)
        .expect("append today's classification");
    append_received(&ledger, "unrelated", "session-a", "agent:test:queue", 0);

    let status = daily_cap_status(&ledger, &TenantId::local()).expect("budget");

    assert_eq!(status.classified_today, 1);
}

// --- Worker B's pass: the daily cap and the batches that fail (hivemind-49ddt) ---

/// Stands in for the model: records which batches it was asked about and fails those it is told
/// to. The `model` it answers with is what the write path records.
struct ScriptedModel {
    failing: HashSet<&'static str>,
    model: &'static str,
    asked: std::sync::Mutex<Vec<String>>,
}

impl ScriptedModel {
    fn new(failing: &[&'static str]) -> Self {
        Self {
            failing: failing.iter().copied().collect(),
            model: "claude-haiku-4-5-20251001",
            asked: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().expect("asked lock").clone()
    }
}

impl BatchClassifier for ScriptedModel {
    async fn classify(
        &self,
        batch: &BatchInfo,
    ) -> Result<ClassifiedBatch, crate::anthropic::BoxError> {
        self.asked
            .lock()
            .expect("asked lock")
            .push(batch.batch_id.clone());
        if self.failing.contains(batch.batch_id.as_str()) {
            return Err(
                "Anthropic API stopped at max_tokens (1200): the output was cut off".into(),
            );
        }
        Ok(ClassifiedBatch {
            model: self.model.to_owned(),
            captures: Vec::new(),
        })
    }
}

/// A folder whose SQLite ledger holds these received batches, oldest first.
fn dir_with_received(batch_ids: &[&str]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let ledger = SqliteEventLedger::open(dir.path()).expect("open ledger");
    for (second, batch_id) in batch_ids.iter().enumerate() {
        append_received(
            &ledger,
            batch_id,
            "session-a",
            "agent:test:queue",
            second as u32,
        );
    }
    dir
}

/// A classification another worker recorded just now, covering `batch_id`.
fn append_classified_now(dir: &tempfile::TempDir, batch_id: &str) {
    let ledger = SqliteEventLedger::open(dir.path()).expect("open ledger");
    let mut event = queue_event(
        EventType::IngestBatchClassified,
        serde_json::json!({ "batch_id": batch_id, "batch_ids": [batch_id], "captures": [] }),
        0,
    );
    event.ts = Some(chrono::Utc::now());
    ledger
        .append_for_tenant(&TenantId::local(), event)
        .expect("append classification");
}

async fn pass(model: &ScriptedModel, dir: &tempfile::TempDir, cap: usize, state: &mut PassState) {
    classify_pending_batches(
        model,
        &dir.path().to_path_buf(),
        &TenantId::local(),
        cap,
        state,
    )
    .await;
}

fn pending_ids(dir: &tempfile::TempDir) -> Vec<String> {
    let ledger = SqliteEventLedger::open(dir.path()).expect("open ledger");
    list_pending_batches_for_ledger(&ledger, &TenantId::local(), None, 100)
        .expect("pending batches")
        .batches
        .into_iter()
        .map(|batch| batch.batch_id)
        .collect()
}

#[tokio::test]
async fn worker_b_stops_calling_the_model_when_the_daily_cap_is_spent() {
    let dir = dir_with_received(&["b1", "b2", "b3"]);
    let model = ScriptedModel::new(&[]);
    let mut state = PassState::default();

    pass(&model, &dir, 2, &mut state).await;
    pass(&model, &dir, 2, &mut state).await;

    assert_eq!(
        model.asked(),
        ["b1", "b2"],
        "two calls the cap allows, and none after it is spent"
    );
    assert_eq!(pending_ids(&dir), ["b3"], "the batch past the cap waits");
    assert!(state.cap_said, "the spent cap is said once");
}

#[tokio::test]
async fn worker_b_counts_the_classifications_other_workers_recorded_today() {
    let dir = dir_with_received(&["b1", "b2"]);
    append_classified_now(&dir, "elsewhere");
    let model = ScriptedModel::new(&[]);

    pass(&model, &dir, 2, &mut PassState::default()).await;

    assert_eq!(
        model.asked(),
        ["b1"],
        "one of the two calls was already spent"
    );
}

#[tokio::test]
async fn a_batch_whose_call_failed_is_not_sent_again_the_same_day() {
    let dir = dir_with_received(&["b1", "b2"]);
    let model = ScriptedModel::new(&["b1"]);
    let mut state = PassState::default();

    pass(&model, &dir, 150, &mut state).await;
    pass(&model, &dir, 150, &mut state).await;
    pass(&model, &dir, 150, &mut state).await;

    assert_eq!(
        model.asked(),
        ["b1", "b2"],
        "the failing batch costs one call, and does not hold up the batch behind it"
    );
    assert_eq!(pending_ids(&dir), ["b1"], "it stays pending in the ledger");
}

#[tokio::test]
async fn a_failed_call_spends_a_call_of_the_cap() {
    let dir = dir_with_received(&["b1", "b2", "b3"]);
    let model = ScriptedModel::new(&["b1", "b2"]);
    let mut state = PassState::default();

    pass(&model, &dir, 2, &mut state).await;
    pass(&model, &dir, 2, &mut state).await;

    assert_eq!(
        model.asked(),
        ["b1", "b2"],
        "two failed calls are two calls: the third batch waits for tomorrow"
    );
}

#[tokio::test]
async fn a_write_the_ledger_refuses_is_not_repeated() {
    let dir = dir_with_received(&["b1"]);
    let mut model = ScriptedModel::new(&[]);
    model.model = "";
    let mut state = PassState::default();

    pass(&model, &dir, 150, &mut state).await;
    pass(&model, &dir, 150, &mut state).await;

    assert_eq!(
        model.asked(),
        ["b1"],
        "the refused write is not paid for twice"
    );
    assert_eq!(pending_ids(&dir), ["b1"]);
}

#[tokio::test]
async fn yesterdays_failures_are_forgotten_when_a_new_day_starts() {
    let dir = dir_with_received(&["b1"]);
    let model = ScriptedModel::new(&[]);
    let first_event = {
        let ledger = SqliteEventLedger::open(dir.path()).expect("open ledger");
        pending_batch_heads(&ledger, &TenantId::local()).expect("heads")[0].event_id
    };
    let mut state = PassState {
        day: Some(chrono::Utc::now().date_naive() - chrono::Duration::days(1)),
        failed: HashSet::from([first_event]),
        cap_said: true,
    };

    pass(&model, &dir, 150, &mut state).await;

    assert_eq!(
        model.asked(),
        ["b1"],
        "the batch is tried again on the new day"
    );
    assert!(state.failed.is_empty() && !state.cap_said);
}
