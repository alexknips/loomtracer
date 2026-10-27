//! Decision context features: condition signals for why a decision was made the way it was.
//!
//! Derives INDEPENDENT VARIABLES for failure-mode attribution — the circumstances under
//! which a decision was captured — purely from the graph. No LLMs. Works on any deployment.
//!
//! Six feature groups:
//!
//! - `authorship`: who produced the decision (human-authored / agent-proposed+human-accepted /
//!   agent-only / unknown) via `PROPOSED_BY` and `ACCEPTED_BY` edges.
//! - `source` / `source_ref`: which system or model captured it (cli/agent/human/slack/…).
//! - `review`: how substantively the decision was reviewed (unreviewed / self_accepted /
//!   peer_reviewed / disputed / rejected).
//! - `delegated_by`: the human whose delegation an agent's self-acceptance fell within
//!   (absent for an agent deciding alone).
//! - `evidence_count` / `hypothesis_count`: sufficiency of supporting substrate.
//! - `options_count` / `rationale_chars`: context richness proxies.
//!
//! Pair with `DecisionOutcome` from `outcome.rs` for causal attribution: context is the
//! independent variable, outcome is the dependent variable.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::projector::{GraphParams, GraphValue, GraphView, NodeKind};
use crate::Result;

use super::shared::{
    node_rows, optional_datetime, optional_int, optional_string, query_error, query_timer_start,
    relayer_of, required_string, MAX_QUERY_RESULTS,
};
use super::QueryResponse;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// How a decision was authored: who produced it and whether a peer reviewed it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorshipShape {
    /// The actor who proposed the decision is a human (`actor_id` starts with `human:`).
    HumanAuthored,
    /// An agent proposed the decision and at least one human actor accepted it.
    AgentProposedHumanAccepted,
    /// An agent proposed the decision; no human actor appears in `ACCEPTED_BY` edges.
    AgentOnly,
    /// No `PROPOSED_BY` edge exists (e.g., decision arrived via a path that omitted provenance).
    Unknown,
}

/// How substantively the decision was reviewed by actors other than the proposer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewShape {
    /// No `ACCEPTED_BY` or `REJECTED_BY` edges — decision was never explicitly reviewed.
    Unreviewed,
    /// The only `ACCEPTED_BY` actor is the proposer themselves (self-acceptance), or, on a
    /// classifier draft that names no proposer, the agent that ran the session: its own call,
    /// which nobody else reviewed (hivemind-26br4).
    SelfAccepted,
    /// At least one `ACCEPTED_BY` actor is distinct from the proposer.
    PeerReviewed,
    /// Both `ACCEPTED_BY` and `REJECTED_BY` actors exist — actively contested.
    Disputed,
    /// At least one `REJECTED_BY` actor and no `ACCEPTED_BY` actor — someone reviewed it and
    /// turned it down, and nobody has accepted it. Not `Unreviewed`: a rejection is a review.
    Rejected,
}

/// Derived context record for a single decision: the conditions under which it was made.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DecisionContext {
    pub decision_id: String,
    /// Authorship shape derived from `PROPOSED_BY` and `ACCEPTED_BY` edges.
    pub authorship: AuthorshipShape,
    /// Actor ID of the proposer (`PROPOSED_BY` target), if present. This is the actor who
    /// *recorded* the decision, not necessarily the one who made it -- see `accepted_by`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposer_id: Option<String>,
    /// Actor IDs of every `ACCEPTED_BY` target: the actor(s) who actually *decided*. May
    /// differ from `proposer_id` (an agent scribing a human's decision, `decided_by` at
    /// capture time) or overlap it (self-acceptance). Empty when `review` is `Unreviewed`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub accepted_by: Vec<String>,
    /// Source system that captured the decision (cli / agent / human / slack / document / api).
    pub source: String,
    /// Free-text model/session reference from the capturing event (e.g. `claude:opus:session-xyz`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<String>,
    /// The human whose delegated scope an agent's self-acceptance fell within
    /// (hivemind-zdsh.6), read off the Decision node's `delegated_by` property. Orthogonal to
    /// `authorship`/`review`: those enums are unchanged, so an agent deciding under a
    /// delegation still derives `AgentOnly` + `SelfAccepted` — this field is the extra fact
    /// that separates it from an agent deciding alone (same shapes, `None` here).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delegated_by: Option<String>,
    /// Review depth: was the decision examined by a peer?
    pub review: ReviewShape,
    /// Actors the decision names as deciders who said the call was not theirs
    /// (`decision.attribution_disputed`), one per actor with their latest statement. The
    /// acceptance stays in `accepted_by`, and `authorship` and `review` stay as recorded: a
    /// dispute sits beside the acceptance, never replaces it (hivemind-purmd, hivemind-1sanf).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attribution_disputes: Vec<AttributionDispute>,
    /// `authorship` reads `AgentProposedHumanAccepted`, but every human who accepted it has
    /// disputed that acceptance, so no human is left standing behind the call. The failure-mode
    /// report groups such a decision apart rather than with the human-accepted ones.
    #[serde(skip)]
    pub(crate) human_acceptance_disputed: bool,
    /// `review` reads `PeerReviewed`, but every acceptor other than the recorder has disputed
    /// that acceptance, so nobody else is left having reviewed the call.
    #[serde(skip)]
    pub(crate) peer_review_disputed: bool,
    /// Number of actors who accepted the decision.
    pub accepted_count: i64,
    /// Number of actors who rejected the decision.
    pub rejected_count: i64,
    /// Number of `BASED_ON` evidence edges.
    pub evidence_count: i64,
    /// Number of hypotheses premised on (via options or direct `PREMISED_ON_DIRECT` edges).
    pub hypothesis_count: i64,
    /// Number of options considered (`HAS_OPTION` edges).
    pub options_count: i64,
    /// Character count of the decision rationale (proxy for context richness).
    pub rationale_chars: i64,
}

/// One actor's statement that a decision names them as its decider and the call was not theirs.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AttributionDispute {
    pub disputed_by: String,
    /// What they said: their own words, typed with their credential or, when `relayed_by` is
    /// present, the verbatim quote their agent recorded.
    pub reason: String,
    /// The agent that recorded the statement for the person (hivemind-f9xgu): the event's
    /// `agent` source reference when it is not the disputing actor itself. Absent when the actor
    /// disputed with their own credential.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relayed_by: Option<String>,
    /// Where they said it (a session, thread or message), as the relaying agent stated it.
    /// Absent when nobody did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub said_in: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disputed_at: Option<DateTime<Utc>>,
}

/// Request for the bulk context-candidates query.
#[derive(Clone, Debug, Default)]
pub struct DecisionContextRequest {
    /// Minimum ledger offset to include (inclusive). Filters by the decision's `event_origin`.
    pub since_event_origin: Option<i64>,
    /// Maximum number of results (capped at `MAX_QUERY_RESULTS`).
    pub limit: usize,
    /// Skip N results (offset-based cursor).
    pub cursor: Option<String>,
}

// ---------------------------------------------------------------------------
// Single-decision context
// ---------------------------------------------------------------------------

/// Derive the context record for a single decision, or `None` if the decision does not exist.
pub fn get_decision_context(
    graph: &impl GraphView,
    decision_id: &str,
) -> Result<QueryResponse<Option<DecisionContext>>> {
    let started = query_timer_start();

    let decision_rows = graph.query(
        "MATCH (d:`Decision` {id: $id}) \
         RETURN d.id AS id, d.source AS source, d.source_ref AS source_ref, d.rationale AS rationale, d.delegated_by AS delegated_by \
         LIMIT 1;",
        &GraphParams::from([("id".to_owned(), GraphValue::String(decision_id.to_owned()))]),
    )?;

    let data = if let Some(row) = decision_rows.first() {
        let id = required_string(row, "id")?;
        let source = optional_string(row, "source").unwrap_or_else(|| "cli".to_owned());
        let source_ref = optional_string(row, "source_ref");
        let rationale_chars = optional_string(row, "rationale")
            .map(|r| r.len() as i64)
            .unwrap_or(0);
        let attribution_disputes = attribution_disputes_by_decision(graph)?
            .remove(&id)
            .unwrap_or_default();
        Some(derive_context(
            graph,
            &id,
            source,
            source_ref,
            optional_string(row, "delegated_by"),
            rationale_chars,
            attribution_disputes,
        )?)
    } else {
        None
    };

    Ok(QueryResponse {
        result_count: usize::from(data.is_some()),
        truncated: false,
        latency_ms: started.elapsed().as_millis(),
        data,
    })
}

// ---------------------------------------------------------------------------
// Bulk context-candidates query
// ---------------------------------------------------------------------------

/// Return context records for all decisions (or a filtered subset), paginated.
/// Complements `get_decision_quality_candidates` — context is the independent variable
/// side of the causal pair.
pub fn get_decision_context_candidates(
    graph: &impl GraphView,
    request: &DecisionContextRequest,
) -> Result<QueryResponse<Vec<DecisionContext>>> {
    let started = Instant::now();
    let limit = if request.limit == 0 {
        MAX_QUERY_RESULTS
    } else {
        request.limit.min(MAX_QUERY_RESULTS)
    };
    let skip: usize = match request.cursor.as_deref() {
        None => 0,
        Some(c) => c
            .parse::<usize>()
            .map_err(|e| query_error(format!("cursor must be a non-negative offset: {e}")))?,
    };

    let decision_rows = if let Some(since) = request.since_event_origin {
        graph.query(
            "MATCH (d:`Decision`) WHERE d.event_origin >= $since \
             RETURN d.id AS id, d.source AS source, d.source_ref AS source_ref, d.rationale AS rationale, d.delegated_by AS delegated_by \
             ORDER BY d.event_origin, d.id;",
            &GraphParams::from([("since".to_owned(), GraphValue::Int(since))]),
        )?
    } else {
        graph.query(
            "MATCH (d:`Decision`) \
             RETURN d.id AS id, d.source AS source, d.source_ref AS source_ref, d.rationale AS rationale, d.delegated_by AS delegated_by \
             ORDER BY d.event_origin, d.id;",
            &GraphParams::new(),
        )?
    };

    let paged: Vec<_> = decision_rows
        .into_iter()
        .skip(skip)
        .take(limit + 1)
        .collect();
    let truncated = paged.len() > limit;
    let window = paged.get(..paged.len().min(limit)).unwrap_or(&[]);

    // One read of the (few) dispute nodes for the whole page, not one per decision.
    let mut disputes_by_decision = attribution_disputes_by_decision(graph)?;
    let mut contexts = Vec::with_capacity(window.len());
    for row in window {
        let id = required_string(row, "id")?;
        let source = optional_string(row, "source").unwrap_or_else(|| "cli".to_owned());
        let source_ref = optional_string(row, "source_ref");
        let rationale_chars = optional_string(row, "rationale")
            .map(|r| r.len() as i64)
            .unwrap_or(0);
        let attribution_disputes = disputes_by_decision.remove(&id).unwrap_or_default();
        contexts.push(derive_context(
            graph,
            &id,
            source,
            source_ref,
            optional_string(row, "delegated_by"),
            rationale_chars,
            attribution_disputes,
        )?);
    }

    Ok(QueryResponse {
        result_count: contexts.len(),
        truncated,
        latency_ms: started.elapsed().as_millis(),
        data: contexts,
    })
}

// ---------------------------------------------------------------------------
// Core derivation logic
// ---------------------------------------------------------------------------

fn derive_context(
    graph: &impl GraphView,
    decision_id: &str,
    source: String,
    source_ref: Option<String>,
    delegated_by: Option<String>,
    rationale_chars: i64,
    attribution_disputes: Vec<AttributionDispute>,
) -> Result<DecisionContext> {
    let proposer = query_proposer(graph, decision_id)?;
    let proposer_id = proposer.as_ref().map(|(id, _)| id.clone());
    let proposer_kind = proposer.as_ref().map(|(_, k)| k.as_str()).unwrap_or("");

    let acceptors = query_actor_ids_by_edge(graph, decision_id, "ACCEPTED_BY")?;
    let accepted_count = acceptors.len() as i64;

    let rejected_count = query_actor_edge_count(graph, decision_id, "REJECTED_BY")?;

    let authorship = derive_authorship(proposer_kind, &acceptors);
    // `proposer_id` and `authorship` stay as recorded: a proposer is never inferred. Only the
    // review reads the session agent as the one who made the call, because no one else is
    // on record as having looked at it.
    let recorder = match &proposer_id {
        Some(id) => Some(id.clone()),
        None => session_agent_deciding_alone(graph, decision_id, &acceptors)?,
    };
    let review = derive_review(
        recorder.as_deref(),
        &acceptors,
        accepted_count,
        rejected_count,
    );

    let evidence_count = query_edge_count(graph, decision_id, "BASED_ON", "Evidence")?;
    let hypothesis_count = query_hypothesis_count(graph, decision_id)?;
    let options_count = query_edge_count(graph, decision_id, "HAS_OPTION", "Option")?;

    let accepted_by: Vec<String> = acceptors.iter().map(|(id, _)| id.clone()).collect();

    // The acceptances that still stand: the recorded ones less those their own actor disputed.
    // `authorship` and `review` stay as recorded, so these two flags say only that the reading
    // they would give without the disputed acceptances is a weaker one.
    let disputers: HashSet<&str> = attribution_disputes
        .iter()
        .map(|dispute| dispute.disputed_by.as_str())
        .collect();
    let standing: Vec<(String, String)> = acceptors
        .iter()
        .filter(|(id, _)| !disputers.contains(id.as_str()))
        .cloned() // ubs:ignore: owned pairs required; the acceptors list is borrowed
        .collect();
    let human_acceptance_disputed = authorship == AuthorshipShape::AgentProposedHumanAccepted
        && derive_authorship(proposer_kind, &standing) != authorship;
    let peer_review_disputed = review == ReviewShape::PeerReviewed
        && derive_review(
            recorder.as_deref(),
            &standing,
            standing.len() as i64,
            rejected_count,
        ) != review;

    Ok(DecisionContext {
        decision_id: decision_id.to_owned(),
        authorship,
        proposer_id,
        accepted_by,
        source,
        source_ref,
        delegated_by,
        review,
        attribution_disputes,
        human_acceptance_disputed,
        peer_review_disputed,
        accepted_count,
        rejected_count,
        evidence_count,
        hypothesis_count,
        options_count,
        rationale_chars,
    })
}

/// The attribution disputes recorded in the ledger, by the decision they dispute: each
/// disputing actor's latest statement, in the order those statements were recorded. One bulk
/// read of the (few) dispute nodes: they have no edges, so a decision is matched on the node's
/// `decision_id` property.
fn attribution_disputes_by_decision(
    graph: &impl GraphView,
) -> Result<HashMap<String, Vec<AttributionDispute>>> {
    let mut recorded: HashMap<String, Vec<(i64, AttributionDispute)>> = HashMap::new();
    for (_, row) in node_rows(graph, NodeKind::AttributionDispute)? {
        let decision_id = required_string(&row, "decision_id")?;
        let origin = optional_int(&row, "event_origin").unwrap_or(i64::MAX);
        let disputed_by = optional_string(&row, "disputed_by").unwrap_or_default();
        let relayed_by = relayer_of(
            optional_string(&row, "source").as_deref(),
            optional_string(&row, "source_ref"),
            &disputed_by,
        );
        let dispute = AttributionDispute {
            disputed_by,
            reason: optional_string(&row, "reason").unwrap_or_default(),
            relayed_by,
            said_in: optional_string(&row, "said_in"),
            disputed_at: optional_datetime(&row, "disputed_at")?,
        };
        let disputes = recorded.entry(decision_id).or_default();
        match disputes
            .iter_mut()
            .find(|(_, seen)| seen.disputed_by == dispute.disputed_by)
        {
            Some((seen_origin, _)) if *seen_origin > origin => {}
            Some(slot) => *slot = (origin, dispute),
            None => disputes.push((origin, dispute)),
        }
    }
    Ok(recorded
        .into_iter()
        .map(|(decision_id, mut disputes)| {
            disputes.sort_by(|(a, left), (b, right)| {
                a.cmp(b)
                    .then_with(|| left.disputed_by.cmp(&right.disputed_by))
            });
            (
                decision_id,
                disputes.into_iter().map(|(_, dispute)| dispute).collect(),
            )
        })
        .collect())
}

fn derive_authorship(proposer_kind: &str, acceptors: &[(String, String)]) -> AuthorshipShape {
    match proposer_kind {
        "human" => AuthorshipShape::HumanAuthored,
        "agent" => {
            let has_human_acceptor = acceptors.iter().any(|(_, k)| k == "human");
            if has_human_acceptor {
                AuthorshipShape::AgentProposedHumanAccepted
            } else {
                AuthorshipShape::AgentOnly
            }
        }
        _ => AuthorshipShape::Unknown,
    }
}

/// The sole acceptor of a decision that names no proposer, when it is an agent that took part in
/// the conversation the decision was drafted from (`PARTICIPATED_BY`): the session agent made
/// the call and carried it out, and the draft credits no one else. A person who accepts such a
/// draft afterwards, or an agent that was not in the session, reviewed it.
fn session_agent_deciding_alone(
    graph: &impl GraphView,
    decision_id: &str,
    acceptors: &[(String, String)],
) -> Result<Option<String>> {
    let [(acceptor_id, kind)] = acceptors else {
        return Ok(None);
    };
    if kind != "agent" {
        return Ok(None);
    }
    let participants = linked_actor_ids(graph, decision_id, "PARTICIPATED_BY")?;
    Ok(participants
        .contains(acceptor_id)
        .then(|| acceptor_id.clone()))
}

fn derive_review(
    proposer_id: Option<&str>,
    acceptors: &[(String, String)],
    accepted_count: i64,
    rejected_count: i64,
) -> ReviewShape {
    if accepted_count > 0 && rejected_count > 0 {
        return ReviewShape::Disputed;
    }
    if accepted_count == 0 {
        return if rejected_count > 0 {
            ReviewShape::Rejected
        } else {
            ReviewShape::Unreviewed
        };
    }
    // accepted_count > 0, rejected_count == 0
    let only_self = match proposer_id {
        Some(pid) => acceptors.len() == 1 && acceptors.first().is_some_and(|(id, _)| id == pid),
        None => false,
    };
    if only_self {
        ReviewShape::SelfAccepted
    } else {
        ReviewShape::PeerReviewed
    }
}

// ---------------------------------------------------------------------------
// Graph sub-queries
// ---------------------------------------------------------------------------

/// Returns `Some((actor_id, kind))` for the single `PROPOSED_BY` actor, or `None`.
fn query_proposer(graph: &impl GraphView, decision_id: &str) -> Result<Option<(String, String)>> {
    let rows = graph.query(
        "MATCH (d:`Decision` {id: $id})-[:`PROPOSED_BY`]->(a:`Actor`) \
         RETURN a.id AS actor_id, a.kind AS kind \
         LIMIT 1;",
        &GraphParams::from([("id".to_owned(), GraphValue::String(decision_id.to_owned()))]),
    )?;
    Ok(rows.first().map(|row| {
        let actor_id = optional_string(row, "actor_id").unwrap_or_default();
        let kind = optional_string(row, "kind").unwrap_or_default();
        (actor_id, kind)
    }))
}

/// The ids of the actors `decision_id` points at with `relation` (a table name such as
/// `REJECTED_BY` or `INITIATED_BY`), sorted by id.
pub(super) fn linked_actor_ids(
    graph: &impl GraphView,
    decision_id: &str,
    relation: &str,
) -> Result<Vec<String>> {
    Ok(query_actor_ids_by_edge(graph, decision_id, relation)?
        .into_iter()
        .map(|(actor_id, _kind)| actor_id)
        .collect())
}

/// Returns `(actor_id, kind)` pairs for all actors connected via the given relationship label.
fn query_actor_ids_by_edge(
    graph: &impl GraphView,
    decision_id: &str,
    relation: &str,
) -> Result<Vec<(String, String)>> {
    let cypher = format!(
        "MATCH (d:`Decision` {{id: $id}})-[:`{relation}`]->(a:`Actor`) \
         RETURN a.id AS actor_id, a.kind AS kind \
         ORDER BY a.id;"
    );
    let rows = graph.query(
        &cypher,
        &GraphParams::from([("id".to_owned(), GraphValue::String(decision_id.to_owned()))]),
    )?;
    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        let actor_id = optional_string(&row, "actor_id").unwrap_or_default();
        let kind = optional_string(&row, "kind").unwrap_or_default();
        result.push((actor_id, kind));
    }
    Ok(result)
}

/// Returns the count of actors connected via the given relationship label.
fn query_actor_edge_count(
    graph: &impl GraphView,
    decision_id: &str,
    relation: &str,
) -> Result<i64> {
    let cypher = format!(
        "MATCH (d:`Decision` {{id: $id}})-[:`{relation}`]->(:`Actor`) \
         RETURN count(*) AS cnt;"
    );
    let rows = graph.query(
        &cypher,
        &GraphParams::from([("id".to_owned(), GraphValue::String(decision_id.to_owned()))]),
    )?;
    Ok(rows
        .first()
        .and_then(|r| optional_int(r, "cnt"))
        .unwrap_or(0))
}

/// Returns the count of nodes connected via a given relationship label.
fn query_edge_count(
    graph: &impl GraphView,
    decision_id: &str,
    relation: &str,
    target_kind: &str,
) -> Result<i64> {
    let cypher = format!(
        "MATCH (d:`Decision` {{id: $id}})-[:`{relation}`]->(:`{target_kind}`) \
         RETURN count(*) AS cnt;"
    );
    let rows = graph.query(
        &cypher,
        &GraphParams::from([("id".to_owned(), GraphValue::String(decision_id.to_owned()))]),
    )?;
    Ok(rows
        .first()
        .and_then(|r| optional_int(r, "cnt"))
        .unwrap_or(0))
}

/// Returns the total hypothesis count (via options or direct `PREMISED_ON_DIRECT` edges).
fn query_hypothesis_count(graph: &impl GraphView, decision_id: &str) -> Result<i64> {
    let rows = graph.query(
        "MATCH (d:`Decision` {id: $id})-[:`CHOSE`]->(:`Option`)-[:`PREMISED_ON`]->(h:`Hypothesis`) RETURN h.id AS hid \
         UNION \
         MATCH (d:`Decision` {id: $id})-[:`PREMISED_ON_DIRECT`]->(h:`Hypothesis`) RETURN h.id AS hid \
         ORDER BY hid;",
        &GraphParams::from([("id".to_owned(), GraphValue::String(decision_id.to_owned()))]),
    )?;
    Ok(rows.len() as i64)
}

// ---------------------------------------------------------------------------
// Cursor helpers (public for MCP serialization)
// ---------------------------------------------------------------------------

pub fn context_next_cursor(skip: usize, window_len: usize) -> Option<String> {
    if window_len > 0 {
        Some((skip + window_len).to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests;
