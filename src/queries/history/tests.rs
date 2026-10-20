// Parent module gates this file with #[cfg(test)]; repeat the marker so UBS can filter test-only assertions.
#[cfg(test)]
use chrono::TimeZone;
use serde_json::json;
use uuid::Uuid;

use crate::ledger::InMemoryEventLedger;
use crate::queries::test_fixtures::{
    attention_scenario, decision_capture, floor_scenario, reaffirmation_scenario,
    reexamine_scenario, revisions_scenario, ts as test_ts, EveryEventRead, TypedReadsOnly,
};

use super::*;

#[test]
fn recent_activity_is_bounded_newest_first_and_cited() -> Result<()> {
    let ledger = fixture_ledger()?;

    let response = get_recent_activity(
        &ledger,
        &graph_of(&ledger)?,
        &RecentActivityRequest {
            limit: 2,
            ..RecentActivityRequest::default()
        },
    )?;

    assert_eq!(response.result_count, 2);
    assert!(response.truncated);
    assert_eq!(response.data.next_cursor.as_deref(), Some("2"));
    assert!(
        response.data.items[0].event_origin > response.data.items[1].event_origin,
        "recent activity must be newest first"
    );
    assert_eq!(response.data.items[0].source, EventSource::Slack);
    assert_eq!(
        response.data.items[0].source_ref.as_deref(),
        Some("thread-1")
    );
    assert_eq!(response.data.items[0].citation_id, "event:8");

    Ok(())
}

#[test]
fn changed_since_classifies_decision_history_and_paginates() -> Result<()> {
    let ledger = fixture_ledger()?;

    let response = get_decisions_changed_since(
        &ledger,
        &graph_of(&ledger)?,
        &ChangedSinceRequest {
            since_offset: Some(1),
            limit: 4,
            ..ChangedSinceRequest::default()
        },
    )?;

    assert_eq!(response.result_count, 4);
    assert!(response.truncated);
    assert_eq!(response.data.next_cursor.as_deref(), Some("4"));
    let kinds = response
        .data
        .items
        .iter()
        .map(|row| row.change_kind)
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            HistoryChangeKind::ContextChange,
            HistoryChangeKind::NewDecision,
            HistoryChangeKind::StatusChange,
            HistoryChangeKind::NewDecision,
        ]
    );

    let second_page = get_decisions_changed_since(
        &ledger,
        &graph_of(&ledger)?,
        &ChangedSinceRequest {
            since_offset: Some(1),
            limit: 10,
            cursor: Some("4".to_owned()),
            ..ChangedSinceRequest::default()
        },
    )?;
    let kinds = second_page
        .data
        .items
        .iter()
        .map(|row| row.change_kind)
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            HistoryChangeKind::NewEvidence,
            HistoryChangeKind::StalePremise,
            HistoryChangeKind::Supersession,
        ]
    );
    let refuted = second_page
        .data
        .items
        .iter()
        .find(|row| row.change_kind == HistoryChangeKind::StalePremise)
        .expect("refuted assumption row");
    assert_eq!(refuted.decision_ids, vec!["decision-a".to_owned()]);
    assert_eq!(refuted.source_ref.as_deref(), Some("thread-1"));

    Ok(())
}

#[test]
fn changed_since_windows_by_event_timestamp_and_leaves_the_offsets_unbounded() -> Result<()> {
    let ledger = fixture_ledger()?;

    let response = get_decisions_changed_since(
        &ledger,
        &graph_of(&ledger)?,
        &ChangedSinceRequest {
            since_timestamp: Some(ts(3)),
            until_timestamp: Some(ts(6)),
            limit: 10,
            ..ChangedSinceRequest::default()
        },
    )?;

    // Only the timestamps bound the window, so the offsets are the whole ledger's.
    assert_eq!(response.data.resolved_since.offset, 0);
    assert_eq!(response.data.resolved_since.timestamp, Some(ts(3)));
    assert_eq!(response.data.resolved_until.offset, 8);
    assert_eq!(response.data.resolved_until.timestamp, Some(ts(6)));
    assert_eq!(response.data.ledger_range.from_offset_exclusive, 0);
    assert_eq!(response.data.ledger_range.to_offset_inclusive, 8);
    assert_eq!(
        response
            .data
            .items
            .iter()
            .map(|row| row.event_origin)
            .collect::<Vec<_>>(),
        vec![4, 5, 6]
    );

    Ok(())
}

#[test]
fn changed_since_handles_request_blocker_and_notification_events() -> Result<()> {
    let ledger = InMemoryEventLedger::new();
    let notification_id = Uuid::from_u128(3).to_string();
    for event in [
        event(
            1,
            EventType::DecisionRequested,
            "actor:requester",
            json!({
                "topic_keys": ["ops"],
                "decision_id": "decision-a",
                "reason": "Need an owner decision",
                "priority": "P1",
                "required_owner_id": "actor:owner",
                "authority_class": "operational",
                "requested_by": "actor:requester",
                "client_request_id": "request-1"
            }),
        ),
        event(
            2,
            EventType::BlockerReported,
            "actor:blocked",
            json!({
                "blocker_id": "blocker-1",
                "blocked_actor_id": "actor:blocked",
                "decision_id": "decision-a",
                "topic_keys": ["ops"],
                "blocked_ref": "task-1",
                "blocked_ref_type": "bead",
                "reason": "Needs decision",
                "priority": "P1",
                "last_progress_at": "2026-05-19T12:00:00Z",
                "required_owner_id": "actor:owner"
            }),
        ),
        event(
            3,
            EventType::NotificationSent,
            "actor:notifier",
            json!({
                "blocker_id": "blocker-1",
                "recipient_actor_id": "actor:owner",
                "channel": "gc",
                "threshold_rule": "p1",
                "source_event_ids": [2],
                "dedupe_key": "blocker-1:p1",
                "sent_at": "2026-05-19T12:00:03Z"
            }),
        ),
        event(
            4,
            EventType::BlockerResolved,
            "actor:owner",
            json!({
                "blocker_id": "blocker-1",
                "resolution_event_id": 4,
                "resolution_reason": "Decision owner responded"
            }),
        ),
        event(
            5,
            EventType::NotificationAcknowledged,
            "actor:owner",
            json!({
                "notification_id": notification_id,
                "ack_at": "2026-05-19T12:00:05Z",
                "snooze_until": null
            }),
        ),
    ] {
        ledger.append(event)?;
    }

    let response = get_decisions_changed_since(
        &ledger,
        &graph_of(&ledger)?,
        &ChangedSinceRequest {
            since_offset: Some(0),
            limit: 10,
            ..ChangedSinceRequest::default()
        },
    )?;

    assert_eq!(response.result_count, 5);
    assert!(response
        .data
        .items
        .iter()
        .all(|row| row.change_kind == HistoryChangeKind::ContextChange));
    assert_eq!(response.data.items[0].decision_ids, vec!["decision-a"]);
    assert_eq!(response.data.items[1].decision_ids, vec!["decision-a"]);
    assert!(response.data.items[2].decision_ids.is_empty());
    assert!(response.data.items[3].decision_ids.is_empty());
    assert!(response.data.items[4].decision_ids.is_empty());
    assert!(response.data.items[0]
        .affected_nodes
        .contains(&affected_node(
            &Uuid::from_u128(1).to_string(),
            NodeKind::DecisionRequest
        )));
    assert!(response.data.items[1]
        .affected_nodes
        .contains(&affected_node("blocker-1", NodeKind::Blocker)));
    assert!(response.data.items[2]
        .affected_nodes
        .contains(&affected_node(
            &Uuid::from_u128(3).to_string(),
            NodeKind::Notification
        )));
    assert!(response.data.items[3]
        .affected_nodes
        .contains(&affected_node("blocker-1", NodeKind::Blocker)));
    assert!(response.data.items[4]
        .affected_nodes
        .contains(&affected_node(
            &Uuid::from_u128(3).to_string(),
            NodeKind::Notification
        )));

    Ok(())
}

#[test]
fn export_summary_includes_query_params_ledger_range_and_citations() -> Result<()> {
    let ledger = fixture_ledger()?;
    let generated_at = Utc.with_ymd_and_hms(2026, 5, 19, 12, 0, 0).unwrap();

    let response = export_read_only_summary(
        &ledger,
        &graph_of(&ledger)?,
        &ReadOnlyExportRequest {
            query: ReadOnlyExportQuery::RecentActivity(RecentActivityRequest {
                limit: 1,
                ..RecentActivityRequest::default()
            }),
            format: ReadOnlyExportFormat::Markdown,
            generated_at,
        },
    )?;

    assert_eq!(response.result_count, 1);
    assert!(response.truncated);
    assert_eq!(response.data.generated_at, generated_at);
    assert_eq!(response.data.ledger_range.to_offset_inclusive, 8);
    assert_eq!(response.data.continuation_cursor.as_deref(), Some("1"));
    assert_eq!(
        response.data.query_params.get("query"),
        Some(&json!("recent_activity"))
    );
    assert!(response.data.citation_map.contains_key("event:8"));
    assert!(response.data.json.is_none());
    assert!(response
        .data
        .markdown
        .as_deref()
        .expect("markdown")
        .contains("citation=event:8"));

    Ok(())
}

/// A malformed assessment row (`decision.scored`, schema version 2) that a buggy or old producer,
/// a partial import or a hand edit left in the ledger. `validate` refuses it; every ledger read
/// has to answer anyway.
fn malformed_assessment_event(sequence: u64) -> Event {
    event(
        sequence,
        EventType::DecisionScored,
        "agent:hivemind:scorer",
        json!({
            "schema_version": 2,
            "decision_id": "decision-a",
            "model": "model-x",
            "prompt_version": "prompt-x",
            "dimensions": {"framing": "not an answer"}
        }),
    )
}

#[test]
fn every_history_read_answers_the_same_with_a_malformed_assessment_row_in_the_ledger() -> Result<()>
{
    // What a read says about decisions, not the ledger range it reports: the range counts every
    // event in the ledger, the row that is skipped included.
    fn without_ledger_range(mut answer: Value) -> Value {
        if let Some(object) = answer.as_object_mut() {
            object.remove("ledger_range");
            object.remove("resolved_until");
        }
        answer
    }

    fn answers(ledger: &InMemoryEventLedger) -> Result<Vec<Value>> {
        let graph = crate::projector::memory::MemoryGraph::default();
        crate::projector::rebuild_graph(ledger, &graph)?;
        Ok(vec![
            without_ledger_range(
                serde_json::to_value(
                    get_recent_activity(
                        ledger,
                        &graph_of(ledger)?,
                        &RecentActivityRequest::default(),
                    )?
                    .data,
                )
                .expect("serializes"),
            ),
            without_ledger_range(
                serde_json::to_value(
                    get_decisions_changed_since(
                        ledger,
                        &graph_of(ledger)?,
                        &ChangedSinceRequest::default(),
                    )?
                    .data,
                )
                .expect("serializes"),
            ),
            without_ledger_range(
                serde_json::to_value(
                    get_recent_decisions(
                        ledger,
                        &graph,
                        &RecentDecisionsRequest {
                            since_timestamp: Utc.with_ymd_and_hms(2026, 5, 19, 0, 0, 0).unwrap(),
                            until_timestamp: None,
                            filters: RecentDecisionFilterRequest::default(),
                            limit: 25,
                            cursor: None,
                        },
                    )?
                    .data,
                )
                .expect("serializes"),
            ),
            without_ledger_range(
                serde_json::to_value(
                    crate::queries::get_decision_timeline(&graph, ledger, "decision-a")?.data,
                )
                .expect("serializes"),
            ),
            without_ledger_range(
                serde_json::to_value(
                    crate::queries::get_changed_decisions(
                        &graph,
                        ledger,
                        &crate::queries::ChangedDecisionsRequest::default(),
                    )?
                    .data,
                )
                .expect("serializes"),
            ),
        ])
    }

    let ledger = fixture_ledger()?;
    let clean = answers(&ledger)?;

    // Appended after everything else, so no other event moves; nothing may be added or dropped.
    ledger.append(malformed_assessment_event(900))?;
    assert_eq!(answers(&ledger)?, clean);
    Ok(())
}

fn fixture_ledger() -> Result<InMemoryEventLedger> {
    let ledger = InMemoryEventLedger::new();
    for (index, event) in [
        event(
            1,
            EventType::EvidenceRecorded,
            "actor:researcher",
            json!({
                "evidence_id": "evidence-1",
                "content": "Baseline evidence",
                "source": "seed"
            }),
        ),
        event(
            2,
            EventType::HypothesisRecorded,
            "actor:researcher",
            json!({
                "hypothesis_id": "hypothesis-1",
                "statement": "Queue pressure stays low"
            }),
        ),
        event(
            3,
            EventType::DecisionProposed,
            "actor:planner",
            json!({
                "decision_id": "decision-a",
                "title": "Use batch queue",
                "rationale": "It keeps writes simple",
                "topic_keys": ["infra"],
                "option_ids": [],
                "chosen_option_id": null,
                "hypothesis_ids": ["hypothesis-1"],
                "evidence_ids": ["evidence-1"]
            }),
        ),
        event(
            4,
            EventType::DecisionAccepted,
            "actor:reviewer",
            json!({ "decision_id": "decision-a" }),
        ),
        event(
            5,
            EventType::DecisionProposed,
            "actor:planner",
            json!({
                "decision_id": "decision-b",
                "title": "Use streaming queue",
                "rationale": "It reduces latency",
                "topic_keys": ["infra"],
                "option_ids": [],
                "chosen_option_id": null,
                "hypothesis_ids": [],
                "evidence_ids": []
            }),
        ),
        event(
            6,
            EventType::RelationAdded,
            "actor:analyst",
            json!({
                "relation": "BASED_ON",
                "from_id": "decision-b",
                "to_id": "evidence-1"
            }),
        ),
        event(
            7,
            EventType::RelationAdded,
            "actor:auditor",
            json!({
                "relation": "REFUTES",
                "from_id": "evidence-1",
                "to_id": "hypothesis-1"
            }),
        ),
        event(
            8,
            EventType::DecisionSuperseded,
            "actor:architect",
            json!({
                "old_decision_id": "decision-a",
                "new_decision_id": "decision-b"
            }),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let mut event = event;
        if index >= 6 {
            event.source = EventSource::Slack;
            event.source_ref = Some("thread-1".to_owned());
        }
        ledger.append(event)?;
    }
    Ok(ledger)
}

fn event(sequence: u64, event_type: EventType, actor_id: &str, payload: Value) -> Event {
    Event {
        tenant_id: Default::default(),
        event_id: None,
        event_uuid: Uuid::from_u128(u128::from(sequence)),
        correlation_id: Some("history-test".to_owned()),
        causation_event_id: None,
        event_type,
        actor_id: actor_id.to_owned(),
        source: EventSource::Api,
        source_ref: Some("history-test".to_owned()),
        payload,
        ts: Some(ts(sequence)),
    }
}

fn graph_of(ledger: &InMemoryEventLedger) -> Result<crate::projector::memory::MemoryGraph> {
    let graph = crate::projector::memory::MemoryGraph::default();
    crate::projector::rebuild_graph(ledger, &graph)?;
    Ok(graph)
}

fn ts(sequence: u64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 5, 19, 12, 0, 0).unwrap()
        + chrono::Duration::seconds(i64::try_from(sequence).unwrap())
}

#[test]
fn added_since_returns_empty_for_window_before_any_event() -> Result<()> {
    let ledger = fixture_ledger()?;
    let response = get_decisions_added_since(
        &ledger,
        &graph_of(&ledger)?,
        &DecisionsAddedSinceRequest {
            since_timestamp: Some(Utc.with_ymd_and_hms(2026, 5, 18, 0, 0, 0).unwrap()),
            until_timestamp: Some(Utc.with_ymd_and_hms(2026, 5, 19, 11, 0, 0).unwrap()),
            limit: 10,
            ..DecisionsAddedSinceRequest::default()
        },
    )?;

    assert_eq!(response.data.total_added, 0);
    assert_eq!(response.data.total_changed_existing, 0);
    assert!(response.data.added_decisions.is_empty());
    assert!(response.data.changed_existing_decisions.is_empty());
    Ok(())
}

#[test]
fn added_since_separates_added_and_changed_with_deterministic_order() -> Result<()> {
    let ledger = fixture_ledger()?;

    let response = get_decisions_added_since(
        &ledger,
        &graph_of(&ledger)?,
        &DecisionsAddedSinceRequest {
            since_offset: Some(0),
            limit: 50,
            ..DecisionsAddedSinceRequest::default()
        },
    )?;

    assert_eq!(response.data.total_added, 2);
    assert_eq!(response.data.total_changed_existing, 0);
    let added_ids: Vec<_> = response
        .data
        .added_decisions
        .iter()
        .map(|entry| entry.decision_id.clone())
        .collect();
    assert_eq!(added_ids, vec!["decision-a", "decision-b"]);

    let decision_a = &response.data.added_decisions[0];
    assert_eq!(decision_a.creation.event_origin, 3);
    assert_eq!(decision_a.status, DecisionStatus::Superseded);
    assert_eq!(decision_a.evidence_ids, vec!["evidence-1"]);
    assert_eq!(decision_a.hypothesis_ids, vec!["hypothesis-1"]);
    let kinds: Vec<_> = decision_a
        .changes_in_window
        .iter()
        .map(|change| change.change_kind)
        .collect();
    assert!(kinds.contains(&HistoryChangeKind::StatusChange));
    assert!(kinds.contains(&HistoryChangeKind::StalePremise));
    assert!(kinds.contains(&HistoryChangeKind::Supersession));

    let decision_b = &response.data.added_decisions[1];
    assert_eq!(decision_b.creation.event_origin, 5);
    assert!(decision_b
        .changes_in_window
        .iter()
        .any(|change| change.change_kind == HistoryChangeKind::NewEvidence));
    assert_eq!(decision_b.evidence_ids, vec!["evidence-1"]);

    Ok(())
}

#[test]
fn added_since_classifies_changed_existing_when_creation_is_before_window() -> Result<()> {
    let ledger = fixture_ledger()?;

    let response = get_decisions_added_since(
        &ledger,
        &graph_of(&ledger)?,
        &DecisionsAddedSinceRequest {
            since_offset: Some(5),
            limit: 50,
            ..DecisionsAddedSinceRequest::default()
        },
    )?;

    assert_eq!(response.data.total_added, 0);
    assert_eq!(response.data.total_changed_existing, 2);
    let changed_ids: Vec<_> = response
        .data
        .changed_existing_decisions
        .iter()
        .map(|entry| entry.decision_id.clone())
        .collect();
    assert_eq!(changed_ids, vec!["decision-b", "decision-a"]);

    let decision_b_change = &response.data.changed_existing_decisions[0];
    assert!(decision_b_change.creation.is_some());
    assert_eq!(decision_b_change.creation.as_ref().unwrap().event_origin, 5);
    let decision_a_change = &response.data.changed_existing_decisions[1];
    assert!(decision_a_change
        .changes_in_window
        .iter()
        .any(|change| change.change_kind == HistoryChangeKind::Supersession));

    Ok(())
}

#[test]
fn added_since_windows_by_event_timestamp_and_reports_the_bounds() -> Result<()> {
    let ledger = fixture_ledger()?;

    let response = get_decisions_added_since(
        &ledger,
        &graph_of(&ledger)?,
        &DecisionsAddedSinceRequest {
            since_timestamp: Some(ts(2)),
            until_timestamp: Some(ts(5)),
            limit: 10,
            ..DecisionsAddedSinceRequest::default()
        },
    )?;

    assert_eq!(response.data.resolved_since.offset, 0);
    assert_eq!(response.data.resolved_since.timestamp, Some(ts(2)));
    assert_eq!(response.data.resolved_until.offset, 8);
    assert_eq!(response.data.resolved_until.timestamp, Some(ts(5)));
    assert_eq!(response.data.total_added, 2);
    assert_eq!(response.data.total_changed_existing, 0);
    Ok(())
}

#[test]
fn added_since_paginates_with_continuation_cursor() -> Result<()> {
    let ledger = fixture_ledger()?;

    let first = get_decisions_added_since(
        &ledger,
        &graph_of(&ledger)?,
        &DecisionsAddedSinceRequest {
            since_offset: Some(0),
            limit: 1,
            ..DecisionsAddedSinceRequest::default()
        },
    )?;
    assert_eq!(first.result_count, 1);
    assert!(first.truncated);
    assert_eq!(first.data.next_cursor.as_deref(), Some("1"));
    assert_eq!(first.data.added_decisions.len(), 1);
    assert_eq!(first.data.added_decisions[0].decision_id, "decision-a");

    let second = get_decisions_added_since(
        &ledger,
        &graph_of(&ledger)?,
        &DecisionsAddedSinceRequest {
            since_offset: Some(0),
            limit: 1,
            cursor: Some("1".to_owned()),
            ..DecisionsAddedSinceRequest::default()
        },
    )?;
    assert_eq!(second.result_count, 1);
    assert!(!second.truncated);
    assert!(second.data.next_cursor.is_none());
    assert_eq!(second.data.added_decisions[0].decision_id, "decision-b");

    Ok(())
}

#[test]
fn added_since_filters_by_import_run_id_extracted_from_source_ref() -> Result<()> {
    use crate::events::EventSource;
    use crate::ledger::InMemoryEventLedger;

    let ledger = InMemoryEventLedger::new();
    let import_source_ref = serde_json::json!({
        "source": "document",
        "path": "/docs/notes.md",
        "sha256": "abc",
        "import_run_id": "import:2026-05-19T12:00:00Z:r1",
        "block_id": "blk-1",
        "source_span": {"start_byte": 0, "end_byte": 10},
        "source_snippet": "hi",
        "importer_actor_id": "actor:importer",
        "original_actor_id": null,
        "provisional_actor": true
    })
    .to_string();

    let mut e1 = event(
        1,
        EventType::DecisionProposed,
        "actor:doc",
        json!({
            "decision_id": "decision-doc-1",
            "title": "Doc decision",
            "rationale": "Imported",
            "topic_keys": ["docs"],
            "option_ids": [],
            "chosen_option_id": null,
            "hypothesis_ids": [],
            "evidence_ids": []
        }),
    );
    e1.source = EventSource::Document;
    e1.source_ref = Some(import_source_ref.clone());
    ledger.append(e1)?;

    let mut e2 = event(
        2,
        EventType::DecisionProposed,
        "actor:planner",
        json!({
            "decision_id": "decision-other",
            "title": "Other",
            "rationale": "Plain",
            "topic_keys": ["other"],
            "option_ids": [],
            "chosen_option_id": null,
            "hypothesis_ids": [],
            "evidence_ids": []
        }),
    );
    e2.source = EventSource::Api;
    e2.source_ref = Some("plain-ref".to_owned());
    ledger.append(e2)?;

    let response = get_decisions_added_since(
        &ledger,
        &graph_of(&ledger)?,
        &DecisionsAddedSinceRequest {
            since_offset: Some(0),
            limit: 10,
            filters: DecisionsAddedSinceFilterRequest {
                import_run_ids: vec!["import:2026-05-19T12:00:00Z:r1".to_owned()],
                ..DecisionsAddedSinceFilterRequest::default()
            },
            ..DecisionsAddedSinceRequest::default()
        },
    )?;

    assert_eq!(response.data.total_added, 1);
    assert_eq!(
        response.data.added_decisions[0].decision_id,
        "decision-doc-1"
    );
    assert_eq!(
        response.data.added_decisions[0]
            .creation
            .import_run_id
            .as_deref(),
        Some("import:2026-05-19T12:00:00Z:r1")
    );
    Ok(())
}

/// A decision proposed in `billing`, moved to `pricing` by Alex (with a reason) and moved back
/// by Bea (without one): the two moves are separate facts by separate actors at separate times.
fn moved_and_moved_back_ledger() -> Result<InMemoryEventLedger> {
    let ledger = InMemoryEventLedger::new();
    ledger.append(event(
        1,
        EventType::DecisionProposed,
        "human:alice",
        json!({
            "decision_id": "decision-a",
            "title": "Use per-seat pricing",
            "rationale": "Simpler to reason about at our scale",
            "topic_keys": ["pricing"],
            "option_ids": [],
            "chosen_option_id": null,
            "hypothesis_ids": [],
            "evidence_ids": [],
            "project": "billing",
            "project_source": "stated"
        }),
    ))?;
    ledger.append(event(
        2,
        EventType::DecisionMoved,
        "human:alex",
        json!({
            "decision_id": "decision-a",
            "from": "billing",
            "to": "pricing",
            "reason": "per-seat pricing decisions live under Pricing"
        }),
    ))?;
    ledger.append(event(
        3,
        EventType::DecisionMoved,
        "human:bea",
        json!({ "decision_id": "decision-a", "from": "pricing", "to": "billing" }),
    ))?;
    Ok(ledger)
}

fn expected_project_moves() -> [(EventId, &'static str, DateTime<Utc>, ProjectMove); 2] {
    [
        (
            2,
            "human:alex",
            ts(2),
            ProjectMove {
                from: "billing".to_owned(),
                to: "pricing".to_owned(),
                reason: Some("per-seat pricing decisions live under Pricing".to_owned()),
            },
        ),
        (
            3,
            "human:bea",
            ts(3),
            ProjectMove {
                from: "pricing".to_owned(),
                to: "billing".to_owned(),
                reason: None,
            },
        ),
    ]
}

#[test]
fn changed_since_reports_a_move_and_its_reversal_with_from_to_actor_and_time() -> Result<()> {
    // Approved record shape, item 3: `decision.moved` keeps every move readable in history --
    // its own kind, not folded into `ContextChange`, never dropped -- and (mayor's ruling on
    // hivemind-s15q.10) the row itself says from, to, who and when. A move back is another row.
    let ledger = moved_and_moved_back_ledger()?;

    let rows = changed_since_start(&ledger)?;
    let moves: Vec<_> = rows
        .iter()
        .filter(|row| row.change_kind == HistoryChangeKind::ProjectMoved)
        .collect();
    assert_eq!(moves.len(), 2, "both moves are reported: {rows:?}");
    for (row, (origin, actor, at, expected)) in moves.iter().zip(expected_project_moves()) {
        assert_eq!(row.event_origin, origin);
        assert_eq!(row.event_type, EventType::DecisionMoved);
        assert_eq!(row.actor_id, actor);
        assert_eq!(row.ts, Some(at));
        assert_eq!(row.project_move.as_ref(), Some(&expected));
        assert_eq!(row.decision_ids, vec!["decision-a".to_owned()]);
        assert!(row
            .affected_nodes
            .iter()
            .any(|node| node.id == "decision-a" && node.kind == NodeKind::Decision));
    }
    assert_eq!(
        rows.iter().filter(|row| row.project_move.is_some()).count(),
        2,
        "only a move carries project_move"
    );
    Ok(())
}

#[test]
fn recent_activity_reports_a_move_and_its_reversal_with_from_to_actor_and_time() -> Result<()> {
    let ledger = moved_and_moved_back_ledger()?;

    let activity = get_recent_activity(
        &ledger,
        &graph_of(&ledger)?,
        &RecentActivityRequest::default(),
    )?
    .data;
    // Newest first: the move back, then the move, then the proposal (no project_move).
    let moves: Vec<_> = activity
        .items
        .iter()
        .filter(|row| row.change_kind == HistoryChangeKind::ProjectMoved)
        .collect();
    assert_eq!(moves.len(), 2, "both moves are reported: {activity:?}");
    for (row, (origin, actor, at, expected)) in moves.iter().rev().zip(expected_project_moves()) {
        assert_eq!(row.event_origin, origin);
        assert_eq!(row.actor_id, actor);
        assert_eq!(row.ts, Some(at));
        assert_eq!(row.project_move.as_ref(), Some(&expected));
    }
    assert_eq!(
        activity
            .items
            .last()
            .and_then(|row| row.project_move.as_ref()),
        None,
        "the proposal row is not a move"
    );
    Ok(())
}

#[test]
fn added_since_lists_each_move_in_the_decisions_changes_with_from_and_to() -> Result<()> {
    let ledger = moved_and_moved_back_ledger()?;

    let response = get_decisions_added_since(
        &ledger,
        &graph_of(&ledger)?,
        &DecisionsAddedSinceRequest {
            since_offset: Some(0),
            limit: 50,
            ..DecisionsAddedSinceRequest::default()
        },
    )?;

    let added = &response.data.added_decisions;
    assert_eq!(added.len(), 1);
    let moves: Vec<_> = added[0]
        .changes_in_window
        .iter()
        .filter(|change| change.change_kind == HistoryChangeKind::ProjectMoved)
        .collect();
    assert_eq!(moves.len(), 2);
    for (change, (origin, actor, at, expected)) in moves.iter().zip(expected_project_moves()) {
        assert_eq!(change.provenance.event_origin, origin);
        assert_eq!(change.provenance.actor_id, actor);
        assert_eq!(change.provenance.ts, Some(at));
        assert_eq!(change.project_move.as_ref(), Some(&expected));
    }
    Ok(())
}

#[test]
fn a_move_serializes_from_and_to_on_the_row_and_other_rows_omit_the_field() -> Result<()> {
    // The CLI, MCP and HTTP surfaces all emit these row structs as JSON.
    let ledger = moved_and_moved_back_ledger()?;

    let rows = changed_since_start(&ledger)?;
    let json_rows: Vec<Value> = rows
        .iter()
        .map(|row| serde_json::to_value(row).expect("row serializes"))
        .collect();
    let moved: Vec<_> = json_rows
        .iter()
        .filter(|row| row["change_kind"] == "project_moved")
        .collect();
    assert_eq!(moved.len(), 2);
    assert_eq!(
        moved[0]["project_move"],
        json!({
            "from": "billing",
            "to": "pricing",
            "reason": "per-seat pricing decisions live under Pricing"
        })
    );
    assert_eq!(
        moved[1]["project_move"],
        json!({ "from": "pricing", "to": "billing" }),
        "a move without a reason omits the key"
    );
    assert_eq!(moved[0]["actor_id"], "human:alex");
    assert_eq!(moved[1]["actor_id"], "human:bea");
    assert!(moved[0]["ts"].is_string(), "the row carries the time");
    for row in json_rows
        .iter()
        .filter(|row| row["change_kind"] != "project_moved")
    {
        assert!(
            row.get("project_move").is_none(),
            "only a move carries project_move: {row}"
        );
    }
    Ok(())
}

/// A decision proposed with a long title, retitled to a short one by Alex (with a reason) and
/// retitled back by Bea (without one): the two retitles are separate facts by separate actors
/// at separate times. Mirrors `moved_and_moved_back_ledger` exactly (hivemind-ydmp).
fn retitled_and_retitled_back_ledger() -> Result<InMemoryEventLedger> {
    let ledger = InMemoryEventLedger::new();
    ledger.append(event(
        1,
        EventType::DecisionProposed,
        "human:alice",
        json!({
            "decision_id": "decision-a",
            "title": "A decision with a very long paragraph title that predates the cap",
            "rationale": "Simpler to reason about at our scale",
            "topic_keys": ["pricing"],
            "option_ids": [],
            "chosen_option_id": null,
            "hypothesis_ids": [],
            "evidence_ids": [],
            "project": "billing",
            "project_source": "stated"
        }),
    ))?;
    ledger.append(event(
        2,
        EventType::DecisionRetitled,
        "human:alex",
        json!({
            "decision_id": "decision-a",
            "from": "A decision with a very long paragraph title that predates the cap",
            "to": "A short name",
            "reason": "titles are a name, not a paragraph"
        }),
    ))?;
    ledger.append(event(
        3,
        EventType::DecisionRetitled,
        "human:bea",
        json!({
            "decision_id": "decision-a",
            "from": "A short name",
            "to": "A decision with a very long paragraph title that predates the cap"
        }),
    ))?;
    Ok(ledger)
}

fn expected_title_changes() -> [(EventId, &'static str, DateTime<Utc>, TitleChange); 2] {
    [
        (
            2,
            "human:alex",
            ts(2),
            TitleChange {
                from: "A decision with a very long paragraph title that predates the cap"
                    .to_owned(),
                to: "A short name".to_owned(),
                reason: Some("titles are a name, not a paragraph".to_owned()),
            },
        ),
        (
            3,
            "human:bea",
            ts(3),
            TitleChange {
                from: "A short name".to_owned(),
                to: "A decision with a very long paragraph title that predates the cap".to_owned(),
                reason: None,
            },
        ),
    ]
}

#[test]
fn changed_since_reports_a_retitle_and_its_reversal_with_from_to_actor_and_time() -> Result<()> {
    // hivemind-ydmp: `decision.retitled` keeps every retitle readable in history -- its own
    // kind, not folded into `ContextChange`, never dropped -- and the row itself says from, to,
    // who and when. A retitle back is another row. Mirrors the equivalent `decision.moved` test.
    let ledger = retitled_and_retitled_back_ledger()?;

    let rows = changed_since_start(&ledger)?;
    let retitles: Vec<_> = rows
        .iter()
        .filter(|row| row.change_kind == HistoryChangeKind::TitleChanged)
        .collect();
    assert_eq!(retitles.len(), 2, "both retitles are reported: {rows:?}");
    for (row, (origin, actor, at, expected)) in retitles.iter().zip(expected_title_changes()) {
        assert_eq!(row.event_origin, origin);
        assert_eq!(row.event_type, EventType::DecisionRetitled);
        assert_eq!(row.actor_id, actor);
        assert_eq!(row.ts, Some(at));
        assert_eq!(row.title_change.as_ref(), Some(&expected));
        assert_eq!(row.decision_ids, vec!["decision-a".to_owned()]);
        assert!(row
            .affected_nodes
            .iter()
            .any(|node| node.id == "decision-a" && node.kind == NodeKind::Decision));
    }
    assert_eq!(
        rows.iter().filter(|row| row.title_change.is_some()).count(),
        2,
        "only a retitle carries title_change"
    );
    Ok(())
}

#[test]
fn recent_activity_reports_a_retitle_and_its_reversal_with_from_to_actor_and_time() -> Result<()> {
    let ledger = retitled_and_retitled_back_ledger()?;

    let activity = get_recent_activity(
        &ledger,
        &graph_of(&ledger)?,
        &RecentActivityRequest::default(),
    )?
    .data;
    // Newest first: the retitle back, then the retitle, then the proposal (no title_change).
    let retitles: Vec<_> = activity
        .items
        .iter()
        .filter(|row| row.change_kind == HistoryChangeKind::TitleChanged)
        .collect();
    assert_eq!(
        retitles.len(),
        2,
        "both retitles are reported: {activity:?}"
    );
    for (row, (origin, actor, at, expected)) in retitles.iter().rev().zip(expected_title_changes())
    {
        assert_eq!(row.event_origin, origin);
        assert_eq!(row.actor_id, actor);
        assert_eq!(row.ts, Some(at));
        assert_eq!(row.title_change.as_ref(), Some(&expected));
    }
    assert_eq!(
        activity
            .items
            .last()
            .and_then(|row| row.title_change.as_ref()),
        None,
        "the proposal row is not a retitle"
    );
    Ok(())
}

#[test]
fn added_since_lists_each_retitle_in_the_decisions_changes_with_from_and_to() -> Result<()> {
    let ledger = retitled_and_retitled_back_ledger()?;

    let response = get_decisions_added_since(
        &ledger,
        &graph_of(&ledger)?,
        &DecisionsAddedSinceRequest {
            since_offset: Some(0),
            limit: 50,
            ..DecisionsAddedSinceRequest::default()
        },
    )?;

    let added = &response.data.added_decisions;
    assert_eq!(added.len(), 1);
    let retitles: Vec<_> = added[0]
        .changes_in_window
        .iter()
        .filter(|change| change.change_kind == HistoryChangeKind::TitleChanged)
        .collect();
    assert_eq!(retitles.len(), 2);
    for (change, (origin, actor, at, expected)) in retitles.iter().zip(expected_title_changes()) {
        assert_eq!(change.provenance.event_origin, origin);
        assert_eq!(change.provenance.actor_id, actor);
        assert_eq!(change.provenance.ts, Some(at));
        assert_eq!(change.title_change.as_ref(), Some(&expected));
    }
    Ok(())
}

#[test]
fn a_retitle_serializes_from_and_to_on_the_row_and_other_rows_omit_the_field() -> Result<()> {
    // The CLI, MCP and HTTP surfaces all emit these row structs as JSON.
    let ledger = retitled_and_retitled_back_ledger()?;

    let rows = changed_since_start(&ledger)?;
    let json_rows: Vec<Value> = rows
        .iter()
        .map(|row| serde_json::to_value(row).expect("row serializes"))
        .collect();
    let retitled: Vec<_> = json_rows
        .iter()
        .filter(|row| row["change_kind"] == "title_changed")
        .collect();
    assert_eq!(retitled.len(), 2);
    assert_eq!(
        retitled[0]["title_change"],
        json!({
            "from": "A decision with a very long paragraph title that predates the cap",
            "to": "A short name",
            "reason": "titles are a name, not a paragraph"
        })
    );
    assert_eq!(
        retitled[1]["title_change"],
        json!({
            "from": "A short name",
            "to": "A decision with a very long paragraph title that predates the cap"
        }),
        "a retitle without a reason omits the key"
    );
    assert_eq!(retitled[0]["actor_id"], "human:alex");
    assert_eq!(retitled[1]["actor_id"], "human:bea");
    assert!(retitled[0]["ts"].is_string(), "the row carries the time");
    for row in json_rows
        .iter()
        .filter(|row| row["change_kind"] != "title_changed")
    {
        assert!(
            row.get("title_change").is_none(),
            "only a retitle carries title_change: {row}"
        );
    }
    Ok(())
}

// ── a premise going stale is never silent on the decisions that rest on it (hivemind-gwhr.3) ──

use crate::queries::test_fixtures::Scenario;

/// `d:goal` (accepted), with `d:derived` following from it at capture and `d:bystander` not.
fn goal_with_a_derived_decision() -> Result<Scenario> {
    let scenario = Scenario::new();
    scenario.decision(
        "d:goal",
        "Ship the hosted MVP",
        "human:alex",
        "2026-01-01T00:00:00Z",
    )?;
    scenario.accept("d:goal", "human:alex", "2026-01-01T00:00:01Z")?;
    let proposal = scenario.decision(
        "d:derived",
        "Use Postgres for the MVP",
        "agent:claude:crew",
        "2026-01-02T00:00:00Z",
    )?;
    scenario.relation(
        "FOLLOWS_FROM",
        "d:derived",
        "d:goal",
        "agent:claude:crew",
        Some(proposal),
        "2026-01-02T00:00:01Z",
    )?;
    scenario.decision(
        "d:bystander",
        "Pick a logo",
        "human:alex",
        "2026-01-02T00:00:02Z",
    )?;
    Ok(scenario)
}

fn changed_since_start(ledger: &InMemoryEventLedger) -> Result<Vec<DecisionChangeRow>> {
    Ok(get_decisions_changed_since(
        ledger,
        &graph_of(ledger)?,
        &ChangedSinceRequest {
            since_offset: Some(0),
            limit: 100,
            ..ChangedSinceRequest::default()
        },
    )?
    .data
    .items)
}

#[test]
fn following_a_premise_is_a_context_change() -> Result<()> {
    let scenario = goal_with_a_derived_decision()?;

    let rows = changed_since_start(scenario.ledger())?;

    let link = rows
        .iter()
        .find(|row| row.event_type == EventType::RelationAdded)
        .expect("the FOLLOWS_FROM relation event");
    assert_eq!(link.change_kind, HistoryChangeKind::ContextChange);
    assert_eq!(link.decision_ids, vec!["d:derived".to_owned()]);
    assert!(!rows
        .iter()
        .any(|row| row.change_kind == HistoryChangeKind::StalePremise));
    Ok(())
}

#[test]
fn superseding_a_premise_surfaces_the_derived_decisions_as_a_stale_premise() -> Result<()> {
    let scenario = goal_with_a_derived_decision()?;
    scenario.decision(
        "d:goal-2",
        "Ship the self-hosted MVP",
        "human:alex",
        "2026-02-01T00:00:00Z",
    )?;
    scenario.supersede("d:goal", "d:goal-2", "human:alex", "2026-02-01T00:00:01Z")?;

    let rows = changed_since_start(scenario.ledger())?;

    // The supersession keeps its own row...
    let supersession = rows
        .iter()
        .find(|row| row.change_kind == HistoryChangeKind::Supersession)
        .expect("the supersession row");
    assert_eq!(
        supersession.decision_ids,
        vec!["d:goal".to_owned(), "d:goal-2".to_owned()]
    );
    // ...and the decision that rested on the old premise gets a StalePremise row for the same event.
    let stale = rows
        .iter()
        .find(|row| row.change_kind == HistoryChangeKind::StalePremise)
        .expect("a stale-premise row");
    assert_eq!(stale.decision_ids, vec!["d:derived".to_owned()]);
    assert_eq!(stale.event_origin, supersession.event_origin);
    assert_eq!(stale.event_type, EventType::DecisionSuperseded);

    // The same in the recent-activity feed, and nothing for the bystander.
    let activity = get_recent_activity(
        scenario.ledger(),
        &graph_of(scenario.ledger())?,
        &RecentActivityRequest::default(),
    )?
    .data;
    let stale_activity: Vec<_> = activity
        .items
        .iter()
        .filter(|row| row.change_kind == HistoryChangeKind::StalePremise)
        .collect();
    assert_eq!(stale_activity.len(), 1);
    assert_eq!(stale_activity[0].decision_ids, vec!["d:derived".to_owned()]);
    assert!(!rows
        .iter()
        .any(|row| row.decision_ids.contains(&"d:bystander".to_owned())
            && row.change_kind == HistoryChangeKind::StalePremise));
    Ok(())
}

#[test]
fn rejecting_a_premise_surfaces_the_derived_decisions_as_a_stale_premise() -> Result<()> {
    let scenario = goal_with_a_derived_decision()?;
    scenario.reject("d:goal", "human:bob", "2026-02-01T00:00:00Z")?;

    let rows = changed_since_start(scenario.ledger())?;

    let stale = rows
        .iter()
        .find(|row| row.change_kind == HistoryChangeKind::StalePremise)
        .expect("a stale-premise row");
    assert_eq!(stale.decision_ids, vec!["d:derived".to_owned()]);
    assert_eq!(stale.event_type, EventType::DecisionRejected);
    Ok(())
}

#[test]
fn added_since_lists_the_stale_premise_on_the_derived_decision() -> Result<()> {
    let scenario = goal_with_a_derived_decision()?;
    scenario.decision(
        "d:goal-2",
        "Ship the self-hosted MVP",
        "human:alex",
        "2026-02-01T00:00:00Z",
    )?;
    scenario.supersede("d:goal", "d:goal-2", "human:alex", "2026-02-01T00:00:01Z")?;

    let response = get_decisions_added_since(
        scenario.ledger(),
        &graph_of(scenario.ledger())?,
        &DecisionsAddedSinceRequest {
            since_offset: Some(0),
            limit: 100,
            ..DecisionsAddedSinceRequest::default()
        },
    )?;

    let derived = response
        .data
        .added_decisions
        .iter()
        .find(|entry| entry.decision_id == "d:derived")
        .expect("derived decision listed");
    let kinds: Vec<HistoryChangeKind> = derived
        .changes_in_window
        .iter()
        .map(|change| change.change_kind)
        .collect();
    assert!(kinds.contains(&HistoryChangeKind::StalePremise));
    // It was not itself superseded.
    assert!(!kinds.contains(&HistoryChangeKind::Supersession));
    Ok(())
}

#[test]
fn a_decision_that_starts_following_a_premise_after_it_went_stale_is_not_reported_against_that_event(
) -> Result<()> {
    let scenario = goal_with_a_derived_decision()?;
    scenario.decision(
        "d:goal-2",
        "Ship the self-hosted MVP",
        "human:alex",
        "2026-02-01T00:00:00Z",
    )?;
    scenario.supersede("d:goal", "d:goal-2", "human:alex", "2026-02-01T00:00:01Z")?;
    // Named a stale premise after the fact: stale from birth, and its link is its own row.
    let late = scenario.decision(
        "d:late",
        "Late follower",
        "agent:claude:crew",
        "2026-03-01T00:00:00Z",
    )?;
    scenario.relation(
        "FOLLOWS_FROM",
        "d:late",
        "d:goal",
        "agent:claude:crew",
        Some(late),
        "2026-03-01T00:00:01Z",
    )?;

    let rows = changed_since_start(scenario.ledger())?;

    let stale_ids: Vec<&String> = rows
        .iter()
        .filter(|row| row.change_kind == HistoryChangeKind::StalePremise)
        .flat_map(|row| row.decision_ids.iter())
        .collect();
    assert_eq!(stale_ids, vec![&"d:derived".to_owned()]);
    Ok(())
}

// A decision a classifier drafted from a transcript (`capture:<event>:<index>`) has no
// `decision.proposed`, so the windows below must find it in the graph, dated by the turn it was
// said in and not by the classification that recorded it (hivemind-23xw6).

const CLASSIFIER: &str = "agent:claude:classifier";

/// A hand-recorded decision at 08:00, a draft said at 07:30 and classified at 09:00, and a draft
/// from a conversation of the 2nd classified at 09:05, all recorded on 2026-10-11.
fn drafts_scenario() -> Result<(Scenario, String, String)> {
    let scenario = Scenario::new();
    scenario.decision(
        "decision-hand",
        "Hand recorded",
        "human:alex",
        "2026-10-11T08:00:00Z",
    )?;
    let said_today = scenario.classified_decision_said_at(
        CLASSIFIER,
        "2026-10-11T09:00:00Z",
        "Said this morning",
        Some("human:alex"),
        Some("2026-10-11T07:30:00Z"),
    )?;
    let said_earlier = scenario.classified_decision_said_at(
        CLASSIFIER,
        "2026-10-11T09:05:00Z",
        "Said on the second",
        None,
        Some("2026-10-02T12:00:00Z"),
    )?;
    Ok((scenario, said_today, said_earlier))
}

fn recent_request(since: &str, until: Option<&str>) -> RecentDecisionsRequest {
    RecentDecisionsRequest {
        since_timestamp: test_ts(since),
        until_timestamp: until.map(test_ts),
        filters: RecentDecisionFilterRequest::default(),
        limit: 25,
        cursor: None,
    }
}

fn recent_in(scenario: &Scenario, request: &RecentDecisionsRequest) -> Result<Vec<String>> {
    let graph = scenario.graph()?;
    let response = get_recent_decisions(scenario.ledger(), &graph, request)?;
    Ok(response
        .data
        .items
        .into_iter()
        .map(|item| item.decision_id)
        .collect())
}

#[test]
fn recent_decisions_window_a_draft_by_when_it_was_said() -> Result<()> {
    let (scenario, said_today, said_earlier) = drafts_scenario()?;

    // Newest record first. Both classifications fall in the window; only the draft said in it is
    // listed beside the hand-recorded decision.
    assert_eq!(
        recent_in(&scenario, &recent_request("2026-10-11T07:00:00Z", None))?,
        vec![said_today.clone(), "decision-hand".to_owned()]
    );
    assert_eq!(
        recent_in(&scenario, &recent_request("2026-10-01T00:00:00Z", None))?,
        vec![said_earlier.clone(), said_today, "decision-hand".to_owned()]
    );
    // The window of the earlier conversation holds its draft and nothing recorded since.
    assert_eq!(
        recent_in(
            &scenario,
            &recent_request("2026-10-01T00:00:00Z", Some("2026-10-05T00:00:00Z"))
        )?,
        vec![said_earlier]
    );
    // Both classifications come after this window opens, and neither draft was said in it.
    assert!(recent_in(&scenario, &recent_request("2026-10-11T08:30:00Z", None))?.is_empty());
    Ok(())
}

#[test]
fn a_recent_draft_says_when_it_was_said_and_what_recorded_it() -> Result<()> {
    let (scenario, said_today, _) = drafts_scenario()?;
    let graph = scenario.graph()?;

    let response = get_recent_decisions(
        scenario.ledger(),
        &graph,
        &recent_request("2026-10-11T07:00:00Z", None),
    )?;

    let draft = response
        .data
        .items
        .iter()
        .find(|item| item.decision_id == said_today)
        .ok_or_else(|| query_error("the draft is listed"))?;
    assert_eq!(draft.title, "Said this morning");
    assert_eq!(draft.status, DecisionStatus::Proposed);
    assert_eq!(draft.decided_at, Some(test_ts("2026-10-11T07:30:00Z")));
    assert_eq!(draft.creation.ts, Some(test_ts("2026-10-11T09:00:00Z")));
    assert_eq!(draft.creation.event_type, EventType::IngestBatchClassified);
    assert_eq!(draft.creation.actor_id, CLASSIFIER);
    assert_eq!(draft.project, personal_project_handle(CLASSIFIER));
    assert_eq!(draft.topic_keys, vec!["capture".to_owned()]);
    assert!(draft.actor_ids.contains(&"human:alex".to_owned()));
    assert!(draft.actor_ids.contains(&CLASSIFIER.to_owned()));
    let hand = response
        .data
        .items
        .iter()
        .find(|item| item.decision_id == "decision-hand")
        .ok_or_else(|| query_error("the hand-recorded decision is listed"))?;
    assert_eq!(hand.decided_at, hand.creation.ts);
    Ok(())
}

#[test]
fn recent_decisions_filter_a_draft_like_any_other() -> Result<()> {
    let (scenario, said_today, said_earlier) = drafts_scenario()?;
    scenario.accept(&said_today, "human:alex", "2026-10-11T10:00:00Z")?;

    let accepted = RecentDecisionsRequest {
        filters: RecentDecisionFilterRequest {
            statuses: vec![DecisionStatus::Accepted],
            ..RecentDecisionFilterRequest::default()
        },
        ..recent_request("2026-10-01T00:00:00Z", None)
    };
    assert_eq!(recent_in(&scenario, &accepted)?, vec![said_today.clone()]);

    let by_the_classifier = RecentDecisionsRequest {
        filters: RecentDecisionFilterRequest {
            actor_patterns: vec!["agent:claude:*".to_owned()],
            ..RecentDecisionFilterRequest::default()
        },
        ..recent_request("2026-10-01T00:00:00Z", None)
    };
    assert_eq!(
        recent_in(&scenario, &by_the_classifier)?,
        vec![said_earlier, said_today]
    );
    Ok(())
}

fn added_in(
    scenario: &Scenario,
    request: &DecisionsAddedSinceRequest,
) -> Result<(Vec<String>, Vec<String>)> {
    let graph = scenario.graph()?;
    let response = get_decisions_added_since(scenario.ledger(), &graph, request)?;
    Ok((
        response
            .data
            .added_decisions
            .into_iter()
            .map(|entry| entry.decision_id)
            .collect(),
        response
            .data
            .changed_existing_decisions
            .into_iter()
            .map(|entry| entry.decision_id)
            .collect(),
    ))
}

#[test]
fn added_since_lists_a_draft_in_the_windows_it_was_said_and_recorded_in() -> Result<()> {
    let (scenario, said_today, said_earlier) = drafts_scenario()?;
    let since = |timestamp: &str| DecisionsAddedSinceRequest {
        since_timestamp: Some(test_ts(timestamp)),
        limit: 25,
        ..DecisionsAddedSinceRequest::default()
    };

    let (added, changed) = added_in(&scenario, &since("2026-10-11T07:00:00Z"))?;
    assert_eq!(added, vec!["decision-hand".to_owned(), said_today.clone()]);
    assert!(changed.is_empty());

    // Said on the 2nd, so a window opening on the 11th leaves it out although it was classified
    // after the window opened; one opening on the 1st holds all three.
    let (added, _) = added_in(&scenario, &since("2026-10-01T00:00:00Z"))?;
    assert_eq!(
        added,
        vec![
            "decision-hand".to_owned(),
            said_today.clone(),
            said_earlier.clone()
        ]
    );

    // An offset bound is the ledger's: the classification of `said_today` is event 2.
    let after_it = DecisionsAddedSinceRequest {
        since_offset: Some(2),
        ..since("2026-10-01T00:00:00Z")
    };
    assert_eq!(added_in(&scenario, &after_it)?.0, vec![said_earlier]);
    Ok(())
}

#[test]
fn an_added_draft_carries_the_changes_made_to_it_in_the_window() -> Result<()> {
    let (scenario, said_today, _) = drafts_scenario()?;
    scenario.accept(&said_today, "human:alex", "2026-10-11T10:00:00Z")?;
    let graph = scenario.graph()?;

    let response = get_decisions_added_since(
        scenario.ledger(),
        &graph,
        &DecisionsAddedSinceRequest {
            since_timestamp: Some(test_ts("2026-10-11T07:00:00Z")),
            limit: 25,
            ..DecisionsAddedSinceRequest::default()
        },
    )?;

    // Added once, with its acceptance, and not listed again as a changed decision.
    assert!(response.data.changed_existing_decisions.is_empty());
    let draft = response
        .data
        .added_decisions
        .iter()
        .find(|entry| entry.decision_id == said_today)
        .ok_or_else(|| query_error("the draft is added"))?;
    assert_eq!(draft.status, DecisionStatus::Accepted);
    assert_eq!(draft.decided_at, Some(test_ts("2026-10-11T07:30:00Z")));
    assert_eq!(draft.creation.event_type, EventType::IngestBatchClassified);
    assert_eq!(draft.changes_in_window.len(), 1);
    Ok(())
}

#[test]
fn a_draft_said_before_the_window_is_only_changed_by_an_acceptance_in_it() -> Result<()> {
    let (scenario, said_today, _) = drafts_scenario()?;
    scenario.accept(&said_today, "human:alex", "2026-10-11T10:00:00Z")?;

    let (added, changed) = added_in(
        &scenario,
        &DecisionsAddedSinceRequest {
            since_timestamp: Some(test_ts("2026-10-11T08:30:00Z")),
            limit: 25,
            ..DecisionsAddedSinceRequest::default()
        },
    )?;

    assert!(added.is_empty());
    assert_eq!(changed, vec![said_today]);
    Ok(())
}

// ── event time does not rise with ledger offset (hivemind-rk3p8) ──
//
// An importer or a classified transcript answer dates its event at its source's time, not at
// append time, so an event appended last can carry the oldest `ts` in the ledger. A time window
// is cut by each event's own `ts`, never by the offset of the last event dated before the bound.

use crate::commands::{CommandContext, Commands, DecisionProposalInput, FloorInput, Grounding};
use crate::events::EventProvenance;

const BEFORE_THE_PROPOSALS: &str = "2026-10-18T09:45:00Z";

struct BackDatedLedger {
    ledger: InMemoryEventLedger,
    first: String,
    second: String,
    imported: String,
}

/// Appended in this order, each dated as shown:
///
/// 1. `first` proposed at 10:00
/// 2. `second` proposed at 10:05
/// 3. `first` accepted at 10:30
/// 4. evidence recorded at 09:30, a classified answer written late for a turn of the morning
/// 5. `imported` proposed on 2026-09-01, a document imported today
fn back_dated_ledger() -> Result<BackDatedLedger> {
    let ledger = InMemoryEventLedger::new();
    let first = propose_at(&ledger, "2026-10-18T10:00:00Z", "Use the batch queue")?;
    let second = propose_at(&ledger, "2026-10-18T10:05:00Z", "Use the streaming queue")?;
    commands_at(&ledger, "2026-10-18T10:30:00Z").accept_decision(&first, "human:alex")?;
    commands_at(&ledger, "2026-10-18T09:30:00Z")
        .record_evidence("human:alex", "Queue depth stayed flat overnight")?;
    let imported = propose_at(&ledger, "2026-09-01T00:00:00Z", "Keep the old retry policy")?;
    Ok(BackDatedLedger {
        ledger,
        first,
        second,
        imported,
    })
}

fn commands_at<'a>(ledger: &'a InMemoryEventLedger, at: &str) -> Commands<'a, InMemoryEventLedger> {
    Commands::new_with_context(
        ledger,
        CommandContext::local(EventProvenance::cli()).with_event_ts(Some(test_ts(at))),
    )
}

/// An open recommendation (a chosen option, kept at `proposed`), dated `at` along with its option.
fn propose_at(ledger: &InMemoryEventLedger, at: &str, title: &str) -> Result<String> {
    let commands = commands_at(ledger, at);
    let option_id = commands.record_option("human:alex", "Adopt it", "Adopt the change")?;
    commands.propose_decision(DecisionProposalInput {
        grounding: Grounding::NotAsked,
        expressed_confidence: None,
        actor_id: "human:alex",
        title,
        rationale: "The window test needs a rationale to read on its own",
        topic_keys: &["history".to_owned()],
        option_ids: std::slice::from_ref(&option_id),
        option_labels: &["Adopt it".to_owned()],
        chosen_option_id: Some(option_id.as_str()),
        decided_by: None,
        still_proposed: true,
        hypothesis_ids: &[],
        evidence_ids: &[],
        quote: None,
        question: None,
        delegated_by: None,
        project: None,
        floor: FloorInput::NotAsked,
    })
}

/// The proposals, acceptances and evidence a changed-since window holds, oldest offset first,
/// leaving out the option events recorded along with each proposal.
fn changed_in(
    ledger: &InMemoryEventLedger,
    request: ChangedSinceRequest,
) -> Result<Vec<(EventId, EventType)>> {
    Ok(
        get_decisions_changed_since(ledger, &graph_of(ledger)?, &request)?
            .data
            .items
            .into_iter()
            .map(|row| (row.event_origin, row.event_type))
            .filter(|(_, event_type)| {
                matches!(
                    event_type,
                    EventType::DecisionProposed
                        | EventType::DecisionAccepted
                        | EventType::EvidenceRecorded
                )
            })
            .collect(),
    )
}

fn event_types(changes: &[(EventId, EventType)]) -> Vec<EventType> {
    changes.iter().map(|(_, event_type)| *event_type).collect()
}

fn added_ids(
    ledger: &InMemoryEventLedger,
    request: &DecisionsAddedSinceRequest,
) -> Result<(Vec<String>, Vec<String>)> {
    let data = get_decisions_added_since(ledger, &graph_of(ledger)?, request)?.data;
    Ok((
        data.added_decisions
            .into_iter()
            .map(|entry| entry.decision_id)
            .collect(),
        data.changed_existing_decisions
            .into_iter()
            .map(|entry| entry.decision_id)
            .collect(),
    ))
}

#[test]
fn recent_decisions_since_a_time_between_them_lists_the_proposals_a_back_dated_event_follows(
) -> Result<()> {
    let fixture = back_dated_ledger()?;
    let graph = graph_of(&fixture.ledger)?;

    let response = get_recent_decisions(
        &fixture.ledger,
        &graph,
        &recent_request(BEFORE_THE_PROPOSALS, None),
    )?;

    // Newest record first. The import dated 2026-09-01 is outside the window though it was
    // appended last.
    let listed: Vec<_> = response
        .data
        .items
        .iter()
        .map(|item| item.decision_id.clone())
        .collect();
    assert_eq!(listed, vec![fixture.second, fixture.first]);
    Ok(())
}

#[test]
fn recent_decisions_until_before_the_proposals_leaves_them_out_of_a_window_in_the_past(
) -> Result<()> {
    let fixture = back_dated_ledger()?;
    let graph = graph_of(&fixture.ledger)?;

    let response = get_recent_decisions(
        &fixture.ledger,
        &graph,
        &recent_request("2026-08-01T00:00:00Z", Some(BEFORE_THE_PROPOSALS)),
    )?;

    let listed: Vec<_> = response
        .data
        .items
        .iter()
        .map(|item| item.decision_id.clone())
        .collect();
    assert_eq!(listed, vec![fixture.imported]);
    Ok(())
}

#[test]
fn changed_since_a_time_between_them_lists_the_changes_a_back_dated_event_follows() -> Result<()> {
    let fixture = back_dated_ledger()?;

    let changes = changed_in(
        &fixture.ledger,
        ChangedSinceRequest {
            since_timestamp: Some(test_ts(BEFORE_THE_PROPOSALS)),
            limit: 25,
            ..ChangedSinceRequest::default()
        },
    )?;

    // The evidence dated 09:30 and the import dated 2026-09-01 are outside the window though
    // they were appended last.
    assert_eq!(
        event_types(&changes),
        vec![
            EventType::DecisionProposed,
            EventType::DecisionProposed,
            EventType::DecisionAccepted,
        ]
    );
    Ok(())
}

#[test]
fn changed_until_before_the_proposals_leaves_them_out_of_a_window_in_the_past() -> Result<()> {
    let fixture = back_dated_ledger()?;

    let changes = changed_in(
        &fixture.ledger,
        ChangedSinceRequest {
            until_timestamp: Some(test_ts(BEFORE_THE_PROPOSALS)),
            limit: 25,
            ..ChangedSinceRequest::default()
        },
    )?;

    // The proposals and the acceptance, recorded today, are outside a window that ends before
    // them though they were appended first.
    assert_eq!(
        event_types(&changes),
        vec![EventType::EvidenceRecorded, EventType::DecisionProposed]
    );
    Ok(())
}

#[test]
fn an_offset_the_caller_gives_still_bounds_a_time_window() -> Result<()> {
    let fixture = back_dated_ledger()?;
    let since_timestamp = Some(test_ts(BEFORE_THE_PROPOSALS));
    let both_proposals = changed_in(
        &fixture.ledger,
        ChangedSinceRequest {
            since_timestamp,
            limit: 25,
            ..ChangedSinceRequest::default()
        },
    )?;
    let (first_proposal, _) = both_proposals[0];

    let changes = changed_in(
        &fixture.ledger,
        ChangedSinceRequest {
            since_offset: Some(first_proposal),
            since_timestamp,
            limit: 25,
            ..ChangedSinceRequest::default()
        },
    )?;

    assert_eq!(
        event_types(&changes),
        vec![EventType::DecisionProposed, EventType::DecisionAccepted]
    );
    Ok(())
}

#[test]
fn added_since_a_time_between_them_lists_the_proposals_a_back_dated_event_follows() -> Result<()> {
    let fixture = back_dated_ledger()?;

    let (added, changed) = added_ids(
        &fixture.ledger,
        &DecisionsAddedSinceRequest {
            since_timestamp: Some(test_ts(BEFORE_THE_PROPOSALS)),
            limit: 25,
            ..DecisionsAddedSinceRequest::default()
        },
    )?;

    assert_eq!(added, vec![fixture.first, fixture.second]);
    assert!(changed.is_empty());
    Ok(())
}

#[test]
fn added_since_a_time_after_a_proposal_reports_its_later_change_not_the_back_dated_import(
) -> Result<()> {
    let fixture = back_dated_ledger()?;

    let (added, changed) = added_ids(
        &fixture.ledger,
        &DecisionsAddedSinceRequest {
            since_timestamp: Some(test_ts("2026-10-18T10:15:00Z")),
            limit: 25,
            ..DecisionsAddedSinceRequest::default()
        },
    )?;

    assert!(added.is_empty());
    assert_eq!(changed, vec![fixture.first]);
    Ok(())
}

#[test]
fn added_until_before_the_proposals_leaves_them_out_of_a_window_in_the_past() -> Result<()> {
    let fixture = back_dated_ledger()?;

    let (added, changed) = added_ids(
        &fixture.ledger,
        &DecisionsAddedSinceRequest {
            until_timestamp: Some(test_ts(BEFORE_THE_PROPOSALS)),
            limit: 25,
            ..DecisionsAddedSinceRequest::default()
        },
    )?;

    assert_eq!(added, vec![fixture.imported]);
    assert!(changed.is_empty());
    Ok(())
}

// A classified capture can replace a decision a `decision.proposed` recorded (`supersedes_id`).
// The ledger holds no `decision.superseded` for it, so the standing of the replaced decision is
// the graph's, as a draft's is (hivemind-u46nq).

/// A proposal recorded at 08:00 and a capture at 09:00 that replaces it; the replacement's id.
fn proposal_replaced_by_a_capture() -> Result<(Scenario, String)> {
    let scenario = Scenario::new();
    scenario.decision(
        "decision-old",
        "Use tabs",
        "human:alex",
        "2026-10-11T08:00:00Z",
    )?;
    let batch = scenario.classified_captures(
        CLASSIFIER,
        "2026-10-11T09:00:00Z",
        vec![decision_capture(
            "Use spaces",
            json!({"supersedes_id": "decision-old"}),
        )],
    )?;
    Ok((scenario, format!("capture:{batch}:0")))
}

#[test]
fn recent_decisions_list_a_proposal_a_capture_replaced_as_superseded() -> Result<()> {
    let (scenario, replacement) = proposal_replaced_by_a_capture()?;
    let graph = scenario.graph()?;

    let response = get_recent_decisions(
        scenario.ledger(),
        &graph,
        &recent_request("2026-10-11T07:00:00Z", None),
    )?;

    let status_of = |decision_id: &str| {
        response
            .data
            .items
            .iter()
            .find(|item| item.decision_id == decision_id)
            .map(|item| item.status)
    };
    assert_eq!(status_of("decision-old"), Some(DecisionStatus::Superseded));
    assert_eq!(status_of(&replacement), Some(DecisionStatus::Proposed));

    let superseded = RecentDecisionsRequest {
        filters: RecentDecisionFilterRequest {
            statuses: vec![DecisionStatus::Superseded],
            ..RecentDecisionFilterRequest::default()
        },
        ..recent_request("2026-10-11T07:00:00Z", None)
    };
    assert_eq!(
        recent_in(&scenario, &superseded)?,
        vec!["decision-old".to_owned()]
    );
    let proposed = RecentDecisionsRequest {
        filters: RecentDecisionFilterRequest {
            statuses: vec![DecisionStatus::Proposed],
            ..RecentDecisionFilterRequest::default()
        },
        ..recent_request("2026-10-11T07:00:00Z", None)
    };
    assert_eq!(recent_in(&scenario, &proposed)?, vec![replacement]);
    Ok(())
}

#[test]
fn added_since_gives_a_proposal_a_capture_replaced_its_standing() -> Result<()> {
    let (scenario, _) = proposal_replaced_by_a_capture()?;
    let graph = scenario.graph()?;

    let response = get_decisions_added_since(
        scenario.ledger(),
        &graph,
        &DecisionsAddedSinceRequest {
            since_timestamp: Some(test_ts("2026-10-11T07:00:00Z")),
            limit: 25,
            ..DecisionsAddedSinceRequest::default()
        },
    )?;

    let old = response
        .data
        .added_decisions
        .iter()
        .find(|entry| entry.decision_id == "decision-old")
        .ok_or_else(|| query_error("the proposal is added"))?;
    assert_eq!(old.status, DecisionStatus::Superseded);
    Ok(())
}

// ── a standing a capture states is a change, as the hand-recorded one is (hivemind-pf6xi) ──

/// `d:goal` replaced by a decision a classification accepts, and a second classification that
/// rejects the decision it records, on a ledger where `d:derived` follows `d:goal`. Returns the
/// scenario, the replacing classification's offset and decision, and the rejecting one's.
fn goal_replaced_and_a_capture_rejected() -> Result<(Scenario, EventId, String, EventId, String)> {
    let scenario = goal_with_a_derived_decision()?;
    let replacing = scenario.classified_captures(
        CLASSIFIER,
        "2026-10-11T09:00:00Z",
        vec![decision_capture(
            "Ship the self-hosted MVP",
            json!({"supersedes_id": "d:goal", "accepted_by": ["human:alice"]}),
        )],
    )?;
    let rejecting = scenario.classified_captures(
        CLASSIFIER,
        "2026-10-11T09:05:00Z",
        vec![decision_capture(
            "Nightly dumps go to S3",
            json!({"rejected_by": ["human:bob"]}),
        )],
    )?;
    Ok((
        scenario,
        replacing,
        format!("capture:{replacing}:0"),
        rejecting,
        format!("capture:{rejecting}:0"),
    ))
}

fn sorted(mut ids: Vec<String>) -> Vec<String> {
    ids.sort();
    ids
}

#[test]
fn changed_since_lists_what_a_capture_replaces_and_rejects_as_it_lists_the_hand_recorded(
) -> Result<()> {
    let (scenario, replacing, replacement, rejecting, rejected) =
        goal_replaced_and_a_capture_rejected()?;

    let rows = changed_since_start(scenario.ledger())?;

    let supersession = rows
        .iter()
        .find(|row| row.change_kind == HistoryChangeKind::Supersession)
        .expect("the capture's supersession is a row");
    assert_eq!(
        supersession.decision_ids,
        sorted(vec!["d:goal".to_owned(), replacement.clone()])
    );
    // Cited by the classification that stated it, as that event is cited everywhere else.
    assert_eq!(supersession.event_origin, replacing);
    assert_eq!(supersession.event_type, EventType::IngestBatchClassified);
    assert_eq!(supersession.actor_id, CLASSIFIER);
    assert_eq!(supersession.ts, Some(test_ts("2026-10-11T09:00:00Z")));
    assert_eq!(supersession.citation_id, format!("event:{replacing}"));

    let stale = rows
        .iter()
        .find(|row| row.change_kind == HistoryChangeKind::StalePremise)
        .expect("the decision that rested on the replaced one is told");
    assert_eq!(stale.decision_ids, vec!["d:derived".to_owned()]);
    assert_eq!(stale.event_origin, replacing);

    let rejection = rows
        .iter()
        .find(|row| {
            row.change_kind == HistoryChangeKind::StatusChange
                && row.event_type == EventType::IngestBatchClassified
        })
        .expect("the capture's rejection is a row");
    assert_eq!(rejection.decision_ids, vec![rejected]);
    assert_eq!(rejection.event_origin, rejecting);
    assert_eq!(rejection.event_type, EventType::IngestBatchClassified);

    // The classification keeps its own row, and nothing is said of the bystander.
    assert_eq!(
        rows.iter()
            .filter(|row| row.event_type == EventType::IngestBatchClassified
                && row.change_kind == HistoryChangeKind::ContextChange)
            .count(),
        2
    );
    assert!(!rows
        .iter()
        .any(|row| row.decision_ids.contains(&"d:bystander".to_owned())
            && row.change_kind != HistoryChangeKind::NewDecision));
    Ok(())
}

#[test]
fn recent_activity_lists_what_a_capture_replaces_and_rejects() -> Result<()> {
    let (scenario, replacing, _, rejecting, _) = goal_replaced_and_a_capture_rejected()?;

    let activity = get_recent_activity(
        scenario.ledger(),
        &scenario.graph()?,
        &RecentActivityRequest {
            limit: 100,
            ..RecentActivityRequest::default()
        },
    )?
    .data;

    let at = |origin: EventId, kind: HistoryChangeKind| {
        activity
            .items
            .iter()
            .filter(|row| row.event_origin == origin && row.change_kind == kind)
            .count()
    };
    assert_eq!(at(replacing, HistoryChangeKind::Supersession), 1);
    assert_eq!(at(replacing, HistoryChangeKind::StalePremise), 1);
    assert_eq!(at(rejecting, HistoryChangeKind::StatusChange), 1);
    Ok(())
}

#[test]
fn a_supersession_the_ledger_states_is_listed_once_though_the_graph_holds_its_edge() -> Result<()> {
    let scenario = goal_with_a_derived_decision()?;
    scenario.decision(
        "d:goal-2",
        "Ship the self-hosted MVP",
        "human:alex",
        "2026-02-01T00:00:00Z",
    )?;
    scenario.supersede("d:goal", "d:goal-2", "human:alex", "2026-02-01T00:00:01Z")?;

    let rows = changed_since_start(scenario.ledger())?;

    assert_eq!(
        rows.iter()
            .filter(|row| row.change_kind == HistoryChangeKind::Supersession)
            .count(),
        1
    );
    assert_eq!(
        rows.iter()
            .filter(|row| row.change_kind == HistoryChangeKind::StalePremise)
            .count(),
        1
    );
    Ok(())
}

#[test]
fn the_feeds_filter_what_a_capture_states_by_the_status_and_topic_the_graph_gives() -> Result<()> {
    let (scenario, _, replacement, _, rejected) = goal_replaced_and_a_capture_rejected()?;
    let listed = |filters: HistoryFilterRequest| -> Result<Vec<Vec<String>>> {
        Ok(get_decisions_changed_since(
            scenario.ledger(),
            &scenario.graph()?,
            &ChangedSinceRequest {
                since_offset: Some(0),
                filters,
                limit: 100,
                ..ChangedSinceRequest::default()
            },
        )?
        .data
        .items
        .into_iter()
        .filter(|row| row.event_type == EventType::IngestBatchClassified)
        .map(|row| row.decision_ids)
        .filter(|ids| !ids.is_empty())
        .collect())
    };

    // The graph's word: the replaced decision is superseded, the one a capture rejected is rejected,
    // and the replacement a capture accepted is accepted, which no event of the ledger says.
    assert_eq!(
        listed(HistoryFilterRequest {
            statuses: vec![DecisionStatus::Rejected],
            ..HistoryFilterRequest::default()
        })?,
        vec![vec![rejected.clone()]]
    );
    assert_eq!(
        listed(HistoryFilterRequest {
            statuses: vec![DecisionStatus::Accepted],
            ..HistoryFilterRequest::default()
        })?,
        vec![sorted(vec!["d:goal".to_owned(), replacement.clone()])]
    );
    // The topics a capture states are on the decision's node, not in a proposal.
    assert_eq!(
        listed(HistoryFilterRequest {
            topic_keys: vec!["capture".to_owned()],
            ..HistoryFilterRequest::default()
        })?,
        vec![
            sorted(vec!["d:goal".to_owned(), replacement]),
            vec![rejected]
        ]
    );
    assert!(listed(HistoryFilterRequest {
        topic_keys: vec!["unrelated".to_owned()],
        ..HistoryFilterRequest::default()
    })?
    .is_empty());
    Ok(())
}

#[test]
fn changed_since_a_capture_leaves_out_what_the_capture_stated_before_the_window() -> Result<()> {
    let (scenario, replacing, _, _, _) = goal_replaced_and_a_capture_rejected()?;

    let rows = get_decisions_changed_since(
        scenario.ledger(),
        &scenario.graph()?,
        &ChangedSinceRequest {
            since_offset: Some(replacing),
            limit: 100,
            ..ChangedSinceRequest::default()
        },
    )?
    .data
    .items;

    assert!(rows
        .iter()
        .all(|row| row.change_kind != HistoryChangeKind::Supersession
            && row.change_kind != HistoryChangeKind::StalePremise));
    assert!(rows
        .iter()
        .any(|row| row.change_kind == HistoryChangeKind::StatusChange));
    Ok(())
}

#[test]
fn a_feed_refuses_a_standing_whose_classification_the_ledger_does_not_hold() -> Result<()> {
    let (scenario, ..) = goal_replaced_and_a_capture_rejected()?;
    let graph = scenario.graph()?;

    // A window that quietly lost the standing would look complete.
    let missing = get_decisions_changed_since(
        &InMemoryEventLedger::new(),
        &graph,
        &ChangedSinceRequest::default(),
    );
    assert!(missing.is_err());
    let missing = get_decisions_added_since(
        &InMemoryEventLedger::new(),
        &graph,
        &DecisionsAddedSinceRequest::default(),
    );
    assert!(missing.is_err());
    Ok(())
}

#[test]
fn added_since_lists_the_decision_a_capture_replaced_and_the_one_resting_on_it_as_changed(
) -> Result<()> {
    let (scenario, replacing, replacement, _, rejected) = goal_replaced_and_a_capture_rejected()?;

    // The window opens at the first classification, after `d:goal` and `d:derived` were recorded.
    let data = get_decisions_added_since(
        scenario.ledger(),
        &scenario.graph()?,
        &DecisionsAddedSinceRequest {
            since_offset: Some(replacing - 1),
            limit: 100,
            ..DecisionsAddedSinceRequest::default()
        },
    )?
    .data;

    let changed = |decision_id: &str| {
        data.changed_existing_decisions
            .iter()
            .find(|entry| entry.decision_id == decision_id)
    };
    let goal = changed("d:goal").expect("the replaced decision is changed");
    assert_eq!(goal.status, DecisionStatus::Superseded);
    assert_eq!(
        goal.changes_in_window
            .iter()
            .map(|change| (change.change_kind, change.provenance.event_origin))
            .collect::<Vec<_>>(),
        vec![(HistoryChangeKind::Supersession, replacing)]
    );
    let derived = changed("d:derived").expect("the decision resting on it is changed");
    assert_eq!(
        derived
            .changes_in_window
            .iter()
            .map(|change| change.change_kind)
            .collect::<Vec<_>>(),
        vec![HistoryChangeKind::StalePremise]
    );
    assert!(changed("d:bystander").is_none());

    // The decisions the captures record are added, with the standing the captures state; their own
    // replacing and rejecting is no change to them.
    let added = |decision_id: &str| {
        data.added_decisions
            .iter()
            .find(|entry| entry.decision_id == decision_id)
    };
    let replacement = added(&replacement).expect("the replacing decision is added");
    assert_eq!(replacement.status, DecisionStatus::Accepted);
    assert!(replacement.changes_in_window.is_empty());
    let rejected = added(&rejected).expect("the rejected decision is added");
    assert_eq!(rejected.status, DecisionStatus::Rejected);
    assert!(rejected.changes_in_window.is_empty());
    Ok(())
}

// ── a history read costs the events it answers from, not the transcripts around them (hivemind-gxypz) ──

/// What the two feeds and the two decision lists answer over the whole ledger, without the ledger
/// range, which counts every event, the padding of the test below included.
fn history_answers(
    ledger: &impl EventLedger,
    graph: &crate::projector::memory::MemoryGraph,
) -> Result<Vec<Value>> {
    fn answer<T: Serialize>(data: &T) -> Result<Value> {
        let mut answer = serde_json::to_value(data).map_err(json_query_error)?;
        if let Some(object) = answer.as_object_mut() {
            object.remove("ledger_range");
            object.remove("resolved_until");
        }
        Ok(answer)
    }

    Ok(vec![
        answer(
            &get_recent_activity(
                ledger,
                graph,
                &RecentActivityRequest {
                    limit: 500,
                    ..RecentActivityRequest::default()
                },
            )?
            .data,
        )?,
        answer(
            &get_decisions_changed_since(
                ledger,
                graph,
                &ChangedSinceRequest {
                    since_offset: Some(0),
                    limit: 500,
                    ..ChangedSinceRequest::default()
                },
            )?
            .data,
        )?,
        answer(
            &get_recent_decisions(ledger, graph, &recent_request("2000-01-01T00:00:00Z", None))?
                .data,
        )?,
        answer(
            &get_decisions_added_since(
                ledger,
                graph,
                &DecisionsAddedSinceRequest {
                    since_offset: Some(0),
                    limit: 500,
                    ..DecisionsAddedSinceRequest::default()
                },
            )?
            .data,
        )?,
    ])
}

#[test]
fn the_history_reads_answer_the_same_among_transcripts_and_never_read_the_whole_ledger(
) -> Result<()> {
    // Each of these read every event of the ledger and validated it, ingest transcripts and all:
    // on a cell of 130k events, 127k of them transcripts, a list of a dozen decisions took
    // seconds. The ledger below refuses a whole read, so a read that does not name its types
    // fails; the drafts are found by the classification event ids the graph names.
    let (scenario, said_today, said_earlier) = drafts_scenario()?;
    scenario.decision(
        "decision-next",
        "Replaces the hand recorded one",
        "human:alex",
        "2026-10-11T10:00:00Z",
    )?;
    scenario.supersede(
        "decision-hand",
        "decision-next",
        "human:alex",
        "2026-10-11T10:01:00Z",
    )?;
    scenario.evidence("evidence-a", "Measured", None, "2026-10-11T10:02:00Z")?;
    let graph = scenario.graph()?;

    let plain = history_answers(scenario.ledger(), &graph)?;
    let listed = |answer: &Value, key: &str| -> Vec<String> {
        answer[key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|entry| entry["decision_id"].as_str().map(str::to_owned))
            .collect()
    };
    assert!(plain[0]["items"]
        .as_array()
        .is_some_and(|rows| rows.len() > 5));
    for draft in [&said_today, &said_earlier] {
        assert!(listed(&plain[2], "items").contains(draft));
        assert!(listed(&plain[3], "added_decisions").contains(draft));
    }
    // From offset 0 every decision is added; the supersession is a change inside its window.
    let hand = plain[3]["added_decisions"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|entry| entry["decision_id"] == "decision-hand")
        .expect("the hand-recorded decision is added");
    assert_eq!(hand["status"], "superseded");
    assert_eq!(hand["changes_in_window"][0]["change_kind"], "supersession");

    assert_eq!(
        history_answers(&TypedReadsOnly(scenario.ledger()), &graph)?,
        plain,
        "naming its types changes no answer"
    );

    // Transcripts after everything else, so no other event moves: nothing may be added or dropped.
    for index in 0..40 {
        scenario.received_batch(
            &format!("batch-{index}"),
            &format!("session-{index}"),
            "2026-10-12T00:00:00Z",
        )?;
    }
    assert_eq!(
        history_answers(&TypedReadsOnly(scenario.ledger()), &graph)?,
        plain,
        "a ledger padded with transcripts answers the same"
    );

    // The feeds do not list a transcript: the classification that records what it decided is the
    // event with a row.
    let activity = get_recent_activity(
        &TypedReadsOnly(scenario.ledger()),
        &graph,
        &RecentActivityRequest {
            limit: 500,
            ..RecentActivityRequest::default()
        },
    )?
    .data;
    assert!(activity
        .items
        .iter()
        .all(|row| row.event_type != EventType::IngestBatchReceived));
    assert!(activity
        .items
        .iter()
        .any(|row| row.event_type == EventType::IngestBatchClassified));
    Ok(())
}

#[test]
fn the_types_a_history_read_names_leave_out_nothing_it_answers_from() -> Result<()> {
    // The reads once walked every event. Each scenario holds the event types a ledger carries
    // beside decisions (evidence, hypotheses, questions, blockers, notifications, assessments,
    // moves, reaffirmations); the answers over the named types must be the answers over all of
    // them. The feeds list no raw transcript, so their whole-ledger answer leaves those out too.
    let (drafts, _, _) = drafts_scenario()?;
    let scenarios = [
        ("drafts", drafts),
        ("attention", attention_scenario()?),
        ("floor", floor_scenario()?),
        ("reexamine", reexamine_scenario()?),
        ("revisions", revisions_scenario()?),
        ("reaffirmation", reaffirmation_scenario()?),
    ];
    for (name, scenario) in &scenarios {
        let graph = scenario.graph()?;
        let named = history_answers(&TypedReadsOnly(scenario.ledger()), &graph)?;
        let whole = history_answers(
            &EveryEventRead {
                ledger: scenario.ledger(),
                without_received: false,
            },
            &graph,
        )?;
        let feeds = history_answers(
            &EveryEventRead {
                ledger: scenario.ledger(),
                without_received: true,
            },
            &graph,
        )?;
        assert_eq!(named[0], feeds[0], "{name}: get_recent_activity");
        assert_eq!(named[1], feeds[1], "{name}: get_decisions_changed_since");
        assert_eq!(named[2], whole[2], "{name}: get_recent_decisions");
        assert_eq!(named[3], whole[3], "{name}: get_decisions_added_since");
        assert!(
            named[0]["items"]
                .as_array()
                .is_some_and(|rows| !rows.is_empty()),
            "{name}: the feed lists something"
        );
    }
    Ok(())
}

#[test]
fn the_feeds_list_every_event_type_but_the_raw_transcripts() -> Result<()> {
    // A type the list forgets would drop out of the feeds without a word. Every type has a v0
    // fixture, so a new type that the list lacks fails here.
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v0");
    let mut checked = 0;
    for entry in std::fs::read_dir(&fixtures).map_err(query_error)? {
        let path = entry.map_err(query_error)?.path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let event: Event =
            serde_json::from_str(&std::fs::read_to_string(&path).map_err(query_error)?)
                .map_err(query_error)?;
        assert_eq!(
            FEED_EVENT_TYPES.contains(&event.event_type),
            event.event_type != EventType::IngestBatchReceived,
            "{}",
            path.display()
        );
        checked += 1;
    }
    assert!(checked > 0, "the fixtures were found");
    Ok(())
}
