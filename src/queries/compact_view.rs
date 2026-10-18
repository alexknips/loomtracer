//! Compact decision summaries for agent consumption: bounded, provenance-stripped view.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::events::{BlockerPriority, HypothesisKind};
use crate::projector::{GraphView, RelationKind};
use crate::Result;

use super::active_blockers::{ActiveDecisionBlockersRequest, DecisionBlockerFilters};
use super::decision::{get_decision_with_labels, get_hypothesis_statement, DecisionView};
use super::grounding::GroundingState;
use super::neighborhood::{neighborhood_structure, NeighborhoodRequest};
use super::project_label::ProjectLabels;
use super::reexamine::{
    propagation_of_decision, reexamine_entries_for, Propagation, ReexamineStatus,
    DEFAULT_PROPAGATION_WINDOW_DAYS,
};
use super::shared::{query_error, MAX_QUERY_RESULTS};
use super::status::{DecisionStatus, HypothesisStatus};
use super::supersession::{
    walk_supersession, SupersessionFork, SupersessionLine, SupersessionSide,
};
use super::{get_active_decision_blockers, QueryResponse};

// ---------------------------------------------------------------------------
// Output types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CompactView {
    pub decision: DecisionView,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supersession_chain: Option<SupersessionSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contest: Option<ContestView>,
    pub hypotheses: Vec<HypothesisSummaryView>,
    pub evidence_ids: Vec<String>,
    /// The prior decisions this one follows from, with whether each still stands.
    pub premises: Vec<PremiseSummaryView>,
    /// Grounded, a declared bet, or nothing declared ("never asked").
    pub grounding_state: GroundingState,
    /// How many other decisions follow from this one.
    pub dependents_count: usize,
    /// When this decision itself changed (rejected or contested): how many of the decisions that
    /// rest on it have been looked at again, how many are still open (hivemind-ottw.3). Absent
    /// when it has not changed, or nothing rested on it when it did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub propagation: Option<Propagation>,
    pub active_blockers: Vec<BlockerSummary>,
    pub elided: ElidedSummary,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PremiseSummaryView {
    pub decision_id: String,
    pub status: DecisionStatus,
    /// Whether anyone has said this decision still holds after the premise changed. Absent while
    /// the premise stands.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reexamine: Option<ReexamineStatus>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SupersessionSummary {
    pub chain_length: usize,
    pub oldest_id: String,
    /// Where the line stops because more than one decision comes next: a decision that two
    /// actors superseded (no single decision is current, and the one shown is superseded), or
    /// one that supersedes several. Absent when the line runs straight.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub forks: Vec<SupersessionFork>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ContestView {
    pub accepted_by: Vec<String>,
    pub rejected_by: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HypothesisSummaryView {
    pub id: String,
    pub status: HypothesisStatus,
    /// `assumption` (did it hold?) or `bet` (did we check?).
    pub kind: HypothesisKind,
    /// When to check a bet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check_by: Option<DateTime<Utc>>,
    pub statement: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refuting_evidence_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supporting_evidence_ids: Option<Vec<String>>,
    /// Whether anyone has said the decision still holds after this hypothesis was refuted. Absent
    /// unless it was.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reexamine: Option<ReexamineStatus>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BlockerSummary {
    pub id: String,
    pub reason: String,
    pub priority: BlockerPriority,
    pub blocked_actor_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ElidedSummary {
    pub superseded_decision_count: usize,
    pub unchosen_option_count: usize,
    pub resolved_blocker_count: usize,
    pub notification_count: usize,
    pub event_origins: Vec<u64>,
}

// ---------------------------------------------------------------------------
// Layer-3 function — wraps Layer-2 only; no direct graph queries
// ---------------------------------------------------------------------------

pub fn get_compact_view(
    graph: &impl GraphView,
    decision_id: &str,
) -> Result<QueryResponse<Option<CompactView>>> {
    get_compact_view_at(graph, decision_id, Utc::now())
}

/// [`get_compact_view`] as of `now`, which only decides what reads as unfinished in the
/// propagation of a decision that changed.
pub fn get_compact_view_at(
    graph: &impl GraphView,
    decision_id: &str,
    now: DateTime<Utc>,
) -> Result<QueryResponse<Option<CompactView>>> {
    let started = Instant::now();
    let decision_id = decision_id.trim();
    if decision_id.is_empty() {
        return Err(query_error("decision_id must not be empty").into());
    }

    // 1. Resolve focal decision
    let labels = ProjectLabels::from_graph(graph)?;
    let focal = get_decision_with_labels(graph, decision_id, &labels)?;
    let Some(focal_decision) = focal.data else {
        return Ok(QueryResponse {
            result_count: 0,
            truncated: false,
            latency_ms: started.elapsed().as_millis(),
            data: None,
        });
    };

    // 2. Supersession line → find terminal (newest, last in line). A line that forks has no
    // single terminal: it ends at the fork, and the fork is reported with the summary.
    let SupersessionLine { chain, forks } = walk_supersession(graph, decision_id)?.data;
    let terminal_id = chain
        .decision_ids
        .last()
        .cloned()
        .unwrap_or_else(|| decision_id.to_owned());

    // 3. Terminal decision (reuse focal when chain has length 1)
    let terminal_decision = if terminal_id == decision_id {
        focal_decision
    } else {
        get_decision_with_labels(graph, &terminal_id, &labels)?
            .data
            .unwrap_or(focal_decision)
    };

    // 4. Neighborhood of the terminal decision
    let neighborhood =
        neighborhood_structure(graph, &terminal_id, &NeighborhoodRequest::all())?.data;

    // 5. Active blockers scoped to this decision
    let blockers = get_active_decision_blockers(
        graph,
        &ActiveDecisionBlockersRequest {
            filters: DecisionBlockerFilters {
                decision_ids: vec![terminal_id.clone()],
                ..DecisionBlockerFilters::default()
            },
            limit: MAX_QUERY_RESULTS,
            cursor: None,
        },
    )?
    .data;

    // -----------------------------------------------------------------------
    // Extract contest actors (only meaningful when status == Contested)
    // -----------------------------------------------------------------------
    let contest = if terminal_decision.status == DecisionStatus::Contested {
        let accepted_by: Vec<String> = neighborhood
            .edges
            .iter()
            .filter(|e| e.relation == RelationKind::AcceptedBy && e.stored_source() == terminal_id)
            .map(|e| e.stored_target().to_owned())
            .collect();
        let rejected_by: Vec<String> = neighborhood
            .edges
            .iter()
            .filter(|e| e.relation == RelationKind::RejectedBy && e.stored_source() == terminal_id)
            .map(|e| e.stored_target().to_owned())
            .collect();
        Some(ContestView {
            accepted_by,
            rejected_by,
        })
    } else {
        None
    };

    // -----------------------------------------------------------------------
    // Supersession summary (omit when the decision is not part of a chain)
    // -----------------------------------------------------------------------
    // The decisions an older fork names are superseded too, though the line does not run through
    // them.
    let superseded_decision_count = chain.decision_ids.len().saturating_sub(1)
        + forks
            .iter()
            .filter(|fork| fork.side == SupersessionSide::Older)
            .map(|fork| fork.candidates.len())
            .sum::<usize>();
    let supersession_chain = if chain.decision_ids.len() > 1 || !forks.is_empty() {
        Some(SupersessionSummary {
            chain_length: chain.decision_ids.len(),
            oldest_id: chain.decision_ids.first().cloned().unwrap_or_default(),
            forks,
        })
    } else {
        None
    };

    // -----------------------------------------------------------------------
    // Options: track chosen set to count unchosen
    // -----------------------------------------------------------------------
    let chose_set: BTreeSet<String> = neighborhood
        .edges
        .iter()
        .filter(|e| e.relation == RelationKind::Chose && e.stored_source() == terminal_id)
        .map(|e| e.stored_target().to_owned())
        .collect();
    let unchosen_option_count = neighborhood
        .edges
        .iter()
        .filter(|e| {
            e.relation == RelationKind::HasOption
                && e.stored_source() == terminal_id
                && !chose_set.contains(e.stored_target())
        })
        .count();

    // -----------------------------------------------------------------------
    // Re-examination: what the decision rests on that changed, and whether anyone looked again.
    // Anchored reads, and none at all when nothing it rests on changed.
    // -----------------------------------------------------------------------
    let changed_premises: Vec<String> = neighborhood
        .edges
        .iter()
        .filter(|e| e.relation == RelationKind::FollowsFrom && e.stored_source() == terminal_id)
        .filter(|e| {
            neighborhood
                .nodes
                .iter()
                .find(|node| node.id == e.stored_target())
                .and_then(|node| node.decision_status)
                .is_some_and(|status| {
                    status != DecisionStatus::Proposed && status != DecisionStatus::Accepted
                })
        })
        .map(|e| e.stored_target().to_owned()) // ubs:ignore: owned id; the neighborhood is only borrowed
        .collect();
    let refuted_hypotheses: Vec<String> = terminal_decision
        .hypotheses
        .iter()
        .filter(|hyp| hyp.status == HypothesisStatus::Refuted)
        .map(|hyp| hyp.id.clone()) // ubs:ignore: owned id; the decision view is only borrowed
        .collect();
    let reexamine_of: BTreeMap<String, ReexamineStatus> = reexamine_entries_for(
        graph,
        &labels,
        &terminal_id,
        terminal_decision.project.as_deref(),
        &changed_premises,
        &refuted_hypotheses,
    )?
    .into_iter()
    .filter_map(|entry| Some((entry.premise_id, entry.state.status()?)))
    .collect();

    // -----------------------------------------------------------------------
    // Hypotheses: build summaries with evidence IDs from neighborhood hop-2
    // -----------------------------------------------------------------------
    let mut hypotheses = Vec::with_capacity(terminal_decision.hypotheses.len());
    for hyp in &terminal_decision.hypotheses {
        let refuting_ids: Vec<String> = neighborhood
            .edges
            .iter()
            .filter(|e| e.relation == RelationKind::Refutes && e.stored_target() == hyp.id)
            .map(|e| e.stored_source().to_owned())
            .collect();
        let supporting_ids: Vec<String> = neighborhood
            .edges
            .iter()
            .filter(|e| e.relation == RelationKind::Supports && e.stored_target() == hyp.id)
            .map(|e| e.stored_source().to_owned())
            .collect();

        let statement = get_hypothesis_statement(graph, &hyp.id)?.unwrap_or_default();

        let (refuting_evidence_ids, supporting_evidence_ids) = match hyp.status {
            HypothesisStatus::Refuted => (Some(refuting_ids), None),
            HypothesisStatus::Supported => (
                None,
                if supporting_ids.is_empty() {
                    None
                } else {
                    Some(supporting_ids)
                },
            ),
            HypothesisStatus::Open => (None, None),
        };

        hypotheses.push(HypothesisSummaryView {
            id: hyp.id.to_owned(),
            status: hyp.status,
            kind: hyp.kind,
            check_by: hyp.check_by,
            statement,
            refuting_evidence_ids,
            supporting_evidence_ids,
            reexamine: reexamine_of.get(&hyp.id).copied(),
        });
    }

    // -----------------------------------------------------------------------
    // What the decision rests on: premise decisions (with status) and who rests on it,
    // straight from the neighborhood's FOLLOWS_FROM edges
    // -----------------------------------------------------------------------
    let premises: Vec<PremiseSummaryView> = neighborhood
        .edges
        .iter()
        .filter(|e| e.relation == RelationKind::FollowsFrom && e.stored_source() == terminal_id)
        .filter_map(|e| {
            neighborhood
                .nodes
                .iter()
                .find(|node| node.id == e.stored_target())
                .and_then(|node| node.decision_status)
                .map(|status| PremiseSummaryView {
                    decision_id: e.stored_target().to_owned(), // ubs:ignore: owned id for the view; the neighborhood is only borrowed
                    status,
                    reexamine: reexamine_of.get(e.stored_target()).copied(),
                })
        })
        .collect();
    let dependents_count = neighborhood
        .edges
        .iter()
        .filter(|e| e.relation == RelationKind::FollowsFrom && e.stored_target() == terminal_id)
        .map(|e| e.stored_source())
        .collect::<BTreeSet<_>>()
        .len();
    let grounding_state = terminal_decision.grounding_state();
    let propagation = match terminal_decision.status {
        DecisionStatus::Superseded | DecisionStatus::Rejected | DecisionStatus::Contested => {
            propagation_of_decision(graph, &terminal_id, DEFAULT_PROPAGATION_WINDOW_DAYS, now)?
        }
        DecisionStatus::Proposed | DecisionStatus::Accepted => None,
    };

    // -----------------------------------------------------------------------
    // Active blockers summary
    // -----------------------------------------------------------------------
    let active_blockers: Vec<BlockerSummary> = blockers
        .items
        .into_iter()
        .map(|b| BlockerSummary {
            id: b.id,
            reason: b.reason,
            priority: b.priority,
            blocked_actor_id: b.blocked_actor_id,
        })
        .collect();

    // -----------------------------------------------------------------------
    // ElidedSummary — collect event_origins for elided graph elements
    // -----------------------------------------------------------------------
    let mut event_origins: Vec<u64> = Vec::new();

    // Superseded decision edges (terminal -[SUPERSEDES]-> older)
    for edge in &neighborhood.edges {
        if edge.relation == RelationKind::Supersedes && edge.stored_source() == terminal_id {
            if let Some(origin) = edge.event_origin {
                if let Ok(u) = u64::try_from(origin) {
                    event_origins.push(u);
                }
            }
        }
    }

    // Unchosen option edges
    for edge in &neighborhood.edges {
        if edge.relation == RelationKind::HasOption
            && edge.stored_source() == terminal_id
            && !chose_set.contains(edge.stored_target())
        {
            if let Some(origin) = edge.event_origin {
                if let Ok(u) = u64::try_from(origin) {
                    event_origins.push(u);
                }
            }
        }
    }

    event_origins.sort_unstable();
    event_origins.dedup();

    let elided = ElidedSummary {
        superseded_decision_count,
        unchosen_option_count,
        resolved_blocker_count: 0,
        notification_count: 0,
        event_origins,
    };

    Ok(QueryResponse {
        result_count: 1,
        truncated: false,
        latency_ms: started.elapsed().as_millis(),
        data: Some(CompactView {
            evidence_ids: terminal_decision.evidence_ids.clone(),
            decision: terminal_decision,
            supersession_chain,
            contest,
            hypotheses,
            premises,
            grounding_state,
            dependents_count,
            propagation,
            active_blockers,
            elided,
        }),
    })
}
