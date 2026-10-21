// Parent module gates this file with #[cfg(test)]; repeat the marker so UBS can filter test-only assertions.
#[cfg(test)]
use serde_json::json;

use crate::events::EventId;
use crate::queries::test_fixtures::Scenario;
use crate::Result;

use super::*;

fn at(second: u32) -> String {
    format!("2026-01-01T00:00:{second:02}Z")
}

/// A decision filed under `project`, with one option it chose, resting on `evidence` and
/// assuming `hypotheses`.
fn decision(
    scenario: &Scenario,
    decision_id: &str,
    project: &str,
    second: u32,
    evidence: &[&str],
    hypotheses: &[&str],
) -> Result<EventId> {
    let option_id = format!("{decision_id}:opt");
    scenario.proposal(
        "human:alex",
        &at(second),
        json!({
            "decision_id": decision_id,
            "title": format!("Decision {decision_id}"),
            "rationale": "Rationale long enough to read on its own",
            "topic_keys": ["grounding"],
            "option_ids": [option_id],
            "option_labels": ["Option A"],
            "chosen_option_id": option_id,
            "hypothesis_ids": hypotheses,
            "evidence_ids": evidence,
            "project": project,
            "project_source": "stated",
        }),
    )
}

fn page(
    scenario: &Scenario,
    address: &str,
    limit: usize,
    cursor: Option<&str>,
) -> Result<ProjectGraphPage> {
    project_graph_page(
        &scenario.graph()?,
        &ProjectGraphRequest {
            address: address.to_owned(),
            limit,
            cursor: cursor.map(|text| ProjectGraphCursor::parse(text).expect("a cursor we gave")),
        },
    )
}

fn ids_of(page: &ProjectGraphPage, kind: NodeKind) -> Vec<&str> {
    page.nodes
        .iter()
        .filter(|(node_kind, _)| *node_kind == kind)
        .map(|(_, id)| id.as_str())
        .collect()
}

fn arrows_of(page: &ProjectGraphPage, relation: RelationKind) -> Vec<(&str, &str)> {
    page.arrows
        .iter()
        .filter(|arrow| arrow.relation == relation)
        .map(|arrow| (arrow.from_id.as_str(), arrow.to_id.as_str()))
        .collect()
}

#[test]
fn a_page_holds_the_newest_decisions_and_the_cursor_walks_back_to_the_oldest() -> Result<()> {
    let scenario = Scenario::new();
    for number in 1..=5 {
        scenario.decision_in(
            &format!("d:{number}"),
            "A decision",
            "billing",
            "human:alex",
            &at(number),
        )?;
    }
    scenario.decision_in("d:other", "Elsewhere", "platform", "human:alex", &at(6))?;

    let newest = page(&scenario, "billing", 2, None)?;
    assert_eq!(newest.total_matches, 5);
    assert_eq!(ids_of(&newest, NodeKind::Decision), vec!["d:4", "d:5"]);
    let middle = page(&scenario, "billing", 2, newest.next_cursor.as_deref())?;
    assert_eq!(ids_of(&middle, NodeKind::Decision), vec!["d:2", "d:3"]);
    let oldest = page(&scenario, "billing", 2, middle.next_cursor.as_deref())?;
    assert_eq!(ids_of(&oldest, NodeKind::Decision), vec!["d:1"]);
    assert_eq!(oldest.next_cursor, None);
    assert_eq!(oldest.total_matches, 5);

    // A page that reaches the oldest decision says so; one that does not points at the rest.
    let all = page(&scenario, "billing", 5, None)?;
    assert_eq!(all.next_cursor, None);
    assert_eq!(ids_of(&all, NodeKind::Decision).len(), 5);
    Ok(())
}

#[test]
fn what_hangs_off_a_decision_comes_with_it_and_nothing_else_does() -> Result<()> {
    let scenario = Scenario::new();
    scenario.evidence("e:1", "Invoices lag by a day", None, &at(1))?;
    scenario.evidence("e:2", "A survey found the opposite", None, &at(2))?;
    scenario.hypothesis(
        "h:1",
        "Customers read invoices monthly",
        "assumption",
        None,
        &at(3),
    )?;
    decision(&scenario, "d:1", "billing", 4, &["e:1"], &["h:1"])?;
    scenario.relation("supports", "e:2", "h:1", "agent:tester", None, &at(5))?;
    // Another project's decision resting on the same evidence is not what hangs off d:1.
    decision(&scenario, "d:other", "platform", 6, &["e:1"], &[])?;

    let billing = page(&scenario, "billing", 10, None)?;
    assert_eq!(ids_of(&billing, NodeKind::Decision), vec!["d:1"]);
    assert_eq!(ids_of(&billing, NodeKind::Option), vec!["d:1:opt"]);
    assert_eq!(ids_of(&billing, NodeKind::Evidence), vec!["e:1", "e:2"]);
    assert_eq!(ids_of(&billing, NodeKind::Hypothesis), vec!["h:1"]);
    assert_eq!(ids_of(&billing, NodeKind::Actor), vec!["human:alex"]);
    assert_eq!(
        arrows_of(&billing, RelationKind::HasOption).len()
            + arrows_of(&billing, RelationKind::Chose).len()
            + arrows_of(&billing, RelationKind::BasedOn).len()
            + arrows_of(&billing, RelationKind::PremisedOn).len()
            + arrows_of(&billing, RelationKind::Supports).len(),
        5
    );

    // Every edge of the page has both its ends in it.
    for arrow in &billing.arrows {
        assert!(billing
            .nodes
            .contains(&(arrow.from_kind, arrow.from_id.clone())));
        assert!(billing
            .nodes
            .contains(&(arrow.to_kind, arrow.to_id.clone())));
    }
    Ok(())
}

#[test]
fn an_edge_asserted_twice_is_one_arrow() -> Result<()> {
    let scenario = Scenario::new();
    decision(&scenario, "d:1", "billing", 1, &[], &[])?;
    // What a capture also does: the same pair again as its own event.
    scenario.relation("has_option", "d:1", "d:1:opt", "human:alex", None, &at(2))?;
    scenario.relation("chose", "d:1", "d:1:opt", "human:alex", None, &at(3))?;

    let billing = page(&scenario, "billing", 10, None)?;
    assert_eq!(arrows_of(&billing, RelationKind::HasOption).len(), 1);
    assert_eq!(arrows_of(&billing, RelationKind::Chose).len(), 1);
    Ok(())
}

#[test]
fn the_far_end_of_an_edge_to_another_projects_decision_is_a_node_and_is_not_followed() -> Result<()>
{
    let scenario = Scenario::new();
    decision(&scenario, "d:old", "billing", 1, &[], &[])?;
    decision(&scenario, "d:new", "platform", 2, &[], &[])?;
    scenario.supersede("d:old", "d:new", "human:alex", &at(3))?;

    let billing = page(&scenario, "billing", 10, None)?;
    assert_eq!(ids_of(&billing, NodeKind::Decision), vec!["d:new", "d:old"]);
    assert_eq!(
        arrows_of(&billing, RelationKind::Supersedes).len(),
        1,
        "the supersession is drawn"
    );
    // The page is billing's: platform's decision is named, and its own option is not.
    assert_eq!(ids_of(&billing, NodeKind::Option), vec!["d:old:opt"]);
    assert_eq!(billing.total_matches, 1);
    Ok(())
}

#[test]
fn a_reaffirmation_comes_with_the_decision_it_concerns() -> Result<()> {
    let scenario = Scenario::new();
    scenario.hypothesis(
        "h:1",
        "Customers read invoices monthly",
        "assumption",
        None,
        &at(1),
    )?;
    decision(&scenario, "d:1", "billing", 2, &[], &["h:1"])?;
    decision(&scenario, "d:other", "platform", 3, &[], &[])?;
    scenario.reaffirmed("d:1", "h:1", "Still true", "human:alex", &at(4))?;

    let billing = page(&scenario, "billing", 10, None)?;
    assert_eq!(billing.derived.len(), 2);
    assert!(billing
        .derived
        .iter()
        .all(|arrow| arrow.from_kind == NodeKind::Reaffirmation));
    assert_eq!(ids_of(&billing, NodeKind::Reaffirmation).len(), 1);

    let platform = page(&scenario, "platform", 10, None)?;
    assert!(platform.derived.is_empty());
    assert!(ids_of(&platform, NodeKind::Reaffirmation).is_empty());
    Ok(())
}

#[test]
fn a_project_with_no_decision_has_an_empty_page() -> Result<()> {
    let scenario = Scenario::new();
    decision(&scenario, "d:1", "billing", 1, &[], &[])?;

    let empty = page(&scenario, "nobody", 10, None)?;
    assert_eq!(empty.total_matches, 0);
    assert!(empty.nodes.is_empty());
    assert!(empty.arrows.is_empty());
    assert_eq!(empty.next_cursor, None);
    Ok(())
}

#[test]
fn a_cursor_reads_back_and_a_garbled_one_does_not() {
    let cursor = ProjectGraphCursor::parse("42:capture:42:0").expect("a capture id holds colons");
    assert_eq!(cursor.encode(), "42:capture:42:0");
    for garbled in ["", "42", "42:", "x:d:1", ":d:1"] {
        assert_eq!(ProjectGraphCursor::parse(garbled), None, "{garbled:?}");
    }
}
