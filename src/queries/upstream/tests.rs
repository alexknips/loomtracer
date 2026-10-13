// Parent module gates this file with #[cfg(test)]; repeat the marker so UBS can filter test-only assertions.
#[cfg(test)]
use crate::queries::test_fixtures::Scenario;
use crate::Result;

use super::*;

/// `d:goal` (accepted) with three decisions following from it: `d:leaf` (the one acted on),
/// `d:sibling` and `d:other`, plus `d:lone`, which follows from nothing.
fn goal_with_dependents() -> Result<Scenario> {
    let scenario = Scenario::new();
    scenario.decision(
        "d:goal",
        "Ship the hosted MVP",
        "human:alex",
        "2026-01-01T00:00:00Z",
    )?;
    scenario.accept("d:goal", "human:alex", "2026-01-01T00:00:01Z")?;
    for (id, title) in [
        ("d:leaf", "Use Postgres for the MVP"),
        ("d:sibling", "Run the MVP on one node"),
        ("d:other", "Price the MVP per seat"),
    ] {
        let proposal = scenario.decision(id, title, "agent:claude:crew", "2026-01-02T00:00:00Z")?;
        scenario.relation(
            "FOLLOWS_FROM",
            id,
            "d:goal",
            "agent:claude:crew",
            Some(proposal),
            "2026-01-02T00:00:01Z",
        )?;
    }
    scenario.decision(
        "d:lone",
        "Name the product",
        "human:alex",
        "2026-01-03T00:00:00Z",
    )?;
    Ok(scenario)
}

#[test]
fn a_decision_resting_on_a_standing_premise_lists_it_with_the_other_decisions_that_rest_on_it(
) -> Result<()> {
    let graph = goal_with_dependents()?.graph()?;

    let upstream = upstream_of(&graph, "d:leaf", None)?;

    assert_eq!(upstream.rests_on.len(), 1);
    let premise = &upstream.rests_on[0];
    assert_eq!(premise.item.kind, GroundingKind::Decision);
    assert_eq!(premise.item.id, "d:goal");
    assert_eq!(premise.item.state, GroundingItemState::Holds);
    // All three rest on it, the one acted on included; the other two are the "others".
    assert_eq!(premise.dependents_count, Some(3));
    assert_eq!(
        upstream.raise_upstream,
        vec![RaiseUpstream {
            premise_id: "d:goal".to_owned(),
            label: "Ship the hosted MVP".to_owned(),
            other_dependents: 2,
        }]
    );
    Ok(())
}

#[test]
fn a_decision_that_rests_on_nothing_has_nothing_to_raise() -> Result<()> {
    let graph = goal_with_dependents()?.graph()?;

    let upstream = upstream_of(&graph, "d:lone", None)?;

    assert_eq!(upstream, Upstream::default());
    // The goal itself rests on nothing either, though three decisions rest on it.
    assert_eq!(upstream_of(&graph, "d:goal", None)?, Upstream::default());
    Ok(())
}

#[test]
fn the_only_dependent_still_gets_the_hint_with_zero_others() -> Result<()> {
    let scenario = Scenario::new();
    scenario.decision("d:p", "Premise", "human:alex", "2026-01-01T00:00:00Z")?;
    let proposal =
        scenario.decision("d:only", "Only child", "human:alex", "2026-01-02T00:00:00Z")?;
    scenario.relation(
        "FOLLOWS_FROM",
        "d:only",
        "d:p",
        "human:alex",
        Some(proposal),
        "2026-01-02T00:00:01Z",
    )?;
    let graph = scenario.graph()?;

    let upstream = upstream_of(&graph, "d:only", None)?;

    assert_eq!(upstream.raise_upstream.len(), 1);
    assert_eq!(upstream.raise_upstream[0].other_dependents, 0);
    assert_eq!(upstream.rests_on[0].dependents_count, Some(1));
    Ok(())
}

#[test]
fn the_replacement_is_not_another_decision_relying_on_the_premise() -> Result<()> {
    // The replacement is written resting on the same premise, as a supersession usually is.
    let scenario = goal_with_dependents()?;
    let proposal = scenario.decision(
        "d:leaf-2",
        "Use SQLite for the MVP",
        "agent:claude:crew",
        "2026-02-01T00:00:00Z",
    )?;
    scenario.relation(
        "FOLLOWS_FROM",
        "d:leaf-2",
        "d:goal",
        "agent:claude:crew",
        Some(proposal),
        "2026-02-01T00:00:01Z",
    )?;
    scenario.supersede(
        "d:leaf",
        "d:leaf-2",
        "agent:claude:crew",
        "2026-02-01T00:00:02Z",
    )?;
    let graph = scenario.graph()?;

    let upstream = upstream_of(&graph, "d:leaf", Some("d:leaf-2"))?;

    assert_eq!(upstream.raise_upstream.len(), 1);
    assert_eq!(upstream.raise_upstream[0].other_dependents, 2);
    // `rests_on` is the plain count: four decisions rest on the goal now.
    assert_eq!(upstream.rests_on[0].dependents_count, Some(4));
    // Without naming the replacement it reads as a third other.
    assert_eq!(
        upstream_of(&graph, "d:leaf", None)?.raise_upstream[0].other_dependents,
        3
    );
    Ok(())
}

#[test]
fn a_premise_that_no_longer_stands_is_shown_but_not_offered_for_raising() -> Result<()> {
    let scenario = Scenario::new();
    for (id, title) in [
        ("d:superseded", "Old direction"),
        ("d:rejected", "Rejected direction"),
        ("d:contested", "Contested direction"),
    ] {
        scenario.decision(id, title, "human:alex", "2026-01-01T00:00:00Z")?;
    }
    scenario.accept("d:superseded", "human:alex", "2026-01-01T00:00:01Z")?;
    scenario.decision(
        "d:new",
        "New direction",
        "human:alex",
        "2026-02-01T00:00:00Z",
    )?;
    scenario.supersede(
        "d:superseded",
        "d:new",
        "human:alex",
        "2026-02-01T00:00:01Z",
    )?;
    scenario.reject("d:rejected", "human:alex", "2026-02-02T00:00:00Z")?;
    scenario.accept("d:contested", "human:alex", "2026-02-03T00:00:00Z")?;
    scenario.reject("d:contested", "human:bea", "2026-02-03T00:00:01Z")?;
    let proposal = scenario.decision("d:leaf", "Leaf", "human:alex", "2026-03-01T00:00:00Z")?;
    for premise in ["d:superseded", "d:rejected", "d:contested"] {
        scenario.relation(
            "FOLLOWS_FROM",
            "d:leaf",
            premise,
            "human:alex",
            Some(proposal),
            "2026-03-01T00:00:01Z",
        )?;
    }
    let graph = scenario.graph()?;

    let upstream = upstream_of(&graph, "d:leaf", None)?;

    let states: Vec<(&str, &GroundingItemState)> = upstream
        .rests_on
        .iter()
        .map(|premise| (premise.item.id.as_str(), &premise.item.state))
        .collect();
    assert_eq!(states.len(), 3, "{states:?}");
    assert!(matches!(
        states[0],
        ("d:contested", GroundingItemState::Contested)
    ));
    assert!(matches!(
        states[1],
        ("d:rejected", GroundingItemState::Rejected)
    ));
    assert!(matches!(
        states[2],
        ("d:superseded", GroundingItemState::Superseded { .. })
    ));
    // Each already has the disagreement on record: nothing left to raise.
    assert!(upstream.raise_upstream.is_empty());
    Ok(())
}

#[test]
fn only_a_standing_premise_is_offered_when_the_leaf_rests_on_both() -> Result<()> {
    let scenario = goal_with_dependents()?;
    scenario.decision(
        "d:old",
        "Old direction",
        "human:alex",
        "2026-01-01T00:00:00Z",
    )?;
    scenario.decision(
        "d:new",
        "New direction",
        "human:alex",
        "2026-02-01T00:00:00Z",
    )?;
    scenario.supersede("d:old", "d:new", "human:alex", "2026-02-01T00:00:01Z")?;
    scenario.relation(
        "FOLLOWS_FROM",
        "d:leaf",
        "d:old",
        "human:alex",
        None,
        "2026-03-01T00:00:00Z",
    )?;
    let graph = scenario.graph()?;

    let upstream = upstream_of(&graph, "d:leaf", None)?;

    assert_eq!(upstream.rests_on.len(), 2);
    assert_eq!(
        upstream
            .raise_upstream
            .iter()
            .map(|hint| hint.premise_id.as_str())
            .collect::<Vec<_>>(),
        ["d:goal"]
    );
    Ok(())
}

#[test]
fn evidence_assumptions_and_bets_are_shown_with_their_standing_and_no_dependents_count(
) -> Result<()> {
    let scenario = Scenario::new();
    scenario.evidence(
        "e:1",
        "Load test hit 4k rps",
        Some("ci://run/7"),
        "2026-01-01T00:00:00Z",
    )?;
    scenario.hypothesis(
        "h:assume",
        "Traffic stays under 5k rps",
        "assumption",
        None,
        "2026-01-01T00:00:00Z",
    )?;
    scenario.hypothesis(
        "h:bet",
        "Nobody needs multi-region this year",
        "bet",
        Some("2999-01-01T00:00:00Z"),
        "2026-01-01T00:00:00Z",
    )?;
    scenario.decision_with(
        "d:leaf",
        "Run on one node",
        "human:alex",
        "2026-01-02T00:00:00Z",
        false,
        &["e:1"],
        &["h:assume", "h:bet"],
        None,
    )?;
    let graph = scenario.graph()?;

    let upstream = upstream_of(&graph, "d:leaf", None)?;

    let kinds: Vec<GroundingKind> = upstream
        .rests_on
        .iter()
        .map(|premise| premise.item.kind)
        .collect();
    assert_eq!(
        kinds,
        [
            GroundingKind::Evidence,
            GroundingKind::Assumption,
            GroundingKind::Bet
        ]
    );
    assert!(upstream
        .rests_on
        .iter()
        .all(|premise| premise.dependents_count.is_none()));
    // The reply names no action for them: they are answered with evidence, not a disagreement.
    assert!(upstream.raise_upstream.is_empty());
    Ok(())
}

#[test]
fn the_reply_fields_serialize_flat_with_the_item_state_tag(
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let graph = goal_with_dependents()?.graph()?;

    let value = serde_json::to_value(upstream_of(&graph, "d:leaf", None)?)?;

    let premise = &value["rests_on"][0];
    assert_eq!(premise["kind"], "decision");
    assert_eq!(premise["id"], "d:goal");
    assert_eq!(premise["label"], "Ship the hosted MVP");
    assert_eq!(premise["state"], "holds");
    assert_eq!(premise["dependents_count"], 3);
    assert_eq!(premise["added"]["when"], "at_capture");
    assert_eq!(
        value["raise_upstream"][0],
        serde_json::json!({
            "premise_id": "d:goal",
            "label": "Ship the hosted MVP",
            "other_dependents": 2,
        })
    );
    // A decision that rests on nothing still carries both keys: empty means nothing to raise.
    let lone = serde_json::to_value(upstream_of(&graph, "d:lone", None)?)?;
    assert_eq!(
        lone,
        serde_json::json!({"rests_on": [], "raise_upstream": []})
    );
    Ok(())
}

#[test]
fn a_premise_cycle_terminates_with_one_hop_reads() -> Result<()> {
    let scenario = Scenario::new();
    scenario.decision("d:a", "A", "human:alex", "2026-01-01T00:00:00Z")?;
    scenario.decision("d:b", "B", "human:alex", "2026-01-01T00:00:01Z")?;
    for (from, to) in [("d:a", "d:b"), ("d:b", "d:a")] {
        scenario.relation(
            "FOLLOWS_FROM",
            from,
            to,
            "human:alex",
            None,
            "2026-01-02T00:00:00Z",
        )?;
    }
    let graph = scenario.graph()?;

    let upstream = upstream_of(&graph, "d:a", None)?;

    assert_eq!(upstream.raise_upstream.len(), 1);
    assert_eq!(upstream.raise_upstream[0].premise_id, "d:b");
    assert_eq!(upstream.raise_upstream[0].other_dependents, 0);
    Ok(())
}
