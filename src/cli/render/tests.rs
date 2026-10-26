// Parent module gates this file with #[cfg(test)]; repeat the marker so UBS can filter test-only assertions.
#[cfg(test)]
use crate::projector::memory::MemoryGraph;
use crate::quality_profile::{
    scan_decision_quality_at, score_decision, verify_with_quality, why_with_quality, ScanRequest,
};
use crate::queries::test_fixtures::{
    attention_scenario, floor_scenario, project_first_fixture, record_payload, reexamine_scenario,
    ts, Scenario, ATTENTION_NOW, REEXAMINE_NOW,
};
use crate::queries::{
    get_compact_view, get_decision_brief_at, get_decision_neighborhood,
    get_decisions_changed_since, get_recent_activity, get_situational_decisions, search_decisions,
    search_decisions_with_ledger, ChangedSinceRequest, NeighborhoodRequest, QueryContext,
    RecentActivityRequest, SearchDecisionRequest, SituationalRequest,
};
use crate::summarize::{DecisionSummary, RecallRanked, RecallResponse, SummarizeMode};
use crate::Result;

use super::*;

fn brief_text(graph: &MemoryGraph, decision_id: &str) -> Result<String> {
    let brief = get_decision_brief_at(graph, decision_id, ts("2026-06-01T00:00:00Z"))?;
    let answer = verify_with_quality(graph, brief)?.data;
    Ok(render_decision_brief_summary(&answer))
}

/// What follows `still holds: yes —` for a decision that has held `days` days.
fn held_clause(days: &str) -> String {
    format!(
        "held {days}; nothing in the graph has superseded, contested or rejected it, or undercut what it rests on (whether it was carried out is not tracked)"
    )
}

/// `d:goal` (accepted by Alex) and `d:derived`, which follows from it, cites evidence and
/// carries a bet, all named at capture, with the decider's stated confidence.
fn grounded_scenario() -> Result<Scenario> {
    let scenario = Scenario::new();
    scenario.decision(
        "d:goal",
        "Ship the hosted MVP",
        "human:alex",
        "2026-01-01T00:00:00Z",
    )?;
    scenario.accept("d:goal", "human:alex", "2026-01-01T00:00:01Z")?;
    scenario.evidence(
        "e:audit",
        "27 of 27 captures carry no premise",
        Some("mayor audit 2026-09-22"),
        "2026-01-01T00:00:02Z",
    )?;
    scenario.hypothesis(
        "h:bet",
        "Agents answer the question instead of skipping capture",
        "bet",
        Some("2026-10-15T00:00:00Z"),
        "2026-01-01T00:00:03Z",
    )?;
    let proposal = scenario.decision_with(
        "d:derived",
        "Premises are asked at capture",
        "agent:claude:crew",
        "2026-01-02T00:00:00Z",
        false,
        &["e:audit"],
        &["h:bet"],
        Some("high"),
    )?;
    scenario.accept("d:derived", "human:alex", "2026-01-02T00:00:01Z")?;
    scenario.relation(
        "FOLLOWS_FROM",
        "d:derived",
        "d:goal",
        "agent:claude:crew",
        Some(proposal),
        "2026-01-02T00:00:02Z",
    )?;
    Ok(scenario)
}

#[test]
fn brief_says_nothing_declared_for_a_decision_nobody_asked_about() -> Result<()> {
    let scenario = Scenario::new();
    scenario.decision(
        "d:logo",
        "Pick a logo",
        "human:alex",
        "2026-01-01T00:00:00Z",
    )?;

    let text = brief_text(&scenario.graph()?, "d:logo")?;

    assert!(
        text.contains(
            "  rests on: nothing declared (never asked; add with loomtracer ground \"Pick a logo\" --rests-on-decision \"...\")\n"
        ),
        "{text}"
    );
    // 2026-01-01 to the fixed clock of `brief_text`, 2026-06-01.
    assert!(
        text.contains(&format!(
            "  still holds: yes — {}\n",
            held_clause("151 days")
        )),
        "{text}"
    );
    assert!(!text.contains("confidence at capture"), "{text}");
    assert!(!text.contains("rest on this"), "{text}");
    Ok(())
}

#[test]
fn brief_lists_each_thing_it_rests_on_with_its_state_and_provenance() -> Result<()> {
    let graph = grounded_scenario()?.graph()?;

    let text = brief_text(&graph, "d:derived")?;

    for expected in [
        "  rests on:\n",
        "    decision   \"Ship the hosted MVP\"  still holds  (at capture)\n",
        "    evidence   \"27 of 27 captures carry no premise\" (mayor audit 2026-09-22)  (at capture)\n",
        "    bet        \"Agents answer the question instead of skipping capture\"  check by 2026-10-15, open  (at capture)\n",
        "  still holds: yes — held 150 days; nothing in the graph has superseded",
        "  confidence at capture: high (decider's words)\n",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in:\n{text}");
    }

    // The premise is told who rests on it.
    let goal = brief_text(&graph, "d:goal")?;
    assert!(goal.contains("  1 decision rests on this\n"), "{goal}");
    Ok(())
}

#[test]
fn brief_attributes_a_later_premise_to_whoever_added_it() -> Result<()> {
    let scenario = Scenario::new();
    scenario.decision(
        "d:goal",
        "Ship the hosted MVP",
        "human:alex",
        "2026-01-01T00:00:00Z",
    )?;
    scenario.decision(
        "d:old",
        "Use Postgres",
        "agent:claude:crew",
        "2026-01-02T00:00:00Z",
    )?;
    scenario.relation(
        "FOLLOWS_FROM",
        "d:old",
        "d:goal",
        "human:alex",
        None,
        "2026-03-05T10:00:00Z",
    )?;

    let text = brief_text(&scenario.graph()?, "d:old")?;

    assert!(
        text.contains(
            "    decision   \"Ship the hosted MVP\"  still holds  (attributed later by human:alex on 2026-03-05)\n"
        ),
        "{text}"
    );
    Ok(())
}

#[test]
fn brief_says_no_when_a_premise_was_superseded() -> Result<()> {
    let scenario = grounded_scenario()?;
    scenario.decision(
        "d:goal-2",
        "Ship the self-hosted MVP",
        "human:alex",
        "2026-02-01T00:00:00Z",
    )?;
    scenario.supersede("d:goal", "d:goal-2", "human:alex", "2026-02-01T00:00:01Z")?;

    let text = brief_text(&scenario.graph()?, "d:derived")?;

    for expected in [
        "    decision   \"Ship the hosted MVP\"  SUPERSEDED 2026-02-01 by \"Ship the self-hosted MVP\"  (at capture)\n",
        "  still holds: NO: premise superseded; re-examine open since 2026-02-01\n",
        "    - follows from \"Ship the hosted MVP\", which was superseded by \"Ship the self-hosted MVP\"\n",
        "    - re-examine open: premise \"Ship the hosted MVP\" (in alex's personal project) was superseded 2026-02-01; nobody has said this decision still holds\n",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in:\n{text}");
    }
    Ok(())
}

#[test]
fn brief_says_no_when_a_premise_was_rejected() -> Result<()> {
    let scenario = Scenario::new();
    scenario.decision(
        "d:goal",
        "Ship the hosted MVP",
        "human:alex",
        "2026-01-01T00:00:00Z",
    )?;
    let proposal = scenario.decision(
        "d:derived",
        "Use Postgres",
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
    scenario.reject("d:goal", "human:bob", "2026-01-03T00:00:00Z")?;

    let text = brief_text(&scenario.graph()?, "d:derived")?;

    // A rejection carries no time in the graph, so the open item says no date.
    assert!(
        text.contains("  still holds: NO: premise rejected; re-examine open\n"),
        "{text}"
    );
    assert!(
        text.contains("    - re-examine open: premise \"Ship the hosted MVP\" (in alex's personal project) was rejected; nobody has said this decision still holds\n"),
        "{text}"
    );
    assert!(
        text.contains("    decision   \"Ship the hosted MVP\"  REJECTED  (at capture)\n"),
        "{text}"
    );
    Ok(())
}

#[test]
fn brief_puts_a_lone_bet_on_one_line() -> Result<()> {
    let scenario = Scenario::new();
    scenario.hypothesis(
        "h:judgement",
        "Judgement call: Pick a logo",
        "bet",
        None,
        "2026-01-01T00:00:00Z",
    )?;
    scenario.decision_with(
        "d:logo",
        "Pick a logo",
        "agent:claude:crew",
        "2026-01-02T00:00:00Z",
        false,
        &[],
        &["h:judgement"],
        None,
    )?;
    scenario.accept("d:logo", "human:alex", "2026-01-02T00:00:01Z")?;

    let text = brief_text(&scenario.graph()?, "d:logo")?;

    assert!(
        text.contains(
            "  rests on: a bet, declared at capture by human:alex: \"Judgement call: Pick a logo\", no check date\n"
        ),
        "{text}"
    );
    Ok(())
}

#[test]
fn brief_reports_an_overdue_bet_as_unchecked_while_it_still_holds() -> Result<()> {
    let scenario = Scenario::new();
    scenario.hypothesis(
        "h:late",
        "Nobody will notice",
        "bet",
        Some("2026-02-01T00:00:00Z"),
        "2026-01-01T00:00:00Z",
    )?;
    scenario.decision_with(
        "d:1",
        "Ship it anyway",
        "human:alice",
        "2026-01-02T00:00:00Z",
        false,
        &[],
        &["h:late"],
        None,
    )?;

    let text = brief_text(&scenario.graph()?, "d:1")?;

    assert!(text.contains("  still holds: yes — held "), "{text}");
    assert!(
        text.contains(
            "  unchecked: bet \"Nobody will notice\" — check date 2026-02-01 has passed, no evidence recorded either way\n"
        ),
        "{text}"
    );
    Ok(())
}

#[test]
fn brief_says_a_rejected_decision_nobody_accepted_does_not_hold_and_who_rejected_it() -> Result<()>
{
    let scenario = Scenario::new();
    scenario.decision(
        "d:mail",
        "Checker report goes to the mayor by mail",
        "human:alice",
        "2026-01-01T00:00:00Z",
    )?;
    scenario.reject("d:mail", "human:carol", "2026-01-01T00:00:01Z")?;

    let text = brief_text(&scenario.graph()?, "d:mail")?;

    assert!(text.contains("[rejected]"), "{text}");
    assert!(text.contains("  still holds: NO: rejected\n"), "{text}");
    assert!(
        text.contains("    - rejected by human:carol and accepted by no one\n"),
        "{text}"
    );
    assert!(!text.contains("still holds: yes"), "{text}");
    Ok(())
}

#[test]
fn situational_summary_names_why_a_decision_is_stale() -> Result<()> {
    let scenario = grounded_scenario()?;
    scenario.decision(
        "d:goal-2",
        "Ship the self-hosted MVP",
        "human:alex",
        "2026-02-01T00:00:00Z",
    )?;
    scenario.supersede("d:goal", "d:goal-2", "human:alex", "2026-02-01T00:00:01Z")?;
    let graph = scenario.graph()?;

    let results = get_situational_decisions(
        &QueryContext::local(),
        &graph,
        scenario.ledger(),
        &SituationalRequest {
            paths: vec!["grounding".to_owned()],
            limit: 10,
            ..SituationalRequest::default()
        },
    )?
    .data;
    let text = render_situational_summary(&results);

    let derived_row = text
        .lines()
        .find(|line| line.contains("d:derived"))
        .expect("derived decision listed");
    assert!(
        derived_row.contains("STALE(premise superseded, re-examine open)"),
        "{derived_row}"
    );
    // A decision that still holds is unchanged.
    let holding_row = text
        .lines()
        .find(|line| line.contains("d:goal-2"))
        .expect("superseding decision listed");
    assert!(holding_row.contains("\tholds\t"), "{holding_row}");
    Ok(())
}

#[test]
fn situational_summary_of_a_project_scoped_answer_says_where_each_decision_came_from() -> Result<()>
{
    let fixture = project_first_fixture()?;
    let graph = MemoryGraph::default();
    crate::projector::rebuild_graph(&fixture.ledger, &graph)?;
    let ask = |project: Option<&str>, paths: &str| {
        get_situational_decisions(
            &QueryContext::local(),
            &graph,
            &fixture.ledger,
            &SituationalRequest {
                paths: vec![paths.to_owned()],
                limit: 50,
                project: project.map(str::to_owned),
                ..SituationalRequest::default()
            },
        )
    };

    let text = render_situational_summary(&ask(Some("billing"), "pricing")?.data);
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("terms\tpricing"));
    assert_eq!(
        lines.next(),
        Some("scope\tLooked in Billing (own project), Platform (Billing is part of it), Auth (Billing depends on it); not followed: 1 more level up the part_of chain, 2 more linked projects.")
    );
    let rows: Vec<&str> = lines.collect();
    assert_eq!(rows.len(), 4, "{text}");
    assert!(
        rows[0].contains("\tproject=Billing\tscope=own project\t"),
        "{}",
        rows[0]
    );
    assert!(
        rows[1].contains("\tproject=Platform\tscope=from Platform; Billing is part of it\t"),
        "{}",
        rows[1]
    );
    for row in &rows[2..] {
        assert!(
            row.contains("\tproject=Auth\tscope=from Auth; Billing depends on it\t"),
            "{row}"
        );
    }
    // The superseded Auth decision keeps its staleness label across the hop.
    assert!(
        rows.iter().any(|row| row.contains("STALE(superseded)")),
        "{text}"
    );

    // An empty scoped answer still says which projects it looked in.
    let empty = render_situational_summary(&ask(Some("billing"), "nothingmatches")?.data);
    assert!(
        empty.starts_with(
            "No decisions bear on this situation (terms: nothingmatches)\nscope\tLooked in Billing"
        ),
        "{empty}"
    );

    // Without a project the rows and header are exactly what they were.
    let unscoped = render_situational_summary(&ask(None, "pricing")?.data);
    assert!(!unscoped.contains("scope"), "{unscoped}");
    Ok(())
}

#[test]
fn recall_summary_of_a_project_scoped_answer_says_where_each_decision_came_from() -> Result<()> {
    let fixture = project_first_fixture()?;
    let graph = MemoryGraph::default();
    crate::projector::rebuild_graph(&fixture.ledger, &graph)?;
    // The recall response around the search it is built on; the digest is not what is tested.
    let ask = |project: Option<&str>, question: &str| -> Result<RecallResponse> {
        let search = search_decisions_with_ledger(
            &QueryContext::local(),
            &fixture.ledger,
            &graph,
            &SearchDecisionRequest {
                query: Some(question.to_owned()),
                limit: 10,
                project: project.map(str::to_owned),
                ..SearchDecisionRequest::default()
            },
        )?
        .data;
        Ok(RecallResponse {
            query: Some(question.to_owned()),
            ignored_words: Vec::new(),
            ranked: RecallRanked {
                total_matches: search.total_matches,
                truncated: false,
                next_cursor: None,
                items: search.items,
            },
            digest: DecisionSummary {
                summary: "digest".to_owned(),
                cited_decision_ids: Vec::new(),
                unit: SummarizeMode::Cluster,
            },
            scope: search.scope,
        })
    };

    let text = render_recall_summary(&ask(Some("billing"), "pricing")?);
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("scope\tLooked in Billing (own project), Platform (Billing is part of it), Auth (Billing depends on it); not followed: 1 more level up the part_of chain, 2 more linked projects.")
    );
    assert_eq!(lines.next(), Some("digest\tdigest"));
    assert_eq!(lines.next(), Some("cited\t"));
    let rows: Vec<&str> = lines.collect();
    assert_eq!(rows.len(), 4, "{text}");
    assert!(
        rows[0].ends_with("\tproject=Billing\tscope=own project"),
        "{}",
        rows[0]
    );
    assert!(
        rows[1].ends_with("\tproject=Platform\tscope=from Platform; Billing is part of it"),
        "{}",
        rows[1]
    );
    for row in &rows[2..] {
        assert!(
            row.ends_with("\tproject=Auth\tscope=from Auth; Billing depends on it"),
            "{row}"
        );
    }

    // An empty scoped answer still says which projects it looked in.
    let empty = render_recall_summary(&ask(Some("billing"), "nothingmatches")?);
    assert!(
        empty.starts_with("No decisions found matching the query.\nscope\tLooked in Billing"),
        "{empty}"
    );

    // Without a project the rows and header are exactly what they were.
    let unscoped = render_recall_summary(&ask(None, "pricing")?);
    assert!(!unscoped.contains("scope"), "{unscoped}");
    Ok(())
}

#[test]
fn compact_view_summary_says_what_it_rests_on_and_flags_a_premise_that_no_longer_stands(
) -> Result<()> {
    let scenario = grounded_scenario()?;
    scenario.decision(
        "d:goal-2",
        "Ship the self-hosted MVP",
        "human:alex",
        "2026-02-01T00:00:00Z",
    )?;
    scenario.supersede("d:goal", "d:goal-2", "human:alex", "2026-02-01T00:00:01Z")?;
    let graph = scenario.graph()?;

    let view = get_compact_view(&graph, "d:derived")?.data;
    let text = render_compact_view_summary(&view);

    assert!(
        text.contains("  rests on: decision x1, evidence x1, bet (check by 2026-10-15)\n"),
        "{text}"
    );
    assert!(
        text.contains("  STALE: follows from d:goal, which is superseded (re-examine open)\n"),
        "{text}"
    );
    Ok(())
}

#[test]
fn compact_view_summary_says_nothing_declared_for_a_decision_nobody_asked_about() -> Result<()> {
    let scenario = Scenario::new();
    scenario.decision(
        "d:logo",
        "Pick a logo",
        "human:alex",
        "2026-01-01T00:00:00Z",
    )?;

    let view = get_compact_view(&scenario.graph()?, "d:logo")?.data;
    let text = render_compact_view_summary(&view);

    assert!(
        text.contains("  rests on: nothing declared (never asked; add with loomtracer ground"),
        "{text}"
    );
    Ok(())
}

#[test]
fn history_change_kind_label_covers_both_kinds_of_stale_premise() {
    assert_eq!(
        change_kind_label(HistoryChangeKind::StalePremise),
        "stale_premise"
    );
}

#[test]
fn history_change_kind_label_names_a_project_move() {
    assert_eq!(
        change_kind_label(HistoryChangeKind::ProjectMoved),
        "project_moved"
    );
}

#[test]
fn history_lines_say_where_a_moved_decision_went_and_when_and_leave_other_lines_alone() -> Result<()>
{
    // Mayor's ruling on hivemind-s15q.10: the history entry for a move carries from, to, actor
    // and time; the finished "moved from A to B by X on <date>" sentence ships with the verb.
    let scenario = Scenario::new();
    scenario.decision(
        "d:pricing",
        "Per-seat pricing",
        "human:alice",
        "2026-01-01T00:00:00Z",
    )?;
    scenario.moved(
        "d:pricing",
        "billing",
        "pricing",
        "human:alex",
        "2026-01-02T03:04:05Z",
    )?;

    let activity = get_recent_activity(
        scenario.ledger(),
        &scenario.graph()?,
        &RecentActivityRequest::default(),
    )?
    .data;
    let activity_text = render_recent_activity_summary(&activity);
    let mut activity_lines = activity_text.lines();
    let moved_line = activity_lines.next().expect("the move is the newest row");
    assert!(
        moved_line.contains("\tproject_moved\tdecision.moved\tactor=human:alex\t"),
        "{moved_line}"
    );
    assert!(
        moved_line.ends_with("\tmoved=billing->pricing\tat=2026-01-02T03:04:05+00:00"),
        "{moved_line}"
    );
    let proposal_line = activity_lines.next().expect("the proposal row");
    assert!(
        !proposal_line.contains("moved="),
        "only a move line carries the move: {proposal_line}"
    );

    let changed = get_decisions_changed_since(
        scenario.ledger(),
        &scenario.graph()?,
        &ChangedSinceRequest {
            since_offset: Some(0),
            limit: 10,
            ..ChangedSinceRequest::default()
        },
    )?
    .data;
    let changed_text = render_changed_since_summary(&changed);
    assert!(
        changed_text
            .lines()
            .any(|line| line.contains("\tproject_moved\t")
                && line.ends_with("\tmoved=billing->pricing\tat=2026-01-02T03:04:05+00:00")),
        "{changed_text}"
    );
    Ok(())
}

#[test]
fn history_change_kind_label_names_a_title_change() {
    assert_eq!(
        change_kind_label(HistoryChangeKind::TitleChanged),
        "title_changed"
    );
}

#[test]
fn history_lines_say_what_a_retitled_decision_became_and_when_and_leave_other_lines_alone(
) -> Result<()> {
    // hivemind-ydmp: the history entry for a retitle carries from, to, actor and time; the
    // finished "retitled from A to B by X on <date>" sentence ships with the verb. Mirrors the
    // equivalent decision.moved test.
    let scenario = Scenario::new();
    scenario.decision(
        "d:pricing",
        "A decision with a very long paragraph title that predates the cap",
        "human:alice",
        "2026-01-01T00:00:00Z",
    )?;
    scenario.retitled(
        "d:pricing",
        "A decision with a very long paragraph title that predates the cap",
        "A short name",
        "human:alex",
        "2026-01-02T03:04:05Z",
    )?;

    let activity = get_recent_activity(
        scenario.ledger(),
        &scenario.graph()?,
        &RecentActivityRequest::default(),
    )?
    .data;
    let activity_text = render_recent_activity_summary(&activity);
    let mut activity_lines = activity_text.lines();
    let retitled_line = activity_lines
        .next()
        .expect("the retitle is the newest row");
    assert!(
        retitled_line.contains("\ttitle_changed\tdecision.retitled\tactor=human:alex\t"),
        "{retitled_line}"
    );
    assert!(
        retitled_line.ends_with(
            "\tretitled=A decision with a very long paragraph title that predates the cap->A \
short name\tat=2026-01-02T03:04:05+00:00"
        ),
        "{retitled_line}"
    );
    let proposal_line = activity_lines.next().expect("the proposal row");
    assert!(
        !proposal_line.contains("retitled="),
        "only a retitle line carries the retitle: {proposal_line}"
    );

    let changed = get_decisions_changed_since(
        scenario.ledger(),
        &scenario.graph()?,
        &ChangedSinceRequest {
            since_offset: Some(0),
            limit: 10,
            ..ChangedSinceRequest::default()
        },
    )?
    .data;
    let changed_text = render_changed_since_summary(&changed);
    assert!(
        changed_text.lines().any(|line| {
            line.contains("\ttitle_changed\t")
            && line.ends_with(
                "\tretitled=A decision with a very long paragraph title that predates the cap->A \
short name\tat=2026-01-02T03:04:05+00:00"
            )
        }),
        "{changed_text}"
    );
    Ok(())
}

// ── score_decision / scan_decision_quality summaries (hivemind-qo11.5) ───────────────────────

#[test]
fn a_score_summary_has_one_line_per_dimension_with_its_reasons_and_ids() -> Result<()> {
    let graph = attention_scenario()?.graph()?;

    let response = score_decision(&graph, "d:bet-overdue")?;
    let text = render_score_report_summary(&response.data);

    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "profile\td:bet-overdue\tfloor_version=2");
    let names: Vec<&str> = lines
        .iter()
        .filter(|line| line.starts_with("dimension\t"))
        .filter_map(|line| line.split('\t').nth(1))
        .collect();
    assert_eq!(
        names,
        [
            "framing",
            "alternatives",
            "information",
            "reasoning",
            "values_tradeoffs",
            "bias_exposure",
            "calibration",
        ]
    );
    let information = lines
        .iter()
        .find(|line| line.starts_with("dimension\tinformation\t"))
        .expect("an information line");
    assert!(information.contains("\tlevel=partial\t"), "{information}");
    assert!(information.contains("ids=h:bet-overdue"), "{information}");
    assert!(
        information.contains("\treasons=1 bet declared"),
        "{information}"
    );
    let values = lines
        .iter()
        .find(|line| line.starts_with("dimension\tvalues_tradeoffs\t"))
        .expect("a values line");
    assert!(
        values.contains("\tnot_assessed\twhy=Judged only"),
        "{values}"
    );
    assert!(!values.contains("level="), "{values}");
    assert_eq!(
        lines.last().copied(),
        Some("provenance\tauthorship=agent_only\treview=unreviewed\tnot yet reviewed by a human")
    );
    Ok(())
}

/// What a model said about a dimension is printed on the line right after that dimension's
/// floor, with the passage it quotes; a dimension it did not assess says why, with no level.
#[test]
fn a_score_summary_prints_what_a_model_said_right_after_each_floor_line() -> Result<()> {
    let graph = floor_scenario()?.graph()?;

    let text = render_score_report_summary(&score_decision(&graph, "d:solid")?.data);

    let lines: Vec<&str> = text.lines().collect();
    let header = lines
        .iter()
        .find(|line| line.starts_with("model_assessment\t"))
        .expect("a model assessment line");
    assert!(
        header
            .starts_with("model_assessment\tmodel-b\tprompt_version=assessment-v2\tevent_origin="),
        "{header}"
    );
    let after = |dimension: &str| -> (&str, &str) {
        let at = lines
            .iter()
            .position(|line| line.starts_with(&format!("dimension\t{dimension}\t")))
            .expect("a floor line for the dimension");
        (lines[at], lines[at + 1])
    };
    let (floor, model) = after("framing");
    assert!(floor.contains("\tlevel=partial\t"), "{floor}");
    assert_eq!(
        model,
        "model_dimension\tframing\tlevel=solid\texplanation=second: the question is stated\tquote=Which store should hold the ledger?"
    );
    let (_, model) = after("information");
    assert_eq!(
        model,
        "model_dimension\tinformation\tnot_assessed\twhy=second: nothing in the text to rest it on"
    );
    // A none answer that gave no quote prints no quote cell: nothing is invented.
    let (_, model) = after("bias_exposure");
    assert_eq!(
        model,
        "model_dimension\tbias_exposure\tlevel=none\texplanation=second: nothing in the record bears on a distortion"
    );

    let bare = render_score_report_summary(&score_decision(&graph, "d:bare")?.data);
    assert!(!bare.contains("model_assessment"), "{bare}");
    assert!(!bare.contains("model_dimension"), "{bare}");
    Ok(())
}

#[test]
fn a_score_summary_of_a_missing_decision_says_so() -> Result<()> {
    let graph = attention_scenario()?.graph()?;

    let response = score_decision(&graph, "d:nobody")?;

    assert_eq!(
        render_score_report_summary(&response.data),
        "No decision found"
    );
    Ok(())
}

#[test]
fn a_scan_summary_lists_each_finding_with_the_dimensions_it_bears_on() -> Result<()> {
    let graph = attention_scenario()?.graph()?;
    let request = ScanRequest {
        kinds: vec![crate::quality_profile::FindingKind::BetPastCheckDate],
        limit: 10,
        ..ScanRequest::default()
    };

    let response = scan_decision_quality_at(&graph, &request, ts(ATTENTION_NOW))?;
    let text = render_scan_report_summary(&response.data);

    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines[0],
        "attention\tas_of=2026-09-26T00:00:00Z\tevidence_window_days=90"
    );
    let findings: Vec<&&str> = lines
        .iter()
        .filter(|l| l.starts_with("finding\t"))
        .collect();
    assert_eq!(findings.len(), 2);
    assert!(
        findings[0].contains("\tbet_past_check_date\td:bet-overdue\t"),
        "{}",
        findings[0]
    );
    assert!(
        findings[0].contains("\tbasis_at=2026-09-01T00:00:00Z\t"),
        "{}",
        findings[0]
    );
    assert!(
        findings[0].contains("\tids=d:bet-overdue,h:bet-overdue\t"),
        "{}",
        findings[0]
    );
    // Each finding is followed by its dimensions: information, then calibration.
    assert!(
        lines[2].starts_with("dimension\tinformation\t"),
        "{}",
        lines[2]
    );
    assert!(
        lines[3].starts_with("dimension\tcalibration\t"),
        "{}",
        lines[3]
    );
    assert!(lines[4].starts_with("finding\t"), "{}", lines[4]);
    Ok(())
}

#[test]
fn a_scan_summary_with_no_findings_says_so() {
    let graph = MemoryGraph::default();

    let response = scan_decision_quality_at(&graph, &ScanRequest::default(), ts(ATTENTION_NOW))
        .expect("an empty graph scans");

    assert_eq!(
        render_scan_report_summary(&response.data),
        "No decision needs a look"
    );
}

#[test]
fn brief_shows_the_recorded_option_text_beside_the_words_it_reads_as() -> Result<()> {
    // hivemind-hk5z: `chose: Upheld (recorded as: name-a-upheld)`, so the reading of a
    // slug-shaped label never hides what the record says.
    let scenario = Scenario::new();
    scenario.proposal(
        "human:alex",
        "2026-01-01T00:00:00Z",
        serde_json::json!({
            "decision_id": "d:name",
            "title": "Name the product",
            "rationale": "The name has to hold up",
            "topic_keys": ["brand"],
            "option_ids": ["opt:1", "opt:2", "opt:3"],
            "option_labels": ["name-a-upheld", "name-b-standing", "name-c-decisis"],
            "chosen_option_id": "opt:1",
            "hypothesis_ids": [],
            "evidence_ids": []
        }),
    )?;

    let text = brief_text(&scenario.graph()?, "d:name")?;

    assert!(
        text.contains("  chose: Upheld (recorded as: name-a-upheld)\n"),
        "{text}"
    );
    assert!(
        text.contains(
            "  rejected: Standing (recorded as: name-b-standing), Decisis (recorded as: name-c-decisis) (neither has a reason of its own; both share the rationale above)\n"
        ),
        "{text}"
    );
    Ok(())
}

/// `d:pick`: a question, the chosen `options[0]` and the rest rejected. `options` are `(id,
/// label, description)`; an empty description is an alternative with no reason of its own, which
/// a capture surface records as the placeholder text it generates (the projector refuses an empty
/// one).
fn rejected_scenario(options: &[(&str, &str, &str)]) -> Result<Scenario> {
    let scenario = Scenario::new();
    let placeholders: Vec<String> = options
        .iter()
        .map(|(_, label, _)| crate::option_filler::cli(label))
        .collect();
    let options: Vec<(&str, &str, &str)> = options
        .iter()
        .zip(&placeholders)
        .map(|((id, label, description), placeholder)| {
            let description = if description.is_empty() {
                placeholder.as_str()
            } else {
                description
            };
            (*id, *label, description)
        })
        .collect();
    let options = options.as_slice();
    scenario.proposal(
        "human:alex",
        "2026-01-01T00:00:00Z",
        record_payload(
            "d:pick",
            Some("Which store holds the ledger?"),
            options,
            Some(options[0].0),
            &[],
        ),
    )?;
    Ok(scenario)
}

fn rejected_line(text: &str) -> &str {
    text.lines()
        .find(|line| line.starts_with("  rejected: "))
        .unwrap_or_else(|| panic!("no rejected line in:\n{text}"))
}

#[test]
fn brief_prints_one_quality_line_per_dimension_before_the_ref() -> Result<()> {
    let scenario = rejected_scenario(&[
        ("o:sqlite", "SQLite", "Embedded, no server to run"),
        ("o:pg", "Postgres", "Shared across hosts, one more service"),
    ])?;

    let text = brief_text(&scenario.graph()?, "d:pick")?;

    assert!(
        text.contains(&format!(
            "  quality (what the record states, not whether it is sound; floor v{}):\n",
            crate::quality_profile::FLOOR_VERSION
        )),
        "{text}"
    );
    for dimension in [
        "framing",
        "alternatives",
        "information",
        "reasoning",
        "values_tradeoffs",
        "bias_exposure",
        "calibration",
    ] {
        let prefix = format!("    {dimension}: ");
        assert_eq!(
            text.lines()
                .filter(|line| line.starts_with(&prefix))
                .count(),
            1,
            "one line for {dimension} in:\n{text}"
        );
    }
    // An assessed dimension prints its level and its reasons; a dimension with no basis says so.
    assert!(
        text.contains("    alternatives: solid — "),
        "an assessed dimension prints level and reasons:\n{text}"
    );
    assert!(
        text.contains("    values_tradeoffs: not assessed — "),
        "an unassessed dimension prints why:\n{text}"
    );
    // The profile has no number or grade to print.
    assert!(!text.contains("score"), "{text}");
    assert!(!text.contains("tier"), "{text}");
    let quality_at = text.find("  quality (").expect("quality block");
    let ref_at = text.find("  ref: d:pick").expect("ref line");
    assert!(quality_at < ref_at, "the ref line stays last:\n{text}");
    assert!(text.ends_with("  ref: d:pick"), "{text}");
    Ok(())
}

#[test]
fn rejected_line_has_no_shared_rationale_note_when_every_option_has_its_own_reason() -> Result<()> {
    let scenario = rejected_scenario(&[
        ("o:sqlite", "SQLite", "Embedded, no server to run"),
        ("o:pg", "Postgres", "Shared across hosts, one more service"),
        ("o:files", "Flat files", "No concurrent writers"),
    ])?;

    let text = brief_text(&scenario.graph()?, "d:pick")?;

    assert_eq!(
        rejected_line(&text),
        "  rejected: Flat files, Postgres",
        "{text}"
    );
    Ok(())
}

#[test]
fn rejected_line_names_only_the_options_that_share_the_rationale() -> Result<()> {
    let scenario = rejected_scenario(&[
        ("o:sqlite", "SQLite", "Embedded, no server to run"),
        ("o:pg", "Postgres", "Shared across hosts, one more service"),
        ("o:files", "Flat files", ""),
    ])?;

    let text = brief_text(&scenario.graph()?, "d:pick")?;

    assert_eq!(
        rejected_line(&text),
        "  rejected: Flat files, Postgres (no reason of its own for Flat files; it shares the rationale above)",
        "{text}"
    );
    Ok(())
}

#[test]
fn rejected_line_says_so_when_none_of_the_options_has_a_reason_of_its_own() -> Result<()> {
    let one = rejected_scenario(&[("o:sqlite", "SQLite", ""), ("o:pg", "Postgres", "")])?;
    let two = rejected_scenario(&[
        ("o:sqlite", "SQLite", ""),
        ("o:pg", "Postgres", ""),
        ("o:files", "Flat files", ""),
    ])?;
    let three = rejected_scenario(&[
        ("o:sqlite", "SQLite", ""),
        ("o:pg", "Postgres", ""),
        ("o:files", "Flat files", ""),
        ("o:kuzu", "Kuzu", ""),
    ])?;

    let one = brief_text(&one.graph()?, "d:pick")?;
    let two = brief_text(&two.graph()?, "d:pick")?;
    let three = brief_text(&three.graph()?, "d:pick")?;

    assert_eq!(
        rejected_line(&one),
        "  rejected: Postgres (it has no reason of its own; it shares the rationale above)"
    );
    assert_eq!(
        rejected_line(&two),
        "  rejected: Flat files, Postgres (neither has a reason of its own; both share the rationale above)"
    );
    assert_eq!(
        rejected_line(&three),
        "  rejected: Flat files, Kuzu, Postgres (none has a reason of its own; all share the rationale above)"
    );
    Ok(())
}

#[test]
fn brief_counts_the_days_a_live_decision_has_held_and_says_that_is_not_proof_it_was_carried_out(
) -> Result<()> {
    let scenario = Scenario::new();
    scenario.decision(
        "d:yesterday",
        "Adopt the error shape",
        "human:alex",
        "2026-05-31T00:00:00Z",
    )?;

    let text = brief_text(&scenario.graph()?, "d:yesterday")?;

    assert!(
        text.contains(&format!("  still holds: yes — {}\n", held_clause("1 day"))),
        "one day is singular:\n{text}"
    );
    Ok(())
}

#[test]
fn brief_says_how_long_a_superseded_decision_held_before_it_was_replaced() -> Result<()> {
    let scenario = Scenario::new();
    scenario.decision(
        "d:old",
        "Use JSON-RPC error 32001",
        "human:alex",
        "2026-01-01T00:00:00Z",
    )?;
    scenario.decision(
        "d:new",
        "Use a typed miss",
        "human:alex",
        "2026-01-11T00:00:00Z",
    )?;
    scenario.supersede("d:old", "d:new", "human:alex", "2026-01-11T00:00:01Z")?;

    let text = brief_text(&scenario.graph()?, "d:old")?;

    // The count stops at the superseder, not at the clock of the question (2026-06-01).
    assert!(
        text.contains(
            "  held 10 days until it was superseded (whether it was carried out is not tracked)\n"
        ),
        "{text}"
    );
    assert!(!text.contains("still holds: yes"), "{text}");
    Ok(())
}

#[test]
fn why_summary_prints_the_same_quality_lines_as_verify() -> Result<()> {
    let scenario = rejected_scenario(&[
        ("o:sqlite", "SQLite", "Embedded, no server to run"),
        ("o:pg", "Postgres", ""),
    ])?;
    let graph = scenario.graph()?;
    let view = get_decision_neighborhood(&graph, "d:pick", &NeighborhoodRequest::all())?;

    let why = render_neighborhood_summary(&why_with_quality(&graph, view)?.data);

    let verify = brief_text(&graph, "d:pick")?;
    let block = |text: &str| -> Vec<String> {
        text.lines()
            .skip_while(|line| !line.starts_with("  quality ("))
            .take_while(|line| line.starts_with("  quality (") || line.starts_with("    "))
            .map(str::to_owned)
            .collect()
    };
    let verify_block = block(&verify);
    assert_eq!(
        verify_block.len(),
        8,
        "header plus seven dimensions:\n{verify}"
    );
    assert_eq!(block(&why), verify_block, "{why}");
    Ok(())
}

#[test]
fn why_summary_for_a_decision_that_is_not_there_prints_no_quality() -> Result<()> {
    let graph = Scenario::new().graph()?;
    let view = get_decision_neighborhood(&graph, "d:nope", &NeighborhoodRequest::all())?;

    let why = render_neighborhood_summary(&why_with_quality(&graph, view)?.data);

    assert!(!why.contains("quality"), "{why}");
    assert!(why.contains("present=false"), "{why}");
    Ok(())
}

#[test]
fn brief_lists_the_options_of_a_decision_nobody_has_decided_as_open_not_rejected() -> Result<()> {
    // hivemind-d6ar: nobody turned these options down, so the summary must not say they were.
    let scenario = Scenario::new();
    scenario.proposal(
        "human:alice",
        "2026-01-01T00:00:00Z",
        serde_json::json!({
            "decision_id": "d:cadence",
            "title": "Checker report cadence",
            "rationale": "Daily or weekly; nobody has decided yet",
            "topic_keys": ["reports"],
            "option_ids": ["opt:1", "opt:2"],
            "option_labels": ["Daily", "Weekly"],
            "chosen_option_id": null,
            "hypothesis_ids": [],
            "evidence_ids": []
        }),
    )?;

    let text = brief_text(&scenario.graph()?, "d:cadence")?;

    assert!(
        text.contains("  options: Daily, Weekly (open — no choice is recorded)\n"),
        "{text}"
    );
    assert!(
        !text.lines().any(|line| line.starts_with("  rejected:")),
        "{text}"
    );
    assert!(!text.contains("  chose:"), "{text}");
    Ok(())
}

#[test]
fn brief_sets_a_label_that_holds_list_punctuation_apart_in_quotes() -> Result<()> {
    // hivemind-2gbo: three options read as three, however the labels are punctuated.
    let scenario = Scenario::new();
    scenario.proposal(
        "human:alice",
        "2026-01-01T00:00:00Z",
        serde_json::json!({
            "decision_id": "d:timing",
            "title": "Rename timing",
            "rationale": "Nobody has decided yet",
            "topic_keys": ["brand"],
            "option_ids": ["opt:1", "opt:2", "opt:3"],
            "option_labels": [
                "Rename now",
                "Rename after the comparison, before the first listing",
                "Say \"later\"; then ask"
            ],
            "chosen_option_id": null,
            "hypothesis_ids": [],
            "evidence_ids": []
        }),
    )?;

    let text = brief_text(&scenario.graph()?, "d:timing")?;

    assert!(
        text.contains(
            "  options: Rename now, \"Rename after the comparison, before the first listing\", \
             \"Say \\\"later\\\"; then ask\" (open"
        ),
        "{text}"
    );
    Ok(())
}

#[test]
fn the_importance_summary_gives_each_decision_its_place_its_decider_and_its_basis() -> Result<()> {
    let scenario = Scenario::new();
    scenario.decision(
        "d:root",
        "The ledger is the source of truth",
        "agent:claude:crew",
        "2026-01-01T00:00:00Z",
    )?;
    scenario.decision(
        "d:leaf",
        "Status is derived",
        "agent:claude:crew",
        "2026-01-02T00:00:00Z",
    )?;
    scenario.relation(
        "FOLLOWS_FROM",
        "d:leaf",
        "d:root",
        "agent:claude:crew",
        None,
        "2026-01-03T00:00:00Z",
    )?;
    scenario.accept("d:root", "human:alex", "2026-01-04T00:00:00Z")?;
    let report = crate::importance::rank_decisions_by_importance(
        &scenario.graph()?,
        &crate::importance::ImportanceRequest::default(),
    )?
    .data;

    let text = render_importance_summary(&report);

    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines[0],
        "importance\tranked=1\tnot_assessed=1\tleft_out_superseded=0\tleft_out_rejected=0"
    );
    assert_eq!(
        lines[1],
        "ranked_decided_by\tperson=1\tagent=0\tagent_within_delegation=0\tmixed=0\tunknown=0\tnone_recorded=0"
    );
    assert_eq!(
        lines[2],
        "#1\td:root\tThe ledger is the source of truth\tstatus=accepted\tdecided_by=person(human:alex)\trecorded_by=agent:claude:crew\t1 decision follows from it directly"
    );
    assert!(
        lines[3].starts_with(
            "not_assessed\td:leaf\tStatus is derived\tstatus=proposed\tdecided_by=none_recorded\trecorded_by=agent:claude:crew\tnot assessed:"
        ),
        "{text}"
    );
    Ok(())
}

#[test]
fn the_importance_summary_of_an_empty_ledger_says_there_is_nothing_to_rank() -> Result<()> {
    let report = crate::importance::rank_decisions_by_importance(
        &Scenario::new().graph()?,
        &crate::importance::ImportanceRequest::default(),
    )?
    .data;

    assert_eq!(render_importance_summary(&report), "No decisions to rank");
    Ok(())
}

/// hivemind-6td5: a decision someone rejected and nobody accepted is not "not yet decided".
#[test]
fn brief_names_who_rejected_a_decision_nobody_accepted() -> Result<()> {
    let scenario = Scenario::new();
    scenario.decision(
        "d:logo",
        "Pick a logo",
        "agent:claude:crew",
        "2026-01-01T00:00:00Z",
    )?;
    scenario.reject("d:logo", "human:dana", "2026-01-01T00:00:01Z")?;

    let text = brief_text(&scenario.graph()?, "d:logo")?;

    assert!(
        text.contains("  rejected by: human:dana (review=Rejected) -- nobody has accepted it\n"),
        "{text}"
    );
    assert!(!text.contains("not yet decided"), "{text}");
    Ok(())
}

/// hivemind-6td5 + hivemind-d6ar + hivemind-2gbo: a decision with options, no choice and a
/// rejection names who turned the decision down and still lists the options as open, never as
/// rejected options, with a label that holds a comma set apart in quotes so it counts as one.
#[test]
fn brief_of_a_rejected_decision_with_options_and_no_choice_reads_right_under_all_rules(
) -> Result<()> {
    let scenario = Scenario::new();
    scenario.proposal(
        "agent:claude:crew",
        "2026-01-01T00:00:00Z",
        serde_json::json!({
            "decision_id": "d:cadence",
            "title": "Checker report cadence",
            "rationale": "Daily or weekly; nobody has decided yet",
            "topic_keys": ["reports"],
            "option_ids": ["opt:1", "opt:2"],
            "option_labels": ["Daily", "Weekly, on Fridays"],
            "chosen_option_id": null,
            "hypothesis_ids": [],
            "evidence_ids": []
        }),
    )?;
    scenario.reject("d:cadence", "human:dana", "2026-01-01T00:00:01Z")?;

    let text = brief_text(&scenario.graph()?, "d:cadence")?;

    assert!(
        text.contains("  rejected by: human:dana (review=Rejected) -- nobody has accepted it\n"),
        "{text}"
    );
    assert!(
        text.contains("  options: Daily, \"Weekly, on Fridays\" (open — no choice is recorded)\n"),
        "{text}"
    );
    assert!(!text.contains("  rejected: "), "{text}");
    assert!(!text.contains("not yet decided"), "{text}");
    assert!(!text.contains("  chose:"), "{text}");
    Ok(())
}

/// hivemind-6td5: a classifier draft says which conversation it came from and with whom, as the
/// received batch states it, and why nobody has decided it.
#[test]
fn brief_of_a_classifier_draft_says_where_it_was_drafted_and_why_nobody_decided() -> Result<()> {
    let scenario = Scenario::new();
    scenario.received_batch("batch:1", "sess-1", "2026-06-01T00:00:00Z")?;
    let draft = scenario.classified_decision(
        "agent:claude:classifier",
        "2026-06-01T00:01:00Z",
        "Use the queue",
        None,
    )?;

    let text = brief_text(&scenario.graph()?, &draft)?;

    assert!(
        text.contains(
            "  drafted in: session sess-1, started by agent:claude:hook, with agent:claude:hook\n"
        ),
        "{text}"
    );
    assert!(
        text.contains(
            "  undecided: a classifier draft is never self-accepted, and the transcript names no one who accepted it\n"
        ),
        "{text}"
    );
    Ok(())
}

/// A draft from a conversation the ledger never received says nothing about who was in it.
#[test]
fn brief_of_a_draft_from_an_unreceived_conversation_does_not_guess_who_was_there() -> Result<()> {
    let scenario = Scenario::new();
    let draft = scenario.classified_decision(
        "agent:claude:classifier",
        "2026-06-01T00:01:00Z",
        "Use the queue",
        None,
    )?;

    let text = brief_text(&scenario.graph()?, &draft)?;

    assert!(!text.contains("drafted in:"), "{text}");
    assert!(text.contains("  undecided: a classifier draft"), "{text}");
    Ok(())
}

/// A hand-proposed decision nobody has accepted is unreviewed, but it is no classifier draft.
#[test]
fn brief_of_a_hand_proposed_decision_makes_no_classifier_draft_claim() -> Result<()> {
    let scenario = Scenario::new();
    scenario.decision(
        "d:logo",
        "Pick a logo",
        "human:alex",
        "2026-01-01T00:00:00Z",
    )?;

    let text = brief_text(&scenario.graph()?, "d:logo")?;

    assert!(!text.contains("drafted in:"), "{text}");
    assert!(!text.contains("undecided:"), "{text}");
    Ok(())
}

// ── re-examine (hivemind-ottw.3) ─────────────────────────────────────────────

fn reexamine_text_at(graph: &MemoryGraph, decision_id: &str) -> Result<String> {
    let brief = get_decision_brief_at(graph, decision_id, ts(REEXAMINE_NOW))?;
    let answer = verify_with_quality(graph, brief)?.data;
    Ok(render_decision_brief_summary(&answer))
}

#[test]
fn brief_says_whether_anyone_looked_again_and_who() -> Result<()> {
    let scenario = reexamine_scenario()?;
    let graph = scenario.graph()?;

    // Nobody looked again: the answer is no, and says the item is open since the change.
    let open = reexamine_text_at(&graph, "d:open")?;
    for expected in [
        "  still holds: NO: premise superseded; re-examine open since 2026-09-02\n",
        "    - re-examine open: premise \"Cap requests per tenant\" was superseded 2026-09-02; nobody has said this decision still holds\n",
    ] {
        assert!(open.contains(expected), "missing {expected:?} in:\n{open}");
    }

    // Someone said it still holds: it reads as holding again, with who, when and why visible,
    // and the clause no longer claims nothing changed under it.
    let twice = reexamine_text_at(&graph, "d:twice")?;
    assert!(
        twice.contains("  still holds: yes — held 29 days; a premise it rests on changed and someone said it still holds (below);"),
        "{twice}"
    );
    assert!(
        twice.contains("    - re-examined: premise \"Cap requests per tenant\" (in Billing) was superseded 2026-09-02; re-examined 2026-09-04 by agent:codex:reviewer: \"Checked again against the per-user cap\" (1 more reaffirmation since)\n"),
        "{twice}"
    );

    // A contested premise opens an item but is not a verdict: still holding, item open.
    let contested = reexamine_text_at(&graph, "d:on-cont")?;
    assert!(
        contested.contains("  still holds: yes — held 29 days; a premise it rests on is contested (below), a live disagreement and not a verdict;"),
        "{contested}"
    );
    assert!(
        contested.contains("    - re-examine open: premise \"Use queue Y\" is contested (a live disagreement, not a verdict); nobody has said this decision still holds\n"),
        "{contested}"
    );

    // A dependent that went out itself has nothing left to re-examine.
    let out = reexamine_text_at(&graph, "d:after-sup")?;
    assert!(
        out.contains("    - re-examine closed: premise \"Cap requests per tenant\" was superseded 2026-09-02; this decision was itself superseded by d:after-sup-v2\n"),
        "{out}"
    );
    Ok(())
}

#[test]
fn brief_of_a_changed_decision_says_how_far_the_change_travelled_without_naming_anyone(
) -> Result<()> {
    let scenario = reexamine_scenario()?;
    let graph = scenario.graph()?;

    let superseded = reexamine_text_at(&graph, "d:p-sup")?;
    assert!(
        superseded.contains("\n  propagation: 3 of 5 dependents re-examined; 2 open since 2026-09-02 [UNFINISHED]\n"),
        "{superseded}"
    );
    // A rejection states no time, so the line has no date and is never unfinished by age.
    let rejected = reexamine_text_at(&graph, "d:p-rej")?;
    assert!(
        rejected.contains("\n  propagation: 1 of 2 dependents re-examined; 1 open\n"),
        "{rejected}"
    );
    // A decision that did not change has no propagation line, and nor does one that went out
    // with nothing resting on it.
    assert!(!reexamine_text_at(&graph, "d:open")?.contains("propagation:"));
    // The line names no actor, only counts.
    let line = superseded
        .lines()
        .find(|line| line.contains("propagation:"))
        .expect("a propagation line");
    assert!(
        !line.contains("human:") && !line.contains("agent:"),
        "{line}"
    );
    Ok(())
}

#[test]
fn situational_summary_says_whether_a_stale_decision_was_looked_at_again() -> Result<()> {
    let scenario = reexamine_scenario()?;
    let graph = scenario.graph()?;
    let results = get_situational_decisions(
        &QueryContext::local(),
        &graph,
        scenario.ledger(),
        &SituationalRequest {
            paths: vec!["grounding".to_owned()],
            limit: 200,
            ..SituationalRequest::default()
        },
    )?
    .data;
    let text = render_situational_summary(&results);
    let row = |id: &str| -> String {
        text.lines()
            .find(|line| line.contains(&format!("\t{id}\t")))
            .unwrap_or_else(|| panic!("no row for {id}: {text}"))
            .to_owned()
    };

    assert!(row("d:open").contains("\tSTALE(premise superseded, re-examine open)\t"));
    assert!(row("d:twice").contains("\tholds(re-examined)\t"));
    assert!(row("d:on-cont").contains("\tholds(re-examine open)\t"));
    // Nothing changed under it: the cell is what it was.
    assert!(row("d:on-fine").contains("\tholds\t"));
    Ok(())
}

#[test]
fn a_match_says_when_what_it_rests_on_changed() -> Result<()> {
    let scenario = reexamine_scenario()?;
    let graph = scenario.graph()?;
    let results = search_decisions(
        &graph,
        &SearchDecisionRequest {
            topic_keys: vec!["grounding".to_owned()],
            limit: 200,
            ..SearchDecisionRequest::default()
        },
    )?
    .data;
    let row_of = |text: &str, id: &str| -> String {
        text.lines()
            .find(|line| line.contains(&format!("\t{id}\t")))
            .unwrap_or_else(|| panic!("no row for {id}: {text}"))
            .to_owned()
    };

    // search and recall rows
    let search_text = render_search_summary(&results);
    assert!(row_of(&search_text, "d:open").ends_with("\tstale=yes\treexamine_open=1"));
    assert!(row_of(&search_text, "d:twice").ends_with("\treaffirmed=1"));
    let contested = row_of(&search_text, "d:on-cont");
    assert!(contested.ends_with("\treexamine_open=1") && !contested.contains("stale"));
    assert!(!row_of(&search_text, "d:p-sup").contains("reexamine"));

    let recall = RecallResponse {
        query: None,
        ignored_words: Vec::new(),
        ranked: RecallRanked {
            total_matches: results.total_matches,
            truncated: false,
            next_cursor: None,
            items: results.items,
        },
        digest: DecisionSummary {
            summary: "digest".to_owned(),
            cited_decision_ids: Vec::new(),
            unit: SummarizeMode::Cluster,
        },
        scope: None,
    };
    let recall_text = render_recall_summary(&recall);
    assert!(row_of(&recall_text, "d:open").ends_with("\tstale=yes\treexamine_open=1"));
    assert!(row_of(&recall_text, "d:twice").ends_with("\treaffirmed=1"));

    // the topic lookup rows
    let relevant = crate::queries::get_relevant_decisions(&graph, "grounding", None)?.data;
    let list_text = render_decision_list_summary(&relevant);
    assert!(row_of(&list_text, "d:open").ends_with("\tstale=yes\treexamine_open=1"));
    assert!(!row_of(&list_text, "d:p-sup").contains("reexamine"));
    Ok(())
}

#[test]
fn compact_view_summary_says_whether_a_premise_was_looked_at_again() -> Result<()> {
    let scenario = reexamine_scenario()?;
    let graph = scenario.graph()?;
    let text_of = |id: &str| -> Result<String> {
        Ok(render_compact_view_summary(
            &crate::queries::get_compact_view_at(&graph, id, ts(REEXAMINE_NOW))?.data,
        ))
    };

    assert!(text_of("d:open")?
        .contains("  STALE: follows from d:p-sup, which is superseded (re-examine open)\n"));
    assert!(text_of("d:twice")?
        .contains("  STALE: follows from d:p-sup, which is superseded (re-examined)\n"));
    assert!(text_of("d:on-cont")?.contains(
        "  CONTESTED PREMISE: follows from d:p-cont, which is contested (re-examine open)\n"
    ));
    assert!(text_of("d:on-assumption")?
        .contains("rests on h:assumption, which was refuted (re-examine open)\n"));
    assert!(
        text_of("d:p-rej")?.contains("\n  propagation: 1 of 2 dependents re-examined; 1 open\n")
    );
    Ok(())
}

#[test]
fn history_change_kind_label_names_a_reaffirmation() {
    assert_eq!(
        change_kind_label(HistoryChangeKind::Reaffirmed),
        "reaffirmed"
    );
}

#[test]
fn history_lines_say_which_premise_a_decision_still_holds_after_and_why() -> Result<()> {
    let scenario = reexamine_scenario()?;

    let activity = get_recent_activity(
        scenario.ledger(),
        &scenario.graph()?,
        &RecentActivityRequest {
            limit: 500,
            ..RecentActivityRequest::default()
        },
    )?
    .data;
    let text = render_recent_activity_summary(&activity);
    let line = text
        .lines()
        .find(|line| {
            line.contains("\treaffirmed\tdecision.reaffirmed\t")
                && line.contains("decisions=d:twice")
        })
        .unwrap_or_else(|| panic!("no reaffirmed line for d:twice in:\n{text}"));
    assert!(line.contains("\tpremise=d:p-sup\treason="), "{line}");
    assert!(line.contains("\tat=2026-"), "{line}");
    // Only a reaffirmation line carries the premise and reason.
    assert!(text
        .lines()
        .filter(|line| !line.contains("\treaffirmed\t"))
        .all(|line| !line.contains("\tpremise=")));

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
    assert!(render_changed_since_summary(&changed)
        .lines()
        .any(|line| line.contains("\treaffirmed\t") && line.contains("\tpremise=")));
    Ok(())
}

#[test]
fn the_decision_timeline_names_who_said_a_decision_still_holds() -> Result<()> {
    let scenario = reexamine_scenario()?;
    let graph = scenario.graph()?;
    let timeline = crate::queries::get_decision_timeline(&graph, scenario.ledger(), "d:twice")?
        .data
        .expect("d:twice is in the scenario");
    let mut text = String::new();
    write_timeline(&mut text, &timeline);

    let reaffirmed: Vec<&str> = text
        .lines()
        .filter(|line| line.contains("  reaffirmed  by "))
        .collect();
    assert_eq!(reaffirmed.len(), 2, "{text}");
    assert!(
        reaffirmed[0].contains("still holds after d:p-sup changed: "),
        "{text}"
    );
    Ok(())
}

#[test]
fn the_dot_export_breaks_a_label_with_one_backslash_and_still_escapes_the_title() -> Result<()> {
    let scenario = Scenario::new();
    scenario.decision(
        "d:quoted",
        r#"A "quoted" \ title"#,
        "human:alex",
        "2026-01-01T00:00:00Z",
    )?;
    scenario.accept("d:quoted", "human:alex", "2026-01-01T00:00:01Z")?;
    let dot = render_dot(&scenario.graph()?)?;

    // The quote and the backslash in the title are escaped; the break before the status line is
    // a single backslash and `n`, which Graphviz draws as a second line.
    assert!(
        dot.contains(r#"label="A \"quoted\" \\ title\nstatus: accepted""#),
        "{dot}"
    );
    assert!(!dot.contains(r"\\nstatus"), "{dot}");
    Ok(())
}

#[test]
fn the_dot_export_draws_a_reaffirmation_with_the_decision_it_is_about() -> Result<()> {
    let scenario = reexamine_scenario()?;
    let dot = render_dot(&scenario.graph()?)?;

    // The node reads as its reason, and is tied to its decision and premise by dashed lines,
    // because it has no stored edge of its own.
    assert!(
        dot.contains("Still holds\\nChecked again against the per-user cap"),
        "{dot}"
    );
    let lines: Vec<&str> = dot
        .lines()
        .filter(|line| line.contains("style=dashed"))
        .collect();
    assert!(!lines.is_empty(), "{dot}");
    assert!(lines
        .iter()
        .any(|line| line.contains("[label=\"reaffirms\", style=dashed]")
            && line.contains("-> \"Decision:d:twice\"")));
    assert!(lines.iter().any(|line| line
        .contains("[label=\"after the change to\", style=dashed]")
        && line.contains("-> \"Hypothesis:h:bet\"")));
    // A dashed line starts only at a reaffirmation.
    assert!(lines
        .iter()
        .all(|line| line.trim_start().starts_with("\"Reaffirmation:")));
    Ok(())
}

fn listed_candidate(
    id: &str,
    title: &str,
    status: DecisionStatus,
    superseded_by: &[&str],
) -> ResolvedCandidate {
    ResolvedCandidate {
        decision_id: id.to_owned(),
        title: title.to_owned(),
        rank: 1,
        event_origin: 1,
        status,
        superseded_by: superseded_by.iter().map(|by| (*by).to_owned()).collect(),
        matched_fields: vec!["title".to_owned()],
        missing_terms: Vec::new(),
        polarity_mismatch: false,
        also_recorded_as: Vec::new(),
        resolved_via: Vec::new(),
        consequences: Vec::new(),
    }
}

#[test]
fn a_listed_candidate_says_its_status_and_a_replaced_one_names_what_replaced_it() {
    let summary = render_resolve_outcome_summary(&ResolveOutcome::Ambiguous {
        candidates: vec![
            listed_candidate(
                "decision-old",
                "Paid shape is chosen after the testers",
                DecisionStatus::Superseded,
                &["decision-new"],
            ),
            listed_candidate(
                "decision-new",
                "Paid shape is chosen before the first testers",
                DecisionStatus::Accepted,
                &[],
            ),
            listed_candidate(
                "decision-draft",
                "Nothing is paywalled until the trial ends",
                DecisionStatus::Proposed,
                &[],
            ),
        ],
    });

    assert_eq!(
        summary,
        "ambiguous: 3 candidates match — resolve with --pick N, #N, or --id\n\
         #1\tdecision-old\tPaid shape is chosen after the testers\tstatus: superseded by decision-new\n\
         #2\tdecision-new\tPaid shape is chosen before the first testers\tstatus: accepted\n\
         #3\tdecision-draft\tNothing is paywalled until the trial ends\tstatus: proposed"
    );
}

#[test]
fn a_close_candidate_says_its_status_before_the_words_it_lacks() {
    let mut close = listed_candidate(
        "decision-old",
        "Paid shape is chosen after the testers",
        DecisionStatus::Superseded,
        &["decision-new"],
    );
    close.missing_terms = vec!["pricing".to_owned()];

    let summary = render_resolve_outcome_summary(&ResolveOutcome::Ambiguous {
        candidates: vec![close],
    });

    assert!(
        summary.ends_with(
            "#1\tdecision-old\tPaid shape is chosen after the testers\tstatus: superseded by decision-new\tmissing: pricing"
        ),
        "{summary}"
    );
}

#[test]
fn a_candidate_names_the_decisions_that_rest_on_it() {
    let mut root = listed_candidate(
        "decision-split",
        "Split the frontend into its own repository",
        DecisionStatus::Accepted,
        &[],
    );
    root.consequences = vec![crate::queries::RecordedCopy {
        decision_id: "decision-remove".to_owned(),
        title: "Remove web/ from the main repository".to_owned(),
    }];

    assert_eq!(
        render_resolution_notices(&root),
        "consequences: decision-remove \"Remove web/ from the main repository\" (rest on this decision)"
    );

    let summary = render_resolve_outcome_summary(&ResolveOutcome::Ambiguous {
        candidates: vec![root],
    });
    assert!(
        summary.ends_with("\tstatus: accepted\tconsequences: decision-remove"),
        "{summary}"
    );
}
