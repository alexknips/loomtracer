//! `verify` and `why`: the answer to "what did we decide and why, and did it hold", with the
//! decision's quality profile in the same response.
//!
//! The two queries (`get_decision_brief`, `get_decision_neighborhood`) answer with the decision,
//! its rationale, who decided it and whether it still holds. How well the record supports the
//! decision is a separate question the profile answers, and a person asking "why" should not need
//! to know to ask it. [`verify_with_quality`] and [`why_with_quality`] put the two in one
//! response: the profile is added as `quality` and nothing else in the answer changes.
//!
//! `quality` is the [`QualityProfile`] exactly as `score_decision` returns it (seven dimensions,
//! each assessed with its reasons and node ids or not assessed and why; the attention lines; a
//! model's assessment beside the floors when one was asked). There is no composite, no tier and no
//! number that says how good the decision is. The provenance line stays on `score_decision`:
//! `decided_by` already says who decided and how far it was reviewed.
//!
//! # Placement
//! Layer 3, with the rest of the profile. The queries stay pure and never import this module
//! (`tests::queries_and_commands_never_import_the_profile`); the transports (the stdio and HTTP
//! MCP servers, the native HTTP routes and the CLI) call it after the query. The profile is read
//! from the same graph with a handful of anchored lookups: no scan, no model, no network, so
//! every backend gives the same profile.

use std::time::Instant;

use serde::Serialize;

use crate::error::QueryError;
use crate::projector::GraphView;
use crate::queries::{
    DecisionBrief, DecisionTimeline, NeighborEdge, NeighborNode, NeighborhoodRoot,
    NeighborhoodView, QueryResponse,
};
use crate::Result;

use super::{quality_profile_of, QualityProfile};

/// The `verify` answer: the decision's brief, flattened, and its profile beside it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct VerifyAnswer {
    #[serde(flatten)]
    pub brief: DecisionBrief,
    pub quality: QualityProfile,
}

/// The `why` answer: the decision's neighborhood, with the profile on the root beside the brief.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WhyAnswer {
    pub root: WhyRoot,
    pub nodes: Vec<NeighborNode>,
    pub edges: Vec<NeighborEdge>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeline: Option<DecisionTimeline>,
}

/// The root of a `why` answer. `quality` is absent exactly when the decision is not present,
/// the same case in which the root carries no brief.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WhyRoot {
    #[serde(flatten)]
    pub root: NeighborhoodRoot,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<QualityProfile>,
}

/// `response` with the profile of the decision it answers about. A decision that does not exist
/// (`data` is `None`) is returned unchanged: there is nothing to assess.
pub fn verify_with_quality(
    graph: &impl GraphView,
    response: QueryResponse<Option<DecisionBrief>>,
) -> Result<QueryResponse<Option<VerifyAnswer>>> {
    // ubs:ignore: Instant measures response latency only; it does not generate secrets.
    let started = Instant::now();
    let QueryResponse {
        result_count,
        truncated,
        latency_ms,
        data,
    } = response;
    let data = match data {
        None => None,
        Some(brief) => {
            let quality = profile_of_present(graph, &brief.decision_id)?;
            Some(VerifyAnswer { brief, quality })
        }
    };
    Ok(QueryResponse {
        result_count,
        truncated,
        latency_ms: latency_ms + started.elapsed().as_millis(),
        data,
    })
}

/// `response` with the profile of its root decision. A neighborhood whose root is not present
/// carries no profile.
pub fn why_with_quality(
    graph: &impl GraphView,
    response: QueryResponse<NeighborhoodView>,
) -> Result<QueryResponse<WhyAnswer>> {
    // ubs:ignore: Instant measures response latency only; it does not generate secrets.
    let started = Instant::now();
    let QueryResponse {
        result_count,
        truncated,
        latency_ms,
        data,
    } = response;
    let NeighborhoodView {
        root,
        nodes,
        edges,
        timeline,
    } = data;
    let quality = if root.present {
        Some(profile_of_present(graph, &root.id)?)
    } else {
        None
    };
    Ok(QueryResponse {
        result_count,
        truncated,
        latency_ms: latency_ms + started.elapsed().as_millis(),
        data: WhyAnswer {
            root: WhyRoot { root, quality },
            nodes,
            edges,
            timeline,
        },
    })
}

/// The profile of a decision the caller has just read, so its absence is a fault in the graph and
/// not an answer.
fn profile_of_present(graph: &impl GraphView, decision_id: &str) -> Result<QualityProfile> {
    quality_profile_of(graph, decision_id)?.ok_or_else(|| {
        QueryError::Execution(format!(
            "decision {decision_id} was read a moment ago but has no profile: it is not in the graph"
        ))
        .into()
    })
}

#[cfg(test)]
mod tests;
