// Parent module gates this file with #[cfg(test)]; repeat the marker so UBS can filter test-only assertions.
#[cfg(test)]
use serde_json::{json, Value};

use crate::projector::memory::MemoryGraph;
use crate::quality_profile::report::tests::assert_no_grade;
use crate::quality_profile::{score_decision, Dimension};
use crate::queries::test_fixtures::{record_payload, ts, Scenario};
use crate::queries::{
    get_decision_brief_at, get_decision_neighborhood, NeighborhoodRequest, QueryResponse,
};
use crate::Result;

use super::*;

/// `d:store` (a question, one described and one undescribed alternative, a stated rationale) and
/// `d:next`, which supersedes it.
fn store_scenario() -> Result<Scenario> {
    let scenario = Scenario::new();
    scenario.proposal(
        "human:alex",
        "2026-01-01T00:00:00Z",
        record_payload(
            "d:store",
            Some("Which store holds the ledger?"),
            &[
                ("o:sqlite", "SQLite", "Embedded, no server to run"),
                ("o:pg", "Postgres", "Shared across hosts, one more service"),
                (
                    "o:files",
                    "Flat files",
                    "Option generated from CLI value 'Flat files'",
                ),
            ],
            Some("o:sqlite"),
            &[],
        ),
    )?;
    scenario.accept("d:store", "human:alex", "2026-01-01T00:00:01Z")?;
    Ok(scenario)
}

fn without_quality(mut answer: Value) -> Value {
    answer
        .as_object_mut()
        .expect("the answer is an object")
        .remove("quality");
    answer
}

fn profile_json(graph: &MemoryGraph, decision_id: &str) -> Result<Value> {
    let profile = quality_profile_of(graph, decision_id)?.expect("the decision has a profile");
    Ok(serde_json::to_value(profile).expect("the profile serializes"))
}

#[test]
fn verify_carries_the_profile_beside_a_brief_that_is_otherwise_unchanged() -> Result<()> {
    let graph = store_scenario()?.graph()?;
    let brief = get_decision_brief_at(&graph, "d:store", ts("2026-06-01T00:00:00Z"))?;
    let plain = serde_json::to_value(&brief.data).expect("the brief serializes");

    let response = verify_with_quality(&graph, brief.clone())?;

    let answer = response.data.as_ref().expect("the decision exists");
    let json = serde_json::to_value(answer).expect("the answer serializes");
    assert_eq!(json["quality"], profile_json(&graph, "d:store")?);
    assert_eq!(
        without_quality(json),
        plain,
        "every field the brief had is still there, in place"
    );
    assert_eq!(response.result_count, brief.result_count);
    assert_eq!(response.truncated, brief.truncated);
    Ok(())
}

#[test]
fn verify_names_each_of_the_seven_dimensions_as_assessed_or_not_assessed() -> Result<()> {
    let graph = store_scenario()?.graph()?;
    let brief = get_decision_brief_at(&graph, "d:store", ts("2026-06-01T00:00:00Z"))?;

    let answer = verify_with_quality(&graph, brief)?
        .data
        .expect("the decision exists");
    let json = serde_json::to_value(&answer).expect("the answer serializes");

    let quality = &json["quality"];
    for dimension in Dimension::ALL {
        let entry = &quality[dimension.as_str()];
        match entry["status"].as_str() {
            Some("assessed") => {
                assert!(
                    ["none", "partial", "solid"].contains(&entry["level"].as_str().unwrap_or("")),
                    "{} has an ordinal level: {entry}",
                    dimension.as_str()
                );
                assert!(entry["reasons"].is_array(), "{entry}");
            }
            Some("not_assessed") => {
                assert!(entry.get("level").is_none(), "{entry}");
                assert!(entry["why"].as_str().is_some_and(|why| !why.is_empty()));
            }
            other => panic!("{} has status {other:?}: {entry}", dimension.as_str()),
        }
    }
    // The alternatives floor found the undescribed option and named it.
    let undescribed = quality["alternatives"]["reasons"]
        .as_array()
        .expect("reasons")
        .iter()
        .find(|reason| reason["kind"] == "alternatives_undescribed")
        .expect("the flat-files option has no description of its own");
    assert_eq!(undescribed["node_ids"], json!(["o:files"]));
    assert_no_grade(quality, "quality");
    Ok(())
}

#[test]
fn verify_of_a_decision_that_does_not_exist_is_unchanged_and_carries_no_profile() -> Result<()> {
    let graph = Scenario::new().graph()?;
    let brief = get_decision_brief_at(&graph, "d:nope", ts("2026-06-01T00:00:00Z"))?;
    assert!(brief.data.is_none());

    let response = verify_with_quality(&graph, brief.clone())?;

    assert!(response.data.is_none());
    assert_eq!(response.result_count, 0);
    assert_eq!(
        serde_json::to_value(&response.data).expect("serializes"),
        Value::Null
    );
    Ok(())
}

#[test]
fn why_carries_the_profile_on_the_root_and_leaves_nodes_edges_and_timeline_alone() -> Result<()> {
    let graph = store_scenario()?.graph()?;
    let view: QueryResponse<NeighborhoodView> =
        get_decision_neighborhood(&graph, "d:store", &NeighborhoodRequest::all())?;
    let plain = serde_json::to_value(&view.data).expect("the view serializes");

    let response = why_with_quality(&graph, view.clone())?;

    let json = serde_json::to_value(&response.data).expect("the answer serializes");
    assert_eq!(
        json["root"]["quality"],
        profile_json(&graph, "d:store")?,
        "the profile sits on the root, beside the brief"
    );
    let mut stripped = json.clone();
    stripped["root"]
        .as_object_mut()
        .expect("the root is an object")
        .remove("quality");
    assert_eq!(
        stripped, plain,
        "the root's brief, the nodes, the edges and the timeline are as they were"
    );
    assert_eq!(response.result_count, view.result_count);
    assert_eq!(response.truncated, view.truncated);
    Ok(())
}

#[test]
fn why_for_a_root_that_is_not_present_carries_no_profile() -> Result<()> {
    let graph = Scenario::new().graph()?;
    let view = get_decision_neighborhood(&graph, "d:nope", &NeighborhoodRequest::all())?;
    assert!(!view.data.root.present);

    let response = why_with_quality(&graph, view.clone())?;

    let json = serde_json::to_value(&response.data).expect("the answer serializes");
    assert!(json["root"].get("quality").is_none(), "{json}");
    assert_eq!(
        json,
        serde_json::to_value(&view.data).expect("the view serializes")
    );
    Ok(())
}

#[test]
fn the_profile_in_the_answer_is_the_one_score_decision_returns() -> Result<()> {
    let graph = store_scenario()?.graph()?;
    let brief = get_decision_brief_at(&graph, "d:store", ts("2026-06-01T00:00:00Z"))?;

    let answer = verify_with_quality(&graph, brief)?
        .data
        .expect("the decision exists");
    let scored = score_decision(&graph, "d:store")?
        .data
        .expect("the decision is scored");

    assert_eq!(answer.quality, scored.profile);
    Ok(())
}
