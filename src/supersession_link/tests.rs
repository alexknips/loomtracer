// Parent module gates this file with #[cfg(test)]; repeat the marker so UBS can filter test-only assertions.
#[cfg(test)]
use crate::events::EventType;
use crate::queries::test_fixtures::Scenario;
use crate::queries::{get_decision, walk_supersession, DecisionStatus};
use crate::Result;

use super::*;

const ALEX: &str = "human:alex";
const CREW: &str = "agent:claude:crew";

fn link(scenario: &Scenario, old: &str, new: &str, actor: &str) -> Result<SupersessionLinkOutcome> {
    link_supersession(
        scenario.ledger(),
        &TenantId::local(),
        EventProvenance::agent(actor.to_owned()),
        &scenario.graph()?,
        SupersessionLinkCall {
            actor_id: actor,
            old_decision_id: old,
            new_decision_id: new,
        },
    )
}

fn refusal(outcome: SupersessionLinkOutcome) -> String {
    match outcome {
        SupersessionLinkOutcome::Refused(text) => text,
        other => panic!("expected a refusal, got {other:?}"), // ubs:ignore: test-only; panicking is correct in tests
    }
}

fn reply(outcome: SupersessionLinkOutcome) -> SupersessionLinkReply {
    match outcome {
        SupersessionLinkOutcome::Linked(reply) => reply,
        other => panic!("expected a reply, got {other:?}"), // ubs:ignore: test-only; panicking is correct in tests
    }
}

/// The offset of the newest event: unchanged when a call wrote nothing.
fn offset(scenario: &Scenario) -> Result<EventId> {
    scenario
        .ledger()
        .latest_offset_for_tenant(&TenantId::local())
}

/// Two recorded decisions: A the crew's combined record, B a person's own answer.
fn pair() -> Result<Scenario> {
    let s = Scenario::new();
    s.decision(
        "d:a",
        "One combined projects record",
        CREW,
        "2026-09-01T00:00:00Z",
    )?;
    s.decision(
        "d:b",
        "Eight answers on projects",
        ALEX,
        "2026-09-21T00:00:00Z",
    )?;
    Ok(s)
}

fn chain_through(scenario: &Scenario, decision_id: &str) -> Result<Vec<String>> {
    Ok(walk_supersession(&scenario.graph()?, decision_id)?
        .data
        .chain
        .decision_ids)
}

#[test]
fn linking_two_recorded_decisions_appends_one_superseded_event_by_the_caller() -> Result<()> {
    let scenario = pair()?;
    let before = offset(&scenario)?;

    let linked = reply(link(&scenario, "d:a", "d:b", CREW)?);

    assert!(linked.linked);
    assert_eq!(linked.already_linked_through, None);
    assert_eq!(offset(&scenario)?, before + 1, "exactly one event");
    let appended = scenario
        .ledger()
        .read_for_tenant(&TenantId::local(), before, 10)?;
    assert_eq!(appended.len(), 1);
    assert_eq!(appended[0].event_type, EventType::DecisionSuperseded);
    assert_eq!(appended[0].actor_id, CREW, "the link is the caller's");
    assert_eq!(linked.event_id, appended[0].event_id);
    assert_eq!(
        appended[0].payload,
        serde_json::json!({"old_decision_id": "d:a", "new_decision_id": "d:b"})
    );
    Ok(())
}

#[test]
fn the_replaced_decision_reads_superseded_and_the_chain_walks_to_the_new_one() -> Result<()> {
    let scenario = pair()?;

    link(&scenario, "d:a", "d:b", CREW)?;

    let graph = scenario.graph()?;
    let old = get_decision(&graph, "d:a")?.data.map(|d| d.status);
    assert_eq!(old, Some(DecisionStatus::Superseded));
    assert_eq!(
        chain_through(&scenario, "d:a")?,
        vec!["d:a".to_owned(), "d:b".to_owned()]
    );
    Ok(())
}

#[test]
fn a_pair_already_linked_writes_nothing_and_names_the_chain() -> Result<()> {
    let scenario = pair()?;
    link(&scenario, "d:a", "d:b", CREW)?;
    let before = offset(&scenario)?;

    let again = reply(link(&scenario, "d:a", "d:b", CREW)?);

    assert!(!again.linked);
    assert_eq!(again.event_id, None);
    assert_eq!(
        again.already_linked_through,
        Some(vec!["d:b".to_owned(), "d:a".to_owned()])
    );
    assert_eq!(offset(&scenario)?, before, "nothing was written");
    Ok(())
}

#[test]
fn a_replacement_already_reached_through_another_writes_nothing() -> Result<()> {
    let scenario = pair()?;
    scenario.decision(
        "d:c",
        "Eight answers, restated",
        ALEX,
        "2026-09-22T00:00:00Z",
    )?;
    link(&scenario, "d:a", "d:b", CREW)?;
    link(&scenario, "d:b", "d:c", CREW)?;
    let before = offset(&scenario)?;

    let shortcut = reply(link(&scenario, "d:a", "d:c", CREW)?);

    assert!(!shortcut.linked);
    assert_eq!(
        shortcut.already_linked_through,
        Some(vec!["d:c".to_owned(), "d:b".to_owned(), "d:a".to_owned()])
    );
    assert_eq!(offset(&scenario)?, before, "no shortcut edge");
    assert_eq!(
        chain_through(&scenario, "d:a")?.len(),
        3,
        "the chain stays one line, not a fork"
    );
    Ok(())
}

#[test]
fn a_decision_cannot_replace_itself() -> Result<()> {
    let scenario = pair()?;
    let before = offset(&scenario)?;

    let text = refusal(link(&scenario, "d:a", "d:a", CREW)?);

    assert!(text.contains("cannot supersede itself"), "{text}");
    assert_eq!(offset(&scenario)?, before);
    Ok(())
}

#[test]
fn an_id_that_is_not_recorded_is_refused_whichever_end_it_is() -> Result<()> {
    let scenario = pair()?;
    let before = offset(&scenario)?;

    let old_missing = refusal(link(&scenario, "d:nope", "d:b", CREW)?);
    let new_missing = refusal(link(&scenario, "d:a", "d:nope", CREW)?);

    assert!(
        old_missing.contains("decision is not recorded: d:nope"),
        "{old_missing}"
    );
    assert!(
        new_missing.contains("decision is not recorded: d:nope"),
        "{new_missing}"
    );
    assert_eq!(offset(&scenario)?, before);
    Ok(())
}

#[test]
fn an_empty_id_is_a_validation_error() -> Result<()> {
    let scenario = pair()?;

    assert!(link(&scenario, " ", "d:b", CREW).is_err());
    assert!(link(&scenario, "d:a", "", CREW).is_err());
    Ok(())
}

#[test]
fn the_reverse_of_a_recorded_replacement_is_a_loop_and_is_refused() -> Result<()> {
    let scenario = pair()?;
    link(&scenario, "d:a", "d:b", CREW)?;
    let before = offset(&scenario)?;

    // b replaces a is recorded, so "a replaces b" would close the loop.
    let text = refusal(link(&scenario, "d:b", "d:a", CREW)?);

    assert!(
        text.contains("would close a loop of supersessions"),
        "{text}"
    );
    assert!(
        text.contains("\"Eight answers on projects\" (d:b) already supersedes"),
        "{text}"
    );
    assert_eq!(offset(&scenario)?, before, "nothing was written");
    Ok(())
}

#[test]
fn a_loop_through_other_replacements_is_refused_and_the_message_names_the_chain() -> Result<()> {
    let scenario = pair()?;
    scenario.decision(
        "d:c",
        "Eight answers, restated",
        ALEX,
        "2026-09-22T00:00:00Z",
    )?;
    link(&scenario, "d:a", "d:b", CREW)?;
    link(&scenario, "d:b", "d:c", CREW)?;
    let before = offset(&scenario)?;

    // c replaces b replaces a: "a replaces c" would close the loop.
    let text = refusal(link(&scenario, "d:c", "d:a", CREW)?);

    assert!(
        text.contains("would close a loop of supersessions"),
        "{text}"
    );
    assert!(
        text.contains("supersedes \"Eight answers on projects\" (d:b) supersedes"),
        "{text}"
    );
    assert_eq!(offset(&scenario)?, before);
    Ok(())
}
