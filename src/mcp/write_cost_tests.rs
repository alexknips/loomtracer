//! A write's cost must not grow with the part of the ledger it has no business reading
//! (hivemind-f38ka).
//!
//! On the city cell the ledger is 130k events and 127k of them are raw `ingest.batch_received`
//! transcripts. `capture_decision` and `supersede_decision` over MCP took about two minutes
//! there, so every agent's client gave up on a write that was recorded: each existence check
//! walked every event (about 15 passes per capture) and a named premise replayed the whole
//! ledger into a fresh graph. These tests count what the core's reads materialise and hold a
//! capture and a supersede to "no transcript, and no more events than with a quarter of them".

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use super::core::{
    capture_decision, supersede_decision, CaptureDecisionArgs, CoreError, GraphSource,
    LedgerHandle, LedgerProvider, SupersedeDecisionArgs,
};
use crate::commands::Commands;
use crate::events::{Event, EventId, EventType, TenantId};
use crate::ledger::{EventLedger, InMemoryEventLedger};
use crate::projector::memory::MemoryGraph;
use crate::projector::rebuild_graph_for_tenant;

const ACTOR: &str = "agent:test:write-cost";

/// What the ledger handed back, by event type, since the last [`Reads::reset`].
#[derive(Default)]
struct Reads {
    by_type: Mutex<Vec<(EventType, usize)>>,
}

impl Reads {
    fn note(&self, events: &[Event]) {
        let mut by_type = self.by_type.lock().expect("reads lock"); // ubs:ignore: test-only
        for event in events {
            match by_type
                .iter_mut()
                .find(|(seen, _)| *seen == event.event_type)
            {
                Some((_, count)) => *count += 1,
                None => by_type.push((event.event_type, 1)),
            }
        }
    }

    fn reset(&self) {
        self.by_type.lock().expect("reads lock").clear(); // ubs:ignore: test-only
    }

    fn of(&self, event_type: EventType) -> usize {
        let by_type = self.by_type.lock().expect("reads lock"); // ubs:ignore: test-only
        by_type
            .iter()
            .find(|(seen, _)| *seen == event_type)
            .map_or(0, |(_, count)| *count)
    }

    fn total(&self) -> usize {
        let by_type = self.by_type.lock().expect("reads lock"); // ubs:ignore: test-only
        by_type.iter().map(|(_, count)| count).sum()
    }
}

/// An in-memory ledger that counts every event a read returns. `read_types_for_tenant` counts
/// only what it returns, the way the SQL ledgers filter in the query.
#[derive(Clone, Default)]
struct CountingLedger {
    inner: Arc<InMemoryEventLedger>,
    reads: Arc<Reads>,
}

impl EventLedger for CountingLedger {
    fn bound_tenant(&self) -> TenantId {
        self.inner.bound_tenant()
    }

    fn append_for_tenant(&self, tenant_id: &TenantId, event: Event) -> crate::Result<EventId> {
        self.inner.append_for_tenant(tenant_id, event)
    }

    fn read_for_tenant(
        &self,
        tenant_id: &TenantId,
        offset: EventId,
        limit: usize,
    ) -> crate::Result<Vec<Event>> {
        let events = self.inner.read_for_tenant(tenant_id, offset, limit)?;
        self.reads.note(&events);
        Ok(events)
    }

    fn replay_from_for_tenant(
        &self,
        tenant_id: &TenantId,
        offset: EventId,
        callback: &mut dyn FnMut(&Event) -> crate::Result<()>,
    ) -> crate::Result<()> {
        self.inner
            .replay_from_for_tenant(tenant_id, offset, &mut |event| {
                self.reads.note(std::slice::from_ref(event));
                callback(event)
            })
    }

    fn latest_offset_for_tenant(&self, tenant_id: &TenantId) -> crate::Result<EventId> {
        self.inner.latest_offset_for_tenant(tenant_id)
    }

    fn read_types_for_tenant(
        &self,
        tenant_id: &TenantId,
        types: &[EventType],
        omit_payload_keys: &[&str],
        offset: EventId,
        limit: usize,
    ) -> crate::Result<Vec<Event>> {
        let events =
            self.inner
                .read_types_for_tenant(tenant_id, types, omit_payload_keys, offset, limit)?;
        self.reads.note(&events);
        Ok(events)
    }
}

/// The transport double: hands the core the counting ledger and, when `cached`, a graph source
/// that serves a graph current as of the ask without reading through the counter, the way the
/// HTTP server's cache catches up from its last offset. Counts its asks.
struct Provider {
    ledger: CountingLedger,
    cached: bool,
    graph_asks: Arc<AtomicUsize>,
}

impl Provider {
    fn replaying(ledger: &CountingLedger) -> Self {
        Self {
            ledger: ledger.clone(),
            cached: false,
            graph_asks: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn cached(ledger: &CountingLedger) -> Self {
        Self {
            cached: true,
            ..Self::replaying(ledger)
        }
    }
}

impl LedgerProvider for Provider {
    type Ledger = CountingLedger;

    fn ledger(&self) -> Result<LedgerHandle<Self::Ledger>, CoreError> {
        let graph_source = self.cached.then(|| {
            let asks = Arc::clone(&self.graph_asks);
            Box::new(move |ledger: &CountingLedger, tenant_id: &TenantId| {
                asks.fetch_add(1, Ordering::SeqCst);
                let graph = MemoryGraph::default();
                rebuild_graph_for_tenant(ledger.inner.as_ref(), tenant_id, &graph)?;
                Ok(Arc::new(graph))
            }) as GraphSource<CountingLedger>
        });
        Ok(LedgerHandle {
            ledger: self.ledger.clone(),
            tenant_id: TenantId::local(),
            graph_source,
        })
    }
}

fn args_of(value: &Value) -> serde_json::Map<String, Value> {
    value.as_object().cloned().expect("args are an object") // ubs:ignore: test-only
}

fn capture(provider: &Provider, args: &Value) -> Value {
    let args = CaptureDecisionArgs::from_json(&args_of(args), ACTOR.to_owned())
        .expect("capture args parse"); // ubs:ignore: test-only
    capture_decision(provider, args)
        .expect("capture succeeds") // ubs:ignore: test-only
        .into_value()
}

fn supersede(provider: &Provider, args: &Value) -> Value {
    let args = SupersedeDecisionArgs::from_json(&args_of(args), ACTOR.to_owned())
        .expect("supersede args parse"); // ubs:ignore: test-only
    supersede_decision(provider, args)
        .expect("supersede succeeds") // ubs:ignore: test-only
        .into_value()
}

fn floor() -> Value {
    json!({
        "binds": {"level": "something", "statement": "Later work builds on it"},
        "undo_cost": {"level": "cheap"},
    })
}

fn with_floor(mut args: Value) -> Value {
    if let (Some(args), Some(floor)) = (args.as_object_mut(), floor().as_object()) {
        args.extend(floor.clone());
    }
    args
}

fn append_transcript(ledger: &CountingLedger, n: usize) {
    let event = Event {
        tenant_id: TenantId::local(),
        event_id: None,
        event_uuid: uuid::Uuid::new_v4(),
        correlation_id: None,
        causation_event_id: None,
        event_type: EventType::IngestBatchReceived,
        actor_id: "agent:test:shipper".to_owned(),
        source: Default::default(),
        source_ref: None,
        payload: json!({
            "batch_id": format!("batch-{n}"),
            "session_id": format!("session-{}", n % 20),
            "agent_tool": "claude",
            "turns": [{ "turn_id": "t1", "role": "user", "text": format!("text of batch {n}"),
                        "truncated": false }],
        }),
        ts: None,
    };
    ledger
        .append_for_tenant(&TenantId::local(), event)
        .expect("append transcript"); // ubs:ignore: test-only
}

/// A ledger with a registered project, one decision to rest on, and `transcripts` raw
/// transcripts after it; returns the ledger and the premise's id. Reads are reset.
fn ledger_with_transcripts(transcripts: usize) -> (CountingLedger, String) {
    let ledger = CountingLedger::default();
    let commands = Commands::new(&ledger);
    commands
        .register_project("human:seed", "billing", None, None)
        .expect("register project"); // ubs:ignore: test-only
    commands
        .declare_project_topic("human:seed", "billing", "billing")
        .expect("declare topic"); // ubs:ignore: test-only
    let premise = capture(
        &Provider::replaying(&ledger),
        &with_floor(json!({
            "title": "Queue billing events",
            "rationale": "Bounded retries avoid unbounded backlog growth",
            "topic_keys": ["billing"],
            "project": "billing",
            "options": [{"label": "queue"}],
            "grounding": [{"kind": "bet"}],
        })),
    )["decision_id"]
        .as_str()
        .expect("premise id") // ubs:ignore: test-only
        .to_owned();
    for n in 0..transcripts {
        append_transcript(&ledger, n);
    }
    ledger.reads.reset();
    (ledger, premise)
}

/// A capture that exercises every check a real one does: a registered project and its topic
/// vocabulary, a premise decision named by id, new evidence, a question, a chosen option and a
/// rationale that cites a decision id.
fn capture_with_a_premise(premise: &str) -> Value {
    with_floor(json!({
        "title": "Stream billing events instead",
        "rationale": format!("Follows {premise}; the queue fell behind under load"),
        "topic_keys": ["billing"],
        "project": "billing",
        "options": [{"label": "queue"}, {"label": "stream"}],
        "chosen_option_label": "stream",
        "grounding": [
            {"kind": "decision", "decision_id": premise},
            {"kind": "evidence", "content": "backlog reached 40k", "source": "dashboard"},
        ],
        "question": "How do we move billing events?",
    }))
}

/// The same capture resting on evidence alone, so no premise decision needs a graph; the
/// rationale still cites a decision id, which is read from the ledger by literal id.
fn capture_without_a_premise(cited: &str) -> Value {
    with_floor(json!({
        "title": "Stream billing events instead",
        "rationale": format!("Mentions {cited}; the queue fell behind under load"),
        "topic_keys": ["billing"],
        "project": "billing",
        "options": [{"label": "queue"}, {"label": "stream"}],
        "chosen_option_label": "stream",
        "grounding": [{"kind": "evidence", "content": "backlog reached 40k", "source": "dashboard"}],
        "question": "How do we move billing events?",
    }))
}

fn supersede_resting_on(old: &str, premise: &str) -> Value {
    with_floor(json!({
        "old_decision_id": old,
        "title": "Batch billing events nightly",
        "rationale": "Streaming cost more than the delay it saved",
        "topic_keys": ["billing"],
        "options": [{"label": "batch"}],
        "chosen_option_label": "batch",
        "grounding": [{"kind": "decision", "decision_id": premise}],
    }))
}

/// What a write read: the events its reads returned, and how many of them were transcripts.
#[derive(Debug, PartialEq, Eq)]
struct Cost {
    events: usize,
    transcripts: usize,
}

fn cost_of(ledger: &CountingLedger) -> Cost {
    let cost = Cost {
        events: ledger.reads.total(),
        transcripts: ledger.reads.of(EventType::IngestBatchReceived),
    };
    ledger.reads.reset();
    cost
}

/// A capture with a premise, then a supersede of it, on a ledger the transport serves a current
/// graph for (the HTTP server's cache). Returns what each read.
fn cost_with_a_cached_graph(transcripts: usize) -> (Cost, Cost) {
    let (ledger, premise) = ledger_with_transcripts(transcripts);
    let provider = Provider::cached(&ledger);

    let captured = capture(&provider, &capture_with_a_premise(&premise));
    let captured_id = captured["decision_id"]
        .as_str()
        .unwrap_or_else(|| panic!("the capture recorded: {captured}")) // ubs:ignore: test-only
        .to_owned();
    let capture_cost = cost_of(&ledger);

    let replaced = supersede(&provider, &supersede_resting_on(&captured_id, &premise));
    assert!(
        replaced["new_decision_id"].is_string(),
        "the supersede recorded: {replaced}"
    );
    (capture_cost, cost_of(&ledger))
}

/// A capture that needs no graph, on a transport that would replay the ledger to build one.
fn cost_without_a_premise(transcripts: usize) -> Cost {
    let (ledger, premise) = ledger_with_transcripts(transcripts);
    let provider = Provider::replaying(&ledger);

    let captured = capture(&provider, &capture_without_a_premise(&premise));
    assert!(
        captured["decision_id"].is_string(),
        "the capture recorded: {captured}"
    );
    cost_of(&ledger)
}

#[test]
fn the_counter_sees_a_transcript_when_something_reads_one() {
    let (ledger, _) = ledger_with_transcripts(5);
    let page = ledger
        .read_for_tenant(&TenantId::local(), 0, 1024)
        .expect("read a page"); // ubs:ignore: test-only
    assert!(!page.is_empty());
    assert_eq!(ledger.reads.of(EventType::IngestBatchReceived), 5);
}

#[test]
fn a_capture_that_needs_no_graph_never_reads_a_transcript() {
    let cost = cost_without_a_premise(300);
    assert_eq!(
        cost.transcripts, 0,
        "a capture read {} raw transcripts; on the city cell that is every transcript, a pass \
         per existence check ({cost:?})",
        cost.transcripts
    );
}

#[test]
fn a_capture_with_a_premise_and_a_supersede_never_read_a_transcript_given_a_current_graph() {
    let (capture_cost, supersede_cost) = cost_with_a_cached_graph(300);
    assert_eq!(capture_cost.transcripts, 0, "capture: {capture_cost:?}");
    assert_eq!(
        supersede_cost.transcripts, 0,
        "supersede: {supersede_cost:?}"
    );
}

#[test]
fn a_write_costs_the_same_on_a_ledger_four_times_as_long() {
    assert_eq!(
        cost_without_a_premise(100),
        cost_without_a_premise(400),
        "a capture's reads changed with the number of transcripts"
    );
    assert_eq!(
        cost_with_a_cached_graph(100),
        cost_with_a_cached_graph(400),
        "a capture's and a supersede's reads changed with the number of transcripts"
    );
}

#[test]
fn naming_a_premise_asks_the_transport_for_its_graph_instead_of_replaying_the_ledger() {
    let (ledger, premise) = ledger_with_transcripts(50);
    let provider = Provider::cached(&ledger);

    capture(&provider, &capture_with_a_premise(&premise));

    assert!(
        provider.graph_asks.load(Ordering::SeqCst) >= 1,
        "naming a premise resolves it against the transport's graph"
    );
    assert_eq!(cost_of(&ledger).transcripts, 0);
}
