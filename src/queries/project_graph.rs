//! One project's page of the decision graph: its decisions, newest first, and what hangs off them.
//!
//! `GET /v1/graph` answers for the whole tenant, which at tens of thousands of decisions is more
//! than a page of one project needs. This answers for one project and is bounded: a page holds at
//! most `limit` of its decisions, and a cursor names where the next, older page starts, so a
//! reader can walk the whole project and a short answer never reads as a complete one
//! (AGENTS.md §6). It selects nodes and edges; the caller shapes them.
//!
//! What hangs off a page's decisions:
//! - every edge that has a page decision at either end, whatever its relation, and the node at its
//!   other end (an actor, an option, evidence, a hypothesis, a question, a request, a blocker, or
//!   another decision, which may belong to another project);
//! - from an option reached that way, what it is premised on (`PREMISED_ON`);
//! - from evidence and hypotheses reached that way, one hop of `SUPPORTS` and `REFUTES`;
//! - a reaffirmation whose line ends at a node reached that way.
//!
//! Nodes at the far end are named and not followed: a decision of another project appears as a
//! node and shows no edges of its own beyond those to the page. Every edge in the page has both of
//! its ends in the page. Arrows run newer to older like every arrow (`docs/GRAPH_CONTRACT.md`).
//! No LLM, no ranking, no inference.

use std::collections::{BTreeMap, BTreeSet};

use crate::projector::arrow::Arrow;
use crate::projector::{GraphView, NodeKind, RelationKind};
use crate::Result;

use super::arrows::{reaffirmation_arrows_among, DerivedArrow, NodeTimes};
use super::shared::{node_rows, optional_int, optional_string, relation_edges_by_kind};

/// A decision's place in a project's order, oldest first: the ledger offset that recorded it, then
/// its id. As a cursor it names the oldest decision of the page just read, and the next page is
/// every decision older than it.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ProjectGraphCursor {
    origin: i64,
    decision_id: String,
}

impl ProjectGraphCursor {
    /// Reads a cursor [`ProjectGraphPage::next_cursor`] gave: `<offset>:<decision id>`. A decision
    /// id can hold colons, so only the first splits.
    pub fn parse(text: &str) -> Option<Self> {
        let (origin, decision_id) = text.split_once(':')?;
        if decision_id.is_empty() {
            return None;
        }
        Some(Self {
            origin: origin.parse().ok()?,
            decision_id: decision_id.to_owned(),
        })
    }

    fn encode(&self) -> String {
        format!("{}:{}", self.origin, self.decision_id)
    }
}

#[derive(Clone, Debug)]
pub struct ProjectGraphRequest {
    /// The address the project's decisions are filed under (see `project_address`).
    pub address: String,
    /// How many of the project's decisions the page holds, at least one.
    pub limit: usize,
    /// Start just older than this decision; `None` starts at the newest.
    pub cursor: Option<ProjectGraphCursor>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProjectGraphPage {
    /// Every decision the project holds, in this page or not.
    pub total_matches: usize,
    /// Where the next, older page starts; `None` when this page reaches the oldest decision.
    pub next_cursor: Option<String>,
    /// Every node of the page, by kind then id. The page's decisions are among the `Decision`s,
    /// with the far end of every edge that is a decision.
    pub nodes: Vec<(NodeKind, String)>,
    pub arrows: Vec<Arrow>,
    /// Lines drawn from a `Reaffirmation`'s own properties (see `reaffirmation_arrows`); the
    /// reaffirmation is among the nodes.
    pub derived: Vec<DerivedArrow>,
}

/// The nodes and stored edges a page has taken so far.
#[derive(Default)]
struct Selection {
    nodes: BTreeSet<(NodeKind, String)>,
    edges: BTreeMap<RelationKind, BTreeSet<(String, String)>>,
}

impl Selection {
    fn holds(&self, kind: NodeKind, id: &str) -> bool {
        self.nodes.contains(&(kind, id.to_owned()))
    }

    /// Takes the edge `from -[relation]-> to` and both its ends.
    fn take(&mut self, relation: RelationKind, from: &str, to: &str) {
        let (from_kind, to_kind) = relation.endpoints();
        self.nodes.insert((from_kind, from.to_owned()));
        self.nodes.insert((to_kind, to.to_owned()));
        self.edges
            .entry(relation)
            .or_default()
            .insert((from.to_owned(), to.to_owned()));
    }

    fn ids_of(&self, kind: NodeKind) -> BTreeSet<String> {
        self.nodes
            .iter()
            .filter(|(node_kind, _)| *node_kind == kind)
            .map(|(_, id)| id.clone())
            .collect()
    }
}

pub fn project_graph_page(
    graph: &impl GraphView,
    request: &ProjectGraphRequest,
) -> Result<ProjectGraphPage> {
    // The project's decisions oldest first. `total_matches` is all of them; the page is the
    // `limit` just older than the cursor.
    let mut keys: Vec<ProjectGraphCursor> = node_rows(graph, NodeKind::Decision)?
        .into_iter()
        .filter(|(_, row)| {
            optional_string(row, "project").as_deref() == Some(request.address.as_str())
        })
        .map(|(decision_id, row)| ProjectGraphCursor {
            origin: optional_int(&row, "event_origin").unwrap_or(i64::MIN),
            decision_id,
        })
        .collect();
    keys.sort();
    let total_matches = keys.len();
    let end = request.cursor.as_ref().map_or(total_matches, |cursor| {
        keys.partition_point(|key| key < cursor)
    });
    let start = end.saturating_sub(request.limit.max(1));
    let next_cursor = keys
        .get(start)
        .filter(|_| start > 0)
        .map(ProjectGraphCursor::encode);
    let page: BTreeSet<&str> = keys
        .get(start..end)
        .unwrap_or_default()
        .iter()
        .map(|key| key.decision_id.as_str())
        .collect();

    let edges = relation_edges_by_kind(graph)?;
    let mut selection = Selection::default();
    selection
        .nodes
        .extend(page.iter().map(|id| (NodeKind::Decision, (*id).to_owned())));

    // Every edge with a page decision at an end.
    for (relation, pairs) in &edges {
        let (from_kind, to_kind) = relation.endpoints();
        for (from, to) in pairs {
            let on_page = (from_kind == NodeKind::Decision && page.contains(from.as_str()))
                || (to_kind == NodeKind::Decision && page.contains(to.as_str()));
            if on_page {
                selection.take(*relation, from, to);
            }
        }
    }
    // What the options reached are premised on, then one hop from the evidence and hypotheses
    // held after that. The second picks its edges before taking any, so what it adds is not
    // followed in turn.
    for (from, to) in edges.get(&RelationKind::PremisedOn).into_iter().flatten() {
        if selection.holds(NodeKind::Option, from) {
            selection.take(RelationKind::PremisedOn, from, to);
        }
    }
    let followed: Vec<(RelationKind, &str, &str)> = [RelationKind::Supports, RelationKind::Refutes]
        .into_iter()
        .flat_map(|relation| {
            edges
                .get(&relation)
                .into_iter()
                .flatten()
                .map(move |(from, to)| (relation, from.as_str(), to.as_str()))
        })
        .filter(|(_, from, to)| {
            selection.holds(NodeKind::Evidence, from) || selection.holds(NodeKind::Hypothesis, to)
        })
        .collect();
    for (relation, from, to) in followed {
        selection.take(relation, from, to);
    }

    let derived = reaffirmation_arrows_among(
        graph,
        &selection.ids_of(NodeKind::Decision),
        &selection.ids_of(NodeKind::Hypothesis),
    )?;
    selection.nodes.extend(
        derived
            .iter()
            .map(|arrow| (NodeKind::Reaffirmation, arrow.from_id.clone())),
    );

    let mut times = NodeTimes::default();
    let mut arrows = Vec::new();
    for (relation, pairs) in &selection.edges {
        for (from, to) in pairs {
            arrows.push(times.arrow(graph, *relation, from, to)?);
        }
    }

    Ok(ProjectGraphPage {
        total_matches,
        next_cursor,
        nodes: selection.nodes.into_iter().collect(),
        arrows,
        derived,
    })
}

#[cfg(test)]
mod tests;
