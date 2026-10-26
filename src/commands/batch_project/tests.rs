// Parent module gates this file with #[cfg(test)]; repeat the marker so UBS can filter test-only assertions.
#[cfg(test)]
use serde_json::json;

use crate::commands::{
    personal_project_handle, ClassifiedBatchRecorded, CommandContext, Commands, DeterminedProject,
    ProjectRenameInput,
};
use crate::events::{
    CaptureItem, Event, EventProvenance, EventType, ProjectLinkKind, ProjectSource, TenantId,
};
use crate::ledger::{EventLedger, InMemoryEventLedger};
use crate::projector::{memory::MemoryGraph, rebuild_graph_for_tenant, GraphValue, NodeKind};

use super::nearest_common_project;

const SUBMITTER: &str = "agent:claude:hook";
const RECORDER: &str = "agent:loomtracer:classifier";
const ADMIN: &str = "human:admin";

fn commands(ledger: &InMemoryEventLedger) -> Commands<'_, InMemoryEventLedger> {
    Commands::new_with_context(
        ledger,
        CommandContext::new(TenantId::local(), EventProvenance::agent(RECORDER)),
    )
}

fn ledger_with_projects(handles: &[&str]) -> InMemoryEventLedger {
    let ledger = InMemoryEventLedger::new();
    for handle in handles {
        Commands::new(&ledger)
            .register_project(ADMIN, handle, None, None)
            .expect("project registers");
    }
    ledger
}

fn marker(handle: &str) -> Option<DeterminedProject<'_>> {
    Some(DeterminedProject {
        handle,
        source: ProjectSource::FolderMarker,
    })
}

/// Receives `batch_id`, naming `project` (or none).
fn receive(
    ledger: &InMemoryEventLedger,
    batch_id: &str,
    project: Option<DeterminedProject<'_>>,
) -> crate::Result<crate::events::EventId> {
    commands(ledger).record_ingest_batch_for_project(
        SUBMITTER,
        batch_id,
        "claude",
        "session-1",
        Vec::new(),
        project,
    )
}

fn decision(title: &str) -> CaptureItem {
    serde_json::from_value(json!({
        "kind": "decision",
        "title": title,
        "rationale": "it scales and the team knows it",
        "topic_keys": ["storage"],
        "evidence_ids": [],
        "options": ["postgres", "mysql"],
        "chosen_option": "postgres",
        "extraction_confidence": 0.9
    }))
    .expect("capture parses")
}

fn classify(
    ledger: &InMemoryEventLedger,
    batch_ids: &[&str],
) -> crate::Result<ClassifiedBatchRecorded> {
    let batch_ids: Vec<String> = batch_ids.iter().map(|id| (*id).to_owned()).collect();
    commands(ledger).record_ingest_batch_classified(
        RECORDER,
        &batch_ids,
        "claude-haiku-4-5-20251001",
        "2",
        vec![decision("Use Postgres for storage")],
        None,
    )
}

fn events_of(ledger: &InMemoryEventLedger, event_type: EventType) -> Vec<Event> {
    ledger
        .read(0, 1000)
        .expect("ledger reads")
        .into_iter()
        .filter(|event| event.event_type == event_type)
        .collect()
}

fn classification_event(ledger: &InMemoryEventLedger, recorded: &ClassifiedBatchRecorded) -> Event {
    ledger
        .read(recorded.event_id - 1, 1)
        .expect("ledger reads")
        .remove(0)
}

/// The `(project, project_source)` of the Decision node the first capture of `recorded` projects.
fn node_project(
    ledger: &InMemoryEventLedger,
    recorded: &ClassifiedBatchRecorded,
) -> (String, String) {
    let graph = MemoryGraph::default();
    rebuild_graph_for_tenant(ledger, &TenantId::local(), &graph).expect("graph rebuilds");
    let (nodes, _) = graph.nodes_and_edges().expect("graph reads");
    let decision_id = format!("capture:{}:0", recorded.event_id);
    let properties = nodes
        .get(&(NodeKind::Decision, decision_id))
        .expect("the decision is projected");
    let text = |key: &str| match properties.get(key) {
        Some(GraphValue::String(value)) => value.clone(),
        other => panic!("{key} is not a string: {other:?}"),
    };
    (text("project"), text("project_source"))
}

#[test]
fn a_batch_records_the_project_its_client_named_and_how_it_found_it() {
    let ledger = ledger_with_projects(&["billing"]);

    receive(&ledger, "b1", marker("billing")).expect("batch is received");

    let received = events_of(&ledger, EventType::IngestBatchReceived);
    assert_eq!(received[0].payload["project"], "billing");
    assert_eq!(received[0].payload["project_source"], "folder_marker");
}

#[test]
fn a_batch_naming_a_project_with_no_source_is_stated() {
    let ledger = ledger_with_projects(&["billing"]);

    receive(&ledger, "b1", Some(DeterminedProject::stated("billing"))).expect("received");

    let received = events_of(&ledger, EventType::IngestBatchReceived);
    assert_eq!(received[0].payload["project_source"], "stated");
}

#[test]
fn a_batch_naming_no_project_records_neither_field() {
    let ledger = ledger_with_projects(&["billing"]);

    receive(&ledger, "b1", None).expect("received");

    let received = events_of(&ledger, EventType::IngestBatchReceived);
    assert!(received[0].payload.get("project").is_none());
    assert!(received[0].payload.get("project_source").is_none());
}

#[test]
fn a_project_nobody_registered_is_refused_with_the_register_command_and_nothing_is_recorded() {
    let ledger = ledger_with_projects(&["billing"]);

    let error = receive(&ledger, "b1", marker("biling")).expect_err("a typo is refused");

    let message = error.to_string();
    assert!(
        message.contains("project not registered: biling"),
        "{message}"
    );
    assert!(
        message.contains("loomtracer project register biling"),
        "{message}"
    );
    assert!(events_of(&ledger, EventType::IngestBatchReceived).is_empty());
}

#[test]
fn a_personal_address_cannot_be_named_by_a_batch() {
    let ledger = ledger_with_projects(&[]);

    let error = receive(&ledger, "b1", marker("personal:agent:claude")).expect_err("refused");

    assert!(error.to_string().contains("personal"), "{error}");
    assert!(events_of(&ledger, EventType::IngestBatchReceived).is_empty());
}

#[test]
fn a_source_only_the_write_layer_records_cannot_come_with_a_handle() {
    let ledger = ledger_with_projects(&["billing"]);

    for source in [
        ProjectSource::PersonalFallback,
        ProjectSource::Moved,
        ProjectSource::Inherited,
    ] {
        let error = receive(
            &ledger,
            "b1",
            Some(DeterminedProject {
                handle: "billing",
                source,
            }),
        )
        .expect_err("refused");
        assert!(error.to_string().contains("cannot accompany"), "{error}");
    }
    assert!(events_of(&ledger, EventType::IngestBatchReceived).is_empty());
}

#[test]
fn a_classification_files_its_decisions_under_the_project_its_batch_named() {
    let ledger = ledger_with_projects(&["billing"]);
    receive(&ledger, "b1", marker("billing")).expect("received");

    let recorded = classify(&ledger, &["b1"]).expect("classification is recorded");

    let event = classification_event(&ledger, &recorded);
    assert_eq!(event.payload["project"], "billing");
    assert_eq!(event.payload["project_source"], "folder_marker");
    assert_eq!(
        node_project(&ledger, &recorded),
        ("billing".to_owned(), "folder_marker".to_owned())
    );
    let mut reply = json!({});
    recorded.annotate_reply(&mut reply);
    assert_eq!(reply["project"], "billing");
    assert_eq!(reply["project_source"], "folder_marker");
    assert!(reply.get("project_notice").is_none());
}

#[test]
fn a_stated_project_is_filed_as_stated() {
    let ledger = ledger_with_projects(&["billing"]);
    receive(&ledger, "b1", Some(DeterminedProject::stated("billing"))).expect("received");

    let recorded = classify(&ledger, &["b1"]).expect("recorded");

    assert_eq!(
        node_project(&ledger, &recorded),
        ("billing".to_owned(), "stated".to_owned())
    );
}

#[test]
fn batches_that_name_no_project_stay_in_the_recorders_personal_project() {
    let ledger = ledger_with_projects(&["billing"]);
    receive(&ledger, "b1", None).expect("received");

    let recorded = classify(&ledger, &["b1"]).expect("recorded");

    let event = classification_event(&ledger, &recorded);
    assert!(event.payload.get("project").is_none());
    assert!(event.payload.get("project_source").is_none());
    assert_eq!(
        node_project(&ledger, &recorded),
        (
            personal_project_handle(RECORDER),
            "personal_fallback".to_owned()
        )
    );
    let mut reply = json!({});
    recorded.annotate_reply(&mut reply);
    assert_eq!(
        reply,
        json!({}),
        "a reply says nothing new when no batch named a project"
    );
}

#[test]
fn a_classification_of_a_batch_the_ledger_never_received_stays_personal() {
    // `emit ingest.batch_classified` names a batch id it makes up for text that never arrived.
    let ledger = ledger_with_projects(&["billing"]);

    let recorded = classify(&ledger, &["made-up"]).expect("recorded");

    assert_eq!(
        node_project(&ledger, &recorded).1,
        "personal_fallback".to_owned()
    );
}

#[test]
fn a_batch_that_names_no_project_does_not_vote_against_the_ones_that_do() {
    let ledger = ledger_with_projects(&["billing"]);
    receive(&ledger, "early", None).expect("received before the client was configured");
    receive(&ledger, "later", marker("billing")).expect("received");

    let recorded = classify(&ledger, &["early", "later"]).expect("recorded");

    assert_eq!(
        node_project(&ledger, &recorded),
        ("billing".to_owned(), "folder_marker".to_owned())
    );
    let mut reply = json!({});
    recorded.annotate_reply(&mut reply);
    assert!(
        reply.get("project_notice").is_none(),
        "one project is no span"
    );
}

#[test]
fn the_source_is_the_first_covered_batch_that_named_a_project() {
    let ledger = ledger_with_projects(&["billing"]);
    receive(&ledger, "b1", Some(DeterminedProject::stated("billing"))).expect("received");
    receive(&ledger, "b2", marker("billing")).expect("received");

    let in_order = classify(&ledger, &["b1", "b2"]).expect("recorded");
    let reversed = classify(&ledger, &["b2", "b1"]).expect("recorded");

    assert_eq!(node_project(&ledger, &in_order).1, "stated");
    assert_eq!(node_project(&ledger, &reversed).1, "folder_marker");
}

#[test]
fn a_batch_received_twice_keeps_the_project_of_its_first_receipt() {
    let ledger = ledger_with_projects(&["billing", "platform"]);
    receive(&ledger, "b1", marker("billing")).expect("received");
    receive(&ledger, "b1", marker("platform")).expect("received again");

    let recorded = classify(&ledger, &["b1"]).expect("recorded");

    assert_eq!(node_project(&ledger, &recorded).0, "billing");
}

#[test]
fn a_span_sent_again_with_its_project_is_filed_under_it() {
    // A cell that predates project filing recorded the batch with no project; the client holds
    // the span back and sends it again once the cell is updated.
    let ledger = ledger_with_projects(&["billing"]);
    receive(&ledger, "b1", None).expect("received without a project");
    receive(&ledger, "b1", marker("billing")).expect("received again with one");

    let recorded = classify(&ledger, &["b1"]).expect("recorded");

    assert_eq!(
        node_project(&ledger, &recorded),
        ("billing".to_owned(), "folder_marker".to_owned())
    );
}

fn ledger_with_billing_and_payments_under_platform() -> InMemoryEventLedger {
    let ledger = ledger_with_projects(&["platform", "billing", "payments", "unrelated"]);
    let commands = Commands::new(&ledger);
    for child in ["billing", "payments"] {
        commands
            .link_project(ADMIN, child, "platform", ProjectLinkKind::PartOf)
            .expect("link records");
    }
    ledger
}

#[test]
fn batches_naming_sibling_projects_are_filed_under_the_parent_they_share() {
    let ledger = ledger_with_billing_and_payments_under_platform();
    receive(&ledger, "b1", marker("billing")).expect("received");
    receive(&ledger, "b2", marker("payments")).expect("received");

    let recorded = classify(&ledger, &["b1", "b2"]).expect("recorded");

    assert_eq!(
        node_project(&ledger, &recorded),
        ("platform".to_owned(), "folder_marker".to_owned())
    );
    let mut reply = json!({});
    recorded.annotate_reply(&mut reply);
    assert_eq!(reply["project"], "platform");
    assert_eq!(
        reply["project_notice"],
        "recorded for platform: these batches span billing and payments"
    );
}

#[test]
fn batches_naming_a_project_and_its_sub_project_are_filed_under_the_project() {
    let ledger = ledger_with_billing_and_payments_under_platform();
    receive(&ledger, "b1", marker("billing")).expect("received");
    receive(&ledger, "b2", marker("platform")).expect("received");

    let recorded = classify(&ledger, &["b1", "b2"]).expect("recorded");

    assert_eq!(node_project(&ledger, &recorded).0, "platform");
}

#[test]
fn batches_naming_projects_with_no_parent_in_common_are_saved_personally_and_say_so() {
    let ledger = ledger_with_billing_and_payments_under_platform();
    receive(&ledger, "b1", marker("billing")).expect("received");
    receive(&ledger, "b2", marker("unrelated")).expect("received");

    let recorded = classify(&ledger, &["b1", "b2"]).expect("recorded");

    let event = classification_event(&ledger, &recorded);
    assert!(event.payload.get("project").is_none());
    assert_eq!(
        node_project(&ledger, &recorded),
        (
            personal_project_handle(RECORDER),
            "personal_fallback".to_owned()
        )
    );
    let mut reply = json!({});
    recorded.annotate_reply(&mut reply);
    assert!(reply.get("project").is_none());
    let notice = reply["project_notice"].as_str().expect("a notice");
    assert!(
        notice.starts_with("these batches span billing and unrelated, which share no parent"),
        "{notice}"
    );
}

#[test]
fn a_part_of_link_that_was_unlinked_no_longer_joins_projects() {
    let ledger = ledger_with_billing_and_payments_under_platform();
    Commands::new(&ledger)
        .unlink_project(ADMIN, "payments", "platform", ProjectLinkKind::PartOf)
        .expect("unlink records");
    receive(&ledger, "b1", marker("billing")).expect("received");
    receive(&ledger, "b2", marker("payments")).expect("received");

    let recorded = classify(&ledger, &["b1", "b2"]).expect("recorded");

    assert_eq!(
        node_project(&ledger, &recorded).1,
        "personal_fallback",
        "payments is no longer part of platform"
    );
}

#[test]
fn a_part_of_cycle_ends_the_walk() {
    let ledger = ledger_with_projects(&["alpha", "beta", "gamma"]);
    let commands = Commands::new(&ledger);
    commands
        .link_project(ADMIN, "alpha", "beta", ProjectLinkKind::PartOf)
        .expect("link records");
    commands
        .link_project(ADMIN, "beta", "alpha", ProjectLinkKind::PartOf)
        .expect("a cycle is not forbidden");
    receive(&ledger, "b1", marker("alpha")).expect("received");
    receive(&ledger, "b2", marker("gamma")).expect("received");

    let recorded = classify(&ledger, &["b1", "b2"]).expect("terminates and records");

    assert_eq!(node_project(&ledger, &recorded).1, "personal_fallback");
}

#[test]
fn the_classifier_cannot_name_the_project_of_a_capture() {
    let attempt = serde_json::from_value::<CaptureItem>(json!({
        "kind": "decision",
        "title": "Use Postgres for storage",
        "rationale": "it scales",
        "topic_keys": [],
        "evidence_ids": [],
        "options": null,
        "chosen_option": null,
        "extraction_confidence": 0.9,
        "project": "billing"
    }));

    assert!(attempt.is_err(), "a capture has no project field to set");
}

#[test]
fn a_decision_filed_under_a_project_moves_from_that_project_and_not_from_the_personal_one() {
    let ledger = ledger_with_projects(&["billing", "platform"]);
    receive(&ledger, "b1", marker("billing")).expect("received");
    let recorded = classify(&ledger, &["b1"]).expect("recorded");
    let decision_id = format!("capture:{}:0", recorded.event_id);
    let commands = Commands::new(&ledger);

    assert_eq!(
        commands
            .current_decision_project(&decision_id)
            .expect("project"),
        Some("billing".to_owned())
    );
    let moved = commands
        .move_decision_to("human:alice", &decision_id, "platform", None)
        .expect("a filed draft moves");
    assert_eq!(moved.from, "billing");
    assert_eq!(moved.to, "platform");
}

#[test]
fn a_rename_carries_the_decisions_a_classification_filed() {
    let ledger = ledger_with_projects(&["billing"]);
    receive(&ledger, "b1", marker("billing")).expect("received");
    let recorded = classify(&ledger, &["b1"]).expect("recorded");
    let decision_id = format!("capture:{}:0", recorded.event_id);
    let commands = Commands::new(&ledger);

    let outcome = commands
        .rename_project(ProjectRenameInput {
            actor_id: ADMIN,
            from: "billing",
            to: "invoicing",
            display_name: None,
            purpose: None,
        })
        .expect("rename succeeds");

    assert_eq!(outcome.decisions_moved, 1);
    assert_eq!(
        commands
            .current_decision_project(&decision_id)
            .expect("project"),
        Some("invoicing".to_owned())
    );
}

#[test]
fn the_topic_keys_of_a_filed_decision_count_as_in_use_when_a_project_adopts_its_vocabulary() {
    let ledger = ledger_with_projects(&["billing"]);
    receive(&ledger, "b1", marker("billing")).expect("received");
    classify(&ledger, &["b1"]).expect("recorded");

    let declared = Commands::new(&ledger)
        .declare_topics_in_use(ADMIN, "billing")
        .expect("adoption succeeds");

    let keys: Vec<&str> = declared.iter().map(|d| d.topic_key.as_str()).collect();
    assert_eq!(keys, vec!["storage"]);
}

#[test]
fn a_replacement_of_a_filed_decision_inherits_its_project() {
    let ledger = ledger_with_projects(&["billing"]);
    receive(&ledger, "b1", marker("billing")).expect("received");
    let recorded = classify(&ledger, &["b1"]).expect("recorded");
    let decision_id = format!("capture:{}:0", recorded.event_id);

    let filing = Commands::new(&ledger)
        .current_decision_filing(&decision_id)
        .expect("filing")
        .expect("the draft is recorded");

    assert_eq!(
        filing.inherited_by_replacement(),
        (Some("billing".to_owned()), ProjectSource::Inherited)
    );
}

#[test]
fn the_nearest_project_every_chain_holds_is_the_first_of_the_first_chain() {
    let chains: Vec<Vec<&str>> = vec![
        vec!["billing", "platform", "company"],
        vec!["payments", "platform", "company"],
    ];
    assert_eq!(nearest_common_project(&chains), Some("platform".to_owned()));
    assert_eq!(
        nearest_common_project(&[vec!["a"], vec!["b"]]),
        None,
        "chains that share nothing"
    );
    assert_eq!(nearest_common_project(&[]), None);
}
