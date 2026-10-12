// Parent module gates this file with #[cfg(test)]; repeat the marker so UBS can filter test-only assertions.
#[cfg(test)]
use crate::commands::{
    Commands, DecisionProposalInput, FloorInput, Grounding, GroundingPlan, NewBet, NewEvidence,
    SupersedeInput,
};
use crate::events::{EventType, TenantId};
use crate::floor::{BindsAnswer, FloorAnswers, FloorVerdict, UndoCostAnswer};
use crate::ledger::{EventLedger, InMemoryEventLedger};
use crate::projector::{memory::MemoryGraph, rebuild_graph_for_tenant};
use crate::queries::{DecisionStatus, GroundingKind};
use crate::Result;

use super::{get_open_why_decisions, OpenWhyRequest, OpenWhyResults};

const AGENT: &str = "agent:codex:worker";
const OTHER_AGENT: &str = "agent:codex:other";
const PERSON: &str = "human:alex";

fn binding() -> FloorAnswers {
    FloorAnswers {
        binds: BindsAnswer::something("every script that calls the old name"),
        undo_cost: UndoCostAnswer::cheap(),
    }
}

fn costly() -> FloorAnswers {
    FloorAnswers {
        binds: BindsAnswer::nothing(),
        undo_cost: UndoCostAnswer::costly("a merge cannot be unmerged"),
    }
}

fn below() -> FloorAnswers {
    FloorAnswers {
        binds: BindsAnswer::nothing(),
        undo_cost: UndoCostAnswer::cheap(),
    }
}

fn evidence_plan(content: &str) -> GroundingPlan {
    GroundingPlan {
        new_evidence: vec![NewEvidence {
            content: content.to_owned(),
            source: Some("ci run 42".to_owned()),
        }],
        ..GroundingPlan::default()
    }
}

/// Everything a test varies about one capture.
struct Capture<'a> {
    actor: &'a str,
    title: &'a str,
    plan: GroundingPlan,
    floor: Option<FloorAnswers>,
    decided_by: Option<&'a str>,
    delegated_by: Option<&'a str>,
    still_proposed: bool,
}

impl<'a> Capture<'a> {
    fn new(title: &'a str, plan: GroundingPlan) -> Self {
        Self {
            actor: AGENT,
            title,
            plan,
            floor: Some(binding()),
            decided_by: None,
            delegated_by: None,
            still_proposed: false,
        }
    }

    fn floor(mut self, floor: Option<FloorAnswers>) -> Self {
        self.floor = floor;
        self
    }
}

fn record(commands: &Commands<'_, InMemoryEventLedger>, capture: Capture<'_>) -> Result<String> {
    let option_id = commands.record_option(capture.actor, "A", "Option A")?;
    let labels = ["A".to_owned()];
    let topics = ["naming".to_owned()];
    let input = DecisionProposalInput {
        grounding: Grounding::NotAsked,
        expressed_confidence: None,
        actor_id: capture.actor,
        title: capture.title,
        rationale: "Rationale text long enough for the readable floor",
        topic_keys: &topics,
        option_ids: std::slice::from_ref(&option_id),
        option_labels: &labels,
        chosen_option_id: Some(option_id.as_str()),
        decided_by: capture.decided_by,
        still_proposed: capture.still_proposed,
        hypothesis_ids: &[],
        evidence_ids: &[],
        quote: None,
        question: None,
        delegated_by: capture.delegated_by,
        project: None,
        floor: capture
            .floor
            .as_ref()
            .map_or(FloorInput::NotAsked, FloorInput::Stated),
    };
    Ok(commands
        .propose_grounded_decision(input, &capture.plan)?
        .decision_id)
}

fn graph(ledger: &InMemoryEventLedger) -> Result<MemoryGraph> {
    let graph = MemoryGraph::default();
    rebuild_graph_for_tenant(ledger, &TenantId::local(), &graph)?;
    Ok(graph)
}

fn listed(ledger: &InMemoryEventLedger) -> Result<OpenWhyResults> {
    page(ledger, 0, None)
}

fn page(
    ledger: &InMemoryEventLedger,
    limit: usize,
    cursor: Option<&str>,
) -> Result<OpenWhyResults> {
    Ok(get_open_why_decisions(
        &graph(ledger)?,
        &OpenWhyRequest {
            limit,
            cursor: cursor.map(str::to_owned),
        },
    )?
    .data)
}

/// The id of the first observation the ledger holds.
fn first_evidence_id(ledger: &InMemoryEventLedger) -> String {
    ledger
        .read(0, 200)
        .expect("read events")
        .iter()
        .find(|event| event.event_type == EventType::EvidenceRecorded)
        .and_then(|event| event.payload.get("evidence_id"))
        .and_then(|value| value.as_str())
        .expect("an evidence.recorded event")
        .to_owned()
}

fn ids(results: &OpenWhyResults) -> Vec<&str> {
    results
        .items
        .iter()
        .map(|item| item.decision_id.as_str())
        .collect()
}

#[test]
fn an_empty_ledger_lists_nothing() -> Result<()> {
    let ledger = InMemoryEventLedger::new();

    let results = listed(&ledger)?;

    assert!(results.items.is_empty());
    assert_eq!(results.total_matches, 0);
    assert_eq!(results.next_cursor, None);
    assert_eq!(results.left_out.below_floor, 0);
    assert_eq!(results.left_out.floor_not_asked, 0);
    Ok(())
}

#[test]
fn a_binding_decision_resting_only_on_its_own_evidence_is_listed_with_what_it_rests_on(
) -> Result<()> {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let id = record(
        &commands,
        Capture::new(
            "Rename the check command",
            evidence_plan("14 scripts call it"),
        ),
    )?;

    let results = listed(&ledger)?;

    assert_eq!(ids(&results), vec![id.as_str()]);
    assert_eq!(results.total_matches, 1);
    let item = &results.items[0];
    assert_eq!(item.title, "Rename the check command");
    assert_eq!(item.status, DecisionStatus::Accepted);
    assert!(item.captured_at.is_some());
    assert_eq!(item.recorded_by.as_deref(), Some(AGENT));
    assert_eq!(item.deciders.len(), 1);
    assert_eq!(item.deciders[0].id, AGENT);
    assert_eq!(item.deciders[0].kind, "agent");
    assert_eq!(item.floor.verdict, FloorVerdict::Cleared);
    assert_eq!(item.rests_on.len(), 1);
    assert_eq!(item.rests_on[0].kind, GroundingKind::Evidence);
    assert_eq!(item.rests_on[0].label, "14 scripts call it");
    assert_eq!(item.rests_on[0].source.as_deref(), Some("ci run 42"));
    Ok(())
}

#[test]
fn a_decision_costly_to_undo_clears_the_floor_without_binding_anything() -> Result<()> {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let id = record(
        &commands,
        Capture::new("Merge tonight", evidence_plan("the release is tomorrow"))
            .floor(Some(costly())),
    )?;

    assert_eq!(ids(&listed(&ledger)?), vec![id.as_str()]);
    Ok(())
}

#[test]
fn a_decision_below_the_floor_is_not_listed_and_is_counted() -> Result<()> {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    record(
        &commands,
        Capture::new(
            "Type apple, not Apple",
            evidence_plan("the shop sells apples"),
        )
        .floor(Some(below())),
    )?;

    let results = listed(&ledger)?;

    assert!(results.items.is_empty());
    assert_eq!(results.left_out.below_floor, 1);
    assert_eq!(results.left_out.floor_not_asked, 0);
    Ok(())
}

#[test]
fn a_decision_whose_floor_was_never_asked_is_not_listed_and_is_counted() -> Result<()> {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    record(
        &commands,
        Capture::new("Recorded before the floor", evidence_plan("it was noted")).floor(None),
    )?;

    let results = listed(&ledger)?;

    assert!(results.items.is_empty());
    assert_eq!(results.left_out.below_floor, 0);
    assert_eq!(results.left_out.floor_not_asked, 1);
    Ok(())
}

#[test]
fn a_decision_that_follows_from_an_earlier_decision_is_not_listed() -> Result<()> {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let earlier = record(
        &commands,
        Capture::new("Adopt the queue", evidence_plan("throughput was measured")),
    )?;
    let follower = record(
        &commands,
        Capture::new(
            "Size the queue at 10k",
            GroundingPlan {
                premise_decision_ids: vec![earlier.clone()],
                new_evidence: vec![NewEvidence {
                    content: "peak was 8k".to_owned(),
                    source: None,
                }],
                ..GroundingPlan::default()
            },
        ),
    )?;

    let results = listed(&ledger)?;

    assert_eq!(ids(&results), vec![earlier.as_str()]);
    assert!(!ids(&results).contains(&follower.as_str()));
    Ok(())
}

#[test]
fn a_decision_a_person_made_is_a_root_and_is_not_listed() -> Result<()> {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let agent_call = record(
        &commands,
        Capture::new("Agent call", evidence_plan("agent observation")),
    )?;
    record(
        &commands,
        Capture {
            decided_by: Some(PERSON),
            ..Capture::new("Alex's ruling", evidence_plan("Alex said so in the thread"))
        },
    )?;
    record(
        &commands,
        Capture {
            delegated_by: Some(PERSON),
            ..Capture::new("Within a delegation", evidence_plan("scope was delegated"))
        },
    )?;

    let results = listed(&ledger)?;

    assert_eq!(ids(&results), vec![agent_call.as_str()]);
    Ok(())
}

#[test]
fn a_decision_nobody_accepted_is_listed_with_no_decider_and_its_recorder() -> Result<()> {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let id = record(
        &commands,
        Capture {
            still_proposed: true,
            ..Capture::new("A recommendation", evidence_plan("the numbers"))
        },
    )?;

    let results = listed(&ledger)?;

    assert_eq!(ids(&results), vec![id.as_str()]);
    assert_eq!(results.items[0].status, DecisionStatus::Proposed);
    assert!(results.items[0].deciders.is_empty());
    assert_eq!(results.items[0].recorded_by.as_deref(), Some(AGENT));
    Ok(())
}

#[test]
fn assumptions_and_bets_from_the_same_capture_are_its_own() -> Result<()> {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let id = record(
        &commands,
        Capture::new(
            "Ship on Friday",
            GroundingPlan {
                new_assumptions: vec!["traffic stays under 1k rps".to_owned()],
                bet: Some(NewBet {
                    statement: Some("Friday is safe".to_owned()),
                    would_change_if: Some("p95 rises above 300ms".to_owned()),
                    check_by: None,
                }),
                ..GroundingPlan::default()
            },
        ),
    )?;

    let results = listed(&ledger)?;

    assert_eq!(ids(&results), vec![id.as_str()]);
    let rests_on: Vec<(GroundingKind, &str)> = results.items[0]
        .rests_on
        .iter()
        .map(|item| (item.kind, item.label.as_str()))
        .collect();
    assert_eq!(
        rests_on,
        vec![
            (GroundingKind::Assumption, "traffic stays under 1k rps"),
            (GroundingKind::Bet, "Friday is safe"),
        ]
    );
    Ok(())
}

#[test]
fn an_observation_two_decisions_rest_on_is_reused_and_lists_neither() -> Result<()> {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    record(
        &commands,
        Capture::new("First call", evidence_plan("a shared measurement")),
    )?;
    let evidence_id = first_evidence_id(&ledger);
    record(
        &commands,
        Capture {
            actor: OTHER_AGENT,
            ..Capture::new(
                "Second call",
                GroundingPlan {
                    evidence_ids: vec![evidence_id],
                    ..GroundingPlan::default()
                },
            )
        },
    )?;
    // A third decision keeps its own evidence and stays listed.
    let alone = record(
        &commands,
        Capture::new("Third call", evidence_plan("its own measurement")),
    )?;

    let results = listed(&ledger)?;

    assert_eq!(ids(&results), vec![alone.as_str()]);
    Ok(())
}

#[test]
fn evidence_attributed_after_the_capture_takes_the_decision_off_the_list() -> Result<()> {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let id = record(
        &commands,
        Capture::new("Pick the queue", evidence_plan("throughput was measured")),
    )?;
    assert_eq!(ids(&listed(&ledger)?), vec![id.as_str()]);

    commands.ground_decision_with_plan(OTHER_AGENT, &id, &evidence_plan("a later review"))?;

    assert!(listed(&ledger)?.items.is_empty());
    Ok(())
}

#[test]
fn a_decision_grounded_later_on_an_earlier_decision_leaves_the_list() -> Result<()> {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let earlier = record(
        &commands,
        Capture::new("Adopt the queue", evidence_plan("throughput was measured")),
    )?;
    let later = record(
        &commands,
        Capture::new("Size the queue", evidence_plan("peak was 8k")),
    )?;
    assert_eq!(
        ids(&listed(&ledger)?),
        vec![earlier.as_str(), later.as_str()]
    );

    commands.ground_decision_with_plan(
        AGENT,
        &later,
        &GroundingPlan {
            premise_decision_ids: vec![earlier.clone()],
            ..GroundingPlan::default()
        },
    )?;

    assert_eq!(ids(&listed(&ledger)?), vec![earlier.as_str()]);
    Ok(())
}

#[test]
fn a_superseded_or_rejected_decision_is_not_in_force_and_a_contested_one_is() -> Result<()> {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let replaced = record(
        &commands,
        Capture::new("Old name", evidence_plan("the old name was chosen")),
    )?;
    let rejected = record(
        &commands,
        Capture {
            still_proposed: true,
            ..Capture::new("Doomed name", evidence_plan("someone proposed it"))
        },
    )?;
    commands.reject_decision(&rejected, OTHER_AGENT)?;
    let contested = record(
        &commands,
        Capture::new("Disputed name", evidence_plan("two people disagree")),
    )?;
    commands.reject_decision(&contested, OTHER_AGENT)?;
    let topics = ["naming".to_owned()];
    let labels = ["B".to_owned()];
    let plan = evidence_plan("the new name was measured");
    commands.supersede(SupersedeInput {
        actor_id: AGENT,
        old_decision_id: &replaced,
        new_title: "New name",
        new_rationale: "Rationale text long enough for the readable floor",
        topic_keys: &topics,
        option_labels: &labels,
        chosen_option_label: Some("B"),
        still_proposed: false,
        hypothesis_ids: &[],
        evidence_ids: &[],
        project: None,
        grounding: Some(&plan),
        floor: FloorInput::Stated(&binding()),
        expressed_confidence: None,
    })?;

    let results = listed(&ledger)?;

    // The superseded and the rejected decision are not in force. The replacement names the
    // decision it overturns, which is the reference a pivot has. Contested stays in force.
    assert_eq!(ids(&results), vec![contested.as_str()]);
    assert_eq!(results.items[0].status, DecisionStatus::Contested);
    Ok(())
}

#[test]
fn a_decision_that_declares_nothing_is_listed_with_an_empty_rests_on() -> Result<()> {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let option_id = commands.record_option(AGENT, "A", "Option A")?;
    let labels = ["A".to_owned()];
    let topics = ["naming".to_owned()];
    let answers = binding();
    let id = commands.propose_decision(DecisionProposalInput {
        grounding: Grounding::NotAsked,
        expressed_confidence: None,
        actor_id: AGENT,
        title: "Declared nothing",
        rationale: "Rationale text long enough for the readable floor",
        topic_keys: &topics,
        option_ids: std::slice::from_ref(&option_id),
        option_labels: &labels,
        chosen_option_id: Some(option_id.as_str()),
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

    let results = listed(&ledger)?;

    assert_eq!(ids(&results), vec![id.as_str()]);
    assert!(results.items[0].rests_on.is_empty());
    Ok(())
}

#[test]
fn pages_follow_a_position_and_never_skip_or_repeat_across_changes() -> Result<()> {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let mut recorded = Vec::new();
    for title in ["One", "Two", "Three"] {
        recorded.push(record(
            &commands,
            Capture::new(title, evidence_plan(&format!("{title} was measured"))),
        )?);
    }

    let first = page(&ledger, 1, None)?;
    assert_eq!(ids(&first), vec![recorded[0].as_str()]);
    assert_eq!(first.total_matches, 3);
    let cursor = first.next_cursor.clone().expect("more follow");

    // The decision the first page ended on is answered, and a newer one is recorded, between calls.
    commands.ground_decision_with_plan(
        OTHER_AGENT,
        &recorded[0],
        &evidence_plan("answered between pages"),
    )?;
    let newer = record(
        &commands,
        Capture::new("Four", evidence_plan("Four was measured")),
    )?;

    let second = page(&ledger, 2, Some(&cursor))?;
    assert_eq!(
        ids(&second),
        vec![recorded[1].as_str(), recorded[2].as_str()]
    );
    let cursor = second.next_cursor.clone().expect("the newer one follows");
    let third = page(&ledger, 2, Some(&cursor))?;
    assert_eq!(ids(&third), vec![newer.as_str()]);
    assert_eq!(third.next_cursor, None);
    Ok(())
}

#[test]
fn a_cursor_this_list_did_not_return_is_refused() {
    let ledger = InMemoryEventLedger::new();

    let error = page(&ledger, 0, Some("7")).expect_err("a bare offset is not a position");

    assert!(error.to_string().contains("cursor"), "{error}");
}
