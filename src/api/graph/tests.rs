// Parent module gates this file with #[cfg(test)]; repeat the marker so UBS can filter test-only assertions.
#[cfg(test)]
use super::*;

use std::sync::mpsc;
use std::sync::Mutex;
use std::time::Duration;

use chrono::Utc;
use serde_json::json;
use uuid::Uuid;

use crate::events::{Event, EventId, EventSource, EventType};
use crate::ledger::InMemoryEventLedger;

/// A ledger that takes in the events it was given the moment a replay starts: the ones appended
/// after `get_cached_graph` read the ledger's latest offset and before its replay read the
/// ledger, which is how a write racing a refresh lands in the graph above the offset read.
struct AppendsWhenReplayStarts {
    inner: InMemoryEventLedger,
    late: Mutex<Vec<Event>>,
}

impl AppendsWhenReplayStarts {
    fn new(early: Vec<Event>, late: Vec<Event>) -> Self {
        let inner = InMemoryEventLedger::new();
        for event in early {
            inner.append(event).expect("append an early event");
        }
        Self {
            inner,
            late: Mutex::new(late),
        }
    }
}

impl EventLedger for AppendsWhenReplayStarts {
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
        self.inner.read_for_tenant(tenant_id, offset, limit)
    }

    fn replay_from_for_tenant(
        &self,
        tenant_id: &TenantId,
        offset: EventId,
        callback: &mut dyn FnMut(&Event) -> crate::Result<()>,
    ) -> crate::Result<()> {
        let late = std::mem::take(&mut *self.late.lock().expect("late lock"));
        for event in late {
            self.inner.append_for_tenant(tenant_id, event)?;
        }
        self.inner
            .replay_from_for_tenant(tenant_id, offset, callback)
    }

    fn latest_offset_for_tenant(&self, tenant_id: &TenantId) -> crate::Result<EventId> {
        self.inner.latest_offset_for_tenant(tenant_id)
    }
}

fn event(event_type: EventType, payload: serde_json::Value) -> Event {
    Event {
        tenant_id: Default::default(),
        event_id: None,
        event_uuid: Uuid::new_v4(),
        correlation_id: Some("graph-cache-test".to_owned()),
        causation_event_id: None,
        event_type,
        actor_id: "actor:alice".to_owned(),
        source: EventSource::Agent,
        source_ref: Some("graph-cache-test".to_owned()),
        payload,
        ts: Some(Utc::now()),
    }
}

fn proposal(decision_id: &str, title: &str) -> Event {
    event(
        EventType::DecisionProposed,
        json!({
            "decision_id": decision_id,
            "title": title,
            "rationale": "Simpler to reason about at our scale",
            "topic_keys": ["pricing"],
            "option_ids": [],
            "chosen_option_id": null,
            "hypothesis_ids": [],
            "evidence_ids": []
        }),
    )
}

/// An assessment row nobody can parse: the replay skips it and reports it.
fn malformed_assessment() -> Event {
    event(
        EventType::DecisionScored,
        json!({
            "schema_version": 2,
            "decision_id": "decision:early",
            "model": "model-x",
            "prompt_version": "prompt-x",
            "dimensions": {"framing": "not an answer"}
        }),
    )
}

fn empty_cache() -> GraphCache {
    GraphCache::default()
}

fn refresh(ledger: &impl EventLedger, cache: &GraphCache) -> Arc<MemoryGraph> {
    get_cached_graph(ledger, &TenantId::local(), cache).expect("refresh the graph cache")
}

fn cached_offset(cache: &GraphCache) -> Option<EventId> {
    cache
        .slot(&TenantId::local())
        .expect("the tenant's slot")
        .published()
        .expect("read the published graph")
        .map(|(offset, _)| offset)
}

fn slug_of(graph: &MemoryGraph, decision_id: &str) -> Option<GraphValue> {
    let rows = graph
        .query(
            "MATCH (node:`Decision` {id: $id}) RETURN node.id AS id, node.slug AS slug ORDER BY node.id;",
            &GraphParams::from([(
                "id".to_owned(),
                GraphValue::String(decision_id.to_owned()),
            )]),
        )
        .expect("query the decision's slug");
    rows.into_iter()
        .next()
        .and_then(|row| row.get("slug").cloned())
}

#[test]
fn an_event_appended_during_a_refresh_is_not_applied_again_by_the_next() {
    let cache = empty_cache();
    let ledger = AppendsWhenReplayStarts::new(
        vec![proposal("decision:early", "Pick a database")],
        vec![proposal("decision:late", "Use per-seat pricing")],
    );

    // The refresh reads the latest offset (1), then the late event lands (2), then the replay
    // runs to the end of the ledger and so applies both.
    let first = refresh(&ledger, &cache);
    let bare = Some(GraphValue::String("use-per-seat-pricing".to_owned()));
    assert_eq!(slug_of(&first, "decision:late"), bare);
    assert_eq!(
        cached_offset(&cache),
        Some(2),
        "the cache records the last event the replay applied, not the offset read before it"
    );

    // Nothing new has been appended, so the next refresh has nothing to apply: it is the same
    // graph, and the late decision still has the link it was first given.
    let second = refresh(&ledger, &cache);
    assert!(Arc::ptr_eq(&first, &second), "nothing was left to replay");
    assert_eq!(slug_of(&second, "decision:late"), bare);
}

#[test]
fn a_refresh_after_a_late_event_still_applies_what_comes_after_it() {
    let cache = empty_cache();
    let ledger = AppendsWhenReplayStarts::new(
        vec![proposal("decision:early", "Pick a database")],
        vec![proposal("decision:late", "Use per-seat pricing")],
    );
    let first = refresh(&ledger, &cache);

    ledger
        .append_for_tenant(
            &TenantId::local(),
            proposal("decision:next", "Pick a cache"),
        )
        .expect("append after the refresh");
    let second = refresh(&ledger, &cache);

    assert!(
        !Arc::ptr_eq(&first, &second),
        "an event after the cache was applied"
    );
    assert_eq!(cached_offset(&cache), Some(3));
    assert_eq!(
        slug_of(&second, "decision:late"),
        Some(GraphValue::String("use-per-seat-pricing".to_owned())),
        "the late decision keeps its link through the next refresh"
    );
    assert_eq!(
        slug_of(&second, "decision:next"),
        Some(GraphValue::String("pick-a-cache".to_owned()))
    );
}

#[test]
fn an_unreadable_annotation_appended_during_a_refresh_is_reported_once() {
    let cache = empty_cache();
    let ledger = AppendsWhenReplayStarts::new(
        vec![proposal("decision:early", "Pick a database")],
        vec![malformed_assessment()],
    );

    let first = refresh(&ledger, &cache);
    assert_eq!(first.unreadable_annotations().len(), 1);

    let second = refresh(&ledger, &cache);
    assert_eq!(
        second.unreadable_annotations().len(),
        1,
        "the next refresh does not read the same bad row again and note it twice"
    );
}

/// A ledger whose replay stops partway and waits to be let go: a refresh that is in progress for
/// as long as the test wants it to be.
struct StallsInReplay {
    inner: InMemoryEventLedger,
    entered: Mutex<mpsc::Sender<()>>,
    release: Mutex<mpsc::Receiver<()>>,
}

impl EventLedger for StallsInReplay {
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
        self.inner.read_for_tenant(tenant_id, offset, limit)
    }

    fn replay_from_for_tenant(
        &self,
        tenant_id: &TenantId,
        offset: EventId,
        callback: &mut dyn FnMut(&Event) -> crate::Result<()>,
    ) -> crate::Result<()> {
        self.entered
            .lock()
            .expect("entered lock")
            .send(())
            .expect("tell the test the replay started");
        self.release
            .lock()
            .expect("release lock")
            .recv_timeout(Duration::from_secs(30))
            .expect("the test lets the replay go");
        self.inner
            .replay_from_for_tenant(tenant_id, offset, callback)
    }

    fn latest_offset_for_tenant(&self, tenant_id: &TenantId) -> crate::Result<EventId> {
        self.inner.latest_offset_for_tenant(tenant_id)
    }
}

/// A ledger whose replay takes a moment, so refreshes that race overlap instead of finishing one
/// after another by luck, and which notes which thread replayed from which offset.
struct SlowReplay {
    inner: InMemoryEventLedger,
    replays: Mutex<Vec<(std::thread::ThreadId, EventId)>>,
}

impl EventLedger for SlowReplay {
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
        self.inner.read_for_tenant(tenant_id, offset, limit)
    }

    fn replay_from_for_tenant(
        &self,
        tenant_id: &TenantId,
        offset: EventId,
        callback: &mut dyn FnMut(&Event) -> crate::Result<()>,
    ) -> crate::Result<()> {
        self.replays
            .lock()
            .expect("replays lock")
            .push((std::thread::current().id(), offset));
        std::thread::sleep(Duration::from_millis(2));
        self.inner
            .replay_from_for_tenant(tenant_id, offset, callback)
    }

    fn latest_offset_for_tenant(&self, tenant_id: &TenantId) -> crate::Result<EventId> {
        self.inner.latest_offset_for_tenant(tenant_id)
    }
}

#[test]
fn a_refresh_in_progress_for_one_tenant_does_not_hold_up_another_tenants_reads() {
    let cache = empty_cache();
    let tenant_a = TenantId::new("tenant-a").expect("tenant id");
    let tenant_b = TenantId::new("tenant-b").expect("tenant id");
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let ledger_a = StallsInReplay {
        inner: InMemoryEventLedger::new(),
        entered: Mutex::new(entered_tx),
        release: Mutex::new(release_rx),
    };
    ledger_a
        .append_for_tenant(&tenant_a, proposal("decision:a", "Pick a database"))
        .expect("append to tenant a");
    let ledger_b = InMemoryEventLedger::new();
    ledger_b
        .append_for_tenant(&tenant_b, proposal("decision:b", "Use per-seat pricing"))
        .expect("append to tenant b");
    let warm_b = get_cached_graph(&ledger_b, &tenant_b, &cache).expect("warm tenant b");

    std::thread::scope(|scope| {
        let refresh_a = scope.spawn(|| get_cached_graph(&ledger_a, &tenant_a, &cache));
        entered_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("tenant a's refresh started its replay");

        // Tenant a is mid-refresh. Tenant b reads its warm graph, then refreshes after a write,
        // then a tenant nobody has read yet builds its graph: none of them waits for tenant a.
        let (done_tx, done_rx) = mpsc::channel();
        let (ledger_b, tenant_b, cache) = (&ledger_b, &tenant_b, &cache);
        let readers = scope.spawn(move || {
            let hit = get_cached_graph(ledger_b, tenant_b, cache);
            ledger_b
                .append_for_tenant(tenant_b, proposal("decision:b2", "Pick a cache"))
                .expect("append to tenant b");
            let refreshed = get_cached_graph(ledger_b, tenant_b, cache);
            let tenant_c = TenantId::new("tenant-c").expect("tenant id");
            let cold = get_cached_graph(&InMemoryEventLedger::new(), &tenant_c, cache);
            done_tx
                .send((hit, refreshed, cold))
                .expect("report the reads");
        });
        let reads = done_rx.recv_timeout(Duration::from_secs(10));
        release_tx.send(()).expect("let tenant a's replay go");
        let (hit, refreshed, cold) =
            reads.expect("tenant b's reads finished while tenant a's refresh was in progress");
        readers.join().expect("the readers finished");

        assert!(Arc::ptr_eq(&hit.expect("tenant b hit"), &warm_b));
        let refreshed = refreshed.expect("tenant b refresh");
        assert_eq!(
            slug_of(&refreshed, "decision:b2"),
            Some(GraphValue::String("pick-a-cache".to_owned()))
        );
        cold.expect("a tenant with no events");
        let graph_a = refresh_a
            .join()
            .expect("tenant a's refresh finished")
            .expect("tenant a's graph");
        assert_eq!(
            slug_of(&graph_a, "decision:a"),
            Some(GraphValue::String("pick-a-database".to_owned()))
        );
    });
}

#[test]
fn a_reader_of_the_same_tenant_waits_for_the_refresh_and_does_not_replay_again() {
    let cache = empty_cache();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let ledger = StallsInReplay {
        inner: InMemoryEventLedger::new(),
        entered: Mutex::new(entered_tx),
        release: Mutex::new(release_rx),
    };
    ledger
        .append_for_tenant(
            &TenantId::local(),
            proposal("decision:a", "Pick a database"),
        )
        .expect("append");

    std::thread::scope(|scope| {
        let first = scope.spawn(|| refresh(&ledger, &cache));
        entered_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the first refresh started its replay");
        let second = scope.spawn(|| refresh(&ledger, &cache));
        // A second replay would announce itself on `entered` and then wait for a release that
        // never comes; a reader that waits for the first refresh just takes what it published.
        release_tx.send(()).expect("let the replay go");
        let first = first.join().expect("first refresh");
        let second = second.join().expect("second refresh");
        assert!(Arc::ptr_eq(&first, &second));
        assert!(
            entered_rx.try_recv().is_err(),
            "the second reader replayed the ledger again"
        );
    });
}

#[test]
fn refreshes_racing_on_one_tenant_apply_every_event_exactly_once() {
    let cache = empty_cache();
    let ledger = SlowReplay {
        inner: InMemoryEventLedger::new(),
        replays: Mutex::new(Vec::new()),
    };
    let writers = 8_usize;
    let rounds = 5_usize;

    // Each event is an assessment nobody can parse, which the graph notes once per time it is
    // applied: an event applied twice shows as an extra note, one lost to a refresh that
    // published over a newer graph as a missing one.
    std::thread::scope(|scope| {
        for _ in 0..writers {
            let (ledger, cache) = (&ledger, &cache);
            scope.spawn(move || {
                for _ in 0..rounds {
                    ledger
                        .append_for_tenant(&TenantId::local(), malformed_assessment())
                        .expect("append");
                    refresh(ledger, cache);
                }
            });
        }
    });
    let graph = refresh(&ledger, &cache);

    // A refresh starts from the offset the one before it published, so no two refreshes replay
    // from the same one: the same thread does twice, to find the batches a classification names
    // and to project, but never two threads.
    let mut replayers_by_offset: std::collections::BTreeMap<EventId, Vec<std::thread::ThreadId>> =
        std::collections::BTreeMap::new();
    for (thread, offset) in ledger.replays.lock().expect("replays lock").iter() {
        let threads = replayers_by_offset.entry(*offset).or_default();
        if !threads.contains(thread) {
            threads.push(*thread);
        }
    }
    for (offset, threads) in &replayers_by_offset {
        assert_eq!(
            threads.len(),
            1,
            "{} refreshes replayed from offset {offset} side by side",
            threads.len()
        );
    }

    let events = (writers * rounds) as EventId;
    assert_eq!(cached_offset(&cache), Some(events));
    assert_eq!(
        graph.unreadable_annotations().len(),
        writers * rounds,
        "every event was applied exactly once"
    );
}
