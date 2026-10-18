// Parent module gates this file with #[cfg(test)]; repeat the marker so UBS can filter test-only assertions.
#[cfg(test)]
use std::cell::Cell;

use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::commands::{
    Commands, DecisionProposalInput, DeterminedProject, FloorInput, Grounding, SupersedeInput,
};
use crate::events::{Event, EventId, EventSource, EventType, ProjectLinkKind, TenantId};
use crate::ledger::{EventLedger, InMemoryEventLedger};
use crate::projector::{
    memory::MemoryGraph, rebuild_graph, GraphParams, GraphProperties, GraphRow, GraphValue,
    GraphView, NodeKind, RelationKind,
};
use crate::Result;

/// A small ledger builder for tests about what decisions rest on: each call appends one event
/// with a fixed, increasing timestamp, so the graph a test reads is exactly the events it wrote.
///
/// Capture-time premise links carry `causation_event_id` = the proposal event, the same way
/// `Commands::propose_decision` fans them out; links added later carry none.
pub(crate) struct Scenario {
    ledger: InMemoryEventLedger,
    sequence: Cell<u128>,
}

/// A decision capture as the classifier wrote it, said by `human:alex`, with the object `stated`
/// laid over its fields: `accepted_by`, `rejected_by`, `supersedes_id`, `premised_on_ids`,
/// `source_ts` and the like.
pub(crate) fn decision_capture(title: &str, stated: Value) -> Value {
    let mut capture = json!({
        "kind": "decision",
        "title": title,
        "rationale": format!("Rationale for {title}, long enough to read on its own"),
        "topic_keys": ["capture"],
        "evidence_ids": [],
        "options": null,
        "chosen_option": null,
        "extraction_confidence": 0.9,
        "actor_id": "human:alex",
    });
    lay_over(&mut capture, stated);
    capture
}

/// An evidence capture as the classifier wrote it, with the object `stated` laid over its fields:
/// `refutes_ids`, `supports_ids`.
pub(crate) fn evidence_capture(content: &str, stated: Value) -> Value {
    let mut capture = json!({
        "kind": "evidence",
        "title": content,
        "rationale": format!("Why {content} matters, long enough to read on its own"),
        "topic_keys": ["capture"],
        "evidence_ids": [],
        "options": null,
        "chosen_option": null,
        "extraction_confidence": 0.9,
    });
    lay_over(&mut capture, stated);
    capture
}

fn lay_over(capture: &mut Value, stated: Value) {
    if let (Some(fields), Value::Object(stated)) = (capture.as_object_mut(), stated) {
        fields.extend(stated);
    }
}

pub(crate) fn ts(timestamp: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(timestamp)
        .expect("test timestamp parses")
        .with_timezone(&Utc)
}

impl Scenario {
    pub(crate) fn new() -> Self {
        Self {
            ledger: InMemoryEventLedger::new(),
            sequence: Cell::new(0),
        }
    }

    pub(crate) fn ledger(&self) -> &InMemoryEventLedger {
        &self.ledger
    }

    pub(crate) fn graph(&self) -> Result<MemoryGraph> {
        let graph = MemoryGraph::default();
        rebuild_graph(&self.ledger, &graph)?;
        Ok(graph)
    }

    fn push(
        &self,
        actor_id: &str,
        event_type: EventType,
        payload: Value,
        causation_event_id: Option<EventId>,
        timestamp: &str,
    ) -> Result<EventId> {
        self.push_with_source_ref(
            actor_id,
            event_type,
            payload,
            causation_event_id,
            timestamp,
            None,
        )
    }

    /// `push` for the event types that require a `source_ref`.
    fn push_with_source_ref(
        &self,
        actor_id: &str,
        event_type: EventType,
        payload: Value,
        causation_event_id: Option<EventId>,
        timestamp: &str,
        source_ref: Option<&str>,
    ) -> Result<EventId> {
        let sequence = self.sequence.get() + 1;
        self.sequence.set(sequence);
        self.ledger.append(Event {
            tenant_id: Default::default(),
            event_id: None,
            event_uuid: Uuid::from_u128(sequence),
            correlation_id: Some("grounding-test".to_owned()),
            causation_event_id,
            event_type,
            actor_id: actor_id.to_owned(),
            source: EventSource::Cli,
            source_ref: source_ref.map(str::to_owned),
            payload,
            ts: Some(ts(timestamp)),
        })
    }

    /// `decision.proposed` with no option, evidence or hypothesis.
    pub(crate) fn decision(
        &self,
        decision_id: &str,
        title: &str,
        actor_id: &str,
        timestamp: &str,
    ) -> Result<EventId> {
        self.decision_with(
            decision_id,
            title,
            actor_id,
            timestamp,
            false,
            &[],
            &[],
            None,
        )
    }

    /// `decision.proposed` filed under `project` (a stated one), with no option, evidence or
    /// hypothesis.
    pub(crate) fn decision_in(
        &self,
        decision_id: &str,
        title: &str,
        project: &str,
        actor_id: &str,
        timestamp: &str,
    ) -> Result<EventId> {
        self.proposal(
            actor_id,
            timestamp,
            json!({
                "decision_id": decision_id,
                "title": title,
                "rationale": format!("Rationale for {title}, long enough to read on its own"),
                "topic_keys": ["grounding"],
                "option_ids": [],
                "option_labels": [],
                "chosen_option_id": Value::Null,
                "hypothesis_ids": [],
                "evidence_ids": [],
                "project": project,
                "project_source": "stated",
            }),
        )
    }

    /// `decision.proposed` with everything a capture can name. `chosen` picks an option, so
    /// hypotheses hang off it (the capture-time shape); without one they hang off the decision.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn decision_with(
        &self,
        decision_id: &str,
        title: &str,
        actor_id: &str,
        timestamp: &str,
        chosen: bool,
        evidence_ids: &[&str],
        hypothesis_ids: &[&str],
        expressed_confidence: Option<&str>,
    ) -> Result<EventId> {
        let option_id = format!("{decision_id}:opt:a");
        let mut payload = json!({
            "decision_id": decision_id,
            "title": title,
            "rationale": format!("Rationale for {title}, long enough to read on its own"),
            "topic_keys": ["grounding"],
            "option_ids": if chosen { vec![option_id.clone()] } else { Vec::new() },
            "option_labels": if chosen { vec!["Option A".to_owned()] } else { Vec::new() },
            "chosen_option_id": if chosen { Value::String(option_id) } else { Value::Null },
            "hypothesis_ids": hypothesis_ids,
            "evidence_ids": evidence_ids,
        });
        if let Some(confidence) = expressed_confidence {
            payload["expressed_confidence"] = json!(confidence);
        }
        self.push(
            actor_id,
            EventType::DecisionProposed,
            payload,
            None,
            timestamp,
        )
    }

    /// `decision.proposed` filed under exactly `topic_keys`, for tests about what topic keys link.
    pub(crate) fn decision_topics(
        &self,
        decision_id: &str,
        title: &str,
        topic_keys: &[&str],
        actor_id: &str,
        timestamp: &str,
    ) -> Result<EventId> {
        self.proposal(
            actor_id,
            timestamp,
            json!({
                "decision_id": decision_id,
                "title": title,
                "rationale": format!("Rationale for {title}, long enough to read on its own"),
                "topic_keys": topic_keys,
                "option_ids": [],
                "option_labels": [],
                "chosen_option_id": Value::Null,
                "hypothesis_ids": [],
                "evidence_ids": [],
            }),
        )
    }

    /// One `ingest.batch_classified` event recording a decision capture per `(title, topic_keys)`
    /// out of one conversation. The decisions are `capture:<returned event id>:<index>`, and all
    /// of them carry the returned event id as their `event_origin`.
    pub(crate) fn classified_decisions(
        &self,
        decisions: &[(&str, &[&str])],
        timestamp: &str,
    ) -> Result<EventId> {
        self.classified_decisions_from_batches(
            &[&format!("session-{timestamp}:0")],
            decisions,
            timestamp,
        )
    }

    /// One `ingest.batch_received` event: batch `batch_id`, shipped by capture session
    /// `session_id`. A classification that names the batch records its decisions from that
    /// session.
    pub(crate) fn received_batch(
        &self,
        batch_id: &str,
        session_id: &str,
        timestamp: &str,
    ) -> Result<EventId> {
        self.push(
            "agent:claude:hook",
            EventType::IngestBatchReceived,
            json!({
                "batch_id": batch_id,
                "agent_tool": "claude",
                "session_id": session_id,
                "turns": [],
            }),
            None,
            timestamp,
        )
    }

    /// [`Self::classified_decisions`] for a classification that names `batch_ids`; their sessions
    /// are those of the batches received (`Self::received_batch`) under these ids.
    pub(crate) fn classified_decisions_from_batches(
        &self,
        batch_ids: &[&str],
        decisions: &[(&str, &[&str])],
        timestamp: &str,
    ) -> Result<EventId> {
        let captures: Vec<Value> = decisions
            .iter()
            .map(|(title, topic_keys)| {
                json!({
                    "kind": "decision",
                    "title": title,
                    "rationale": format!("Rationale for {title}, long enough to read on its own"),
                    "topic_keys": topic_keys,
                    "evidence_ids": [],
                    "options": Value::Null,
                    "chosen_option": Value::Null,
                    "extraction_confidence": 0.9,
                })
            })
            .collect();
        self.push(
            "agent:claude:classifier",
            EventType::IngestBatchClassified,
            json!({
                "batch_id": batch_ids[0],
                "batch_ids": batch_ids,
                "classifier_model": "test-classifier",
                "schema_version": "2",
                "captures": captures,
            }),
            None,
            timestamp,
        )
    }

    /// `decision.proposed` from a hand-built payload, for tests that control exactly which
    /// options, question and evidence a record carries (see `record_payload`).
    pub(crate) fn proposal(
        &self,
        actor_id: &str,
        timestamp: &str,
        payload: Value,
    ) -> Result<EventId> {
        self.push(
            actor_id,
            EventType::DecisionProposed,
            payload,
            None,
            timestamp,
        )
    }

    /// `ingest.batch_classified` recorded by `recorder`, holding one `decision` capture whose
    /// classifier-named actor is `capture_actor_id` (no acceptor, no rejecter). Returns the
    /// capture's node id, `capture:<event>:0`.
    pub(crate) fn classified_decision(
        &self,
        recorder: &str,
        timestamp: &str,
        title: &str,
        capture_actor_id: Option<&str>,
    ) -> Result<String> {
        self.classified_decision_said_at(recorder, timestamp, title, capture_actor_id, None)
    }

    /// [`Self::classified_decision`] whose capture names the time of the turn it was said in
    /// (`source_ts`), which can be days before the classification at `timestamp`.
    pub(crate) fn classified_decision_said_at(
        &self,
        recorder: &str,
        timestamp: &str,
        title: &str,
        capture_actor_id: Option<&str>,
        said_at: Option<&str>,
    ) -> Result<String> {
        let event_id = self.push(
            recorder,
            EventType::IngestBatchClassified,
            json!({
                "batch_id": "batch:1",
                "classifier_model": "claude-haiku-4-5-20251001",
                "schema_version": "2",
                "captures": [{
                    "kind": "decision",
                    "title": title,
                    "rationale": format!("Rationale for {title}, long enough to read on its own"),
                    "topic_keys": ["capture"],
                    "evidence_ids": [],
                    "options": ["A", "B"],
                    "chosen_option": "A",
                    "extraction_confidence": 0.9,
                    "actor_id": capture_actor_id,
                    "source_ts": said_at,
                    "accepted_by": [],
                    "rejected_by": []
                }]
            }),
            None,
            timestamp,
        )?;
        Ok(format!("capture:{event_id}:0"))
    }

    /// One `ingest.batch_classified` event holding `captures` as the classifier wrote them (see
    /// [`decision_capture`] and [`evidence_capture`]). The capture at index `i` becomes the node
    /// `capture:<event>:<i>`.
    pub(crate) fn classified_captures(
        &self,
        recorder: &str,
        timestamp: &str,
        captures: Vec<Value>,
    ) -> Result<EventId> {
        self.push(
            recorder,
            EventType::IngestBatchClassified,
            json!({
                "batch_id": "batch:1",
                "classifier_model": "claude-haiku-4-5-20251001",
                "schema_version": "2",
                "captures": captures,
            }),
            None,
            timestamp,
        )
    }

    /// `decision.scored`, schema version 2: a model's assessment of `decision_id`. The payload is
    /// written raw, as a replayed ledger would hold it, so the write path's quote check is not
    /// applied here (its own tests cover it); `dimensions` is the seven answers, see
    /// [`assessed_dimension`] and [`not_assessed_dimension`].
    pub(crate) fn assessment(
        &self,
        decision_id: &str,
        model: &str,
        prompt_version: &str,
        dimensions: Value,
        timestamp: &str,
    ) -> Result<EventId> {
        self.push(
            "agent:hivemind:scorer",
            EventType::DecisionScored,
            json!({
                "schema_version": 2,
                "decision_id": decision_id,
                "model": model,
                "prompt_version": prompt_version,
                "dimensions": dimensions,
            }),
            None,
            timestamp,
        )
    }

    /// `assessment` where the model also judged the three importance factors (stakes,
    /// irreversibility, actionability), each with an explanation, as `decision.scored` carries
    /// them.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn assessment_with_importance(
        &self,
        decision_id: &str,
        model: &str,
        prompt_version: &str,
        dimensions: Value,
        importance: (f64, f64, f64),
        timestamp: &str,
    ) -> Result<EventId> {
        let (stakes, irreversibility, actionability) = importance;
        self.push(
            "agent:hivemind:scorer",
            EventType::DecisionScored,
            json!({
                "schema_version": 2,
                "decision_id": decision_id,
                "model": model,
                "prompt_version": prompt_version,
                "dimensions": dimensions,
                "importance": {
                    "stakes": stakes,
                    "stakes_explanation": "Reaches every later store decision",
                    "irreversibility": irreversibility,
                    "irreversibility_explanation": "Migrating off it means a full export",
                    "actionability": actionability,
                    "actionability_explanation": "The team can act on it now",
                },
            }),
            None,
            timestamp,
        )
    }

    /// `decision.requested` naming `decision_id`, which nobody has proposed: the graph gets a
    /// bare stub node for it, with none of a proposal's properties.
    pub(crate) fn request_naming(&self, decision_id: &str, timestamp: &str) -> Result<EventId> {
        self.push_with_source_ref(
            "agent:tester",
            EventType::DecisionRequested,
            json!({
                "topic_keys": ["quality"],
                "decision_id": decision_id,
                "reason": "Somebody has to decide this",
                "priority": "P2",
                "authority_class": "team",
                "requested_by": "agent:tester",
                "client_request_id": format!("request-{decision_id}"),
            }),
            None,
            timestamp,
            Some("test:scenario"),
        )
    }

    pub(crate) fn evidence(
        &self,
        evidence_id: &str,
        content: &str,
        source: Option<&str>,
        timestamp: &str,
    ) -> Result<EventId> {
        let mut payload = json!({"evidence_id": evidence_id, "content": content});
        if let Some(source) = source {
            payload["source"] = json!(source);
        }
        self.push(
            "agent:tester",
            EventType::EvidenceRecorded,
            payload,
            None,
            timestamp,
        )
    }

    /// `hypothesis.recorded`; `kind` is `assumption` or `bet`.
    pub(crate) fn hypothesis(
        &self,
        hypothesis_id: &str,
        statement: &str,
        kind: &str,
        check_by: Option<&str>,
        timestamp: &str,
    ) -> Result<EventId> {
        let mut payload = json!({
            "hypothesis_id": hypothesis_id,
            "statement": statement,
            "kind": kind,
        });
        if let Some(check_by) = check_by {
            payload["check_by"] = json!(check_by);
        }
        self.push(
            "agent:tester",
            EventType::HypothesisRecorded,
            payload,
            None,
            timestamp,
        )
    }

    pub(crate) fn accept(&self, decision_id: &str, actor_id: &str, timestamp: &str) -> Result<()> {
        self.push(
            actor_id,
            EventType::DecisionAccepted,
            json!({"decision_id": decision_id}),
            None,
            timestamp,
        )
        .map(|_| ())
    }

    /// `decision.moved`: the decision leaves project `from` for project `to`.
    pub(crate) fn moved(
        &self,
        decision_id: &str,
        from: &str,
        to: &str,
        actor_id: &str,
        timestamp: &str,
    ) -> Result<()> {
        self.push(
            actor_id,
            EventType::DecisionMoved,
            json!({"decision_id": decision_id, "from": from, "to": to}),
            None,
            timestamp,
        )
        .map(|_| ())
    }

    /// `decision.retitled`: the decision's title changes from `from` to `to` (mirrors `moved`
    /// exactly; hivemind-ydmp).
    pub(crate) fn retitled(
        &self,
        decision_id: &str,
        from: &str,
        to: &str,
        actor_id: &str,
        timestamp: &str,
    ) -> Result<()> {
        self.push(
            actor_id,
            EventType::DecisionRetitled,
            json!({"decision_id": decision_id, "from": from, "to": to}),
            None,
            timestamp,
        )
        .map(|_| ())
    }

    /// `decision.reaffirmed`: `decision_id` "still holds" after `premise_id` changed. Returns the
    /// event id, so a test can order the reaffirmation against a later change.
    pub(crate) fn reaffirmed(
        &self,
        decision_id: &str,
        premise_id: &str,
        reason: &str,
        actor_id: &str,
        timestamp: &str,
    ) -> Result<EventId> {
        self.push(
            actor_id,
            EventType::DecisionReaffirmed,
            json!({"decision_id": decision_id, "premise_id": premise_id, "reason": reason}),
            None,
            timestamp,
        )
    }

    /// `suggestion.surfaced`: `finding_id` (about `decision_id`) shown to `recipient`. Returns the
    /// notification id (the event's uuid) an acknowledgement names.
    pub(crate) fn surfaced(
        &self,
        finding_id: &str,
        decision_id: &str,
        recipient: &str,
        timestamp: &str,
    ) -> Result<String> {
        let notification_id = Uuid::from_u128(self.sequence.get() + 1).to_string();
        self.push_with_source_ref(
            recipient,
            EventType::SuggestionSurfaced,
            json!({
                "finding_id": finding_id,
                "decision_id": decision_id,
                "recipient_actor_id": recipient,
                "channel": "test",
                "sent_at": timestamp,
            }),
            None,
            timestamp,
            Some("test:scenario"),
        )?;
        Ok(notification_id)
    }

    /// `notification.acknowledged` of `notification_id`, with the `action` and `snooze_until` a
    /// test names (none for neither).
    pub(crate) fn acknowledged(
        &self,
        notification_id: &str,
        actor_id: &str,
        action: Option<&str>,
        snooze_until: Option<&str>,
        timestamp: &str,
    ) -> Result<()> {
        let mut payload = json!({
            "notification_id": notification_id,
            "ack_at": timestamp,
            "snooze_until": snooze_until,
        });
        if let Some(action) = action {
            payload["action"] = json!(action);
        }
        self.push_with_source_ref(
            actor_id,
            EventType::NotificationAcknowledged,
            payload,
            None,
            timestamp,
            Some("test:scenario"),
        )
        .map(|_| ())
    }

    pub(crate) fn reject(&self, decision_id: &str, actor_id: &str, timestamp: &str) -> Result<()> {
        self.push(
            actor_id,
            EventType::DecisionRejected,
            json!({"decision_id": decision_id}),
            None,
            timestamp,
        )
        .map(|_| ())
    }

    pub(crate) fn supersede(
        &self,
        old_decision_id: &str,
        new_decision_id: &str,
        actor_id: &str,
        timestamp: &str,
    ) -> Result<()> {
        self.push(
            actor_id,
            EventType::DecisionSuperseded,
            json!({"old_decision_id": old_decision_id, "new_decision_id": new_decision_id}),
            None,
            timestamp,
        )
        .map(|_| ())
    }

    /// `relation.added`. `causation` is the proposal event for a link named at capture, `None`
    /// for one attributed later.
    pub(crate) fn relation(
        &self,
        relation: &str,
        from_id: &str,
        to_id: &str,
        actor_id: &str,
        causation: Option<EventId>,
        timestamp: &str,
    ) -> Result<()> {
        self.push(
            actor_id,
            EventType::RelationAdded,
            json!({"relation": relation, "from_id": from_id, "to_id": to_id}),
            causation,
            timestamp,
        )
        .map(|_| ())
    }
}

/// A ledger that refuses to be read whole, as a cell's is too big to be: only a read of named
/// event types gets through.
pub(crate) struct TypedReadsOnly<'a>(pub(crate) &'a InMemoryEventLedger);

impl EventLedger for TypedReadsOnly<'_> {
    fn bound_tenant(&self) -> TenantId {
        self.0.bound_tenant()
    }

    fn append_for_tenant(&self, tenant_id: &TenantId, event: Event) -> Result<EventId> {
        self.0.append_for_tenant(tenant_id, event)
    }

    fn read_for_tenant(
        &self,
        _tenant_id: &TenantId,
        _offset: EventId,
        _limit: usize,
    ) -> Result<Vec<Event>> {
        Err(super::shared::query_error("read the whole ledger").into())
    }

    fn replay_from_for_tenant(
        &self,
        tenant_id: &TenantId,
        offset: EventId,
        callback: &mut dyn FnMut(&Event) -> Result<()>,
    ) -> Result<()> {
        self.0.replay_from_for_tenant(tenant_id, offset, callback)
    }

    fn latest_offset_for_tenant(&self, tenant_id: &TenantId) -> Result<EventId> {
        self.0.latest_offset_for_tenant(tenant_id)
    }

    fn read_types_for_tenant(
        &self,
        tenant_id: &TenantId,
        types: &[EventType],
        omit_payload_keys: &[&str],
        offset: EventId,
        limit: usize,
    ) -> Result<Vec<Event>> {
        self.0
            .read_types_for_tenant(tenant_id, types, omit_payload_keys, offset, limit)
    }
}

/// A graph that counts the reads made through it, for tests of what a read costs.
pub(crate) struct CountingGraph<'a, G: GraphView> {
    inner: &'a G,
    queries: Cell<usize>,
}

impl<'a, G: GraphView> CountingGraph<'a, G> {
    pub(crate) fn new(inner: &'a G) -> Self {
        Self {
            inner,
            queries: Cell::new(0),
        }
    }

    /// How many `query` calls have gone through so far.
    pub(crate) fn queries(&self) -> usize {
        self.queries.get()
    }
}

impl<G: GraphView> GraphView for CountingGraph<'_, G> {
    fn upsert_node(&self, kind: NodeKind, id: &str, properties: &GraphProperties) -> Result<()> {
        self.inner.upsert_node(kind, id, properties)
    }

    fn upsert_edge(
        &self,
        kind: RelationKind,
        from_id: &str,
        to_id: &str,
        properties: &GraphProperties,
    ) -> Result<()> {
        self.inner.upsert_edge(kind, from_id, to_id, properties)
    }

    fn query(&self, cypher: &str, params: &GraphParams) -> Result<Vec<GraphRow>> {
        self.queries.set(self.queries.get() + 1);
        self.inner.query(cypher, params)
    }

    fn wipe(&self) -> Result<()> {
        self.inner.wipe()
    }
}

/// A `decision.proposed` payload the way a capture writes one. `options` are `(id, label,
/// description)` and `chosen` names one of them; a `question` comes with the `quote` it answers.
pub(crate) fn record_payload(
    decision_id: &str,
    question: Option<&str>,
    options: &[(&str, &str, &str)],
    chosen: Option<&str>,
    evidence_ids: &[&str],
) -> Value {
    let mut payload = json!({
        "decision_id": decision_id,
        "title": format!("Decision {decision_id}"),
        "rationale": format!("Why {decision_id} was taken, in the decider's words"),
        "topic_keys": ["quality"],
        "option_ids": options.iter().map(|(id, _, _)| *id).collect::<Vec<_>>(),
        "option_labels": options.iter().map(|(_, label, _)| *label).collect::<Vec<_>>(),
        "option_descriptions": options.iter().map(|(_, _, description)| *description).collect::<Vec<_>>(),
        "chosen_option_id": chosen,
        "evidence_ids": evidence_ids,
    });
    if let Some(question) = question {
        payload["question"] = json!(question);
        payload["quote"] = json!("Agreed in the review");
    }
    payload
}

/// One assessed dimension of a model's answer, as it is written in a `decision.scored` payload.
pub(crate) fn assessed_dimension(level: &str, explanation: &str, quote: &str) -> Value {
    json!({"status": "assessed", "level": level, "explanation": explanation, "quote": quote})
}

/// A `none` answer that leaves its quote out (an absence cannot be quoted), as it is written in
/// a `decision.scored` payload.
pub(crate) fn none_dimension_without_quote(explanation: &str) -> Value {
    json!({"status": "assessed", "level": "none", "explanation": explanation})
}

/// One dimension a model did not assess, as it is written in a `decision.scored` payload.
pub(crate) fn not_assessed_dimension(reason: &str) -> Value {
    json!({"status": "not_assessed", "reason": reason})
}

/// A model's answer for all seven dimensions of `d:solid`: four assessed with a passage of its
/// own record quoted, one assessed `none` with no quote, two not assessed. `tag` is worked into
/// the explanations so two answers for the same decision can be told apart.
pub(crate) fn solid_assessment_dimensions(tag: &str) -> Value {
    json!({
        "framing": assessed_dimension(
            "solid",
            &format!("{tag}: the question is stated"),
            "Which store should hold the ledger?",
        ),
        "alternatives": assessed_dimension(
            "partial",
            &format!("{tag}: two alternatives are described, neither weighed"),
            "A file per tenant; rejected, no cross-tenant queries",
        ),
        "information": not_assessed_dimension(&format!("{tag}: nothing in the text to rest it on")),
        "reasoning": assessed_dimension(
            "partial",
            &format!("{tag}: a rationale is given"),
            "Why d:solid was taken",
        ),
        "values_tradeoffs": assessed_dimension(
            "partial",
            &format!("{tag}: a cost is named for one alternative"),
            "rejected, its C++ build is too slow",
        ),
        "bias_exposure": none_dimension_without_quote(
            &format!("{tag}: nothing in the record bears on a distortion"),
        ),
        "calibration": not_assessed_dimension(&format!("{tag}: no confidence was declared")),
    })
}

/// Every decision `floor_scenario` records that a profile is read for, plus one id that is not in
/// the graph.
pub(crate) const FLOOR_SCENARIO_DECISIONS: [&str; 18] = [
    "d:solid",
    "d:placeholders",
    "d:bare",
    "d:later-evidence",
    "d:backfilled",
    "d:mixed",
    "d:single",
    "d:undecided",
    "d:stub",
    "d:premised",
    "d:bet-high",
    "d:high-nothing",
    "d:backfilled-premise",
    "d:late-premise",
    "d:on-superseded",
    "d:challenged",
    "d:refuted-later",
    "d:missing",
];

/// Records that land on every rung of every floor, so a backend that agrees on all of them is not
/// agreeing on emptiness. `e:sourced` and `e:unsourced` pre-date every decision; `e:after` and
/// `e:after-too` are recorded after the decisions they are linked to.
///
/// - `d:solid`: a question, three described options, sourced evidence at capture.
/// - `d:placeholders`: no question; rejected options carrying only text a capture surface
///   generated; unsourced evidence at capture.
/// - `d:bare`: nothing but a title and a rationale.
/// - `d:later-evidence`: no evidence at capture; `e:after` linked afterwards.
/// - `d:backfilled`: no evidence at capture; `e:sourced`, recorded before it, linked afterwards.
/// - `d:mixed`: unsourced evidence at capture; sourced `e:after-too` linked afterwards.
/// - `d:single`: one option, the chosen one. `d:undecided`: two described options, none chosen.
/// - `d:stub`: named by a decision request and never proposed, so its node has no rationale.
/// - `d:premised`: two options and a medium confidence; follows from `d:solid` at capture.
/// - `d:bet-high`: high confidence over a declared bet (`h:bet`) alone.
/// - `d:high-nothing`: high confidence, nothing declared.
/// - `d:backfilled-premise`: follows from `d:solid`, attributed afterwards; `d:solid` pre-dates it.
/// - `d:late-premise`: follows from `d:after-premise`, which was recorded after it.
/// - `d:on-superseded`: follows from `d:old-premise` at capture; `d:new-premise` supersedes that
///   afterwards.
/// - `d:challenged`: rests on `h:doubtful`, which `e:counter` refuted before the decision.
///   `d:refuted-later` rests on `h:later-doubt`, refuted only after it.
/// - `d:solid` is also assessed twice by a model (`model-a`, then `model-b`); nothing else is.
pub(crate) fn floor_scenario() -> Result<Scenario> {
    let s = Scenario::new();
    let crew = "agent:claude:crew";
    s.evidence(
        "e:sourced",
        "27 of 27 captures carry no premise",
        Some("mayor audit 2026-09-22"),
        "2026-01-01T00:00:01Z",
    )?;
    s.evidence(
        "e:unsourced",
        "Two users asked for it",
        None,
        "2026-01-01T00:00:02Z",
    )?;

    s.proposal(
        crew,
        "2026-01-02T00:00:00Z",
        record_payload(
            "d:solid",
            Some("Which store should hold the ledger?"),
            &[
                ("d:solid:pg", "Postgres", "One server for every tenant"),
                (
                    "d:solid:sqlite",
                    "SQLite",
                    "A file per tenant; rejected, no cross-tenant queries",
                ),
                (
                    "d:solid:kuzu",
                    "Kuzu",
                    "Embedded graph; rejected, its C++ build is too slow",
                ),
            ],
            Some("d:solid:pg"),
            &["e:sourced"],
        ),
    )?;
    s.proposal(
        crew,
        "2026-01-02T00:00:01Z",
        record_payload(
            "d:placeholders",
            None,
            &[
                (
                    "d:placeholders:a",
                    "Gateway",
                    "Route every call through one gateway",
                ),
                (
                    "d:placeholders:b",
                    "Direct",
                    "Option generated from MCP value 'Direct'",
                ),
                (
                    "d:placeholders:c",
                    "Queue",
                    "Option generated from CLI value 'Queue'",
                ),
            ],
            Some("d:placeholders:a"),
            &["e:unsourced"],
        ),
    )?;
    s.decision("d:bare", "Keep the default", crew, "2026-01-02T00:00:02Z")?;

    s.proposal(
        crew,
        "2026-01-02T00:00:03Z",
        record_payload("d:later-evidence", None, &[], None, &[]),
    )?;
    s.evidence(
        "e:after",
        "A benchmark run after the call was made",
        Some("bench run 2026-02-01"),
        "2026-02-01T00:00:00Z",
    )?;
    s.relation(
        "BASED_ON",
        "d:later-evidence",
        "e:after",
        "human:alex",
        None,
        "2026-02-01T00:00:01Z",
    )?;

    s.proposal(
        crew,
        "2026-01-02T00:00:04Z",
        record_payload("d:backfilled", None, &[], None, &[]),
    )?;
    s.relation(
        "BASED_ON",
        "d:backfilled",
        "e:sourced",
        "human:alex",
        None,
        "2026-02-01T00:00:02Z",
    )?;

    s.proposal(
        crew,
        "2026-01-02T00:00:05Z",
        record_payload("d:mixed", None, &[], None, &["e:unsourced"]),
    )?;
    s.evidence(
        "e:after-too",
        "A second benchmark run",
        Some("bench run 2026-02-02"),
        "2026-02-02T00:00:00Z",
    )?;
    s.relation(
        "BASED_ON",
        "d:mixed",
        "e:after-too",
        "human:alex",
        None,
        "2026-02-02T00:00:01Z",
    )?;

    s.proposal(
        crew,
        "2026-01-02T00:00:06Z",
        record_payload(
            "d:single",
            None,
            &[("d:single:a", "Only way", "The one option on the table")],
            Some("d:single:a"),
            &[],
        ),
    )?;
    s.proposal(
        crew,
        "2026-01-02T00:00:07Z",
        record_payload(
            "d:undecided",
            None,
            &[
                ("d:undecided:a", "Shard by tenant", "One shard per tenant"),
                ("d:undecided:b", "Shard by hash", "Hash the decision id"),
            ],
            None,
            &[],
        ),
    )?;
    s.request_naming("d:stub", "2026-01-02T00:00:08Z")?;
    add_grounded_decisions(&s, crew)?;
    // Two model assessments of `d:solid`, the second after the first: the profile shows the
    // newest, beside floors that do not move. No other decision has one.
    s.assessment(
        "d:solid",
        "model-a",
        "assessment-v1",
        solid_assessment_dimensions("first"),
        "2026-02-03T00:00:00Z",
    )?;
    s.assessment(
        "d:solid",
        "model-b",
        "assessment-v2",
        solid_assessment_dimensions("second"),
        "2026-02-04T00:00:00Z",
    )?;
    Ok(s)
}

/// The decisions of `floor_scenario` that rest on a prior decision, an assumption or a bet, or
/// declare a confidence.
fn add_grounded_decisions(s: &Scenario, crew: &str) -> Result<()> {
    s.hypothesis(
        "h:bet",
        "Load stays flat",
        "bet",
        Some("2026-06-01T00:00:00Z"),
        "2026-01-03T00:00:00Z",
    )?;
    s.hypothesis(
        "h:doubtful",
        "Users tolerate a slow first load",
        "assumption",
        None,
        "2026-01-03T00:00:01Z",
    )?;
    s.evidence(
        "e:counter",
        "First load took nine seconds in the field test",
        Some("field test 2026-01-02"),
        "2026-01-03T00:00:02Z",
    )?;
    s.relation(
        "REFUTES",
        "e:counter",
        "h:doubtful",
        "agent:tester",
        None,
        "2026-01-03T00:00:03Z",
    )?;

    let mut premised = record_payload(
        "d:premised",
        None,
        &[
            (
                "d:premised:a",
                "Reuse the store",
                "Keep the ledger store we already chose",
            ),
            (
                "d:premised:b",
                "Switch again",
                "Move to a new store; rejected, the churn is not worth it",
            ),
        ],
        Some("d:premised:a"),
        &[],
    );
    premised["expressed_confidence"] = json!("medium");
    let proposal = s.proposal(crew, "2026-03-01T00:00:00Z", premised)?;
    s.relation(
        "FOLLOWS_FROM",
        "d:premised",
        "d:solid",
        crew,
        Some(proposal),
        "2026-03-01T00:00:01Z",
    )?;

    s.decision_with(
        "d:bet-high",
        "Ship on the bet",
        crew,
        "2026-03-02T00:00:00Z",
        false,
        &[],
        &["h:bet"],
        Some("high"),
    )?;
    s.decision_with(
        "d:high-nothing",
        "Trust the vendor",
        crew,
        "2026-03-02T00:00:01Z",
        false,
        &[],
        &[],
        Some("high"),
    )?;

    s.decision(
        "d:backfilled-premise",
        "Keep the store",
        crew,
        "2026-03-02T00:00:02Z",
    )?;
    s.relation(
        "FOLLOWS_FROM",
        "d:backfilled-premise",
        "d:solid",
        "human:alex",
        None,
        "2026-04-01T00:00:00Z",
    )?;

    s.decision(
        "d:late-premise",
        "Keep the queue",
        crew,
        "2026-03-02T00:00:03Z",
    )?;
    s.decision(
        "d:after-premise",
        "Queue sizing rule",
        crew,
        "2026-03-03T00:00:00Z",
    )?;
    s.relation(
        "FOLLOWS_FROM",
        "d:late-premise",
        "d:after-premise",
        "human:alex",
        None,
        "2026-04-01T00:00:01Z",
    )?;

    s.decision(
        "d:old-premise",
        "Bill per call",
        crew,
        "2026-03-02T00:00:04Z",
    )?;
    let proposal = s.decision(
        "d:on-superseded",
        "Cache billing reads",
        crew,
        "2026-03-02T00:00:05Z",
    )?;
    s.relation(
        "FOLLOWS_FROM",
        "d:on-superseded",
        "d:old-premise",
        crew,
        Some(proposal),
        "2026-03-02T00:00:06Z",
    )?;
    s.decision(
        "d:new-premise",
        "Bill per seat",
        crew,
        "2026-03-05T00:00:00Z",
    )?;
    s.supersede(
        "d:old-premise",
        "d:new-premise",
        crew,
        "2026-03-05T00:00:01Z",
    )?;

    s.decision_with(
        "d:challenged",
        "Ship the slow first load",
        crew,
        "2026-03-06T00:00:00Z",
        false,
        &[],
        &["h:doubtful"],
        None,
    )?;
    s.hypothesis(
        "h:later-doubt",
        "Users will not notice",
        "assumption",
        None,
        "2026-03-06T00:00:01Z",
    )?;
    s.decision_with(
        "d:refuted-later",
        "Ship the unnoticed change",
        crew,
        "2026-03-06T00:00:02Z",
        false,
        &[],
        &["h:later-doubt"],
        None,
    )?;
    s.evidence(
        "e:counter-later",
        "Users noticed within a day",
        Some("support inbox 2026-03-08"),
        "2026-03-08T00:00:00Z",
    )?;
    s.relation(
        "REFUTES",
        "e:counter-later",
        "h:later-doubt",
        "agent:tester",
        None,
        "2026-03-08T00:00:01Z",
    )
}

/// The clock `attention_scenario` is read at; its evidence window (90 days) reaches back to
/// 2026-06-28T00:00:00Z.
pub(crate) const ATTENTION_NOW: &str = "2026-09-26T00:00:00Z";

/// Decisions that need a look, and their look-alikes that do not, for the attention findings.
/// Read at `ATTENTION_NOW`. Every decision is proposed by `agent:claude:crew`.
///
/// Bets (`h:bet-*`, check dates in `h:*` names): `d:bet-overdue` (through its chosen option) and
/// `d:bet-overdue-direct` (directly) share an overdue bet; `d:bet-future`, `d:bet-undated` and
/// `d:bet-held` (supported) are not overdue; `d:bet-failed` rests on a refuted bet;
/// `d:bet-dead` (superseded) and `d:bet-refused` (rejected) rest on the overdue bet too but no
/// longer stand. `d:assumption-refuted` rests on an assumption refuted twice (the earlier
/// refutation has the later id); `d:assumption-open` on one nothing refutes.
///
/// Premises: `d:p-old` was superseded by `d:p-new`; `d:p-double` by `d:p-double-a` (2026-08-10)
/// and `d:p-double-b` (2026-08-05, the earlier); `d:p-rejected` was rejected; `d:p-contested` was
/// accepted and rejected; `d:p-standing` stands. `d:f-superseded`, `d:f-concurrent`,
/// `d:f-rejected` and `d:f-both` (superseded and rejected premise) follow from them and are
/// flagged; `d:f-contested` and `d:f-standing` are not; `d:f-dead` (superseded) and
/// `d:f-refused` (rejected) follow from `d:p-old` but no longer stand. `d:cycle-a` and
/// `d:cycle-b` follow from each other and `d:cycle-a` also from `d:p-old`; `d:cycle-self`
/// follows from itself.
///
/// Evidence (recorded dates in `e:*`): `d:ev-stale` cites only `e:old`; `d:ev-stale-two` cites
/// `e:very-old` and `e:old`; `d:ev-fresh` a recent item; `d:ev-mixed` an old and a recent one;
/// `d:ev-rechecked-later` an old one at capture and a recent one linked afterwards;
/// `d:ev-just-over` an item one second past the window and `d:ev-boundary` one exactly at it;
/// `d:ev-rejected` cites `e:old` but was rejected; `d:ev-none` cites nothing.
pub(crate) fn attention_scenario() -> Result<Scenario> {
    let s = Scenario::new();
    let crew = "agent:claude:crew";
    let alex = "human:alex";

    // Bets and assumptions.
    s.hypothesis(
        "h:bet-overdue",
        "Load stays flat",
        "bet",
        Some("2026-09-01T00:00:00Z"),
        "2026-01-01T00:00:00Z",
    )?;
    s.hypothesis(
        "h:bet-future",
        "Vendor ships in Q4",
        "bet",
        Some("2026-12-01T00:00:00Z"),
        "2026-01-01T00:00:01Z",
    )?;
    s.hypothesis(
        "h:bet-undated",
        "Nobody else builds it",
        "bet",
        None,
        "2026-01-01T00:00:02Z",
    )?;
    s.hypothesis(
        "h:bet-held",
        "The cache holds",
        "bet",
        Some("2026-08-01T00:00:00Z"),
        "2026-01-01T00:00:03Z",
    )?;
    s.hypothesis(
        "h:bet-failed",
        "Latency stays low",
        "bet",
        Some("2026-08-01T00:00:00Z"),
        "2026-01-01T00:00:04Z",
    )?;
    s.hypothesis(
        "h:assumption-refuted",
        "Users log in daily",
        "assumption",
        None,
        "2026-01-01T00:00:05Z",
    )?;
    s.hypothesis(
        "h:assumption-open",
        "The API is stable",
        "assumption",
        None,
        "2026-01-01T00:00:06Z",
    )?;

    // Evidence: what checks the bets, and what the decisions cite.
    s.evidence(
        "e:support",
        "The cache held under load",
        Some("load test"),
        "2026-08-20T00:00:00Z",
    )?;
    s.evidence(
        "e:refute-bet",
        "p95 doubled",
        Some("dashboard"),
        "2026-08-15T00:00:00Z",
    )?;
    s.evidence(
        "e:refute-a2",
        "Half of users log in weekly",
        Some("analytics"),
        "2026-06-01T00:00:00Z",
    )?;
    s.evidence(
        "e:refute-a1",
        "Most users log in monthly",
        Some("analytics"),
        "2026-07-01T00:00:00Z",
    )?;
    s.evidence(
        "e:very-old",
        "Benchmark from January",
        Some("bench run"),
        "2026-01-01T00:00:00Z",
    )?;
    s.evidence(
        "e:old",
        "Benchmark from March",
        Some("bench run"),
        "2026-03-01T00:00:00Z",
    )?;
    s.evidence(
        "e:just-over",
        "Just outside the window",
        None,
        "2026-06-27T23:59:59Z",
    )?;
    s.evidence(
        "e:boundary",
        "Exactly at the window",
        None,
        "2026-06-28T00:00:00Z",
    )?;
    s.evidence(
        "e:fresh",
        "Measured last week",
        Some("dashboard"),
        "2026-09-10T00:00:00Z",
    )?;
    s.evidence(
        "e:newer",
        "Measured this month",
        Some("dashboard"),
        "2026-09-15T00:00:00Z",
    )?;
    s.relation(
        "SUPPORTS",
        "e:support",
        "h:bet-held",
        alex,
        None,
        "2026-08-20T00:00:01Z",
    )?;
    s.relation(
        "REFUTES",
        "e:refute-bet",
        "h:bet-failed",
        alex,
        None,
        "2026-08-15T00:00:01Z",
    )?;
    s.relation(
        "REFUTES",
        "e:refute-a2",
        "h:assumption-refuted",
        alex,
        None,
        "2026-06-01T00:00:01Z",
    )?;
    s.relation(
        "REFUTES",
        "e:refute-a1",
        "h:assumption-refuted",
        alex,
        None,
        "2026-07-01T00:00:01Z",
    )?;

    // Decisions resting on bets and assumptions. A chosen option carries them at capture; without
    // one they hang off the decision.
    s.decision_with(
        "d:bet-overdue",
        "Size the fleet for flat load",
        crew,
        "2026-02-01T00:00:00Z",
        true,
        &[],
        &["h:bet-overdue"],
        None,
    )?;
    s.decision_with(
        "d:bet-overdue-direct",
        "Skip autoscaling",
        crew,
        "2026-02-01T00:00:01Z",
        false,
        &[],
        &["h:bet-overdue"],
        None,
    )?;
    s.decision_with(
        "d:bet-future",
        "Wait for the vendor",
        crew,
        "2026-02-01T00:00:02Z",
        true,
        &[],
        &["h:bet-future"],
        None,
    )?;
    s.decision_with(
        "d:bet-undated",
        "Build it ourselves",
        crew,
        "2026-02-01T00:00:03Z",
        true,
        &[],
        &["h:bet-undated"],
        None,
    )?;
    s.decision_with(
        "d:bet-held",
        "Cache the catalogue",
        crew,
        "2026-02-01T00:00:04Z",
        true,
        &[],
        &["h:bet-held"],
        None,
    )?;
    s.decision_with(
        "d:bet-failed",
        "Serve from one region",
        crew,
        "2026-02-01T00:00:05Z",
        true,
        &[],
        &["h:bet-failed"],
        None,
    )?;
    s.decision_with(
        "d:assumption-refuted",
        "Poll once a day",
        crew,
        "2026-02-01T00:00:06Z",
        true,
        &[],
        &["h:assumption-refuted"],
        None,
    )?;
    s.decision_with(
        "d:assumption-open",
        "Pin the API version",
        crew,
        "2026-02-01T00:00:07Z",
        true,
        &[],
        &["h:assumption-open"],
        None,
    )?;
    s.decision_with(
        "d:bet-dead",
        "Old fleet sizing",
        crew,
        "2026-02-01T00:00:08Z",
        true,
        &[],
        &["h:bet-overdue"],
        None,
    )?;
    s.decision(
        "d:bet-replacement",
        "New fleet sizing",
        crew,
        "2026-03-01T00:00:00Z",
    )?;
    s.supersede(
        "d:bet-dead",
        "d:bet-replacement",
        crew,
        "2026-03-01T00:00:01Z",
    )?;
    s.decision_with(
        "d:bet-refused",
        "Fleet sizing nobody wanted",
        crew,
        "2026-02-01T00:00:09Z",
        true,
        &[],
        &["h:bet-overdue"],
        None,
    )?;
    s.reject("d:bet-refused", alex, "2026-03-02T00:00:00Z")?;

    // Premises and the decisions that follow from them.
    s.decision("d:p-old", "Use the old queue", crew, "2026-02-01T00:00:10Z")?;
    s.decision("d:p-new", "Use the new queue", crew, "2026-08-01T00:00:00Z")?;
    s.supersede("d:p-old", "d:p-new", crew, "2026-08-01T00:00:01Z")?;
    s.decision("d:p-double", "Use one region", crew, "2026-02-01T00:00:11Z")?;
    s.decision(
        "d:p-double-a",
        "Use two regions",
        crew,
        "2026-08-10T00:00:00Z",
    )?;
    s.decision(
        "d:p-double-b",
        "Use three regions",
        crew,
        "2026-08-05T00:00:00Z",
    )?;
    s.supersede("d:p-double", "d:p-double-a", crew, "2026-08-10T00:00:01Z")?;
    s.supersede("d:p-double", "d:p-double-b", crew, "2026-08-10T00:00:02Z")?;
    s.decision(
        "d:p-rejected",
        "Adopt the vendor SDK",
        crew,
        "2026-02-01T00:00:12Z",
    )?;
    s.reject("d:p-rejected", alex, "2026-03-03T00:00:00Z")?;
    s.decision(
        "d:p-contested",
        "Adopt the open SDK",
        crew,
        "2026-02-01T00:00:13Z",
    )?;
    s.accept("d:p-contested", "human:alice", "2026-03-04T00:00:00Z")?;
    s.reject("d:p-contested", "human:bob", "2026-03-04T00:00:01Z")?;
    s.decision(
        "d:p-standing",
        "Adopt the platform SDK",
        crew,
        "2026-02-01T00:00:14Z",
    )?;
    s.accept("d:p-standing", alex, "2026-03-05T00:00:00Z")?;

    let follow =
        |decision_id: &str, title: &str, timestamp: &str, premises: &[&str]| -> Result<()> {
            let proposal = s.decision(decision_id, title, crew, timestamp)?;
            for premise in premises {
                s.relation(
                    "FOLLOWS_FROM",
                    decision_id,
                    premise,
                    crew,
                    Some(proposal),
                    timestamp,
                )?;
            }
            Ok(())
        };
    follow(
        "d:f-superseded",
        "Batch jobs on the old queue",
        "2026-04-01T00:00:00Z",
        &["d:p-old"],
    )?;
    follow(
        "d:f-concurrent",
        "Pin the region count",
        "2026-04-01T00:00:01Z",
        &["d:p-double"],
    )?;
    follow(
        "d:f-rejected",
        "Wrap the vendor SDK",
        "2026-04-01T00:00:02Z",
        &["d:p-rejected"],
    )?;
    follow(
        "d:f-both",
        "Vendor SDK on the old queue",
        "2026-04-01T00:00:03Z",
        &["d:p-old", "d:p-rejected"],
    )?;
    follow(
        "d:f-contested",
        "Document the open SDK",
        "2026-04-01T00:00:04Z",
        &["d:p-contested"],
    )?;
    follow(
        "d:f-standing",
        "Document the platform SDK",
        "2026-04-01T00:00:05Z",
        &["d:p-standing"],
    )?;
    follow(
        "d:f-dead",
        "Old batch jobs",
        "2026-04-01T00:00:06Z",
        &["d:p-old"],
    )?;
    s.decision(
        "d:f-replacement",
        "New batch jobs",
        crew,
        "2026-05-01T00:00:00Z",
    )?;
    s.supersede("d:f-dead", "d:f-replacement", crew, "2026-05-01T00:00:01Z")?;
    follow(
        "d:f-refused",
        "Refused batch jobs",
        "2026-04-01T00:00:07Z",
        &["d:p-old"],
    )?;
    s.reject("d:f-refused", alex, "2026-05-02T00:00:00Z")?;
    // A cycle needs both decisions to exist before the links that close it.
    s.decision("d:cycle-a", "Cycle A", crew, "2026-04-01T00:00:08Z")?;
    s.decision("d:cycle-b", "Cycle B", crew, "2026-04-01T00:00:09Z")?;
    s.decision(
        "d:cycle-self",
        "Cycle on itself",
        crew,
        "2026-04-01T00:00:10Z",
    )?;
    for (from, to) in [
        ("d:cycle-a", "d:cycle-b"),
        ("d:cycle-a", "d:p-old"),
        ("d:cycle-b", "d:cycle-a"),
        ("d:cycle-self", "d:cycle-self"),
    ] {
        s.relation("FOLLOWS_FROM", from, to, crew, None, "2026-04-01T00:00:11Z")?;
    }

    // Decisions citing evidence.
    s.decision_with(
        "d:ev-stale",
        "Trust the March benchmark",
        crew,
        "2026-04-02T00:00:00Z",
        false,
        &["e:old"],
        &[],
        None,
    )?;
    s.decision_with(
        "d:ev-stale-two",
        "Trust both benchmarks",
        crew,
        "2026-04-02T00:00:01Z",
        false,
        &["e:very-old", "e:old"],
        &[],
        None,
    )?;
    s.decision_with(
        "d:ev-fresh",
        "Trust the dashboard",
        crew,
        "2026-09-11T00:00:00Z",
        false,
        &["e:fresh"],
        &[],
        None,
    )?;
    s.decision_with(
        "d:ev-mixed",
        "Trust the old and the new",
        crew,
        "2026-09-16T00:00:00Z",
        false,
        &["e:very-old", "e:newer"],
        &[],
        None,
    )?;
    s.decision_with(
        "d:ev-rechecked-later",
        "Trust the January benchmark",
        crew,
        "2026-04-02T00:00:02Z",
        false,
        &["e:very-old"],
        &[],
        None,
    )?;
    s.relation(
        "BASED_ON",
        "d:ev-rechecked-later",
        "e:newer",
        alex,
        None,
        "2026-09-15T00:00:01Z",
    )?;
    s.decision_with(
        "d:ev-just-over",
        "Trust the June note",
        crew,
        "2026-06-29T00:00:00Z",
        false,
        &["e:just-over"],
        &[],
        None,
    )?;
    s.decision_with(
        "d:ev-boundary",
        "Trust the other June note",
        crew,
        "2026-06-29T00:00:01Z",
        false,
        &["e:boundary"],
        &[],
        None,
    )?;
    s.decision_with(
        "d:ev-rejected",
        "Trust a benchmark nobody wanted",
        crew,
        "2026-04-02T00:00:03Z",
        false,
        &["e:old"],
        &[],
        None,
    )?;
    s.reject("d:ev-rejected", alex, "2026-05-03T00:00:00Z")?;
    s.decision(
        "d:ev-none",
        "Decided without evidence",
        crew,
        "2026-04-02T00:00:04Z",
    )?;
    Ok(s)
}

/// The projects and decisions behind the project-first "what should I know" answer
/// (hivemind-s15q.6). Links, `part_of` up and `depends_on` across:
///
/// ```text
/// Billing --part_of--> Platform --part_of--> City
///    |                    `--depends_on--> Infra
///    `--depends_on--> Auth --depends_on--> Crypto        Marketing (linked to nothing)
/// ```
///
/// One decision on the topic `pricing` sits in each project, `auth_old` is superseded by
/// `auth_new` (which inherits Auth), and `personal` was recorded with no project at all. Asked
/// from Billing, the answer is Billing, Platform, then Auth's two; it leaves out City (one level
/// too high), Crypto and Infra (one link too far), Marketing and the personal decision.
pub(crate) struct ProjectFirstFixture {
    pub(crate) ledger: InMemoryEventLedger,
    pub(crate) billing: String,
    pub(crate) platform: String,
    pub(crate) city: String,
    pub(crate) auth_old: String,
    pub(crate) auth_new: String,
    pub(crate) crypto: String,
    pub(crate) infra: String,
    pub(crate) marketing: String,
    pub(crate) personal: String,
}

fn propose_pricing(
    commands: &Commands<'_, InMemoryEventLedger>,
    title: &str,
    project: Option<&str>,
) -> Result<String> {
    let actor = "human:alex";
    // A registered project's topic vocabulary is explicit (hivemind-zywz): declare the one
    // topic this fixture files under, once per project (declaring twice records nothing).
    if let Some(handle) = project {
        commands.declare_project_topic(actor, handle, "pricing")?;
    }
    let option_id = commands.record_option(actor, "Per seat", "Charge each seat")?;
    commands.propose_decision(DecisionProposalInput {
        grounding: Grounding::NotAsked,
        expressed_confidence: None,
        actor_id: actor,
        title,
        rationale: "Scoping a question by project keeps the answer to what applies here",
        topic_keys: &["pricing".to_owned()],
        option_ids: std::slice::from_ref(&option_id),
        option_labels: &["Per seat".to_owned()],
        chosen_option_id: Some(option_id.as_str()),
        decided_by: None,
        still_proposed: false,
        hypothesis_ids: &[],
        evidence_ids: &[],
        quote: None,
        question: None,
        delegated_by: None,
        project: project.map(DeterminedProject::stated),
        floor: FloorInput::NotAsked,
    })
}

pub(crate) fn project_first_fixture() -> Result<ProjectFirstFixture> {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let alex = "human:alex";
    for (handle, display_name) in [
        ("city", Some("City")),
        ("platform", Some("Platform")),
        ("billing", Some("Billing")),
        ("auth", Some("Auth")),
        ("crypto", None),
        ("infra", None),
        ("marketing", Some("Marketing")),
    ] {
        commands.register_project(alex, handle, display_name, None)?;
    }
    commands.link_project(alex, "platform", "city", ProjectLinkKind::PartOf)?;
    commands.link_project(alex, "billing", "platform", ProjectLinkKind::PartOf)?;
    commands.link_project(alex, "billing", "auth", ProjectLinkKind::DependsOn)?;
    commands.link_project(alex, "auth", "crypto", ProjectLinkKind::DependsOn)?;
    commands.link_project(alex, "platform", "infra", ProjectLinkKind::DependsOn)?;

    let billing = propose_pricing(&commands, "Price per seat", Some("billing"))?;
    let platform = propose_pricing(&commands, "Quote every price in euros", Some("platform"))?;
    let city = propose_pricing(&commands, "No free tiers anywhere", Some("city"))?;
    let auth_old = propose_pricing(&commands, "Bill token issuance per call", Some("auth"))?;
    let crypto = propose_pricing(&commands, "Rotate signing keys monthly", Some("crypto"))?;
    let infra = propose_pricing(&commands, "Run on shared hosts", Some("infra"))?;
    let marketing = propose_pricing(&commands, "Launch with a promo price", Some("marketing"))?;
    let personal = propose_pricing(&commands, "Keep a personal scratch price", None)?;
    let auth_new = commands
        .supersede(SupersedeInput {
            actor_id: alex,
            old_decision_id: &auth_old,
            new_title: "Bill token issuance per seat",
            new_rationale: "Per-call billing surprised customers; seats are predictable",
            topic_keys: &["pricing".to_owned()],
            option_labels: &["Per seat".to_owned()],
            chosen_option_label: Some("Per seat"),
            still_proposed: false,
            hypothesis_ids: &[],
            evidence_ids: &[],
            project: None,
            grounding: None,
            expressed_confidence: None,
            floor: FloorInput::NotAsked,
        })?
        .new_decision_id;

    Ok(ProjectFirstFixture {
        ledger,
        billing,
        platform,
        city,
        auth_old,
        auth_new,
        crypto,
        infra,
        marketing,
        personal,
    })
}

/// `d:dependent` follows from `d:premise`, which `d:replacement` supersedes, and a hypothesis
/// `h:premise` it assumes. Three `decision.reaffirmed` events follow: `d:dependent` still holds on
/// `d:premise` twice (alice, then a reviewer agent) and once on `h:premise`.
///
/// The two reaffirmations of one pair are the point: an edge from the dependent to the premise
/// is unique per (kind, from, to) on Postgres and Kuzu, so a second reaffirmation stored as an
/// edge would collapse into the first. Stored as nodes they both survive on every backend.
pub(crate) fn reaffirmation_scenario() -> Result<Scenario> {
    let s = Scenario::new();
    s.hypothesis(
        "h:premise",
        "Traffic stays under the cap",
        "assumption",
        None,
        "2026-09-01T00:00:00Z",
    )?;
    s.decision(
        "d:premise",
        "Cap requests per tenant",
        "human:alex",
        "2026-09-01T00:00:01Z",
    )?;
    s.decision(
        "d:replacement",
        "Cap requests per user",
        "human:alex",
        "2026-09-01T00:00:02Z",
    )?;
    s.decision(
        "d:dependent",
        "Shard by tenant",
        "agent:claude:crew",
        "2026-09-01T00:00:03Z",
    )?;
    s.relation(
        "FOLLOWS_FROM",
        "d:dependent",
        "d:premise",
        "agent:claude:crew",
        None,
        "2026-09-01T00:00:04Z",
    )?;
    s.relation(
        "ASSUMES",
        "d:dependent",
        "h:premise",
        "agent:claude:crew",
        None,
        "2026-09-01T00:00:05Z",
    )?;
    s.supersede(
        "d:premise",
        "d:replacement",
        "human:alex",
        "2026-09-02T00:00:00Z",
    )?;
    s.reaffirmed(
        "d:dependent",
        "d:premise",
        "Sharding never depended on the per-tenant cap",
        "human:alice",
        "2026-09-03T00:00:00Z",
    )?;
    s.reaffirmed(
        "d:dependent",
        "d:premise",
        "Checked again after the second change to the cap",
        "agent:codex:reviewer",
        "2026-09-04T00:00:00Z",
    )?;
    s.reaffirmed(
        "d:dependent",
        "h:premise",
        "The hypothesis is only a sizing note here",
        "human:alice",
        "2026-09-05T00:00:00Z",
    )?;
    Ok(s)
}

/// What [`reaffirmation_scenario`] reads as on `graph`, through the shared bulk node read the
/// re-examine reader uses: one `Reaffirmation` node per event, in ledger order, carrying the
/// dependent, the premise, the reason, the actor and the time, with no edge touching any of them.
/// Every backend passes it, so a `Reaffirmation` is stored the same way on memory, Postgres and
/// Kuzu.
pub(crate) fn assert_reaffirmation_nodes(graph: &impl GraphView) -> Result<()> {
    let text = |row: &GraphRow, key: &str| match row.get(key) {
        Some(GraphValue::String(value)) => Some(value.clone()),
        _ => None,
    };
    let field = |row: &GraphRow, key: &str| text(row, key).unwrap_or_default();
    let mut rows: Vec<GraphRow> = super::shared::node_rows(graph, NodeKind::Reaffirmation)?
        .into_values()
        .collect();
    rows.sort_by_key(|row| match row.get("event_origin") {
        Some(GraphValue::Int(origin)) => *origin,
        _ => i64::MAX,
    });

    let seen: Vec<[String; 4]> = rows
        .iter()
        .map(|row| {
            [
                field(row, "decision_id"),
                field(row, "premise_id"),
                field(row, "reaffirmed_by"),
                field(row, "reason"),
            ]
        })
        .collect();
    let expected: Vec<[String; 4]> = [
        (
            "d:premise",
            "human:alice",
            "Sharding never depended on the per-tenant cap",
        ),
        (
            "d:premise",
            "agent:codex:reviewer",
            "Checked again after the second change to the cap",
        ),
        (
            "h:premise",
            "human:alice",
            "The hypothesis is only a sizing note here",
        ),
    ]
    .into_iter()
    .map(|(premise, actor, reason)| {
        [
            "d:dependent".to_owned(),
            premise.to_owned(),
            actor.to_owned(),
            reason.to_owned(),
        ]
    })
    .collect();
    if seen != expected {
        return Err(super::shared::query_error(format!(
            "reaffirmation nodes read {seen:?}, expected {expected:?}"
        ))
        .into());
    }

    let origins: Vec<i64> = rows
        .iter()
        .filter_map(|row| match row.get("event_origin") {
            Some(GraphValue::Int(origin)) => Some(*origin),
            _ => None,
        })
        .collect();
    if origins.len() != 3 || origins.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(super::shared::query_error(format!(
            "each reaffirmation carries its own increasing event_origin, read {origins:?}"
        ))
        .into());
    }
    if rows.iter().any(|row| text(row, "reaffirmed_at").is_none()) {
        return Err(super::shared::query_error("a reaffirmation carries its time").into());
    }

    let reaffirmation_ids: Vec<String> = rows.iter().filter_map(|row| text(row, "id")).collect();
    for relation in RelationKind::ALL {
        for (from_id, to_id) in super::shared::relation_edges(graph, relation)? {
            if reaffirmation_ids.contains(&from_id) || reaffirmation_ids.contains(&to_id) {
                return Err(super::shared::query_error(format!(
                    "a reaffirmation has no edges, found {relation:?} {from_id} -> {to_id}"
                ))
                .into());
            }
        }
    }
    Ok(())
}

// ── re-examine scenario (hivemind-ottw.2) ───────────────────────────────────────────

/// The instant [`reexamine_scenario`] is read at: what "unfinished" is judged against.
pub(crate) const REEXAMINE_NOW: &str = "2026-10-01T00:00:00Z";

/// Premises that changed, and the decisions that rest on them, in every state an item can be in.
///
/// - `d:p-sup` (billing) is superseded by `d:p-sup-new` on 2026-09-02, with five dependents:
///   `d:open` (nobody looked again), `d:twice` (platform; reaffirmed by alice and by a reviewer
///   agent), `d:after-sup` (itself superseded afterwards), `d:after-rej` (itself rejected
///   afterwards) and `d:late-link` (marketing; linked after the change). `d:before-sup` was
///   superseded *before* the change, so it has no item at all.
/// - `d:p-conc` is superseded twice; the supersession recorded first names the later decision.
/// - `d:p-rej` is rejected, `d:p-cont` contested; `d:p-cont2` is contested, reaffirmed, and then
///   superseded (the earlier reaffirmation no longer closes it); `d:p-cont3` is contested and
///   reaffirmed.
/// - `h:assumption` and `h:bet` are refuted (the assumption twice, the earlier evidence
///   recorded second); `d:on-bet` was reaffirmed, `d:on-assumption` was not. `d:on-fine` rests on
///   a hypothesis nobody refuted.
/// - `d:p-edge-eq` and `d:p-edge-over` were superseded exactly 14 days and 14 days and a second
///   before `REEXAMINE_NOW`.
/// - A `FOLLOWS_FROM` cycle (`d:cyc-a` and `d:cyc-b`), a decision that follows itself, and a
///   superseded decision nothing rests on.
///
/// Only the `d:p-sup` family is filed under projects (billing is part of platform; marketing is
/// apart), so a question asked from billing sees four of its five dependents and counts the fifth.
pub(crate) fn reexamine_scenario() -> Result<Scenario> {
    let s = Scenario::new();
    let (alex, alice, bob) = ("human:alex", "human:alice", "human:bob");
    let (crew, reviewer) = ("agent:claude:crew", "agent:codex:reviewer");

    let commands = Commands::new(s.ledger());
    commands.register_project(alex, "billing", Some("Billing"), None)?;
    commands.register_project(alex, "platform", Some("Platform"), None)?;
    commands.register_project(alex, "marketing", Some("Marketing"), None)?;
    commands.link_project(alex, "billing", "platform", ProjectLinkKind::PartOf)?;

    let stand = |ids: &[&str]| -> Result<()> {
        for id in ids {
            s.accept(id, alex, "2026-09-01T12:00:00Z")?;
        }
        Ok(())
    };
    let follows = |dependent: &str, premise: &str| -> Result<()> {
        s.relation(
            "FOLLOWS_FROM",
            dependent,
            premise,
            crew,
            None,
            "2026-09-01T12:00:30Z",
        )
    };

    // The d:p-sup family.
    for (id, title, project, at) in [
        (
            "d:p-sup",
            "Cap requests per tenant",
            "billing",
            "2026-09-01T00:00:00Z",
        ),
        (
            "d:p-sup-new",
            "Cap requests per user",
            "billing",
            "2026-09-02T00:00:00Z",
        ),
        (
            "d:open",
            "Shard by tenant",
            "billing",
            "2026-09-01T00:01:00Z",
        ),
        (
            "d:twice",
            "Bill by request count",
            "platform",
            "2026-09-01T00:02:00Z",
        ),
        (
            "d:after-sup",
            "Quota dashboard per tenant",
            "billing",
            "2026-09-01T00:03:00Z",
        ),
        (
            "d:after-sup-v2",
            "Quota dashboard per user",
            "billing",
            "2026-09-01T00:04:00Z",
        ),
        (
            "d:after-rej",
            "Tenant-level alerts",
            "billing",
            "2026-09-01T00:05:00Z",
        ),
        (
            "d:late-link",
            "Promo caps per tenant",
            "marketing",
            "2026-09-01T00:06:00Z",
        ),
        (
            "d:before-sup",
            "Per-tenant invoices",
            "billing",
            "2026-09-01T00:07:00Z",
        ),
        (
            "d:before-sup-v2",
            "Per-user invoices",
            "billing",
            "2026-09-01T00:08:00Z",
        ),
    ] {
        s.decision_in(id, title, project, crew, at)?;
    }
    stand(&[
        "d:p-sup",
        "d:p-sup-new",
        "d:open",
        "d:twice",
        "d:after-sup",
        "d:after-sup-v2",
        "d:late-link",
        "d:before-sup",
        "d:before-sup-v2",
    ])?;
    for dependent in [
        "d:open",
        "d:twice",
        "d:after-sup",
        "d:after-rej",
        "d:before-sup",
    ] {
        follows(dependent, "d:p-sup")?;
    }
    s.supersede(
        "d:before-sup",
        "d:before-sup-v2",
        alex,
        "2026-09-01T13:00:00Z",
    )?;
    s.supersede("d:p-sup", "d:p-sup-new", alex, "2026-09-02T00:00:01Z")?;
    s.reaffirmed(
        "d:twice",
        "d:p-sup",
        "Billing by request count never depended on the per-tenant cap",
        alice,
        "2026-09-03T00:00:00Z",
    )?;
    s.reaffirmed(
        "d:twice",
        "d:p-sup",
        "Checked again against the per-user cap",
        reviewer,
        "2026-09-04T00:00:00Z",
    )?;
    s.supersede(
        "d:after-sup",
        "d:after-sup-v2",
        alex,
        "2026-09-05T00:00:00Z",
    )?;
    s.reject("d:after-rej", bob, "2026-09-05T00:00:01Z")?;
    follows("d:late-link", "d:p-sup")?;

    // Concurrent superseders: recorded b first, then a.
    for (id, title, at) in [
        (
            "d:p-conc",
            "Retry failed jobs twice",
            "2026-09-01T01:00:00Z",
        ),
        ("d:conc-a", "Retry failed jobs once", "2026-09-10T00:00:00Z"),
        (
            "d:conc-b",
            "Never retry failed jobs",
            "2026-09-12T00:00:00Z",
        ),
        (
            "d:on-conc",
            "Alert after two retries",
            "2026-09-01T01:01:00Z",
        ),
    ] {
        s.decision(id, title, crew, at)?;
    }
    stand(&["d:p-conc", "d:conc-a", "d:conc-b", "d:on-conc"])?;
    follows("d:on-conc", "d:p-conc")?;
    s.supersede("d:p-conc", "d:conc-b", alex, "2026-09-12T00:00:01Z")?;
    s.supersede("d:p-conc", "d:conc-a", alice, "2026-09-12T00:00:02Z")?;

    // A rejected premise: one dependent open, one reaffirmed.
    for (id, title, at) in [
        ("d:p-rej", "Adopt vendor X", "2026-08-01T00:00:00Z"),
        (
            "d:on-rej",
            "Migrate the queue to vendor X",
            "2026-08-01T00:01:00Z",
        ),
        (
            "d:on-rej-ok",
            "Train support on vendor X",
            "2026-08-01T00:02:00Z",
        ),
    ] {
        s.decision(id, title, crew, at)?;
    }
    stand(&["d:on-rej", "d:on-rej-ok"])?;
    follows("d:on-rej", "d:p-rej")?;
    follows("d:on-rej-ok", "d:p-rej")?;
    s.reject("d:p-rej", bob, "2026-08-02T00:00:00Z")?;
    s.reaffirmed(
        "d:on-rej-ok",
        "d:p-rej",
        "Support training is vendor-neutral",
        alice,
        "2026-08-03T00:00:00Z",
    )?;

    // Contested premises.
    for (id, title, at) in [
        ("d:p-cont", "Use queue Y", "2026-09-01T02:00:00Z"),
        (
            "d:on-cont",
            "Size the workers for queue Y",
            "2026-09-01T02:01:00Z",
        ),
        ("d:p-cont2", "Use cache Z", "2026-09-01T02:02:00Z"),
        (
            "d:on-cont2",
            "Warm cache Z at deploy",
            "2026-09-01T02:03:00Z",
        ),
        ("d:cont2-new", "Use cache W", "2026-09-20T00:00:00Z"),
        ("d:p-cont3", "Use region R", "2026-09-01T02:04:00Z"),
        (
            "d:on-cont3",
            "Pin the database to region R",
            "2026-09-01T02:05:00Z",
        ),
    ] {
        s.decision(id, title, crew, at)?;
    }
    stand(&["d:on-cont", "d:on-cont2", "d:cont2-new", "d:on-cont3"])?;
    follows("d:on-cont", "d:p-cont")?;
    follows("d:on-cont2", "d:p-cont2")?;
    follows("d:on-cont3", "d:p-cont3")?;
    s.accept("d:p-cont", alice, "2026-09-06T00:00:00Z")?;
    s.reject("d:p-cont", bob, "2026-09-06T00:00:01Z")?;
    s.accept("d:p-cont2", alice, "2026-09-06T00:01:00Z")?;
    s.reject("d:p-cont2", bob, "2026-09-06T00:01:01Z")?;
    s.reaffirmed(
        "d:on-cont2",
        "d:p-cont2",
        "Warming is useful whichever cache wins",
        alice,
        "2026-09-07T00:00:00Z",
    )?;
    s.supersede("d:p-cont2", "d:cont2-new", alex, "2026-09-20T00:00:01Z")?;
    s.accept("d:p-cont3", alice, "2026-09-06T00:02:00Z")?;
    s.reject("d:p-cont3", bob, "2026-09-06T00:02:01Z")?;
    s.reaffirmed(
        "d:on-cont3",
        "d:p-cont3",
        "The database stays in region R either way",
        alice,
        "2026-09-07T00:01:00Z",
    )?;

    // Refuted hypotheses.
    s.hypothesis(
        "h:assumption",
        "Users log in daily",
        "assumption",
        None,
        "2026-09-01T03:00:00Z",
    )?;
    s.hypothesis(
        "h:bet",
        "The cache holds under load",
        "bet",
        Some("2026-12-01T00:00:00Z"),
        "2026-09-01T03:00:01Z",
    )?;
    s.hypothesis(
        "h:fine",
        "The API is stable",
        "assumption",
        None,
        "2026-09-01T03:00:02Z",
    )?;
    s.evidence(
        "e:r1",
        "Median session length fell below one a week",
        Some("analytics"),
        "2026-09-05T00:00:00Z",
    )?;
    s.evidence(
        "e:r2",
        "Weekly actives are half the daily estimate",
        Some("analytics"),
        "2026-09-03T00:00:00Z",
    )?;
    s.evidence(
        "e:bet",
        "p95 doubled under load",
        Some("dashboard"),
        "2026-09-12T00:00:00Z",
    )?;
    s.decision_with(
        "d:on-assumption",
        "Send the digest daily",
        crew,
        "2026-09-01T03:01:00Z",
        false,
        &[],
        &["h:assumption"],
        None,
    )?;
    s.decision_with(
        "d:on-bet",
        "Skip the second cache tier",
        crew,
        "2026-09-01T03:02:00Z",
        true,
        &[],
        &["h:bet"],
        None,
    )?;
    s.decision_with(
        "d:on-fine",
        "Pin the client to the v2 API",
        crew,
        "2026-09-01T03:03:00Z",
        false,
        &[],
        &["h:fine"],
        None,
    )?;
    stand(&["d:on-assumption", "d:on-bet", "d:on-fine"])?;
    s.relation(
        "REFUTES",
        "e:r1",
        "h:assumption",
        alex,
        None,
        "2026-09-05T00:00:01Z",
    )?;
    s.relation(
        "REFUTES",
        "e:r2",
        "h:assumption",
        alex,
        None,
        "2026-09-05T00:00:02Z",
    )?;
    s.relation(
        "REFUTES",
        "e:bet",
        "h:bet",
        alex,
        None,
        "2026-09-12T00:00:01Z",
    )?;
    s.reaffirmed(
        "d:on-bet",
        "h:bet",
        "The second tier would not have helped at this load",
        reviewer,
        "2026-09-13T00:00:00Z",
    )?;

    // Exactly 14 days, and 14 days and a second, before REEXAMINE_NOW.
    for (id, title, at) in [
        ("d:p-edge-eq", "Ship weekly", "2026-09-01T04:00:00Z"),
        ("d:edge-eq-new", "Ship daily", "2026-09-17T00:00:00Z"),
        (
            "d:on-edge-eq",
            "Staff the weekly release",
            "2026-09-01T04:01:00Z",
        ),
        (
            "d:p-edge-over",
            "Review every change twice",
            "2026-09-01T04:02:00Z",
        ),
        (
            "d:edge-over-new",
            "Review every change once",
            "2026-09-16T23:59:59Z",
        ),
        (
            "d:on-edge-over",
            "Hire a second reviewer",
            "2026-09-01T04:03:00Z",
        ),
    ] {
        s.decision(id, title, crew, at)?;
    }
    stand(&[
        "d:p-edge-eq",
        "d:edge-eq-new",
        "d:on-edge-eq",
        "d:p-edge-over",
        "d:edge-over-new",
        "d:on-edge-over",
    ])?;
    follows("d:on-edge-eq", "d:p-edge-eq")?;
    follows("d:on-edge-over", "d:p-edge-over")?;
    s.supersede("d:p-edge-eq", "d:edge-eq-new", alex, "2026-09-17T00:00:01Z")?;
    s.supersede(
        "d:p-edge-over",
        "d:edge-over-new",
        alex,
        "2026-09-16T23:59:59Z",
    )?;

    // A cycle, a decision that follows itself, a superseded decision nothing rests on.
    for (id, title, at) in [
        ("d:cyc-a", "Run the batch at night", "2026-09-01T05:00:00Z"),
        (
            "d:cyc-b",
            "Reserve the night window",
            "2026-09-01T05:01:00Z",
        ),
        (
            "d:cyc-a-new",
            "Run the batch at noon",
            "2026-09-05T00:00:00Z",
        ),
        ("d:self", "Rotate keys yearly", "2026-09-01T05:02:00Z"),
        ("d:self-new", "Rotate keys monthly", "2026-09-05T00:00:00Z"),
        ("d:lonely", "Name the service Atlas", "2026-09-01T05:03:00Z"),
        (
            "d:lonely-new",
            "Name the service Orion",
            "2026-09-05T00:00:00Z",
        ),
    ] {
        s.decision(id, title, crew, at)?;
    }
    stand(&[
        "d:cyc-a",
        "d:cyc-b",
        "d:cyc-a-new",
        "d:self",
        "d:self-new",
        "d:lonely",
        "d:lonely-new",
    ])?;
    follows("d:cyc-a", "d:cyc-b")?;
    follows("d:cyc-b", "d:cyc-a")?;
    follows("d:self", "d:self")?;
    s.supersede("d:cyc-a", "d:cyc-a-new", alex, "2026-09-05T00:00:01Z")?;
    s.supersede("d:self", "d:self-new", alex, "2026-09-05T00:00:02Z")?;
    s.supersede("d:lonely", "d:lonely-new", alex, "2026-09-05T00:00:03Z")?;
    Ok(s)
}

/// What every backend must read from [`reexamine_scenario`] at `REEXAMINE_NOW`: the same sixteen
/// items in the same order, whole or paged, the same completeness for each changed premise, the
/// same answer from a project, and the same entries on each dependent's own outcome as in the
/// list. Shared by the memory, Postgres and Kuzu tests.
pub(crate) fn assert_reexamine_scenario(scenario: &Scenario, graph: &impl GraphView) -> Result<()> {
    use crate::queries::{
        get_decision_outcome_at, get_propagation_at, get_reexamine_items_at, ChangeKind, ItemState,
        OutcomeReason, PremiseKind, QueryContext, ReexamineItem, ReexamineRequest,
        ReexamineResults,
    };

    let now = ts(REEXAMINE_NOW);
    let context = QueryContext::local();
    let ledger = scenario.ledger();
    let list = |request: ReexamineRequest| -> Result<ReexamineResults> {
        Ok(get_reexamine_items_at(&context, graph, ledger, &request, now)?.data)
    };
    let tag = |state: &ItemState| -> String {
        match state {
            ItemState::Open => "open".to_owned(),
            ItemState::Reaffirmed { reaffirmations } => {
                format!("reaffirmed:{}", reaffirmations.len())
            }
            ItemState::DependentSuperseded { by_id } => format!("dependent_superseded:{by_id}"),
            ItemState::DependentRejected => "dependent_rejected".to_owned(),
        }
    };
    let ids = |items: &[ReexamineItem]| -> Vec<(String, String)> {
        items
            .iter()
            .map(|item| (item.dependent.id.clone(), item.entry.premise_id.clone()))
            .collect()
    };

    // ── the items, whole ────────────────────────────────────────────────────────
    use ChangeKind::{Contested, Refuted, Rejected, Superseded};
    let mut expected: Vec<(&str, &str, ChangeKind, &str, usize)> = vec![
        (
            "d:after-rej",
            "d:p-sup",
            Superseded,
            "dependent_rejected",
            0,
        ),
        (
            "d:after-sup",
            "d:p-sup",
            Superseded,
            "dependent_superseded:d:after-sup-v2",
            0,
        ),
        ("d:cyc-b", "d:cyc-a", Superseded, "open", 0),
        ("d:late-link", "d:p-sup", Superseded, "open", 0),
        ("d:on-assumption", "h:assumption", Refuted, "open", 0),
        ("d:on-bet", "h:bet", Refuted, "reaffirmed:1", 0),
        ("d:on-conc", "d:p-conc", Superseded, "open", 0),
        ("d:on-cont", "d:p-cont", Contested, "open", 0),
        ("d:on-cont2", "d:p-cont2", Superseded, "open", 1),
        ("d:on-cont3", "d:p-cont3", Contested, "reaffirmed:1", 0),
        ("d:on-edge-eq", "d:p-edge-eq", Superseded, "open", 0),
        ("d:on-edge-over", "d:p-edge-over", Superseded, "open", 0),
        ("d:on-rej", "d:p-rej", Rejected, "open", 0),
        ("d:on-rej-ok", "d:p-rej", Rejected, "reaffirmed:1", 0),
        ("d:open", "d:p-sup", Superseded, "open", 0),
        ("d:twice", "d:p-sup", Superseded, "reaffirmed:2", 0),
    ];
    expected.sort_by_key(|row| (row.0, row.1));
    let summary = |items: &[ReexamineItem]| -> Vec<(String, String, ChangeKind, String, usize)> {
        items
            .iter()
            .map(|item| {
                (
                    item.dependent.id.clone(),
                    item.entry.premise_id.clone(),
                    item.entry.change.kind,
                    tag(&item.entry.state),
                    item.entry.earlier_reaffirmations.len(),
                )
            })
            .collect()
    };
    let want = |only_open: bool| -> Vec<(String, String, ChangeKind, String, usize)> {
        expected
            .iter()
            .filter(|row| !only_open || row.3 == "open")
            .map(|row| {
                (
                    row.0.to_owned(),
                    row.1.to_owned(),
                    row.2,
                    row.3.to_owned(),
                    row.4,
                )
            })
            .collect()
    };

    let everything = list(ReexamineRequest {
        include_closed: true,
        limit: 100,
        ..ReexamineRequest::default()
    })?;
    assert_eq!(summary(&everything.items), want(false));
    assert_eq!(everything.next_cursor, None);
    assert_eq!(everything.not_shown, None);
    assert!(everything.scope.is_none() && everything.propagation.is_none());

    // Open ones only, by default: what is still waiting for someone to look again.
    let open = list(ReexamineRequest {
        limit: 100,
        ..ReexamineRequest::default()
    })?;
    assert_eq!(summary(&open.items), want(true));
    assert_eq!(open.items.len(), 10);

    // ── offsets order a reaffirmation against its change ───────────────────────────
    for item in &everything.items {
        let change = item
            .entry
            .change
            .at_offset
            .expect("every change has an offset");
        let reaffirmed = match &item.entry.state {
            ItemState::Reaffirmed { reaffirmations } => reaffirmations.as_slice(),
            _ => &[],
        };
        for reaffirmation in reaffirmed {
            assert!(
                reaffirmation.event_origin.expect("reaffirmation offset") > change,
                "{} closes only after the change",
                item.dependent.id
            );
        }
        for earlier in &item.entry.earlier_reaffirmations {
            assert!(
                earlier.event_origin.expect("reaffirmation offset") < change,
                "{} keeps only reaffirmations from before the change",
                item.dependent.id
            );
        }
    }
    let find = |dependent: &str| -> &ReexamineItem {
        everything
            .items
            .iter()
            .find(|item| item.dependent.id == dependent)
            .unwrap_or_else(|| panic!("no item on {dependent}"))
    };

    // ── who, when, in whose words ───────────────────────────────────────────────
    let day = |text: &str| Some(ts(text));
    let change_of = |dependent: &str| find(dependent).entry.change.clone();
    assert_eq!(change_of("d:open").at_time, day("2026-09-02T00:00:00Z"));
    assert_eq!(change_of("d:open").by_ids, ["d:p-sup-new"]);
    // Concurrent superseders: both named, the one recorded first dates the change.
    assert_eq!(change_of("d:on-conc").by_ids, ["d:conc-a", "d:conc-b"]);
    assert_eq!(change_of("d:on-conc").at_time, day("2026-09-12T00:00:00Z"));
    // A rejection or a contest states no time in the graph.
    assert_eq!(change_of("d:on-rej").by_ids, ["human:bob"]);
    assert_eq!(change_of("d:on-rej").at_time, None);
    assert_eq!(change_of("d:on-cont").at_time, None);
    // A refutation is dated by the earliest evidence, not the first one recorded.
    assert_eq!(change_of("d:on-assumption").by_ids, ["e:r1", "e:r2"]);
    assert_eq!(
        change_of("d:on-assumption").at_time,
        day("2026-09-03T00:00:00Z")
    );
    // A re-change is a new change, dated by its own supersession.
    assert_eq!(change_of("d:on-cont2").at_time, day("2026-09-20T00:00:00Z"));
    let open_item = find("d:open");
    assert_eq!(open_item.dependent.title, "Shard by tenant");
    assert_eq!(open_item.dependent.project.as_deref(), Some("billing"));
    assert_eq!(open_item.dependent.project_label, "Billing");
    assert_eq!(open_item.entry.premise_label, "Cap requests per tenant");
    assert_eq!(open_item.entry.premise_kind, PremiseKind::Decision);
    assert_eq!(open_item.premise_project.as_deref(), Some("billing"));
    // An item that crossed a project link names where the change came from.
    assert_eq!(
        find("d:twice").dependent.project.as_deref(),
        Some("platform")
    );
    assert_eq!(find("d:twice").premise_project.as_deref(), Some("billing"));
    assert_eq!(
        find("d:twice").premise_project_label.as_deref(),
        Some("Billing")
    );
    let hypothesis_item = find("d:on-assumption");
    assert_eq!(hypothesis_item.entry.premise_kind, PremiseKind::Hypothesis);
    assert_eq!(hypothesis_item.entry.premise_label, "Users log in daily");
    assert_eq!(hypothesis_item.premise_project, None);
    if let ItemState::Reaffirmed { reaffirmations } = &find("d:twice").entry.state {
        let by: Vec<&str> = reaffirmations.iter().map(|r| r.actor_id.as_str()).collect();
        assert_eq!(by, ["human:alice", "agent:codex:reviewer"]);
        assert_eq!(
            reaffirmations[1].reason,
            "Checked again against the per-user cap"
        );
    } else {
        panic!("d:twice is reaffirmed");
    }
    // Contested, reaffirmed, then superseded: reopened, with the earlier word kept.
    let reopened = find("d:on-cont2");
    assert_eq!(tag(&reopened.entry.state), "open");
    assert_eq!(
        reopened.entry.earlier_reaffirmations[0].reason,
        "Warming is useful whichever cache wins"
    );

    // A dependent that was out before the change, a decision that follows itself, a hypothesis
    // nobody refuted: no item.
    for absent in ["d:before-sup", "d:self", "d:on-fine", "d:p-sup", "d:lonely"] {
        assert!(
            everything
                .items
                .iter()
                .all(|item| item.dependent.id != absent),
            "{absent} has no item"
        );
    }

    // ── paging: nothing cut silently ─────────────────────────────────────────────
    for (include_closed, limit, sizes) in [
        (false, 2, &[2_usize, 2, 2, 2, 2][..]),
        (true, 3, &[3, 3, 3, 3, 3, 1][..]),
        (true, 16, &[16][..]),
    ] {
        let mut cursor = None;
        let mut seen = Vec::new();
        let mut page_sizes = Vec::new();
        loop {
            let response = get_reexamine_items_at(
                &context,
                graph,
                ledger,
                &ReexamineRequest {
                    include_closed,
                    limit,
                    cursor: cursor.clone(),
                    ..ReexamineRequest::default()
                },
                now,
            )?;
            assert_eq!(response.truncated, response.data.next_cursor.is_some());
            assert_eq!(response.result_count, response.data.items.len());
            page_sizes.push(response.data.items.len());
            seen.extend(ids(&response.data.items));
            cursor = response.data.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(
            page_sizes, sizes,
            "include_closed={include_closed} limit={limit}"
        );
        let whole = if include_closed { &everything } else { &open };
        assert_eq!(seen, ids(&whole.items));
    }

    // ── filters ──────────────────────────────────────────────────────────────────
    let one = list(ReexamineRequest {
        dependent_id: Some("d:twice".to_owned()),
        include_closed: true,
        ..ReexamineRequest::default()
    })?;
    assert_eq!(
        ids(&one.items),
        [("d:twice".to_owned(), "d:p-sup".to_owned())]
    );
    let of_premise = list(ReexamineRequest {
        premise_id: Some("d:p-sup".to_owned()),
        include_closed: true,
        ..ReexamineRequest::default()
    })?;
    assert_eq!(of_premise.items.len(), 5);
    assert!(of_premise.propagation.is_some());

    // ── N of M, and unfinished ───────────────────────────────────────────────────
    // (premise, kind, change, M, N, still open, unfinished, oldest open since)
    type Row = (
        &'static str,
        PremiseKind,
        ChangeKind,
        usize,
        usize,
        usize,
        bool,
        Option<&'static str>,
    );
    let rows: [Row; 11] = [
        (
            "d:p-sup",
            PremiseKind::Decision,
            Superseded,
            5,
            3,
            2,
            true,
            Some("2026-09-02T00:00:00Z"),
        ),
        (
            "d:p-conc",
            PremiseKind::Decision,
            Superseded,
            1,
            0,
            1,
            true,
            Some("2026-09-12T00:00:00Z"),
        ),
        (
            "d:p-rej",
            PremiseKind::Decision,
            Rejected,
            2,
            1,
            1,
            false,
            None,
        ),
        (
            "d:p-cont",
            PremiseKind::Decision,
            Contested,
            1,
            0,
            1,
            false,
            None,
        ),
        (
            "d:p-cont2",
            PremiseKind::Decision,
            Superseded,
            1,
            0,
            1,
            false,
            Some("2026-09-20T00:00:00Z"),
        ),
        (
            "d:p-cont3",
            PremiseKind::Decision,
            Contested,
            1,
            1,
            0,
            false,
            None,
        ),
        (
            "h:assumption",
            PremiseKind::Hypothesis,
            Refuted,
            1,
            0,
            1,
            true,
            Some("2026-09-03T00:00:00Z"),
        ),
        (
            "h:bet",
            PremiseKind::Hypothesis,
            Refuted,
            1,
            1,
            0,
            false,
            None,
        ),
        // Exactly 14 days is not past the window; a second more is.
        (
            "d:p-edge-eq",
            PremiseKind::Decision,
            Superseded,
            1,
            0,
            1,
            false,
            Some("2026-09-17T00:00:00Z"),
        ),
        (
            "d:p-edge-over",
            PremiseKind::Decision,
            Superseded,
            1,
            0,
            1,
            true,
            Some("2026-09-16T23:59:59Z"),
        ),
        (
            "d:cyc-a",
            PremiseKind::Decision,
            Superseded,
            1,
            0,
            1,
            true,
            Some("2026-09-05T00:00:00Z"),
        ),
    ];
    for (premise, kind, change, total, done, still_open, unfinished, since) in rows {
        let propagation = get_propagation_at(graph, premise, None, now)?
            .unwrap_or_else(|| panic!("{premise} has a propagation"));
        assert_eq!(propagation.premise_kind, kind, "{premise}");
        assert_eq!(propagation.change.kind, change, "{premise}");
        assert_eq!(
            (
                propagation.dependents_total,
                propagation.reexamined,
                propagation.open
            ),
            (total, done, still_open),
            "{premise}: N of M"
        );
        assert_eq!(propagation.unfinished, unfinished, "{premise}: unfinished");
        assert_eq!(propagation.window_days, 14);
        assert_eq!(
            propagation.oldest_open_since,
            since.and_then(day).filter(|_| still_open > 0),
            "{premise}: oldest open since"
        );
        assert_eq!(
            (propagation.not_shown, propagation.not_shown_open),
            (None, None)
        );
    }
    // The same change, a different window: 2026-09-02 to REEXAMINE_NOW is exactly 29 days.
    let within = |days: u32| -> Result<bool> {
        Ok(get_propagation_at(graph, "d:p-sup", Some(days), now)?
            .expect("d:p-sup changed")
            .unfinished)
    };
    assert!(!within(29)? && within(28)? && !within(60)?);
    // Nothing to propagate: not changed, no dependent that stood, a self-link, an unknown id.
    for quiet in ["d:p-sup-new", "d:lonely", "d:self", "h:fine", "d:nowhere"] {
        assert!(
            get_propagation_at(graph, quiet, None, now)?.is_none(),
            "{quiet}"
        );
    }

    // ── asked from a project: counted, never named ─────────────────────────────────
    let from_billing = list(ReexamineRequest {
        project: Some("billing".to_owned()),
        include_closed: true,
        limit: 100,
        ..ReexamineRequest::default()
    })?;
    assert_eq!(
        ids(&from_billing.items),
        [
            ("d:after-rej", "d:p-sup"),
            ("d:after-sup", "d:p-sup"),
            ("d:open", "d:p-sup"),
            ("d:twice", "d:p-sup"),
        ]
        .map(|(dependent, premise)| (dependent.to_owned(), premise.to_owned()))
    );
    assert_eq!(from_billing.not_shown, Some(12));
    let scope = from_billing
        .scope
        .expect("a scoped answer says where it looked");
    let followed: Vec<&str> = scope.followed.iter().map(|p| p.project.as_str()).collect();
    assert_eq!(followed, ["billing", "platform"]);
    let scoped_open = list(ReexamineRequest {
        project: Some("billing".to_owned()),
        limit: 100,
        ..ReexamineRequest::default()
    })?;
    assert_eq!(
        ids(&scoped_open.items),
        [("d:open".to_owned(), "d:p-sup".to_owned())]
    );
    assert_eq!(scoped_open.not_shown, Some(9));
    let scoped_premise = list(ReexamineRequest {
        project: Some("billing".to_owned()),
        premise_id: Some("d:p-sup".to_owned()),
        include_closed: true,
        ..ReexamineRequest::default()
    })?;
    assert_eq!(scoped_premise.not_shown, Some(1));
    let propagation = scoped_premise.propagation.expect("the premise changed");
    assert_eq!(
        (
            propagation.dependents_total,
            propagation.reexamined,
            propagation.open,
            propagation.not_shown,
            propagation.not_shown_open,
        ),
        (4, 3, 1, Some(1), Some(1))
    );
    // The one open item the scope hides still counts toward unfinished.
    assert!(propagation.unfinished);

    // ── the outcome carries the same entries as the list ─────────────────────────────
    let mut dependents: Vec<&str> = everything
        .items
        .iter()
        .map(|item| item.dependent.id.as_str())
        .collect();
    dependents.dedup();
    for dependent in &dependents {
        let outcome = get_decision_outcome_at(graph, dependent, now)?
            .data
            .unwrap_or_else(|| panic!("{dependent} has an outcome"));
        let listed: Vec<_> = everything
            .items
            .iter()
            .filter(|item| item.dependent.id == *dependent)
            .map(|item| item.entry.clone())
            .collect();
        assert_eq!(outcome.reexamine, listed, "{dependent}: outcome vs list");
    }
    for absent in ["d:before-sup", "d:self", "d:on-fine", "d:p-sup", "d:lonely"] {
        let outcome = get_decision_outcome_at(graph, absent, now)?
            .data
            .unwrap_or_else(|| panic!("{absent} has an outcome"));
        assert!(outcome.reexamine.is_empty(), "{absent}");
    }

    // R1 and R3: a reaffirmed change holds up again (the reason stays, with the entry saying who
    // looked), a contested premise opens an item and never flips `held_up`, a re-change reopens.
    for (dependent, held_up) in [
        ("d:open", false),
        ("d:twice", true),
        ("d:after-sup", false),
        ("d:on-assumption", false),
        ("d:on-bet", true),
        ("d:on-conc", false),
        ("d:on-cont", true),
        ("d:on-cont2", false),
        ("d:on-cont3", true),
        ("d:on-rej", false),
        ("d:on-rej-ok", true),
        ("d:on-edge-over", false),
    ] {
        let outcome = get_decision_outcome_at(graph, dependent, now)?
            .data
            .expect("outcome");
        assert_eq!(outcome.held_up, held_up, "{dependent}: held_up");
    }
    let twice = get_decision_outcome_at(graph, "d:twice", now)?
        .data
        .expect("outcome");
    assert!(twice.stale_premises);
    assert!(twice.reasons.iter().any(|reason| matches!(
        reason,
        OutcomeReason::PremiseSuperseded { decision_id, .. } if decision_id == "d:p-sup"
    )));
    let bet = get_decision_outcome_at(graph, "d:on-bet", now)?
        .data
        .expect("outcome");
    assert!(bet.reasons.iter().any(|reason| matches!(
        reason,
        OutcomeReason::PremisedOnRefuted { hypothesis_id } if hypothesis_id == "h:bet"
    )));
    Ok(())
}

// ── a decision that supersedes its own premise (hivemind-tfmfn) ──────────────────────

/// Revisions: a decision that follows its premise and supersedes it, which is how "B revises A"
/// is normally said. Read at `REEXAMINE_NOW`.
///
/// - `d:old` is superseded by `d:revised` alone, and `d:revised` follows from it: the revision is
///   the change, so it is no dependent of `d:old` and has no item. `d:bystander` follows from
///   `d:old` too and did not make the change, so it has the one item.
/// - `d:two` is superseded by both `d:two-a` and `d:two-b`, each of which follows from it. Each is
///   told about the other supersession, so each keeps its item.
/// - `d:solo` is superseded by `d:solo-new` alone, which follows it, and nothing else rests on
///   it: no dependent stood when it changed, so it has no completeness line at all.
pub(crate) fn revisions_scenario() -> Result<Scenario> {
    let s = Scenario::new();
    let (alex, crew) = ("human:alex", "agent:claude:crew");
    let decisions = [
        ("d:old", "Cap requests per tenant", "2026-09-01T00:00:00Z"),
        (
            "d:revised",
            "Cap requests per tenant, tuned",
            "2026-09-02T00:00:00Z",
        ),
        ("d:bystander", "Shard by tenant", "2026-09-01T00:01:00Z"),
        ("d:two", "Use one region", "2026-09-01T00:02:00Z"),
        ("d:two-a", "Use two regions", "2026-09-03T00:00:00Z"),
        ("d:two-b", "Use three regions", "2026-09-04T00:00:00Z"),
        ("d:solo", "Name the service Atlas", "2026-09-01T00:03:00Z"),
        (
            "d:solo-new",
            "Name the service Atlas, in full",
            "2026-09-05T00:00:00Z",
        ),
    ];
    for (id, title, at) in decisions {
        s.decision(id, title, crew, at)?;
        s.accept(id, alex, "2026-09-06T00:00:00Z")?;
    }
    for (dependent, premise) in [
        ("d:revised", "d:old"),
        ("d:bystander", "d:old"),
        ("d:two-a", "d:two"),
        ("d:two-b", "d:two"),
        ("d:solo-new", "d:solo"),
    ] {
        s.relation(
            "FOLLOWS_FROM",
            dependent,
            premise,
            crew,
            None,
            "2026-09-06T00:00:01Z",
        )?;
    }
    s.supersede("d:old", "d:revised", alex, "2026-09-02T00:00:01Z")?;
    s.supersede("d:two", "d:two-a", alex, "2026-09-03T00:00:01Z")?;
    s.supersede("d:two", "d:two-b", alex, "2026-09-04T00:00:01Z")?;
    s.supersede("d:solo", "d:solo-new", alex, "2026-09-05T00:00:01Z")?;
    Ok(s)
}

/// What every backend must read from [`revisions_scenario`] at `REEXAMINE_NOW`: a decision that is
/// the only one to supersede its premise is no dependent of it, on every surface that counts
/// dependents (the list, the premise's completeness read in bulk and anchored, the marker, the
/// decision's own outcome); one of several superseders still is. Shared by the memory and
/// Postgres tests.
pub(crate) fn assert_revisions_scenario(scenario: &Scenario, graph: &impl GraphView) -> Result<()> {
    use super::reexamine::ReexamineMarkers;
    use crate::queries::{
        get_decision_brief_at, get_decision_outcome_at, get_propagation_at, get_reexamine_items_at,
        OutcomeReason, QueryContext, ReexamineMarker, ReexamineRequest,
    };

    let now = ts(REEXAMINE_NOW);

    // ── the list: three items, none of them the revision's own ──────────────────────
    let request = ReexamineRequest {
        include_closed: true,
        limit: 100,
        ..ReexamineRequest::default()
    };
    let items = get_reexamine_items_at(
        &QueryContext::local(),
        graph,
        scenario.ledger(),
        &request,
        now,
    )?
    .data
    .items;
    let pairs: Vec<(&str, &str)> = items
        .iter()
        .map(|item| (item.dependent.id.as_str(), item.entry.premise_id.as_str()))
        .collect();
    assert_eq!(
        pairs,
        [
            ("d:bystander", "d:old"),
            ("d:two-a", "d:two"),
            ("d:two-b", "d:two")
        ]
    );
    assert!(items.iter().all(|item| item.entry.state.is_open()));

    // ── the premise's completeness: bulk (the list's) and anchored (verify's) agree ──
    let counts = |propagation: &crate::queries::Propagation| {
        (
            propagation.dependents_total,
            propagation.reexamined,
            propagation.open,
        )
    };
    for (premise_id, expected) in [("d:old", (1, 0, 1)), ("d:two", (2, 0, 2))] {
        let bulk = get_propagation_at(graph, premise_id, None, now)?
            .expect("the premise changed and a dependent stood");
        assert_eq!(counts(&bulk), expected, "{premise_id}");
        let verify = get_decision_brief_at(graph, premise_id, now)?
            .data
            .expect("the premise is recorded")
            .propagation;
        assert_eq!(verify.as_ref(), Some(&bulk), "{premise_id}");
    }
    // Nothing but the revision rests on `d:solo`: no dependent, so no completeness line.
    assert_eq!(get_propagation_at(graph, "d:solo", None, now)?, None);
    assert_eq!(
        get_decision_brief_at(graph, "d:solo", now)?
            .data
            .expect("the premise is recorded")
            .propagation,
        None
    );

    // ── the marker: none on a revision, one open item on the rest ────────────────────
    let markers = ReexamineMarkers::load(graph)?;
    assert_eq!(markers.of("d:revised"), None);
    assert_eq!(markers.of("d:solo-new"), None);
    for decision_id in ["d:bystander", "d:two-a", "d:two-b"] {
        assert_eq!(
            markers.of(decision_id),
            Some(ReexamineMarker {
                stale: true,
                open: 1,
                reaffirmed: 0
            }),
            "{decision_id}"
        );
    }

    // ── the decision's own outcome: a revision is not stale for its own change ────────
    for decision_id in ["d:revised", "d:solo-new"] {
        let outcome = get_decision_outcome_at(graph, decision_id, now)?
            .data
            .expect("outcome");
        assert!(outcome.held_up, "{decision_id}");
        assert!(!outcome.stale_premises, "{decision_id}");
        assert!(outcome.reexamine.is_empty(), "{decision_id}");
        assert!(
            !outcome
                .reasons
                .iter()
                .any(|reason| matches!(reason, OutcomeReason::PremiseSuperseded { .. })),
            "{decision_id}"
        );
    }
    for decision_id in ["d:bystander", "d:two-a", "d:two-b"] {
        let outcome = get_decision_outcome_at(graph, decision_id, now)?
            .data
            .expect("outcome");
        assert!(!outcome.held_up, "{decision_id}");
        assert!(outcome.stale_premises, "{decision_id}");
        assert_eq!(outcome.reexamine.len(), 1, "{decision_id}");
    }

    assert_revisions_reaffirm_refusals(scenario, graph)?;
    assert_revisions_history(scenario)
}

/// `reaffirm` on a revision says what is true: the decision is the one that superseded its
/// premise, so it has nothing to re-examine about it. It never says the premise has not changed
/// (hivemind-851kw). One of several superseders has an open item, so its reaffirmation goes
/// through, and a premise nobody changed keeps "has not changed".
fn assert_revisions_reaffirm_refusals(scenario: &Scenario, graph: &impl GraphView) -> Result<()> {
    use crate::queries::QueryContext;
    use crate::reaffirmation::{plan_reaffirmation, ReaffirmPlan};

    let plan = |decision_id: &str, premise: Option<&str>| {
        plan_reaffirmation(
            &QueryContext::local(),
            graph,
            scenario.ledger(),
            decision_id,
            premise,
        )
    };
    let refusal = |decision_id: &str, premise: Option<&str>| -> Result<String> {
        match plan(decision_id, premise)? {
            ReaffirmPlan::Refused(reason) => Ok(reason),
            other => panic!("{decision_id} {premise:?}: expected a refusal, got {other:?}"),
        }
    };

    for (decision_id, premise, premise_title) in [
        ("d:revised", Some("d:old"), "Cap requests per tenant"),
        ("d:revised", Some("tenant"), "Cap requests per tenant"),
        ("d:revised", None, "Cap requests per tenant"),
        ("d:solo-new", Some("d:solo"), "Name the service Atlas"),
        ("d:solo-new", None, "Name the service Atlas"),
    ] {
        let reason = refusal(decision_id, premise)?;
        assert!(
            reason.contains("is the decision that superseded")
                && reason.contains(&format!("\"{premise_title}\"")),
            "{decision_id} {premise:?}: {reason}"
        );
        assert!(
            !reason.contains("has not changed") && !reason.contains("has changed"),
            "{decision_id} {premise:?}: {reason}"
        );
        assert!(
            reason.ends_with("nothing was written"),
            "{decision_id} {premise:?}: {reason}"
        );
    }

    // One of several superseders is told about the others: its item is open.
    for decision_id in ["d:two-a", "d:two-b"] {
        assert!(
            matches!(
                plan(decision_id, Some("d:two"))?,
                ReaffirmPlan::Ready(target) if target.premise_id == "d:two"
            ),
            "{decision_id}"
        );
    }
    assert!(matches!(
        plan("d:bystander", Some("d:old"))?,
        ReaffirmPlan::Ready(_)
    ));
    Ok(())
}

/// The history feeds list as stale on a supersession the decisions that followed the premise, bar
/// the one that alone superseded it (hivemind-851kw): `d:bystander` for `d:old`, nothing for
/// `d:solo`, and both superseders for `d:two`, each told about the other. Read from the ledger
/// alone, so the same on every backend.
fn assert_revisions_history(scenario: &Scenario) -> Result<()> {
    use crate::queries::{
        get_decisions_changed_since, get_recent_activity, ChangedSinceRequest, HistoryChangeKind,
        RecentActivityRequest,
    };

    let stale_rows = |mut rows: Vec<(u64, Vec<String>)>| {
        rows.sort();
        rows.into_iter().map(|(_, ids)| ids).collect::<Vec<_>>()
    };
    let expected: Vec<Vec<String>> = [
        vec!["d:bystander"],
        vec!["d:two-a", "d:two-b"],
        vec!["d:two-a", "d:two-b"],
    ]
    .into_iter()
    .map(|ids| ids.into_iter().map(str::to_owned).collect())
    .collect();

    let activity = get_recent_activity(
        scenario.ledger(),
        &RecentActivityRequest {
            limit: 200,
            ..RecentActivityRequest::default()
        },
    )?
    .data;
    assert!(activity.next_cursor.is_none());
    assert_eq!(
        stale_rows(
            activity
                .items
                .iter()
                .filter(|row| row.change_kind == HistoryChangeKind::StalePremise)
                .map(|row| (row.event_origin, row.decision_ids.clone()))
                .collect()
        ),
        expected
    );

    let changed = get_decisions_changed_since(
        scenario.ledger(),
        &ChangedSinceRequest {
            since_offset: Some(0),
            limit: 200,
            ..ChangedSinceRequest::default()
        },
    )?
    .data;
    assert_eq!(
        stale_rows(
            changed
                .items
                .iter()
                .filter(|row| row.change_kind == HistoryChangeKind::StalePremise)
                .map(|row| (row.event_origin, row.decision_ids.clone()))
                .collect()
        ),
        expected
    );
    Ok(())
}
