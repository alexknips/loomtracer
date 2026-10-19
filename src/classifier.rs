//! Layer-3 background classifier: reads ingest.batch_received events and
//! annotates them with ingest.batch_classified events via Haiku 4.5.
//!
//! The worker is entirely optional: if ANTHROPIC_API_KEY is absent it exits
//! immediately and the rest of the system stays fully correct without it. It reads the SQLite
//! ledger in a folder, so `serve` starts it only on the SQLite backend.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::commands::{CommandContext, Commands};
use crate::events::{
    classified_batch_ids, normalize_question_text, CaptureItem, EventId, EventProvenance,
    EventType, TenantId,
};
use crate::ledger::{EventLedger, SqliteEventLedger};
use crate::projector::{memory::MemoryGraph, rebuild_graph_for_tenant};

const CLASSIFIER_MODEL: &str = "claude-haiku-4-5-20251001";
const CLASSIFIER_MODEL_ENV: &str = "HIVEMIND_CLASSIFIER_MODEL";
pub const SCHEMA_VERSION: &str = "2";
const ACTOR_ID: &str = "agent:hivemind:classifier";
const POLL_INTERVAL: Duration = Duration::from_secs(10);
const HAIKU_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_TOKENS: u32 = 1200;

// Classifier prompt from CAPTURE_CLASSIFIER.md
const CLASSIFIER_PROMPT: &str = r#"You are the HiveMind capture classifier.

HiveMind stores organizational decision memory: durable decisions, evidence,
hypotheses, blockers, and notifications with provenance. It does not store chat
history, task tracking, private scratch notes, or raw tool logs.

Read the batch of recent agent activity. Return only JSON matching the capture
schema. Most batches should return {"captures":[]}.

Capture a decision only when the text shows a chosen path among plausible
alternatives, gives or implies a reason, and passes the decision test: the
choice binds something beyond the edit in front of its author (other people,
other code, a later choice), or undoing it would be costly. Choosing "apple"
over "Apple" in one comment binds nothing and is cheap to undo, so it is not a
decision. Merging tonight despite a failing check binds nothing later but is
costly to undo, so it is. Text that asks for work to be done, or only asks
someone to choose, is a request, not a decision: a request goes to your
tracker, so capture nothing for it.

Capture evidence only when there is an observation with a referent that could
support or refute a later decision, such as a test result, production symptom,
verified external fact, measured latency, or explicit user research finding.

Capture a hypothesis only when the text states a proposition being tested or a
claim that may later be supported or refuted.

Capture a blocker only when progress is materially stopped by a dependency,
permission issue, unavailable service, missing artifact, failing gate, or
unresolved external decision.

Capture a notification only when another actor would need the announcement
after restart, such as handoff state, merge readiness with proof, rejection
state, or completed verification.

Do not capture synthetic test data, fixture/demo content, gc/br plumbing,
branch-name mechanics, routine gate chatter, raw command output, stack traces,
file diffs, TODO lists, status narration, generic plans, or requests for work.
Settle a borderline choice with the decision test above: if it binds nothing and
is cheap to undo, omit it.

Keep titles short. Use 1 to 5 lowercase topic keys. Confidence is your
self-estimate for offline tuning, not authoritative truth.

Write `title` as one plain sentence a stranger understands, and keep
`rationale` readable without the tracker. If a tracker or ticket id (a bead,
Jira, GitHub issue) appears in the source text, leave it out of the title and
rationale — it is not part of the captured content.

RELATIONAL FIELDS — populate only from explicit text, never infer:

CROSS-REFERENCE RULE (applies to evidence_ids, premised_on_ids, supersedes_id,
supports_ids, refutes_ids): each id is valid in exactly two cases — (1) it is
a real id that appears verbatim in the input text (the agent quoted an
existing id such as "decision:..." or "hyp:..."), or (2) it is the exact
`title` you are giving to ANOTHER capture in this same response, copied
verbatim, when this item explicitly references that other one (e.g. a
decision that supersedes another decision you are capturing right now, or
evidence that supports a hypothesis you are capturing right now). Never
invent an id that is neither of these — if the referenced item's real id is
unknown and it is not being captured in this same response, leave the field
null/empty.

expressed_confidence: For decisions only. Extract the decider's own words about
confidence level: "low" (tentative, provisional, lean, unsure), "medium"
(reasonably confident but caveated), "high" (firm, definite, committed). Use
null if the decider expresses no explicit confidence level.

actor_id: The specific actor who proposed/made/reported this item, IF named in
the input. Use their ID if present, otherwise null. Never infer a proposer from
context; only record them when explicitly stated. Naming an actor here says
nothing about who decided: it is not an acceptance.

accepted_by / rejected_by: For decisions. Who DECIDED,
not who spoke. List an actor only when the text shows that actor deciding: a
person stating the choice themselves ("we'll go with X", "I'm going with X"),
or any actor approving, accepting or rejecting a choice ("approved", "yes, do
it" in answer to a proposal). When the text reports someone's call ("Priya
approved X"), list Priya, not whoever reports it. Proposing, recommending,
carrying out, reporting or relaying a choice is not deciding it: an actor who
only suggests X ("I suggest X", "we could use X"), only implements it, only says
it was decided, or passes on a call without saying who made it is not listed,
and a proposal nobody has accepted stays empty however firmly it is worded. An
agent narrating or carrying out its own choice is proposing it, so the decision
stays unaccepted until a person is shown deciding it. A speaker the text does
not name gives no one to list. Never inferred and never taken from actor_id;
list every actor the text shows deciding, never deduplicated to one. Empty array
if none.

evidence_ids: For decisions. Evidence ids (per the cross-reference rule above)
this decision is explicitly based on. Empty array if none.

supersedes_id: For decisions that explicitly replace a prior decision. An id
per the cross-reference rule above. null if none.

premised_on_ids: For decisions. Hypothesis ids (per the cross-reference rule
above) this decision is explicitly premised on. Empty array if none.

supports_ids / refutes_ids: For evidence. Hypothesis ids (per the
cross-reference rule above) this evidence explicitly supports or refutes.
Empty array if none.

blocked_actor_id: For blockers. The actor being blocked, IF named in the input.
decision_id: For blockers. The decision being blocked, IF its ID appears in
the input. Both null if not explicitly stated.

SOURCE TURN AND QUESTION — populate only from explicit text, never infer:

Each turn in the batch starts with a header like `[user turn <id>]`.

source_turn_id: The id from the header of the one turn this capture came from,
copied exactly: for a decision, the turn in which the choice was made or stated.
null when you cannot tell which single turn.

question: For a decision, the question it answers, only when the text states
that question, in the words the asker used. null otherwise, and always null for
every other kind. Never write a question the text does not contain."#;

/// JSON Schema for the classifier's structured output. Shared across both LLM
/// backends (the metered Anthropic API and the fidelity evaluator's Claude
/// Code CLI backend) so their outputs are schema-identical.
pub fn capture_schema() -> serde_json::Value {
    let nullable_string = serde_json::json!({
        "oneOf": [{ "type": "string" }, { "type": "null" }]
    });
    let string_array = serde_json::json!({
        "type": "array",
        "items": { "type": "string" }
    });
    serde_json::json!({
        "type": "object",
        "properties": {
            "captures": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "kind": {
                            "type": "string",
                            "enum": ["decision", "evidence", "hypothesis", "blocker", "notification"]
                        },
                        "title": { "type": "string" },
                        "rationale": { "type": "string" },
                        "topic_keys": string_array.clone(),
                        "evidence_ids": string_array.clone(),
                        "options": {
                            "oneOf": [
                                { "type": "array", "items": { "type": "string" } },
                                { "type": "null" }
                            ]
                        },
                        "chosen_option": nullable_string.clone(),
                        "extraction_confidence": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                        "expressed_confidence": {
                            "oneOf": [
                                { "type": "string", "enum": ["low", "medium", "high"] },
                                { "type": "null" }
                            ]
                        },
                        "supersedes_id": nullable_string.clone(),
                        "premised_on_ids": string_array.clone(),
                        "supports_ids": string_array.clone(),
                        "refutes_ids": string_array.clone(),
                        "actor_id": nullable_string.clone(),
                        "accepted_by": string_array.clone(),
                        "rejected_by": string_array.clone(),
                        "blocked_actor_id": nullable_string.clone(),
                        "decision_id": nullable_string.clone(),
                        "source_turn_id": nullable_string.clone(),
                        "question": nullable_string.clone()
                    },
                    "required": [
                        "kind", "title", "rationale", "topic_keys", "evidence_ids",
                        "options", "chosen_option", "extraction_confidence",
                        "expressed_confidence", "supersedes_id",
                        "premised_on_ids", "supports_ids", "refutes_ids",
                        "actor_id", "accepted_by", "rejected_by",
                        "blocked_actor_id", "decision_id",
                        "source_turn_id", "question"
                    ],
                    "additionalProperties": false
                }
            }
        },
        "required": ["captures"],
        "additionalProperties": false
    })
}

#[derive(Debug, Deserialize)]
struct ClassifierOutput {
    captures: Vec<CaptureItemRaw>,
}

#[derive(Debug, Deserialize)]
struct CaptureItemRaw {
    kind: String,
    title: String,
    rationale: String,
    topic_keys: Vec<String>,
    evidence_ids: Vec<String>,
    options: Option<Vec<String>>,
    chosen_option: Option<String>,
    extraction_confidence: f64,
    expressed_confidence: Option<String>,
    supersedes_id: Option<String>,
    #[serde(default, alias = "assumes_ids")]
    premised_on_ids: Vec<String>,
    #[serde(default)]
    supports_ids: Vec<String>,
    #[serde(default)]
    refutes_ids: Vec<String>,
    actor_id: Option<String>,
    #[serde(default)]
    accepted_by: Vec<String>,
    #[serde(default)]
    rejected_by: Vec<String>,
    blocked_actor_id: Option<String>,
    decision_id: Option<String>,
    #[serde(default)]
    source_turn_id: Option<String>,
    #[serde(default)]
    question: Option<String>,
}

/// Spawn the background classifier task. Returns immediately; the worker runs
/// in the background until the process exits.
pub fn spawn_classifier(hivemind_dir: Arc<PathBuf>, tenant_id: TenantId, api_key: String) {
    tokio::spawn(async move {
        run_classifier_loop(hivemind_dir, tenant_id, api_key).await;
    });
}

async fn run_classifier_loop(hivemind_dir: Arc<PathBuf>, tenant_id: TenantId, api_key: String) {
    info!(target: "hivemind::classifier", "classifier worker started");
    let client = match reqwest::Client::builder().timeout(HAIKU_TIMEOUT).build() {
        Ok(c) => c,
        Err(e) => {
            warn!(target: "hivemind::classifier", "failed to build http client: {e}");
            return;
        }
    };

    let classifier = HaikuClassifier {
        client: &client,
        api_key: &api_key,
        hivemind_dir: &hivemind_dir,
        tenant_id: &tenant_id,
    };
    let mut state = PassState::default();
    loop {
        classify_pending_batches(
            &classifier,
            &hivemind_dir,
            &tenant_id,
            daily_classification_cap(),
            &mut state,
        )
        .await;
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// What the worker remembers from pass to pass, for the current UTC day. A call that failed is
/// not made again until the next day: the batch stays pending in the ledger for that long, and
/// a retry on the next pass would fail the same way and be billed the same.
#[derive(Default)]
struct PassState {
    day: Option<chrono::NaiveDate>,
    /// Received-batch events whose call or write failed today. Each spent a call of the cap.
    failed: std::collections::HashSet<EventId>,
    /// Whether the spent cap has been said today.
    cap_said: bool,
}

impl PassState {
    /// Forgets yesterday's failures and starts the day's cap over.
    fn start_pass(&mut self, today: chrono::NaiveDate) {
        if self.day != Some(today) {
            self.day = Some(today);
            self.failed.clear();
            self.cap_said = false;
        }
    }
}

/// Summary of a pending (unclassified) ingest batch, for `classify-queue list` output.
#[derive(Debug, Clone, Serialize)]
pub struct PendingBatch {
    pub batch_id: String,
    pub submitted_at: Option<DateTime<Utc>>,
    pub actor_id: String,
    pub turn_count: usize,
    /// `session_id` from the originating `IngestBatchReceived` event; lets a
    /// classifier scope `classify-queue list` to one session and submit that
    /// session's batches together (hivemind-zdsh.18).
    pub session_id: String,
    pub agent_tool: String,
    /// Rendered turn text (see [`render_batch_text`]) — what a classifier
    /// actually reads. A caller never needs a second round trip to see what
    /// it is about to classify.
    pub batch_text: String,
}

/// How many events one typed read of the queue's event types asks for. Large enough that the
/// city cell's ~90k received batches take a handful of round trips, not hundreds.
const QUEUE_READ_PAGE: usize = 10_000;

/// The payload keys of a received batch that choosing and grouping batches needs, in the order
/// [`pending_batch_heads`] reads them. The turns, the bulk of the event, are not among them: only
/// the batches a caller is handed are read in full.
const RECEIVED_HEAD_KEYS: &[&str] = &["batch_id", "session_id"];

/// Payload keys of a classification that the queue's scans leave out: the captures are the bulk
/// of the event, and the queue needs only which batches it covers.
const CLASSIFIED_BULK_KEYS: &[&str] = &["captures"];

/// Payload keys of a classification that counting today's classifications leaves out: all of
/// its content, since only its time matters.
const CLASSIFIED_COUNT_BULK_KEYS: &[&str] = &["captures", "batch_ids", "batch_id"];

/// A received batch without its turns: all that choosing, grouping and counting pending batches
/// needs. The turns are read only for the batches a caller is handed.
struct BatchHead {
    event_id: EventId,
    submitted_at: Option<DateTime<Utc>>,
    actor_id: String,
    session_id: String,
}

/// One page of the pending queue, oldest batch first.
#[derive(Debug, Clone, Serialize)]
pub struct PendingBatchPage {
    /// The oldest pending batches that match the filter, at most the limit, with their turn text.
    pub batches: Vec<PendingBatch>,
    /// How many pending batches match the filter, whatever the limit.
    pub pending_total: usize,
    /// True when more batches are pending than `batches` holds: the page is not the whole queue.
    pub truncated: bool,
}

/// One session's pending batches, summarised: what a worker needs to choose which session to
/// classify first, before it fetches that session's batches (`session_id` filter).
#[derive(Debug, Clone, Serialize)]
pub struct PendingSession {
    pub session_id: String,
    pub actor_id: String,
    pub batch_count: usize,
    pub oldest_submitted_at: Option<DateTime<Utc>>,
    pub newest_submitted_at: Option<DateTime<Utc>>,
}

/// The sessions with pending batches, most recently active first.
#[derive(Debug, Clone, Serialize)]
pub struct PendingSessionPage {
    pub sessions: Vec<PendingSession>,
    /// How many sessions have pending batches, whatever the limit.
    pub session_total: usize,
    /// How many batches are pending across all of them.
    pub batch_total: usize,
    /// True when more sessions have pending batches than `sessions` holds.
    pub truncated: bool,
}

/// Reads an optional string field from a raw event payload; absent or
/// non-string values read as the empty string.
fn payload_string(payload: &serde_json::Value, key: &str) -> String {
    payload
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_owned()
}

/// Ids of every batch some classification covers. Reads only the classification events, and of
/// those only the ids.
fn classified_batch_id_set(
    ledger: &impl EventLedger,
    tenant_id: &TenantId,
) -> crate::Result<std::collections::HashSet<String>> {
    let mut classified = std::collections::HashSet::new();
    let mut offset = 0u64;
    loop {
        let events = ledger.read_types_for_tenant(
            tenant_id,
            &[EventType::IngestBatchClassified],
            CLASSIFIED_BULK_KEYS,
            offset,
            QUEUE_READ_PAGE,
        )?;
        for event in &events {
            classified.extend(classified_batch_ids(&event.payload));
        }
        match events.last().and_then(|event| event.event_id) {
            Some(last) if events.len() == QUEUE_READ_PAGE => offset = last,
            _ => break,
        }
    }
    Ok(classified)
}

/// Every received batch that no classification covers, oldest first, as heads. The turn text of
/// the ledger's history is never read: classifications are read without their captures, received
/// batches as their id, time, actor, batch id and session id alone, and the text of a batch only
/// when a caller is handed it. What remains per received batch, classified or not, is one
/// five-field row.
fn pending_batch_heads(
    ledger: &impl EventLedger,
    tenant_id: &TenantId,
) -> crate::Result<Vec<BatchHead>> {
    let classified = classified_batch_id_set(ledger, tenant_id)?;
    let mut pending = Vec::new();
    let mut offset = 0u64;
    loop {
        let rows = ledger.read_fields_for_tenant(
            tenant_id,
            EventType::IngestBatchReceived,
            RECEIVED_HEAD_KEYS,
            offset,
            QUEUE_READ_PAGE,
        )?;
        for row in &rows {
            let [Some(batch_id), session_id] = row.fields.as_slice() else {
                continue;
            };
            if classified.contains(batch_id) {
                continue;
            }
            pending.push(BatchHead {
                event_id: row.event_id,
                submitted_at: row.ts,
                actor_id: row.actor_id.clone(), // ubs:ignore: field copy from a borrowed row
                session_id: session_id.clone().unwrap_or_default(), // ubs:ignore: field copy from a borrowed row
            });
        }
        match rows.last() {
            Some(last) if rows.len() == QUEUE_READ_PAGE => offset = last.event_id,
            _ => break,
        }
    }
    Ok(pending)
}

/// The pending batch a received-batch event describes, with its rendered turn text. `None` for
/// an event with no `batch_id`, which the queue never lists.
fn pending_batch_from_event(event: &crate::events::Event) -> Option<PendingBatch> {
    let batch_id = event.payload.get("batch_id").and_then(|v| v.as_str())?;
    let turn_count = event
        .payload
        .get("turns")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    Some(PendingBatch {
        batch_id: batch_id.to_owned(),
        submitted_at: event.ts,
        actor_id: event.actor_id.clone(),
        turn_count,
        session_id: payload_string(&event.payload, "session_id"),
        agent_tool: payload_string(&event.payload, "agent_tool"),
        batch_text: render_batch_text(event),
    })
}

/// Lists batches received but not yet classified (no `IngestBatchClassified`
/// event covers their batch id), for `classify-queue list`: the oldest `limit`
/// of them, optionally only one session's. Generic over any [`EventLedger`]
/// backend so the HTTP API (SQLite dev mode or Postgres) and the local CLI
/// path share one implementation.
///
/// Which batches are pending is decided from heads alone; the turn text of
/// only the returned batches is read, in one query by event id.
pub fn list_pending_batches_for_ledger(
    ledger: &impl EventLedger,
    tenant_id: &TenantId,
    session_id: Option<&str>,
    limit: usize,
) -> crate::Result<PendingBatchPage> {
    let mut heads = pending_batch_heads(ledger, tenant_id)?;
    if let Some(session_id) = session_id {
        heads.retain(|head| head.session_id == session_id);
    }
    let pending_total = heads.len();
    heads.truncate(limit);
    let truncated = pending_total > heads.len();

    let event_ids: Vec<EventId> = heads.iter().map(|head| head.event_id).collect();
    let batches = ledger
        .read_ids_for_tenant(tenant_id, &event_ids)?
        .iter()
        .filter_map(pending_batch_from_event)
        .collect();

    Ok(PendingBatchPage {
        batches,
        pending_total,
        truncated,
    })
}

/// Summarises the pending queue by session: one row per (session, actor) with the batch count and
/// the oldest and newest submission time, most recently active first. Reads heads only, so it
/// costs the same however much turn text is waiting.
pub fn list_pending_sessions_for_ledger(
    ledger: &impl EventLedger,
    tenant_id: &TenantId,
    limit: usize,
) -> crate::Result<PendingSessionPage> {
    let heads = pending_batch_heads(ledger, tenant_id)?;
    let batch_total = heads.len();

    let mut position_of: std::collections::HashMap<(String, String), usize> =
        std::collections::HashMap::new();
    let mut sessions: Vec<PendingSession> = Vec::new();
    for head in heads {
        let position = *position_of
            .entry((head.session_id, head.actor_id))
            .or_insert_with_key(|(session_id, actor_id)| {
                sessions.push(PendingSession {
                    session_id: session_id.clone(), // ubs:ignore: once per session, not per batch
                    actor_id: actor_id.clone(),     // ubs:ignore: once per session, not per batch
                    batch_count: 0,
                    oldest_submitted_at: None,
                    newest_submitted_at: None,
                });
                sessions.len() - 1
            });
        if let Some(session) = sessions.get_mut(position) {
            session.batch_count += 1;
            session.oldest_submitted_at = session
                .oldest_submitted_at
                .into_iter()
                .chain(head.submitted_at)
                .min();
            session.newest_submitted_at = session
                .newest_submitted_at
                .into_iter()
                .chain(head.submitted_at)
                .max();
        }
    }

    sessions.sort_by(|a, b| {
        b.newest_submitted_at
            .cmp(&a.newest_submitted_at)
            .then_with(|| a.session_id.cmp(&b.session_id))
            .then_with(|| a.actor_id.cmp(&b.actor_id))
    });
    let session_total = sessions.len();
    sessions.truncate(limit);
    let truncated = session_total > sessions.len();

    Ok(PendingSessionPage {
        sessions,
        session_total,
        batch_total,
        truncated,
    })
}

/// What `classify-queue list` answers, on a local ledger, over HTTP and over MCP alike: one page of
/// the pending queue and today's classification budget. `pending_total` is the whole queue's depth
/// whatever the limit, so `list --limit 1` is the cheap way to count it.
#[derive(Debug, Clone)]
pub struct PendingQueue {
    pub page: PendingBatchPage,
    pub budget: DailyCapStatus,
}

impl PendingQueue {
    /// The JSON object every `classify-queue list` surface returns.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "batches": self.page.batches,
            "pending_total": self.page.pending_total,
            "truncated": self.page.truncated,
            "budget": self.budget,
        })
    }
}

/// The pending queue for `classify-queue list`: [`list_pending_batches_for_ledger`] plus the budget.
pub fn pending_queue_for_ledger(
    ledger: &impl EventLedger,
    tenant_id: &TenantId,
    session_id: Option<&str>,
    limit: usize,
) -> crate::Result<PendingQueue> {
    let page = list_pending_batches_for_ledger(ledger, tenant_id, session_id, limit)?;
    let budget = daily_cap_status(ledger, tenant_id)?;
    Ok(PendingQueue { page, budget })
}

/// Default daily cap on classification calls (ledger events, not batches
/// covered) per tenant, city-wide. Alex's chosen cadence (bead comment,
/// 2026-09-23): classify each session in one model call at session end, not
/// per batch or hourly, capped at ~150 calls/day; over the cap, batches wait
/// for the next day rather than being dropped.
pub const DEFAULT_DAILY_CLASSIFICATION_CAP: usize = 150;

/// Resolves the active daily cap: `HIVEMIND_CLASSIFY_DAILY_CAP` when set to a
/// valid positive integer, otherwise [`DEFAULT_DAILY_CLASSIFICATION_CAP`].
/// The override exists so tests can exercise cap enforcement without writing
/// 150 real events.
pub fn daily_classification_cap() -> usize {
    std::env::var("HIVEMIND_CLASSIFY_DAILY_CAP")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_DAILY_CLASSIFICATION_CAP)
}

/// Classification budget for the current UTC day, for a tenant.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct DailyCapStatus {
    /// Number of `IngestBatchClassified` events already recorded today
    /// (UTC) — one per classification call, regardless of how many batches
    /// that call covered.
    pub classified_today: usize,
    pub cap: usize,
    pub remaining: usize,
}

/// Counts `IngestBatchClassified` events recorded for `tenant_id` since the
/// start of the current UTC day and returns the resulting budget. Reads the
/// classification events alone, and only their times.
pub fn daily_cap_status(
    ledger: &impl EventLedger,
    tenant_id: &TenantId,
) -> crate::Result<DailyCapStatus> {
    daily_cap_status_under(ledger, tenant_id, daily_classification_cap())
}

/// [`daily_cap_status`] under a cap the caller names, so the worker reads the environment once per
/// pass and a test needs no process-wide variable.
fn daily_cap_status_under(
    ledger: &impl EventLedger,
    tenant_id: &TenantId,
    cap: usize,
) -> crate::Result<DailyCapStatus> {
    let today = Utc::now().date_naive();
    let mut offset = 0u64;
    let mut classified_today = 0usize;

    loop {
        let events = ledger.read_types_for_tenant(
            tenant_id,
            &[EventType::IngestBatchClassified],
            CLASSIFIED_COUNT_BULK_KEYS,
            offset,
            QUEUE_READ_PAGE,
        )?;
        classified_today += events
            .iter()
            .filter(|event| event.ts.is_some_and(|ts| ts.date_naive() >= today))
            .count();
        match events.last().and_then(|event| event.event_id) {
            Some(last) if events.len() == QUEUE_READ_PAGE => offset = last,
            _ => break,
        }
    }

    Ok(DailyCapStatus {
        classified_today,
        cap,
        remaining: cap.saturating_sub(classified_today),
    })
}

struct BatchInfo {
    event_id: u64,
    batch_id: String,
    batch_text: String,
    /// The `turn_id` of every turn in the batch: the ids a capture's `source_turn_id` may name.
    turn_ids: Vec<String>,
}

/// A batch's captures as the model gave them, ready to be written, and the model that gave them.
struct ClassifiedBatch {
    model: String,
    captures: Vec<CaptureItem>,
}

/// The model-facing half of a pass: what a batch's text comes to. A trait so the pass's own
/// bookkeeping, the cap and the failed batches, is tested without a network.
trait BatchClassifier {
    async fn classify(
        &self,
        batch: &BatchInfo,
    ) -> Result<ClassifiedBatch, crate::anthropic::BoxError>;
}

/// [`BatchClassifier`] over the metered Anthropic API.
struct HaikuClassifier<'a> {
    client: &'a reqwest::Client,
    api_key: &'a str,
    hivemind_dir: &'a PathBuf,
    tenant_id: &'a TenantId,
}

impl BatchClassifier for HaikuClassifier<'_> {
    async fn classify(
        &self,
        batch: &BatchInfo,
    ) -> Result<ClassifiedBatch, crate::anthropic::BoxError> {
        let (output, model) = call_haiku(self.client, self.api_key, &batch.batch_text).await?;
        let mut captures: Vec<CaptureItem> = output
            .captures
            .into_iter()
            .map(|r| CaptureItem {
                kind: r.kind,
                title: r.title,
                rationale: r.rationale,
                topic_keys: r.topic_keys,
                evidence_ids: r.evidence_ids,
                options: r.options,
                chosen_option: r.chosen_option,
                extraction_confidence: r.extraction_confidence,
                expressed_confidence: r.expressed_confidence,
                supersedes_id: r.supersedes_id,
                premised_on_ids: r.premised_on_ids,
                supports_ids: r.supports_ids,
                refutes_ids: r.refutes_ids,
                actor_id: r.actor_id,
                accepted_by: r.accepted_by,
                rejected_by: r.rejected_by,
                blocked_actor_id: r.blocked_actor_id,
                decision_id: r.decision_id,
                participants: Vec::new(),
                restates_id: None,
                source_turn_id: r.source_turn_id,
                source_ts: None,
                question: r.question,
                session_initiator: None,
            })
            .collect();

        conform_to_batch(&mut captures, &batch.turn_ids);
        mark_restatements_best_effort(
            self.client,
            self.api_key,
            &model,
            self.hivemind_dir,
            self.tenant_id,
            &mut captures,
        )
        .await;
        Ok(ClassifiedBatch { model, captures })
    }
}

/// One pass of Worker B: classifies the batches still pending, oldest first, one call each, while
/// the day's cap lasts. The cap counts the classifications already recorded today, by any worker,
/// and the calls this worker has seen fail. A call or a write that fails is logged once and the
/// batch is left for tomorrow (see [`PassState`]).
async fn classify_pending_batches(
    classifier: &impl BatchClassifier,
    hivemind_dir: &PathBuf,
    tenant_id: &TenantId,
    cap: usize,
    state: &mut PassState,
) {
    state.start_pass(Utc::now().date_naive());
    let Some(budget) = calls_left(hivemind_dir, tenant_id, cap, state) else {
        return;
    };
    let batches = match pending_batches(hivemind_dir, tenant_id, &state.failed, budget) {
        Ok(b) => b,
        Err(e) => {
            warn!(target: "hivemind::classifier", "ledger scan failed: {e}");
            return;
        }
    };

    for batch in batches {
        // Another worker may have spent the cap since the pass began.
        if calls_left(hivemind_dir, tenant_id, cap, state).is_none() {
            return;
        }
        debug!(target: "hivemind::classifier", batch_id = %batch.batch_id, "classifying batch");

        let outcome = match classifier.classify(&batch).await {
            Ok(classified) => write_classification(
                hivemind_dir,
                tenant_id,
                &batch.batch_id,
                &classified.model,
                classified.captures,
                Some(batch.event_id),
            )
            .map_err(|e| format!("write failed: {e}")),
            Err(e) => Err(format!("haiku call failed: {e}")),
        };
        if let Err(reason) = outcome {
            warn!(
                target: "hivemind::classifier",
                batch_id = %batch.batch_id,
                "{reason}; the batch is not sent again today"
            );
            state.failed.insert(batch.event_id);
        }
    }
}

/// How many more calls today's cap allows, or `None` when the cap is spent (said once a day) or
/// the ledger could not be read.
fn calls_left(
    hivemind_dir: &PathBuf,
    tenant_id: &TenantId,
    cap: usize,
    state: &mut PassState,
) -> Option<usize> {
    let status = SqliteEventLedger::open(hivemind_dir)
        .and_then(|ledger| daily_cap_status_under(&ledger, tenant_id, cap));
    let left = match status {
        Ok(status) => status.remaining.saturating_sub(state.failed.len()),
        Err(e) => {
            warn!(target: "hivemind::classifier", "daily cap unreadable: {e}");
            return None;
        }
    };
    if left == 0 {
        if !state.cap_said {
            info!(
                target: "hivemind::classifier",
                cap,
                "daily classification cap spent; batches wait for the next UTC day"
            );
            state.cap_said = true;
        }
        return None;
    }
    Some(left)
}

/// Layer 3's restatement judgement (hivemind-83cj): each decision capture is compared with the
/// closest recorded decisions and, when the judge says it states one again, carries that
/// decision's id in `restates_id`. Best effort: any failure leaves the captures as extracted.
async fn mark_restatements_best_effort(
    client: &reqwest::Client,
    api_key: &str,
    model: &str,
    hivemind_dir: &PathBuf,
    tenant_id: &TenantId,
    captures: &mut [CaptureItem],
) {
    if !captures.iter().any(|capture| capture.kind == "decision") {
        return;
    }
    let graph = match recorded_decisions_graph(hivemind_dir, tenant_id) {
        Ok(graph) => graph,
        Err(e) => {
            warn!(target: "hivemind::classifier", "restatement check skipped: {e}");
            return;
        }
    };
    crate::restatement::mark_restatements(client, api_key, model, &graph, captures).await;
}

/// The recorded decisions as a graph, for the restatement judgement. The ledger is closed again
/// before the model is asked anything.
fn recorded_decisions_graph(
    hivemind_dir: &PathBuf,
    tenant_id: &TenantId,
) -> crate::Result<MemoryGraph> {
    let ledger = SqliteEventLedger::open(hivemind_dir)?;
    let graph = MemoryGraph::default();
    rebuild_graph_for_tenant(&ledger, tenant_id, &graph)?;
    Ok(graph)
}

/// The oldest `limit` received batches that no classification covers and no call has failed on
/// today, with their turns. Which batches are pending is decided from heads, as the queue listing
/// does; the turns of only the chosen few are read, in one query by event id.
fn pending_batches(
    hivemind_dir: &PathBuf,
    tenant_id: &TenantId,
    failed: &std::collections::HashSet<EventId>,
    limit: usize,
) -> crate::Result<Vec<BatchInfo>> {
    let ledger = SqliteEventLedger::open(hivemind_dir)?;
    let event_ids: Vec<EventId> = pending_batch_heads(&ledger, tenant_id)?
        .into_iter()
        .map(|head| head.event_id)
        .filter(|event_id| !failed.contains(event_id))
        .take(limit)
        .collect();
    Ok(ledger
        .read_ids_for_tenant(tenant_id, &event_ids)?
        .iter()
        .filter_map(batch_info_from_event)
        .collect())
}

/// The batch a received-batch event describes. `None` for an event with no `batch_id`.
fn batch_info_from_event(event: &crate::events::Event) -> Option<BatchInfo> {
    let event_id = event.event_id?;
    let batch_id = event.payload.get("batch_id").and_then(|v| v.as_str())?;
    let batch_text = render_batch_text(event);
    if batch_text.is_empty() {
        debug!(
            target: "hivemind::classifier",
            "batch {batch_id} has no renderable turns; will classify empty text"
        );
    }
    Some(BatchInfo {
        event_id,
        batch_id: batch_id.to_owned(),
        batch_text,
        turn_ids: turn_ids(event),
    })
}

/// The batch's turns as the classifier reads them: one `[<role> turn <turn_id>] <text>` per
/// turn (`[<role>] <text>` for a turn with no id). The turn id is what a capture's
/// `source_turn_id` names; the turn's time is not shown, because the write path reads it from
/// the received turn rather than from the model.
fn render_batch_text(event: &crate::events::Event) -> String {
    let turns = event
        .payload
        .get("turns")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut out = String::new();
    for turn in &turns {
        let role = turn
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let turn_id = turn.get("turn_id").and_then(|v| v.as_str()).unwrap_or("");
        let text = turn.get("text").and_then(|v| v.as_str()).unwrap_or("");
        let truncated = turn
            .get("truncated")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let _ = write!(out, "[{role}");
        if !turn_id.is_empty() {
            let _ = write!(out, " turn {turn_id}");
        }
        let marker = if truncated { " [TRUNCATED]" } else { "" };
        let _ = writeln!(out, "] {text}{marker}");
    }
    out
}

/// The `turn_id` of every turn of a received batch.
fn turn_ids(event: &crate::events::Event) -> Vec<String> {
    event
        .payload
        .get("turns")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|turn| turn.get("turn_id").and_then(|v| v.as_str()))
        .map(str::to_owned)
        .collect()
}

/// Holds the model to the contract the write path enforces, so a slip of the model's costs one
/// field instead of the whole classification: a `source_turn_id` that names no turn of the batch
/// is dropped (nothing is guessed from it), as is a `question` on a kind that takes none or with
/// no words in it.
fn conform_to_batch(captures: &mut [CaptureItem], turn_ids: &[String]) {
    for capture in captures {
        if capture
            .source_turn_id
            .as_ref()
            .is_some_and(|id| !turn_ids.contains(id))
        {
            capture.source_turn_id = None;
        }
        let takes_question = capture.kind == "decision";
        if capture
            .question
            .as_ref()
            .is_some_and(|q| !takes_question || normalize_question_text(q).is_empty())
        {
            capture.question = None;
        }
    }
}

/// Resolve the classifier model id given an already-read `HIVEMIND_CLASSIFIER_MODEL`
/// value: the override if present, otherwise the pinned default.
fn resolve_classifier_model(env_override: Option<String>) -> String {
    env_override.unwrap_or_else(|| CLASSIFIER_MODEL.to_owned())
}

/// Model id actually used for classification: `HIVEMIND_CLASSIFIER_MODEL` if
/// set to a non-empty value, otherwise the pinned default. Callers that both
/// invoke the API and record provenance must call this once and reuse the
/// result so the two agree even if the environment changes between calls.
fn effective_classifier_model() -> String {
    resolve_classifier_model(crate::identity::env_value(CLASSIFIER_MODEL_ENV))
}

/// Public accessor for [`effective_classifier_model`], for callers (the
/// fidelity evaluator) that need to record which model `classify_text` used
/// without duplicating the `HIVEMIND_CLASSIFIER_MODEL` resolution logic.
pub fn resolved_model() -> String {
    effective_classifier_model()
}

/// Compose the full classifier prompt for a given input/batch text. Shared by
/// both LLM backends so the prompt never drifts between them.
pub fn build_prompt(input_text: &str) -> String {
    format!("{CLASSIFIER_PROMPT}\n\n---BATCH---\n{input_text}")
}

async fn call_haiku(
    client: &reqwest::Client,
    api_key: &str,
    batch_text: &str,
) -> Result<(ClassifierOutput, String), crate::anthropic::BoxError> {
    let model = effective_classifier_model();
    let user_content = build_prompt(batch_text);
    let output = crate::anthropic::call_json_schema(
        client,
        api_key,
        &model,
        MAX_TOKENS,
        user_content,
        capture_schema(),
    )
    .await?;
    Ok((output, model))
}

/// Map raw schema-shaped output into the shared `CaptureItem` type. Who the session was held
/// with is not the classifier's to say: the projector reads it from the received batches.
fn raw_captures_to_items(captures: Vec<CaptureItemRaw>) -> Vec<CaptureItem> {
    captures
        .into_iter()
        .map(|r| CaptureItem {
            kind: r.kind,
            title: r.title,
            rationale: r.rationale,
            topic_keys: r.topic_keys,
            evidence_ids: r.evidence_ids,
            options: r.options,
            chosen_option: r.chosen_option,
            extraction_confidence: r.extraction_confidence,
            expressed_confidence: r.expressed_confidence,
            supersedes_id: r.supersedes_id,
            premised_on_ids: r.premised_on_ids,
            supports_ids: r.supports_ids,
            refutes_ids: r.refutes_ids,
            actor_id: r.actor_id,
            accepted_by: r.accepted_by,
            rejected_by: r.rejected_by,
            blocked_actor_id: r.blocked_actor_id,
            decision_id: r.decision_id,
            participants: vec![],
            restates_id: None,
            source_turn_id: None,
            source_ts: None,
            question: r.question,
            session_initiator: None,
        })
        .collect()
}

/// Parse a classifier response body — raw JSON text constrained to
/// [`capture_schema`], as returned by either LLM backend — into CaptureItems.
/// Shared so the metered Anthropic API and the fidelity evaluator's Claude
/// Code CLI backend produce identical CaptureItem shapes from identical text.
pub fn parse_capture_response(text: &str) -> Result<Vec<CaptureItem>, serde_json::Error> {
    let output: ClassifierOutput = serde_json::from_str(text)?;
    Ok(raw_captures_to_items(output.captures))
}

fn write_classification(
    hivemind_dir: &PathBuf,
    tenant_id: &TenantId,
    batch_id: &str,
    model: &str,
    captures: Vec<CaptureItem>,
    causation_event_id: Option<u64>,
) -> crate::Result<()> {
    let ledger = SqliteEventLedger::open(hivemind_dir)?;
    let commands = Commands::new_with_context(
        &ledger,
        CommandContext::new(tenant_id.clone(), EventProvenance::agent(ACTOR_ID)),
    );
    let batch_ids = [batch_id.to_owned()];
    commands.record_ingest_batch_classified(
        ACTOR_ID,
        &batch_ids,
        model,
        SCHEMA_VERSION,
        captures,
        causation_event_id,
    )?;
    Ok(())
}

/// Classify a single free-text input and return the raw capture items without
/// touching the ledger. Used by the fidelity evaluator to test the classifier
/// against the hand-authored gold corpus.
pub async fn classify_text(
    client: &reqwest::Client,
    api_key: &str,
    input: &str,
) -> Result<Vec<crate::events::CaptureItem>, Box<dyn std::error::Error + Send + Sync>> {
    let (output, _model) = call_haiku(client, api_key, input).await?;
    Ok(raw_captures_to_items(output.captures))
}

/// The key the Layer-3 workers run on: `ANTHROPIC_API_KEY`, unless it is unset or blank (compose
/// passes an empty value through when the operator set none).
pub(crate) fn anthropic_api_key() -> Option<String> {
    std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .filter(|key| !key.trim().is_empty())
}

/// Try to read the API key and spawn the worker. Logs a warning and returns
/// `None` if the key is absent.
pub fn try_spawn(hivemind_dir: Arc<PathBuf>, tenant_id: TenantId) -> Option<()> {
    match anthropic_api_key() {
        Some(key) => {
            spawn_classifier(hivemind_dir, tenant_id, key);
            Some(())
        }
        None => {
            warn!(
                target: "hivemind::classifier",
                "ANTHROPIC_API_KEY not set — Layer-3 classifier disabled"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests;
