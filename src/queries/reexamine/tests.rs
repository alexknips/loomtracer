// Parent module gates this file with #[cfg(test)]; repeat the marker so UBS can filter test-only assertions.
#[cfg(test)]
use crate::projector::memory::MemoryGraph;
use crate::queries::test_fixtures::{
    assert_reexamine_scenario, reexamine_scenario, ts, CountingGraph, Scenario, REEXAMINE_NOW,
};
use crate::queries::GROUNDING_FACT_READS;
use crate::Result;

use super::*;

// ── helpers ───────────────────────────────────────────────────────────────────

fn now() -> DateTime<Utc> {
    ts(REEXAMINE_NOW)
}

fn ask(
    scenario: &Scenario,
    graph: &impl GraphView,
    request: &ReexamineRequest,
) -> Result<QueryResponse<ReexamineResults>> {
    get_reexamine_items_at(
        &QueryContext::local(),
        graph,
        scenario.ledger(),
        request,
        now(),
    )
}

fn open_ids(results: &ReexamineResults) -> Vec<&str> {
    results
        .items
        .iter()
        .map(|item| item.dependent.id.as_str())
        .collect()
}

/// One dependent that follows one premise that a newer decision supersedes. Returns the scenario
/// with nothing after the supersession, so a test appends what it wants to read.
fn superseded_premise() -> Result<Scenario> {
    let s = Scenario::new();
    let crew = "agent:claude:crew";
    for (id, title, at) in [
        (
            "d:premise",
            "Cap requests per tenant",
            "2026-09-01T00:00:00Z",
        ),
        ("d:newer", "Cap requests per user", "2026-09-02T00:00:00Z"),
        ("d:dependent", "Shard by tenant", "2026-09-01T00:01:00Z"),
    ] {
        s.decision(id, title, crew, at)?;
    }
    s.relation(
        "FOLLOWS_FROM",
        "d:dependent",
        "d:premise",
        crew,
        None,
        "2026-09-01T00:02:00Z",
    )?;
    Ok(s)
}

// ── the scenario ───────────────────────────────────────────────────────────────

#[test]
fn the_scenario_reads_as_every_backend_must_read_it() -> Result<()> {
    let scenario = reexamine_scenario()?;

    assert_reexamine_scenario(&scenario, &scenario.graph()?)
}

#[test]
fn the_bulk_and_the_anchored_standing_of_a_decision_are_the_same_summary() -> Result<()> {
    let scenario = reexamine_scenario()?;
    let graph = scenario.graph()?;
    let facts = get_grounding_facts(&graph)?;

    let decisions = node_rows(&graph, NodeKind::Decision)?;

    assert!(decisions.len() >= 40);
    for decision_id in decisions.keys() {
        assert_eq!(
            facts.standings.summary_of(decision_id),
            anchored_standing(&graph, decision_id)?,
            "{decision_id}"
        );
    }
    Ok(())
}

// ── refusals ──────────────────────────────────────────────────────────────────

#[test]
fn a_project_nobody_registered_is_refused_not_answered_empty() -> Result<()> {
    let scenario = reexamine_scenario()?;
    let graph = scenario.graph()?;

    let refused = ask(
        &scenario,
        &graph,
        &ReexamineRequest {
            project: Some("nowhere".to_owned()),
            ..ReexamineRequest::default()
        },
    )
    .expect_err("an unregistered project is refused");

    assert!(refused.to_string().contains("not registered"), "{refused}");
    Ok(())
}

#[test]
fn a_cursor_this_query_did_not_return_is_refused() -> Result<()> {
    let scenario = reexamine_scenario()?;
    let graph = scenario.graph()?;

    let refused = ask(
        &scenario,
        &graph,
        &ReexamineRequest {
            cursor: Some("not a cursor".to_owned()),
            ..ReexamineRequest::default()
        },
    )
    .expect_err("a foreign cursor is refused");

    assert!(refused.to_string().contains("cursor"), "{refused}");
    Ok(())
}

// ── paging while the graph moves ─────────────────────────────────────────────────

#[test]
fn items_that_open_or_close_between_pages_never_make_a_reader_skip_or_repeat_one_that_stood(
) -> Result<()> {
    let scenario = reexamine_scenario()?;
    let first_page = ask(
        &scenario,
        &scenario.graph()?,
        &ReexamineRequest {
            limit: 2,
            ..ReexamineRequest::default()
        },
    )?;
    assert!(first_page.truncated);
    assert_eq!(open_ids(&first_page.data), ["d:cyc-b", "d:late-link"]);
    let cursor = first_page.data.next_cursor.clone();

    // Between the pages: an item behind the cursor closes, one ahead of it closes, and a new
    // dependent of the superseded premise appears (it sorts before everything on page one).
    let alice = "human:alice";
    scenario.reaffirmed(
        "d:cyc-b",
        "d:cyc-a",
        "Nights are fixed",
        alice,
        "2026-09-28T00:00:00Z",
    )?;
    scenario.reaffirmed(
        "d:on-assumption",
        "h:assumption",
        "The digest is weekly after all",
        alice,
        "2026-09-28T00:00:01Z",
    )?;
    scenario.decision(
        "d:aaa-new",
        "Cap alerts per tenant",
        "agent:claude:crew",
        "2026-09-28T00:00:02Z",
    )?;
    scenario.relation(
        "FOLLOWS_FROM",
        "d:aaa-new",
        "d:p-sup",
        "agent:claude:crew",
        None,
        "2026-09-28T00:00:03Z",
    )?;

    let rest = {
        let graph = scenario.graph()?;
        let mut collected = Vec::new();
        let mut cursor = cursor;
        loop {
            let page = ask(
                &scenario,
                &graph,
                &ReexamineRequest {
                    limit: 3,
                    cursor: cursor.clone(),
                    ..ReexamineRequest::default()
                },
            )?;
            collected.extend(open_ids(&page.data).into_iter().map(str::to_owned));
            cursor = page.data.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        collected
    };

    // Everything that stood open throughout and sorts after the cursor, once and in order; the
    // closed item is gone, the new one behind the cursor is not offered on this walk.
    assert_eq!(
        rest,
        [
            "d:on-conc",
            "d:on-cont",
            "d:on-cont2",
            "d:on-edge-eq",
            "d:on-edge-over",
            "d:on-rej",
            "d:open",
        ]
    );
    Ok(())
}

// ── what moves a change ───────────────────────────────────────────────────────────

#[test]
fn a_supersession_asserted_again_does_not_move_the_change() -> Result<()> {
    let s = superseded_premise()?;
    s.supersede("d:premise", "d:newer", "human:alex", "2026-09-02T00:00:01Z")?;
    s.reaffirmed(
        "d:dependent",
        "d:premise",
        "Sharding never depended on the cap",
        "human:alice",
        "2026-09-03T00:00:00Z",
    )?;
    // The same supersession recorded again after the reaffirmation: the change is still the
    // first one, so the reaffirmation still closes the item (Postgres and Kuzu keep one edge per
    // pair, the first write, and agree).
    s.supersede("d:premise", "d:newer", "human:alex", "2026-09-04T00:00:00Z")?;
    let graph = s.graph()?;

    let items = ask(
        &s,
        &graph,
        &ReexamineRequest {
            include_closed: true,
            ..ReexamineRequest::default()
        },
    )?
    .data
    .items;

    assert_eq!(items.len(), 1);
    let ItemState::Reaffirmed { reaffirmations } = &items[0].entry.state else {
        panic!(
            "closed by the reaffirmation, got {:?}",
            items[0].entry.state
        );
    };
    assert!(
        items[0].entry.change.at_offset < reaffirmations[0].event_origin,
        "the change is the first supersession"
    );
    assert!(items[0].entry.earlier_reaffirmations.is_empty());
    Ok(())
}

#[test]
fn a_reaffirmation_that_matches_no_changed_premise_is_ignored() -> Result<()> {
    let s = superseded_premise()?;
    let crew = "agent:claude:crew";
    s.decision(
        "d:standing",
        "A premise nobody changed",
        crew,
        "2026-09-01T00:03:00Z",
    )?;
    s.decision(
        "d:stranger",
        "Does not follow anything",
        crew,
        "2026-09-01T00:04:00Z",
    )?;
    // Before the change: history, not a close.
    s.reaffirmed(
        "d:dependent",
        "d:premise",
        "Early word",
        "human:alice",
        "2026-09-01T12:00:00Z",
    )?;
    s.supersede("d:premise", "d:newer", "human:alex", "2026-09-02T00:00:01Z")?;
    // Nothing to re-examine: the premise stands, or the decision never followed it.
    s.reaffirmed(
        "d:dependent",
        "d:standing",
        "Looked anyway",
        "human:alice",
        "2026-09-03T00:00:00Z",
    )?;
    s.reaffirmed(
        "d:stranger",
        "d:premise",
        "Not mine to say",
        "human:alice",
        "2026-09-03T00:00:01Z",
    )?;
    let graph = s.graph()?;

    let items = ask(
        &s,
        &graph,
        &ReexamineRequest {
            include_closed: true,
            ..ReexamineRequest::default()
        },
    )?
    .data
    .items;

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].dependent.id, "d:dependent");
    assert!(
        items[0].entry.state.is_open(),
        "an earlier word does not close a later change"
    );
    assert_eq!(items[0].entry.earlier_reaffirmations.len(), 1);
    Ok(())
}

// ── the rules, without a graph ──────────────────────────────────────────────────────

fn change_at(offset: Option<i64>) -> PremiseChange {
    PremiseChange {
        kind: ChangeKind::Superseded,
        by_ids: vec!["d:newer".to_owned()],
        at_offset: offset,
        at_time: None,
        dated_by: Some("d:newer".to_owned()),
    }
}

fn reaffirmation_at(offset: Option<i64>) -> ReaffirmationRecord {
    ReaffirmationRecord {
        id: format!("r:{offset:?}"),
        decision_id: "d:dependent".to_owned(),
        premise_id: "d:premise".to_owned(),
        reason: "Still holds".to_owned(),
        reaffirmed_by: "human:alice".to_owned(),
        reaffirmed_at: None,
        event_origin: offset,
    }
}

fn standing_dependent() -> StandingSummary {
    StandingSummary::from_edges(&[], &[Some(1)], &[])
}

fn judged(
    dependent: &StandingSummary,
    change: Option<i64>,
    reaffirmations: &[Option<i64>],
) -> Option<Judged> {
    let records: Vec<ReaffirmationRecord> = reaffirmations
        .iter()
        .copied()
        .map(reaffirmation_at)
        .collect();
    let refs: Vec<&ReaffirmationRecord> = records.iter().collect();
    judge(dependent, PremiseKind::Decision, change_at(change), &refs)
}

#[test]
fn a_reaffirmation_closes_an_item_only_from_a_later_offset() {
    let at = |reaffirmed| judged(&standing_dependent(), Some(10), &[reaffirmed]).expect("an item");

    assert!(!at(Some(11)).state.is_open());
    // The same offset is not later; an earlier one is history.
    for not_later in [Some(10), Some(3)] {
        let item = at(not_later);
        assert!(item.state.is_open());
        assert_eq!(item.earlier.len(), 1);
    }
}

#[test]
fn an_offset_nobody_knows_never_closes_anything() {
    // The reaffirmation cannot be placed against the change, or the change against it.
    for (change, reaffirmed) in [(Some(10), None), (None, Some(99)), (None, None)] {
        let item = judged(&standing_dependent(), change, &[reaffirmed]).expect("an item");
        assert!(item.state.is_open(), "{change:?} {reaffirmed:?}");
    }
}

#[test]
fn every_reaffirmation_of_a_pair_is_kept_in_ledger_order() {
    let item = judged(
        &standing_dependent(),
        Some(10),
        &[Some(4), Some(12), Some(15)],
    )
    .expect("an item");

    let ItemState::Reaffirmed { reaffirmations } = &item.state else {
        panic!("reaffirmed, got {:?}", item.state);
    };
    let offsets: Vec<Option<i64>> = reaffirmations.iter().map(|r| r.event_origin).collect();
    assert_eq!(offsets, [Some(12), Some(15)]);
    assert_eq!(item.earlier.len(), 1);
}

#[test]
fn a_dependent_that_went_out_before_the_change_is_not_asked() {
    let superseded_at =
        |offset| StandingSummary::from_edges(&[("d:v2".to_owned(), offset)], &[Some(1)], &[]);
    let rejected_at =
        |offset| StandingSummary::from_edges(&[], &[], &[("human:bob".to_owned(), offset)]);

    // Before the change: nothing to re-examine, not counted.
    assert!(judged(&superseded_at(Some(5)), Some(10), &[]).is_none());
    assert!(judged(&rejected_at(Some(5)), Some(10), &[]).is_none());
    // After it: closed by the dependent's own fate, and the state says which.
    let after = judged(&superseded_at(Some(20)), Some(10), &[]).expect("an item");
    assert_eq!(
        after.state,
        ItemState::DependentSuperseded {
            by_id: "d:v2".to_owned()
        }
    );
    let after = judged(&rejected_at(Some(20)), Some(10), &[]).expect("an item");
    assert_eq!(after.state, ItemState::DependentRejected);
}

#[test]
fn a_reaffirmation_wins_the_label_over_the_dependent_going_out_later() {
    let superseded = StandingSummary::from_edges(&[("d:v2".to_owned(), Some(30))], &[Some(1)], &[]);

    let item = judged(&superseded, Some(10), &[Some(20)]).expect("an item");

    assert!(matches!(item.state, ItemState::Reaffirmed { .. }));
}

#[test]
fn a_contested_dependent_still_stands_and_is_asked() {
    let contested =
        StandingSummary::from_edges(&[], &[Some(1)], &[("human:bob".to_owned(), Some(2))]);

    let item = judged(&contested, Some(10), &[]).expect("an item");

    assert!(item.state.is_open());
}

// ── what a page costs ────────────────────────────────────────────────────────────

/// `scenario` plus `fillers` decisions that follow a premise nobody changed and rest on a
/// hypothesis nobody refuted: more graph, the same items.
fn with_fillers(fillers: usize) -> Result<MemoryGraph> {
    let s = reexamine_scenario()?;
    let crew = "agent:claude:crew";
    s.decision(
        "d:filler-premise",
        "A premise that stands",
        crew,
        "2026-09-01T00:00:00Z",
    )?;
    s.accept("d:filler-premise", "human:alex", "2026-09-01T00:00:01Z")?;
    for index in 0..fillers {
        let id = format!("d:filler-{index}");
        s.decision_with(
            &id,
            "Filler",
            crew,
            "2026-09-01T00:00:02Z",
            false,
            &[],
            &["h:fine"],
            None,
        )?;
        s.relation(
            "FOLLOWS_FROM",
            &id,
            "d:filler-premise",
            crew,
            None,
            "2026-09-01T00:00:03Z",
        )?;
    }
    s.graph()
}

#[test]
fn a_page_reads_the_same_however_large_the_graph_is() -> Result<()> {
    let small = reexamine_scenario()?;
    let small_graph = small.graph()?;
    let large_graph = with_fillers(300)?;

    let reads = |graph: &MemoryGraph, request: &ReexamineRequest| -> Result<usize> {
        let counting = CountingGraph::new(graph);
        ask(&small, &counting, request)?;
        Ok(counting.queries())
    };
    let whole = ReexamineRequest {
        include_closed: true,
        limit: 100,
        ..ReexamineRequest::default()
    };
    let one = ReexamineRequest {
        limit: 1,
        ..ReexamineRequest::default()
    };

    assert_eq!(reads(&small_graph, &whole)?, reads(&large_graph, &whole)?);
    assert_eq!(reads(&small_graph, &one)?, reads(&large_graph, &one)?);
    // The bulk facts and the project labels are fixed; what a page adds is wording its own items.
    assert!(reads(&small_graph, &one)? > GROUNDING_FACT_READS);
    assert!(reads(&small_graph, &one)? < reads(&small_graph, &whole)?);
    Ok(())
}

#[test]
fn a_decisions_own_entries_cost_nothing_until_a_premise_changed_and_never_grow_with_the_graph(
) -> Result<()> {
    let small_graph = reexamine_scenario()?.graph()?;
    let large_graph = with_fillers(300)?;

    let entry_reads =
        |graph: &MemoryGraph, decision_id: &str, changed: &[String]| -> Result<usize> {
            let counting = CountingGraph::new(graph);
            reexamine_entries_for(&counting, decision_id, changed, &[])?;
            Ok(counting.queries())
        };
    let changed = ["d:p-sup".to_owned()];

    assert_eq!(entry_reads(&small_graph, "d:open", &[])?, 0);
    assert_eq!(entry_reads(&large_graph, "d:open", &[])?, 0);
    let small_cost = entry_reads(&small_graph, "d:open", &changed)?;
    assert!(small_cost > 0);
    assert_eq!(small_cost, entry_reads(&large_graph, "d:open", &changed)?);

    // The whole outcome of a decision with a changed premise: the same on both graphs.
    let outcome_reads = |graph: &MemoryGraph| -> Result<usize> {
        let counting = CountingGraph::new(graph);
        crate::queries::get_decision_outcome_at(&counting, "d:open", now())?;
        Ok(counting.queries())
    };
    assert_eq!(outcome_reads(&small_graph)?, outcome_reads(&large_graph)?);
    Ok(())
}

// ── configuration ───────────────────────────────────────────────────────────────────

#[test]
fn the_unfinished_window_defaults_to_two_weeks() {
    assert_eq!(DEFAULT_PROPAGATION_WINDOW_DAYS, 14);
}
