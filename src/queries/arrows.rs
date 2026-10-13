//! Oriented edges: the read side of `projector::arrow`. Every arrow a reader is shown runs from
//! the newer node to the older node; the graph itself keeps each relation in its stored,
//! semantic direction. See `docs/GRAPH_CONTRACT.md`.

use std::collections::{BTreeMap, BTreeSet};

use crate::projector::arrow::{orient, Arrow, NodeTime};
use crate::projector::{GraphRow, GraphView, NodeKind, RelationKind};
use crate::Result;

use super::shared::{node_row, node_rows, optional_int, optional_string, relation_edges};

/// Recorded times of nodes (the `event_origin` that created each), fetched on demand and
/// remembered for the rest of one read.
#[derive(Default)]
pub(crate) struct NodeTimes {
    times: BTreeMap<NodeKind, BTreeMap<String, NodeTime>>,
    /// Kinds whose every node was loaded in one query, so a miss is a real absence.
    loaded: BTreeSet<NodeKind>,
}

impl NodeTimes {
    /// Loads every node of `kind` in one query. For a read that walks a whole relation.
    fn load_kind(&mut self, graph: &impl GraphView, kind: NodeKind) -> Result<()> {
        if !self.loaded.insert(kind) {
            return Ok(());
        }
        let times = self.times.entry(kind).or_default();
        for (id, row) in node_rows(graph, kind)? {
            times.insert(id, optional_int(&row, "event_origin"));
        }
        Ok(())
    }

    /// One node's recorded time; `None` when the node is missing or carries none.
    fn time_of(&mut self, graph: &impl GraphView, kind: NodeKind, id: &str) -> Result<NodeTime> {
        if let Some(time) = self.times.get(&kind).and_then(|times| times.get(id)) {
            return Ok(*time);
        }
        if self.loaded.contains(&kind) {
            return Ok(None);
        }
        let time = node_row(graph, kind, id)?.and_then(|row| optional_int(&row, "event_origin"));
        self.times
            .entry(kind)
            .or_default()
            .insert(id.to_owned(), time);
        Ok(time)
    }

    /// Orients one stored edge `stored_from -[relation]-> stored_to` newer → older.
    pub(crate) fn arrow(
        &mut self,
        graph: &impl GraphView,
        relation: RelationKind,
        stored_from: &str,
        stored_to: &str,
    ) -> Result<Arrow> {
        // A relation that can never reverse (its target is an actor) needs no lookups.
        if relation.arrow_labels().reversed.is_none() {
            return Ok(orient(relation, stored_from, None, stored_to, None));
        }
        let (from_kind, to_kind) = relation.endpoints();
        let from_time = self.time_of(graph, from_kind, stored_from)?;
        let to_time = self.time_of(graph, to_kind, stored_to)?;
        Ok(orient(relation, stored_from, from_time, stored_to, to_time))
    }
}

/// Every edge of the graph as an arrow, newer → older. Relations come in
/// [`RelationKind::ALL`] order and, within one relation, in stored (source id, target id)
/// order, so the result is deterministic.
pub fn oriented_edges(graph: &impl GraphView) -> Result<Vec<Arrow>> {
    let mut times = NodeTimes::default();
    let mut arrows = Vec::new();
    for relation in RelationKind::ALL {
        let edges = relation_edges(graph, relation)?;
        if edges.is_empty() {
            continue;
        }
        if relation.arrow_labels().reversed.is_some() {
            let (from_kind, to_kind) = relation.endpoints();
            times.load_kind(graph, from_kind)?;
            times.load_kind(graph, to_kind)?;
        }
        for (from, to) in edges {
            arrows.push(times.arrow(graph, relation, &from, &to)?);
        }
    }
    Ok(arrows)
}

/// A line a graph view draws that the graph does not store as an edge. Drawn from the newer node
/// to the older one like every arrow (`docs/GRAPH_CONTRACT.md`); a reader that draws it says it is
/// derived, so no one mistakes it for a relation the ledger recorded.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct DerivedArrow {
    /// What the line means, in the same upper-case spelling the stored relations use.
    pub relation: &'static str,
    /// The line read along the arrow, an active phrase.
    pub label: &'static str,
    pub from_kind: NodeKind,
    pub from_id: String,
    pub to_kind: NodeKind,
    pub to_id: String,
}

/// The decision a `Reaffirmation` is about.
pub const REAFFIRMS: &str = "REAFFIRMS";
/// The premise whose change a `Reaffirmation` answers.
pub const REAFFIRMED_AFTER: &str = "REAFFIRMED_AFTER";

/// The lines that tie each `Reaffirmation` node to the decision it concerns and the premise that
/// changed. The node has no edges in the graph, deliberately (`docs/GRAPH_CONTRACT.md`: a second
/// reaffirmation of one pair would collapse into the first on Postgres and Kuzu), so a view that
/// draws edges would show it alone. These two lines are read from its own properties, never
/// stored, and a line is left out when the node it names is not in the graph. A reaffirmation is
/// always newer than both ends (the command refuses one whose ends do not exist yet), so every
/// line runs from it.
pub fn reaffirmation_arrows(graph: &impl GraphView) -> Result<Vec<DerivedArrow>> {
    let reaffirmations = node_rows(graph, NodeKind::Reaffirmation)?;
    if reaffirmations.is_empty() {
        return Ok(Vec::new());
    }
    let decisions: BTreeSet<String> = node_rows(graph, NodeKind::Decision)?.into_keys().collect();
    let hypotheses: BTreeSet<String> = node_rows(graph, NodeKind::Hypothesis)?
        .into_keys()
        .collect();

    Ok(reaffirmations
        .iter()
        .flat_map(|(id, row)| arrows_of_reaffirmation(id, row, &decisions, &hypotheses))
        .collect())
}

/// The arrows one `Reaffirmation` node derives: `REAFFIRMS` to the decision it was recorded
/// on and `REAFFIRMED_AFTER` to the premise whose change it answered. An endpoint that is not a
/// node in the graph (a premise outside the replayed ledger) gets no arrow.
fn arrows_of_reaffirmation(
    id: &str,
    row: &GraphRow,
    decisions: &BTreeSet<String>,
    hypotheses: &BTreeSet<String>,
) -> Vec<DerivedArrow> {
    let mut arrows = Vec::with_capacity(2);
    if let Some(decision_id) = optional_string(row, "decision_id") {
        if decisions.contains(&decision_id) {
            arrows.push(DerivedArrow {
                relation: REAFFIRMS,
                label: "reaffirms",
                from_kind: NodeKind::Reaffirmation,
                from_id: id.to_owned(),
                to_kind: NodeKind::Decision,
                to_id: decision_id,
            });
        }
    }
    if let Some(premise_id) = optional_string(row, "premise_id") {
        let to_kind = if decisions.contains(&premise_id) {
            Some(NodeKind::Decision)
        } else if hypotheses.contains(&premise_id) {
            Some(NodeKind::Hypothesis)
        } else {
            None
        };
        if let Some(to_kind) = to_kind {
            arrows.push(DerivedArrow {
                relation: REAFFIRMED_AFTER,
                label: "after the change to",
                from_kind: NodeKind::Reaffirmation,
                from_id: id.to_owned(),
                to_kind,
                to_id: premise_id,
            });
        }
    }
    arrows
}

#[cfg(test)]
mod tests;
