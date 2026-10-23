// Parent module gates this file with #[cfg(test)]; repeat the marker so UBS can filter test-only assertions.
#[cfg(test)]
use super::*;
use crate::commands::{DecisionProposalInput, FloorInput, Grounding};
use crate::events::{CaptureItem, ModelDimension, QualityLevel};
use crate::floor::{BindsAnswer, FloorAnswers, UndoCostAnswer};
use crate::ledger::InMemoryEventLedger;

fn unique_test_dir(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "loomtracer-scorer-test-{name}-{}",
        uuid::Uuid::new_v4()
    ))
}

// --- scorer model override ---

#[test]
fn resolve_scorer_model_defaults_when_no_override() {
    assert_eq!(resolve_scorer_model(None), SCORER_MODEL);
}

#[test]
fn resolve_scorer_model_uses_override_when_present() {
    assert_eq!(
        resolve_scorer_model(Some("claude-sonnet-5".to_owned())),
        "claude-sonnet-5"
    );
}

// --- render_decision_text ---

#[test]
fn render_decision_text_empty_capture_yields_empty_string() {
    assert_eq!(render_decision_text(&serde_json::json!({})), "");
}

#[test]
fn render_decision_text_all_fields() {
    let capture = serde_json::json!({
        "title": "Use SQLite",
        "rationale": "Simple and embeddable",
        "options": ["SQLite", "PostgreSQL"],
        "chosen_option": "SQLite",
        "expressed_confidence": "high",
        "topic_keys": ["storage", "database"]
    });
    let text = render_decision_text(&capture);
    assert!(text.contains("Title: Use SQLite\n"));
    assert!(text.contains("Rationale: Simple and embeddable\n"));
    assert!(text.contains("Option: SQLite\n"));
    assert!(text.contains("Option: PostgreSQL\n"));
    assert!(text.contains("Chosen option: SQLite\n"));
    assert!(text.contains("Recorded context, expressed confidence: high\n"));
    assert!(text.contains("Recorded context, topic keys: storage, database\n"));
}

#[test]
fn render_decision_text_omits_absent_optional_fields() {
    let capture = serde_json::json!({
        "title": "Use caching",
        "rationale": "Reduces latency"
    });
    let text = render_decision_text(&capture);
    assert!(text.contains("Title: Use caching\n"));
    assert!(text.contains("Rationale: Reduces latency\n"));
    assert!(!text.contains("Option"));
    assert!(!text.contains("Chosen option"));
    assert!(!text.contains("Recorded context"));
}

#[test]
fn render_decision_text_empty_options_array_omitted() {
    let capture = serde_json::json!({
        "title": "Deploy now",
        "options": []
    });
    let text = render_decision_text(&capture);
    assert!(!text.contains("Option"));
}

// --- render_proposed_decision_text ---

#[test]
fn render_proposed_decision_text_empty_payload_yields_empty_string() {
    assert_eq!(render_proposed_decision_text(&serde_json::json!({})), "");
}

#[test]
fn render_proposed_decision_text_all_fields() {
    let payload = serde_json::json!({
        "title": "Pick queue",
        "question": "Which queue should we use?",
        "quote": "Use the async queue",
        "rationale": "Durable ingestion under load",
        "option_labels": ["sync", "async"],
        "option_descriptions": ["Blocking", "Non-blocking"]
    });
    let text = render_proposed_decision_text(&payload);
    assert!(text.contains("Title: Pick queue\n"));
    assert!(text.contains("Question: Which queue should we use?\n"));
    assert!(text.contains("Quote: Use the async queue\n"));
    assert!(text.contains("Rationale: Durable ingestion under load\n"));
    assert!(text.contains("Option: sync\n"));
    assert!(text.contains("Option description: Blocking\n"));
    assert!(text.contains("Option: async\n"));
    assert!(text.contains("Option description: Non-blocking\n"));
}

#[test]
fn render_proposed_decision_text_omits_absent_optional_fields() {
    let payload = serde_json::json!({
        "title": "Pick queue",
        "rationale": "Durable ingestion under load"
    });
    let text = render_proposed_decision_text(&payload);
    assert!(text.contains("Title: Pick queue\n"));
    assert!(text.contains("Rationale: Durable ingestion under load\n"));
    assert!(!text.contains("Question"));
    assert!(!text.contains("Quote"));
    assert!(!text.contains("Option"));
}

#[test]
fn render_proposed_decision_text_option_without_description_omits_description_line() {
    let payload = serde_json::json!({
        "title": "Pick queue",
        "option_labels": ["sync"]
    });
    let text = render_proposed_decision_text(&payload);
    assert!(text.contains("Option: sync\n"));
    assert!(!text.contains("Option description"));
}

// --- the decision floor's answers as input facts (hivemind-bbnw.11) ---

#[test]
fn render_proposed_decision_text_carries_both_recorded_answers_as_input_facts() {
    let payload = serde_json::json!({
        "title": "Rename the checker",
        "rationale": "The old name misleads",
        "binds": {"level": "something", "statement": "Users and scripts that call the old name"},
        "undo_cost": {"level": "costly", "statement": "A release cannot be unpublished"}
    });
    let text = render_proposed_decision_text(&payload);
    assert!(
        text.contains(
            "Recorded answer, binds: something (Users and scripts that call the old name)\n"
        ),
        "{text}"
    );
    assert!(
        text.contains("Recorded answer, undo cost: costly (A release cannot be unpublished)\n"),
        "{text}"
    );

    let apple = serde_json::json!({
        "title": "Type apple",
        "rationale": "Either spelling works",
        "binds": {"level": "nothing"},
        "undo_cost": {"level": "cheap"}
    });
    let text = render_proposed_decision_text(&apple);
    assert!(text.contains("Recorded answer, binds: nothing\n"), "{text}");
    assert!(
        text.contains("Recorded answer, undo cost: cheap\n"),
        "{text}"
    );
}

#[test]
fn render_proposed_decision_text_adds_no_answer_line_for_a_decision_never_asked() {
    let payload = serde_json::json!({
        "title": "Pick queue",
        "rationale": "Durable ingestion under load"
    });
    let text = render_proposed_decision_text(&payload);
    assert!(!text.contains("Recorded answer"), "{text}");
}

#[test]
fn the_prompt_says_the_recorded_answers_and_context_are_input_facts_and_not_quotable() {
    assert!(ASSESSOR_PROMPT.contains("Lines labeled \"Recorded answer\" are not part of that text"));
    assert!(ASSESSOR_PROMPT.contains("never quote from these lines"));
    assert!(ASSESSOR_PROMPT.contains("When they are absent the capturer was not asked"));
    assert!(
        ASSESSOR_PROMPT.contains("Lines labeled \"Recorded context\" are not part of that text")
    );
    assert_eq!(ASSESSOR_PROMPT_VERSION, "assessment-v3");
}

#[test]
fn a_captured_decisions_pending_assessment_input_shows_both_answers_and_a_quote_from_them_is_refused(
) {
    let loomtracer_dir = unique_test_dir("floor-answers-in-scorer-input");
    let tenant_id = TenantId::local();
    {
        let ledger = SqliteEventLedger::open(&loomtracer_dir).expect("ledger opens");
        let commands = Commands::new(&ledger);
        let option = commands
            .record_option("actor:test", "Rename", "Rename the command")
            .expect("option");
        let answers = FloorAnswers {
            binds: BindsAnswer::something("Users and scripts that call the old name"),
            undo_cost: UndoCostAnswer::cheap(),
        };
        commands
            .propose_decision(DecisionProposalInput {
                grounding: Grounding::NotAsked,
                expressed_confidence: None,
                project: None,
                actor_id: "actor:test",
                title: "Rename the checker command",
                rationale: "The old name misleads",
                topic_keys: &["cli".to_owned()],
                option_ids: std::slice::from_ref(&option),
                option_labels: &["Rename".to_owned()],
                chosen_option_id: Some(option.as_str()),
                decided_by: None,
                delegated_by: None,
                still_proposed: false,
                hypothesis_ids: &[],
                evidence_ids: &[],
                quote: None,
                question: None,
                floor: FloorInput::Stated(&answers),
            })
            .expect("propose decision");
    }

    let pending = find_unscored_decisions(&loomtracer_dir, &tenant_id).expect("scan succeeds");
    assert_eq!(pending.len(), 1);
    let text = &pending[0].decision_text;
    assert!(
        text.contains(
            "Recorded answer, binds: something (Users and scripts that call the old name)\n"
        ),
        "{text}"
    );
    assert!(
        text.contains("Recorded answer, undo cost: cheap\n"),
        "{text}"
    );

    // The prompt tells the model not to quote these lines, and the write path agrees: the quote
    // check reads the decision's own text, not the recorded answers.
    let mut dims = all_not_assessed_dimensions_json();
    dims["framing"] = serde_json::json!({
        "status": "assessed",
        "level": "partial",
        "explanation": "The answers name who is bound",
        "quote": "Users and scripts that call the old name"
    });
    let output: AssessmentOutput =
        serde_json::from_value(serde_json::json!({ "dimensions": dims })).expect("output");
    {
        let ledger = SqliteEventLedger::open(&loomtracer_dir).expect("ledger opens");
        let commands = Commands::new(&ledger);
        let payload = build_assessed_payload(
            &pending[0].decision_id,
            "test-model",
            ASSESSOR_PROMPT_VERSION,
            output,
        );
        let refusal = commands
            .record_decision_assessed(ACTOR_ID, payload, None)
            .expect_err("a quote from a recorded answer is not in the decision's text");
        assert!(
            refusal
                .to_string()
                .contains("does not occur in the recorded text"),
            "{refusal}"
        );
    }

    let _ = std::fs::remove_dir_all(&loomtracer_dir);
}

// --- the lines offered as quotable are the text the quote check accepts (hivemind-m88ju) ---

/// Tries every labeled line of `text` as the quote of a `partial` framing answer for
/// `decision_id`, through the write path's own check. A line the scorer offers as quotable must
/// be accepted; a `Recorded ...` line must be refused. Returns the labels of the quotable lines,
/// in the order offered.
fn offered_quotable_labels(loomtracer_dir: &PathBuf, decision_id: &str, text: &str) -> Vec<String> {
    let ledger = SqliteEventLedger::open(loomtracer_dir).expect("ledger opens");
    let commands = Commands::new(&ledger);
    let mut quotable = Vec::new();
    for line in text.lines() {
        let (label, value) = line.split_once(": ").expect("every line is labeled");
        let mut dims = all_not_assessed_dimensions_json();
        dims["framing"] = serde_json::json!({
            "status": "assessed",
            "level": "partial",
            "explanation": "quoted from a line the scorer offered",
            "quote": value
        });
        let output: AssessmentOutput =
            serde_json::from_value(serde_json::json!({ "dimensions": dims })).expect("output");
        let payload =
            build_assessed_payload(decision_id, "test-model", ASSESSOR_PROMPT_VERSION, output);
        let result = commands.record_decision_assessed(ACTOR_ID, payload, None);
        if label.starts_with("Recorded ") {
            let refusal = result.expect_err(line);
            assert!(
                refusal
                    .to_string()
                    .contains("does not occur in the recorded text"),
                "{line}: {refusal}"
            );
        } else {
            result.unwrap_or_else(|error| {
                panic!("the scorer offered {line:?} as quotable but the write path refused it: {error}")
            });
            quotable.push(label.to_owned());
        }
    }
    quotable
}

#[test]
fn every_line_offered_as_quotable_for_a_classified_capture_is_accepted_and_no_other_is() {
    let loomtracer_dir = unique_test_dir("quotable-classified");
    let tenant_id = TenantId::local();
    {
        let ledger = SqliteEventLedger::open(&loomtracer_dir).expect("ledger opens");
        let commands = Commands::new(&ledger);
        let mut capture = minimal_decision_capture();
        capture.title = "Store events in SQLite".to_owned();
        capture.rationale = "One file, no server to run".to_owned();
        capture.options = Some(vec!["SQLite file".to_owned(), "Postgres server".to_owned()]);
        capture.chosen_option = Some("SQLite file".to_owned());
        capture.expressed_confidence = Some("fairly sure".to_owned());
        capture.topic_keys = vec!["storage-engine".to_owned(), "durability".to_owned()];
        commands
            .record_ingest_batch_classified(
                "actor:test",
                &["batch-quotable".to_owned()],
                "haiku",
                "2",
                vec![capture],
                None,
            )
            .expect("record batch classified");
    }

    let pending = find_unscored_decisions(&loomtracer_dir, &tenant_id).expect("scan succeeds");
    assert_eq!(pending.len(), 1);
    let text = &pending[0].decision_text;
    assert!(
        text.contains("Recorded context, expressed confidence: fairly sure\n"),
        "{text}"
    );
    assert!(
        text.contains("Recorded context, topic keys: storage-engine, durability\n"),
        "{text}"
    );
    assert!(!text.contains("Options considered"), "{text}");

    let quotable = offered_quotable_labels(&loomtracer_dir, &pending[0].decision_id, text);
    assert_eq!(
        quotable,
        ["Title", "Rationale", "Option", "Option", "Chosen option"]
    );

    let _ = std::fs::remove_dir_all(&loomtracer_dir);
}

#[test]
fn every_line_offered_as_quotable_for_a_directly_captured_decision_is_accepted_and_no_other_is() {
    let loomtracer_dir = unique_test_dir("quotable-proposed");
    let tenant_id = TenantId::local();
    {
        let ledger = SqliteEventLedger::open(&loomtracer_dir).expect("ledger opens");
        let commands = Commands::new(&ledger);
        let rest = commands
            .record_option("actor:test", "REST", "REST is simpler to operate")
            .expect("option a");
        let grpc = commands
            .record_option("actor:test", "gRPC", "gRPC needs HTTP/2 and codegen")
            .expect("option b");
        let answers = FloorAnswers {
            binds: BindsAnswer::something("Every client of the public API"),
            undo_cost: UndoCostAnswer::cheap(),
        };
        commands
            .propose_decision(DecisionProposalInput {
                grounding: Grounding::NotAsked,
                expressed_confidence: None,
                project: None,
                actor_id: "actor:test",
                title: "Pick the API style",
                rationale: "REST is simpler to operate for this team",
                topic_keys: &["api".to_owned()],
                option_ids: &[rest.clone(), grpc.clone()],
                option_labels: &["REST".to_owned(), "gRPC".to_owned()],
                chosen_option_id: Some(rest.as_str()),
                decided_by: None,
                delegated_by: None,
                still_proposed: false,
                hypothesis_ids: &[],
                evidence_ids: &[],
                quote: Some("let us use the plain one"),
                question: Some("Which API style do we expose?"),
                floor: FloorInput::Stated(&answers),
            })
            .expect("propose decision");
    }

    let pending = find_unscored_decisions(&loomtracer_dir, &tenant_id).expect("scan succeeds");
    assert_eq!(pending.len(), 1);
    let quotable = offered_quotable_labels(
        &loomtracer_dir,
        &pending[0].decision_id,
        &pending[0].decision_text,
    );
    assert_eq!(
        quotable,
        [
            "Title",
            "Question",
            "Quote",
            "Rationale",
            "Option",
            "Option description",
            "Option",
            "Option description"
        ]
    );

    let _ = std::fs::remove_dir_all(&loomtracer_dir);
}

// --- build_assessed_payload ---

fn all_not_assessed_dimensions_json() -> serde_json::Value {
    serde_json::json!({
        "framing": {"status": "not_assessed", "reason": "n/a"},
        "alternatives": {"status": "not_assessed", "reason": "n/a"},
        "information": {"status": "not_assessed", "reason": "n/a"},
        "reasoning": {"status": "not_assessed", "reason": "n/a"},
        "values_tradeoffs": {"status": "not_assessed", "reason": "n/a"},
        "bias_exposure": {"status": "not_assessed", "reason": "n/a"},
        "calibration": {"status": "not_assessed", "reason": "n/a"}
    })
}

/// A stub model answer that needs no quote at all (every dimension is either `not_assessed` or
/// `assessed` at level `none`), so it can be written against any decision's recorded text
/// without the write path's substring check ever having anything to check.
fn stub_assessment_no_quotes_needed() -> AssessmentOutput {
    let mut dims = all_not_assessed_dimensions_json();
    dims["framing"] = serde_json::json!({"status": "assessed", "level": "none", "explanation": "no question recorded"});
    dims["values_tradeoffs"] = serde_json::json!({"status": "assessed", "level": "none", "explanation": "no tradeoff named"});
    serde_json::from_value(serde_json::json!({ "dimensions": dims }))
        .expect("valid assessment output")
}

#[test]
fn build_assessed_payload_sets_schema_version_and_ids() {
    let mut dims = all_not_assessed_dimensions_json();
    dims["framing"] = serde_json::json!({
        "status": "assessed", "level": "solid", "explanation": "e", "quote": "Use REST"
    });
    let output: AssessmentOutput =
        serde_json::from_value(serde_json::json!({ "dimensions": dims })).expect("valid output");

    let payload = build_assessed_payload(
        "decision-abc",
        "claude-haiku-4-5-20251001",
        "assessment-v1",
        output,
    );

    assert_eq!(payload.schema_version, DECISION_ASSESSED_SCHEMA_VERSION);
    assert_eq!(payload.decision_id, "decision-abc");
    assert_eq!(payload.model, "claude-haiku-4-5-20251001");
    assert_eq!(payload.prompt_version, "assessment-v1");
    assert!(payload.supersedes_score_id.is_none());
    assert!(payload.importance.is_none());
    assert!(matches!(
        payload.dimensions.framing,
        ModelDimension::Assessed {
            level: QualityLevel::Solid,
            ..
        }
    ));
}

#[test]
fn build_assessed_payload_carries_optional_importance_when_given() {
    let output: AssessmentOutput = serde_json::from_value(serde_json::json!({
        "dimensions": all_not_assessed_dimensions_json(),
        "importance": {
            "stakes": 5.0, "stakes_explanation": "e",
            "irreversibility": 0.5, "irreversibility_explanation": "e",
            "actionability": 0.5, "actionability_explanation": "e"
        }
    }))
    .expect("valid output");

    let payload = build_assessed_payload("decision-x", "model", "v1", output);

    assert_eq!(payload.importance.expect("importance present").stakes, 5.0);
}

// --- find_unscored_decisions (also covers hivemind-qo11.9's acceptance: a directly captured
// decision, not just a classifier capture, gets picked up for assessment) ---

fn minimal_decision_capture() -> CaptureItem {
    CaptureItem {
        kind: "decision".to_owned(),
        title: "Use Postgres".to_owned(),
        rationale: "Concurrent writes required".to_owned(),
        topic_keys: vec![],
        evidence_ids: vec![],
        options: None,
        chosen_option: None,
        extraction_confidence: 0.9,
        expressed_confidence: None,
        supersedes_id: None,
        premised_on_ids: vec![],
        supports_ids: vec![],
        refutes_ids: vec![],
        actor_id: None,
        accepted_by: vec![],
        rejected_by: vec![],
        blocked_actor_id: None,
        decision_id: None,
        participants: vec![],
        restates_id: None,
        source_turn_id: None,
        source_ts: None,
        question: None,
        session_initiator: None,
    }
}

#[test]
fn find_unscored_decisions_includes_classified_and_direct_and_dedupes_v2() {
    let loomtracer_dir = unique_test_dir("find-unscored");
    let tenant_id = TenantId::local();
    {
        let ledger = SqliteEventLedger::open(&loomtracer_dir).expect("ledger opens");
        let commands = Commands::new(&ledger);

        let option_a = commands
            .record_option("actor:test", "REST", "REST is simpler to operate")
            .expect("option a");
        let option_b = commands
            .record_option("actor:test", "gRPC", "gRPC needs HTTP/2 and codegen")
            .expect("option b");
        commands
            .propose_decision(DecisionProposalInput {
                grounding: Grounding::NotAsked,
                expressed_confidence: None,
                project: None,
                actor_id: "actor:test",
                title: "Pick the API style",
                rationale: "REST is simpler to operate for this team",
                topic_keys: &["api".to_owned()],
                option_ids: &[option_a.clone(), option_b.clone()],
                option_labels: &["REST".to_owned(), "gRPC".to_owned()],
                chosen_option_id: Some(option_a.as_str()),
                decided_by: None,
                delegated_by: None,
                still_proposed: false,
                hypothesis_ids: &[],
                evidence_ids: &[],
                quote: None,
                question: None,
                floor: FloorInput::NotAsked,
            })
            .expect("propose decision");

        commands
            .record_ingest_batch_classified(
                "actor:test",
                &["batch-unscored".to_owned()],
                "haiku",
                "2",
                vec![minimal_decision_capture()],
                None,
            )
            .expect("record batch classified");
    }

    let pending = find_unscored_decisions(&loomtracer_dir, &tenant_id).expect("scan succeeds");
    assert_eq!(
        pending.len(),
        2,
        "both the directly proposed decision and the classified capture are pending"
    );

    let capture_id = pending
        .iter()
        .find(|p| p.decision_id.starts_with("capture:"))
        .expect("the classified capture is pending")
        .decision_id
        .clone();
    let direct_id = pending
        .iter()
        .find(|p| !p.decision_id.starts_with("capture:"))
        .expect("the directly proposed decision is pending")
        .decision_id
        .clone();
    assert!(pending
        .iter()
        .find(|p| p.decision_id == capture_id)
        .expect("capture entry")
        .decision_text
        .contains("Use Postgres"));
    assert!(pending
        .iter()
        .find(|p| p.decision_id == direct_id)
        .expect("direct entry")
        .decision_text
        .contains("Pick the API style"));

    // Assess the capture: it drops out of the next scan, the direct decision remains.
    {
        let ledger = SqliteEventLedger::open(&loomtracer_dir).expect("ledger opens");
        let commands = Commands::new(&ledger);
        let payload = build_assessed_payload(
            &capture_id,
            "test-model",
            "test-prompt-v0",
            stub_assessment_no_quotes_needed(),
        );
        commands
            .record_decision_assessed(ACTOR_ID, payload, None)
            .expect("record assessed capture");
    }

    let pending = find_unscored_decisions(&loomtracer_dir, &tenant_id).expect("scan succeeds");
    assert_eq!(
        pending.len(),
        1,
        "the assessed capture drops out; the direct decision remains unscored"
    );
    assert_eq!(pending[0].decision_id, direct_id);

    // Assess the direct decision too: nothing remains pending.
    {
        let ledger = SqliteEventLedger::open(&loomtracer_dir).expect("ledger opens");
        let commands = Commands::new(&ledger);
        let payload = build_assessed_payload(
            &direct_id,
            "test-model",
            "test-prompt-v0",
            stub_assessment_no_quotes_needed(),
        );
        commands
            .record_decision_assessed(ACTOR_ID, payload, None)
            .expect("record assessed direct decision");
    }

    let pending = find_unscored_decisions(&loomtracer_dir, &tenant_id).expect("scan succeeds");
    assert!(
        pending.is_empty(),
        "both decisions are assessed; nothing left to score"
    );

    let _ = std::fs::remove_dir_all(&loomtracer_dir);
}

#[test]
fn a_malformed_v2_row_does_not_count_as_an_assessment_so_the_decision_is_still_pending() {
    let loomtracer_dir = unique_test_dir("find-unscored-malformed");
    let tenant_id = TenantId::local();
    let decision_id = {
        let ledger = SqliteEventLedger::open(&loomtracer_dir).expect("ledger opens");
        let commands = Commands::new(&ledger);
        let option_a = commands
            .record_option("actor:test", "REST", "REST is simpler to operate")
            .expect("option a");
        let option_b = commands
            .record_option("actor:test", "gRPC", "gRPC needs HTTP/2 and codegen")
            .expect("option b");
        let decision_id = commands
            .propose_decision(DecisionProposalInput {
                grounding: Grounding::NotAsked,
                expressed_confidence: None,
                project: None,
                actor_id: "actor:test",
                title: "Pick the API style",
                rationale: "REST is simpler to operate for this team",
                topic_keys: &["api".to_owned()],
                option_ids: &[option_a.clone(), option_b],
                option_labels: &["REST".to_owned(), "gRPC".to_owned()],
                chosen_option_id: Some(option_a.as_str()),
                decided_by: None,
                delegated_by: None,
                still_proposed: false,
                hypothesis_ids: &[],
                evidence_ids: &[],
                quote: None,
                question: None,
                floor: FloorInput::NotAsked,
            })
            .expect("propose decision");
        // A row a buggy producer or a hand edit left behind: schema version 2, bad shape. The
        // write path refuses it, so it goes in through the ledger.
        ledger
            .append(crate::events::Event {
                tenant_id: tenant_id.clone(),
                event_id: None,
                event_uuid: uuid::Uuid::new_v4(),
                correlation_id: None,
                causation_event_id: None,
                event_type: EventType::DecisionScored,
                actor_id: ACTOR_ID.to_owned(),
                source: crate::events::EventSource::Agent,
                source_ref: None,
                payload: serde_json::json!({
                    "schema_version": 2,
                    "decision_id": decision_id,
                    "model": "model-x",
                    "prompt_version": "prompt-x",
                    "dimensions": {"framing": "not an answer"}
                }),
                ts: Some(chrono::Utc::now()),
            })
            .expect("a ledger appends what it is given");
        decision_id
    };

    let pending = find_unscored_decisions(&loomtracer_dir, &tenant_id).expect("scan succeeds");
    assert_eq!(
        pending.len(),
        1,
        "the unreadable row is no assessment: the decision stays pending"
    );
    assert_eq!(pending[0].decision_id, decision_id);

    let _ = std::fs::remove_dir_all(&loomtracer_dir);
}

// --- resolve_capture_node_id ---

#[test]
fn resolve_capture_node_id_finds_decision_capture() {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let event_id = commands
        .record_ingest_batch_classified(
            "actor:test",
            &["batch-1".to_owned()],
            "haiku",
            "2",
            vec![minimal_decision_capture()],
            None,
        )
        .expect("record succeeds")
        .event_id;

    let (node_id, causation_event_id) =
        resolve_capture_node_id(&ledger, &TenantId::local(), "batch-1", 0)
            .expect("resolves successfully");

    assert_eq!(node_id, format!("capture:{event_id}:0"));
    assert_eq!(causation_event_id, event_id);
}

#[test]
fn resolve_capture_node_id_rejects_non_decision_kind() {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let mut evidence_capture = minimal_decision_capture();
    evidence_capture.kind = "evidence".to_owned();
    commands
        .record_ingest_batch_classified(
            "actor:test",
            &["batch-2".to_owned()],
            "haiku",
            "2",
            vec![evidence_capture],
            None,
        )
        .expect("record succeeds");

    let err = resolve_capture_node_id(&ledger, &TenantId::local(), "batch-2", 0).unwrap_err();
    assert!(err.to_string().contains("not \"decision\""));
}

#[test]
fn resolve_capture_node_id_rejects_out_of_range_index() {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    commands
        .record_ingest_batch_classified(
            "actor:test",
            &["batch-3".to_owned()],
            "haiku",
            "2",
            vec![minimal_decision_capture()],
            None,
        )
        .expect("record succeeds");

    let err = resolve_capture_node_id(&ledger, &TenantId::local(), "batch-3", 5).unwrap_err();
    assert!(err.to_string().contains("out of range"));
}

#[test]
fn resolve_capture_node_id_rejects_unknown_batch() {
    let ledger = InMemoryEventLedger::new();
    let err = resolve_capture_node_id(&ledger, &TenantId::local(), "no-such-batch", 0).unwrap_err();
    assert!(err.to_string().contains("no-such-batch"));
}

// --- a decision that keeps failing is not paid for on every pass (hivemind-m88ju) ---

#[test]
fn a_decision_never_tried_is_ready() {
    let attempts = AttemptLog::default();
    assert_eq!(
        attempts.standing("decision-a", Instant::now()),
        Standing::Ready
    );
}

#[test]
fn a_failed_decision_waits_out_the_backoff_and_is_then_ready_again() {
    let mut attempts = AttemptLog::default();
    let start = Instant::now();
    assert_eq!(
        attempts.record_failure("decision-a", start),
        AfterFailure::Retry { attempt: 1 }
    );

    assert_eq!(
        attempts.standing("decision-a", start + POLL_INTERVAL),
        Standing::Backoff,
        "the next pass, 30 s later, does not send it again"
    );
    assert_eq!(
        attempts.standing("decision-a", start + RETRY_BACKOFF),
        Standing::Ready
    );
}

#[test]
fn a_decision_that_fails_max_attempts_times_is_given_up_for_good() {
    let mut attempts = AttemptLog::default();
    let start = Instant::now();
    let mut outcomes = Vec::new();
    for round in 0..MAX_ATTEMPTS {
        let at = start + RETRY_BACKOFF * round;
        assert_eq!(attempts.standing("decision-a", at), Standing::Ready);
        outcomes.push(attempts.record_failure("decision-a", at));
    }

    assert_eq!(
        outcomes,
        [
            AfterFailure::Retry { attempt: 1 },
            AfterFailure::Retry { attempt: 2 },
            AfterFailure::GaveUp { attempts: 3 },
        ]
    );
    let much_later = start + RETRY_BACKOFF * 1000;
    assert_eq!(
        attempts.standing("decision-a", much_later),
        Standing::GivenUp,
        "waiting does not bring the decision back: the bound is on attempts, not on time"
    );
}

#[test]
fn one_decisions_failures_do_not_hold_back_another() {
    let mut attempts = AttemptLog::default();
    let now = Instant::now();
    for _ in 0..MAX_ATTEMPTS {
        attempts.record_failure("decision-a", now);
    }

    assert_eq!(attempts.standing("decision-a", now), Standing::GivenUp);
    assert_eq!(attempts.standing("decision-b", now), Standing::Ready);
}
