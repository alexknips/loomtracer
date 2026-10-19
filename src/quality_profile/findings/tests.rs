// Parent module gates this file with #[cfg(test)]; repeat the marker so UBS can filter test-only assertions.
#[cfg(test)]
use std::collections::BTreeSet;

use chrono::{DateTime, Duration, Utc};

use crate::projector::memory::MemoryGraph;
use crate::projector::GraphView;
use crate::queries::test_fixtures::{
    attention_scenario, reexamine_scenario, revisions_scenario, ts, CountingGraph, Scenario,
    ATTENTION_NOW, REEXAMINE_NOW,
};
use crate::queries::{get_decision_brief_at, get_propagation_at, GROUNDING_FACT_READS};
use crate::Result;

use super::*;

// ── helpers ───────────────────────────────────────────────────────────────────

fn now() -> DateTime<Utc> {
    ts(ATTENTION_NOW)
}

/// What a finding says, without its wording: decision, kind, nodes and basis time.
type Summary = (String, FindingKind, Vec<String>, Option<DateTime<Utc>>);

fn summary(finding: &AttentionFinding) -> Summary {
    (
        finding.decision_id.clone(),
        finding.kind,
        finding.node_ids.clone(),
        finding.basis_at,
    )
}

fn summaries(findings: &[AttentionFinding]) -> Vec<Summary> {
    findings.iter().map(summary).collect()
}

/// The findings of `attention_scenario` at `ATTENTION_NOW` with the default window, in the order
/// they come in: by decision id, then kind, then subject.
fn expected_scenario_findings() -> Vec<Summary> {
    use FindingKind::{
        AssumptionRefuted, BetFailed, BetPastCheckDate, EvidenceNotRechecked, PremiseRejected,
        PremiseSuperseded, PropagationUnfinished,
    };
    let table: [(&str, FindingKind, &[&str], Option<&str>); 15] = [
        (
            "d:assumption-refuted",
            AssumptionRefuted,
            &[
                "d:assumption-refuted",
                "e:refute-a2",
                "h:assumption-refuted",
            ],
            Some("2026-06-01T00:00:01Z"),
        ),
        (
            "d:bet-failed",
            BetFailed,
            &["d:bet-failed", "e:refute-bet", "h:bet-failed"],
            Some("2026-08-15T00:00:01Z"),
        ),
        (
            "d:bet-overdue",
            BetPastCheckDate,
            &["d:bet-overdue", "h:bet-overdue"],
            Some("2026-09-01T00:00:00Z"),
        ),
        (
            "d:bet-overdue-direct",
            BetPastCheckDate,
            &["d:bet-overdue-direct", "h:bet-overdue"],
            Some("2026-09-01T00:00:00Z"),
        ),
        (
            "d:cycle-a",
            PremiseSuperseded,
            &["d:cycle-a", "d:p-new", "d:p-old"],
            Some("2026-08-01T00:00:01Z"),
        ),
        (
            "d:ev-just-over",
            EvidenceNotRechecked,
            &["d:ev-just-over", "e:just-over"],
            Some("2026-06-27T23:59:59Z"),
        ),
        (
            "d:ev-stale",
            EvidenceNotRechecked,
            &["d:ev-stale", "e:old"],
            Some("2026-03-01T00:00:00Z"),
        ),
        (
            "d:ev-stale-two",
            EvidenceNotRechecked,
            &["d:ev-stale-two", "e:old"],
            Some("2026-03-01T00:00:00Z"),
        ),
        (
            "d:f-both",
            PremiseSuperseded,
            &["d:f-both", "d:p-new", "d:p-old"],
            Some("2026-08-01T00:00:01Z"),
        ),
        (
            "d:f-both",
            PremiseRejected,
            &["d:f-both", "d:p-rejected"],
            None,
        ),
        (
            "d:f-concurrent",
            PremiseSuperseded,
            &["d:f-concurrent", "d:p-double", "d:p-double-b"],
            Some("2026-08-10T00:00:01Z"),
        ),
        (
            "d:f-rejected",
            PremiseRejected,
            &["d:f-rejected", "d:p-rejected"],
            None,
        ),
        (
            "d:f-superseded",
            PremiseSuperseded,
            &["d:f-superseded", "d:p-new", "d:p-old"],
            Some("2026-08-01T00:00:01Z"),
        ),
        // Dated by the supersession recorded first (`d:p-double-a`), the rule `Propagation` ages
        // by, where `d:f-concurrent` above is dated by the one stamped earlier (`d:p-double-b`).
        (
            "d:p-double",
            PropagationUnfinished,
            &["d:p-double", "d:p-double-a"],
            Some("2026-08-10T00:00:02Z"),
        ),
        (
            "d:p-old",
            PropagationUnfinished,
            &["d:p-new", "d:p-old"],
            Some("2026-08-01T00:00:01Z"),
        ),
    ];
    table
        .iter()
        .map(|(decision_id, kind, node_ids, basis_at)| {
            (
                (*decision_id).to_owned(),
                *kind,
                node_ids.iter().map(|id| (*id).to_owned()).collect(),
                basis_at.map(ts),
            )
        })
        .collect()
}

fn request(kinds: &[FindingKind], limit: usize) -> AttentionRequest {
    AttentionRequest {
        kinds: kinds.to_vec(),
        limit,
        ..AttentionRequest::default()
    }
}

/// Every finding, in one page.
fn all_findings(graph: &impl GraphView) -> Result<Vec<AttentionFinding>> {
    Ok(attention_findings_at(
        graph,
        &request(&[], MAX_QUERY_RESULTS),
        &AttentionConfig::default(),
        now(),
    )?
    .findings)
}

fn decisions_flagged(findings: &[AttentionFinding]) -> BTreeSet<&str> {
    findings
        .iter()
        .map(|finding| finding.decision_id.as_str())
        .collect()
}

/// Every page of `request` walked through its cursors, as (page sizes, all findings).
fn walk(
    graph: &impl GraphView,
    request: &AttentionRequest,
    config: &AttentionConfig,
    now: DateTime<Utc>,
) -> Result<(Vec<usize>, Vec<AttentionFinding>)> {
    let mut request = request.clone();
    let mut sizes = Vec::new();
    let mut findings = Vec::new();
    loop {
        let page = attention_findings_at(graph, &request, config, now)?;
        assert_eq!(page.next_cursor.is_some(), page.truncated);
        sizes.push(page.findings.len());
        findings.extend(page.findings);
        match page.next_cursor {
            Some(cursor) => request.cursor = Some(cursor),
            None => return Ok((sizes, findings)),
        }
        assert!(sizes.len() < 10_000, "the walk does not end");
    }
}

/// What every backend must read from `attention_scenario`: the same findings, in the same order,
/// whole or paged, and the same cost. Shared by the memory, Postgres and Kuzu tests.
pub(crate) fn assert_attention_scenario(graph: &impl GraphView) -> Result<()> {
    let expected = expected_scenario_findings();
    assert_eq!(summaries(&all_findings(graph)?), expected);

    let config = AttentionConfig::default();
    let (sizes, paged) = walk(graph, &request(&[], 4), &config, now())?;
    assert_eq!(sizes, [4, 4, 4, 3]);
    assert_eq!(summaries(&paged), expected);

    for kind in FindingKind::ALL {
        let of_kind = attention_findings_at(graph, &request(&[kind], 0), &config, now())?.findings;
        let want: Vec<&Summary> = expected.iter().filter(|s| s.1 == kind).collect();
        assert_eq!(
            summaries(&of_kind).iter().collect::<Vec<_>>(),
            want,
            "{kind:?}"
        );
    }
    Ok(())
}

// ── the lists ─────────────────────────────────────────────────────────────────

#[test]
fn every_list_holds_what_it_should_on_the_scenario() -> Result<()> {
    let graph = attention_scenario()?.graph()?;

    assert_attention_scenario(&graph)
}

#[test]
fn each_kind_finds_exactly_its_own_decisions() -> Result<()> {
    let graph = attention_scenario()?.graph()?;
    let of = |kind: FindingKind| -> Result<Vec<String>> {
        Ok(attention_findings_at(
            &graph,
            &request(&[kind], 0),
            &AttentionConfig::default(),
            now(),
        )?
        .findings
        .into_iter()
        .map(|finding| finding.decision_id)
        .collect())
    };

    assert_eq!(
        of(FindingKind::BetPastCheckDate)?,
        ["d:bet-overdue", "d:bet-overdue-direct"]
    );
    assert_eq!(
        of(FindingKind::PremiseSuperseded)?,
        ["d:cycle-a", "d:f-both", "d:f-concurrent", "d:f-superseded"]
    );
    assert_eq!(
        of(FindingKind::PremiseRejected)?,
        ["d:f-both", "d:f-rejected"]
    );
    assert_eq!(
        of(FindingKind::AssumptionRefuted)?,
        ["d:assumption-refuted"]
    );
    assert_eq!(of(FindingKind::BetFailed)?, ["d:bet-failed"]);
    assert_eq!(
        of(FindingKind::EvidenceNotRechecked)?,
        ["d:ev-just-over", "d:ev-stale", "d:ev-stale-two"]
    );
    // The changed premises, not the decisions resting on them: the rejected and the contested
    // ones never appear, and nor does a premise whose dependents are all closed or gone.
    assert_eq!(
        of(FindingKind::PropagationUnfinished)?,
        ["d:p-double", "d:p-old"]
    );
    Ok(())
}

#[test]
fn a_bet_is_past_its_check_date_only_while_nothing_has_been_said_either_way() -> Result<()> {
    let graph = attention_scenario()?.graph()?;
    let findings = all_findings(&graph)?;
    let bet_findings: BTreeSet<&str> = findings
        .iter()
        .filter(|finding| finding.kind == FindingKind::BetPastCheckDate)
        .map(|finding| finding.decision_id.as_str())
        .collect();

    // Past its date and open: flagged. Not yet due, undated and supported: not. Refuted: that is
    // a failed bet, not an unchecked one.
    assert_eq!(
        bet_findings,
        BTreeSet::from(["d:bet-overdue", "d:bet-overdue-direct"])
    );
    let flagged = decisions_flagged(&findings);
    for decision_id in [
        "d:bet-future",
        "d:bet-undated",
        "d:bet-held",
        "d:assumption-open",
    ] {
        assert!(!flagged.contains(decision_id), "{decision_id}");
    }
    Ok(())
}

#[test]
fn the_check_date_is_measured_against_the_clock_it_is_asked_at() -> Result<()> {
    let graph = attention_scenario()?.graph()?;
    let bets_at = |at: DateTime<Utc>| -> Result<usize> {
        Ok(attention_findings_at(
            &graph,
            &request(&[FindingKind::BetPastCheckDate], 0),
            &AttentionConfig::default(),
            at,
        )?
        .findings
        .len())
    };

    assert_eq!(bets_at(ts("2026-08-31T23:59:59Z"))?, 0);
    // Not yet past: the date itself is the last moment it may still be checked.
    assert_eq!(bets_at(ts("2026-09-01T00:00:00Z"))?, 0);
    assert_eq!(bets_at(ts("2026-09-01T00:00:01Z"))?, 2);
    // One more bet falls due in December; the undated one never does.
    assert_eq!(bets_at(ts("2026-12-02T00:00:00Z"))?, 3);
    Ok(())
}

#[test]
fn a_decision_that_no_longer_stands_is_never_flagged() -> Result<()> {
    let graph = attention_scenario()?.graph()?;

    let findings = all_findings(&graph)?;
    let flagged = decisions_flagged(&findings);

    // Each of these would be flagged if it stood: an overdue bet, a superseded premise or stale
    // evidence. Superseded and rejected decisions are done with.
    for decision_id in [
        "d:bet-dead",
        "d:bet-refused",
        "d:f-dead",
        "d:f-refused",
        "d:ev-rejected",
    ] {
        assert!(!flagged.contains(decision_id), "{decision_id}");
    }
    Ok(())
}

#[test]
fn a_contested_premise_is_a_disagreement_not_a_change() -> Result<()> {
    let graph = attention_scenario()?.graph()?;

    let findings = all_findings(&graph)?;
    let flagged = decisions_flagged(&findings);

    assert!(!flagged.contains("d:f-contested"));
    assert!(!flagged.contains("d:f-standing"));
    Ok(())
}

#[test]
fn the_first_superseder_and_the_first_refutation_are_the_basis_not_the_lowest_id() -> Result<()> {
    let graph = attention_scenario()?.graph()?;
    let findings = all_findings(&graph)?;
    let of = |decision_id: &str, kind: FindingKind| {
        findings
            .iter()
            .find(|finding| finding.decision_id == decision_id && finding.kind == kind)
            .expect("the finding exists")
    };

    // `d:p-double-a` sorts first and was recorded first, but its supersession is stamped a second
    // after `d:p-double-b`'s.
    let superseded = of("d:f-concurrent", FindingKind::PremiseSuperseded);
    assert!(superseded.node_ids.contains(&"d:p-double-b".to_owned()));
    assert!(!superseded.node_ids.contains(&"d:p-double-a".to_owned()));
    assert_eq!(superseded.basis_at, Some(ts("2026-08-10T00:00:01Z")));
    // `e:refute-a1` sorts first but its link to the assumption was recorded a month after
    // `e:refute-a2`'s.
    let refuted = of("d:assumption-refuted", FindingKind::AssumptionRefuted);
    assert!(refuted.node_ids.contains(&"e:refute-a2".to_owned()));
    assert!(!refuted.node_ids.contains(&"e:refute-a1".to_owned()));
    assert_eq!(refuted.basis_at, Some(ts("2026-06-01T00:00:01Z")));
    Ok(())
}

#[test]
fn a_premise_that_was_rejected_has_no_recorded_time_and_says_so() -> Result<()> {
    let graph = attention_scenario()?.graph()?;

    let page = attention_findings_at(
        &graph,
        &request(&[FindingKind::PremiseRejected], 0),
        &AttentionConfig::default(),
        now(),
    )?;

    assert_eq!(page.findings.len(), 2);
    for finding in &page.findings {
        assert_eq!(finding.basis_at, None);
        assert!(!finding.reason.contains(" on "), "{}", finding.reason);
    }
    Ok(())
}

#[test]
fn a_superseder_that_states_no_time_is_dated_by_the_edge_that_links_it() -> Result<()> {
    let scenario = Scenario::new();
    let crew = "agent:claude:crew";
    scenario.decision(
        "d:premise",
        "Use the old queue",
        crew,
        "2026-02-01T00:00:00Z",
    )?;
    let proposal = scenario.decision("d:follower", "Batch jobs", crew, "2026-03-01T00:00:00Z")?;
    scenario.relation(
        "FOLLOWS_FROM",
        "d:follower",
        "d:premise",
        crew,
        Some(proposal),
        "2026-03-01T00:00:01Z",
    )?;
    // Named by a request and never proposed, so its node states no time.
    scenario.request_naming("d:stub-new", "2026-04-01T00:00:00Z")?;
    scenario.supersede("d:premise", "d:stub-new", crew, "2026-04-01T00:00:01Z")?;
    let graph = scenario.graph()?;

    let findings = findings_at(
        &graph,
        &[FindingKind::PremiseSuperseded],
        &AttentionConfig::default(),
        now(),
    )?;

    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].kind, FindingKind::PremiseSuperseded);
    // The stub states no time, the supersession does: the change is dated by the edge.
    assert_eq!(findings[0].basis_at, Some(ts("2026-04-01T00:00:01Z")));
    assert_eq!(
        findings[0].node_ids,
        ["d:follower", "d:premise", "d:stub-new"]
    );
    assert_eq!(
        findings[0].reason,
        "the decision it follows from, 'Use the old queue', was superseded by 'd:stub-new' on 2026-04-01"
    );
    Ok(())
}

#[test]
fn a_decision_that_superseded_its_own_premise_is_not_flagged_for_it() -> Result<()> {
    let scenario = Scenario::new();
    let crew = "agent:claude:crew";
    let follows = |decision_id: &str, premise_id: &str, timestamp: &str| -> Result<()> {
        scenario.relation(
            "FOLLOWS_FROM",
            decision_id,
            premise_id,
            crew,
            None,
            timestamp,
        )
    };
    scenario.decision("d:old", "Use the old queue", crew, "2026-02-01T00:00:00Z")?;
    scenario.decision(
        "d:refiner",
        "Use the old queue, tuned",
        crew,
        "2026-03-01T00:00:00Z",
    )?;
    scenario.decision("d:bystander", "Batch jobs", crew, "2026-03-02T00:00:00Z")?;
    follows("d:refiner", "d:old", "2026-03-01T00:00:01Z")?;
    follows("d:bystander", "d:old", "2026-03-02T00:00:01Z")?;
    scenario.supersede("d:old", "d:refiner", crew, "2026-03-01T00:00:02Z")?;
    // A premise superseded twice, once by the decision that follows it and once by another.
    scenario.decision("d:old2", "Use one region", crew, "2026-02-02T00:00:00Z")?;
    scenario.decision("d:both", "Use two regions", crew, "2026-03-10T00:00:00Z")?;
    scenario.decision("d:x", "Use three regions", crew, "2026-05-01T00:00:00Z")?;
    follows("d:both", "d:old2", "2026-03-10T00:00:01Z")?;
    scenario.supersede("d:old2", "d:both", crew, "2026-03-10T00:00:02Z")?;
    scenario.supersede("d:old2", "d:x", crew, "2026-05-01T00:00:01Z")?;
    let graph = scenario.graph()?;

    let findings = all_findings(&graph)?;

    // The refiner is the change. The bystander did not make it. `d:both` is told about the
    // other supersession, dated by that one and not by its own. The same goes for the premises:
    // `d:old` is unfinished for the bystander alone (the refiner is the change, so it is not
    // counted), and `d:old2` for `d:both`, which is only one of its two superseders.
    assert_eq!(
        summaries(&findings),
        vec![
            (
                "d:both".to_owned(),
                FindingKind::PremiseSuperseded,
                vec!["d:both".to_owned(), "d:old2".to_owned(), "d:x".to_owned()],
                Some(ts("2026-05-01T00:00:01Z")),
            ),
            (
                "d:bystander".to_owned(),
                FindingKind::PremiseSuperseded,
                vec![
                    "d:bystander".to_owned(),
                    "d:old".to_owned(),
                    "d:refiner".to_owned()
                ],
                Some(ts("2026-03-01T00:00:02Z")),
            ),
            (
                "d:old".to_owned(),
                FindingKind::PropagationUnfinished,
                vec!["d:old".to_owned(), "d:refiner".to_owned()],
                Some(ts("2026-03-01T00:00:02Z")),
            ),
            (
                "d:old2".to_owned(),
                FindingKind::PropagationUnfinished,
                vec!["d:both".to_owned(), "d:old2".to_owned()],
                Some(ts("2026-03-10T00:00:02Z")),
            ),
        ]
    );
    assert_eq!(
        findings[2].reason,
        "it was superseded by 'Use the old queue, tuned' on 2026-03-01; more than 14 days later, 1 of 1 decision resting on it is not yet recorded as still holding"
    );
    Ok(())
}

#[test]
fn a_follows_from_cycle_is_read_once_and_never_loops() -> Result<()> {
    let graph = attention_scenario()?.graph()?;

    let findings = all_findings(&graph)?;
    let of_cycle: Vec<&AttentionFinding> = findings
        .iter()
        .filter(|finding| finding.decision_id.starts_with("d:cycle"))
        .collect();

    // `d:cycle-a` follows a superseded premise as well; the two decisions that only follow each
    // other, and the one that follows itself, have nothing that changed.
    assert_eq!(of_cycle.len(), 1);
    assert_eq!(of_cycle[0].decision_id, "d:cycle-a");
    assert_eq!(of_cycle[0].kind, FindingKind::PremiseSuperseded);
    Ok(())
}

// ── evidence and its window ───────────────────────────────────────────────────

#[test]
fn evidence_is_not_re_checked_when_the_newest_item_linked_is_older_than_the_window() -> Result<()> {
    let graph = attention_scenario()?.graph()?;
    let findings = all_findings(&graph)?;

    let stale: BTreeSet<&str> = findings
        .iter()
        .filter(|finding| finding.kind == FindingKind::EvidenceNotRechecked)
        .map(|finding| finding.decision_id.as_str())
        .collect();

    // 90 days before 2026-09-26 is exactly 2026-06-28T00:00:00Z: evidence recorded then
    // (`d:ev-boundary`) is not yet old, one second earlier (`d:ev-just-over`) is.
    assert_eq!(
        stale,
        BTreeSet::from(["d:ev-just-over", "d:ev-stale", "d:ev-stale-two"])
    );
    // The newest item is the basis, not the oldest.
    let two = findings
        .iter()
        .find(|finding| finding.decision_id == "d:ev-stale-two")
        .expect("flagged");
    assert_eq!(two.basis_at, Some(ts("2026-03-01T00:00:00Z")));
    assert!(two.node_ids.contains(&"e:old".to_owned()));
    // A recent item anywhere among the evidence, at capture or linked later, is a re-check.
    let flagged = decisions_flagged(&findings);
    for decision_id in [
        "d:ev-fresh",
        "d:ev-mixed",
        "d:ev-rechecked-later",
        "d:ev-none",
    ] {
        assert!(!flagged.contains(decision_id), "{decision_id}");
    }
    Ok(())
}

#[test]
fn the_window_is_configurable() -> Result<()> {
    let graph = attention_scenario()?.graph()?;
    let stale_at = |days: u32| -> Result<BTreeSet<String>> {
        Ok(attention_findings_at(
            &graph,
            &request(&[FindingKind::EvidenceNotRechecked], 0),
            &AttentionConfig {
                evidence_window_days: days,
                ..AttentionConfig::default()
            },
            now(),
        )?
        .findings
        .into_iter()
        .map(|finding| finding.decision_id)
        .collect())
    };

    let quarter = stale_at(DEFAULT_EVIDENCE_WINDOW_DAYS)?;
    assert_eq!(quarter.len(), 3);
    // A month: the June notes and the boundary are old too, the dashboard readings are not.
    let month = stale_at(30)?;
    assert!(month.contains("d:ev-boundary"));
    assert!(!month.contains("d:ev-fresh"));
    assert!(month.is_superset(&quarter));
    // No window at all: everything cited before now is old.
    let none = stale_at(0)?;
    assert!(none.contains("d:ev-fresh"));
    assert!(none.contains("d:ev-mixed"));
    assert!(!none.contains("d:ev-none"));
    // A window longer than the record flags nothing, and cannot overflow the clock.
    assert!(stale_at(u32::MAX)?.is_empty());
    assert!(stale_at(365 * 10)?.is_empty());
    Ok(())
}

#[test]
fn a_page_says_which_clock_and_window_it_was_derived_with() -> Result<()> {
    let graph = attention_scenario()?.graph()?;

    let page = attention_findings_at(
        &graph,
        &request(&[], 0),
        &AttentionConfig {
            evidence_window_days: 45,
            ..AttentionConfig::default()
        },
        now(),
    )?;

    assert_eq!(page.as_of, now());
    assert_eq!(page.evidence_window_days, 45);
    Ok(())
}

// ── identity ──────────────────────────────────────────────────────────────────

#[test]
fn a_finding_id_is_a_fixed_hash_of_kind_nodes_and_basis() {
    // Pinned: consumers dedupe on these across releases, so the input must not drift.
    assert_eq!(
        finding_id(
            FindingKind::BetPastCheckDate,
            &["d:1".to_owned(), "h:1".to_owned()],
            Some(ts("2026-09-01T00:00:00Z")),
        ),
        "finding-9759111b9669b66ccdf1b44757e6d2b6"
    );
    assert_eq!(
        finding_id(
            FindingKind::PremiseRejected,
            &["d:1".to_owned(), "d:2".to_owned()],
            None,
        ),
        "finding-95e5128ede038972a92a5e4f407c5f08"
    );
}

#[test]
fn a_finding_id_changes_with_the_kind_the_nodes_or_the_basis() {
    let nodes = ["d:1".to_owned(), "h:1".to_owned()];
    let at = Some(ts("2026-09-01T00:00:00Z"));
    let base = finding_id(FindingKind::BetPastCheckDate, &nodes, at);

    assert_eq!(base, finding_id(FindingKind::BetPastCheckDate, &nodes, at));
    assert_ne!(base, finding_id(FindingKind::BetFailed, &nodes, at));
    assert_ne!(
        base,
        finding_id(
            FindingKind::BetPastCheckDate,
            &["d:1".to_owned(), "h:2".to_owned()],
            at
        )
    );
    assert_ne!(
        base,
        finding_id(
            FindingKind::BetPastCheckDate,
            &nodes,
            Some(ts("2026-09-01T00:00:01Z"))
        )
    );
    assert_ne!(
        base,
        finding_id(FindingKind::BetPastCheckDate, &nodes, None)
    );
}

#[test]
fn ids_are_the_same_on_every_read_and_on_a_graph_rebuilt_from_the_ledger() -> Result<()> {
    let scenario = attention_scenario()?;
    let graph = scenario.graph()?;
    let rebuilt = scenario.graph()?;

    let first = all_findings(&graph)?;

    assert_eq!(first, all_findings(&graph)?);
    assert_eq!(first, all_findings(&rebuilt)?);
    assert_eq!(
        serde_json::to_string(&first).expect("serializes"),
        serde_json::to_string(&all_findings(&rebuilt)?).expect("serializes")
    );
    let ids: BTreeSet<&str> = first.iter().map(|f| f.finding_id.as_str()).collect();
    assert_eq!(ids.len(), first.len(), "every finding has its own id");
    Ok(())
}

#[test]
fn an_id_does_not_depend_on_the_clock_while_the_basis_holds() -> Result<()> {
    let graph = attention_scenario()?.graph()?;
    let config = AttentionConfig::default();
    let at = |now: DateTime<Utc>| -> Result<Vec<AttentionFinding>> {
        Ok(attention_findings_at(&graph, &request(&[], MAX_QUERY_RESULTS), &config, now)?.findings)
    };

    let today = at(now())?;
    let next_week = at(now() + Duration::days(7))?;

    for finding in &today {
        let later = next_week
            .iter()
            .find(|later| later.decision_id == finding.decision_id && later.kind == finding.kind)
            .expect("still flagged a week on");
        assert_eq!(
            later.finding_id, finding.finding_id,
            "{}",
            finding.decision_id
        );
    }
    Ok(())
}

#[test]
fn newer_evidence_gives_the_finding_a_new_id_and_a_newer_basis() -> Result<()> {
    let scenario = attention_scenario()?;
    let before = all_findings(&scenario.graph()?)?;
    // Another benchmark, still outside the window, is linked to `d:ev-stale` afterwards.
    scenario.relation(
        "BASED_ON",
        "d:ev-stale",
        "e:just-over",
        "human:alex",
        None,
        "2026-09-20T00:00:00Z",
    )?;

    let after = all_findings(&scenario.graph()?)?;
    let stale = |findings: &[AttentionFinding]| -> AttentionFinding {
        findings
            .iter()
            .find(|finding| finding.decision_id == "d:ev-stale")
            .expect("flagged")
            .clone()
    };

    let (old, new) = (stale(&before), stale(&after));
    assert_ne!(old.finding_id, new.finding_id);
    assert_eq!(old.basis_at, Some(ts("2026-03-01T00:00:00Z")));
    assert_eq!(new.basis_at, Some(ts("2026-06-27T23:59:59Z")));
    // Everything else stands as it was, ids included.
    let others = |findings: &[AttentionFinding]| -> Vec<AttentionFinding> {
        findings
            .iter()
            .filter(|finding| finding.decision_id != "d:ev-stale")
            .cloned()
            .collect()
    };
    assert_eq!(others(&before), others(&after));
    Ok(())
}

#[test]
fn a_supersession_stamped_earlier_moves_the_basis_and_the_id_of_a_premise_finding() -> Result<()> {
    let scenario = attention_scenario()?;
    let before = all_findings(&scenario.graph()?)?;
    scenario.decision(
        "d:p-older-new",
        "A queue nobody remembers",
        "agent:claude:crew",
        "2026-07-01T00:00:00Z",
    )?;
    // Recorded after the first supersession, by offset, but stamped a month before it.
    scenario.supersede(
        "d:p-old",
        "d:p-older-new",
        "agent:claude:crew",
        "2026-07-01T00:00:01Z",
    )?;

    let after = all_findings(&scenario.graph()?)?;
    let of = |findings: &[AttentionFinding]| -> AttentionFinding {
        findings
            .iter()
            .find(|finding| {
                finding.decision_id == "d:f-superseded"
                    && finding.kind == FindingKind::PremiseSuperseded
            })
            .expect("flagged")
            .clone()
    };

    let (old, new) = (of(&before), of(&after));
    assert_eq!(old.basis_at, Some(ts("2026-08-01T00:00:01Z")));
    assert_eq!(new.basis_at, Some(ts("2026-07-01T00:00:01Z")));
    assert!(new.node_ids.contains(&"d:p-older-new".to_owned()));
    assert_ne!(old.finding_id, new.finding_id);
    Ok(())
}

#[test]
fn a_superseder_linked_later_does_not_move_the_basis_however_old_the_decision_is() -> Result<()> {
    let scenario = attention_scenario()?;
    let before = all_findings(&scenario.graph()?)?;
    // Captured in July, linked as a superseder of the same premise in September: the premise
    // stopped standing at the first link, and the later link changes neither the date nor the id.
    scenario.decision(
        "d:p-older-new",
        "A queue nobody remembers",
        "agent:claude:crew",
        "2026-07-01T00:00:00Z",
    )?;
    scenario.supersede(
        "d:p-old",
        "d:p-older-new",
        "agent:claude:crew",
        "2026-09-21T00:00:00Z",
    )?;

    let after = all_findings(&scenario.graph()?)?;
    let of = |findings: &[AttentionFinding]| -> AttentionFinding {
        findings
            .iter()
            .find(|finding| {
                finding.decision_id == "d:f-superseded"
                    && finding.kind == FindingKind::PremiseSuperseded
            })
            .expect("flagged")
            .clone()
    };

    let (old, new) = (of(&before), of(&after));
    assert_eq!(new.basis_at, Some(ts("2026-08-01T00:00:01Z")));
    assert!(!new.node_ids.contains(&"d:p-older-new".to_owned()));
    assert_eq!(old.finding_id, new.finding_id);
    Ok(())
}

// ── paging ────────────────────────────────────────────────────────────────────

#[test]
fn a_page_that_is_cut_short_says_so_and_says_where_to_resume() -> Result<()> {
    let graph = attention_scenario()?.graph()?;
    let config = AttentionConfig::default();

    let first = attention_findings_at(&graph, &request(&[], 5), &config, now())?;
    assert_eq!(first.findings.len(), 5);
    assert!(first.truncated);
    let cursor = first
        .next_cursor
        .clone()
        .expect("a cut-short page has a cursor");

    let second = attention_findings_at(
        &graph,
        &AttentionRequest {
            cursor: Some(cursor),
            ..request(&[], 5)
        },
        &config,
        now(),
    )?;

    let expected = expected_scenario_findings();
    assert_eq!(summaries(&first.findings), expected[..5]);
    assert_eq!(summaries(&second.findings), expected[5..10]);
    assert!(second.truncated);
    Ok(())
}

#[test]
fn a_page_that_holds_everything_left_is_not_truncated() -> Result<()> {
    let graph = attention_scenario()?.graph()?;
    let config = AttentionConfig::default();

    let exact = attention_findings_at(&graph, &request(&[], 15), &config, now())?;
    let one_short = attention_findings_at(&graph, &request(&[], 14), &config, now())?;

    assert_eq!(exact.findings.len(), 15);
    assert!(!exact.truncated);
    assert_eq!(exact.next_cursor, None);
    assert_eq!(one_short.findings.len(), 14);
    assert!(one_short.truncated);
    assert!(one_short.next_cursor.is_some());
    Ok(())
}

#[test]
fn walking_the_cursors_visits_every_finding_once_whatever_the_page_size() -> Result<()> {
    let graph = attention_scenario()?.graph()?;
    let expected = expected_scenario_findings();

    for limit in [1, 2, 3, 5, 7, 14, 15, 16, 1000] {
        let (sizes, findings) = walk(
            &graph,
            &request(&[], limit),
            &AttentionConfig::default(),
            now(),
        )?;
        assert_eq!(summaries(&findings), expected, "limit {limit}");
        assert!(sizes.iter().all(|size| *size <= limit), "limit {limit}");
    }
    Ok(())
}

/// `count` decisions, each resting on its own bet whose check date has passed.
fn overdue_bet_scenario(count: usize) -> Result<Scenario> {
    let scenario = Scenario::new();
    for index in 0..count {
        let hypothesis_id = format!("h:overdue-{index:05}");
        scenario.hypothesis(
            &hypothesis_id,
            "A bet",
            "bet",
            Some("2026-09-01T00:00:00Z"),
            "2026-01-01T00:00:00Z",
        )?;
        scenario.decision_with(
            &format!("d:overdue-{index:05}"),
            "Overdue decision",
            "agent:claude:crew",
            "2026-02-01T00:00:00Z",
            true,
            &[],
            &[hypothesis_id.as_str()],
            None,
        )?;
    }
    Ok(scenario)
}

#[test]
fn a_limit_of_zero_is_the_default_page_and_a_huge_one_is_capped() -> Result<()> {
    let graph = overdue_bet_scenario(MAX_QUERY_RESULTS + 100)?.graph()?;
    let config = AttentionConfig::default();

    let default = attention_findings_at(&graph, &request(&[], 0), &config, now())?;
    let capped = attention_findings_at(&graph, &request(&[], usize::MAX), &config, now())?;

    assert_eq!(default.findings.len(), DEFAULT_PAGE_SIZE);
    assert!(default.truncated);
    // Never more than `MAX_QUERY_RESULTS`, and the rest is still reachable.
    assert_eq!(capped.findings.len(), MAX_QUERY_RESULTS);
    assert!(capped.truncated);
    assert!(capped.next_cursor.is_some());
    Ok(())
}

#[test]
fn the_cursor_is_a_position_so_findings_that_appear_earlier_do_not_shift_the_next_page(
) -> Result<()> {
    let scenario = attention_scenario()?;
    let config = AttentionConfig::default();
    let before = scenario.graph()?;
    let first = attention_findings_at(&before, &request(&[], 5), &config, now())?;
    let cursor = first.next_cursor.clone().expect("cut short");
    let second_before = attention_findings_at(
        &before,
        &AttentionRequest {
            cursor: Some(cursor.clone()),
            ..request(&[], 5)
        },
        &config,
        now(),
    )?;

    // Two decisions that sort before everything else come into attention between the pages.
    scenario.hypothesis(
        "h:early",
        "Early bet",
        "bet",
        Some("2026-09-02T00:00:00Z"),
        "2026-09-22T00:00:00Z",
    )?;
    for decision_id in ["d:a-1", "d:a-2"] {
        scenario.decision_with(
            decision_id,
            "Early decision",
            "agent:claude:crew",
            "2026-09-22T00:00:01Z",
            true,
            &[],
            &["h:early"],
            None,
        )?;
    }
    let after = scenario.graph()?;
    let second_after = attention_findings_at(
        &after,
        &AttentionRequest {
            cursor: Some(cursor),
            ..request(&[], 5)
        },
        &config,
        now(),
    )?;
    let fresh = attention_findings_at(&after, &request(&[], 2), &config, now())?;

    assert_eq!(second_after.findings, second_before.findings);
    assert_eq!(
        fresh
            .findings
            .iter()
            .map(|finding| finding.decision_id.as_str())
            .collect::<Vec<_>>(),
        ["d:a-1", "d:a-2"]
    );
    Ok(())
}

#[test]
fn a_cursor_an_attention_scan_did_not_return_is_refused() -> Result<()> {
    let graph = attention_scenario()?.graph()?;

    for cursor in [
        "5",
        "not json",
        "",
        r#"{"decision_id":"d:1","kind":"no_such_kind","subject_id":"x"}"#,
        r#"{"decision_id":"d:1"}"#,
    ] {
        let result = attention_findings_at(
            &graph,
            &AttentionRequest {
                cursor: Some(cursor.to_owned()),
                ..request(&[], 5)
            },
            &AttentionConfig::default(),
            now(),
        );
        assert!(result.is_err(), "{cursor:?}");
    }
    Ok(())
}

#[test]
fn an_empty_graph_has_no_findings() -> Result<()> {
    let graph = Scenario::new().graph()?;

    let page = attention_findings_at(&graph, &request(&[], 0), &AttentionConfig::default(), now())?;

    assert!(page.findings.is_empty());
    assert!(!page.truncated);
    assert_eq!(page.next_cursor, None);
    Ok(())
}

// ── shape ─────────────────────────────────────────────────────────────────────

#[test]
fn every_finding_lists_its_decision_and_sorted_distinct_node_ids() -> Result<()> {
    let graph = attention_scenario()?.graph()?;

    for finding in all_findings(&graph)? {
        assert!(finding.node_ids.contains(&finding.decision_id));
        let mut sorted = finding.node_ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(finding.node_ids, sorted, "{}", finding.decision_id);
        assert!(finding.finding_id.starts_with("finding-"));
        assert_eq!(finding.finding_id.len(), "finding-".len() + 32);
        assert!(!finding.reason.is_empty());
    }
    Ok(())
}

#[test]
fn each_finding_says_in_words_what_changed_or_lapsed() -> Result<()> {
    let graph = attention_scenario()?.graph()?;
    let findings = all_findings(&graph)?;
    let reason_of = |decision_id: &str, kind: FindingKind| {
        findings
            .iter()
            .find(|finding| finding.decision_id == decision_id && finding.kind == kind)
            .map(|finding| finding.reason.as_str())
            .expect("the finding exists")
    };

    assert_eq!(
        reason_of("d:bet-overdue", FindingKind::BetPastCheckDate),
        "the bet 'Load stays flat' was to be checked by 2026-09-01; nothing has been recorded for or against it"
    );
    assert_eq!(
        reason_of("d:f-superseded", FindingKind::PremiseSuperseded),
        "the decision it follows from, 'Use the old queue', was superseded by 'Use the new queue' on 2026-08-01"
    );
    assert_eq!(
        reason_of("d:f-rejected", FindingKind::PremiseRejected),
        "the decision it follows from, 'Adopt the vendor SDK', was rejected"
    );
    assert_eq!(
        reason_of("d:assumption-refuted", FindingKind::AssumptionRefuted),
        "the assumption it rests on, 'Users log in daily', was refuted by 'Half of users log in weekly' on 2026-06-01"
    );
    assert_eq!(
        reason_of("d:bet-failed", FindingKind::BetFailed),
        "the bet it rests on, 'Latency stays low', failed: refuted by 'p95 doubled' on 2026-08-15"
    );
    assert_eq!(
        reason_of("d:ev-stale", FindingKind::EvidenceNotRechecked),
        "the newest evidence it rests on, 'Benchmark from March', was recorded on 2026-03-01, more than 90 days ago"
    );
    // Five decisions rest on `d:p-old`; two went out themselves after it was superseded, so three
    // are open. No person is named.
    assert_eq!(
        reason_of("d:p-old", FindingKind::PropagationUnfinished),
        "it was superseded by 'Use the new queue' on 2026-08-01; more than 14 days later, 3 of 5 decisions resting on it are not yet recorded as still holding"
    );
    Ok(())
}

#[test]
fn a_one_day_window_reads_day_not_days() -> Result<()> {
    let graph = attention_scenario()?.graph()?;

    let page = attention_findings_at(
        &graph,
        &request(&[FindingKind::EvidenceNotRechecked], 1),
        &AttentionConfig {
            evidence_window_days: 1,
            ..AttentionConfig::default()
        },
        now(),
    )?;

    assert_eq!(page.findings.len(), 1);
    assert!(
        page.findings[0].reason.ends_with("more than 1 day ago"),
        "{}",
        page.findings[0].reason
    );
    Ok(())
}

#[test]
fn the_wire_form_names_kinds_in_snake_case_and_omits_what_is_absent() -> Result<()> {
    let graph = attention_scenario()?.graph()?;

    let page = attention_findings_at(
        &graph,
        &request(&[FindingKind::PremiseRejected], 0),
        &AttentionConfig::default(),
        now(),
    )?;
    let wire = serde_json::to_value(&page).expect("page serializes");

    assert_eq!(wire["truncated"], false);
    assert!(wire.get("next_cursor").is_none());
    assert_eq!(wire["evidence_window_days"], 90);
    let first = &wire["findings"][0];
    assert_eq!(first["kind"], "premise_rejected");
    assert!(first.get("basis_at").is_none());
    assert!(first["finding_id"].as_str().is_some());
    // No grade of any kind.
    for word in ["score", "tier", "rank", "grade"] {
        assert!(first.get(word).is_none(), "{word}");
    }
    Ok(())
}

#[test]
fn kinds_go_by_their_wire_names() {
    let names: Vec<&str> = FindingKind::ALL.iter().map(|kind| kind.as_str()).collect();

    assert_eq!(
        names,
        [
            "bet_past_check_date",
            "premise_superseded",
            "premise_rejected",
            "assumption_refuted",
            "bet_failed",
            "evidence_not_rechecked",
            "propagation_unfinished"
        ]
    );
    for kind in FindingKind::ALL {
        assert_eq!(
            serde_json::to_value(kind).expect("serializes"),
            kind.as_str()
        );
    }
    let mut sorted = FindingKind::ALL;
    sorted.sort();
    assert_eq!(
        sorted,
        FindingKind::ALL,
        "declaration order is the sort order"
    );
}

// ── cost ──────────────────────────────────────────────────────────────────────

/// `decisions` decisions, each resting on a bet (every fifth is overdue), plus a tenth as many
/// decisions following a superseded premise and citing stale evidence. Findings: 1/2 of
/// `decisions` (a fifth, a tenth and a tenth for the three kinds, and another tenth for the
/// superseded premise nobody has looked at since).
fn bulk_scenario(decisions: usize) -> Result<Scenario> {
    let scenario = Scenario::new();
    let crew = "agent:claude:crew";
    for index in 0..decisions {
        let check_by = if index % 5 == 0 {
            "2026-09-01T00:00:00Z"
        } else {
            "2026-12-01T00:00:00Z"
        };
        let hypothesis_id = format!("h:bulk-{index:05}");
        scenario.hypothesis(
            &hypothesis_id,
            "A bet",
            "bet",
            Some(check_by),
            "2026-01-01T00:00:00Z",
        )?;
        scenario.decision_with(
            &format!("d:bulk-{index:05}"),
            "Bulk decision",
            crew,
            "2026-02-01T00:00:00Z",
            true,
            &[],
            &[hypothesis_id.as_str()],
            None,
        )?;
    }
    for index in 0..decisions / 10 {
        let (old, new, dependent) = (
            format!("d:old-{index:05}"),
            format!("d:new-{index:05}"),
            format!("d:dep-{index:05}"),
        );
        scenario.decision(&old, "Old premise", crew, "2026-02-01T00:00:00Z")?;
        scenario.decision(&new, "New premise", crew, "2026-08-01T00:00:00Z")?;
        scenario.supersede(&old, &new, crew, "2026-08-01T00:00:01Z")?;
        let proposal = scenario.decision(&dependent, "Dependent", crew, "2026-04-01T00:00:00Z")?;
        scenario.relation(
            "FOLLOWS_FROM",
            &dependent,
            &old,
            crew,
            Some(proposal),
            "2026-04-01T00:00:01Z",
        )?;

        let evidence_id = format!("e:stale-{index:05}");
        scenario.evidence(&evidence_id, "Old reading", None, "2026-03-01T00:00:00Z")?;
        scenario.decision_with(
            &format!("d:ev-{index:05}"),
            "Cites old evidence",
            crew,
            "2026-04-02T00:00:00Z",
            false,
            &[evidence_id.as_str()],
            &[],
            None,
        )?;
    }
    Ok(scenario)
}

#[test]
fn a_page_costs_a_fixed_number_of_bulk_reads_however_many_decisions_there_are() -> Result<()> {
    let config = AttentionConfig::default();
    let reads = |decisions: usize, kinds: &[FindingKind], limit: usize| -> Result<usize> {
        let graph = bulk_scenario(decisions)?.graph()?;
        let counting = CountingGraph::new(&graph);
        let page = attention_findings_at(&counting, &request(kinds, limit), &config, now())?;
        assert_eq!(page.findings.len(), limit);
        Ok(counting.queries())
    };

    // Bets and stale evidence each cost two anchored reads per finding -- the decision's own
    // anchor (for its title) and the bet's statement or the evidence's content -- and every id in
    // `bulk_scenario` is distinct, so it is the same reads for 50 decisions and for 800.
    for kinds in [
        &[FindingKind::BetPastCheckDate][..],
        &[FindingKind::EvidenceNotRechecked][..],
    ] {
        assert_eq!(reads(50, kinds, 5)?, GROUNDING_FACT_READS + 10);
        assert_eq!(reads(800, kinds, 5)?, GROUNDING_FACT_READS + 10);
    }
    Ok(())
}

#[test]
fn a_page_of_superseded_premises_adds_three_anchored_reads_per_finding_when_every_id_is_distinct(
) -> Result<()> {
    let graph = bulk_scenario(800)?.graph()?;
    let counting = CountingGraph::new(&graph);

    let page = attention_findings_at(
        &counting,
        &request(&[FindingKind::PremiseSuperseded], 25),
        &AttentionConfig::default(),
        now(),
    )?;

    assert_eq!(page.findings.len(), 25);
    assert!(page.truncated);
    // Each dependent has its own premise and superseder here, all three distinct decisions, so
    // 25 findings are 25 * 3 = 75 anchored reads (the decision itself, its premise and the
    // premise's superseder).
    assert_eq!(counting.queries(), GROUNDING_FACT_READS + 25 * 3);
    Ok(())
}

#[test]
fn a_large_graph_pages_through_every_finding_once_within_the_stated_cost() -> Result<()> {
    let decisions = 800;
    let graph = bulk_scenario(decisions)?.graph()?;
    let config = AttentionConfig::default();
    let limit = 60;

    let mut cursor = None;
    let mut seen = Vec::new();
    let mut pages = 0;
    loop {
        let counting = CountingGraph::new(&graph);
        let page = attention_findings_at(
            &counting,
            &AttentionRequest {
                kinds: Vec::new(),
                limit,
                cursor: cursor.take(),
                ..AttentionRequest::default()
            },
            &config,
            now(),
        )?;
        assert!(page.findings.len() <= limit);
        // At most three anchored reads per finding (the decision, its subject and its basis, when
        // all three are distinct decisions -- the `premise_superseded` worst case here).
        assert!(counting.queries() <= GROUNDING_FACT_READS + limit * 3);
        seen.extend(page.findings);
        pages += 1;
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }

    // 160 overdue bets, 80 superseded premises, 80 stale evidence items, and the 80 premises
    // themselves, still unlooked at.
    assert_eq!(seen.len(), decisions / 2);
    assert_eq!(pages, seen.len().div_ceil(limit));
    let ids: BTreeSet<&str> = seen.iter().map(|f| f.finding_id.as_str()).collect();
    assert_eq!(ids.len(), seen.len(), "no finding twice");
    let one_page = all_findings(&graph)?;
    assert_eq!(seen, one_page);
    Ok(())
}

// ── leaving findings out ──────────────────────────────────────────────────────

fn finding_ids(findings: &[AttentionFinding]) -> BTreeSet<String> {
    findings
        .iter()
        .map(|finding| finding.finding_id.clone())
        .collect()
}

fn leaving_out(
    excluded: &BTreeSet<String>,
    kinds: &[FindingKind],
    limit: usize,
) -> AttentionRequest {
    AttentionRequest {
        excluded: excluded.clone(),
        ..request(kinds, limit)
    }
}

#[test]
fn a_finding_that_is_left_out_is_on_no_page_and_the_others_keep_their_order() -> Result<()> {
    let graph = attention_scenario()?.graph()?;
    let everything = all_findings(&graph)?;
    // Every third finding, so the gaps fall inside pages and on their edges.
    let excluded = finding_ids(&everything.iter().step_by(3).cloned().collect::<Vec<_>>());
    let expected: Vec<AttentionFinding> = everything
        .iter()
        .filter(|finding| !excluded.contains(&finding.finding_id))
        .cloned()
        .collect();
    assert_eq!(expected.len(), 10);

    for limit in [1, 2, 3, 4, 7, 10, 11, 15, 1000] {
        let (sizes, findings) = walk(
            &graph,
            &leaving_out(&excluded, &[], limit),
            &AttentionConfig::default(),
            now(),
        )?;
        assert_eq!(findings, expected, "limit {limit}");
        // Leaving findings out never leaves a short page in the middle of a walk.
        let (last, before_last) = sizes.split_last().expect("a page");
        assert!(
            before_last.iter().all(|size| *size == limit),
            "limit {limit}: {sizes:?}"
        );
        assert!((1..=limit).contains(last), "limit {limit}: {sizes:?}");
    }
    Ok(())
}

#[test]
fn a_page_is_truncated_only_when_a_finding_that_is_not_left_out_follows() -> Result<()> {
    let graph = attention_scenario()?.graph()?;
    let config = AttentionConfig::default();
    let everything = all_findings(&graph)?;
    // Everything from the fifth finding on is left out.
    let excluded = finding_ids(&everything[5..]);

    let exact = attention_findings_at(&graph, &leaving_out(&excluded, &[], 5), &config, now())?;
    assert_eq!(exact.findings, everything[..5]);
    assert!(!exact.truncated);
    assert_eq!(exact.next_cursor, None);

    // One short of the five that remain: the fifth follows, and the next page is that one alone.
    let one_short = attention_findings_at(&graph, &leaving_out(&excluded, &[], 4), &config, now())?;
    assert_eq!(one_short.findings, everything[..4]);
    assert!(one_short.truncated);
    let rest = attention_findings_at(
        &graph,
        &AttentionRequest {
            cursor: one_short.next_cursor,
            ..leaving_out(&excluded, &[], 4)
        },
        &config,
        now(),
    )?;
    assert_eq!(rest.findings, everything[4..5]);
    assert!(!rest.truncated);
    assert_eq!(rest.next_cursor, None);
    Ok(())
}

#[test]
fn leaving_out_every_finding_leaves_an_empty_page_that_is_not_truncated() -> Result<()> {
    let graph = attention_scenario()?.graph()?;
    let excluded = finding_ids(&all_findings(&graph)?);

    let page = attention_findings_at(
        &graph,
        &leaving_out(&excluded, &[], 5),
        &AttentionConfig::default(),
        now(),
    )?;

    assert!(page.findings.is_empty());
    assert!(!page.truncated);
    assert_eq!(page.next_cursor, None);
    Ok(())
}

#[test]
fn an_id_that_matches_no_finding_leaves_nothing_out() -> Result<()> {
    let graph = attention_scenario()?.graph()?;
    let excluded = BTreeSet::from(["finding-00000000000000000000000000000000".to_owned()]);

    let page = attention_findings_at(
        &graph,
        &leaving_out(&excluded, &[], MAX_QUERY_RESULTS),
        &AttentionConfig::default(),
        now(),
    )?;

    assert_eq!(page.findings, all_findings(&graph)?);
    Ok(())
}

#[test]
fn a_finding_whose_basis_changed_is_not_left_out_by_its_old_id() -> Result<()> {
    let scenario = attention_scenario()?;
    let excluded = finding_ids(&all_findings(&scenario.graph()?)?);
    // Newer evidence is linked to `d:ev-stale` afterwards: its finding has a new basis.
    scenario.relation(
        "BASED_ON",
        "d:ev-stale",
        "e:just-over",
        "human:alex",
        None,
        "2026-09-20T00:00:00Z",
    )?;

    let page = attention_findings_at(
        &scenario.graph()?,
        &leaving_out(&excluded, &[], MAX_QUERY_RESULTS),
        &AttentionConfig::default(),
        now(),
    )?;

    assert_eq!(page.findings.len(), 1);
    assert_eq!(page.findings[0].decision_id, "d:ev-stale");
    assert_eq!(page.findings[0].basis_at, Some(ts("2026-06-27T23:59:59Z")));
    Ok(())
}

#[test]
fn stepping_over_findings_that_are_left_out_costs_the_same_anchored_reads_each() -> Result<()> {
    let graph = bulk_scenario(800)?.graph()?;
    let config = AttentionConfig::default();
    let kinds = [FindingKind::PremiseSuperseded];
    let first = attention_findings_at(&graph, &request(&kinds, 35), &config, now())?;
    let excluded = finding_ids(&first.findings[..10]);

    let counting = CountingGraph::new(&graph);
    let page = attention_findings_at(
        &counting,
        &leaving_out(&excluded, &kinds, 25),
        &config,
        now(),
    )?;

    assert_eq!(page.findings, first.findings[10..35]);
    assert!(page.truncated);
    // Each dependent has its own premise and superseder here, all three distinct decisions: the
    // ten stepped over, the 25 on the page and the one after it (which is how the page is known
    // to be cut short) each cost three anchored reads.
    assert_eq!(counting.queries(), GROUNDING_FACT_READS + (10 + 25 + 1) * 3);
    Ok(())
}

// ── premises not re-examined, and premises that were (hivemind-ottw.6) ────────────

fn reexamine_now() -> DateTime<Utc> {
    ts(REEXAMINE_NOW)
}

/// The findings of `kinds` (all when empty) as of `at`, in one page, with `config`.
fn findings_at(
    graph: &impl GraphView,
    kinds: &[FindingKind],
    config: &AttentionConfig,
    at: DateTime<Utc>,
) -> Result<Vec<AttentionFinding>> {
    Ok(attention_findings_at(graph, &request(kinds, MAX_QUERY_RESULTS), config, at)?.findings)
}

/// The decisions flagged `propagation_unfinished` on `graph` as of `at`.
fn unfinished_at(
    graph: &impl GraphView,
    config: &AttentionConfig,
    at: DateTime<Utc>,
) -> Result<Vec<String>> {
    Ok(
        findings_at(graph, &[FindingKind::PropagationUnfinished], config, at)?
            .into_iter()
            .map(|finding| finding.decision_id)
            .collect(),
    )
}

/// What every backend must read from `reexamine_scenario` as attention findings at
/// `REEXAMINE_NOW`: the same thirteen findings, whole or paged. The scenario holds a premise
/// superseded in every state an item can be in, so this is where a reaffirmation closing a
/// finding, a reaffirmation that came too early closing nothing, and an unfinished propagation on
/// either side of the window are all read from the graph. Shared by the memory, Postgres and Kuzu
/// tests.
pub(crate) fn assert_propagation_findings(graph: &impl GraphView) -> Result<()> {
    use FindingKind::{
        AssumptionRefuted, PremiseRejected, PremiseSuperseded, PropagationUnfinished,
    };
    // (decision, kind, nodes, basis time). Left out on purpose, and why:
    // - `d:twice`, `d:on-rej-ok`, `d:on-bet`: reaffirmed after the change, so closed;
    // - `d:on-cont2` is here: its reaffirmation came before the premise was superseded;
    // - `d:on-cont`, `d:on-cont3` follow a contested premise, which is shown, not flagged;
    // - `d:p-cont2` (superseded 11 days before), `d:p-edge-eq` (exactly 14) are inside the window;
    // - `d:p-rej`, `d:p-cont`, `d:p-cont3`: no change time, so never unfinished;
    // - `d:lonely`, `d:self`: nothing rests on them.
    let table: [(&str, FindingKind, &[&str], Option<&str>); 13] = [
        (
            "d:cyc-a",
            PropagationUnfinished,
            &["d:cyc-a", "d:cyc-a-new"],
            Some("2026-09-05T00:00:00Z"),
        ),
        (
            "d:cyc-b",
            PremiseSuperseded,
            &["d:cyc-a", "d:cyc-a-new", "d:cyc-b"],
            Some("2026-09-05T00:00:00Z"),
        ),
        (
            "d:late-link",
            PremiseSuperseded,
            &["d:late-link", "d:p-sup", "d:p-sup-new"],
            Some("2026-09-02T00:00:00Z"),
        ),
        (
            "d:on-assumption",
            AssumptionRefuted,
            &["d:on-assumption", "e:r2", "h:assumption"],
            Some("2026-09-03T00:00:00Z"),
        ),
        (
            "d:on-conc",
            PremiseSuperseded,
            &["d:conc-a", "d:on-conc", "d:p-conc"],
            Some("2026-09-10T00:00:00Z"),
        ),
        (
            "d:on-cont2",
            PremiseSuperseded,
            &["d:cont2-new", "d:on-cont2", "d:p-cont2"],
            Some("2026-09-20T00:00:00Z"),
        ),
        (
            "d:on-edge-eq",
            PremiseSuperseded,
            &["d:edge-eq-new", "d:on-edge-eq", "d:p-edge-eq"],
            Some("2026-09-17T00:00:00Z"),
        ),
        (
            "d:on-edge-over",
            PremiseSuperseded,
            &["d:edge-over-new", "d:on-edge-over", "d:p-edge-over"],
            Some("2026-09-16T23:59:59Z"),
        ),
        ("d:on-rej", PremiseRejected, &["d:on-rej", "d:p-rej"], None),
        (
            "d:open",
            PremiseSuperseded,
            &["d:open", "d:p-sup", "d:p-sup-new"],
            Some("2026-09-02T00:00:00Z"),
        ),
        // Dated by the supersession recorded first (`d:conc-b`), where `d:on-conc` above is dated
        // by the earlier one (`d:conc-a`).
        (
            "d:p-conc",
            PropagationUnfinished,
            &["d:conc-b", "d:p-conc"],
            Some("2026-09-12T00:00:00Z"),
        ),
        (
            "d:p-edge-over",
            PropagationUnfinished,
            &["d:edge-over-new", "d:p-edge-over"],
            Some("2026-09-16T23:59:59Z"),
        ),
        (
            "d:p-sup",
            PropagationUnfinished,
            &["d:p-sup", "d:p-sup-new"],
            Some("2026-09-02T00:00:00Z"),
        ),
    ];
    let expected: Vec<Summary> = table
        .iter()
        .map(|(decision_id, kind, node_ids, basis_at)| {
            (
                (*decision_id).to_owned(),
                *kind,
                node_ids.iter().map(|id| (*id).to_owned()).collect(),
                basis_at.map(ts),
            )
        })
        .collect();

    let config = AttentionConfig::default();
    let all = findings_at(graph, &[], &config, reexamine_now())?;
    assert_eq!(summaries(&all), expected);
    for limit in [1, 2, 5, 13, 14] {
        let (sizes, paged) = walk(graph, &request(&[], limit), &config, reexamine_now())?;
        assert_eq!(summaries(&paged), expected, "limit {limit}");
        assert!(sizes.iter().all(|size| *size <= limit), "limit {limit}");
    }
    Ok(())
}

#[test]
fn reaffirmed_dependents_and_unfinished_propagations_read_as_the_re_examine_state_says(
) -> Result<()> {
    let graph = reexamine_scenario()?.graph()?;

    assert_propagation_findings(&graph)
}

/// The count a premise's finding gives ("N of M decisions resting on it") is the count `verify`
/// gives it as "N open of M dependents": one derivation, so a decision that is the whole of the
/// change (the only one to supersede its premise) is no dependent in either (hivemind-tfmfn).
#[test]
fn verify_counts_the_dependents_the_unfinished_finding_counts() -> Result<()> {
    for scenario in [reexamine_scenario()?, revisions_scenario()?] {
        let graph = scenario.graph()?;
        let found = findings_at(
            &graph,
            &[FindingKind::PropagationUnfinished],
            &AttentionConfig::default(),
            reexamine_now(),
        )?;
        assert!(!found.is_empty());

        for finding in &found {
            let verify = get_decision_brief_at(&graph, &finding.decision_id, reexamine_now())?
                .data
                .expect("the premise is recorded")
                .propagation
                .expect("a premise with an open item has a completeness line");
            let said = format!(" later, {} of {} ", verify.open, verify.dependents_total);
            assert!(
                finding.reason.contains(&said),
                "{}: finding says {:?}, verify says {}",
                finding.decision_id,
                finding.reason,
                verify.describe()
            );
        }
    }

    // On the revisions, the finding on `d:old` counts the bystander and not the revision; the
    // finding on `d:two` counts both of its superseders.
    let graph = revisions_scenario()?.graph()?;
    let found = findings_at(
        &graph,
        &[FindingKind::PropagationUnfinished],
        &AttentionConfig::default(),
        reexamine_now(),
    )?;
    let reason_of = |premise_id: &str| {
        found
            .iter()
            .find(|finding| finding.decision_id == premise_id)
            .map(|finding| finding.reason.as_str())
    };
    assert!(reason_of("d:old").is_some_and(|reason| reason.contains(" later, 1 of 1 decision ")));
    assert!(reason_of("d:two").is_some_and(|reason| reason.contains(" later, 2 of 2 decisions ")));
    assert_eq!(reason_of("d:solo"), None);
    Ok(())
}

/// `d:old` was superseded by `d:new` on 2026-08-01, the instant `d:new` was proposed (the change
/// is dated by the supersession, not by the proposal); `d:dep-a` and `d:dep-b` rest on it and
/// nobody has looked at either since.
fn two_dependents() -> Result<Scenario> {
    let scenario = Scenario::new();
    let crew = "agent:claude:crew";
    scenario.decision("d:old", "Use the old queue", crew, "2026-02-01T00:00:00Z")?;
    scenario.decision("d:new", "Use the new queue", crew, "2026-08-01T00:00:00Z")?;
    for (id, title) in [("d:dep-a", "Batch jobs"), ("d:dep-b", "Cron jobs")] {
        scenario.decision(id, title, crew, "2026-04-01T00:00:00Z")?;
        scenario.relation(
            "FOLLOWS_FROM",
            id,
            "d:old",
            crew,
            None,
            "2026-04-01T00:00:01Z",
        )?;
    }
    scenario.supersede("d:old", "d:new", crew, "2026-08-01T00:00:00Z")?;
    Ok(scenario)
}

fn only_unfinished(
    scenario: &Scenario,
    config: &AttentionConfig,
    at: DateTime<Utc>,
) -> Result<Vec<AttentionFinding>> {
    findings_at(
        &scenario.graph()?,
        &[FindingKind::PropagationUnfinished],
        config,
        at,
    )
}

#[test]
fn an_unfinished_propagation_says_how_many_are_open_and_names_nobody() -> Result<()> {
    let scenario = two_dependents()?;
    let config = AttentionConfig::default();

    let before = only_unfinished(&scenario, &config, now())?;
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].decision_id, "d:old");
    assert_eq!(before[0].decision_title, "Use the old queue");
    assert_eq!(before[0].node_ids, ["d:new", "d:old"]);
    assert_eq!(before[0].basis_at, Some(ts("2026-08-01T00:00:00Z")));
    assert_eq!(
        before[0].reason,
        "it was superseded by 'Use the new queue' on 2026-08-01; more than 14 days later, 2 of 2 decisions resting on it are not yet recorded as still holding"
    );

    // One of them is looked at again: the count moves, the finding stays.
    scenario.reaffirmed(
        "d:dep-a",
        "d:old",
        "Batch jobs never used the queue",
        "human:alice",
        "2026-08-02T00:00:00Z",
    )?;
    let half = only_unfinished(&scenario, &config, now())?;
    assert_eq!(half.len(), 1);
    assert_eq!(
        half[0].reason,
        "it was superseded by 'Use the new queue' on 2026-08-01; more than 14 days later, 1 of 2 decisions resting on it are not yet recorded as still holding"
    );
    for text in [&before[0].reason, &half[0].reason] {
        assert!(!text.contains("alice"), "{text}");
    }

    // The other too: nothing is left open, and the finding is gone.
    scenario.reaffirmed(
        "d:dep-b",
        "d:old",
        "Cron jobs never used the queue",
        "agent:codex:reviewer",
        "2026-08-03T00:00:00Z",
    )?;
    assert!(only_unfinished(&scenario, &config, now())?.is_empty());
    // And with it the dependents' own findings for the premise.
    assert!(findings_at(&scenario.graph()?, &[], &config, now())?.is_empty());
    Ok(())
}

#[test]
fn the_id_of_an_unfinished_propagation_ignores_the_counts_and_the_clock() -> Result<()> {
    let scenario = two_dependents()?;
    let config = AttentionConfig::default();
    let id_at = |at: DateTime<Utc>| -> Result<String> {
        let found = only_unfinished(&scenario, &config, at)?;
        assert_eq!(found.len(), 1);
        Ok(found[0].finding_id.clone())
    };

    let first = id_at(now())?;
    // A month on, the same finding.
    assert_eq!(id_at(now() + Duration::days(30))?, first);
    // One of two dependents is reaffirmed: the open count moved, the id did not, so an
    // acknowledgement of the finding still holds.
    scenario.reaffirmed(
        "d:dep-a",
        "d:old",
        "Batch jobs never used the queue",
        "human:alice",
        "2026-08-02T00:00:00Z",
    )?;
    assert_eq!(id_at(now())?, first);
    Ok(())
}

#[test]
fn a_supersession_is_unfinished_only_after_the_window_and_the_window_is_configurable() -> Result<()>
{
    let scenario = two_dependents()?;
    let config = AttentionConfig::default();

    // Exactly 14 days after 2026-08-01T00:00:00Z is not yet past; one second more is.
    assert!(only_unfinished(&scenario, &config, ts("2026-08-15T00:00:00Z"))?.is_empty());
    assert_eq!(
        only_unfinished(&scenario, &config, ts("2026-08-15T00:00:01Z"))?.len(),
        1
    );

    let graph = reexamine_scenario()?.graph()?;
    let flagged = |days: u32| -> Result<Vec<String>> {
        unfinished_at(
            &graph,
            &AttentionConfig {
                propagation_window_days: days,
                ..AttentionConfig::default()
            },
            reexamine_now(),
        )
    };
    // The default two weeks: four premises, one of them 14 days and a second old, none exactly 14.
    assert_eq!(
        flagged(14)?,
        ["d:cyc-a", "d:p-conc", "d:p-edge-over", "d:p-sup"]
    );
    // Only `d:p-sup` (29 days) is older than 28.
    assert_eq!(flagged(28)?, ["d:p-sup"]);
    // No window at all: every superseded premise with an open dependent, inside the old window or
    // not. A window longer than the record flags nothing, and cannot overflow the clock.
    assert_eq!(
        flagged(0)?,
        [
            "d:cyc-a",
            "d:p-conc",
            "d:p-cont2",
            "d:p-edge-eq",
            "d:p-edge-over",
            "d:p-sup"
        ]
    );
    assert!(flagged(u32::MAX)?.is_empty());
    Ok(())
}

#[test]
fn only_a_supersession_can_be_unfinished_whatever_the_clock() -> Result<()> {
    let graph = reexamine_scenario()?.graph()?;

    // Far in the future, every superseded premise with an open dependent is unfinished. A
    // rejected or contested premise (`d:p-rej`, `d:p-cont`, `d:p-cont3`: open dependents, no
    // change time) and a refuted hypothesis (`h:assumption`: not a decision) never are, and
    // neither is a premise nothing rests on (`d:lonely`) or one that rests on itself (`d:self`).
    let far = ts("2036-01-01T00:00:00Z");
    assert_eq!(
        unfinished_at(&graph, &AttentionConfig::default(), far)?,
        [
            "d:cyc-a",
            "d:p-conc",
            "d:p-cont2",
            "d:p-edge-eq",
            "d:p-edge-over",
            "d:p-sup"
        ]
    );
    Ok(())
}

#[test]
fn a_supersession_that_states_no_time_is_never_old() -> Result<()> {
    // The edge states an offset and nothing else, and the stub that superseded it was named by a
    // request and never proposed, so its node states no time either: no age is invented.
    let graph = offset_only_supersession(None)?;

    let found = only_unfinished_on(
        &graph,
        &AttentionConfig::default(),
        ts("2036-01-01T00:00:00Z"),
    )?;

    assert!(found.is_empty());
    Ok(())
}

#[test]
fn a_supersession_still_inside_the_window_does_not_make_a_page_truncated() -> Result<()> {
    // `d:old` is old enough; `d:z-recent` sorts after it and is not, and has to be dated to know.
    let scenario = two_dependents()?;
    let crew = "agent:claude:crew";
    scenario.decision("d:z-recent", "Use one cache", crew, "2026-02-01T00:00:00Z")?;
    scenario.decision("d:z-new", "Use two caches", crew, "2026-09-20T00:00:00Z")?;
    scenario.decision("d:z-dep", "Warm the cache", crew, "2026-04-01T00:00:00Z")?;
    scenario.relation(
        "FOLLOWS_FROM",
        "d:z-dep",
        "d:z-recent",
        crew,
        None,
        "2026-04-01T00:00:01Z",
    )?;
    scenario.supersede("d:z-recent", "d:z-new", crew, "2026-09-20T00:00:01Z")?;
    let graph = scenario.graph()?;
    let config = AttentionConfig::default();
    let kinds = [FindingKind::PropagationUnfinished];

    let first = attention_findings_at(&graph, &request(&kinds, 1), &config, now())?;

    assert_eq!(
        decisions_flagged(&first.findings),
        BTreeSet::from(["d:old"])
    );
    // Nothing follows that is a finding: only a supersession six days old.
    assert!(!first.truncated);
    assert_eq!(first.next_cursor, None);
    // Later on, it is one: the same request is truncated after the first, and walks to both.
    let later = now() + Duration::days(30);
    let (sizes, found) = walk(&graph, &request(&kinds, 1), &config, later)?;
    assert_eq!(sizes, [1, 1]);
    assert_eq!(
        found
            .iter()
            .map(|finding| finding.decision_id.as_str())
            .collect::<Vec<_>>(),
        ["d:old", "d:z-recent"]
    );
    Ok(())
}

#[test]
fn a_reaffirmation_closes_a_dependents_finding_by_ledger_order_never_by_the_clock() -> Result<()> {
    let scenario = Scenario::new();
    let crew = "agent:claude:crew";
    scenario.decision("d:old", "Use the old queue", crew, "2026-02-01T00:00:00Z")?;
    scenario.decision("d:new", "Use the new queue", crew, "2026-08-01T00:00:00Z")?;
    for id in ["d:early", "d:late"] {
        scenario.decision(id, "Batch jobs", crew, "2026-04-01T00:00:00Z")?;
        scenario.relation(
            "FOLLOWS_FROM",
            id,
            "d:old",
            crew,
            None,
            "2026-04-01T00:00:01Z",
        )?;
    }
    // Said before the premise changed: it covers nothing that came after.
    scenario.reaffirmed(
        "d:early",
        "d:old",
        "Fine as it is",
        "human:alice",
        "2026-05-01T00:00:00Z",
    )?;
    scenario.supersede("d:old", "d:new", crew, "2026-08-01T00:00:01Z")?;
    // Said after, in the ledger, with a timestamp from before the change: the ledger decides.
    scenario.reaffirmed(
        "d:late",
        "d:old",
        "Checked against the new queue",
        "human:alice",
        "2026-01-01T00:00:00Z",
    )?;

    let findings = findings_at(
        &scenario.graph()?,
        &[FindingKind::PremiseSuperseded],
        &AttentionConfig::default(),
        now(),
    )?;

    assert_eq!(
        findings
            .iter()
            .map(|finding| finding.decision_id.as_str())
            .collect::<Vec<_>>(),
        ["d:early"]
    );
    Ok(())
}

#[test]
fn an_unfinished_propagation_carries_no_score_or_rank() -> Result<()> {
    let graph = attention_scenario()?.graph()?;
    let wire = serde_json::to_value(
        findings_at(
            &graph,
            &[FindingKind::PropagationUnfinished],
            &AttentionConfig::default(),
            now(),
        )?
        .first()
        .expect("the scenario has one"),
    )
    .expect("a finding serializes");

    assert_eq!(wire["kind"], "propagation_unfinished");
    for word in ["score", "tier", "rank", "grade", "leverage", "deduction"] {
        assert!(wire.get(word).is_none(), "{word}");
    }
    Ok(())
}

// ── a change is dated by the link, not by the nodes it links (hivemind-xhoie) ──────────────────

/// When the link below is recorded: long after both decisions and the evidence it joins.
const LINKED: &str = "2026-09-20T10:00:00Z";

/// `d:premise` (February) is superseded on `LINKED` by `d:successor`, a decision captured back in
/// May: an existing decision linked as the superseder. `d:dependent` follows from the premise and
/// nobody has looked at it since.
fn old_decision_linked_as_superseder() -> Result<Scenario> {
    let scenario = Scenario::new();
    let crew = "agent:claude:crew";
    scenario.decision(
        "d:premise",
        "Use the old queue",
        crew,
        "2026-02-01T00:00:00Z",
    )?;
    scenario.decision(
        "d:successor",
        "Use the new queue",
        crew,
        "2026-05-01T00:00:00Z",
    )?;
    scenario.decision("d:dependent", "Batch jobs", crew, "2026-03-01T00:00:00Z")?;
    scenario.relation(
        "FOLLOWS_FROM",
        "d:dependent",
        "d:premise",
        crew,
        None,
        "2026-03-01T00:00:01Z",
    )?;
    scenario.supersede("d:premise", "d:successor", crew, LINKED)?;
    Ok(scenario)
}

/// `h:assumption` (February) is refuted on `LINKED` by `e:old`, evidence recorded back in May:
/// existing evidence linked as refuting. `d:rests-on` rests on the assumption and nobody has
/// looked at it since.
fn old_evidence_linked_as_refuting() -> Result<Scenario> {
    let scenario = Scenario::new();
    let crew = "agent:claude:crew";
    scenario.hypothesis(
        "h:assumption",
        "The queue keeps up",
        "assumption",
        None,
        "2026-02-01T00:00:00Z",
    )?;
    scenario.evidence(
        "e:old",
        "A benchmark from May",
        None,
        "2026-05-01T00:00:00Z",
    )?;
    scenario.decision_with(
        "d:rests-on",
        "Keep the queue",
        crew,
        "2026-03-01T00:00:00Z",
        false,
        &[],
        &["h:assumption"],
        None,
    )?;
    scenario.relation("REFUTES", "e:old", "h:assumption", crew, None, LINKED)?;
    Ok(scenario)
}

#[test]
fn linking_an_old_decision_as_superseder_dates_the_change_at_the_link() -> Result<()> {
    let graph = old_decision_linked_as_superseder()?.graph()?;
    let linked = ts(LINKED);
    let config = AttentionConfig::default();
    let window = Duration::days(i64::from(config.propagation_window_days));

    // The change is as old as the link, not as old as the decision it names as the superseder:
    // the same day, and for the whole window, nothing is unfinished and `verify` says so.
    let the_day_after = linked + Duration::days(1);
    assert!(only_unfinished_on(&graph, &config, the_day_after)?.is_empty());
    let propagation = get_propagation_at(&graph, "d:premise", None, the_day_after)?
        .expect("the premise has an open dependent");
    assert!(!propagation.unfinished);
    assert!(!propagation.describe().contains("[UNFINISHED]"));
    assert_eq!(propagation.oldest_open_since, Some(linked));
    assert_eq!(propagation.change.at_time, Some(linked));

    // Exactly the window after the link is not yet past; one second more is.
    assert!(only_unfinished_on(&graph, &config, linked + window)?.is_empty());
    let after = linked + window + Duration::seconds(1);
    let found = only_unfinished_on(&graph, &config, after)?;
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].decision_id, "d:premise");
    assert_eq!(found[0].basis_at, Some(linked));
    assert!(found[0]
        .reason
        .contains("superseded by 'Use the new queue' on 2026-09-20"));
    let propagation = get_propagation_at(&graph, "d:premise", None, after)?
        .expect("the premise has an open dependent");
    assert!(propagation.unfinished);
    assert!(propagation
        .describe()
        .ends_with("since 2026-09-20 [UNFINISHED]"));

    // The decision that follows the premise reads the change at the link too.
    let flagged = findings_at(
        &graph,
        &[FindingKind::PremiseSuperseded],
        &config,
        the_day_after,
    )?;
    assert_eq!(flagged.len(), 1);
    assert_eq!(flagged[0].decision_id, "d:dependent");
    assert_eq!(flagged[0].basis_at, Some(linked));
    assert!(
        flagged[0].reason.contains("on 2026-09-20"),
        "{}",
        flagged[0].reason
    );
    Ok(())
}

#[test]
fn linking_old_evidence_as_refuting_dates_the_change_at_the_link() -> Result<()> {
    let graph = old_evidence_linked_as_refuting()?.graph()?;
    let linked = ts(LINKED);
    let config = AttentionConfig::default();
    let window = Duration::days(i64::from(config.propagation_window_days));

    let refuted = findings_at(
        &graph,
        &[FindingKind::AssumptionRefuted],
        &config,
        linked + Duration::days(1),
    )?;
    assert_eq!(refuted.len(), 1);
    assert_eq!(refuted[0].decision_id, "d:rests-on");
    assert_eq!(
        refuted[0].basis_at,
        Some(linked),
        "the evidence was recorded in May; the refutation was recorded when it was linked"
    );
    assert!(
        refuted[0].reason.contains("on 2026-09-20"),
        "{}",
        refuted[0].reason
    );

    let day_after = get_propagation_at(&graph, "h:assumption", None, linked + Duration::days(1))?
        .expect("the hypothesis has an open dependent");
    assert!(!day_after.unfinished);
    assert!(!day_after.describe().contains("[UNFINISHED]"));
    assert_eq!(day_after.change.at_time, Some(linked));
    assert_eq!(day_after.oldest_open_since, Some(linked));

    let at_window = get_propagation_at(&graph, "h:assumption", None, linked + window)?
        .expect("the hypothesis has an open dependent");
    assert!(!at_window.unfinished, "exactly the window is not yet past");
    let past = get_propagation_at(
        &graph,
        "h:assumption",
        None,
        linked + window + Duration::seconds(1),
    )?
    .expect("the hypothesis has an open dependent");
    assert!(past.unfinished);
    assert!(past.describe().ends_with("since 2026-09-20 [UNFINISHED]"));
    Ok(())
}

#[test]
fn the_earliest_link_dates_the_change_when_several_are_recorded() -> Result<()> {
    let scenario = old_decision_linked_as_superseder()?;
    let crew = "agent:claude:crew";
    // A second decision, captured before the first successor, is linked as a superseder a week
    // later: the first link still dates the change, whichever decision is older.
    scenario.decision(
        "d:second",
        "Use a third queue",
        crew,
        "2026-04-01T00:00:00Z",
    )?;
    scenario.supersede("d:premise", "d:second", crew, "2026-09-27T10:00:00Z")?;
    let graph = scenario.graph()?;

    let flagged = findings_at(
        &graph,
        &[FindingKind::PremiseSuperseded],
        &AttentionConfig::default(),
        ts("2026-09-28T00:00:00Z"),
    )?;

    assert_eq!(flagged.len(), 1);
    assert_eq!(flagged[0].basis_at, Some(ts(LINKED)));
    assert_eq!(
        get_propagation_at(&graph, "d:premise", None, ts("2026-09-28T00:00:00Z"))?
            .expect("an open dependent")
            .change
            .at_time,
        Some(ts(LINKED))
    );
    Ok(())
}

/// A graph projected before a change edge carried a time: `d:successor` superseded `d:premise`
/// and the edge states an offset and nothing else, and `d:dependent` follows from the premise.
/// The successor states `occurred_at` when it is given one, and no time otherwise.
fn offset_only_supersession(successor_occurred_at: Option<&str>) -> Result<MemoryGraph> {
    use crate::projector::{GraphProperties, GraphValue, NodeKind, RelationKind};
    let graph = MemoryGraph::default();
    let decision = |id: &str, occurred_at: Option<&str>| {
        let mut properties =
            GraphProperties::from([("title".to_owned(), GraphValue::String(id.to_owned()))]);
        if let Some(occurred_at) = occurred_at {
            properties.insert(
                "occurred_at".to_owned(),
                GraphValue::String(occurred_at.to_owned()),
            );
        }
        graph.upsert_node(NodeKind::Decision, id, &properties)
    };
    decision("d:premise", Some("2026-02-01T00:00:00Z"))?;
    decision("d:successor", successor_occurred_at)?;
    decision("d:dependent", Some("2026-03-01T00:00:00Z"))?;
    let offset =
        |origin: i64| GraphProperties::from([("event_origin".to_owned(), GraphValue::Int(origin))]);
    graph.upsert_edge(
        RelationKind::FollowsFrom,
        "d:dependent",
        "d:premise",
        &offset(1),
    )?;
    graph.upsert_edge(
        RelationKind::Supersedes,
        "d:successor",
        "d:premise",
        &offset(2),
    )?;
    Ok(graph)
}

/// A graph projected before a change edge carried a time: the edges state an offset and nothing
/// else. The nodes' own times date the change, as they did, for exactly these edges.
#[test]
fn an_edge_that_states_no_time_is_dated_by_the_node_it_links() -> Result<()> {
    let graph = offset_only_supersession(Some("2026-05-01T00:00:00Z"))?;

    let flagged = findings_at(
        &graph,
        &[FindingKind::PremiseSuperseded],
        &AttentionConfig::default(),
        ts("2026-10-01T00:00:00Z"),
    )?;
    assert_eq!(flagged.len(), 1);
    assert_eq!(flagged[0].basis_at, Some(ts("2026-05-01T00:00:00Z")));
    let unfinished = only_unfinished_on(
        &graph,
        &AttentionConfig::default(),
        ts("2026-10-01T00:00:00Z"),
    )?;
    assert_eq!(unfinished.len(), 1);
    assert_eq!(unfinished[0].basis_at, Some(ts("2026-05-01T00:00:00Z")));
    assert_eq!(
        get_propagation_at(&graph, "d:premise", None, ts("2026-10-01T00:00:00Z"))?
            .expect("an open dependent")
            .change
            .at_time,
        Some(ts("2026-05-01T00:00:00Z"))
    );
    Ok(())
}

fn only_unfinished_on(
    graph: &impl GraphView,
    config: &AttentionConfig,
    at: DateTime<Utc>,
) -> Result<Vec<AttentionFinding>> {
    findings_at(graph, &[FindingKind::PropagationUnfinished], config, at)
}
