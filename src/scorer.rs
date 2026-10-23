//! Layer-3 background scorer: assesses decisions — both classifier-extracted captures
//! (`ingest.batch_classified`) and directly captured/proposed decisions — with a model's
//! judgment of the seven quality dimensions (docs/DECISION_SCORING.md). Each assessment is a
//! `decision.scored` event of schema version 2 (`DecisionAssessedPayload`): every dimension is
//! either assessed (a level, an explanation and, at `partial` or `solid`, a verbatim quote from
//! the decision's own recorded text) or not assessed, with why. Never a placeholder.
//!
//! The worker is entirely optional: if ANTHROPIC_API_KEY is absent it exits
//! immediately and the rest of the system — including the quality profile's
//! deterministic floors — stays fully correct without it. It reads the SQLite ledger in a
//! folder, so `serve` starts it only on the SQLite backend.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Deserialize;
use tracing::{debug, info, warn};

use crate::commands::{
    classified_capture_fields, proposed_decision_fields, CommandContext, Commands,
};
use crate::events::{
    DecisionAssessedPayload, EventId, EventProvenance, EventType, ImportanceFactors,
    ModelDimensions, TenantId, DECISION_ASSESSED_SCHEMA_VERSION,
};
use crate::floor::{BindsAnswer, UndoCostAnswer};
use crate::ledger::{EventLedger, SqliteEventLedger};

const SCORER_MODEL: &str = "claude-haiku-4-5-20251001";
const SCORER_MODEL_ENV: &str = "LOOMTRACER_SCORER_MODEL";
/// Names the wording of [`ASSESSOR_PROMPT`] on every assessment it produces, so an assessment
/// traces back to it. `assessment-v2` added the decision floor's two recorded answers as input
/// facts (hivemind-bbnw.11). `assessment-v3` stopped offering a classified capture's confidence
/// and topic keys as quotable text and gave each option its own line (hivemind-m88ju).
const ASSESSOR_PROMPT_VERSION: &str = "assessment-v3";
const ACTOR_ID: &str = "agent:loomtracer:scorer";
const POLL_INTERVAL: Duration = Duration::from_secs(30);
/// How many times a decision's assessment may fail, through a failed call or an answer the
/// write path refused, before the worker stops sending it to the model. Each attempt is a
/// metered call, and a refused answer leaves the decision unassessed, so without a bound the
/// same decision is paid for on every pass (hivemind-m88ju).
const MAX_ATTEMPTS: u32 = 3;
/// How long after a failed attempt the same decision is left alone before it is tried again.
const RETRY_BACKOFF: Duration = Duration::from_secs(300);
const API_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_TOKENS: u32 = 3000;

const ASSESSOR_PROMPT: &str = r#"You are the Loomtracer decision quality assessor.

Loomtracer records organizational decisions. Assess one decision's seven quality
dimensions, EX ANTE — only from what was knowable at decision time. Never judge
a dimension by how the decision turned out.

The seven dimensions:
  framing          — Was the right problem/question framed?
  alternatives     — Were genuine alternatives generated and considered?
  information      — Was relevant information gathered and used?
  reasoning        — Is the inference from information to choice sound?
  values_tradeoffs — Were values and tradeoffs made explicit and weighed?
  bias_exposure    — Exposure to cognitive distortions (anchoring, confirmation,
                     sunk-cost, framing, motivated reasoning), other than
                     confidence miscalibration.
  calibration      — Does expressed confidence match the evidence?

Framing and values_tradeoffs have no mechanical floor beyond "a question was
recorded" — assess both. Enrich any of the other five only where the decision's
own text gives you real grounds; leave the rest not_assessed.

Answer each dimension one of two ways, never a placeholder:
  {"status": "assessed", "level": "none"|"partial"|"solid", "explanation": "...", "quote": "..."}
  {"status": "not_assessed", "reason": "..."}

`level` is ordinal (solid > partial > none), never a number. `explanation` is
always required. `quote` is REQUIRED at level "partial" or "solid": copy a
passage VERBATIM from a single one of the decision's own recorded fields below
— never combine two fields, paraphrase, or invent one, and never include the
field's label. `quote` is optional at level "none" (an absence usually cannot
be quoted). A `not_assessed` answer's `reason` says what is missing — the
honest answer, never a guess dressed up as a score.

The decision's recorded text follows, one field per labeled line. Quote only
from within a single field's value, not its label.

Lines labeled "Recorded answer" are not part of that text. They are the
capturer's own answers to what the decision binds beyond the edit in front of
them and what undoing it would cost. They are input facts: where an explanation
speaks to how far the decision reaches or how hard it is to undo, cite the
answer as its basis rather than guessing, and never quote from these lines.
When they are absent the capturer was not asked; do not assume either answer.

Lines labeled "Recorded context" are not part of that text either: they are the
confidence the capturer expressed and the topic keys they filed the decision
under. Use the expressed confidence as the input fact for calibration, and never
quote from these lines.

Return only JSON matching the schema."#;

fn dimension_answer_schema() -> serde_json::Value {
    serde_json::json!({
        "oneOf": [
            {
                "type": "object",
                "additionalProperties": false,
                "required": ["status", "level", "explanation"],
                "properties": {
                    "status": { "const": "assessed" },
                    "level": { "type": "string", "enum": ["none", "partial", "solid"] },
                    "explanation": { "type": "string", "minLength": 1 },
                    "quote": {
                        "oneOf": [
                            { "type": "string", "minLength": 1 },
                            { "type": "null" }
                        ]
                    }
                }
            },
            {
                "type": "object",
                "additionalProperties": false,
                "required": ["status", "reason"],
                "properties": {
                    "status": { "const": "not_assessed" },
                    "reason": { "type": "string", "minLength": 1 }
                }
            }
        ]
    })
}

fn assessment_schema() -> serde_json::Value {
    let dim = dimension_answer_schema();
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["dimensions"],
        "properties": {
            "dimensions": {
                "type": "object",
                "additionalProperties": false,
                "required": [
                    "framing", "alternatives", "information", "reasoning",
                    "values_tradeoffs", "bias_exposure", "calibration"
                ],
                "properties": {
                    "framing": dim.clone(),
                    "alternatives": dim.clone(),
                    "information": dim.clone(),
                    "reasoning": dim.clone(),
                    "values_tradeoffs": dim.clone(),
                    "bias_exposure": dim.clone(),
                    "calibration": dim.clone()
                }
            }
        }
    })
}

/// Raw structured output from a model's assessment call, before it becomes a
/// `DecisionAssessedPayload`. Deserialized both from the server's Haiku call
/// (`call_assessor`) and from a plugin/edge-submitted scores file (`emit decision.scored`) —
/// the same schema, two producers, going through the same [`build_assessed_payload`] and the
/// same write-path validator (`Commands::record_decision_assessed`).
#[derive(Debug, Deserialize)]
pub(crate) struct AssessmentOutput {
    pub(crate) dimensions: ModelDimensions,
    /// A separate axis from the seven dimensions (docs/DECISION_SCORING.md). Optional, not
    /// elicited by the server's own prompt (`ASSESSOR_PROMPT` asks for `dimensions` only), but
    /// a keyless caller may supply it.
    #[serde(default)]
    pub(crate) importance: Option<ImportanceFactors>,
}

/// A decision, however it was captured, that has no version-2 `decision.scored` (model
/// assessment) event yet.
struct PendingAssessment {
    /// The decision's id: either a classifier capture node (`capture:{event}:{idx}`) or a
    /// proposed decision's own id.
    decision_id: String,
    /// The event that established this decision, for causal linkage on the resulting
    /// `decision.scored` event.
    causation_event_id: EventId,
    /// Text description sent to the model.
    decision_text: String,
}

/// Spawn the background scorer task. Returns immediately; the worker runs
/// in the background until the process exits.
pub fn spawn_scorer(loomtracer_dir: Arc<PathBuf>, tenant_id: TenantId, api_key: String) {
    tokio::spawn(async move {
        run_scorer_loop(loomtracer_dir, tenant_id, api_key).await;
    });
}

async fn run_scorer_loop(loomtracer_dir: Arc<PathBuf>, tenant_id: TenantId, api_key: String) {
    info!(target: "loomtracer::scorer", "scorer worker started");
    let client = match reqwest::Client::builder().timeout(API_TIMEOUT).build() {
        Ok(c) => c,
        Err(e) => {
            warn!(target: "loomtracer::scorer", "failed to build http client: {e}");
            return;
        }
    };

    let mut attempts = AttemptLog::default();
    loop {
        assess_pending_decisions(
            &client,
            &loomtracer_dir,
            &tenant_id,
            &api_key,
            &mut attempts,
        )
        .await;
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// What the worker has paid for without result, per decision: the failed attempts and when the
/// last one ended. Held in memory for the life of the process, so a restart starts every
/// decision afresh.
#[derive(Default)]
struct AttemptLog {
    failed: HashMap<String, FailedAttempts>,
}

struct FailedAttempts {
    count: u32,
    last_at: Instant,
}

/// Whether a pending decision may be sent to the model on this pass.
#[derive(Debug, PartialEq, Eq)]
enum Standing {
    Ready,
    /// A recent attempt failed; the next one is allowed once the backoff has passed.
    Backoff,
    /// [`MAX_ATTEMPTS`] attempts failed; the worker sends it no more.
    GivenUp,
}

/// What a failed attempt leaves the decision with.
#[derive(Debug, PartialEq, Eq)]
enum AfterFailure {
    /// `attempt` of [`MAX_ATTEMPTS`] failed; the next one waits out the backoff.
    Retry {
        attempt: u32,
    },
    GaveUp {
        attempts: u32,
    },
}

impl AttemptLog {
    fn standing(&self, decision_id: &str, now: Instant) -> Standing {
        match self.failed.get(decision_id) {
            None => Standing::Ready,
            Some(failed) if failed.count >= MAX_ATTEMPTS => Standing::GivenUp,
            Some(failed) if now.saturating_duration_since(failed.last_at) < RETRY_BACKOFF => {
                Standing::Backoff
            }
            Some(_) => Standing::Ready,
        }
    }

    fn record_failure(&mut self, decision_id: &str, now: Instant) -> AfterFailure {
        let failed = self
            .failed
            .entry(decision_id.to_owned())
            .or_insert(FailedAttempts {
                count: 0,
                last_at: now,
            });
        failed.count += 1;
        failed.last_at = now;
        if failed.count >= MAX_ATTEMPTS {
            AfterFailure::GaveUp {
                attempts: failed.count,
            }
        } else {
            AfterFailure::Retry {
                attempt: failed.count,
            }
        }
    }

    /// Say what a failed attempt leaves the decision with, so a decision that is no longer sent
    /// is never silent about it.
    fn log_failure(&mut self, decision_id: &str, what: &str, error: &dyn std::fmt::Display) {
        match self.record_failure(decision_id, Instant::now()) {
            AfterFailure::Retry { attempt } => warn!(
                target: "loomtracer::scorer",
                decision_id = %decision_id,
                "{what}: {error}; attempt {attempt} of {MAX_ATTEMPTS}, retrying in {}s",
                RETRY_BACKOFF.as_secs()
            ),
            AfterFailure::GaveUp { attempts } => warn!(
                target: "loomtracer::scorer",
                decision_id = %decision_id,
                "{what}: {error}; {attempts} attempts failed, giving up on this decision until the server restarts"
            ),
        }
    }
}

async fn assess_pending_decisions(
    client: &reqwest::Client,
    loomtracer_dir: &PathBuf,
    tenant_id: &TenantId,
    api_key: &str,
    attempts: &mut AttemptLog,
) {
    let pending = match find_unscored_decisions(loomtracer_dir, tenant_id) {
        Ok(p) => p,
        Err(e) => {
            warn!(target: "loomtracer::scorer", "ledger scan failed: {e}");
            return;
        }
    };

    for decision in pending {
        match attempts.standing(&decision.decision_id, Instant::now()) {
            Standing::Ready => {}
            standing => {
                debug!(
                    target: "loomtracer::scorer",
                    decision_id = %decision.decision_id,
                    ?standing,
                    "not assessing decision"
                );
                continue;
            }
        }
        debug!(target: "loomtracer::scorer", decision_id = %decision.decision_id, "assessing decision");

        match call_assessor(client, api_key, &decision.decision_text).await {
            Ok((output, model)) => {
                if let Err(e) = write_assessment(
                    loomtracer_dir,
                    tenant_id,
                    &decision.decision_id,
                    &model,
                    output,
                    Some(decision.causation_event_id),
                ) {
                    attempts.log_failure(&decision.decision_id, "write failed", &e);
                }
            }
            Err(e) => {
                attempts.log_failure(&decision.decision_id, "api call failed", &e);
            }
        }
    }
}

fn find_unscored_decisions(
    loomtracer_dir: &PathBuf,
    tenant_id: &TenantId,
) -> crate::Result<Vec<PendingAssessment>> {
    let ledger = SqliteEventLedger::open(loomtracer_dir)?;
    let mut offset = 0u64;
    const PAGE: usize = 256;

    let mut pending: Vec<PendingAssessment> = Vec::new();
    let mut assessed_decision_ids: std::collections::HashSet<String> =
        std::collections::HashSet::new();

    loop {
        let events = ledger.read_for_tenant(tenant_id, offset, PAGE)?;
        if events.is_empty() {
            break;
        }

        for event in &events {
            match event.event_type {
                EventType::IngestBatchClassified => {
                    if let Some(event_id) = event.event_id {
                        if let Some(captures) =
                            event.payload.get("captures").and_then(|v| v.as_array())
                        {
                            for (idx, capture) in captures.iter().enumerate() {
                                let kind =
                                    capture.get("kind").and_then(|v| v.as_str()).unwrap_or("");
                                if kind == "decision" {
                                    let decision_id = format!("capture:{event_id}:{idx}"); // ubs:ignore: per-capture owned key moved into PendingAssessment.decision_id
                                    pending.push(PendingAssessment {
                                        decision_id,
                                        causation_event_id: event_id,
                                        decision_text: render_decision_text(capture),
                                    });
                                }
                            }
                        }
                    }
                }
                EventType::DecisionProposed => {
                    if let (Some(event_id), Some(decision_id)) = (
                        event.event_id,
                        event.payload.get("decision_id").and_then(|v| v.as_str()),
                    ) {
                        pending.push(PendingAssessment {
                            decision_id: decision_id.to_owned(), // ubs:ignore: owned id moved into PendingAssessment.decision_id
                            causation_event_id: event_id,
                            decision_text: render_proposed_decision_text(&event.payload),
                        });
                    }
                }
                EventType::DecisionScored => {
                    let is_v2 = event
                        .payload
                        .get("schema_version")
                        .and_then(serde_json::Value::as_u64)
                        == Some(u64::from(DECISION_ASSESSED_SCHEMA_VERSION));
                    // A row no reader can parse is "assessment unavailable" (hivemind-qo11.10),
                    // so it must not stop the decision from being assessed properly.
                    if is_v2 && !crate::events::is_unreadable_annotation(event) {
                        if let Some(decision_id) =
                            event.payload.get("decision_id").and_then(|v| v.as_str())
                        {
                            assessed_decision_ids.insert(decision_id.to_owned());
                            // ubs:ignore: borrows from event.payload &str; owned copy needed for HashSet<String>
                        }
                    }
                }
                _ => {}
            }
        }

        if let Some(last) = events.last().and_then(|e| e.event_id) {
            offset = last;
        } else {
            break;
        }
    }

    let unscored: Vec<_> = pending
        .into_iter()
        .filter(|c| !assessed_decision_ids.contains(&c.decision_id))
        .collect();

    Ok(unscored)
}

/// Build a compact text description of a decision capture for the assessment prompt.
///
/// The labeled fields are exactly [`classified_capture_fields`], the text a quote is checked
/// against on the write path, so everything offered as quotable is accepted. The confidence the
/// capture expressed and its topic keys are not part of that text; they ride in as `Recorded
/// context` lines the prompt says never to quote.
fn render_decision_text(capture: &serde_json::Value) -> String {
    let mut out = String::new();

    for (label, value) in classified_capture_fields(capture) {
        let _ = writeln!(out, "{label}: {value}");
    }

    if let Some(expressed) = capture.get("expressed_confidence").and_then(|v| v.as_str()) {
        let _ = writeln!(out, "Recorded context, expressed confidence: {expressed}");
    }
    if let Some(keys) = capture.get("topic_keys").and_then(|v| v.as_array()) {
        let ks: Vec<&str> = keys.iter().filter_map(|k| k.as_str()).collect();
        if !ks.is_empty() {
            let _ = writeln!(out, "Recorded context, topic keys: {}", ks.join(", "));
        }
    }

    out
}

/// Build a compact text description of a directly captured (proposed) decision for the
/// assessment prompt: title, question, quote, rationale and each option's label and
/// description. The labeled fields are exactly [`proposed_decision_fields`], the text
/// `Commands::record_decision_assessed` checks a quote against for a proposed decision.
fn render_proposed_decision_text(payload: &serde_json::Value) -> String {
    let mut out = String::new();

    for (label, value) in proposed_decision_fields(payload) {
        let _ = writeln!(out, "{label}: {value}");
    }

    // The decision floor's two answers (hivemind-bbnw.11), as the capturer stated them. They are
    // input facts for a reach or irreversibility explanation to cite, not decision text: the
    // quote check does not read them, and the prompt says so. A decision that was never asked
    // gets neither line.
    if let Some(binds) = payload
        .get("binds")
        .and_then(|value| serde_json::from_value::<BindsAnswer>(value.clone()).ok())
    {
        let _ = writeln!(out, "Recorded answer, binds: {}", binds.describe());
    }
    if let Some(undo_cost) = payload
        .get("undo_cost")
        .and_then(|value| serde_json::from_value::<UndoCostAnswer>(value.clone()).ok())
    {
        let _ = writeln!(out, "Recorded answer, undo cost: {}", undo_cost.describe());
    }

    out
}

/// Resolve the scorer model id given an already-read `LOOMTRACER_SCORER_MODEL`
/// value: the override if present, otherwise the pinned default.
fn resolve_scorer_model(env_override: Option<String>) -> String {
    env_override.unwrap_or_else(|| SCORER_MODEL.to_owned())
}

/// Model id actually used for scoring: `LOOMTRACER_SCORER_MODEL` if set to a
/// non-empty value, otherwise the pinned default. Callers that both invoke
/// the API and record provenance must call this once and reuse the result so
/// the two agree even if the environment changes between calls.
fn effective_scorer_model() -> String {
    resolve_scorer_model(crate::identity::env_value(SCORER_MODEL_ENV))
}

async fn call_assessor(
    client: &reqwest::Client,
    api_key: &str,
    decision_text: &str,
) -> Result<(AssessmentOutput, String), crate::anthropic::BoxError> {
    let model = effective_scorer_model();
    let user_content = format!("{ASSESSOR_PROMPT}\n\n---DECISION---\n{decision_text}");
    let output = crate::anthropic::call_json_schema(
        client,
        api_key,
        &model,
        MAX_TOKENS,
        user_content,
        assessment_schema(),
    )
    .await?;
    Ok((output, model))
}

/// Build a `DecisionAssessedPayload` (schema version 2) from a model's raw structured output.
/// Shared by the background worker (`write_assessment`) and the keyless CLI path
/// (`emit decision.scored`) so both write the same shape through the same validator
/// (`Commands::record_decision_assessed`) — a plugin/edge Haiku call is not trusted any more
/// than the server's own call.
pub(crate) fn build_assessed_payload(
    decision_id: &str,
    model: &str,
    prompt_version: &str,
    output: AssessmentOutput,
) -> DecisionAssessedPayload {
    DecisionAssessedPayload {
        schema_version: DECISION_ASSESSED_SCHEMA_VERSION,
        decision_id: decision_id.to_owned(),
        model: model.to_owned(),
        prompt_version: prompt_version.to_owned(),
        supersedes_score_id: None,
        dimensions: output.dimensions,
        importance: output.importance,
    }
}

/// Resolve the canonical `capture:{event_id}:{idx}` node id for a decision
/// capture inside a prior `ingest.batch_classified` batch, by scanning the
/// ledger for the event carrying `batch_id`. Used by the keyless CLI path
/// (`emit decision.scored`) so plugins never construct the node-id format
/// themselves — only the server knows that shape.
///
/// Returns the node id plus the classified-batch event id, for use as the
/// resulting `decision.scored` event's `causation_event_id`.
pub(crate) fn resolve_capture_node_id<L: EventLedger>(
    ledger: &L,
    tenant_id: &TenantId,
    batch_id: &str,
    idx: usize,
) -> crate::Result<(String, EventId)> {
    let mut offset = 0u64;
    const PAGE: usize = 256;

    loop {
        let events = ledger.read_for_tenant(tenant_id, offset, PAGE)?;
        if events.is_empty() {
            break;
        }

        for event in &events {
            if event.event_type != EventType::IngestBatchClassified {
                continue;
            }
            let Some(event_id) = event.event_id else {
                continue;
            };
            let Some(this_batch_id) = event.payload.get("batch_id").and_then(|v| v.as_str()) else {
                continue;
            };
            if this_batch_id != batch_id {
                continue;
            }

            let captures = event
                .payload
                .get("captures")
                .and_then(|v| v.as_array())
                .ok_or_else(|| {
                    crate::CommandError::Validation(format!(
                        "batch {batch_id} has no captures array"
                    ))
                })?;
            let capture = captures.get(idx).ok_or_else(|| {
                crate::CommandError::Validation(format!(
                    "capture-index {idx} out of range for batch {batch_id} ({} captures)",
                    captures.len()
                ))
            })?;
            let kind = capture.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            if kind != "decision" {
                return Err(crate::CommandError::Validation(format!( // ubs:ignore: error path — returns immediately, exits the loop, not a per-iteration allocation
                    "capture at index {idx} in batch {batch_id} is kind={kind:?}, not \"decision\" — only decision captures are scored"
                ))
                .into());
            }

            return Ok((format!("capture:{event_id}:{idx}"), event_id)); // ubs:ignore: single owned key on the match-found return path — exits the loop, not a per-iteration allocation
        }

        if let Some(last) = events.last().and_then(|e| e.event_id) {
            offset = last;
        } else {
            break;
        }
    }

    Err(crate::CommandError::Validation(format!(
        "no ingest.batch_classified event found for batch_id {batch_id}"
    ))
    .into())
}

fn write_assessment(
    loomtracer_dir: &PathBuf,
    tenant_id: &TenantId,
    decision_id: &str,
    model: &str,
    output: AssessmentOutput,
    causation_event_id: Option<u64>,
) -> crate::Result<()> {
    let payload = build_assessed_payload(decision_id, model, ASSESSOR_PROMPT_VERSION, output);

    let ledger = SqliteEventLedger::open(loomtracer_dir)?;
    let commands = Commands::new_with_context(
        &ledger,
        CommandContext::new(tenant_id.clone(), EventProvenance::agent(ACTOR_ID)),
    );
    commands.record_decision_assessed(ACTOR_ID, payload, causation_event_id)?;
    Ok(())
}

/// Try to read the API key and spawn the worker. Logs a warning and returns
/// `None` if the key is absent.
pub fn try_spawn(loomtracer_dir: Arc<PathBuf>, tenant_id: TenantId) -> Option<()> {
    match crate::classifier::anthropic_api_key() {
        Some(key) => {
            spawn_scorer(loomtracer_dir, tenant_id, key);
            Some(())
        }
        None => {
            warn!(
                target: "loomtracer::scorer",
                "ANTHROPIC_API_KEY not set — Layer-3 scorer disabled"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests;
