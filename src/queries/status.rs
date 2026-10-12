//! Decision status derivation: computes proposed/accepted/contested/superseded from graph edges.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::projector::{actor_kind, GraphView, NodeKind, RelationKind};
use crate::Result;

use super::shared::{relation_count, relation_edges_with_origin, Direction};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DecisionStatus {
    Proposed,
    Accepted,
    Rejected,
    Contested,
    Superseded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HypothesisStatus {
    Open,
    Supported,
    Refuted,
}

/// The one status rule. A decision anyone has superseded is `superseded` whatever else was said
/// about it; otherwise accepted and rejected positions together make it `contested`, never one
/// side winning silently. Shared by [`derive_decision_status`] (one decision) and
/// [`DecisionStandings`] (every decision at once) so the two cannot drift. The status events read
/// (`status_events`) replays it after each event for the same reason.
pub(super) fn status_from_positions(
    superseded: bool,
    accepted: bool,
    rejected: bool,
) -> DecisionStatus {
    if superseded {
        return DecisionStatus::Superseded;
    }
    match (accepted, rejected) {
        (true, true) => DecisionStatus::Contested,
        (true, false) => DecisionStatus::Accepted,
        (false, true) => DecisionStatus::Rejected,
        (false, false) => DecisionStatus::Proposed,
    }
}

pub fn derive_decision_status(graph: &impl GraphView, decision_id: &str) -> Result<DecisionStatus> {
    let count = |relation, direction| {
        relation_count(graph, relation, direction, NodeKind::Decision, decision_id)
    };
    Ok(status_from_positions(
        count(RelationKind::Supersedes, Direction::Incoming)? > 0,
        count(RelationKind::AcceptedBy, Direction::Outgoing)? > 0,
        count(RelationKind::RejectedBy, Direction::Outgoing)? > 0,
    ))
}

/// An actor who accepted a decision, with whether they are a human or an agent.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Decider {
    pub id: String,
    /// `human`, `agent` or `unknown`, read from the actor-id prefix (`human:` / `agent:`): the
    /// same rule that sets `kind` on the Actor node.
    pub kind: &'static str,
}

/// Every decision's status and deciders, read in three bulk edge scans (`SUPERSEDES`,
/// `ACCEPTED_BY`, `REJECTED_BY`) instead of three lookups per decision. For readers that show
/// the whole graph. A decision no edge names is `proposed` with no decider.
#[derive(Debug, Default)]
pub struct DecisionStandings {
    /// Each superseded decision with the decisions that superseded it, sorted by id.
    superseded_by: BTreeMap<String, Vec<String>>,
    accepted_by: BTreeMap<String, Vec<String>>,
    /// Each rejected decision with the actors who rejected it, sorted by actor id.
    rejected_by: BTreeMap<String, Vec<String>>,
    /// The superseding decision whose supersession was recorded first, with that event's ledger
    /// offset (`None` when the edge states none). Ties on the offset go to the lower id.
    earliest_superseder: BTreeMap<String, (String, Option<i64>)>,
    /// The ledger offset of the earliest acceptance and of the earliest rejection (`None` when
    /// the edges state none).
    first_accepted_at: BTreeMap<String, Option<i64>>,
    first_rejected_at: BTreeMap<String, Option<i64>>,
}

impl DecisionStandings {
    pub fn load(graph: &impl GraphView) -> Result<Self> {
        let mut standings = Self::default();
        // `SUPERSEDES` runs newer decision -> older decision, so the target is the superseded one.
        // Edges arrive sorted by (newer, older), so each decision's superseders are sorted by id.
        // The same supersession asserted twice is one.
        for (newer, superseded, origin) in
            relation_edges_with_origin(graph, RelationKind::Supersedes)?
        {
            note_earliest_superseder(
                &mut standings.earliest_superseder,
                &superseded,
                &newer,
                origin,
            );
            let superseders = standings.superseded_by.entry(superseded).or_default();
            if superseders.last() != Some(&newer) {
                superseders.push(newer);
            }
        }
        // Edges arrive sorted by (decision, actor), so each decision's actors are sorted by id.
        for (decision_id, actor_id, origin) in
            relation_edges_with_origin(graph, RelationKind::AcceptedBy)?
        {
            note_first_origin(&mut standings.first_accepted_at, &decision_id, origin);
            standings
                .accepted_by
                .entry(decision_id)
                .or_default()
                .push(actor_id);
        }
        for (decision_id, actor_id, origin) in
            relation_edges_with_origin(graph, RelationKind::RejectedBy)?
        {
            note_first_origin(&mut standings.first_rejected_at, &decision_id, origin);
            standings
                .rejected_by
                .entry(decision_id)
                .or_default()
                .push(actor_id);
        }
        Ok(standings)
    }

    /// The status [`derive_decision_status`] gives this decision.
    pub fn status_of(&self, decision_id: &str) -> DecisionStatus {
        status_from_positions(
            self.superseded_by.contains_key(decision_id),
            self.accepted_by.contains_key(decision_id),
            self.rejected_by.contains_key(decision_id),
        )
    }

    /// The decisions that superseded this one, sorted by id; empty when nobody has. More than one
    /// means concurrent supersessions, which both stand.
    pub fn superseders_of(&self, decision_id: &str) -> &[String] {
        self.superseded_by
            .get(decision_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Who rejected this decision, sorted by actor id; empty when nobody has.
    pub fn rejecters_of(&self, decision_id: &str) -> &[String] {
        self.rejected_by
            .get(decision_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// What this decision's position edges state, with their offsets. The bulk twin of
    /// [`StandingSummary::from_edges`].
    pub(crate) fn summary_of(&self, decision_id: &str) -> StandingSummary {
        StandingSummary {
            superseders: self.superseders_of(decision_id).to_vec(),
            earliest_superseder: self.earliest_superseder.get(decision_id).cloned(),
            accepted: self.accepted_by.contains_key(decision_id),
            first_accepted_at: self.first_accepted_at.get(decision_id).copied().flatten(),
            rejecters: self.rejecters_of(decision_id).to_vec(),
            first_rejected_at: self.first_rejected_at.get(decision_id).copied().flatten(),
        }
    }

    /// Who accepted this decision, sorted by actor id; empty when nobody has.
    pub fn deciders_of(&self, decision_id: &str) -> Vec<Decider> {
        self.accepted_by
            .get(decision_id)
            .into_iter()
            .flatten()
            .map(|actor_id| Decider {
                id: actor_id.clone(), // ubs:ignore: the answer owns its ids; one clone per decider
                kind: actor_kind(actor_id),
            })
            .collect()
    }
}

/// Keep the superseder with the lowest offset for `superseded`; an unknown offset loses to a known
/// one, and ties go to the lower id (the edges arrive sorted by id, so the first of a tie stays).
fn note_earliest_superseder(
    map: &mut BTreeMap<String, (String, Option<i64>)>,
    superseded: &str,
    superseder: &str,
    origin: Option<i64>,
) {
    match map.get_mut(superseded) {
        None => {
            map.insert(superseded.to_owned(), (superseder.to_owned(), origin));
        }
        Some(existing) => {
            if origin.unwrap_or(i64::MAX) < existing.1.unwrap_or(i64::MAX) {
                *existing = (superseder.to_owned(), origin);
            }
        }
    }
}

/// Keep the lowest offset seen for `key`; an edge with no offset leaves an unknown in place only
/// until an edge that states one is seen.
fn note_first_origin(map: &mut BTreeMap<String, Option<i64>>, key: &str, origin: Option<i64>) {
    let entry = map.entry(key.to_owned()).or_insert(origin);
    *entry = match (*entry, origin) {
        (Some(known), Some(other)) => Some(known.min(other)),
        (known, other) => known.or(other),
    };
}

/// What one decision's position edges state, with the ledger offsets they were taken at: who
/// superseded it (and which supersession came first), whether anyone accepted it, who rejected
/// it. The inputs of the re-examine rules, read either in bulk ([`DecisionStandings::summary_of`])
/// or anchored on one decision ([`StandingSummary::from_edges`]); both give the same summary for
/// the same graph, so the status and the change offsets cannot drift between the two readers.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct StandingSummary {
    /// The decisions that superseded it, sorted by id.
    pub superseders: Vec<String>,
    /// The superseder whose supersession was recorded first, with that event's offset.
    pub earliest_superseder: Option<(String, Option<i64>)>,
    pub accepted: bool,
    /// The offset of the earliest acceptance, when an edge states one.
    pub first_accepted_at: Option<i64>,
    /// The actors who rejected it, sorted by id.
    pub rejecters: Vec<String>,
    /// The offset of the earliest rejection, when an edge states one.
    pub first_rejected_at: Option<i64>,
}

impl StandingSummary {
    /// The summary of edges read anchored on one decision: `(superseding decision, offset)`,
    /// `offset` of each acceptance, `(rejecting actor, offset)`. A pair asserted more than once
    /// counts once.
    pub(crate) fn from_edges(
        superseders: &[(String, Option<i64>)],
        accepted: &[Option<i64>],
        rejected: &[(String, Option<i64>)],
    ) -> Self {
        let mut summary = Self::default();
        let mut superseder_ids: Vec<&String> = superseders.iter().map(|(id, _)| id).collect();
        superseder_ids.sort();
        superseder_ids.dedup();
        summary.superseders = superseder_ids.into_iter().cloned().collect();
        // The first of the earliest offsets wins; an unknown offset sorts after every known one.
        summary.earliest_superseder = superseders
            .iter()
            .min_by_key(|(_, origin)| origin.unwrap_or(i64::MAX))
            .map(|(superseder, origin)| (superseder.clone(), *origin));
        summary.accepted = !accepted.is_empty();
        summary.first_accepted_at = accepted.iter().flatten().min().copied();
        let mut rejecter_ids: Vec<&String> = rejected.iter().map(|(id, _)| id).collect();
        rejecter_ids.sort();
        rejecter_ids.dedup();
        summary.rejecters = rejecter_ids.into_iter().cloned().collect();
        summary.first_rejected_at = rejected.iter().filter_map(|(_, origin)| *origin).min();
        summary
    }

    /// The status the position edges give: the one rule [`status_from_positions`].
    pub(crate) fn status(&self) -> DecisionStatus {
        status_from_positions(
            !self.superseders.is_empty(),
            self.accepted,
            !self.rejecters.is_empty(),
        )
    }

    /// The offset at which a contested decision became contested: when its second side first
    /// spoke, so the later of the first acceptance and the first rejection. `None` when either
    /// side states no offset.
    pub(crate) fn contested_at(&self) -> Option<i64> {
        Some(self.first_accepted_at?.max(self.first_rejected_at?))
    }
}

/// The one hypothesis status rule: evidence that refutes it beats evidence that supports it, and
/// with neither it is open. Shared by [`derive_hypothesis_status`] (one hypothesis) and the bulk
/// grounding read so the two cannot drift.
pub(super) fn hypothesis_status_from_evidence(refuted: bool, supported: bool) -> HypothesisStatus {
    if refuted {
        HypothesisStatus::Refuted
    } else if supported {
        HypothesisStatus::Supported
    } else {
        HypothesisStatus::Open
    }
}

pub fn derive_hypothesis_status(
    graph: &impl GraphView,
    hypothesis_id: &str,
) -> Result<HypothesisStatus> {
    let refuted = relation_count(
        graph,
        RelationKind::Refutes,
        Direction::Incoming,
        NodeKind::Hypothesis,
        hypothesis_id,
    )? > 0;
    // Only a hypothesis nothing refutes needs the second read.
    let supported = !refuted
        && relation_count(
            graph,
            RelationKind::Supports,
            Direction::Incoming,
            NodeKind::Hypothesis,
            hypothesis_id,
        )? > 0;
    Ok(hypothesis_status_from_evidence(refuted, supported))
}
