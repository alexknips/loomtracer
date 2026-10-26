// Parent module gates this file with #[cfg(test)]; repeat the marker so UBS can filter test-only assertions.
#[cfg(test)]
use crate::commands::{Commands, DecisionProposalInput, FloorInput, Grounding};
use crate::error::{CommandError, LoomtracerError};
use crate::events::{CaptureItem, Event, EventType};
use crate::ledger::{EventLedger, InMemoryEventLedger};

/// The agent that wrote the decision down, naming the person as its decider.
const SCRIBE: &str = "agent:claude:scribe";
const PERSON: &str = "human:alex";
const REASON: &str = "I never agreed to this name; an agent relayed a default in my name";

/// A decision `SCRIBE` recorded and accepted on behalf of `PERSON` (`decided_by`).
fn decided_for_person(commands: &Commands<'_, InMemoryEventLedger>) -> String {
    let option_id = commands
        .record_option(SCRIBE, "Upheld", "The new name")
        .expect("record option");
    commands
        .propose_decision(DecisionProposalInput {
            grounding: Grounding::NotAsked,
            expressed_confidence: None,
            actor_id: SCRIBE,
            title: "The product has a new name",
            rationale: "Rationale text long enough for the readable floor",
            topic_keys: &["naming".to_owned()],
            option_ids: std::slice::from_ref(&option_id),
            option_labels: &["Upheld".to_owned()],
            chosen_option_id: Some(&option_id),
            decided_by: Some(PERSON),
            still_proposed: false,
            hypothesis_ids: &[],
            evidence_ids: &[],
            quote: None,
            question: None,
            delegated_by: None,
            project: None,
            floor: FloorInput::NotAsked,
        })
        .expect("propose and accept for the person")
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
fn the_named_decider_disputes_and_exactly_one_event_lands_beside_the_acceptance() {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let decision_id = decided_for_person(&commands);
    let acceptances_before = of_type(&ledger, EventType::DecisionAccepted);
    let before = events(&ledger).len();

    let event_id = commands
        .dispute_attribution(PERSON, &decision_id, REASON)
        .expect("the person named as decider disputes");

    let appended: Vec<Event> = events(&ledger).into_iter().skip(before).collect();
    assert_eq!(appended.len(), 1, "one event, nothing else touched");
    let event = &appended[0];
    assert_eq!(event.event_id, Some(event_id));
    assert_eq!(event.event_type, EventType::DecisionAttributionDisputed);
    assert_eq!(event.actor_id, PERSON, "attributed to the one disputing");
    assert_eq!(payload_str(event, "decision_id"), decision_id);
    assert_eq!(payload_str(event, "reason"), REASON);
    assert_eq!(
        of_type(&ledger, EventType::DecisionAccepted),
        acceptances_before,
        "the acceptance is neither rewritten nor removed"
    );
    assert!(
        of_type(&ledger, EventType::DecisionRejected).is_empty(),
        "a dispute is not a rejection"
    );
}

#[test]
fn an_agent_cannot_dispute_for_the_person_it_named() {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let decision_id = decided_for_person(&commands);
    let before = events(&ledger).len();

    // The scribe recorded the decision but is not its decider, so the dispute is not its to make,
    // and the event takes no target to name the person with.
    let refused = commands
        .dispute_attribution(SCRIBE, &decision_id, REASON)
        .expect_err("the scribe is not a decider");

    match refused {
        LoomtracerError::Command(CommandError::Invariant(message)) => {
            assert!(
                message.contains(&format!("{SCRIBE} is not recorded as a decider")),
                "names who was refused: {message}"
            );
            assert!(
                message.contains("only for itself"),
                "says whose it is to make: {message}"
            );
            assert!(
                message.contains("loomtracer disagree"),
                "points at disagree: {message}"
            );
        }
        other => panic!("expected an invariant refusal, got {other:?}"),
    }
    assert_eq!(events(&ledger).len(), before, "nothing written");
}

#[test]
fn someone_who_was_never_named_as_decider_is_refused() {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let decision_id = decided_for_person(&commands);
    let before = events(&ledger).len();

    let refused = commands
        .dispute_attribution("human:bystander", &decision_id, REASON)
        .expect_err("a bystander has nothing to dispute");

    assert!(matches!(
        refused,
        LoomtracerError::Command(CommandError::Invariant(_))
    ));
    assert_eq!(events(&ledger).len(), before, "nothing written");
}

#[test]
fn an_unknown_decision_a_blank_reason_and_a_bare_uuid_actor_are_refused() {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let decision_id = decided_for_person(&commands);
    let before = events(&ledger).len();

    assert!(matches!(
        commands.dispute_attribution(PERSON, "decision-nope", REASON),
        Err(LoomtracerError::Command(CommandError::Invariant(_)))
    ));
    for reason in ["", "   ", "\n\t"] {
        assert!(
            matches!(
                commands.dispute_attribution(PERSON, &decision_id, reason),
                Err(LoomtracerError::Command(CommandError::Validation(_)))
            ),
            "{reason:?} is not a reason"
        );
    }
    assert!(commands
        .dispute_attribution("018f5d8a-03fb-7df0-8e36-64d7410cfe26", &decision_id, REASON)
        .is_err());
    assert_eq!(events(&ledger).len(), before, "nothing written");
}

#[test]
fn a_decider_who_disputes_may_still_not_reject_so_disagree_stays_a_different_act() {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let decision_id = decided_for_person(&commands);
    commands
        .dispute_attribution(PERSON, &decision_id, REASON)
        .expect("dispute");

    // The refusal that sent the person here still stands: the acceptance is as recorded.
    let refused = commands
        .disagree(PERSON, &decision_id, "no")
        .expect_err("the acceptance stands, so rejecting it is still refused");
    match refused {
        LoomtracerError::Command(CommandError::Invariant(message)) => assert!(
            message.contains("loomtracer dispute-attribution"),
            "the refusal now names the way out: {message}"
        ),
        other => panic!("expected an invariant refusal, got {other:?}"),
    }
}

#[test]
fn a_second_dispute_by_the_same_actor_is_a_new_statement_and_kept() {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let decision_id = decided_for_person(&commands);

    commands
        .dispute_attribution(PERSON, &decision_id, "first words")
        .expect("first");
    commands
        .dispute_attribution(PERSON, &decision_id, "better words")
        .expect("second");

    assert_eq!(
        of_type(&ledger, EventType::DecisionAttributionDisputed).len(),
        2,
        "the ledger keeps both; the read side shows the latest"
    );
}

#[test]
fn a_decider_a_classifier_read_out_of_a_transcript_may_dispute_it() {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    commands
        .record_evidence(PERSON, "an earlier event, so the batch is not event 1")
        .expect("evidence");
    let capture = CaptureItem {
        kind: "decision".to_owned(),
        title: "Use Postgres for the ledger".to_owned(),
        rationale: "It gives us one server for every tenant".to_owned(),
        topic_keys: Vec::new(),
        evidence_ids: Vec::new(),
        options: Some(vec!["Postgres".to_owned(), "SQLite".to_owned()]),
        chosen_option: Some("Postgres".to_owned()),
        extraction_confidence: 0.9,
        expressed_confidence: None,
        supersedes_id: None,
        premised_on_ids: Vec::new(),
        supports_ids: Vec::new(),
        refutes_ids: Vec::new(),
        actor_id: None,
        accepted_by: vec![PERSON.to_owned()],
        rejected_by: Vec::new(),
        blocked_actor_id: None,
        decision_id: None,
        participants: Vec::new(),
        restates_id: None,
        source_turn_id: None,
        source_ts: None,
        question: None,
        session_initiator: None,
    };
    let batch_event = commands
        .record_ingest_batch_classified(
            "agent:loomtracer:classifier",
            &["batch-1".to_owned()],
            "claude-haiku-4-5-20251001",
            "v1",
            vec![capture],
            None,
        )
        .expect("classified batch")
        .event_id;
    let draft_id = format!("capture:{batch_event}:0");

    commands
        .dispute_attribution(PERSON, &draft_id, REASON)
        .expect("the person the transcript names as decider disputes");
    assert!(
        commands
            .dispute_attribution("human:bystander", &draft_id, REASON)
            .is_err(),
        "someone the draft does not name is refused"
    );
    assert_eq!(
        of_type(&ledger, EventType::DecisionAttributionDisputed).len(),
        1
    );
}
