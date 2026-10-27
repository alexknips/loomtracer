// Parent module gates this file with #[cfg(test)]; repeat the marker so UBS can filter test-only assertions.
#[cfg(test)]
use chrono::{Duration, Utc};

use crate::commands::{Commands, DecisionProposalInput, FloorInput, GroundInput, Grounding};
use crate::events::{RelationKind, TenantId};
use crate::floor::{BindsAnswer, FloorAnswers, FloorVerdict, UndoCostAnswer};
use crate::ledger::InMemoryEventLedger;
use crate::projector::{memory::MemoryGraph, rebuild_graph_for_tenant};
use crate::queries::test_fixtures::{
    decision_capture, evidence_capture, ts as test_ts, Scenario, TypedReadsOnly,
};
use crate::queries::DecisionStatus;
use crate::Result;
use serde_json::json;

use super::{
    get_changed_decisions, get_decision_timeline, ChangedDecisionsRequest, DecisionTimeline,
    TimelineFact,
};

const QUESTION: &str = "Which storage engine should the prototype use?";
const ACTOR: &str = "actor:alice";
const OTHER: &str = "actor:bob";
const ASKER: &str = "actor:carol";
/// The agent that recorded a decision for the person, and the agent that relays their dispute.
const SCRIBE: &str = "agent:claude:scribe";
const RELAY: &str = "agent:claude:mayor";
const PERSON: &str = "human:alex";
const QUOTE: &str = "upheld was your idea and I never agreed to it";

/// A captured decision that chose `chose`, accepted by `ACTOR` at capture.
fn capture(
    commands: &Commands<'_, InMemoryEventLedger>,
    title: &str,
    chose: &str,
    question: Option<&str>,
) -> Result<String> {
    let option_id = commands.record_option(ACTOR, chose, chose)?;
    commands.propose_decision(DecisionProposalInput {
        grounding: Grounding::NotAsked,
        expressed_confidence: None,
        actor_id: ACTOR,
        title,
        rationale: "Rationale text long enough for the readable floor",
        topic_keys: &["storage".to_owned()],
        option_ids: std::slice::from_ref(&option_id),
        option_labels: &[chose.to_owned()],
        chosen_option_id: Some(option_id.as_str()),
        decided_by: None,
        still_proposed: false,
        hypothesis_ids: &[],
        evidence_ids: &[],
        quote: None,
        question,
        delegated_by: None,
        project: None,
        floor: FloorInput::NotAsked,
    })
}

fn graph(ledger: &InMemoryEventLedger) -> Result<MemoryGraph> {
    let graph = MemoryGraph::default();
    rebuild_graph_for_tenant(ledger, &TenantId::local(), &graph)?;
    Ok(graph)
}

fn timeline(ledger: &InMemoryEventLedger, decision_id: &str) -> Result<DecisionTimeline> {
    let graph = graph(ledger)?;
    Ok(get_decision_timeline(&graph, ledger, decision_id)?
        .data
        .expect("the decision exists"))
}

fn kinds(timeline: &DecisionTimeline) -> Vec<String> {
    timeline
        .entries
        .iter()
        .map(|entry| {
            serde_json::to_value(&entry.fact)
                .ok()
                .and_then(|fact| fact["kind"].as_str().map(str::to_owned))
                .unwrap_or_default()
        })
        .collect()
}

fn ground_on(
    commands: &Commands<'_, InMemoryEventLedger>,
    decision_id: &str,
    hypothesis_id: &str,
) -> Result<()> {
    commands.ground_decision(GroundInput {
        actor_id: ACTOR,
        decision_id,
        premise_decision_ids: &[],
        evidence_ids: &[],
        hypothesis_ids: &[hypothesis_id.to_owned()],
    })?;
    Ok(())
}

#[test]
fn an_unknown_decision_has_no_timeline() -> Result<()> {
    let ledger = InMemoryEventLedger::new();
    let graph = graph(&ledger)?;
    let response = get_decision_timeline(&graph, &ledger, "decision-nobody-made")?;
    assert_eq!(response.result_count, 0);
    assert!(response.data.is_none());
    Ok(())
}

#[test]
fn the_timeline_tells_the_ask_the_record_and_the_acceptance_in_ledger_order() -> Result<()> {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let plan = commands.plan_ask(QUESTION)?;
    let ask = commands.record_ask(ASKER, &plan)?;
    let id = capture(&commands, "Use SQLite", "sqlite", Some(QUESTION))?;

    let timeline = timeline(&ledger, &id)?;

    assert_eq!(kinds(&timeline), vec!["asked", "recorded", "accepted"]);
    let origins: Vec<u64> = timeline.entries.iter().map(|e| e.event_origin).collect();
    assert!(
        origins.windows(2).all(|pair| pair[0] < pair[1]),
        "{origins:?}"
    );
    for entry in &timeline.entries {
        assert_eq!(entry.citation_id, format!("event:{}", entry.event_origin));
        assert!(entry.ts.is_some());
    }
    assert_eq!(timeline.entries[0].actor_id.as_deref(), Some(ASKER));
    assert_eq!(timeline.entries[1].actor_id.as_deref(), Some(ACTOR));
    assert_eq!(timeline.entries[2].actor_id.as_deref(), Some(ACTOR));
    let TimelineFact::Asked {
        request_id,
        question_id,
    } = &timeline.entries[0].fact
    else {
        panic!("first entry is the ask, got {:?}", timeline.entries[0].fact);
    };
    assert_eq!(request_id, &ask.request_id);
    assert_eq!(question_id, &ask.question_id);

    let (asked_at, decided_at) = (timeline.asked_at.unwrap(), timeline.decided_at.unwrap());
    assert!(asked_at <= decided_at);
    assert_eq!(
        timeline.asked_to_decided_seconds,
        Some((decided_at - asked_at).num_seconds())
    );
    Ok(())
}

#[test]
fn a_decision_nobody_asked_for_has_no_ask_and_no_duration() -> Result<()> {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let id = capture(&commands, "Use SQLite", "sqlite", Some(QUESTION))?;

    let timeline = timeline(&ledger, &id)?;

    assert_eq!(kinds(&timeline), vec!["recorded", "accepted"]);
    assert_eq!(timeline.asked_at, None);
    assert_eq!(timeline.asked_to_decided_seconds, None);
    assert!(timeline.decided_at.is_some());
    Ok(())
}

#[test]
fn an_ask_that_came_after_the_decision_gives_no_negative_duration() -> Result<()> {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let id = capture(&commands, "Use SQLite", "sqlite", Some(QUESTION))?;
    let plan = commands.plan_ask(QUESTION)?;
    commands.record_ask(ASKER, &plan)?;

    let timeline = timeline(&ledger, &id)?;

    assert_eq!(kinds(&timeline), vec!["recorded", "accepted", "asked"]);
    assert_eq!(timeline.asked_to_decided_seconds, None);
    Ok(())
}

#[test]
fn a_rejection_a_retitle_and_a_supersession_are_dated_entries() -> Result<()> {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let old = capture(&commands, "Use SQLite", "sqlite", None)?;
    let new = capture(&commands, "Use Postgres", "postgres", None)?;
    commands.reject_decision(&old, OTHER)?;
    commands.retitle_decision(
        ACTOR,
        &old,
        "Use SQLite",
        "Use SQLite for the prototype",
        Some("scope"),
    )?;
    commands.supersede_decision(&old, &new, ACTOR)?;

    let old_timeline = timeline(&ledger, &old)?;
    assert_eq!(
        kinds(&old_timeline),
        vec!["recorded", "accepted", "rejected", "retitled", "superseded"]
    );
    assert_eq!(old_timeline.entries[2].actor_id.as_deref(), Some(OTHER));
    assert_eq!(
        old_timeline.entries[3].fact,
        TimelineFact::Retitled {
            from: "Use SQLite".to_owned(),
            to: "Use SQLite for the prototype".to_owned(),
            reason: Some("scope".to_owned()),
        }
    );
    assert_eq!(
        old_timeline.entries[4].fact,
        TimelineFact::Superseded { by_id: new.clone() }
    );

    let new_timeline = timeline(&ledger, &new)?;
    assert_eq!(
        kinds(&new_timeline),
        vec!["recorded", "accepted", "supersedes"]
    );
    assert_eq!(
        new_timeline.entries[2].fact,
        TimelineFact::Supersedes { replaces_id: old }
    );
    Ok(())
}

/// A decision the agent `SCRIBE` recorded and accepted for the person `PERSON` (`decided_by`).
fn decided_for_person(commands: &Commands<'_, InMemoryEventLedger>) -> Result<String> {
    let option_id = commands.record_option(SCRIBE, "Upheld", "The new name")?;
    commands.propose_decision(DecisionProposalInput {
        grounding: Grounding::NotAsked,
        expressed_confidence: None,
        actor_id: SCRIBE,
        title: "The product has a new name",
        rationale: "Rationale text long enough for the readable floor",
        topic_keys: &["naming".to_owned()],
        option_ids: std::slice::from_ref(&option_id),
        option_labels: &["Upheld".to_owned()],
        chosen_option_id: Some(option_id.as_str()),
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
}

#[test]
fn a_dispute_a_persons_agent_relayed_names_the_relay_and_where_it_was_said() -> Result<()> {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let decision_id = decided_for_person(&commands)?;
    commands.dispute_attribution(PERSON, &decision_id, "mine, in my own credential")?;
    commands.relay_attribution_dispute(RELAY, PERSON, &decision_id, QUOTE, Some("session-7"))?;
    commands.relay_attribution_dispute(RELAY, PERSON, &decision_id, QUOTE, None)?;

    let timeline = timeline(&ledger, &decision_id)?;
    let disputes: Vec<_> = timeline
        .entries
        .iter()
        .filter(|entry| matches!(entry.fact, TimelineFact::AttributionDisputed { .. }))
        .collect();
    assert_eq!(disputes.len(), 3);
    // The entry's actor is the person each time; the relay is on the fact, not the actor.
    assert!(disputes
        .iter()
        .all(|entry| entry.actor_id.as_deref() == Some(PERSON)));
    assert_eq!(
        disputes[0].fact,
        TimelineFact::AttributionDisputed {
            reason: "mine, in my own credential".to_owned(),
            relayed_by: None,
            said_in: None,
        },
        "a person's own credential names no relay"
    );
    assert_eq!(
        disputes[1].fact,
        TimelineFact::AttributionDisputed {
            reason: QUOTE.to_owned(),
            relayed_by: Some(RELAY.to_owned()),
            said_in: Some("session-7".to_owned()),
        }
    );
    assert_eq!(
        disputes[2].fact,
        TimelineFact::AttributionDisputed {
            reason: QUOTE.to_owned(),
            relayed_by: Some(RELAY.to_owned()),
            said_in: None,
        },
        "a place the agent did not state is not guessed"
    );
    // On the wire the fields appear only when there is something to say.
    let wire: Vec<_> = disputes
        .iter()
        .map(|entry| serde_json::to_value(entry).expect("serializes"))
        .collect();
    assert!(wire[0].get("relayed_by").is_none() && wire[0].get("said_in").is_none());
    assert_eq!(wire[1]["relayed_by"], RELAY);
    assert_eq!(wire[1]["said_in"], "session-7");
    assert_eq!(wire[1]["actor_id"], PERSON);
    assert!(wire[2].get("said_in").is_none());
    Ok(())
}

#[test]
fn a_refuted_premise_reaches_the_decisions_that_rested_on_it_then() -> Result<()> {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let hypothesis =
        commands.record_hypothesis(ACTOR, "Traffic stays under one request a second")?;
    let evidence = commands.record_evidence(OTHER, "A load test served forty requests a second")?;
    let early = capture(&commands, "Use SQLite", "sqlite", None)?;
    ground_on(&commands, &early, &hypothesis)?;

    commands.relate_evidence_to_hypothesis(&evidence, &hypothesis, RelationKind::Refutes, OTHER)?;

    let late = capture(&commands, "Use a file", "file", None)?;
    ground_on(&commands, &late, &hypothesis)?;

    let early_timeline = timeline(&ledger, &early)?;
    assert_eq!(
        kinds(&early_timeline),
        vec!["recorded", "accepted", "premise_refuted"]
    );
    let refuted = &early_timeline.entries[2];
    assert_eq!(
        refuted.fact,
        TimelineFact::PremiseRefuted {
            hypothesis_id: hypothesis.clone(),
            evidence_id: evidence,
        }
    );
    assert_eq!(
        refuted.actor_id.as_deref(),
        Some(OTHER),
        "who recorded the refutation"
    );

    // A decision that started resting on the premise after it was refuted was stale from the
    // start; the refutation did not happen to it, so the timeline does not say it did.
    let late_timeline = timeline(&ledger, &late)?;
    assert_eq!(kinds(&late_timeline), vec!["recorded", "accepted"]);
    Ok(())
}

#[test]
fn a_superseded_or_rejected_premise_reaches_the_decisions_that_follow_from_it() -> Result<()> {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let premise = capture(&commands, "Use SQLite", "sqlite", None)?;
    let other_premise = capture(&commands, "Use tabs", "tabs", None)?;
    let follower = capture(&commands, "Ship a single binary", "binary", None)?;
    commands.link_follows_from(&follower, &premise, ACTOR)?;
    commands.link_follows_from(&follower, &other_premise, ACTOR)?;
    let replacement = capture(&commands, "Use Postgres", "postgres", None)?;

    commands.supersede_decision(&premise, &replacement, ACTOR)?;
    commands.reject_decision(&other_premise, OTHER)?;

    let follower_timeline = timeline(&ledger, &follower)?;
    let follower_kinds = kinds(&follower_timeline);
    assert_eq!(
        follower_kinds,
        vec![
            "recorded",
            "accepted",
            "premise_superseded",
            "premise_rejected"
        ]
    );
    assert_eq!(
        follower_timeline.entries[2].fact,
        TimelineFact::PremiseSuperseded {
            decision_id: premise,
            by_id: replacement.clone(),
        }
    );
    assert_eq!(
        follower_timeline.entries[3].fact,
        TimelineFact::PremiseRejected {
            decision_id: other_premise,
        }
    );
    // The replacement is the new answer, not something that went stale.
    assert!(!kinds(&timeline(&ledger, &replacement)?).contains(&"premise_superseded".to_owned()));
    Ok(())
}

fn changed(
    ledger: &InMemoryEventLedger,
    request: &ChangedDecisionsRequest,
) -> Result<crate::queries::QueryResponse<super::ChangedDecisionsResults>> {
    let graph = graph(ledger)?;
    get_changed_decisions(&graph, ledger, request)
}

#[test]
fn a_changed_decision_carries_the_floor_its_own_answers_reach() -> Result<()> {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let apple = commands.record_option(ACTOR, "apple", "apple")?;
    let answers = FloorAnswers {
        binds: BindsAnswer::nothing(),
        undo_cost: UndoCostAnswer::cheap(),
    };
    let below = commands.propose_decision(DecisionProposalInput {
        grounding: Grounding::NotAsked,
        expressed_confidence: None,
        actor_id: ACTOR,
        title: "Write Apple with a capital letter",
        rationale: "Rationale text long enough for the readable floor",
        topic_keys: &["storage".to_owned()],
        option_ids: std::slice::from_ref(&apple),
        option_labels: &["apple".to_owned()],
        chosen_option_id: Some(apple.as_str()),
        decided_by: None,
        still_proposed: false,
        hypothesis_ids: &[],
        evidence_ids: &[],
        quote: None,
        question: None,
        delegated_by: None,
        project: None,
        floor: FloorInput::Stated(&answers),
    })?;
    let unasked = capture(&commands, "Use tabs", "tabs", None)?;
    commands.retitle_decision(
        ACTOR,
        &below,
        "Write Apple with a capital letter",
        "Write Apple with a capital A",
        None,
    )?;
    commands.retitle_decision(ACTOR, &unasked, "Use tabs", "Use tabs everywhere", None)?;

    let response = changed(&ledger, &ChangedDecisionsRequest::default())?;

    let verdict_of = |id: &str| {
        response
            .data
            .items
            .iter()
            .find(|item| item.decision_id == id)
            .map(|item| item.floor.verdict)
    };
    assert_eq!(verdict_of(&below), Some(FloorVerdict::Below));
    assert_eq!(verdict_of(&unasked), Some(FloorVerdict::NotAsked));
    Ok(())
}

#[test]
fn changed_lists_revisions_and_supersessions_newest_first_and_not_ordinary_life() -> Result<()> {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let retitled = capture(&commands, "Use SQLite", "sqlite", None)?;
    let superseded = capture(&commands, "Use tabs", "tabs", None)?;
    let replacement = capture(&commands, "Use spaces", "spaces", None)?;
    let rejected = capture(&commands, "Use Kuzu", "kuzu", None)?;
    commands.retitle_decision(ACTOR, &retitled, "Use SQLite", "Use SQLite locally", None)?;
    commands.supersede_decision(&superseded, &replacement, ACTOR)?;
    commands.reject_decision(&rejected, OTHER)?;

    let response = changed(&ledger, &ChangedDecisionsRequest::default())?;

    let ids: Vec<&str> = response
        .data
        .items
        .iter()
        .map(|item| item.decision_id.as_str())
        .collect();
    assert_eq!(
        ids,
        vec![superseded.as_str(), retitled.as_str()],
        "newest change first"
    );
    assert_eq!(response.data.total_matches, 2);
    assert!(!response.truncated);
    assert_eq!(response.data.items[0].status, DecisionStatus::Superseded);
    assert_eq!(response.data.items[0].title, "Use tabs");
    assert_eq!(
        response.data.items[1].title, "Use SQLite locally",
        "the title now"
    );
    assert_eq!(
        response.data.items[0].changes[0].fact,
        TimelineFact::Superseded { by_id: replacement }
    );
    assert!(response.data.items[0].last_changed_at.is_some());
    Ok(())
}

#[test]
fn a_changed_decision_carries_its_ask_and_record_times() -> Result<()> {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let plan = commands.plan_ask(QUESTION)?;
    commands.record_ask(ASKER, &plan)?;
    let asked = capture(&commands, "Use SQLite", "sqlite", Some(QUESTION))?;
    let unasked = capture(&commands, "Use tabs", "tabs", None)?;
    commands.retitle_decision(ACTOR, &asked, "Use SQLite", "Use SQLite locally", None)?;
    commands.retitle_decision(ACTOR, &unasked, "Use tabs", "Use tabs everywhere", None)?;

    let response = changed(&ledger, &ChangedDecisionsRequest::default())?;

    let by_id = |id: &str| {
        response
            .data
            .items
            .iter()
            .find(|item| item.decision_id == id)
            .expect("the decision changed")
    };
    let item = by_id(&asked);
    let (asked_at, decided_at) = (item.asked_at.unwrap(), item.decided_at.unwrap());
    assert!(asked_at <= decided_at, "the ask came first");
    let item = by_id(&unasked);
    assert_eq!(item.asked_at, None, "nobody asked");
    assert!(item.decided_at.is_some());
    Ok(())
}

#[test]
fn a_decision_changed_twice_is_listed_once_with_both_changes_oldest_first() -> Result<()> {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let id = capture(&commands, "Use SQLite", "sqlite", None)?;
    commands.retitle_decision(ACTOR, &id, "Use SQLite", "Use SQLite locally", None)?;
    commands.retitle_decision(
        ACTOR,
        &id,
        "Use SQLite locally",
        "Use SQLite everywhere",
        None,
    )?;

    let response = changed(&ledger, &ChangedDecisionsRequest::default())?;

    assert_eq!(response.data.items.len(), 1);
    let changes = &response.data.items[0].changes;
    assert_eq!(changes.len(), 2);
    assert!(changes[0].event_origin < changes[1].event_origin);
    Ok(())
}

#[test]
fn the_window_bounds_the_changes_and_is_echoed() -> Result<()> {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let id = capture(&commands, "Use SQLite", "sqlite", None)?;
    commands.retitle_decision(ACTOR, &id, "Use SQLite", "Use SQLite locally", None)?;
    let now = Utc::now();

    let inside = changed(
        &ledger,
        &ChangedDecisionsRequest {
            since: Some(now - Duration::days(1)),
            until: Some(now + Duration::days(1)),
            ..ChangedDecisionsRequest::default()
        },
    )?;
    assert_eq!(inside.data.items.len(), 1);
    assert_eq!(inside.data.since, Some(now - Duration::days(1)));

    let after = changed(
        &ledger,
        &ChangedDecisionsRequest {
            since: Some(now + Duration::days(1)),
            ..ChangedDecisionsRequest::default()
        },
    )?;
    assert!(after.data.items.is_empty());
    assert_eq!(after.data.total_matches, 0);

    let before = changed(
        &ledger,
        &ChangedDecisionsRequest {
            until: Some(now - Duration::days(1)),
            ..ChangedDecisionsRequest::default()
        },
    )?;
    assert!(before.data.items.is_empty());
    Ok(())
}

#[test]
fn a_cursor_continues_past_the_last_change_shown_and_says_when_truncated() -> Result<()> {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let mut created = Vec::new();
    for (title, chose) in [("One", "a"), ("Two", "b"), ("Three", "c")] {
        let id = capture(&commands, title, chose, None)?;
        commands.retitle_decision(ACTOR, &id, title, &format!("{title} again"), None)?;
        created.push(id);
    }

    let first = changed(
        &ledger,
        &ChangedDecisionsRequest {
            limit: 2,
            ..ChangedDecisionsRequest::default()
        },
    )?;
    assert!(first.truncated);
    assert_eq!(first.data.total_matches, 3);
    assert_eq!(first.data.items[0].decision_id, created[2]);
    assert_eq!(first.data.items[1].decision_id, created[1]);
    let cursor = first.data.next_cursor.clone().expect("more remain");
    let last = &first.data.items[1];
    assert_eq!(
        cursor,
        format!(
            "{}:{}",
            last.changes.last().unwrap().event_origin,
            last.decision_id
        )
    );

    // A change written between the pages does not shift what the next page shows.
    commands.retitle_decision(ACTOR, &created[2], "Three again", "Three once more", None)?;

    let second = changed(
        &ledger,
        &ChangedDecisionsRequest {
            limit: 2,
            cursor: Some(cursor),
            ..ChangedDecisionsRequest::default()
        },
    )?;
    assert!(!second.truncated);
    assert_eq!(second.data.next_cursor, None);
    assert_eq!(second.data.items.len(), 1);
    assert_eq!(second.data.items[0].decision_id, created[0]);
    Ok(())
}

#[test]
fn decisions_sharing_one_change_page_without_losing_any() -> Result<()> {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let premise = capture(&commands, "Use SQLite", "sqlite", None)?;
    let mut followers = Vec::new();
    for (title, chose) in [("One", "a"), ("Two", "b"), ("Three", "c")] {
        let follower = capture(&commands, title, chose, None)?;
        commands.link_follows_from(&follower, &premise, ACTOR)?;
        followers.push(follower);
    }
    let replacement = capture(&commands, "Use Postgres", "postgres", None)?;
    // One event supersedes the premise and, in the same instant, leaves all three followers
    // without it: four decisions whose latest change is the same ledger offset.
    commands.supersede_decision(&premise, &replacement, ACTOR)?;

    let mut seen = Vec::new();
    let mut cursor = None;
    for _ in 0..4 {
        let page = changed(
            &ledger,
            &ChangedDecisionsRequest {
                limit: 2,
                cursor: cursor.clone(),
                ..ChangedDecisionsRequest::default()
            },
        )?;
        assert_eq!(page.data.total_matches, 4);
        seen.extend(page.data.items.iter().map(|item| item.decision_id.clone()));
        match page.data.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }

    let mut expected: Vec<String> = followers;
    expected.push(premise);
    expected.sort();
    seen.sort();
    assert_eq!(seen, expected, "every decision appears exactly once");
    Ok(())
}

#[test]
fn a_cursor_that_is_not_one_this_list_gave_is_refused() -> Result<()> {
    let ledger = InMemoryEventLedger::new();
    for cursor in ["7", "seven:decision-x", ""] {
        let refused = changed(
            &ledger,
            &ChangedDecisionsRequest {
                cursor: Some(cursor.to_owned()),
                ..ChangedDecisionsRequest::default()
            },
        );
        // An empty cursor is no cursor; the others are refused.
        assert_eq!(refused.is_err(), !cursor.is_empty(), "cursor {cursor:?}");
    }
    Ok(())
}

#[test]
fn an_empty_ledger_has_no_changes() -> Result<()> {
    let ledger = InMemoryEventLedger::new();
    let response = changed(&ledger, &ChangedDecisionsRequest::default())?;
    assert!(response.data.items.is_empty());
    assert!(!response.truncated);
    Ok(())
}

#[test]
fn the_timeline_and_the_changed_list_never_read_the_whole_ledger() -> Result<()> {
    // `why` read every event of a cell's ledger, ingest transcripts and all, to find the few
    // dozen decision events it dates (hivemind-7jon8).
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let superseded = capture(&commands, "Use tabs", "tabs", None)?;
    let replacement = capture(&commands, "Use spaces", "spaces", None)?;
    commands.supersede_decision(&superseded, &replacement, ACTOR)?;
    let graph = graph(&ledger)?;
    let guarded = TypedReadsOnly(&ledger);

    let read_whole = get_decision_timeline(&graph, &ledger, &superseded)?;
    let read_typed = get_decision_timeline(&graph, &guarded, &superseded)?;
    assert_eq!(read_typed.data, read_whole.data);
    assert!(
        kinds(&read_typed.data.expect("the decision exists")).contains(&"superseded".to_owned())
    );

    let request = ChangedDecisionsRequest::default();
    let read_whole = get_changed_decisions(&graph, &ledger, &request)?;
    let read_typed = get_changed_decisions(&graph, &guarded, &request)?;
    assert_eq!(read_typed.data, read_whole.data);
    assert_eq!(read_typed.data.total_matches, 1);
    Ok(())
}

// A classified capture states positions inside itself (`accepted_by`, `rejected_by`,
// `supersedes_id`, `premised_on_ids`, `refutes_ids`) and has no event of its own for any of them.
// The timeline and the changed list read them from the graph (hivemind-u46nq).

const CLASSIFIER: &str = "agent:claude:classifier";
const ALEX: &str = "human:alex";

fn capture_id(batch: crate::events::EventId, index: usize) -> String {
    format!("capture:{batch}:{index}")
}

fn timeline_of(scenario: &Scenario, decision_id: &str) -> Result<DecisionTimeline> {
    timeline(scenario.ledger(), decision_id)
}

fn changed_all(scenario: &Scenario) -> Result<super::ChangedDecisionsResults> {
    Ok(changed(
        scenario.ledger(),
        &ChangedDecisionsRequest {
            limit: 25,
            ..ChangedDecisionsRequest::default()
        },
    )?
    .data)
}

#[test]
fn a_capture_accepted_at_capture_shows_its_acceptance_where_it_was_said() -> Result<()> {
    let scenario = Scenario::new();
    let batch = scenario.classified_captures(
        CLASSIFIER,
        "2026-10-11T09:00:00Z",
        vec![decision_capture(
            "Use Postgres",
            json!({"accepted_by": [ALEX], "source_ts": "2026-10-11T07:30:00Z"}),
        )],
    )?;

    let timeline = timeline(scenario.ledger(), &capture_id(batch, 0))?;

    assert_eq!(kinds(&timeline), ["recorded", "accepted"]);
    let accepted = &timeline.entries[1];
    assert_eq!(accepted.event_origin, batch);
    assert_eq!(accepted.citation_id, format!("event:{batch}"));
    assert_eq!(accepted.ts, Some(test_ts("2026-10-11T07:30:00Z")));
    assert_eq!(accepted.actor_id.as_deref(), Some(ALEX));
    // Acceptance is the decision's ordinary life, not a revision.
    assert!(changed_all(&scenario)?.items.is_empty());
    Ok(())
}

#[test]
fn an_acceptance_the_ledger_also_holds_is_listed_once_and_a_second_acceptor_is_listed_too(
) -> Result<()> {
    let scenario = Scenario::new();
    let batch = scenario.classified_captures(
        CLASSIFIER,
        "2026-10-11T09:00:00Z",
        vec![decision_capture(
            "Use Postgres",
            json!({"accepted_by": [ALEX]}),
        )],
    )?;
    let id = capture_id(batch, 0);
    scenario.accept(&id, ALEX, "2026-10-11T10:00:00Z")?;
    scenario.accept(&id, "human:bea", "2026-10-11T11:00:00Z")?;

    let timeline = timeline(scenario.ledger(), &id)?;

    // Alex's acceptance is the event's, dated by it; Bea's is the event's too.
    assert_eq!(kinds(&timeline), ["recorded", "accepted", "accepted"]);
    assert_eq!(
        timeline.entries[1].ts,
        Some(test_ts("2026-10-11T10:00:00Z"))
    );
    assert_eq!(timeline.entries[2].actor_id.as_deref(), Some("human:bea"));
    Ok(())
}

#[test]
fn a_capture_rejected_at_capture_shows_its_rejection() -> Result<()> {
    let scenario = Scenario::new();
    let batch = scenario.classified_captures(
        CLASSIFIER,
        "2026-10-11T09:00:00Z",
        vec![decision_capture(
            "Use Mongo",
            json!({"rejected_by": ["human:bea"]}),
        )],
    )?;

    let timeline = timeline(scenario.ledger(), &capture_id(batch, 0))?;

    assert_eq!(kinds(&timeline), ["recorded", "rejected"]);
    assert_eq!(timeline.entries[1].event_origin, batch);
    assert_eq!(timeline.entries[1].actor_id.as_deref(), Some("human:bea"));
    Ok(())
}

/// A hand-recorded decision at 08:00 on the 10th that a capture of the 11th replaces
/// (`supersedes_id`), and a decision that follows from it.
fn superseded_by_a_capture() -> Result<(Scenario, crate::events::EventId)> {
    let scenario = Scenario::new();
    scenario.decision("decision-old", "Use tabs", ALEX, "2026-10-10T08:00:00Z")?;
    scenario.decision(
        "decision-follower",
        "Indent with tabs",
        ALEX,
        "2026-10-10T09:00:00Z",
    )?;
    scenario.relation(
        "FOLLOWS_FROM",
        "decision-follower",
        "decision-old",
        ALEX,
        None,
        "2026-10-10T10:00:00Z",
    )?;
    let batch = scenario.classified_captures(
        CLASSIFIER,
        "2026-10-11T09:00:00Z",
        vec![decision_capture(
            "Use spaces",
            json!({
                "accepted_by": [ALEX],
                "supersedes_id": "decision-old",
                "source_ts": "2026-10-11T07:30:00Z",
            }),
        )],
    )?;
    Ok((scenario, batch))
}

#[test]
fn a_decision_a_capture_replaces_shows_the_supersession_on_both_sides() -> Result<()> {
    let (scenario, batch) = superseded_by_a_capture()?;
    let replacement = capture_id(batch, 0);

    let old = timeline(scenario.ledger(), "decision-old")?;
    assert_eq!(kinds(&old), ["recorded", "superseded"]);
    let superseded = &old.entries[1];
    assert_eq!(
        superseded.fact,
        TimelineFact::Superseded {
            by_id: replacement.clone()
        }
    );
    assert_eq!(superseded.event_origin, batch);
    assert_eq!(superseded.ts, Some(test_ts("2026-10-11T07:30:00Z")));
    assert_eq!(superseded.actor_id.as_deref(), Some(ALEX));

    let new = timeline(scenario.ledger(), &replacement)?;
    assert_eq!(kinds(&new), ["recorded", "accepted", "supersedes"]);
    assert_eq!(
        new.entries[2].fact,
        TimelineFact::Supersedes {
            replaces_id: "decision-old".to_owned()
        }
    );
    Ok(())
}

#[test]
fn a_decision_a_capture_replaces_is_in_the_changed_list_with_its_followers() -> Result<()> {
    let (scenario, batch) = superseded_by_a_capture()?;
    let replacement = capture_id(batch, 0);

    let results = changed_all(&scenario)?;

    // The replaced decision and the one that follows from it; the replacement is the new answer.
    let ids: Vec<&str> = results
        .items
        .iter()
        .map(|item| item.decision_id.as_str())
        .collect();
    assert_eq!(ids, ["decision-follower", "decision-old"]);
    let old = &results.items[1];
    assert_eq!(old.status, DecisionStatus::Superseded);
    assert_eq!(old.last_changed_at, Some(test_ts("2026-10-11T07:30:00Z")));
    assert_eq!(old.changes.len(), 1);
    assert_eq!(
        old.changes[0].fact,
        TimelineFact::Superseded {
            by_id: replacement.clone()
        }
    );
    assert_eq!(
        results.items[0].changes[0].fact,
        TimelineFact::PremiseSuperseded {
            decision_id: "decision-old".to_owned(),
            by_id: replacement,
        }
    );
    Ok(())
}

#[test]
fn a_supersession_the_ledger_also_holds_is_listed_once() -> Result<()> {
    let (scenario, batch) = superseded_by_a_capture()?;
    scenario.supersede(
        "decision-old",
        &capture_id(batch, 0),
        ALEX,
        "2026-10-11T12:00:00Z",
    )?;

    let old = timeline(scenario.ledger(), "decision-old")?;

    // The event's own entry, dated by it, and not the capture's again.
    assert_eq!(kinds(&old), ["recorded", "superseded"]);
    assert_eq!(old.entries[1].ts, Some(test_ts("2026-10-11T12:00:00Z")));
    Ok(())
}

#[test]
fn a_refutation_a_capture_states_reaches_the_decisions_that_rested_on_the_hypothesis() -> Result<()>
{
    let scenario = Scenario::new();
    scenario.hypothesis(
        "hypothesis-cache",
        "The cache hit rate stays high",
        "assumption",
        None,
        "2026-10-10T07:00:00Z",
    )?;
    // Resting on it by an event, and by a capture's `premised_on_ids`.
    scenario.decision(
        "decision-by-event",
        "Cache reads",
        ALEX,
        "2026-10-10T08:00:00Z",
    )?;
    scenario.relation(
        "ASSUMES",
        "decision-by-event",
        "hypothesis-cache",
        ALEX,
        None,
        "2026-10-10T08:01:00Z",
    )?;
    let premised = scenario.classified_captures(
        CLASSIFIER,
        "2026-10-10T09:00:00Z",
        vec![decision_capture(
            "Cache sessions",
            json!({"premised_on_ids": ["hypothesis-cache"]}),
        )],
    )?;
    let refuted = scenario.classified_captures(
        CLASSIFIER,
        "2026-10-12T09:00:00Z",
        vec![evidence_capture(
            "The load test missed the cache half the time",
            json!({"refutes_ids": ["hypothesis-cache"]}),
        )],
    )?;
    // Premised on it only after it was refuted: stale from the start, so not told again.
    let late = scenario.classified_captures(
        CLASSIFIER,
        "2026-10-13T09:00:00Z",
        vec![decision_capture(
            "Cache pages",
            json!({"premised_on_ids": ["hypothesis-cache"]}),
        )],
    )?;
    let evidence = capture_id(refuted, 0);
    let refutation = TimelineFact::PremiseRefuted {
        hypothesis_id: "hypothesis-cache".to_owned(),
        evidence_id: evidence,
    };

    for id in ["decision-by-event".to_owned(), capture_id(premised, 0)] {
        let timeline = timeline(scenario.ledger(), &id)?;
        let entry = timeline
            .entries
            .iter()
            .find(|entry| entry.fact == refutation)
            .expect("the refutation reaches the decision");
        assert_eq!(entry.event_origin, refuted);
        // Evidence carries no time of its own: the classification's time and recorder.
        assert_eq!(entry.ts, Some(test_ts("2026-10-12T09:00:00Z")));
        assert_eq!(entry.actor_id.as_deref(), Some(CLASSIFIER));
    }
    assert_eq!(
        kinds(&timeline(scenario.ledger(), &capture_id(late, 0))?),
        ["recorded"]
    );

    let results = changed_all(&scenario)?;
    let mut ids: Vec<&str> = results
        .items
        .iter()
        .map(|item| item.decision_id.as_str())
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, [capture_id(premised, 0).as_str(), "decision-by-event"]);
    Ok(())
}

#[test]
fn a_premise_a_capture_rejected_reaches_the_decisions_that_followed_from_it_then() -> Result<()> {
    // The follower names the capture as its premise before the capture is classified (the capture
    // lands at offset 3), so it is told when the capture rejects it.
    let told = Scenario::new();
    told.decision(
        "decision-follower",
        "Indent with tabs",
        ALEX,
        "2026-10-10T09:00:00Z",
    )?;
    told.relation(
        "FOLLOWS_FROM",
        "decision-follower",
        "capture:3:0",
        ALEX,
        None,
        "2026-10-10T09:30:00Z",
    )?;
    let rejected = told.classified_captures(
        CLASSIFIER,
        "2026-10-10T10:00:00Z",
        vec![decision_capture(
            "Use tabs",
            json!({"rejected_by": ["human:bea"]}),
        )],
    )?;
    assert_eq!(rejected, 3);
    let timeline = timeline(told.ledger(), "decision-follower")?;
    assert_eq!(kinds(&timeline), ["recorded", "premise_rejected"]);
    assert_eq!(timeline.entries[1].event_origin, rejected);

    // A follower that started following only after the rejection was in doubt from the start.
    let late = Scenario::new();
    late.decision(
        "decision-follower",
        "Indent with tabs",
        ALEX,
        "2026-10-10T09:00:00Z",
    )?;
    let rejected = late.classified_captures(
        CLASSIFIER,
        "2026-10-10T10:00:00Z",
        vec![decision_capture(
            "Use tabs",
            json!({"rejected_by": ["human:bea"]}),
        )],
    )?;
    late.relation(
        "FOLLOWS_FROM",
        "decision-follower",
        &capture_id(rejected, 0),
        ALEX,
        None,
        "2026-10-10T11:00:00Z",
    )?;
    assert_eq!(
        kinds(&timeline_of(&late, "decision-follower")?),
        ["recorded"]
    );
    Ok(())
}

#[test]
fn capture_stated_positions_are_read_without_reading_the_whole_ledger() -> Result<()> {
    let (scenario, batch) = superseded_by_a_capture()?;
    scenario.hypothesis(
        "hypothesis-cache",
        "The cache hit rate stays high",
        "assumption",
        None,
        "2026-10-11T10:00:00Z",
    )?;
    scenario.classified_captures(
        CLASSIFIER,
        "2026-10-11T11:00:00Z",
        vec![
            decision_capture(
                "Cache sessions",
                json!({"premised_on_ids": ["hypothesis-cache"]}),
            ),
            evidence_capture(
                "The load test missed",
                json!({"refutes_ids": ["hypothesis-cache"]}),
            ),
        ],
    )?;
    let graph = graph(scenario.ledger())?;
    let guarded = TypedReadsOnly(scenario.ledger());
    let id = capture_id(batch, 0);

    for decision_id in ["decision-old", id.as_str()] {
        let whole = get_decision_timeline(&graph, scenario.ledger(), decision_id)?;
        let typed = get_decision_timeline(&graph, &guarded, decision_id)?;
        assert_eq!(typed.data, whole.data);
    }
    let request = ChangedDecisionsRequest::default();
    let whole = get_changed_decisions(&graph, scenario.ledger(), &request)?;
    let typed = get_changed_decisions(&graph, &guarded, &request)?;
    assert_eq!(typed.data, whole.data);
    assert!(typed.data.total_matches >= 3);
    Ok(())
}
