//! Relevant-decision lookup: returns decisions matching a topic, actor, or status filter.

use std::time::Instant;

use serde::Serialize;

use crate::commands::normalize_topic_key;
use crate::error::CommandError;
use crate::projector::{GraphRow, GraphView, NodeKind};
use crate::Result;

use super::decision::DecisionView;
use super::project_label::ProjectLabels;
use super::reexamine::{ReexamineMarker, ReexamineMarkers};
use super::shared::{
    node_rows, optional_string, optional_string_list, recorded_floor_of, required_string,
    stored_topics_include, MAX_QUERY_RESULTS,
};
use super::status::{derive_decision_status, DecisionStatus};
use super::QueryResponse;

/// A decision a topic lookup returns: the shallow view, and whether what it rests on changed. The
/// lookup answers by status alone, so without the marker a decision whose premise was superseded
/// would read exactly like one that stands (hivemind-ottw.3).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RelevantDecision {
    #[serde(flatten)]
    pub decision: DecisionView,
    /// The stale marker and the count of open re-examine items. Absent when no premise changed
    /// under the decision. `verify` shows the detail: which premise, who looked again, when, why.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reexamine: Option<ReexamineMarker>,
}

pub fn get_relevant_decisions(
    graph: &impl GraphView,
    topic: &str,
    status_filter: Option<DecisionStatus>,
) -> Result<QueryResponse<Vec<RelevantDecision>>> {
    let started = Instant::now();
    // Both sides are compared in `normalize_topic_key` form: the caller's `Release Plan` meets a
    // stored `release-plan`, and a key recorded as `Release.Plan` meets either spelling. A topic
    // with no letter or digit left names no key at all. Cypher cannot normalize the stored side,
    // so the matching happens here over the decision rows, in id order. The topic is this
    // lookup's only filter, so one that names no key is the caller's error (REST 400, CLI usage
    // exit, MCP invalid params): dropping it, as the optional topic filters do, would answer with
    // every decision as if they all matched.
    let normalized_topic = normalize_topic_key(topic);
    if normalized_topic.is_empty() {
        return Err(CommandError::Validation(
            "topic must contain at least one letter or digit".to_owned(),
        )
        .into());
    }

    let mut decision_rows: Vec<GraphRow> = node_rows(graph, NodeKind::Decision)?
        .into_values()
        .filter(|row| {
            stored_topics_include(&optional_string_list(row, "topic_keys"), &normalized_topic)
        })
        .collect();
    let truncated = decision_rows.len() > MAX_QUERY_RESULTS;
    decision_rows.truncate(MAX_QUERY_RESULTS);

    let labels = ProjectLabels::from_graph(graph)?;
    let mut decisions = Vec::new();
    for row in decision_rows {
        let id = required_string(&row, "id")?;
        let status = derive_decision_status(graph, &id)?;
        if status_filter.is_some_and(|expected| expected != status) {
            continue;
        }

        let project = optional_string(&row, "project");
        decisions.push(RelevantDecision {
            reexamine: None,
            decision: DecisionView {
                id,
                title: optional_string(&row, "title").unwrap_or_default(),
                rationale: optional_string(&row, "rationale").unwrap_or_default(),
                topic_keys: optional_string_list(&row, "topic_keys"),
                status,
                project_label: labels.label_of(project.as_deref()),
                project,
                chosen_option_id: None,
                option_ids: Vec::new(),
                evidence_ids: Vec::new(),
                hypotheses: Vec::new(),
                premise_decision_ids: Vec::new(),
                // Shallow view like the other fields above this query doesn't fetch
                // (option_ids/evidence_ids/hypotheses/premise_decision_ids) — callers needing
                // quote/question or what the decision rests on follow up with get_decision.
                quote: None,
                question: None,
                question_id: None,
                // The floor answers ride in the same row, so a shallow view still says where the
                // decision stands against the floor: it is not a "not asked" by omission.
                floor: recorded_floor_of(&row),
            },
        });
    }

    // One fixed set of bulk reads for the whole answer, and none for an answer with no decisions.
    if !decisions.is_empty() {
        let markers = ReexamineMarkers::load(graph)?;
        for item in &mut decisions {
            item.reexamine = markers.of(&item.decision.id);
        }
    }

    Ok(QueryResponse {
        result_count: decisions.len(),
        truncated,
        latency_ms: started.elapsed().as_millis(),
        data: decisions,
    })
}
