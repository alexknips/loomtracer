//! What a classified decision is projected with from the batches it was classified from: when
//! they were said, which capture session shipped them, and who the session was held with.
//!
//! A received batch never reaches the graph, and the projector reads one event at a time, so a
//! replay keeps the newest turn time and the session of every batch a classification names. A
//! batch counts when it was received before the classification that names it, whether the replay
//! started at the first event or later. A replay reads the received batches as the few facts a
//! classification reads of them ([`ReceivedBatchFacts`]) and projects whole only the newest of
//! each actor ([`received_to_project`]): they are most of a cell's ledger, and their transcripts
//! are never decoded.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use chrono::{DateTime, Utc};

use crate::events::{
    classified_batch_ids, session_agent_actor, Event, EventId, EventType, TenantId,
};
use crate::ledger::{EventLedger, ReceivedBatchFacts};
use crate::Result;

/// What a replay knows of the batches it has passed: the newest turn time of each whose turns
/// carry one, the session of each that names one, and who submitted each and with which agent
/// tool. A batch with no turn time has no time entry, and a batch that names no session has no
/// session entry: nothing is guessed for either.
#[derive(Debug, Default)]
pub(super) struct ReceivedBatches {
    turn_times: HashMap<String, DateTime<Utc>>,
    sessions: HashMap<String, String>,
    submitters: HashMap<String, Submitter>,
}

/// Who shipped a received batch: the actor on the event, and the agent tool it names.
#[derive(Debug)]
struct Submitter {
    actor_id: String,
    agent_tool: String,
}

/// The conversation a classification was drawn from, as the batches it covers state it. Each part
/// is empty when the ledger received none of the batches (a `loomtracer emit` capture names a fresh
/// batch id): who was in a conversation is never guessed.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct ClassifiedSession {
    /// The capture sessions that shipped the batches, sorted and without repeats.
    pub(super) session_ids: Vec<String>,
    /// Whoever submitted the first covered batch that was received: the one who started the
    /// session.
    pub(super) initiator: Option<String>,
    /// Every covered batch's submitter and the agent that ran beside them
    /// ([`session_agent_actor`]), in the order the batches are named, each once.
    pub(super) participants: Vec<String>,
}

impl ReceivedBatches {
    /// Takes in a received batch. A batch received more than once keeps the newest of its turn
    /// times and the first session it named.
    pub(super) fn note(&mut self, facts: &ReceivedBatchFacts) {
        let Some(batch_id) = facts.batch_id.as_deref() else {
            return;
        };
        if let Some(newest) = facts.newest_turn_time {
            self.turn_times
                .entry(batch_id.to_owned())
                .and_modify(|current| *current = (*current).max(newest))
                .or_insert(newest);
        }
        if let Some(session_id) = facts.session_id.as_deref() {
            self.sessions
                .entry(batch_id.to_owned())
                .or_insert_with(|| session_id.to_owned());
        }
        self.submitters
            .entry(batch_id.to_owned())
            .or_insert_with(|| Submitter {
                actor_id: facts.actor_id.clone(),
                agent_tool: facts.agent_tool.clone(),
            });
    }

    /// The newest turn time across `batch_ids`, `None` when none of them carries one.
    pub(super) fn newest(&self, batch_ids: &[String]) -> Option<DateTime<Utc>> {
        batch_ids
            .iter()
            .filter_map(|batch_id| self.turn_times.get(batch_id.as_str()).copied())
            .max()
    }

    /// The conversation `batch_ids` were shipped from: their sessions, who started it and who
    /// took part.
    pub(super) fn session(&self, batch_ids: &[String]) -> ClassifiedSession {
        let session_ids: BTreeSet<&str> = batch_ids
            .iter()
            .filter_map(|batch_id| self.sessions.get(batch_id.as_str()))
            .map(String::as_str)
            .collect();
        let mut session = ClassifiedSession {
            session_ids: session_ids.into_iter().map(str::to_owned).collect(),
            ..ClassifiedSession::default()
        };
        for submitter in batch_ids
            .iter()
            .filter_map(|batch_id| self.submitters.get(batch_id.as_str()))
        {
            let agent = session_agent_actor(&submitter.actor_id, &submitter.agent_tool);
            for actor in [Some(submitter.actor_id.as_str()), agent.as_deref()]
                .into_iter()
                .flatten()
                .filter(|actor| !actor.is_empty())
            {
                if session.initiator.is_none() && actor == submitter.actor_id {
                    session.initiator = Some(actor.to_owned());
                }
                // ubs:ignore: == compares actor ids (public attribution, not secrets or tokens)
                if !session.participants.iter().any(|known| known == actor) {
                    session.participants.push(actor.to_owned());
                }
            }
        }
        session
    }
}

/// How many received batches, or classifications, one read asks for: the city cell's ~150k
/// received batches take a handful of round trips.
const PAGE: usize = 10_000;

/// What a replay of the events after `offset`, up to `through`, takes from the received batches,
/// in event order: [`Self::next_before`] hands it over as the replay reaches each event.
pub(super) struct ReceivedToProject {
    /// The batches that a classification in the replay names, wherever the ledger received them
    /// (before `offset` too: the server's graph cache replays only what is new).
    named: VecDeque<ReceivedBatchFacts>,
    /// The newest received batch of each actor after `offset`, whole.
    newest_of_actor: VecDeque<Event>,
}

/// A received batch handed to the replay.
pub(super) enum Received {
    /// A batch a classification names, to be noted before that classification reads it.
    Named(ReceivedBatchFacts),
    /// The newest batch of an actor, to be projected: the event that last wrote the actor's node.
    NewestOfActor(Event),
}

impl ReceivedToProject {
    /// The next received batch received before `event_id`, `None` when the next one is received
    /// at or after it (or there is none).
    pub(super) fn next_before(&mut self, event_id: EventId) -> Option<Received> {
        let named = self
            .named
            .front()
            .map(|facts| facts.event_id)
            .filter(|id| *id < event_id);
        let newest = self
            .newest_of_actor
            .front()
            .and_then(|event| event.event_id)
            .filter(|id| *id < event_id);
        let take_named = match (named, newest) {
            (Some(named), Some(newest)) => named <= newest,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => return None,
        };
        if take_named {
            self.named.pop_front().map(Received::Named)
        } else {
            self.newest_of_actor
                .pop_front()
                .map(Received::NewestOfActor)
        }
    }
}

/// The received batches a replay of the events after `offset`, up to `through`, takes from the
/// ledger. Of the ledger's received batches, which on a cell are 97% of its events, only two
/// kinds change the graph:
///
/// - A batch that a classification in the replay names. The classification reads the batch's turn
///   time, session and submitter, so the batch is noted before it. Only the facts of the batch
///   are read ([`EventLedger::read_received_batches_for_tenant`]), never the transcript.
/// - The newest received batch of each actor after `offset`. A batch adds nothing to the graph
///   but its submitter's Actor node, and each projection of an actor writes all of that node over,
///   so the node a replay leaves is the one its last event wrote: only the newest received batch
///   of an actor can be that event, and an older one is written over by it. These few are read
///   whole, by id, and projected like any event.
///
/// Every other received batch is neither kept nor projected.
pub(super) fn received_to_project(
    ledger: &impl EventLedger,
    tenant_id: &TenantId,
    offset: EventId,
    through: EventId,
) -> Result<ReceivedToProject> {
    let names = batches_named_after(ledger, tenant_id, offset, through)?;
    // Before `offset`, a received batch matters only to a classification that names it.
    let mut cursor = if names.is_empty() { offset } else { 0 };

    let mut named: VecDeque<ReceivedBatchFacts> = VecDeque::new();
    let mut newest_of_actor: HashMap<String, EventId> = HashMap::new();
    'pages: loop {
        let page = ledger.read_received_batches_for_tenant(tenant_id, cursor, PAGE)?;
        let page_len = page.len();
        let last = page.last().map(|facts| facts.event_id);
        for facts in page {
            if facts.event_id > through {
                break 'pages;
            }
            if facts.event_id > offset {
                match newest_of_actor.get_mut(facts.actor_id.as_str()) {
                    Some(newest) => *newest = facts.event_id,
                    None => {
                        let actor_id = facts.actor_id.clone(); // ubs:ignore: once per distinct actor, when first seen
                        newest_of_actor.insert(actor_id, facts.event_id);
                    }
                }
            }
            if facts
                .batch_id
                .as_deref()
                .is_some_and(|batch_id| names.contains(batch_id))
            {
                named.push_back(facts);
            }
        }
        match last {
            Some(last) if page_len == PAGE => cursor = last,
            _ => break,
        }
    }

    let event_ids: Vec<EventId> = newest_of_actor.into_values().collect();
    Ok(ReceivedToProject {
        named,
        newest_of_actor: ledger.read_ids_for_tenant(tenant_id, &event_ids)?.into(),
    })
}

/// The ids of the batches that the classifications after `offset`, up to `through`, name.
fn batches_named_after(
    ledger: &impl EventLedger,
    tenant_id: &TenantId,
    offset: EventId,
    through: EventId,
) -> Result<HashSet<String>> {
    let mut named: HashSet<String> = HashSet::new();
    let mut cursor = offset;
    loop {
        // `captures` is the bulk of a classification and names no batch.
        let page = ledger.read_types_for_tenant(
            tenant_id,
            &[EventType::IngestBatchClassified],
            &["captures"],
            cursor,
            PAGE,
        )?;
        for event in &page {
            if event.event_id.is_some_and(|event_id| event_id > through) {
                return Ok(named);
            }
            named.extend(classified_batch_ids(&event.payload));
        }
        match page.last().and_then(|last| last.event_id) {
            Some(last) if page.len() == PAGE => cursor = last,
            _ => return Ok(named),
        }
    }
}
