// Parent module gates this file with #[cfg(test)]; repeat the marker so UBS can filter test-only assertions.
#[cfg(test)]
use crate::commands::{
    AcceptanceInput, Commands, DecisionProposalInput, FloorInput, Grounding, ReaffirmInput,
};
use crate::error::{CommandError, LoomtracerError};
use crate::events::{Event, EventSource, EventType};
use crate::ledger::{EventLedger, InMemoryEventLedger};

/// The agent that heard the person and records it.
const RELAY: &str = "agent:claude:relay";
const PERSON: &str = "human:pat@example.com";
const PROPOSER: &str = "agent:claude:scribe";
const QUOTE: &str = "A per-tenant cap is wrong for us, and that still holds";
const QUESTION: &str = "Should the hosted cell cap requests per tenant?";

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
            // It speaks to the relaying agent and names what that agent can do for the person.
            assert!(
                message.contains(&format!("{RELAY} cannot record")),
                "{message}"
            );
            assert!(message.contains("`dispute_attribution`"), "{message}");
            assert!(
                message.contains(&format!("`disputed_by` {PERSON}")),
                "{message}"
            );
            assert!(message.contains("`capture_decision`"), "{message}");
            assert!(
                message.contains(&format!("`decided_by` {PERSON}")),
                "{message}"
            );
            assert!(message.contains("`link_supersession`"), "{message}");
            // Not the person's own-credential hint, which refuses an agent and loops back here.
            assert!(
                !message.contains("To change your own decision"),
                "{message}"
            );
            assert!(!message.contains("--reason"), "{message}");
        }
        other => panic!("expected an invariant refusal, got {other:?}"),
    }
    assert_eq!(events(&ledger).len(), before, "nothing written");
}

#[test]
fn the_dispute_a_refused_relayed_disagreement_points_at_records_the_persons_words() {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let decision_id = propose(&commands, "Cap requests per tenant");
    commands
        .accept_decision(&decision_id, PERSON)
        .expect("the person accepts");
    commands
        .relay_disagreement(RELAY, PERSON, &decision_id, QUOTE, None)
        .expect_err("they already accepted it");

    // The next step the refusal names, taken as the same agent with the same words.
    commands
        .relay_attribution_dispute(RELAY, PERSON, &decision_id, QUOTE, None)
        .expect("the agent records the person's dispute");

    let disputes = of_type(&ledger, EventType::DecisionAttributionDisputed);
    assert_eq!(disputes.len(), 1);
    assert_eq!(disputes[0].actor_id, PERSON);
    assert_eq!(disputes[0].source_ref.as_deref(), Some(RELAY));
    assert_eq!(payload_str(&disputes[0], "reason"), QUOTE);
}

#[test]
fn a_person_disagreeing_with_their_own_credential_keeps_the_own_decision_refusal() {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let decision_id = propose(&commands, "Cap requests per tenant");
    commands
        .accept_decision(&decision_id, PERSON)
        .expect("the person accepts");

    let refused = commands
        .disagree(PERSON, &decision_id, "I changed my mind")
        .expect_err("they already accepted it");

    match refused {
        LoomtracerError::Command(CommandError::Invariant(message)) => {
            assert!(message.contains("To change your own decision"), "{message}");
            assert!(
                message.contains("loomtracer dispute-attribution"),
                "{message}"
            );
            assert!(!message.contains("cannot record"), "{message}");
        }
        other => panic!("expected an invariant refusal, got {other:?}"),
    }
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

// ── A person's acceptance through their agent (hivemind-fsxx9) ──

fn acceptance<'a>(
    accepted_by: &'a str,
    decision_id: &'a str,
    quote: &'a str,
    question: &'a str,
) -> AcceptanceInput<'a> {
    AcceptanceInput {
        accepted_by,
        decision_id,
        quote,
        question,
    }
}

#[test]
fn an_agent_records_the_persons_acceptance_as_theirs_with_their_words_and_the_relay() {
    let ledger = InMemoryEventLedger::new();
    // A `Commands` built with another provenance (the CLI default): the relay is recorded anyway,
    // because it is part of what the event says.
    let commands = Commands::new(&ledger);
    let decision_id = propose(&commands, "Cap requests per tenant");
    assert!(
        of_type(&ledger, EventType::DecisionAccepted).is_empty(),
        "the proposal is open: nobody has accepted it"
    );
    let before = events(&ledger).len();

    let event_id = commands
        .relay_acceptance(
            RELAY,
            Some("session-7"),
            acceptance(PERSON, &decision_id, QUOTE, QUESTION),
        )
        .expect("the agent records the person's acceptance");

    let appended: Vec<Event> = events(&ledger).into_iter().skip(before).collect();
    assert_eq!(appended.len(), 1, "one event, nothing else touched");
    let event = &appended[0];
    assert_eq!(event.event_id, Some(event_id));
    assert_eq!(event.event_type, EventType::DecisionAccepted);
    assert_eq!(event.actor_id, PERSON, "the event is the person's");
    assert_eq!(event.source, EventSource::Agent);
    assert_eq!(event.source_ref.as_deref(), Some(RELAY), "who relayed it");
    assert_eq!(payload_str(event, "decision_id"), decision_id);
    assert_eq!(payload_str(event, "quote"), QUOTE, "their verbatim words");
    assert_eq!(payload_str(event, "question"), QUESTION);
    assert_eq!(payload_str(event, "said_in"), "session-7");
    assert!(
        event.payload.get("delegated_by").is_none(),
        "a person's acceptance carries no delegation: {}",
        event.payload
    );
}

#[test]
fn where_the_person_said_their_acceptance_is_left_out_when_the_agent_does_not_say() {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let decision_id = propose(&commands, "Cap requests per tenant");

    commands
        .relay_acceptance(
            RELAY,
            None,
            acceptance(PERSON, &decision_id, QUOTE, QUESTION),
        )
        .expect("recorded without a place");

    let accepted = of_type(&ledger, EventType::DecisionAccepted);
    assert!(
        accepted[0].payload.get("said_in").is_none(),
        "a place nobody stated is not guessed: {}",
        accepted[0].payload
    );
}

#[test]
fn the_refusals_accept_decision_has_hold_for_a_relayed_acceptance() {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let decision_id = propose(&commands, "Cap requests per tenant");

    assert!(
        matches!(
            commands.relay_acceptance(
                RELAY,
                None,
                acceptance(PERSON, "decision-nope", QUOTE, QUESTION)
            ),
            Err(LoomtracerError::Command(CommandError::Invariant(_)))
        ),
        "a decision that does not exist"
    );
    commands
        .disagree(PERSON, &decision_id, "I would rather run without a cap")
        .expect("the person rejects it first");
    let before_relay = events(&ledger).len();
    let refused = commands
        .relay_acceptance(
            RELAY,
            None,
            acceptance(PERSON, &decision_id, QUOTE, QUESTION),
        )
        .expect_err("a person who rejected a decision cannot also accept it");
    match refused {
        LoomtracerError::Command(CommandError::Invariant(message)) => assert!(
            message.contains(&format!("{PERSON} already rejected decision {decision_id}")),
            "{message}"
        ),
        other => panic!("expected an invariant refusal, got {other:?}"),
    }
    assert_eq!(
        events(&ledger).len(),
        before_relay,
        "the refused acceptance wrote nothing"
    );
}

#[test]
fn only_an_agent_relays_an_acceptance_and_only_a_person_is_relayed() {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let decision_id = propose(&commands, "Cap requests per tenant");
    let before = events(&ledger).len();

    for (relayed_by, accepted_by) in [
        (RELAY, PROPOSER),
        (RELAY, "someone"),
        (RELAY, "human:"),
        (PERSON, PERSON),
        ("human:bystander", PERSON),
    ] {
        assert!(
            matches!(
                commands.relay_acceptance(
                    relayed_by,
                    None,
                    acceptance(accepted_by, &decision_id, QUOTE, QUESTION)
                ),
                Err(LoomtracerError::Command(CommandError::Validation(_)))
            ),
            "{relayed_by} relaying for {accepted_by}"
        );
    }
    assert!(commands
        .relay_acceptance(
            RELAY,
            None,
            acceptance(
                "018f5d8a-03fb-7df0-8e36-64d7410cfe26",
                &decision_id,
                QUOTE,
                QUESTION
            )
        )
        .is_err());
    assert_eq!(events(&ledger).len(), before, "nothing written");
}

#[test]
fn the_words_and_the_question_they_answer_are_both_required_and_a_blank_place_is_refused() {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let decision_id = propose(&commands, "Cap requests per tenant");
    let before = events(&ledger).len();

    for blank in ["", "   ", "\n\t"] {
        assert!(
            matches!(
                commands.relay_acceptance(
                    RELAY,
                    None,
                    acceptance(PERSON, &decision_id, blank, QUESTION)
                ),
                Err(LoomtracerError::Command(CommandError::Validation(_)))
            ),
            "{blank:?} is not their words"
        );
        assert!(
            matches!(
                commands.relay_acceptance(
                    RELAY,
                    None,
                    acceptance(PERSON, &decision_id, QUOTE, blank)
                ),
                Err(LoomtracerError::Command(CommandError::Validation(_)))
            ),
            "{blank:?} is not a question"
        );
        assert!(
            matches!(
                commands.relay_acceptance(
                    RELAY,
                    Some(blank),
                    acceptance(PERSON, &decision_id, QUOTE, QUESTION)
                ),
                Err(LoomtracerError::Command(CommandError::Validation(_)))
            ),
            "{blank:?} is not a place"
        );
    }
    assert_eq!(events(&ledger).len(), before, "nothing written");
}

#[test]
fn a_person_may_accept_what_an_agent_accepted_alone_and_a_second_person_what_a_first_did() {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let decision_id = propose(&commands, "Cap requests per tenant");
    commands
        .accept_decision(&decision_id, PROPOSER)
        .expect("the agent accepts its own proposal alone");

    commands
        .relay_acceptance(
            RELAY,
            None,
            acceptance(PERSON, &decision_id, QUOTE, QUESTION),
        )
        .expect("the person accepts what the agent accepted alone");
    commands
        .relay_acceptance(
            RELAY,
            None,
            acceptance(
                "human:sam",
                &decision_id,
                "fine by me",
                "Do you agree with a per-tenant cap?",
            ),
        )
        .expect("a second person accepts, each in their own words");

    let acceptances = of_type(&ledger, EventType::DecisionAccepted);
    assert_eq!(acceptances.len(), 3, "the ledger keeps every acceptance");
    assert_eq!(payload_str(&acceptances[1], "quote"), QUOTE);
    assert_eq!(payload_str(&acceptances[2], "quote"), "fine by me");
}

#[test]
fn a_retried_acceptance_relay_with_the_same_uuid_lands_once() {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let decision_id = propose(&commands, "Cap requests per tenant");
    let uuid = uuid::Uuid::new_v4();

    let first = commands
        .relay_acceptance_with_uuid(
            RELAY,
            None,
            acceptance(PERSON, &decision_id, QUOTE, QUESTION),
            uuid,
        )
        .expect("first");
    let retried = commands
        .relay_acceptance_with_uuid(
            RELAY,
            None,
            acceptance(PERSON, &decision_id, QUOTE, QUESTION),
            uuid,
        )
        .expect("retry");

    assert_eq!(retried, first, "the ledger hands back the existing event");
    assert_eq!(of_type(&ledger, EventType::DecisionAccepted).len(), 1);
}
