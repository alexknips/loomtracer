// Parent module gates this file with #[cfg(test)]; repeat the marker so UBS can filter test-only assertions.
#[cfg(test)]
use chrono::{DateTime, Utc};
use serde_json::json;
use uuid::Uuid;

use crate::events::{Event, EventSource, EventType};
use crate::ledger::{EventLedger, InMemoryEventLedger};
use crate::projector::{memory::MemoryGraph, rebuild_graph};
use crate::queries::{get_failure_attribution, AttributionGroup, FailureAttributionRequest};
use crate::Result;

use super::*;

const AGENT: &str = "agent:claude:relay";
const PAT: &str = "human:pat@example.com";
const SAM: &str = "human:sam@example.com";
const REASON: &str = "I never chose Friday; the relay took the default as my answer";

fn graph_from_events(events: impl IntoIterator<Item = Event>) -> Result<MemoryGraph> {
    let ledger = InMemoryEventLedger::new();
    for event in events {
        ledger.append(event)?;
    }
    let graph = MemoryGraph::default();
    rebuild_graph(&ledger, &graph)?;
    Ok(graph)
}

fn event(
    sequence: u128,
    event_type: EventType,
    actor_id: &str,
    payload: serde_json::Value,
) -> Event {
    Event {
        tenant_id: Default::default(),
        event_id: None,
        event_uuid: Uuid::from_u128(sequence),
        correlation_id: Some("context-test".to_owned()),
        causation_event_id: None,
        event_type,
        actor_id: actor_id.to_owned(),
        source: EventSource::Cli,
        source_ref: None,
        payload,
        ts: Some(ts("2026-01-01T00:00:00Z")),
    }
}

fn ts(timestamp: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(timestamp)
        .expect("test timestamp parses")
        .with_timezone(&Utc)
}

/// The agent proposes `decision_id` and each of `acceptors` accepts it. `base` keeps the event
/// uuids of one decision apart from another's.
fn proposed_and_accepted(base: u128, decision_id: &str, acceptors: &[&str]) -> Vec<Event> {
    let mut events = vec![event(
        base,
        EventType::DecisionProposed,
        AGENT,
        json!({
            "decision_id": decision_id,
            "title": format!("Decision {decision_id}"),
            "rationale": "Rationale text long enough for the readable floor",
            "topic_keys": ["release"],
            "option_ids": [],
            "chosen_option_id": null,
            "hypothesis_ids": [],
            "evidence_ids": []
        }),
    )];
    for (offset, acceptor) in acceptors.iter().enumerate() {
        events.push(event(
            base + 1 + offset as u128,
            EventType::DecisionAccepted,
            acceptor,
            json!({ "decision_id": decision_id }),
        ));
    }
    events
}

fn dispute(sequence: u128, decision_id: &str, actor_id: &str) -> Event {
    event(
        sequence,
        EventType::DecisionAttributionDisputed,
        actor_id,
        json!({ "decision_id": decision_id, "reason": REASON }),
    )
}

/// A dispute the person's agent recorded for them (hivemind-f9xgu): the event is the person's, the
/// source names the agent as the one who relayed it, and `said_in` says where they said it.
fn relayed_dispute(
    sequence: u128,
    decision_id: &str,
    actor_id: &str,
    relayer: &str,
    said_in: &str,
) -> Event {
    let mut relayed = event(
        sequence,
        EventType::DecisionAttributionDisputed,
        actor_id,
        json!({ "decision_id": decision_id, "reason": REASON, "said_in": said_in }),
    );
    relayed.source = EventSource::Agent;
    relayed.source_ref = Some(relayer.to_owned());
    relayed
}

fn context_of(graph: &MemoryGraph, decision_id: &str) -> DecisionContext {
    get_decision_context(graph, decision_id)
        .expect("context query")
        .data
        .expect("decision exists")
}

fn group<'a>(groups: &'a [AttributionGroup], label: &str) -> Option<&'a AttributionGroup> {
    groups.iter().find(|group| group.group_label == label)
}

#[test]
fn a_dispute_shows_in_the_context_beside_the_acceptance_and_the_recorded_shapes_stay() -> Result<()>
{
    let mut events = proposed_and_accepted(10, "d:disputed", &[PAT]);
    events.extend(proposed_and_accepted(20, "d:control", &[PAT]));
    events.push(dispute(30, "d:disputed", PAT));
    let graph = graph_from_events(events)?;

    let disputed = context_of(&graph, "d:disputed");
    assert_eq!(disputed.accepted_by, vec![PAT.to_owned()]);
    assert_eq!(
        disputed.authorship,
        AuthorshipShape::AgentProposedHumanAccepted
    );
    assert_eq!(disputed.review, ReviewShape::PeerReviewed);
    assert_eq!(
        disputed.attribution_disputes,
        vec![AttributionDispute {
            disputed_by: PAT.to_owned(),
            reason: REASON.to_owned(),
            relayed_by: None,
            said_in: None,
            disputed_at: Some(ts("2026-01-01T00:00:00Z")),
        }]
    );

    // The control, accepted the same way and never disputed, reads exactly as it did.
    let control = context_of(&graph, "d:control");
    assert!(control.attribution_disputes.is_empty());
    assert!(
        serde_json::to_value(&control)
            .expect("context serializes")
            .get("attribution_disputes")
            .is_none(),
        "an undisputed decision's JSON has no dispute field"
    );
    let disputed_json = serde_json::to_value(&disputed).expect("context serializes");
    assert_eq!(disputed_json["attribution_disputes"][0]["disputed_by"], PAT);
    assert_eq!(disputed_json["attribution_disputes"][0]["reason"], REASON);
    assert!(
        disputed_json.get("human_acceptance_disputed").is_none()
            && disputed_json.get("peer_review_disputed").is_none(),
        "the grouping flags are not part of the answer"
    );
    Ok(())
}

#[test]
fn the_bulk_context_read_carries_the_dispute_of_the_decision_it_names() -> Result<()> {
    let mut events = proposed_and_accepted(10, "d:disputed", &[PAT]);
    events.extend(proposed_and_accepted(20, "d:control", &[PAT]));
    events.push(dispute(30, "d:disputed", PAT));
    let graph = graph_from_events(events)?;

    let contexts = get_decision_context_candidates(
        &graph,
        &DecisionContextRequest {
            since_event_origin: None,
            limit: 10,
            cursor: None,
        },
    )?
    .data;
    let by_id = |id: &str| contexts.iter().find(|context| context.decision_id == id);
    assert_eq!(
        by_id("d:disputed")
            .expect("disputed listed")
            .attribution_disputes
            .len(),
        1
    );
    assert!(by_id("d:control")
        .expect("control listed")
        .attribution_disputes
        .is_empty());
    Ok(())
}

#[test]
fn a_dispute_the_persons_agent_relayed_names_the_relay_and_where_it_was_said_like_why_does(
) -> Result<()> {
    let mut events = proposed_and_accepted(10, "d:relayed", &[PAT]);
    events.extend(proposed_and_accepted(20, "d:own-credential", &[PAT]));
    events.extend(proposed_and_accepted(30, "d:self-sourced", &[PAT]));
    events.push(relayed_dispute(40, "d:relayed", PAT, AGENT, "session-7"));
    events.push(dispute(50, "d:own-credential", PAT));
    // An `agent` source whose reference is the disputing actor is no relay: nobody else spoke.
    events.push(relayed_dispute(60, "d:self-sourced", PAT, PAT, "session-8"));
    let graph = graph_from_events(events)?;

    let relayed = context_of(&graph, "d:relayed");
    assert_eq!(relayed.attribution_disputes.len(), 1);
    assert_eq!(relayed.attribution_disputes[0].disputed_by, PAT);
    assert_eq!(relayed.attribution_disputes[0].reason, REASON);
    assert_eq!(
        relayed.attribution_disputes[0].relayed_by.as_deref(),
        Some(AGENT)
    );
    assert_eq!(
        relayed.attribution_disputes[0].said_in.as_deref(),
        Some("session-7")
    );
    let wire = serde_json::to_value(&relayed).expect("context serializes");
    assert_eq!(wire["attribution_disputes"][0]["relayed_by"], AGENT);
    assert_eq!(wire["attribution_disputes"][0]["said_in"], "session-7");

    // A person who disputed with their own credential has neither field on the wire.
    let own =
        serde_json::to_value(context_of(&graph, "d:own-credential")).expect("context serializes");
    assert_eq!(own["attribution_disputes"][0]["disputed_by"], PAT);
    assert!(own["attribution_disputes"][0].get("relayed_by").is_none());
    assert!(own["attribution_disputes"][0].get("said_in").is_none());

    // The agent source that names the person themselves relays nothing, but the place stays.
    let self_sourced = context_of(&graph, "d:self-sourced");
    assert_eq!(self_sourced.attribution_disputes[0].relayed_by, None);
    assert_eq!(
        self_sourced.attribution_disputes[0].said_in.as_deref(),
        Some("session-8")
    );

    // The bulk read the candidates query pages through says the same.
    let contexts = get_decision_context_candidates(
        &graph,
        &DecisionContextRequest {
            since_event_origin: None,
            limit: 10,
            cursor: None,
        },
    )?
    .data;
    let bulk = contexts
        .iter()
        .find(|context| context.decision_id == "d:relayed")
        .expect("relayed decision listed");
    assert_eq!(bulk.attribution_disputes, relayed.attribution_disputes);
    Ok(())
}

#[test]
fn a_dispute_by_one_of_two_acceptors_leaves_the_other_standing_behind_the_call() -> Result<()> {
    let mut events = proposed_and_accepted(10, "d:two", &[PAT, SAM]);
    events.push(dispute(30, "d:two", PAT));
    let graph = graph_from_events(events)?;

    let context = context_of(&graph, "d:two");
    assert_eq!(context.attribution_disputes.len(), 1);
    assert!(!context.human_acceptance_disputed);
    assert!(!context.peer_review_disputed);

    // Once the second disputes too, no human is left behind it.
    let mut events = proposed_and_accepted(10, "d:two", &[PAT, SAM]);
    events.push(dispute(30, "d:two", PAT));
    events.push(dispute(31, "d:two", SAM));
    let context = context_of(&graph_from_events(events)?, "d:two");
    assert_eq!(context.attribution_disputes.len(), 2);
    assert!(context.human_acceptance_disputed);
    assert!(context.peer_review_disputed);
    Ok(())
}

#[test]
fn failure_modes_count_a_disputed_acceptance_apart_from_the_calls_people_accepted() -> Result<()> {
    let undisputed = |extra: Option<Event>| {
        let mut events = proposed_and_accepted(10, "d:disputed", &[PAT]);
        events.extend(proposed_and_accepted(20, "d:control", &[PAT]));
        events.extend(extra);
        graph_from_events(events)
    };
    let request = FailureAttributionRequest::default();

    // Before the dispute both calls are counted among the ones a person accepted.
    let before = get_failure_attribution(&undisputed(None)?, &request)?.data;
    assert_eq!(
        group(&before.by_authorship, "agent_proposed_human_accepted").map(|g| g.total),
        Some(2)
    );
    assert_eq!(
        group(&before.by_review, "peer_reviewed").map(|g| g.total),
        Some(2)
    );

    let after =
        get_failure_attribution(&undisputed(Some(dispute(30, "d:disputed", PAT)))?, &request)?.data;
    assert_eq!(
        group(&after.by_authorship, "agent_proposed_human_accepted").map(|g| g.total),
        Some(1),
        "only the control is still a call a person accepted"
    );
    assert_eq!(
        group(&after.by_authorship, "agent_proposed_acceptance_disputed").map(|g| g.total),
        Some(1)
    );
    assert_eq!(
        group(&after.by_review, "peer_reviewed").map(|g| g.total),
        Some(1)
    );
    assert_eq!(
        group(&after.by_review, "acceptance_disputed").map(|g| g.total),
        Some(1)
    );
    assert_eq!(after.corpus_stats.total_decisions, 2, "none was dropped");
    Ok(())
}
