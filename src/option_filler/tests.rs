// Parent module gates this file with #[cfg(test)]; repeat the marker so UBS can filter test-only assertions.
#[cfg(test)]
use std::io::Cursor;
use std::path::{Path, PathBuf};

use clap::Parser;
use serde_json::{json, Value};

use super::*;
use crate::cli::{run, Cli};
use crate::commands::{Commands, DecisionProposalInput, FloorInput, Grounding, SupersedeInput};
use crate::events::EventType;
use crate::ingest::{
    extract_slack_decision_draft, import_documents, import_slack_thread,
    parse_slack_thread_fixture, DocumentConflictResolutionAction, DocumentImportFormat,
    DocumentImportRequest, DEFAULT_SLACK_MENTION,
};
use crate::ledger::{EventLedger, InMemoryEventLedger, SqliteEventLedger};
use crate::mcp::{serve, McpConfig};
use crate::projector::{memory::MemoryGraph, rebuild_graph};
use crate::quality_profile::{quality_profile_of, Level};

fn unique_dir(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "loomtracer-option-filler-{label}-{}",
        uuid::Uuid::new_v4()
    ))
}

/// The id of the last decision proposed in the ledger.
fn last_proposed_decision_id(ledger: &impl EventLedger) -> String {
    ledger
        .read(0, 1000)
        .expect("read succeeds")
        .into_iter()
        .rev()
        .find(|event| event.event_type == EventType::DecisionProposed)
        .and_then(|event| {
            event
                .payload
                .get("decision_id")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .expect("a decision was proposed")
}

/// The Alternatives level the floor gives a decision, read the way every reader reads it: by
/// replaying the ledger into the graph.
fn alternatives_level(ledger: &impl EventLedger, decision_id: &str) -> Option<Level> {
    let graph = MemoryGraph::default();
    rebuild_graph(ledger, &graph).expect("ledger replays");
    quality_profile_of(&graph, decision_id)
        .expect("profile reads")
        .expect("the decision has a profile")
        .alternatives
        .level()
}

// ── the recogniser reads what each surface writes ───────────────────────────

#[test]
fn every_surfaces_filler_is_recognised_as_generated() {
    let filler = [
        mcp("Direct"),
        cli("Direct"),
        supersede("Direct"),
        http("Direct"),
        slack("Direct", "slack://T123/C456/1715970800.000100"),
        import("block-7"),
    ];
    for description in filler {
        assert!(
            is_generated("Direct", &description),
            "{description:?} must read as generated"
        );
    }
}

#[test]
fn filler_for_another_option_is_not_filler() {
    assert!(!is_generated("Direct", &mcp("Queue")));
    assert!(!is_generated(
        "Direct",
        &slack("Queue", "slack://T1/C1/1.0")
    ));
}

#[test]
fn words_after_the_quoted_fillers_are_the_authors_own() {
    for filler in [
        mcp("Direct"),
        cli("Direct"),
        supersede("Direct"),
        http("Direct"),
    ] {
        let description = format!("{filler} -- rejected: extra hop for no gain");
        assert!(
            !is_generated("Direct", &description),
            "{description:?} is the author's own"
        );
    }
}

/// The documented exception: the Slack and import fillers end in a tail the recogniser cannot
/// tell from added words, so anything after the fixed text still reads as generated.
#[test]
fn words_after_the_slack_and_import_fillers_still_read_as_generated() {
    assert!(is_generated(
        "Direct",
        &format!("{} and a reason", slack("Direct", "slack://T1/C1/1.0"))
    ));
    assert!(is_generated(
        "Direct",
        &format!("{} and a reason", import("block-7"))
    ));
}

// ── an option with no description, captured through each surface, reads as partial ──────────
//
// Two options, one chosen, neither described: the alternative is the one the floor looks at.
// Were a surface's filler to read as the author's own, the Alternatives floor would read solid.

#[test]
fn mcp_capture_without_an_option_description_reads_as_partial() {
    let dir = unique_dir("mcp");
    let config = McpConfig::new(&dir).with_session_id("option-filler-session");
    let capture = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "capture_decision",
            "arguments": {
                "binds": {"level": "something", "statement": "Later work builds on it"},
                "undo_cost": {"level": "cheap"},
                "grounding": [{"kind": "bet"}],
                "actor_id": "agent:test:option-filler",
                "title": "Use JWT for auth tokens",
                "rationale": "Stateless tokens reduce session storage overhead",
                "topic_keys": ["auth"],
                "options": [{"label": "jwt"}, {"label": "opaque"}],
                "chosen_option_label": "jwt"
            }
        }
    })
    .to_string();
    let mut output: Vec<u8> = Vec::new();
    serve(&config, Cursor::new(format!("{capture}\n")), &mut output).expect("server loop");
    assert!(
        !String::from_utf8(output)
            .expect("utf-8 output")
            .contains("\"isError\":true"),
        "the capture succeeds"
    );

    let ledger = SqliteEventLedger::open(&dir).expect("ledger opens");
    let decision_id = last_proposed_decision_id(&ledger);

    assert_eq!(
        alternatives_level(&ledger, &decision_id),
        Some(Level::Partial)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cli_capture_without_an_option_description_reads_as_partial() {
    let dir = unique_dir("cli");
    run(&Cli::parse_from([
        "loomtracer",
        "--actor",
        "human:alice",
        "--loomtracer-dir",
        dir.to_str().expect("utf-8 temp path"),
        "emit",
        "decision.capture",
        "--title",
        "Use JWT for auth tokens",
        "--rationale",
        "Stateless tokens reduce session storage overhead",
        "--topic-keys",
        "auth",
        "--options",
        "jwt,opaque",
        "--chose",
        "jwt",
        "--rests-on-assumption",
        "Tokens are checked on every request",
        "--binds",
        "Later auth work builds on the token format",
        "--cheap-to-undo",
    ]))
    .expect("capture succeeds");

    let ledger = SqliteEventLedger::open(&dir).expect("ledger opens");
    let decision_id = last_proposed_decision_id(&ledger);

    assert_eq!(
        alternatives_level(&ledger, &decision_id),
        Some(Level::Partial)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn supersede_without_an_option_description_reads_as_partial() {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let option_id = commands
        .record_option("human:alice", "Keep", "Keep the current store")
        .expect("option records");
    let old_decision_id = commands
        .propose_decision(DecisionProposalInput {
            grounding: Grounding::NotAsked,
            expressed_confidence: None,
            actor_id: "human:alice",
            title: "Keep the current store",
            rationale: "Nothing forces a move yet",
            topic_keys: &["storage".to_owned()],
            option_ids: std::slice::from_ref(&option_id),
            option_labels: &["Keep".to_owned()],
            chosen_option_id: Some(option_id.as_str()),
            decided_by: None,
            delegated_by: None,
            still_proposed: false,
            hypothesis_ids: &[],
            evidence_ids: &[],
            quote: None,
            question: None,
            project: None,
            floor: FloorInput::NotAsked,
        })
        .expect("decision proposes");

    let outcome = commands
        .supersede(SupersedeInput {
            actor_id: "human:alice",
            old_decision_id: &old_decision_id,
            new_title: "Move to a new store",
            new_rationale: "The old store no longer fits the load",
            topic_keys: &["storage".to_owned()],
            option_labels: &["Move".to_owned(), "Keep".to_owned()],
            chosen_option_label: Some("Move"),
            still_proposed: false,
            hypothesis_ids: &[],
            evidence_ids: &[],
            project: None,
            grounding: None,
            expressed_confidence: None,
            floor: FloorInput::NotAsked,
        })
        .expect("supersede succeeds");

    assert_eq!(
        alternatives_level(&ledger, &outcome.new_decision_id),
        Some(Level::Partial)
    );
}

#[test]
fn slack_ingest_without_an_option_description_reads_as_partial() {
    let thread = parse_slack_thread_fixture(include_str!(
        "../../tests/fixtures/slack/thread_with_mention.json"
    ))
    .expect("fixture parses");
    let draft =
        extract_slack_decision_draft(&thread, DEFAULT_SLACK_MENTION).expect("thread extracts");
    let ledger = InMemoryEventLedger::new();

    let outcome = import_slack_thread(&ledger, &draft).expect("import succeeds");

    assert_eq!(
        alternatives_level(&ledger, outcome.decision_id()),
        Some(Level::Partial)
    );
}

#[test]
fn document_import_without_an_option_description_reads_as_partial() {
    let ledger = InMemoryEventLedger::new();
    import_documents(
        &ledger,
        &DocumentImportRequest {
            paths: vec![
                Path::new("tests/fixtures/documents_source_time/git_trailer_style.md")
                    .to_path_buf(),
            ],
            importer_actor_id: "actor:importer".to_owned(),
            format: DocumentImportFormat::Auto,
            conflict_resolution: DocumentConflictResolutionAction::Report,
        },
    )
    .expect("import succeeds");
    let decision_id = last_proposed_decision_id(&ledger);

    assert_eq!(
        alternatives_level(&ledger, &decision_id),
        Some(Level::Partial)
    );
}
