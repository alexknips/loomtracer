// Parent module gates this file with #[cfg(test)]; repeat the marker so UBS can filter test-only assertions.
#[cfg(test)]
use super::*;

use std::collections::HashMap;
use std::sync::{Mutex, RwLock};

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
        let late = std::mem::take(&mut *self.late.lock().expect("late lock")); // ubs:ignore: test-only
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
    RwLock::new(HashMap::new())
}

fn refresh(ledger: &impl EventLedger, cache: &GraphCache) -> Arc<MemoryGraph> {
    get_cached_graph(ledger, &TenantId::local(), cache).expect("refresh the graph cache")
}

fn cached_offset(cache: &GraphCache) -> Option<EventId> {
    let guard = cache.read().expect("cache lock"); // ubs:ignore: test-only
    guard.get(&TenantId::local()).map(|(offset, _)| *offset)
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
