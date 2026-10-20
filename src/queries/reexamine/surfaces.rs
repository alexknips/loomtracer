// Parent module gates this file with #[cfg(test)]; repeat the marker so UBS can filter test-only assertions.
#[cfg(test)]
use std::collections::BTreeMap;

use crate::projector::{memory::MemoryGraph, NodeKind};
use crate::queries::shared::node_rows;
use crate::queries::test_fixtures::{reexamine_scenario, ts, Scenario, REEXAMINE_NOW};
use crate::queries::{
    export_decision_log, get_compact_view_at, get_decision_brief_at, get_decision_timeline,
    get_decisions_changed_since, get_propagation_at, get_recent_activity, get_reexamine_items_at,
    get_relevant_decisions, reaffirmation_arrows, search_decisions, ChangedSinceRequest,
    DecisionLogOutcome, DecisionLogRequest, HistoryChangeKind, ItemState, QueryContext,
    RecentActivityRequest, ReexamineMarker, ReexamineRequest, ReexamineStatus,
    SearchDecisionRequest, TimelineFact, REAFFIRMED_AFTER, REAFFIRMS,
};
use crate::Result;

use super::{propagation_of_decision, ReexamineMarkers, DEFAULT_PROPAGATION_WINDOW_DAYS};

fn now() -> chrono::DateTime<chrono::Utc> {
    ts(REEXAMINE_NOW)
}

fn marker(stale: bool, open: usize, reaffirmed: usize) -> ReexamineMarker {
    ReexamineMarker {
        stale,
        open,
        reaffirmed,
    }
}

// ── the brief ──────────────────────────────────────────────────────────────────

#[test]
fn a_brief_carries_the_entries_and_follows_the_headline_rule() -> Result<()> {
    let scenario = reexamine_scenario()?;
    let graph = scenario.graph()?;
    let brief = |id: &str| -> Result<_> {
        Ok(get_decision_brief_at(&graph, id, now())?
            .data
            .expect("the decision is in the scenario"))
    };

    // Nobody looked again: the premise change stands against it.
    let open = brief("d:open")?;
    assert!(!open.still_holds.held_up);
    assert_eq!(open.still_holds.reexamine.len(), 1);
    assert_eq!(open.still_holds.reexamine[0].premise_id, "d:p-sup");
    assert!(open.still_holds.reexamine[0].state.is_open());

    // Two actors said it still holds: it reads as holding up again, and both stay visible.
    let twice = brief("d:twice")?;
    assert!(twice.still_holds.held_up);
    let ItemState::Reaffirmed { reaffirmations } = &twice.still_holds.reexamine[0].state else {
        panic!("d:twice was reaffirmed");
    };
    assert_eq!(reaffirmations.len(), 2);

    // A contested premise opens an item and never flips the headline.
    let contested = brief("d:on-cont")?;
    assert!(contested.still_holds.held_up);
    assert!(contested.still_holds.reexamine[0].state.is_open());

    // The words say where a change came from when it crossed a project link, and only then:
    // d:twice (Platform) rests on a Billing decision, d:open (Billing) on one of its own.
    assert_eq!(
        twice.still_holds.reexamine[0].crossed_from.as_deref(),
        Some("Billing")
    );
    assert!(twice.still_holds.reexamine[0]
        .describe()
        .contains("premise \"Cap requests per tenant\" (in Billing) was superseded"));
    assert_eq!(open.still_holds.reexamine[0].crossed_from, None);
    assert!(!open.still_holds.reexamine[0].describe().contains("(in "));
    // ... on the item list too, where the dependent is worded from its own project.
    let items = get_reexamine_items_at(
        &QueryContext::local(),
        &graph,
        scenario.ledger(),
        &ReexamineRequest {
            dependent_id: Some("d:late-link".to_owned()),
            ..ReexamineRequest::default()
        },
        now(),
    )?
    .data
    .items;
    assert_eq!(items[0].entry.crossed_from.as_deref(), Some("Billing"));

    // Nothing it rests on changed: nothing to report, and no propagation either.
    let unchanged = brief("d:on-fine")?;
    assert!(unchanged.still_holds.reexamine.is_empty());
    assert!(unchanged.propagation.is_none());
    Ok(())
}

#[test]
fn a_changed_decisions_brief_says_how_far_the_change_travelled() -> Result<()> {
    let scenario = reexamine_scenario()?;
    let graph = scenario.graph()?;

    let premise = get_decision_brief_at(&graph, "d:p-sup", now())?
        .data
        .expect("the premise is in the scenario");
    let propagation = premise.propagation.expect("d:p-sup was superseded");
    assert_eq!(
        Some(propagation.clone()),
        get_propagation_at(&graph, "d:p-sup", None, now())?
    );
    // "N of M": the dependents that stood when it changed, the ones looked at since.
    assert_eq!(propagation.dependents_total, 5);
    assert_eq!(propagation.reexamined, 3);
    assert_eq!(propagation.open, 2);
    assert!(!propagation.describe().contains("d:"), "names no decision");
    assert!(!propagation.describe().contains("human:"), "names no actor");

    // The window edge: exactly 14 days is not unfinished, a second more is.
    for (premise_id, unfinished) in [("d:p-edge-eq", false), ("d:p-edge-over", true)] {
        let brief = get_decision_brief_at(&graph, premise_id, now())?
            .data
            .expect("the premise is in the scenario");
        let propagation = brief.propagation.expect("the premise was superseded");
        assert_eq!(propagation.unfinished, unfinished, "{premise_id}");
        assert_eq!(propagation.describe().ends_with("[UNFINISHED]"), unfinished);
    }
    Ok(())
}

#[test]
fn the_anchored_propagation_of_a_decision_is_the_bulk_one() -> Result<()> {
    let scenario = reexamine_scenario()?;
    let graph = scenario.graph()?;

    for premise_id in [
        "d:p-sup",
        "d:p-conc",
        "d:p-rej",
        "d:p-cont",
        "d:p-cont2",
        "d:p-cont3",
        "d:p-edge-eq",
        "d:p-edge-over",
        // Not a premise anything changed under, and a decision nothing rests on.
        "d:open",
        "d:before-sup-v2",
    ] {
        assert_eq!(
            propagation_of_decision(&graph, premise_id, DEFAULT_PROPAGATION_WINDOW_DAYS, now())?,
            get_propagation_at(&graph, premise_id, None, now())?,
            "{premise_id}"
        );
    }
    Ok(())
}

// ── the marker on a match ──────────────────────────────────────────────────────

/// What `get_reexamine_items` lists per dependent, folded to a marker by hand.
fn expected_markers(
    scenario: &Scenario,
    graph: &MemoryGraph,
) -> Result<BTreeMap<String, ReexamineMarker>> {
    let results = get_reexamine_items_at(
        &QueryContext::local(),
        graph,
        scenario.ledger(),
        &ReexamineRequest {
            include_closed: true,
            limit: 500,
            ..ReexamineRequest::default()
        },
        now(),
    )?
    .data;
    assert!(results.next_cursor.is_none(), "the scenario fits one page");
    let mut markers: BTreeMap<String, ReexamineMarker> = BTreeMap::new();
    for item in &results.items {
        let entry = markers
            .entry(item.dependent.id.clone())
            .or_insert_with(|| marker(false, 0, 0));
        match item.entry.state.status() {
            Some(ReexamineStatus::Open) => {
                entry.open += 1;
                entry.stale |= item.entry.change.kind != super::ChangeKind::Contested;
            }
            Some(ReexamineStatus::Reaffirmed) => entry.reaffirmed += 1,
            None => {}
        }
    }
    markers.retain(|_, entry| entry.open + entry.reaffirmed > 0);
    Ok(markers)
}

#[test]
fn a_marker_counts_what_the_item_list_counts() -> Result<()> {
    let scenario = reexamine_scenario()?;
    let graph = scenario.graph()?;
    let expected = expected_markers(&scenario, &graph)?;
    assert!(expected.len() > 10, "the scenario has many dependents");

    let markers = ReexamineMarkers::load(&graph)?;
    for (decision_id, want) in &expected {
        assert_eq!(markers.of(decision_id), Some(*want), "{decision_id}");
    }
    // A decision no item names has no marker, and nor has one that went out itself.
    assert_eq!(markers.of("d:p-sup"), None);
    assert_eq!(markers.of("d:after-sup"), None);
    assert_eq!(markers.of("d:no-such-decision"), None);

    // Spot checks, so the table above is not the only witness.
    assert_eq!(markers.of("d:open"), Some(marker(true, 1, 0)));
    assert_eq!(markers.of("d:twice"), Some(marker(false, 0, 1)));
    assert_eq!(markers.of("d:on-cont"), Some(marker(false, 1, 0)));
    assert_eq!(markers.of("d:on-cont3"), Some(marker(false, 0, 1)));
    Ok(())
}

#[test]
fn search_results_carry_the_marker_recall_never_had() -> Result<()> {
    let scenario = reexamine_scenario()?;
    let graph = scenario.graph()?;
    let results = search_decisions(
        &graph,
        &SearchDecisionRequest {
            topic_keys: vec!["grounding".to_owned()],
            limit: 500,
            ..SearchDecisionRequest::default()
        },
    )?
    .data;
    let by_id: BTreeMap<&str, Option<ReexamineMarker>> = results
        .items
        .iter()
        .map(|item| (item.decision.id.as_str(), item.graph_context.reexamine))
        .collect();

    assert_eq!(by_id["d:open"], Some(marker(true, 1, 0)));
    assert_eq!(by_id["d:twice"], Some(marker(false, 0, 1)));
    assert_eq!(by_id["d:on-cont"], Some(marker(false, 1, 0)));
    assert_eq!(by_id["d:p-sup"], None);
    assert_eq!(by_id["d:on-fine"], None);

    // The wire shape: absent for a decision no premise changed under, an object otherwise.
    let open = results
        .items
        .iter()
        .find(|item| item.decision.id == "d:open")
        .expect("d:open matches");
    let json = serde_json::to_value(&open.graph_context).expect("serializes");
    assert_eq!(
        json["reexamine"],
        serde_json::json!({"stale": true, "open": 1, "reaffirmed": 0})
    );
    let plain = results
        .items
        .iter()
        .find(|item| item.decision.id == "d:p-sup")
        .expect("d:p-sup matches");
    assert!(serde_json::to_value(&plain.graph_context)
        .expect("serializes")
        .get("reexamine")
        .is_none());
    Ok(())
}

#[test]
fn the_topic_lookup_carries_the_marker_beside_the_decision() -> Result<()> {
    let scenario = reexamine_scenario()?;
    let graph = scenario.graph()?;
    let response = get_relevant_decisions(&graph, "grounding", None)?;
    let by_id: BTreeMap<&str, &crate::queries::RelevantDecision> = response
        .data
        .iter()
        .map(|item| (item.decision.id.as_str(), item))
        .collect();

    assert_eq!(by_id["d:open"].reexamine, Some(marker(true, 1, 0)));
    assert_eq!(by_id["d:twice"].reexamine, Some(marker(false, 0, 1)));
    assert_eq!(by_id["d:p-sup"].reexamine, None);

    // The decision's own fields stay where they were: the marker is one more key beside them.
    let json = serde_json::to_value(by_id["d:open"]).expect("serializes");
    assert_eq!(json["id"], "d:open");
    assert_eq!(json["status"], "accepted");
    assert_eq!(json["reexamine"]["open"], 1);
    Ok(())
}

// ── the compact view ───────────────────────────────────────────────────────────

#[test]
fn the_compact_view_says_whether_a_premise_was_looked_at_again() -> Result<()> {
    let scenario = reexamine_scenario()?;
    let graph = scenario.graph()?;
    let view = |id: &str| -> Result<_> {
        Ok(get_compact_view_at(&graph, id, now())?
            .data
            .expect("the decision is in the scenario"))
    };

    assert_eq!(
        view("d:open")?.premises[0].reexamine,
        Some(ReexamineStatus::Open)
    );
    assert_eq!(
        view("d:twice")?.premises[0].reexamine,
        Some(ReexamineStatus::Reaffirmed)
    );
    // An assumption refuted under a decision: open for one, reaffirmed for the other.
    assert_eq!(
        view("d:on-assumption")?.hypotheses[0].reexamine,
        Some(ReexamineStatus::Open)
    );
    assert_eq!(
        view("d:on-bet")?.hypotheses[0].reexamine,
        Some(ReexamineStatus::Reaffirmed)
    );
    // A premise that stands says nothing.
    assert_eq!(view("d:p-sup-new")?.premises, Vec::new());

    // A rejected decision is its own terminal one, and says how far the change travelled.
    let rejected = view("d:p-rej")?;
    let propagation = rejected.propagation.expect("d:p-rej was rejected");
    assert_eq!(
        (
            propagation.dependents_total,
            propagation.reexamined,
            propagation.open
        ),
        (2, 1, 1)
    );
    assert!(view("d:open")?.propagation.is_none());
    Ok(())
}

// ── history, why and chain ─────────────────────────────────────────────────────

#[test]
fn history_gives_a_reaffirmation_its_own_row() -> Result<()> {
    let scenario = reexamine_scenario()?;
    let graph = scenario.graph()?;
    let reaffirmations = node_rows(&graph, NodeKind::Reaffirmation)?.len();
    assert!(reaffirmations > 1, "the scenario reaffirms more than once");

    let activity = get_recent_activity(
        scenario.ledger(),
        &scenario.graph()?,
        &RecentActivityRequest {
            limit: 500,
            ..RecentActivityRequest::default()
        },
    )?
    .data;
    let rows: Vec<_> = activity
        .items
        .iter()
        .filter(|row| row.change_kind == HistoryChangeKind::Reaffirmed)
        .collect();
    assert_eq!(rows.len(), reaffirmations);
    for row in &rows {
        // The dependent is the decision the row is about; the premise and reason ride on it.
        assert_eq!(row.decision_ids.len(), 1);
        let said = row.reaffirmed_premise.as_ref().expect("a reaffirmed row");
        assert!(!said.premise_id.is_empty() && !said.reason.is_empty());
    }
    // No reaffirmation is hidden among the context changes any more.
    assert!(!activity.items.iter().any(|row| {
        row.change_kind == HistoryChangeKind::ContextChange
            && row.event_type == crate::events::EventType::DecisionReaffirmed
    }));

    let changed = get_decisions_changed_since(
        scenario.ledger(),
        &scenario.graph()?,
        &ChangedSinceRequest {
            since_offset: Some(0),
            limit: 500,
            ..ChangedSinceRequest::default()
        },
    )?
    .data;
    assert_eq!(
        changed
            .items
            .iter()
            .filter(|row| row.change_kind == HistoryChangeKind::Reaffirmed)
            .count(),
        reaffirmations
    );
    Ok(())
}

#[test]
fn why_and_chain_show_who_said_a_decision_still_holds() -> Result<()> {
    let scenario = reexamine_scenario()?;
    let graph = scenario.graph()?;
    let timeline = get_decision_timeline(&graph, scenario.ledger(), "d:twice")?
        .data
        .expect("d:twice is in the scenario");

    let reaffirmed: Vec<_> = timeline
        .entries
        .iter()
        .filter_map(|entry| match &entry.fact {
            TimelineFact::Reaffirmed { premise_id, reason } => Some((
                premise_id.as_str(),
                reason.as_str(),
                entry.actor_id.as_deref(),
            )),
            _ => None,
        })
        .collect();
    assert_eq!(reaffirmed.len(), 2, "both reaffirmations, in ledger order");
    assert!(reaffirmed
        .iter()
        .all(|(premise, _, _)| *premise == "d:p-sup"));
    assert!(reaffirmed
        .iter()
        .all(|(_, reason, actor)| !reason.is_empty() && actor.is_some()));
    // The existing premise row stays beside it: the change and the answer are both on the page.
    assert!(timeline
        .entries
        .iter()
        .any(|entry| matches!(entry.fact, TimelineFact::PremiseSuperseded { .. })));
    assert!(TimelineFact::Reaffirmed {
        premise_id: String::new(),
        reason: String::new(),
    }
    .is_revision());
    Ok(())
}

// ── graph views ────────────────────────────────────────────────────────────────

#[test]
fn a_reaffirmation_is_drawn_with_the_decision_it_is_about() -> Result<()> {
    let scenario = reexamine_scenario()?;
    let graph = scenario.graph()?;
    let nodes = node_rows(&graph, NodeKind::Reaffirmation)?;
    let arrows = reaffirmation_arrows(&graph)?;

    // Two lines per reaffirmation: to the dependent, and to the premise that changed.
    assert_eq!(arrows.len(), nodes.len() * 2);
    for (id, row) in &nodes {
        let decision_id = crate::queries::shared::optional_string(row, "decision_id")
            .expect("a reaffirmation names its decision");
        let premise_id = crate::queries::shared::optional_string(row, "premise_id")
            .expect("a reaffirmation names its premise");
        let own: Vec<_> = arrows.iter().filter(|arrow| &arrow.from_id == id).collect();
        assert_eq!(own.len(), 2, "{id}");
        assert!(own
            .iter()
            .all(|arrow| arrow.from_kind == NodeKind::Reaffirmation));
        let to_decision = own
            .iter()
            .find(|arrow| arrow.relation == REAFFIRMS)
            .expect("a line to the dependent");
        assert_eq!(
            (to_decision.to_kind, to_decision.to_id.as_str()),
            (NodeKind::Decision, decision_id.as_str())
        );
        let to_premise = own
            .iter()
            .find(|arrow| arrow.relation == REAFFIRMED_AFTER)
            .expect("a line to the premise");
        assert_eq!(to_premise.to_id, premise_id);
        // A premise is a decision or a hypothesis; the line says which.
        let want = if premise_id.starts_with("h:") {
            NodeKind::Hypothesis
        } else {
            NodeKind::Decision
        };
        assert_eq!(to_premise.to_kind, want, "{premise_id}");
    }
    Ok(())
}

// ── the decision log ───────────────────────────────────────────────────────────

#[test]
fn the_decision_log_records_what_changed_under_a_decision_and_who_looked_again() -> Result<()> {
    let scenario = reexamine_scenario()?;
    let graph = scenario.graph()?;
    let DecisionLogOutcome::Exported(export) = export_decision_log(
        &graph,
        scenario.ledger(),
        &DecisionLogRequest::default(),
        None,
    )?
    else {
        panic!("an unfiltered export exists");
    };
    let file_of = |title: &str| -> String {
        export
            .files
            .iter()
            .find(|(path, content)| {
                path.contains("/decisions/") && content.contains(&format!("\n# {title}\n"))
            })
            .map(|(_, content)| content.clone())
            .unwrap_or_else(|| panic!("no file for {title}"))
    };

    // A decision a premise changed under: the entry, in the same words every text surface uses.
    let open = file_of("Shard by tenant");
    assert!(open.contains("\nRe-examination:\n- re-examine open: premise \"Cap requests per tenant\" was superseded 2026-09-02;"), "{open}");
    let twice = file_of("Bill by request count");
    assert!(
        twice.contains("\n- re-examined: premise \"Cap requests per tenant\""),
        "{twice}"
    );

    // The premise itself says how far the change travelled, without the mark that depends on
    // today's date: the same ledger exports the same file.
    let premise = file_of("Cap requests per tenant");
    let expected = {
        let mut propagation =
            get_propagation_at(&graph, "d:p-sup", None, now())?.expect("d:p-sup was superseded");
        propagation.unfinished = false;
        propagation.describe()
    };
    assert!(
        premise.contains(&format!("\nPropagation: {expected}\n")),
        "{premise}"
    );
    assert!(!premise.contains("UNFINISHED"), "{premise}");

    // A decision nothing changed under gets neither section.
    let plain = file_of("Cap requests per user");
    assert!(
        !plain.contains("Re-examination:") && !plain.contains("Propagation:"),
        "{plain}"
    );
    Ok(())
}
