//! What classified captures state that no fact event of the ledger says (hivemind-u46nq).
//!
//! A capture (`ingest.batch_classified`) can state a decision's standing inside itself:
//! `accepted_by`, `rejected_by`, `supersedes_id` and `premised_on_ids` on a decision, `refutes_ids`
//! on evidence. The projector turns each into an edge, but the capture is no `decision.accepted`,
//! `decision.rejected`, `decision.superseded` or `relation.added` event, so reading the fact events
//! alone drops them. This reads those edges from the graph and gives each the shape of the event it
//! stands for (a [`Step`]), so the one rule `ledger_facts` applies to events applies to them too:
//! a decision a capture supersedes tells the decisions that follow from it, and a hypothesis a
//! capture refutes tells the decisions that rested on it.
//!
//! Dating follows `status_events::capture_entries`. A decision's position is cited by the offset of
//! the decision that made the capture and dated at its recorded time (the source turn's time when
//! the capture names one, else the batch's). Evidence carries no time of its own, so a refutation
//! takes the classification event's own time and recorder, read from the event with its captures
//! left out. Nothing is guessed: an event that cannot be found gives `ts: null`.
//!
//! An edge a fact event already explains is not read a second time: the graph keeps one edge per
//! pair, so an acceptance, rejection, supersession or refutation the ledger holds as its own event
//! is that event's step and not a capture's.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};

use crate::events::{
    DecisionAcceptedPayload, DecisionProposedPayload, DecisionRejectedPayload,
    DecisionSupersededPayload, EventId, EventPayload, EventType, RelationAddedPayload,
    RelationKind as EventRelationKind,
};
use crate::ledger::EventLedger;
use crate::projector::{GraphView, NodeKind, RelationKind};
use crate::queries::brief::is_classified_capture;
use crate::queries::shared::{
    neighbor_pairs, node_rows, optional_int, optional_string, optional_string_list, query_error,
    relation_edges_with_origin, Direction,
};
use crate::queries::status_events::{cited, proposer_of, recorded_of};
use crate::Result;

use super::Step;

/// What the fact events already say, so the graph edges they wrote are not read again. The pairs
/// borrow from the steps, so asking whether an edge is stated allocates nothing.
#[derive(Default)]
struct Stated<'a> {
    /// `(decision, actor)` pairs that accepted.
    accepted: BTreeSet<(&'a str, &'a str)>,
    rejected: BTreeSet<(&'a str, &'a str)>,
    /// `(old, new)` pairs.
    superseded: BTreeSet<(&'a str, &'a str)>,
    /// `(evidence, hypothesis)` pairs.
    refuted: BTreeSet<(&'a str, &'a str)>,
}

impl<'a> Stated<'a> {
    fn of(steps: &'a [Step]) -> Self {
        let mut stated = Self::default();
        for step in steps {
            let actor = step.actor_id.as_deref().unwrap_or_default();
            match &step.payload {
                EventPayload::DecisionAccepted(accepted) => {
                    stated
                        .accepted
                        .insert((accepted.decision_id.as_str(), actor));
                }
                EventPayload::DecisionRejected(rejected) => {
                    stated
                        .rejected
                        .insert((rejected.decision_id.as_str(), actor));
                }
                EventPayload::DecisionSuperseded(superseded) => {
                    stated.superseded.insert((
                        superseded.old_decision_id.as_str(),
                        superseded.new_decision_id.as_str(),
                    ));
                }
                EventPayload::RelationAdded(relation)
                    if relation.relation == EventRelationKind::Refutes =>
                {
                    stated
                        .refuted
                        .insert((relation.from_id.as_str(), relation.to_id.as_str()));
                }
                _ => {}
            }
        }
        stated
    }
}

/// The steps classified captures state beyond `events`, in the order they were stated: premises
/// first, so a decision that premised on a hypothesis in the same capture that refutes it is told.
/// `accepted_of` names the one decision whose acceptances are wanted.
pub(super) fn steps(
    graph: &impl GraphView,
    ledger: &impl EventLedger,
    events: &[Step],
    accepted_of: Option<&str>,
) -> Result<Vec<Step>> {
    let stated = Stated::of(events);
    let refutes = relation_edges_with_origin(graph, RelationKind::Refutes)?;

    let mut steps = Vec::new();
    // A premise only matters once something it premises on is refuted.
    if !refutes.is_empty() {
        steps.extend(premises(graph)?);
    }
    if let Some(decision_id) = accepted_of {
        steps.extend(acceptances(graph, decision_id, &stated)?);
    }
    steps.extend(rejections(graph, &stated)?);
    steps.extend(supersessions(graph, &stated)?);
    steps.extend(refutations(ledger, refutes, &stated)?);
    Ok(steps)
}

/// The rejections and replacements captures state beyond `events`: the two standings a feed lists
/// as a change of their own. Reads the graph and nothing else, so a feed that holds the events it
/// lists pays for no ledger read.
pub(super) fn standings(graph: &impl GraphView, events: &[Step]) -> Result<Vec<Step>> {
    let stated = Stated::of(events);
    let mut steps = rejections(graph, &stated)?;
    steps.extend(supersessions(graph, &stated)?);
    Ok(steps)
}

/// The decisions classified captures record, each as the `decision.proposed` it stands for, cited
/// by the classification event that recorded it (hivemind-svqxz). A capture's decision has no
/// proposal in the ledger, so a feed that lists events alone met its classification as a context
/// change naming no decision, and the decision read as unchanged since before it existed. The
/// payload holds what the node states of the proposal (title, rationale, topics, project) and
/// nothing a proposal would carry that the node does not. Reads the graph and nothing else.
pub(super) fn creations(graph: &impl GraphView) -> Result<Vec<Step>> {
    let mut steps = Vec::new();
    for (decision_id, row) in node_rows(graph, NodeKind::Decision)? {
        if !is_classified_capture(&decision_id) {
            continue;
        }
        let origin = offset_of(optional_int(&row, "event_origin"), "a drafted decision")?;
        steps.push(Step {
            origin,
            ts: None,
            actor_id: None,
            relayed_by: None,
            payload: EventPayload::DecisionProposed(DecisionProposedPayload {
                decision_id,
                title: optional_string(&row, "title").unwrap_or_default(),
                rationale: optional_string(&row, "rationale").unwrap_or_default(),
                topic_keys: optional_string_list(&row, "topic_keys"),
                option_ids: Vec::new(),
                option_labels: Vec::new(),
                option_descriptions: Vec::new(),
                chosen_option_id: None,
                hypothesis_ids: Vec::new(),
                evidence_ids: Vec::new(),
                expressed_confidence: None,
                quote: None,
                question: None,
                project: optional_string(&row, "project"),
                project_source: None,
                binds: None,
                undo_cost: None,
            }),
        });
    }
    // One classification's captures in the order it listed them: ids sort as text, so capture 10
    // would otherwise come before capture 2.
    steps.sort_by_key(|step| (step.origin, capture_index(&step.payload)));
    Ok(steps)
}

/// The position of a drafted decision in its classification (`capture:<event>:<index>`).
fn capture_index(payload: &EventPayload) -> Option<usize> {
    let EventPayload::DecisionProposed(proposed) = payload else {
        return None;
    };
    proposed.decision_id.rsplit(':').next()?.parse().ok()
}

/// The decisions a capture premises on a hypothesis, directly or through the option it chose.
fn premises(graph: &impl GraphView) -> Result<Vec<Step>> {
    let mut steps = Vec::new();
    for (decision_id, hypothesis_id, origin) in
        relation_edges_with_origin(graph, RelationKind::PremisedOnDirect)?
    {
        if is_classified_capture(&decision_id) {
            steps.push(relation_step(
                EventRelationKind::Assumes,
                decision_id,
                hypothesis_id,
                offset_of(origin, "a premise")?,
            ));
        }
    }
    for (option_id, hypothesis_id, origin) in
        relation_edges_with_origin(graph, RelationKind::PremisedOn)?
    {
        if !is_classified_capture(&option_id) {
            continue;
        }
        let origin = offset_of(origin, "a premise")?;
        for (decision_id, _) in neighbor_pairs(
            graph,
            NodeKind::Option,
            &option_id,
            RelationKind::Chose,
            NodeKind::Decision,
            Direction::Incoming,
        )? {
            steps.push(relation_step(
                EventRelationKind::Chose,
                decision_id,
                option_id.clone(), // ubs:ignore: one per decision that chose the option
                origin,
            ));
        }
        steps.push(relation_step(
            EventRelationKind::Assumes,
            option_id,
            hypothesis_id,
            origin,
        ));
    }
    Ok(steps)
}

fn relation_step(
    relation: EventRelationKind,
    from_id: String,
    to_id: String,
    origin: EventId,
) -> Step {
    Step {
        origin,
        ts: None,
        actor_id: None,
        relayed_by: None,
        payload: EventPayload::RelationAdded(RelationAddedPayload {
            relation,
            from_id,
            to_id,
        }),
    }
}

/// The acceptors a capture named for `decision_id`, at the time and offset it was recorded.
fn acceptances(
    graph: &impl GraphView,
    decision_id: &str,
    stated: &Stated<'_>,
) -> Result<Vec<Step>> {
    let recorded = recorded_of(graph, decision_id)?;
    let mut steps = Vec::new();
    for (actor_id, _) in neighbor_pairs(
        graph,
        NodeKind::Decision,
        decision_id,
        RelationKind::AcceptedBy,
        NodeKind::Actor,
        Direction::Outgoing,
    )? {
        if stated.accepted.contains(&(decision_id, actor_id.as_str())) {
            continue;
        }
        steps.push(Step {
            origin: cited(recorded.offset, decision_id)?,
            ts: recorded.at,
            actor_id: Some(actor_id),
            relayed_by: None,
            payload: EventPayload::DecisionAccepted(DecisionAcceptedPayload {
                decision_id: decision_id.to_owned(), // ubs:ignore: each step owns its decision id
                delegated_by: None,
            }),
        });
    }
    Ok(steps)
}

/// The decisions a capture rejected, at the time and offset they were recorded.
fn rejections(graph: &impl GraphView, stated: &Stated<'_>) -> Result<Vec<Step>> {
    let mut steps = Vec::new();
    for (decision_id, actor_id, edge_origin) in
        relation_edges_with_origin(graph, RelationKind::RejectedBy)?
    {
        if stated
            .rejected
            .contains(&(decision_id.as_str(), actor_id.as_str()))
        {
            continue;
        }
        let recorded = recorded_of(graph, &decision_id)?;
        steps.push(Step {
            origin: cited(
                recorded.offset.or_else(|| edge_offset(edge_origin)),
                &decision_id,
            )?,
            ts: recorded.at,
            actor_id: Some(actor_id),
            relayed_by: None,
            payload: EventPayload::DecisionRejected(DecisionRejectedPayload {
                decision_id,
                reason: None,
                said_in: None,
            }),
        });
    }
    Ok(steps)
}

/// The decisions a capture replaces, at the time and offset the replacing decision was recorded
/// and by whoever proposed it.
fn supersessions(graph: &impl GraphView, stated: &Stated<'_>) -> Result<Vec<Step>> {
    let mut steps = Vec::new();
    // `SUPERSEDES` runs newer decision -> older decision.
    for (newer_id, older_id, edge_origin) in
        relation_edges_with_origin(graph, RelationKind::Supersedes)?
    {
        if newer_id == older_id
            || stated
                .superseded
                .contains(&(older_id.as_str(), newer_id.as_str()))
        {
            continue;
        }
        let newer = recorded_of(graph, &newer_id)?;
        steps.push(Step {
            origin: cited(newer.offset.or_else(|| edge_offset(edge_origin)), &newer_id)?,
            ts: newer.at,
            actor_id: proposer_of(graph, &newer_id)?,
            relayed_by: None,
            payload: EventPayload::DecisionSuperseded(DecisionSupersededPayload {
                old_decision_id: older_id,
                new_decision_id: newer_id,
            }),
        });
    }
    Ok(steps)
}

/// The hypotheses an evidence capture refutes, at the time and by the recorder of the
/// classification event that wrote the edge.
fn refutations(
    ledger: &impl EventLedger,
    refutes: Vec<(String, String, Option<i64>)>,
    stated: &Stated<'_>,
) -> Result<Vec<Step>> {
    let mut steps = Vec::new();
    let mut classifications: BTreeMap<EventId, (Option<DateTime<Utc>>, Option<String>)> =
        BTreeMap::new();
    for (evidence_id, hypothesis_id, edge_origin) in refutes {
        if stated
            .refuted
            .contains(&(evidence_id.as_str(), hypothesis_id.as_str()))
        {
            continue;
        }
        let origin = offset_of(edge_origin, "a refutation")?;
        let classification = match classifications.entry(origin) {
            Entry::Occupied(found) => found.into_mut(),
            Entry::Vacant(slot) => slot.insert(classification_of(ledger, origin)?),
        };
        let (ts, actor_id) = classification.clone(); // ubs:ignore: each step owns its recorder
        steps.push(Step {
            origin,
            ts,
            actor_id,
            relayed_by: None,
            payload: EventPayload::RelationAdded(RelationAddedPayload {
                relation: EventRelationKind::Refutes,
                from_id: evidence_id,
                to_id: hypothesis_id,
            }),
        });
    }
    Ok(steps)
}

/// The time and recorder of the classification event at `origin`, read with its (large) captures
/// left out. Absent when the ledger holds no such event.
fn classification_of(
    ledger: &impl EventLedger,
    origin: EventId,
) -> Result<(Option<DateTime<Utc>>, Option<String>)> {
    let found = ledger.read_types_for_tenant(
        &ledger.bound_tenant(),
        &[EventType::IngestBatchClassified],
        &["captures"],
        origin.saturating_sub(1),
        1,
    )?;
    Ok(found
        .into_iter()
        .find(|event| event.event_id == Some(origin))
        .map_or((None, None), |event| (event.ts, Some(event.actor_id))))
}

fn edge_offset(edge_origin: Option<i64>) -> Option<EventId> {
    edge_origin.and_then(|origin| EventId::try_from(origin).ok())
}

/// The offset of the event that wrote an edge. The projector gives every edge one; an edge with
/// none cannot be placed in ledger order, so it is refused rather than listed without a citation.
fn offset_of(edge_origin: Option<i64>, what: &str) -> Result<EventId> {
    edge_offset(edge_origin).ok_or_else(|| {
        query_error(format!("{what} the graph holds carries no ledger offset")).into()
    })
}
