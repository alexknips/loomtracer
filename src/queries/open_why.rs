//! Open whys: binding decisions whose "why" is still to answer (hivemind-lapcm).
//!
//! A decision that clears the decision floor (it binds something, or is costly to undo, by its
//! capturer's own answers) and rests on nothing but what its own capture brought has an open
//! why: nobody has tied it to anything outside the call that recorded it. This list names them,
//! so the why can be answered by a person who knows it. It is an open state, never a refusal:
//! ingest is unchanged and every one of these decisions was written.
//!
//! # What lists a decision
//!
//! All of these hold:
//!
//! - **In force.** Not superseded and not rejected. Contested and merely proposed decisions are
//!   listed.
//! - **Binding.** Its floor verdict is `cleared`. A decision below the floor is not a decision
//!   that needs a parent, and one whose answers were never asked (it predates the floor, or came
//!   by a path that never asks) has no verdict; both are counted in `left_out`, not listed.
//! - **No earlier decision.** It follows from no other decision (`FOLLOWS_FROM`) and replaces
//!   none (`SUPERSEDES`): naming the decision it overturns is the reference a pivot has. A
//!   decision that follows from itself rests on nothing but itself and is not helped by that.
//! - **No person-declared root.** Nobody who is a person accepted it, and it records no
//!   delegation from a person. A ruling a person made is the root of its own why; an agent that
//!   scribed it is the recorder, never the decider.
//! - **Only its own capture's grounding.** Every observation, assumption and bet it rests on is
//!   its own: recorded before the decision's proposal, as part of the call that made it, and
//!   rested on by no other decision. A node a second decision rests on is reused, which is a
//!   link between decisions; a node recorded after the proposal was attributed later by someone
//!   else. Either way the decision rests on something its capture did not bring.
//!
//! A decision with nothing declared at all passes the last test vacuously and is listed with an
//! empty `rests_on`: its why is the most open there is.
//!
//! # What this cannot see
//!
//! The graph does not record which call recorded an observation, so "its own" is read from what
//! the graph does hold (one decision rests on it, and it was recorded before the proposal). An
//! observation recorded on its own earlier and cited by id by exactly one decision therefore
//! reads as its own, and an observation attributed later by an edge alone (`ground` naming an
//! existing node) does too. Both err toward listing the decision: a gap shown that a reader can
//! close, never a gap hidden.
//!
//! Pure Layer-2 read: no write, no ranking, no model, no similarity. It does not suggest a
//! parent; that is the analysis layer's job and stays out of the write path and this read.
//! Derived from the graph at read time, so a decision leaves the list the moment it follows from
//! another, a person decides it, a second decision shares its evidence, or it is superseded.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::events::HypothesisKind;
use crate::floor::{FloorVerdict, RecordedFloor};
use crate::projector::{GraphRow, GraphView, NodeKind, RelationKind};
use crate::Result;

use super::grounding::{hypothesis_facts_from_row, GroundingKind};
use super::grounding_facts::{distinct_links, hypothesis_links as decision_hypothesis_links};
use super::project_label::ProjectLabels;
use super::shared::{
    node_rows, normalized_limit, normalized_query, optional_datetime, optional_int,
    optional_string, query_error, query_timer_start, recorded_floor_of, relation_edges,
};
use super::status::{Decider, DecisionStandings, DecisionStatus};
use super::QueryResponse;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OpenWhyRequest {
    pub limit: usize,
    /// The `next_cursor` of the previous page.
    pub cursor: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OpenWhyResults {
    pub limit: usize,
    pub cursor: Option<String>,
    pub next_cursor: Option<String>,
    /// Every decision on the list, all pages together.
    pub total_matches: usize,
    /// In-force decisions this list did not consider, because of the floor.
    pub left_out: OpenWhyLeftOut,
    pub items: Vec<OpenWhyView>,
}

/// Decisions in force that never reached the list because of their floor verdict, so the list is
/// never mistaken for every decision. All pages together; both are zero on a ledger that holds
/// only binding decisions.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct OpenWhyLeftOut {
    /// Below the floor: the capturer said it binds nothing and is cheap to undo.
    pub below_floor: usize,
    /// Never asked: recorded before the answers existed, or by a path that never asks.
    pub floor_not_asked: usize,
}

/// One thing a listed decision rests on: all of it is its own capture's.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OwnGrounding {
    pub kind: GroundingKind,
    pub id: String,
    /// The text a person reads: the observation's content, or the assumption's or bet's statement.
    pub label: String,
    /// Where an observation was seen (URL, file@commit, test run), as given at capture.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OpenWhyView {
    pub decision_id: String,
    pub title: String,
    /// Stable link segment (`/decisions/<slug>`); absent on a decision predating slugs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    /// Address of the project the decision is filed under (see `DecisionView::project`).
    pub project: Option<String>,
    /// What a person calls that project.
    pub project_label: String,
    pub status: DecisionStatus,
    /// When the decision was captured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub captured_at: Option<DateTime<Utc>>,
    /// Who accepted it, each with the kind the actor-id prefix gives; empty while nobody has.
    pub deciders: Vec<Decider>,
    /// Who recorded it: often a scribe, never read as the decider.
    pub recorded_by: Option<String>,
    /// Its two floor answers and the verdict they reach (always `cleared` here).
    #[serde(flatten)]
    pub floor: RecordedFloor,
    /// What it rests on, all from its own capture: observations first, then assumptions, then
    /// bets, each by id. Empty when it declares nothing.
    pub rests_on: Vec<OwnGrounding>,
}

/// Binding decisions whose why is still to answer, oldest first (the ledger offset of the
/// decision's own record, then its id), one page at a time. `truncated` and `next_cursor`
/// continue; the cursor is a position in that order, so a decision recorded or answered between
/// two calls never makes a page skip or repeat one.
pub fn get_open_why_decisions(
    graph: &impl GraphView,
    request: &OpenWhyRequest,
) -> Result<QueryResponse<OpenWhyResults>> {
    let started = query_timer_start();
    let limit = normalized_limit(request.limit);
    let cursor = normalized_query(request.cursor.as_deref());
    let after = cursor.as_deref().map(parse_position).transpose()?;

    let standings = DecisionStandings::load(graph)?;
    let decisions = node_rows(graph, NodeKind::Decision)?;
    let evidence_rows = node_rows(graph, NodeKind::Evidence)?;
    let hypothesis_rows = node_rows(graph, NodeKind::Hypothesis)?;
    let premise_links = distinct_links(graph, RelationKind::FollowsFrom)?;
    let evidence_links = distinct_links(graph, RelationKind::BasedOn)?;
    let hypothesis_links = decision_hypothesis_links(graph)?;

    // A decision that follows from itself rests on nothing but itself. `SUPERSEDES` runs newer
    // decision -> older decision, so the source is the one that names an earlier decision.
    let replacements = relation_edges(graph, RelationKind::Supersedes)?;
    let names_an_earlier_decision: BTreeSet<&str> = premise_links
        .iter()
        .chain(&replacements)
        .filter(|(from, to)| from != to)
        .map(|(from, _)| from.as_str())
        .collect();
    let evidence = Resting::from_links(&evidence_links);
    let hypotheses = Resting::from_links(&hypothesis_links);

    let mut left_out = OpenWhyLeftOut::default();
    let mut found: Vec<Found<'_>> = Vec::new();
    for (decision_id, row) in &decisions {
        // A node only ever named (by a request, a blocker, a supersession) is not a decision.
        if optional_string(row, "title").is_none() {
            continue;
        }
        if matches!(
            standings.status_of(decision_id),
            DecisionStatus::Superseded | DecisionStatus::Rejected
        ) {
            continue;
        }
        match recorded_floor_of(row).verdict {
            FloorVerdict::Cleared => {}
            FloorVerdict::Below => {
                left_out.below_floor += 1;
                continue;
            }
            FloorVerdict::NotAsked => {
                left_out.floor_not_asked += 1;
                continue;
            }
        }
        if names_an_earlier_decision.contains(decision_id.as_str()) {
            continue;
        }
        let deciders = standings.deciders_of(decision_id);
        if deciders.iter().any(|decider| decider.kind == "human")
            || optional_string(row, "delegated_by").is_some()
        {
            continue;
        }
        // Without its own offset there is no "recorded before the proposal" to test.
        let Some(proposed_at) = optional_int(row, "event_origin") else {
            continue;
        };
        let own_evidence = evidence.nodes_of(decision_id);
        let own_hypotheses = hypotheses.nodes_of(decision_id);
        let is_own = |resting: &Resting<'_>, rows: &BTreeMap<String, GraphRow>, node_id: &str| {
            resting.users_of(node_id) == 1
                && rows
                    .get(node_id)
                    .and_then(|node| optional_int(node, "event_origin"))
                    .is_some_and(|recorded_at| recorded_at < proposed_at)
        };
        if !own_evidence
            .iter()
            .all(|node_id| is_own(&evidence, &evidence_rows, node_id))
            || !own_hypotheses
                .iter()
                .all(|node_id| is_own(&hypotheses, &hypothesis_rows, node_id))
        {
            continue;
        }
        found.push(Found {
            position: (proposed_at, decision_id.as_str()),
            evidence: own_evidence,
            hypotheses: own_hypotheses,
        });
    }
    found.sort_unstable_by(|left, right| left.position.cmp(&right.position));

    let total_matches = found.len();
    let start = after.as_ref().map_or(0, |after| {
        let after = (after.event_origin, after.decision_id.as_str());
        found.partition_point(|candidate| candidate.position <= after)
    });
    let page = found
        .get(start..found.len().min(start.saturating_add(limit)))
        .unwrap_or_default();
    let next_cursor = if start + page.len() < total_matches {
        page.last().map(position_of).transpose()?
    } else {
        None
    };

    // Only the page is filled in.
    let labels = ProjectLabels::from_graph(graph)?;
    let mut recorded_by: BTreeMap<String, String> = BTreeMap::new();
    // Edges arrive sorted by (decision, actor): the first recorder by actor id is the one shown.
    for (decision_id, actor_id) in relation_edges(graph, RelationKind::ProposedBy)? {
        recorded_by.entry(decision_id).or_insert(actor_id);
    }
    let items = page
        .iter()
        .map(|candidate| {
            let decision_id = candidate.position.1;
            let Some(row) = decisions.get(decision_id) else {
                return Ok(None);
            };
            let project = optional_string(row, "project");
            let mut rests_on = Vec::new();
            for node_id in &candidate.evidence {
                rests_on.push(evidence_grounding(node_id, evidence_rows.get(*node_id)));
            }
            for node_id in &candidate.hypotheses {
                rests_on.push(hypothesis_grounding(
                    node_id,
                    hypothesis_rows.get(*node_id),
                )?);
            }
            rests_on.sort_by(|left, right| (left.kind, &left.id).cmp(&(right.kind, &right.id)));
            Ok(Some(OpenWhyView {
                decision_id: decision_id.to_owned(),
                title: optional_string(row, "title").unwrap_or_default(),
                slug: optional_string(row, "slug"),
                project_label: labels.label_of(project.as_deref()),
                project,
                status: standings.status_of(decision_id),
                captured_at: optional_datetime(row, "occurred_at")?,
                deciders: standings.deciders_of(decision_id),
                recorded_by: recorded_by.remove(decision_id),
                floor: recorded_floor_of(row),
                rests_on,
            }))
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

    Ok(QueryResponse {
        result_count: items.len(),
        truncated: next_cursor.is_some(),
        latency_ms: started.elapsed().as_millis(),
        data: OpenWhyResults {
            limit,
            cursor,
            next_cursor,
            total_matches,
            left_out,
            items,
        },
    })
}

/// A decision on the list before its words are filled in, with the ids of what it rests on.
struct Found<'a> {
    /// `(ledger offset of the decision's own record, id)`: the order, and what a cursor is.
    position: (i64, &'a str),
    evidence: Vec<&'a str>,
    hypotheses: Vec<&'a str>,
}

/// Which decisions rest on which node, from a sorted, distinct list of `(decision, node)` links.
struct Resting<'a> {
    by_decision: BTreeMap<&'a str, Vec<&'a str>>,
    users: BTreeMap<&'a str, usize>,
}

impl<'a> Resting<'a> {
    fn from_links(links: &'a [(String, String)]) -> Self {
        let mut by_decision: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        let mut users: BTreeMap<&str, usize> = BTreeMap::new();
        for (decision_id, node_id) in links {
            by_decision.entry(decision_id).or_default().push(node_id);
            *users.entry(node_id).or_default() += 1;
        }
        Self { by_decision, users }
    }

    /// The nodes this decision rests on, sorted by id.
    fn nodes_of(&self, decision_id: &str) -> Vec<&'a str> {
        self.by_decision
            .get(decision_id)
            .cloned()
            .unwrap_or_default()
    }

    /// How many distinct decisions rest on this node.
    fn users_of(&self, node_id: &str) -> usize {
        self.users.get(node_id).copied().unwrap_or(0)
    }
}

fn evidence_grounding(evidence_id: &str, row: Option<&GraphRow>) -> OwnGrounding {
    OwnGrounding {
        kind: GroundingKind::Evidence,
        id: evidence_id.to_owned(),
        label: row
            .and_then(|row| optional_string(row, "content"))
            .unwrap_or_else(|| evidence_id.to_owned()),
        source: row.and_then(|row| optional_string(row, "evidence_source")),
    }
}

fn hypothesis_grounding(hypothesis_id: &str, row: Option<&GraphRow>) -> Result<OwnGrounding> {
    let facts = row.map(hypothesis_facts_from_row).transpose()?;
    let kind = match facts.as_ref().map(|facts| facts.kind) {
        Some(HypothesisKind::Bet) => GroundingKind::Bet,
        _ => GroundingKind::Assumption,
    };
    Ok(OwnGrounding {
        kind,
        id: hypothesis_id.to_owned(),
        label: facts
            .and_then(|facts| facts.statement)
            .unwrap_or_else(|| hypothesis_id.to_owned()),
        source: None,
    })
}

/// A position in the order: the position of the last row a page returned.
#[derive(Deserialize, Serialize)]
struct Position {
    event_origin: i64,
    decision_id: String,
}

fn position_of(last: &Found<'_>) -> Result<String> {
    serde_json::to_string(&Position {
        event_origin: last.position.0,
        decision_id: last.position.1.to_owned(),
    })
    .map_err(|error| query_error(format!("cursor could not be written: {error}")).into())
}

fn parse_position(cursor: &str) -> Result<Position> {
    serde_json::from_str(cursor)
        .map_err(|_| query_error("cursor is not one an open-why list returned".to_owned()).into())
}

#[cfg(test)]
mod tests;
