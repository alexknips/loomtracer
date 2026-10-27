//! `DecisionBrief`: the shared output contract for fluent follow-up verbs.
//!
//! No existing query response leads with "the decision, then why, then who, then does it still
//! hold" — the pieces exist but are scattered across three deterministic Layer-2 query functions.
//! `get_decision_brief` composes them; it does not duplicate their logic and performs no writes,
//! no ranking, and no LLM calls. See docs/AGENT_FLUENT_QUERYING.md §4.
//!
//! Known gap, stated rather than papered over (AGENTS.md §6, no invented confidence): there is no
//! per-option rejection rationale in the graph schema today, only the decision-level `rationale`.
//! `rejected_options` therefore share that single rationale rather than carrying a distinct
//! reason each — a capture-side schema change would be required to do better, and is out of
//! scope for this bead.
//!
//! An option is `rejected` only against a recorded choice. A decision with options and no choice
//! lists them as `open_options`, so an open question never reads as one that turned everything
//! down.

use std::borrow::Cow;

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::floor::RecordedFloor;
use crate::projector::{GraphRow, GraphView, NodeKind, RelationKind};
use crate::Result;

use super::context::{get_decision_context, linked_actor_ids, ReviewShape};
use super::decision::get_decision_with_labels;
use super::grounding::{grounding_of_at, GroundingItem, GroundingState, UncheckedBet};
use super::outcome::{get_decision_outcome_with_labels, OutcomeReason};
use super::project_label::ProjectLabels;
use super::question::{earliest_ask, other_answers, QuestionAnswer};
use super::reexamine::{
    propagation_of_decision, Propagation, ReexamineEntry, DEFAULT_PROPAGATION_WINDOW_DAYS,
};
use super::shared::{
    neighbor_edges_with_time, node_row, node_rows, optional_datetime, optional_int,
    optional_string, optional_string_list, query_error, query_timer_start, required_string,
    Direction,
};
use super::status::DecisionStatus;
use super::QueryResponse;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OptionLabel {
    pub option_id: String,
    /// What a person reads: the recorded label, turned into words when it was recorded as a slug
    /// or a lettered code (`pick-a-postgres` reads `Postgres`).
    pub label: String,
    /// What the capture recorded, when that reads differently from `label`. The ledger is
    /// immutable and `label` is a reading of it, so a reader is told what the record says.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recorded_as: Option<String>,
}

/// An option label as one countable unit inside a text list of labels. A label that holds the
/// list's own punctuation (`,` or `;`) or a double quote is wrapped in double quotes, with inner
/// quotes escaped, so `"Rename after the comparison, before the first listing", Pause` reads as
/// two options. Any other label stays as it is. Text answers only: the JSON keeps labels whole.
pub fn option_label_unit(label: &str) -> Cow<'_, str> {
    if label.contains([',', ';', '"']) {
        Cow::Owned(format!("\"{}\"", label.replace('"', "\\\"")))
    } else {
        Cow::Borrowed(label)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DecidedBy {
    /// The actor who *recorded* this decision (the `PROPOSED_BY` actor) -- often a scribe
    /// writing down someone else's call, not necessarily the one who made it. See
    /// `decider_ids` for who actually decided (hivemind-zdsh.9).
    pub proposer_id: Option<String>,
    /// The actor(s) who actually *decided* (`ACCEPTED_BY` targets). Empty when the decision
    /// hasn't been accepted by anyone yet (`review == Unreviewed`); equal to `proposer_id`
    /// on self-acceptance.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub decider_ids: Vec<String>,
    pub source: String,
    pub source_ref: Option<String>,
    /// The human whose delegated scope an agent's self-acceptance fell within
    /// (hivemind-zdsh.6). Present, an agent decided within that delegation; absent on an
    /// agent's self-accepted decision, the agent decided alone — the two cases that share
    /// `review == SelfAccepted` and a `decider_ids` equal to `proposer_id`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delegated_by: Option<String>,
    pub review: ReviewShape,
    /// The actor(s) who rejected the decision (`REJECTED_BY` targets). Empty when nobody did.
    /// With no `decider_ids`, `review == Rejected` and these are who turned it down.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rejecter_ids: Vec<String>,
    /// Where a decision a classifier drafted from a transcript was held. Present exactly for such
    /// a draft (a `capture:<offset>:<index>` node), whatever it can say about the conversation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drafted_from: Option<DraftedFrom>,
    /// Actors the decision names as deciders who said the call was not theirs
    /// (`decision.attribution_disputed`), one per actor with their latest statement. The
    /// acceptance stays in `decider_ids` as recorded: a dispute sits beside it, never replaces it
    /// (hivemind-purmd).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attribution_disputes: Vec<AttributionDispute>,
}

/// One actor's statement that a decision names them as its decider and the call was not theirs.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AttributionDispute {
    pub disputed_by: String,
    /// What they said: their own words, typed with their credential or, when `relayed_by` is
    /// present, the verbatim quote their agent recorded.
    pub reason: String,
    /// The agent that recorded the statement for the person (hivemind-f9xgu): the event's
    /// `agent` source reference when it is not the disputing actor itself. Absent when the actor
    /// disputed with their own credential.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relayed_by: Option<String>,
    /// Where they said it (a session, thread or message), as the relaying agent stated it.
    /// Absent when nobody did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub said_in: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disputed_at: Option<DateTime<Utc>>,
}

/// The attribution disputes recorded against `decision_id`: each disputing actor's latest
/// statement, in the order those statements were recorded. One bulk read of the (few) dispute
/// nodes: they have no edges, so the decision is matched on the node's `decision_id` property.
fn attribution_disputes(
    graph: &impl GraphView,
    decision_id: &str,
) -> Result<Vec<AttributionDispute>> {
    let mut disputes: Vec<(i64, AttributionDispute)> = Vec::new();
    for (_, row) in node_rows(graph, NodeKind::AttributionDispute)? {
        if required_string(&row, "decision_id")? != decision_id {
            continue;
        }
        let origin = optional_int(&row, "event_origin").unwrap_or(i64::MAX);
        let disputed_by = optional_string(&row, "disputed_by").unwrap_or_default();
        let relayed_by = (optional_string(&row, "source").as_deref() == Some("agent"))
            .then(|| optional_string(&row, "source_ref"))
            .flatten()
            .filter(|relayer| *relayer != disputed_by);
        let dispute = AttributionDispute {
            disputed_by,
            reason: optional_string(&row, "reason").unwrap_or_default(),
            relayed_by,
            said_in: optional_string(&row, "said_in"),
            disputed_at: optional_datetime(&row, "disputed_at")?,
        };
        match disputes
            .iter_mut()
            .find(|(_, seen)| seen.disputed_by == dispute.disputed_by)
        {
            Some((seen_origin, _)) if *seen_origin > origin => {}
            Some(slot) => *slot = (origin, dispute),
            None => disputes.push((origin, dispute)),
        }
    }
    disputes.sort_by(|(a, left), (b, right)| {
        a.cmp(b)
            .then_with(|| left.disputed_by.cmp(&right.disputed_by))
    });
    Ok(disputes.into_iter().map(|(_, dispute)| dispute).collect())
}

/// The conversation a classifier drafted a decision from, as the ledger's received batches state
/// it. Each part is empty when the ledger never received the batches (a `loomtracer emit` capture
/// names a fresh batch id): who was in a conversation is never guessed.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DraftedFrom {
    /// The capture sessions that shipped the batches, sorted.
    pub session_ids: Vec<String>,
    /// Whoever submitted the session's first batch: the one who started it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initiated_by: Option<String>,
    /// Every submitter in the session and the agent beside them.
    pub participants: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StillHolds {
    pub held_up: bool,
    /// Whole days from `occurred_at` until now, or until the supersession was recorded (the
    /// time the link was made, not the date of the decision it names): how long nothing in the
    /// graph replaced it. It is not evidence that the decision was carried out. Absent, never
    /// `0`, when the dates needed are not recorded or run backwards.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub days_held: Option<u32>,
    pub reasons: Vec<OutcomeReason>,
    /// Bets whose check date has passed with no evidence either way. Attention, not staleness:
    /// `held_up` is unaffected.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unchecked: Vec<UncheckedBet>,
    /// The premises this decision rests on that changed (superseded, rejected, contested,
    /// refuted), each open or closed: whether anyone has looked again, and if so who, when and
    /// why. Empty when none changed. `held_up` follows them (hivemind-ottw.2): a change someone
    /// reaffirmed no longer reads as stale, a contested premise opens an item but never flips it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reexamine: Vec<ReexamineEntry>,
}

/// Leads with the decision, then why, who decided, and whether it still holds. IDs are output
/// handles for follow-up (`--id`, `--pick`), never required reading to understand the answer.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DecisionBrief {
    pub decision_id: String,
    pub title: String,
    /// Stable link segment (`/decisions/<slug>`), assigned once at proposal and unchanged by
    /// a later retitle (`GET /v1/graph`, hivemind-nidp). `None` only for a decision from an
    /// event predating this field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    /// Address of the project the decision is filed under (see `DecisionView::project`).
    pub project: Option<String>,
    /// What a person calls that project (see `DecisionView::project_label`).
    pub project_label: String,
    pub rationale: String,
    /// Verbatim words of the decider, self-contained. Always present together with
    /// `question` (hivemind-zdsh.13).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
    /// The question this decision answers, in the capturer's own words.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    /// The `Question` node the decision answers (see `DecisionView::question_id`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub question_id: Option<String>,
    /// What the capturer said the decision binds and what undoing it costs, and the floor those
    /// answers reach (see `DecisionView::floor`). Serialized as `binds`, `undo_cost` and `floor`.
    #[serde(flatten)]
    pub floor: RecordedFloor,
    /// Other decisions that answer the same question, in the order they were recorded, each with
    /// where it stands. A superseded answer is listed too: it is part of the question's history.
    /// Never resolved for the reader: two accepted answers that choose differently also appear
    /// as `still_holds.reasons` `conflicting_answer`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub other_answers: Vec<QuestionAnswer>,
    /// Display-only; `event_origin` stays canonical for resolver ranking (SEARCH_DESIGN.md).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurred_at: Option<DateTime<Utc>>,
    /// When this decision's question was first explicitly asked (`loomtracer ask` /
    /// `request_decision`), if ever — `occurred_at` is when it was answered. `None` when the
    /// question was only ever named via `--question`, never asked first (hivemind-bbnw.4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asked_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chosen_option: Option<OptionLabel>,
    /// The options someone turned down: every recorded option other than the chosen one. Empty
    /// while no option is chosen, because nobody has turned anything down then; see
    /// `open_options`.
    pub rejected_options: Vec<OptionLabel>,
    /// The options on the table when no choice is recorded (`chosen_option` is absent): the
    /// question is open, or the capture named its options and not its pick. Never overlaps
    /// `rejected_options`, and absent from the JSON when a choice is recorded.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub open_options: Vec<OptionLabel>,
    pub decided_by: DecidedBy,
    /// What the decision rests on, each item with its state and whether it was named at capture
    /// or attributed later.
    pub rests_on: Vec<GroundingItem>,
    /// Grounded, a declared bet, or nothing declared ("never asked").
    pub grounding_state: GroundingState,
    /// `low`, `medium` or `high`, in the decider's own words at capture; absent when not given.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expressed_confidence: Option<String>,
    /// How many other decisions follow from this one.
    pub dependents_count: usize,
    /// When this decision itself changed (superseded, rejected or contested): how many of the
    /// decisions that rest on it have been looked at again, and how many are still open. Absent
    /// for a decision that has not changed, and for one nothing rested on when it did. Never
    /// names an actor (hivemind-ottw.3).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub propagation: Option<Propagation>,
    pub still_holds: StillHolds,
    pub topic_keys: Vec<String>,
    pub status: DecisionStatus,
}

/// Compose `get_decision` + `get_decision_context` + `get_decision_outcome` plus an
/// `Option.label` lookup and what the decision rests on into one answer. Deterministic graph
/// reads, no write, no LLM. Reads the clock once, to say whether a bet's check date has passed;
/// see `get_decision_brief_at`.
pub fn get_decision_brief(
    graph: &impl GraphView,
    decision_id: &str,
) -> Result<QueryResponse<Option<DecisionBrief>>> {
    get_decision_brief_at(graph, decision_id, Utc::now())
}

/// `get_decision_brief` as of `now`.
pub fn get_decision_brief_at(
    graph: &impl GraphView,
    decision_id: &str,
    now: DateTime<Utc>,
) -> Result<QueryResponse<Option<DecisionBrief>>> {
    let labels = ProjectLabels::from_graph(graph)?;
    get_decision_brief_with_labels(graph, decision_id, now, &labels)
}

/// `get_decision_brief_at` for callers that compose many briefs in one query (the decision
/// log): they load the project labels once and share them.
pub(crate) fn get_decision_brief_with_labels(
    graph: &impl GraphView,
    decision_id: &str,
    now: DateTime<Utc>,
    labels: &ProjectLabels,
) -> Result<QueryResponse<Option<DecisionBrief>>> {
    let started = query_timer_start();

    let Some(decision) = get_decision_with_labels(graph, decision_id, labels)?.data else {
        return Ok(QueryResponse {
            result_count: 0,
            truncated: false,
            latency_ms: started.elapsed().as_millis(),
            data: None,
        });
    };

    let context = get_decision_context(graph, decision_id)?
        .data
        .ok_or_else(|| query_error("decision exists but has no context"))?;
    let outcome = get_decision_outcome_with_labels(graph, decision_id, now, labels)?
        .data
        .ok_or_else(|| query_error("decision exists but has no outcome"))?;
    let (occurred_at, expressed_confidence, slug, session_ids) =
        decision_capture_facts(graph, decision_id)?;
    let attribution_disputes = attribution_disputes(graph, decision_id)?;
    let rejecter_ids = if context.rejected_count > 0 {
        linked_actor_ids(graph, decision_id, "REJECTED_BY")?
    } else {
        Vec::new()
    };
    let drafted_from = if is_classified_capture(decision_id) {
        Some(drafted_from(graph, decision_id, session_ids)?)
    } else {
        None
    };
    let grounding = grounding_of_at(graph, decision_id, now)?;
    let days_held = days_held(graph, decision_id, occurred_at, now)?;
    // Only a decision that changed has a propagation to report; any other reads nothing extra.
    let propagation = match decision.status {
        DecisionStatus::Superseded | DecisionStatus::Rejected | DecisionStatus::Contested => {
            propagation_of_decision(graph, decision_id, DEFAULT_PROPAGATION_WINDOW_DAYS, now)?
        }
        DecisionStatus::Proposed | DecisionStatus::Accepted => None,
    };
    let (other_answers, asked_at) = match &decision.question_id {
        Some(question_id) => (
            other_answers(graph, decision_id)?,
            earliest_ask(graph, question_id)?,
        ),
        None => (Vec::new(), None),
    };

    let chosen_option = match &decision.chosen_option_id {
        Some(option_id) => Some(resolve_option_label(graph, option_id)?),
        None => None,
    };
    // An option is turned down only against a choice. With no choice recorded the options are
    // still on the table, however the decision stands.
    let mut rejected_options = Vec::new();
    let mut open_options = Vec::new();
    for option_id in &decision.option_ids {
        if decision.chosen_option_id.as_deref() == Some(option_id.as_str()) {
            continue;
        }
        let label = resolve_option_label(graph, option_id)?;
        if chosen_option.is_some() {
            rejected_options.push(label);
        } else {
            open_options.push(label);
        }
    }

    let brief = DecisionBrief {
        decision_id: decision.id,
        title: decision.title,
        slug,
        project: decision.project,
        project_label: decision.project_label,
        rationale: decision.rationale,
        quote: decision.quote,
        question: decision.question,
        question_id: decision.question_id,
        floor: decision.floor,
        other_answers,
        occurred_at,
        asked_at,
        chosen_option,
        rejected_options,
        open_options,
        decided_by: DecidedBy {
            proposer_id: context.proposer_id,
            decider_ids: context.accepted_by,
            source: context.source,
            source_ref: context.source_ref,
            delegated_by: context.delegated_by,
            review: context.review,
            rejecter_ids,
            drafted_from,
            attribution_disputes,
        },
        rests_on: grounding.items,
        grounding_state: grounding.state,
        expressed_confidence,
        dependents_count: grounding.dependents_count,
        propagation,
        still_holds: StillHolds {
            held_up: outcome.held_up,
            days_held,
            reasons: outcome.reasons,
            unchecked: outcome.unchecked,
            reexamine: outcome.reexamine,
        },
        topic_keys: decision.topic_keys,
        status: decision.status,
    };

    Ok(QueryResponse {
        result_count: 1,
        truncated: false,
        latency_ms: started.elapsed().as_millis(),
        data: Some(brief),
    })
}

/// `StillHolds::days_held`: whole days from `occurred_at` to when the first supersession of the
/// decision was recorded, or to `now` while nothing superseded it. The end is the time of the
/// `SUPERSEDES` edge, the same date the re-examine items and the propagation line give the
/// change, never the superseding decision's own `occurred_at`: that decision can be older than
/// the link (`emit decision.superseded --old A --new B` with a B decided weeks ago), and the
/// graph held A as standing until the link was recorded. Only an edge that states no time falls
/// back to the superseder's `occurred_at`. A fact about the dates on the record: `None` when the
/// decision's date, or a supersession's, is not recorded, or when the end comes before the start,
/// because a guess would read as a measurement.
fn days_held(
    graph: &impl GraphView,
    decision_id: &str,
    occurred_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Result<Option<u32>> {
    let Some(start) = occurred_at else {
        return Ok(None);
    };
    let supersessions = neighbor_edges_with_time(
        graph,
        NodeKind::Decision,
        decision_id,
        RelationKind::Supersedes,
        NodeKind::Decision,
        Direction::Incoming,
    )?;
    // Concurrent supersessions both survive; the decision was held until the earliest link. One
    // that cannot be dated could be the earliest, so the figure is left out.
    let mut earliest: Option<DateTime<Utc>> = None;
    for (by_id, stamp) in &supersessions {
        let linked_at = match stamp.at {
            Some(at) => Some(at),
            None => superseder_occurred_at(graph, by_id)?,
        };
        let Some(linked_at) = linked_at else {
            return Ok(None);
        };
        earliest = Some(earliest.map_or(linked_at, |known| known.min(linked_at)));
    }
    let end = earliest.unwrap_or(now);
    Ok(u32::try_from((end - start).num_days()).ok())
}

fn superseder_occurred_at(graph: &impl GraphView, by_id: &str) -> Result<Option<DateTime<Utc>>> {
    match node_row(graph, NodeKind::Decision, by_id)? {
        Some(row) => optional_datetime(&row, "occurred_at"),
        None => Ok(None),
    }
}

pub(super) fn resolve_option_label(graph: &impl GraphView, option_id: &str) -> Result<OptionLabel> {
    let row = node_row(graph, NodeKind::Option, option_id)?;
    Ok(option_label_of(option_id, row.as_ref()))
}

/// The label of an option from its node row. An option with no node, or a node with no label,
/// reads as its id.
pub(super) fn option_label_of(option_id: &str, row: Option<&GraphRow>) -> OptionLabel {
    let label = row
        .and_then(|row| optional_string(row, "label"))
        .unwrap_or_else(|| option_id.to_owned());
    let recorded_as = row
        .and_then(|row| optional_string(row, "recorded_label"))
        .filter(|recorded| *recorded != label);
    OptionLabel {
        option_id: option_id.to_owned(),
        label,
        recorded_as,
    }
}

/// `occurred_at`, `expressed_confidence`, `slug`, `session_ids` -- see `decision_capture_facts`.
type CaptureFacts = (
    Option<DateTime<Utc>>,
    Option<String>,
    Option<String>,
    Vec<String>,
);

/// When the decision was captured (display-only), the confidence its decider expressed then,
/// its stable link segment (`slug`, hivemind-nidp) and the capture sessions that shipped the
/// batches it was classified from.
fn decision_capture_facts(graph: &impl GraphView, decision_id: &str) -> Result<CaptureFacts> {
    match node_row(graph, NodeKind::Decision, decision_id)? {
        Some(row) => Ok((
            optional_datetime(&row, "occurred_at")?,
            optional_string(&row, "expressed_confidence"),
            optional_string(&row, "slug"),
            optional_string_list(&row, "session_ids"),
        )),
        None => Ok((None, None, None, Vec::new())),
    }
}

/// A classifier-drafted decision is a `capture:<offset>:<index>` node, the id the projector gives
/// a capture of `ingest.batch_classified`.
pub(super) fn is_classified_capture(decision_id: &str) -> bool {
    decision_id.starts_with("capture:")
}

/// What the graph states of the conversation a classified capture was drafted in: the sessions on
/// the node, and the `INITIATED_BY` / `PARTICIPATED_BY` actors the projector read from the
/// batches' submitters.
fn drafted_from(
    graph: &impl GraphView,
    decision_id: &str,
    session_ids: Vec<String>,
) -> Result<DraftedFrom> {
    Ok(DraftedFrom {
        session_ids,
        initiated_by: linked_actor_ids(graph, decision_id, "INITIATED_BY")?
            .into_iter()
            .next(),
        participants: linked_actor_ids(graph, decision_id, "PARTICIPATED_BY")?,
    })
}

#[cfg(test)]
mod tests;
