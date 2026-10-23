// Parent module gates this file with #[cfg(test)]; repeat the marker so UBS can filter test-only assertions.
#[cfg(test)]
use uuid::Uuid;

use crate::commands::{Commands, DecisionProposalInput, FloorInput, Grounding, ReaffirmInput};
use crate::error::{CommandError, LoomtracerError};
use crate::events::{Event, EventType};
use crate::ledger::{EventLedger, InMemoryEventLedger};

const PROPOSER: &str = "human:alice";
const REVIEWER: &str = "agent:codex:reviewer";
const REASON: &str = "Sharding never depended on the per-tenant cap";

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

fn reaffirmed(ledger: &InMemoryEventLedger) -> Vec<Event> {
    events(ledger)
        .into_iter()
        .filter(|event| event.event_type == EventType::DecisionReaffirmed)
        .collect()
}

fn input<'a>(decision_id: &'a str, premise_id: &'a str) -> ReaffirmInput<'a> {
    ReaffirmInput {
        actor_id: PROPOSER,
        decision_id,
        premise_id,
        reason: REASON,
    }
}

fn payload_str<'a>(event: &'a Event, key: &str) -> &'a str {
    event
        .payload
        .get(key)
        .and_then(|value| value.as_str())
        .unwrap_or_default()
}

#[test]
fn reaffirms_a_dependent_on_a_premise_decision_and_records_exactly_one_event() {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let premise_id = propose(&commands, "Cap requests per tenant");
    let decision_id = propose(&commands, "Shard by tenant");
    let other_dependent = propose(&commands, "Size the queue per shard");
    commands
        .link_follows_from(&decision_id, &premise_id, PROPOSER)
        .expect("link");
    commands
        .link_follows_from(&other_dependent, &decision_id, PROPOSER)
        .expect("link");
    let before = events(&ledger).len();

    let event_id = commands
        .reaffirm_decision(ReaffirmInput {
            actor_id: REVIEWER,
            decision_id: &decision_id,
            premise_id: &premise_id,
            reason: REASON,
        })
        .expect("reaffirm");

    let appended: Vec<Event> = events(&ledger).into_iter().skip(before).collect();
    assert_eq!(
        appended.len(),
        1,
        "one event, no fan-out to other dependents"
    );
    let event = &appended[0];
    assert_eq!(event.event_id, Some(event_id));
    assert_eq!(event.event_type, EventType::DecisionReaffirmed);
    assert_eq!(
        event.actor_id, REVIEWER,
        "attributed as given; agents are peers"
    );
    assert_eq!(payload_str(event, "decision_id"), decision_id);
    assert_eq!(payload_str(event, "premise_id"), premise_id);
    assert_eq!(payload_str(event, "reason"), REASON);
    assert!(event.causation_event_id.is_none());
}

#[test]
fn a_hypothesis_is_a_premise_too() {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let decision_id = propose(&commands, "Shard by tenant");
    let hypothesis_id = commands
        .record_hypothesis(PROPOSER, "Traffic stays under 1k rps")
        .expect("hypothesis");

    commands
        .reaffirm_decision(input(&decision_id, &hypothesis_id))
        .expect("reaffirm on a hypothesis");

    let recorded = reaffirmed(&ledger);
    assert_eq!(recorded.len(), 1);
    assert_eq!(payload_str(&recorded[0], "premise_id"), hypothesis_id);
}

#[test]
fn a_blank_reason_is_refused_and_nothing_is_written() {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let premise_id = propose(&commands, "Cap requests per tenant");
    let decision_id = propose(&commands, "Shard by tenant");
    let before = events(&ledger).len();

    for reason in ["", "   ", "\n\t"] {
        let refused = commands.reaffirm_decision(ReaffirmInput {
            reason,
            ..input(&decision_id, &premise_id)
        });
        assert!(
            matches!(&refused, Err(LoomtracerError::Command(CommandError::Validation(message))) if message.contains("reason")),
            "a one-line reason is required, got {refused:?}"
        );
    }

    assert_eq!(events(&ledger).len(), before);
}

#[test]
fn an_unknown_dependent_or_premise_is_refused_and_nothing_is_written() {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let known = propose(&commands, "Shard by tenant");
    let before = events(&ledger).len();

    let no_dependent = commands.reaffirm_decision(input("decision:missing", &known));
    assert!(
        matches!(&no_dependent, Err(LoomtracerError::Command(CommandError::Invariant(message))) if message.contains("decision does not exist: decision:missing")),
        "{no_dependent:?}"
    );
    let no_premise = commands.reaffirm_decision(input(&known, "decision:missing"));
    assert!(
        matches!(&no_premise, Err(LoomtracerError::Command(CommandError::Invariant(message))) if message.contains("premise does not exist: decision:missing")),
        "{no_premise:?}"
    );

    assert_eq!(events(&ledger).len(), before);
}

#[test]
fn a_decision_cannot_reaffirm_itself() {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let decision_id = propose(&commands, "Shard by tenant");
    let before = events(&ledger).len();

    let refused = commands.reaffirm_decision(input(&decision_id, &decision_id));

    assert!(
        matches!(&refused, Err(LoomtracerError::Command(CommandError::Validation(message))) if message.contains("its own premise")),
        "{refused:?}"
    );
    assert_eq!(events(&ledger).len(), before);
}

#[test]
fn an_anonymous_or_bare_uuid_actor_is_refused() {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let premise_id = propose(&commands, "Cap requests per tenant");
    let decision_id = propose(&commands, "Shard by tenant");
    let before = events(&ledger).len();
    let bare_uuid = Uuid::new_v4().to_string();

    for actor_id in ["", "  ", bare_uuid.as_str()] {
        let refused = commands.reaffirm_decision(ReaffirmInput {
            actor_id,
            ..input(&decision_id, &premise_id)
        });
        assert!(
            matches!(
                &refused,
                Err(LoomtracerError::Command(CommandError::Validation(_)))
            ),
            "actor {actor_id:?}: {refused:?}"
        );
    }

    assert_eq!(events(&ledger).len(), before);
}

#[test]
fn a_retry_with_the_same_event_uuid_returns_the_existing_event() {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let premise_id = propose(&commands, "Cap requests per tenant");
    let decision_id = propose(&commands, "Shard by tenant");
    let event_uuid = Uuid::new_v4();

    let first = commands
        .reaffirm_decision_with_uuid(input(&decision_id, &premise_id), event_uuid)
        .expect("first call");
    let retry = commands
        .reaffirm_decision_with_uuid(input(&decision_id, &premise_id), event_uuid)
        .expect("retry");

    assert_eq!(retry, first);
    assert_eq!(reaffirmed(&ledger).len(), 1);
}

#[test]
fn the_same_words_again_are_a_second_act_not_a_retry() {
    // The premise may have changed since the first reaffirmation: this one closes the item
    // that change reopened. Matching the earlier event and answering with its id would leave
    // that item open while the caller is told it is closed.
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let premise_id = propose(&commands, "Cap requests per tenant");
    let decision_id = propose(&commands, "Shard by tenant");

    let first = commands
        .reaffirm_decision(input(&decision_id, &premise_id))
        .expect("first");
    let again = commands
        .reaffirm_decision(input(&decision_id, &premise_id))
        .expect("again");

    assert_ne!(again, first);
    assert_eq!(reaffirmed(&ledger).len(), 2);
}

#[test]
fn no_other_command_ever_writes_a_reaffirmation() {
    // Closing a re-examine item is an explicit act of a person or an agent. A supersession,
    // a disagreement or a new link on the premise opens or changes items and never closes one.
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let premise_id = propose(&commands, "Cap requests per tenant");
    let dependent = propose(&commands, "Shard by tenant");
    let replacement = propose(&commands, "Cap requests per user");
    commands
        .link_follows_from(&dependent, &premise_id, PROPOSER)
        .expect("link");

    commands
        .supersede_decision(&premise_id, &replacement, PROPOSER)
        .expect("supersede the premise");
    commands
        .disagree(REVIEWER, &replacement, "the cap should be per organisation")
        .expect("disagree with the replacement");
    commands
        .link_follows_from(&replacement, &premise_id, PROPOSER)
        .expect("link again");

    assert!(reaffirmed(&ledger).is_empty());
}
