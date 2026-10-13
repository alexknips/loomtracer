// Parent module gates this file with #[cfg(test)]; repeat the marker so UBS can filter test-only assertions.
#[cfg(test)]
use super::*;
use crate::queries::{HypothesisStatus, NeighborEdge, NeighborNode, NeighborhoodRoot};

#[test]
fn status_filter_cycles_through_supported_values() {
    let mut app = DecisionSearchApp::new(TuiConfig {
        query: None,
        topic_keys: Vec::new(),
        statuses: Vec::new(),
        actor_ids: Vec::new(),
        sources: Vec::new(),
        limit: 25,
        dot_output: PathBuf::from("out.dot"),
    });

    let expected = [
        "proposed",
        "accepted",
        "rejected",
        "contested",
        "superseded",
        "any",
    ];
    for label in expected {
        app.cycle_status_filter();
        assert_eq!(app.status_label(), label);
    }
}

#[test]
fn search_request_normalizes_inputs() {
    let app = DecisionSearchApp::new(TuiConfig {
        query: Some(" queue ".to_owned()),
        topic_keys: vec!["infra".to_owned(), " storage ".to_owned()],
        statuses: vec![DecisionStatus::Accepted],
        actor_ids: vec!["actor:1".to_owned()],
        sources: vec!["agent".to_owned()],
        limit: 10,
        dot_output: PathBuf::from("out.dot"),
    });

    let request = app.search_request();
    assert_eq!(request.query.as_deref(), Some("queue"));
    assert_eq!(request.topic_keys, vec!["infra", "storage"]);
    assert_eq!(request.statuses, vec![DecisionStatus::Accepted]);
    assert_eq!(request.actor_ids, vec!["actor:1"]);
    assert_eq!(request.sources, vec!["agent"]);
    assert_eq!(request.limit, 10);
}

#[test]
fn dot_export_renders_node_statuses_and_relation_labels() {
    let neighborhood = NeighborhoodView {
        root: NeighborhoodRoot {
            id: "d1".to_owned(),
            kind: NodeKind::Decision,
            present: true,
            brief: None,
        },
        nodes: vec![
            NeighborNode {
                id: "d1".to_owned(),
                kind: NodeKind::Decision,
                project: Some("billing".to_owned()),
                project_label: Some("Billing".to_owned()),
                decision_status: Some(DecisionStatus::Accepted),
                hypothesis_status: None,
                label: None,
            },
            NeighborNode {
                id: "h1".to_owned(),
                kind: NodeKind::Hypothesis,
                project: None,
                project_label: None,
                decision_status: None,
                hypothesis_status: Some(HypothesisStatus::Refuted),
                label: None,
            },
        ],
        edges: vec![NeighborEdge {
            from: "d1".to_owned(),
            to: "h1".to_owned(),
            relation: RelationKind::PremisedOn,
            label: "rests on",
            reversed: false,
            event_origin: Some(7),
        }],
        timeline: None,
    };

    let dot = render_neighborhood_dot(&neighborhood);

    assert!(dot.contains("Decision:d1"));
    assert!(dot.contains("status: accepted"));
    assert!(dot.contains("status: refuted"));
    assert!(dot.contains("[label=\"rests on\"]"));
}

#[test]
fn results_and_the_open_decision_say_whether_a_premise_was_looked_at_again() {
    use crate::queries::test_fixtures::reexamine_scenario;

    let scenario = reexamine_scenario().expect("scenario");
    let graph = scenario.graph().expect("graph");
    let mut app = DecisionSearchApp::new(TuiConfig {
        query: None,
        topic_keys: vec!["grounding".to_owned()],
        statuses: Vec::new(),
        actor_ids: Vec::new(),
        sources: Vec::new(),
        limit: 200,
        dot_output: PathBuf::from("out.dot"),
    });
    app.refresh(&graph);

    let badge_of = |id: &str| -> Option<String> {
        app.results
            .iter()
            .find(|result| result.decision.id == id)
            .map(|result| {
                result
                    .graph_context
                    .reexamine
                    .as_ref()
                    .map_or_else(String::new, reexamine_badge)
            })
    };
    // A change nobody covered is flagged and counted; a contested premise is counted but is
    // not stale; one that was reaffirmed carries no badge (the open decision shows who).
    assert_eq!(badge_of("d:open").as_deref(), Some(" !stale re-examine:1"));
    assert_eq!(badge_of("d:on-cont").as_deref(), Some(" re-examine:1"));
    assert_eq!(badge_of("d:twice").as_deref(), Some(""));
    assert_eq!(badge_of("d:p-sup").as_deref(), Some(""));

    app.open_decision_id(&graph, "d:open", false);
    assert!(app.error_message.is_none(), "{:?}", app.error_message);
    assert_eq!(app.reexamine.len(), 1);
    assert!(app.reexamine[0]
        .describe()
        .starts_with("re-examine open: premise"));
    assert_eq!(app.reexamine[0].state.status(), Some(ReexamineStatus::Open));

    app.open_decision_id(&graph, "d:twice", false);
    assert_eq!(
        app.reexamine[0].state.status(),
        Some(ReexamineStatus::Reaffirmed)
    );
}
