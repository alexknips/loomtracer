// Parent module gates this file with #[cfg(test)]; repeat the marker so UBS can filter test-only assertions.
#[cfg(test)]
use chrono::{TimeZone, Utc};
use serde_json::json;
use uuid::Uuid;

use crate::commands::{Commands, DecisionProposalInput, FloorInput, Grounding};
use crate::events::{Event, EventSource, EventType};
use crate::ledger::{AnyLedger, EventLedger, SqliteEventLedger};
use crate::projector::{memory::MemoryGraph, rebuild_graph};
use crate::summarize::{recall_decisions, RecallRequest, RECALL_MAX_LIMIT};
use crate::Result;

use super::*;

// Capture stores every topic key as lowercase kebab (`normalize_topic_key`): `Release Plan`
// becomes `release-plan`. Every read-side topic filter has to meet the stored form, or a
// decision that is there reads as "nothing recorded on this topic" (hivemind-vwoau).

/// The ways a person types the stored key `release-plan`.
const TYPED_FORMS: [&str; 4] = [
    "Release Plan",
    "release_plan",
    "RELEASE-PLAN",
    "  release plan  ",
];

struct Fixture {
    _dir: tempfile::TempDir,
    ledger: AnyLedger,
    graph: MemoryGraph,
    /// Captured under the topic typed as `Release Plan`.
    release_plan: String,
}

fn propose(commands: &Commands<'_, AnyLedger>, title: &str, topic: &str) -> Result<String> {
    let option_id = commands.record_option("human:alex", "Do it", "Do it")?;
    commands.propose_decision(DecisionProposalInput {
        grounding: Grounding::NotAsked,
        expressed_confidence: None,
        actor_id: "human:alex",
        title,
        rationale: "Reasons that do not matter to a topic filter",
        topic_keys: &[topic.to_owned()],
        option_ids: std::slice::from_ref(&option_id),
        option_labels: &["Do it".to_owned()],
        chosen_option_id: Some(option_id.as_str()),
        decided_by: None,
        still_proposed: false,
        hypothesis_ids: &[],
        evidence_ids: &[],
        quote: None,
        question: None,
        delegated_by: None,
        project: None,
        floor: FloorInput::NotAsked,
    })
}

fn blocker_event(decision_id: &str, topic_keys: &[&str]) -> Event {
    Event {
        tenant_id: Default::default(),
        event_id: None,
        event_uuid: Uuid::from_u128(0xb10c),
        correlation_id: Some("topic-filter-test".to_owned()),
        causation_event_id: None,
        event_type: EventType::BlockerReported,
        actor_id: "agent:worker".to_owned(),
        source: EventSource::Api,
        source_ref: Some("topic-filter-test".to_owned()),
        payload: json!({
            "blocker_id": "blocker:release",
            "blocked_actor_id": "agent:worker",
            "decision_id": decision_id,
            "topic_keys": topic_keys,
            "blocked_ref": "task:ship",
            "blocked_ref_type": "task",
            "reason": "Waiting on the date",
            "priority": "p1",
            "last_progress_at": null,
            "required_owner_id": "actor:owner"
        }),
        ts: Some(
            Utc.with_ymd_and_hms(2026, 1, 2, 0, 0, 0)
                .single()
                .expect("valid timestamp"),
        ),
    }
}

fn fixture() -> Result<Fixture> {
    let dir = tempfile::tempdir().map_err(query_error)?;
    let ledger = AnyLedger::Sqlite(SqliteEventLedger::open(dir.path())?);
    let commands = Commands::new(&ledger);
    let release_plan = propose(&commands, "Ship the release on Fridays", "Release Plan")?;
    propose(&commands, "Charge per seat", "Billing")?;
    ledger.append(blocker_event(&release_plan, &["release-plan"]))?;
    let graph = MemoryGraph::default();
    rebuild_graph(&ledger, &graph)?;
    Ok(Fixture {
        _dir: dir,
        ledger,
        graph,
        release_plan,
    })
}

fn search_request(topic: &str) -> SearchDecisionRequest {
    SearchDecisionRequest {
        topic_keys: vec![topic.to_owned()],
        limit: 10,
        ..SearchDecisionRequest::default()
    }
}

fn found(results: &DecisionSearchResults) -> Vec<&str> {
    results
        .items
        .iter()
        .map(|item| item.decision.id.as_str())
        .collect()
}

#[test]
fn every_search_path_finds_the_decision_by_the_topic_as_typed() -> Result<()> {
    let f = fixture()?;
    let context = QueryContext::local();
    let expected = [f.release_plan.as_str()];

    for typed in TYPED_FORMS {
        let request = search_request(typed);
        assert_eq!(
            found(&search_decisions(&f.graph, &request)?.data),
            expected,
            "graph search for {typed:?}"
        );
        assert_eq!(
            found(&search_decisions_with_ledger(&context, &f.ledger, &f.graph, &request)?.data),
            expected,
            "portable search for {typed:?}"
        );
        assert_eq!(
            found(&search_decisions_any(&context, &f.ledger, &f.graph, &request)?.data),
            expected,
            "full-text search for {typed:?}"
        );
        assert_eq!(
            found(&search_decisions_fluent(&context, &f.ledger, &f.graph, &request, false)?.data),
            expected,
            "recall search for {typed:?}"
        );
    }
    Ok(())
}

#[test]
fn recall_finds_the_decision_by_the_topic_as_typed() -> Result<()> {
    let f = fixture()?;
    for typed in TYPED_FORMS {
        let response = recall_decisions(
            &QueryContext::local(),
            &f.ledger,
            &f.graph,
            &RecallRequest {
                q: None,
                topic_keys: vec![typed.to_owned()],
                statuses: Vec::new(),
                actor_ids: Vec::new(),
                sources: Vec::new(),
                since: None,
                until: None,
                limit: RECALL_MAX_LIMIT,
                cursor: None,
                project: None,
            },
        )?;
        let ids: Vec<&str> = response
            .data
            .ranked
            .items
            .iter()
            .map(|item| item.decision.id.as_str())
            .collect();
        assert_eq!(ids, [f.release_plan.as_str()], "recall for {typed:?}");
    }
    Ok(())
}

#[test]
fn a_topic_that_names_nothing_stored_still_finds_nothing() -> Result<()> {
    let f = fixture()?;
    let context = QueryContext::local();
    let request = search_request("Release Planning");
    assert!(search_decisions(&f.graph, &request)?.data.items.is_empty());
    assert!(
        search_decisions_with_ledger(&context, &f.ledger, &f.graph, &request)?
            .data
            .items
            .is_empty()
    );
    assert!(get_relevant_decisions(&f.graph, "Release Planning", None)?
        .data
        .is_empty());
    Ok(())
}

#[test]
fn get_relevant_decisions_finds_the_decision_by_the_topic_as_typed() -> Result<()> {
    let f = fixture()?;
    for typed in TYPED_FORMS {
        let relevant = get_relevant_decisions(&f.graph, typed, None)?;
        let ids: Vec<&str> = relevant
            .data
            .iter()
            .map(|d| d.decision.id.as_str())
            .collect();
        assert_eq!(ids, [f.release_plan.as_str()], "relevant for {typed:?}");
    }
    Ok(())
}

#[test]
fn get_relevant_decisions_refuses_a_topic_with_no_letter_or_digit() {
    let f = fixture().expect("fixture");
    for blank in ["", "   ", "!!!", "--", "---"] {
        // The caller's error, not a failure of the query: REST answers 400 and the CLI exits 2.
        let error = get_relevant_decisions(&f.graph, blank, None)
            .expect_err(&format!("{blank:?} names no topic"));
        assert!(
            matches!(
                error,
                crate::LoomtracerError::Command(crate::error::CommandError::Validation(_))
            ),
            "{blank:?}: {error:?}"
        );
        assert!(
            error
                .to_string()
                .contains("topic must contain at least one letter or digit"),
            "{blank:?}: {error}"
        );
    }
}

#[test]
fn the_resolver_topic_hint_narrows_by_the_topic_as_typed() -> Result<()> {
    let f = fixture()?;
    for typed in TYPED_FORMS {
        let outcome = resolve_decision_for_reading(&f.graph, "Ship the release", Some(typed))?;
        match outcome.data {
            ResolveOutcome::Resolved { candidate } => {
                assert_eq!(
                    candidate.decision_id, f.release_plan,
                    "resolve for {typed:?}"
                );
            }
            other => panic!("topic hint {typed:?} should resolve, got {other:?}"),
        }
    }
    // The hint still narrows: the other topic holds no decision that says "release".
    let outcome = resolve_decision_for_reading(&f.graph, "Ship the release", Some("Billing"))?;
    assert!(
        !matches!(outcome.data, ResolveOutcome::Resolved { .. }),
        "a hint at another topic does not resolve to the release decision"
    );
    Ok(())
}

#[test]
fn the_decision_log_export_filters_by_the_topic_as_typed() -> Result<()> {
    let f = fixture()?;
    for typed in TYPED_FORMS {
        let outcome = export_decision_log(
            &f.graph,
            &f.ledger,
            &DecisionLogRequest {
                topics: vec![typed.to_owned()],
                ..DecisionLogRequest::default()
            },
            None,
        )?;
        let DecisionLogOutcome::Exported(export) = outcome else {
            panic!("a topic filter exports, never a project miss");
        };
        let exported: Vec<&String> = export
            .files
            .keys()
            .filter(|path| path.contains("/decisions/"))
            .collect();
        assert_eq!(
            exported.len(),
            1,
            "one decision exported for {typed:?}: {exported:?}"
        );
        assert!(
            export.files["INDEX.md"].contains("topic=release-plan"),
            "the Filters line names the key matched for {typed:?}"
        );
    }
    Ok(())
}

#[test]
fn the_history_reads_filter_by_the_topic_as_typed() -> Result<()> {
    let f = fixture()?;
    let since = Utc
        .with_ymd_and_hms(2000, 1, 1, 0, 0, 0)
        .single()
        .expect("valid timestamp");
    for typed in TYPED_FORMS {
        let recent = get_recent_decisions(
            &f.ledger,
            &f.graph,
            &RecentDecisionsRequest {
                since_timestamp: since,
                until_timestamp: None,
                filters: RecentDecisionFilterRequest {
                    topic_keys: vec![typed.to_owned()],
                    ..RecentDecisionFilterRequest::default()
                },
                limit: 10,
                cursor: None,
            },
        )?;
        assert_eq!(recent.data.total_matches, 1, "recent for {typed:?}");

        let added = get_decisions_added_since(
            &f.ledger,
            &f.graph,
            &DecisionsAddedSinceRequest {
                since_offset: Some(0),
                filters: DecisionsAddedSinceFilterRequest {
                    topic_keys: vec![typed.to_owned()],
                    ..DecisionsAddedSinceFilterRequest::default()
                },
                limit: 10,
                ..DecisionsAddedSinceRequest::default()
            },
        )?;
        assert_eq!(added.data.total_added, 1, "added since for {typed:?}");

        let changed = get_decisions_changed_since(
            &f.ledger,
            &f.graph,
            &ChangedSinceRequest {
                since_offset: Some(0),
                filters: HistoryFilterRequest {
                    topic_keys: vec![typed.to_owned()],
                    ..HistoryFilterRequest::default()
                },
                limit: 10,
                ..ChangedSinceRequest::default()
            },
        )?;
        assert_eq!(
            changed.data.filters.topic_keys,
            ["release-plan"],
            "changed since for {typed:?}"
        );
        assert!(
            changed.data.total_matches >= 1,
            "changed since for {typed:?}"
        );
    }
    Ok(())
}

#[test]
fn active_blockers_filter_by_the_topic_as_typed() -> Result<()> {
    let f = fixture()?;
    for typed in TYPED_FORMS {
        let response = get_active_decision_blockers(
            &f.graph,
            &ActiveDecisionBlockersRequest {
                filters: DecisionBlockerFilters {
                    topic_keys: vec![typed.to_owned()],
                    ..DecisionBlockerFilters::default()
                },
                limit: 10,
                cursor: None,
            },
        )?;
        assert_eq!(response.data.total_matches, 1, "blockers for {typed:?}");
    }
    Ok(())
}

// Events appended without the commands layer (classifier drafts, blocker reports, imports,
// fixtures) can carry topic keys exactly as typed. Nodes keep them as recorded (hk5z: a read
// surface must not hide or rewrite what was recorded); the filters normalize the stored key
// and the typed one at comparison time, so either spelling finds it.

/// The keys as the legacy events carry them: dotted, mixed case, and one that names no key.
const LEGACY_KEYS: [&str; 3] = ["org.incident", "Release.Plan", "!!!"];
const LEGACY_ID: &str = "decision:legacy-release";

/// The ways to type the legacy keys: the recorded spelling, and the capture form of each.
const LEGACY_TYPED: [&str; 6] = [
    "org.incident",
    "org-incident",
    "ORG INCIDENT",
    "Release.Plan",
    "release-plan",
    "release_plan",
];

fn legacy_proposed_event(option_id: &str) -> Event {
    Event {
        tenant_id: Default::default(),
        event_id: None,
        event_uuid: Uuid::from_u128(0xdec1),
        correlation_id: Some("topic-filter-test".to_owned()),
        causation_event_id: None,
        event_type: EventType::DecisionProposed,
        actor_id: "human:alex".to_owned(),
        source: EventSource::Api,
        source_ref: Some("topic-filter-test".to_owned()),
        payload: json!({
            "decision_id": LEGACY_ID,
            "title": "Ship the release on Fridays",
            "rationale": "Reasons that do not matter to a topic filter",
            "topic_keys": LEGACY_KEYS,
            "option_ids": [option_id],
            "option_labels": ["Do it"],
            "chosen_option_id": option_id,
            "hypothesis_ids": [],
            "evidence_ids": []
        }),
        ts: Some(
            Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
                .single()
                .expect("valid timestamp"),
        ),
    }
}

fn legacy_fixture() -> Result<Fixture> {
    let dir = tempfile::tempdir().map_err(query_error)?;
    let ledger = AnyLedger::Sqlite(SqliteEventLedger::open(dir.path())?);
    let commands = Commands::new(&ledger);
    let option_id = commands.record_option("human:alex", "Do it", "Do it")?;
    ledger.append(legacy_proposed_event(&option_id))?;
    propose(&commands, "Charge per seat", "Billing")?;
    ledger.append(blocker_event(LEGACY_ID, &LEGACY_KEYS))?;
    let graph = MemoryGraph::default();
    rebuild_graph(&ledger, &graph)?;
    Ok(Fixture {
        _dir: dir,
        ledger,
        graph,
        release_plan: LEGACY_ID.to_owned(),
    })
}

#[test]
fn nodes_keep_the_topic_keys_as_recorded() -> Result<()> {
    let f = legacy_fixture()?;
    let decision = get_decision(&f.graph, &f.release_plan)?
        .data
        .expect("the legacy decision projects");
    assert_eq!(
        decision.topic_keys, LEGACY_KEYS,
        "the graph does not rewrite a key"
    );
    Ok(())
}

#[test]
fn every_topic_filter_finds_a_key_recorded_un_normalized_and_returns_it_as_recorded() -> Result<()>
{
    let f = legacy_fixture()?;
    let context = QueryContext::local();
    let expected = [f.release_plan.as_str()];
    let since = Utc
        .with_ymd_and_hms(2000, 1, 1, 0, 0, 0)
        .single()
        .expect("valid timestamp");

    for typed in LEGACY_TYPED {
        let request = search_request(typed);
        let searched = search_decisions(&f.graph, &request)?;
        assert_eq!(
            found(&searched.data),
            expected,
            "graph search for {typed:?}"
        );
        assert_eq!(
            searched.data.items[0].decision.topic_keys, LEGACY_KEYS,
            "search returns the recorded keys for {typed:?}"
        );
        assert_eq!(
            found(&search_decisions_with_ledger(&context, &f.ledger, &f.graph, &request)?.data),
            expected,
            "portable search for {typed:?}"
        );
        assert_eq!(
            found(&search_decisions_any(&context, &f.ledger, &f.graph, &request)?.data),
            expected,
            "full-text search for {typed:?}"
        );
        assert_eq!(
            found(&search_decisions_fluent(&context, &f.ledger, &f.graph, &request, false)?.data),
            expected,
            "recall search for {typed:?}"
        );

        let relevant = get_relevant_decisions(&f.graph, typed, None)?;
        let ids: Vec<&str> = relevant
            .data
            .iter()
            .map(|d| d.decision.id.as_str())
            .collect();
        assert_eq!(ids, expected, "relevant for {typed:?}");
        assert_eq!(
            relevant.data[0].decision.topic_keys, LEGACY_KEYS,
            "relevant returns the recorded keys for {typed:?}"
        );

        match resolve_decision_for_reading(&f.graph, "Ship the release", Some(typed))?.data {
            ResolveOutcome::Resolved { candidate } => {
                assert_eq!(candidate.decision_id, f.release_plan, "resolve {typed:?}");
            }
            other => panic!("topic hint {typed:?} should resolve, got {other:?}"),
        }

        let blockers = get_active_decision_blockers(
            &f.graph,
            &ActiveDecisionBlockersRequest {
                filters: DecisionBlockerFilters {
                    topic_keys: vec![typed.to_owned()],
                    ..DecisionBlockerFilters::default()
                },
                limit: 10,
                cursor: None,
            },
        )?;
        assert_eq!(blockers.data.total_matches, 1, "blockers for {typed:?}");
        assert_eq!(
            blockers.data.items[0].topic_keys, LEGACY_KEYS,
            "blockers return the recorded keys for {typed:?}"
        );

        let recent = get_recent_decisions(
            &f.ledger,
            &f.graph,
            &RecentDecisionsRequest {
                since_timestamp: since,
                until_timestamp: None,
                filters: RecentDecisionFilterRequest {
                    topic_keys: vec![typed.to_owned()],
                    ..RecentDecisionFilterRequest::default()
                },
                limit: 10,
                cursor: None,
            },
        )?;
        assert_eq!(recent.data.total_matches, 1, "recent for {typed:?}");
        // The history index lists a decision's keys sorted, as it always has; the spelling is
        // the recorded one.
        let mut recorded = LEGACY_KEYS.to_vec();
        recorded.sort_unstable();
        assert_eq!(
            recent.data.items[0].topic_keys, recorded,
            "recent returns the recorded keys for {typed:?}"
        );

        let added = get_decisions_added_since(
            &f.ledger,
            &f.graph,
            &DecisionsAddedSinceRequest {
                since_offset: Some(0),
                filters: DecisionsAddedSinceFilterRequest {
                    topic_keys: vec![typed.to_owned()],
                    ..DecisionsAddedSinceFilterRequest::default()
                },
                limit: 10,
                ..DecisionsAddedSinceRequest::default()
            },
        )?;
        assert_eq!(added.data.total_added, 1, "added since for {typed:?}");

        let outcome = export_decision_log(
            &f.graph,
            &f.ledger,
            &DecisionLogRequest {
                topics: vec![typed.to_owned()],
                ..DecisionLogRequest::default()
            },
            None,
        )?;
        let DecisionLogOutcome::Exported(export) = outcome else {
            panic!("a topic filter exports, never a project miss");
        };
        let exported: Vec<&String> = export
            .files
            .keys()
            .filter(|path| path.contains("/decisions/"))
            .collect();
        assert_eq!(exported.len(), 1, "one decision exported for {typed:?}");
        assert!(
            exported
                .iter()
                .all(|path| export.files[*path].contains("org.incident")),
            "the export shows the recorded spelling for {typed:?}"
        );
    }
    Ok(())
}

const RAW_MCP_ID: &str = "decision:raw-mcp";

/// The legacy fixture plus a second decision recorded under the raw key `MCP`, the way a
/// classifier draft or an import records one.
fn raw_key_fixture() -> Result<Fixture> {
    let dir = tempfile::tempdir().map_err(query_error)?;
    let ledger = AnyLedger::Sqlite(SqliteEventLedger::open(dir.path())?);
    let commands = Commands::new(&ledger);
    let option_id = commands.record_option("human:alex", "Do it", "Do it")?;
    ledger.append(legacy_proposed_event(&option_id))?;
    let mut mcp = legacy_proposed_event(&option_id);
    mcp.event_uuid = Uuid::from_u128(0xdec2);
    mcp.payload["decision_id"] = json!(RAW_MCP_ID);
    mcp.payload["title"] = json!("Serve the MCP tools over HTTP");
    mcp.payload["topic_keys"] = json!(["MCP"]);
    ledger.append(mcp)?;
    propose(&commands, "Charge per seat", "Billing")?;
    let graph = MemoryGraph::default();
    rebuild_graph(&ledger, &graph)?;
    Ok(Fixture {
        _dir: dir,
        ledger,
        graph,
        release_plan: LEGACY_ID.to_owned(),
    })
}

fn scan_for(f: &Fixture, foreign: &[&str]) -> Result<Vec<MisfiledDecisionCandidate>> {
    Ok(scan_misfiled_decisions(
        &f.graph,
        &MisfiledScanRequest {
            foreign_topic_keys: foreign.iter().map(|key| (*key).to_owned()).collect(),
            project: None,
            move_to: None,
            limit: 100,
            cursor: None,
        },
    )?
    .data)
}

#[test]
fn the_misfiled_scan_reports_a_decision_recorded_under_a_raw_key_with_the_key_as_recorded(
) -> Result<()> {
    let f = raw_key_fixture()?;

    // The foreign key is the capture form; the stored key is `MCP` / `org.incident`.
    let mcp = scan_for(&f, &["mcp"])?;
    assert_eq!(mcp.len(), 1, "MCP is reported by the foreign key mcp");
    assert_eq!(mcp[0].decision_id, RAW_MCP_ID);
    assert_eq!(mcp[0].matched_topic_keys, ["MCP"], "the recorded spelling");

    let incident = scan_for(&f, &["org-incident"])?;
    assert_eq!(
        incident.len(),
        1,
        "org.incident is reported by org-incident"
    );
    assert_eq!(incident[0].decision_id, LEGACY_ID);
    assert_eq!(
        incident[0].matched_topic_keys,
        ["org.incident"],
        "the recorded spelling, not the key the match was made on"
    );

    // Only the keys that matched are named; `Release.Plan` and `!!!` were not asked about.
    let both = scan_for(&f, &["org-incident", "release-plan", "mcp"])?;
    let by_id: BTreeMap<&str, &[String]> = both
        .iter()
        .map(|c| (c.decision_id.as_str(), c.matched_topic_keys.as_slice()))
        .collect();
    assert_eq!(by_id.len(), 2, "{both:?}");
    assert_eq!(by_id[LEGACY_ID], ["org.incident", "Release.Plan"]);
    assert_eq!(by_id[RAW_MCP_ID], ["MCP"]);
    Ok(())
}

#[test]
fn the_misfiled_scan_finds_a_raw_key_however_the_foreign_key_is_typed() -> Result<()> {
    let f = raw_key_fixture()?;
    for typed in LEGACY_TYPED {
        let scan = scan_for(&f, &[typed])?;
        let ids: Vec<&str> = scan.iter().map(|c| c.decision_id.as_str()).collect();
        assert_eq!(ids, [LEGACY_ID], "scan for {typed:?}");
    }
    for typed in ["MCP", "mcp", " Mcp "] {
        let scan = scan_for(&f, &[typed])?;
        let ids: Vec<&str> = scan.iter().map(|c| c.decision_id.as_str()).collect();
        assert_eq!(ids, [RAW_MCP_ID], "scan for {typed:?}");
    }
    Ok(())
}
