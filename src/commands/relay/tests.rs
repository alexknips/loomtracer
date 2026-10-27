// Parent module gates this file with #[cfg(test)]; repeat the marker so UBS can filter test-only assertions.
#[cfg(test)]
use crate::commands::{Commands, DecisionProposalInput, FloorInput, Grounding, ReaffirmInput};
use crate::error::{CommandError, LoomtracerError};
use crate::events::{Event, EventSource, EventType};
use crate::ledger::{EventLedger, InMemoryEventLedger};

/// The agent that heard the person and records it.
const RELAY: &str = "agent:claude:relay";
const PERSON: &str = "human:pat@example.com";
const PROPOSER: &str = "agent:claude:scribe";
const QUOTE: &str = "A per-tenant cap is wrong for us, and that still holds";

fn propose(commands: &Commands<'_, InMemoryEventLedger>, title: &str) -> String {
    let option_id = commands
        .record_option(PROPOSER, "Only option", "The only option")
        .expect("record option");
    commands
        .propose_decision(DecisionProposalInput {
            grounding: Grounding::NotAsked,
            expressed_confidence: None,
            actor_id: PROPOSER,
            title,
            rationale: "Rationale text long enough for the readable floor",
            topic_keys: &["topic".to_owned()],
            option_ids: std::slice::from_ref(&option_id),
            option_labels: &["Only option".to_owned()],
            chosen_option_id: None,
            decided_by: None,
            still_proposed: true,
            hypothesis_ids: &[],
            evidence_ids: &[],
            quote: None,
            question: None,
            delegated_by: None,
            project: None,
            floor: FloorInput::NotAsked,
        })
        .expect("propose decision")
}

fn events(ledger: &InMemoryEventLedger) -> Vec<Event> {
    ledger.read(0, 500).expect("read events")
}

fn of_type(ledger: &InMemoryEventLedger, event_type: EventType) -> Vec<Event> {
    events(ledger)
        .into_iter()
        .filter(|event| event.event_type == event_type)
        .collect()
}

fn payload_str<'a>(event: &'a Event, key: &str) -> &'a str {
    event
        .payload
        .get(key)
        .and_then(|value| value.as_str())
        .unwrap_or_default()
}

#[test]
fn a_relayed_disagreement_is_the_persons_event_with_the_agent_as_the_relay() {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let decision_id = propose(&commands, "Cap requests per tenant");
    let before = events(&ledger).len();

    let event_id = commands
        .relay_disagreement(RELAY, PERSON, &decision_id, QUOTE, Some("session-7"))
        .expect("the agent records the person's disagreement");

    let appended: Vec<Event> = events(&ledger).into_iter().skip(before).collect();
    assert_eq!(appended.len(), 1, "one event, nothing else touched");
    let event = &appended[0];
    assert_eq!(event.event_id, Some(event_id));
    assert_eq!(event.event_type, EventType::DecisionRejected);
    assert_eq!(event.actor_id, PERSON, "attributed to the person");
    assert_eq!(event.source, EventSource::Agent, "recorded by an agent");
    assert_eq!(event.source_ref.as_deref(), Some(RELAY), "the relay");
    assert_eq!(payload_str(event, "decision_id"), decision_id);
    assert_eq!(payload_str(event, "reason"), QUOTE, "their verbatim words");
    assert_eq!(payload_str(event, "said_in"), "session-7");
}

#[test]
fn where_the_person_said_it_is_left_out_when_the_agent_does_not_say() {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let decision_id = propose(&commands, "Cap requests per tenant");

    commands
        .relay_disagreement(RELAY, PERSON, &decision_id, QUOTE, None)
        .expect("relay without a place");

    let event = of_type(&ledger, EventType::DecisionRejected).remove(0);
    assert!(
        event.payload.get("said_in").is_none(),
        "nothing is guessed: {}",
        event.payload
    );
}

#[test]
fn a_disagreement_made_directly_is_unchanged_and_carries_no_relay() {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let decision_id = propose(&commands, "Cap requests per tenant");

    commands
        .disagree(PERSON, &decision_id, "I disagree")
        .expect("direct disagreement");

    let event = of_type(&ledger, EventType::DecisionRejected).remove(0);
    assert_eq!(event.actor_id, PERSON);
    assert_ne!(
        event.source_ref.as_deref(),
        Some(RELAY),
        "no agent relayed it"
    );
    assert!(event.payload.get("said_in").is_none(), "{}", event.payload);
}

#[test]
fn a_relay_needs_an_agent_a_person_and_the_persons_words() {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let decision_id = propose(&commands, "Cap requests per tenant");
    let before = events(&ledger).len();

    // A person's name without their words, a person who is not a person, an agent that is not an
    // agent, and a blank place: none is recorded.
    for (relayed_by, disagreed_by, quote, said_in) in [
        (RELAY, PERSON, "", None),
        (RELAY, PERSON, "   ", None),
        (RELAY, "agent:claude:other", QUOTE, None),
        (RELAY, "someone", QUOTE, None),
        (RELAY, "human:", QUOTE, None),
        (PERSON, PERSON, QUOTE, None),
        ("human:bystander", PERSON, QUOTE, None),
        (RELAY, PERSON, QUOTE, Some("  ")),
    ] {
        assert!(
            matches!(
                commands.relay_disagreement(relayed_by, disagreed_by, &decision_id, quote, said_in),
                Err(LoomtracerError::Command(CommandError::Validation(_)))
            ),
            "{relayed_by} relaying for {disagreed_by}: {quote:?} {said_in:?}"
        );
    }
    assert!(
        commands
            .relay_disagreement(RELAY, PERSON, "decision-missing", QUOTE, None)
            .is_err(),
        "a decision that does not exist"
    );
    assert_eq!(events(&ledger).len(), before, "nothing written");
}

#[test]
fn a_person_who_accepted_the_decision_cannot_also_disagree_through_their_agent() {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let decision_id = propose(&commands, "Cap requests per tenant");
    commands
        .accept_decision(&decision_id, PERSON)
        .expect("the person accepts");
    let before = events(&ledger).len();

    let refused = commands
        .relay_disagreement(RELAY, PERSON, &decision_id, QUOTE, None)
        .expect_err("they already accepted it");

    match refused {
        LoomtracerError::Command(CommandError::Invariant(message)) => {
            assert!(message.contains("already accepted"), "{message}");
            assert!(message.contains("dispute-attribution"), "{message}");
        }
        other => panic!("expected an invariant refusal, got {other:?}"),
    }
    assert_eq!(events(&ledger).len(), before, "nothing written");
}

#[test]
fn saying_it_again_is_the_same_disagreement() {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let decision_id = propose(&commands, "Cap requests per tenant");

    let first = commands
        .relay_disagreement(RELAY, PERSON, &decision_id, QUOTE, None)
        .expect("first");
    let second = commands
        .relay_disagreement(RELAY, PERSON, &decision_id, QUOTE, Some("session-8"))
        .expect("again");

    assert_eq!(first, second, "the earlier event is returned");
    assert_eq!(of_type(&ledger, EventType::DecisionRejected).len(), 1);
}

/// "Shard by tenant" follows from "Cap requests per tenant".
fn dependent_on_a_premise(commands: &Commands<'_, InMemoryEventLedger>) -> (String, String) {
    let premise_id = propose(commands, "Cap requests per tenant");
    let decision_id = propose(commands, "Shard by tenant");
    commands
        .link_follows_from(&decision_id, &premise_id, PROPOSER)
        .expect("link");
    (decision_id, premise_id)
}

#[test]
fn a_relayed_reaffirmation_is_the_persons_event_with_the_agent_as_the_relay() {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let (decision_id, premise_id) = dependent_on_a_premise(&commands);
    let before = events(&ledger).len();

    let event_id = commands
        .relay_reaffirmation(
            RELAY,
            Some("thread-3"),
            ReaffirmInput {
                actor_id: PERSON,
                decision_id: &decision_id,
                premise_id: &premise_id,
                reason: QUOTE,
            },
        )
        .expect("the agent records the person's reaffirmation");

    let appended: Vec<Event> = events(&ledger).into_iter().skip(before).collect();
    assert_eq!(appended.len(), 1, "one event, no fan-out");
    let event = &appended[0];
    assert_eq!(event.event_id, Some(event_id));
    assert_eq!(event.event_type, EventType::DecisionReaffirmed);
    assert_eq!(event.actor_id, PERSON, "attributed to the person");
    assert_eq!(event.source, EventSource::Agent);
    assert_eq!(event.source_ref.as_deref(), Some(RELAY), "the relay");
    assert_eq!(payload_str(event, "decision_id"), decision_id);
    assert_eq!(payload_str(event, "premise_id"), premise_id);
    assert_eq!(payload_str(event, "reason"), QUOTE, "their verbatim words");
    assert_eq!(payload_str(event, "said_in"), "thread-3");
}

#[test]
fn a_relayed_reaffirmation_needs_an_agent_a_person_and_the_persons_words() {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let (decision_id, premise_id) = dependent_on_a_premise(&commands);
    let before = events(&ledger).len();

    for (relayed_by, person, quote, said_in) in [
        (RELAY, PERSON, "", None),
        (RELAY, "agent:claude:other", QUOTE, None),
        (RELAY, "someone", QUOTE, None),
        (PERSON, PERSON, QUOTE, None),
        (RELAY, PERSON, QUOTE, Some(" ")),
    ] {
        assert!(
            matches!(
                commands.relay_reaffirmation(
                    relayed_by,
                    said_in,
                    ReaffirmInput {
                        actor_id: person,
                        decision_id: &decision_id,
                        premise_id: &premise_id,
                        reason: quote,
                    }
                ),
                Err(LoomtracerError::Command(CommandError::Validation(_)))
            ),
            "{relayed_by} relaying for {person}: {quote:?} {said_in:?}"
        );
    }
    assert_eq!(events(&ledger).len(), before, "nothing written");
}
