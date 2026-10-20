//! Supersession chain traversal: walks the SUPERSEDES edge graph with cycle protection.

use std::collections::BTreeSet;

use serde::Serialize;

use crate::projector::{GraphParams, GraphValue, GraphView, RelationKind};
use crate::{HivemindError, Result};

use super::follows_from::decision_path;
use super::shared::{query_error, query_timer_start, required_string};
use super::QueryResponse;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SupersessionChain {
    pub decision_ids: Vec<String>,
    pub input_index: usize,
}

/// Which way along SUPERSEDES from a decision a fork lies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SupersessionSide {
    /// The decision supersedes several decisions.
    Older,
    /// Several decisions supersede the decision: nobody settled on a single successor.
    Newer,
}

/// A decision with more than one neighbour on one side of its supersession line, which the line
/// cannot continue past.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SupersessionFork {
    pub at: String,
    pub side: SupersessionSide,
    /// The distinct decisions on that side of `at`, in id order.
    pub candidates: Vec<String>,
}

impl SupersessionFork {
    /// The fork in a sentence; the one place the compact view's renderers and the chain summary
    /// get their wording, so they cannot disagree.
    pub fn note(&self) -> String {
        let candidates = self.candidates.join(", ");
        match self.side {
            SupersessionSide::Older => format!(
                "{} supersedes {} decisions ({candidates})",
                self.at,
                self.candidates.len()
            ),
            SupersessionSide::Newer => format!(
                "{} is superseded by {} decisions ({candidates}), so no single decision is current",
                self.at,
                self.candidates.len()
            ),
        }
    }
}

/// The supersession line through a decision, as far as it runs without forking.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SupersessionLine {
    pub chain: SupersessionChain,
    /// Where the line stopped for lack of a single next decision: at most one fork per side,
    /// the older side first.
    pub forks: Vec<SupersessionFork>,
}

/// The line through `decision_id` in both directions. A fork ends the line on that side and is
/// reported in [`SupersessionLine::forks`]: the write side lets two decisions supersede the same
/// one, and a decision supersede several, so a fork is a fact of the ledger and not a failure.
/// A cycle is still an error.
pub fn walk_supersession(
    graph: &impl GraphView,
    decision_id: &str,
) -> Result<QueryResponse<SupersessionLine>> {
    let started = query_timer_start();
    if decision_id.trim().is_empty() {
        return Err(query_error("decision_id must not be empty").into());
    }

    let mut visited = BTreeSet::new();
    visited.insert(decision_id.to_owned());

    let (mut older, older_fork) =
        walk_side(graph, decision_id, SupersessionSide::Older, &mut visited)?;
    let (newer, newer_fork) = walk_side(graph, decision_id, SupersessionSide::Newer, &mut visited)?;

    older.reverse();
    let input_index = older.len();
    let mut decision_ids = older;
    decision_ids.push(decision_id.to_owned());
    decision_ids.extend(newer);

    Ok(QueryResponse {
        result_count: decision_ids.len(),
        truncated: false,
        latency_ms: started.elapsed().as_millis(),
        data: SupersessionLine {
            chain: SupersessionChain {
                decision_ids,
                input_index,
            },
            forks: older_fork.into_iter().chain(newer_fork).collect(),
        },
    })
}

/// The chain of decisions that connects `from_id` to `to_id` over `SUPERSEDES` — `from_id`
/// supersedes the second id, which supersedes the third, and so on until `to_id` — or `None`
/// when `from_id` does not (transitively) supersede `to_id`. Ids run from `from_id` to `to_id`
/// inclusive; `from_id == to_id` is the one-element chain. It follows every edge, so a fork on
/// either side is no obstacle, and it ends on a graph that already holds a cycle.
pub fn supersedes_path(
    graph: &impl GraphView,
    from_id: &str,
    to_id: &str,
) -> Result<Option<Vec<String>>> {
    decision_path(graph, RelationKind::Supersedes, from_id, to_id)
}

/// The chain through `decision_id`, or an error when it forks. Callers that can say what a fork
/// is use [`walk_supersession`].
pub fn get_supersession_chain(
    graph: &impl GraphView,
    decision_id: &str,
) -> Result<QueryResponse<SupersessionChain>> {
    let line = walk_supersession(graph, decision_id)?;
    if let Some(fork) = line.data.forks.first() {
        return Err(query_error(format!(
            "supersession chain branched at {}: {} candidates",
            fork.at,
            fork.candidates.len()
        ))
        .into());
    }
    Ok(QueryResponse {
        result_count: line.result_count,
        truncated: line.truncated,
        latency_ms: line.latency_ms,
        data: line.data.chain,
    })
}

/// Walk from `start` along one side until the line ends or forks. Returns the decisions walked,
/// nearest first, and the fork that stopped the walk, if any.
fn walk_side(
    graph: &impl GraphView,
    start: &str,
    side: SupersessionSide,
    visited: &mut BTreeSet<String>,
) -> Result<(Vec<String>, Option<SupersessionFork>)> {
    let mut walked = Vec::new();
    let mut cursor = start.to_owned();
    loop {
        let mut neighbors = supersession_neighbors(graph, &cursor, side)?;
        if neighbors.len() > 1 {
            let fork = SupersessionFork {
                at: cursor,
                side,
                candidates: neighbors,
            };
            return Ok((walked, Some(fork)));
        }
        let Some(next) = neighbors.pop() else {
            return Ok((walked, None));
        };
        if !visited.insert(next.clone()) {
            return Err(cycle_error(side, &cursor, &next));
        }
        walked.push(next.clone());
        cursor = next;
    }
}

/// The error for a walk that reached a decision it had already visited: `from` is the decision
/// the walk stood on, `to` the one it stepped to.
fn cycle_error(side: SupersessionSide, from: &str, to: &str) -> HivemindError {
    let edge = match side {
        SupersessionSide::Older => format!("{from} -> {to}"),
        SupersessionSide::Newer => format!("{to} -> {from}"),
    };
    query_error(format!("cycle detected on edge {edge}")).into()
}

/// The distinct decisions on `side` of `decision_id`, in id order. A pair superseded twice (a
/// retried supersede after a client timeout) is two edges on a graph that keys edges by the
/// event that wrote them, but one neighbour.
fn supersession_neighbors(
    graph: &impl GraphView,
    decision_id: &str,
    side: SupersessionSide,
) -> Result<Vec<String>> {
    let cypher = match side {
        SupersessionSide::Older => {
            "MATCH (d:`Decision` {id: $id})-[:`SUPERSEDES`]->(other:`Decision`) RETURN other.id AS id ORDER BY other.id;"
        }
        SupersessionSide::Newer => {
            "MATCH (other:`Decision`)-[:`SUPERSEDES`]->(d:`Decision` {id: $id}) RETURN other.id AS id ORDER BY other.id;"
        }
    };
    let rows = graph.query(
        cypher,
        &GraphParams::from([("id".to_owned(), GraphValue::String(decision_id.to_owned()))]),
    )?;

    let mut ids = BTreeSet::new();
    for row in rows {
        ids.insert(required_string(&row, "id")?);
    }
    Ok(ids.into_iter().collect())
}
