//! Temporal decision history: diffs, weekly summaries, and slot-keyed temporal query output.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::time::Instant;

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{json, Value};

use crate::commands::personal_project_handle;
use crate::error::QueryError;
use crate::events::{
    self, DecisionAcceptedPayload, DecisionMovedPayload, DecisionProposedPayload,
    DecisionReaffirmedPayload, DecisionRejectedPayload, DecisionRetitledPayload,
    DecisionSupersededPayload, Event, EventId, EventPayload, EventSource, EventType,
    RelationAddedPayload, RelationKind as EventRelationKind,
};
use crate::floor::RecordedFloor;
use crate::ledger::EventLedger;
use crate::projector::{GraphRow, GraphView, NodeKind};
use crate::Result;

use self::drafts::{ClassifiedDraft, ClassifiedDrafts};
use super::brief::{is_classified_capture, option_label_of, OptionLabel};
use super::project_label::ProjectLabels;
use super::reexamine;
use super::shared::{
    node_row, node_rows, normalized_query, normalized_topic_filter_values, optional_string_list,
    recorded_floor_of, stored_topics_include,
};
use super::status::DecisionStandings;
use super::timeline::{captured_creations, captured_standings, CapturedStanding};
use super::{DecisionStatus, QueryResponse, MAX_QUERY_RESULTS};

mod drafts;

const DEFAULT_HISTORY_LIMIT: usize = 25;
pub(super) const LEDGER_READ_PAGE_SIZE: usize = 1_000;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HistoryFilterRequest {
    pub actor_ids: Vec<String>,
    pub sources: Vec<String>,
    pub source_refs: Vec<String>,
    pub topic_keys: Vec<String>,
    pub statuses: Vec<DecisionStatus>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RecentActivityRequest {
    pub filters: HistoryFilterRequest,
    pub limit: usize,
    pub cursor: Option<String>,
}

impl Default for RecentActivityRequest {
    fn default() -> Self {
        Self {
            filters: HistoryFilterRequest::default(),
            limit: DEFAULT_HISTORY_LIMIT,
            cursor: None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RecentDecisionFilterRequest {
    pub actor_patterns: Vec<String>,
    pub sources: Vec<String>,
    pub topic_keys: Vec<String>,
    pub statuses: Vec<DecisionStatus>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RecentDecisionsRequest {
    pub since_timestamp: DateTime<Utc>,
    pub until_timestamp: Option<DateTime<Utc>>,
    pub filters: RecentDecisionFilterRequest,
    pub limit: usize,
    pub cursor: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChangedSinceRequest {
    pub since_offset: Option<EventId>,
    pub since_timestamp: Option<DateTime<Utc>>,
    pub until_offset: Option<EventId>,
    pub until_timestamp: Option<DateTime<Utc>>,
    pub filters: HistoryFilterRequest,
    pub limit: usize,
    pub cursor: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReadOnlyExportRequest {
    pub query: ReadOnlyExportQuery,
    pub format: ReadOnlyExportFormat,
    pub generated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ReadOnlyExportQuery {
    RecentActivity(RecentActivityRequest),
    DecisionsChangedSince(ChangedSinceRequest),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadOnlyExportFormat {
    Json,
    Markdown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadOnlyExportQueryKind {
    RecentActivity,
    DecisionsChangedSince,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryChangeKind {
    NewDecision,
    StatusChange,
    NewEvidence,
    /// Something a decision rests on no longer stands: an assumption it premises on was
    /// refuted (a failed bet included), or a prior decision it follows from was superseded or
    /// rejected. The row's `decision_ids` are the affected decisions, not the premise itself.
    StalePremise,
    Supersession,
    /// A decision changed which project it belongs to (`decision.moved`). Its own kind,
    /// not folded into `ContextChange`, so a move is never dropped by compaction and is
    /// always distinguishable in history without inspecting the raw event
    /// (approved record shape, item 3; hivemind-s15q.10).
    ProjectMoved,
    /// A decision's title changed (`decision.retitled`). Its own kind, not folded into
    /// `ContextChange`, so a retitle is never dropped by compaction and is always
    /// distinguishable in history without inspecting the raw event (mirrors
    /// `ProjectMoved`'s treatment; hivemind-ydmp).
    TitleChanged,
    /// Someone said a decision still holds after a premise it rests on changed
    /// (`decision.reaffirmed`). Its own kind, not folded into `ContextChange`, so a "still holds"
    /// is never dropped by compaction and is always distinguishable in history without
    /// inspecting the raw event (mirrors `ProjectMoved` and `TitleChanged`; hivemind-ottw.3).
    Reaffirmed,
    ContextChange,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct HistoryFilters {
    pub actor_ids: Vec<String>,
    pub sources: Vec<String>,
    pub source_refs: Vec<String>,
    pub topic_keys: Vec<String>,
    pub statuses: Vec<DecisionStatus>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AffectedNode {
    pub id: String,
    pub kind: NodeKind,
}

/// Where a `decision.moved` took a decision. Carried on the history row of a
/// [`HistoryChangeKind::ProjectMoved`] change next to the row's own `actor_id` and `ts`, so
/// "moved from Billing to Pricing by Alex on <date>" is answerable from the row alone
/// (approved record shape, item 3; hivemind-s15q.10). `None` on every other kind of change.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProjectMove {
    pub from: String,
    pub to: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// What a `decision.reaffirmed` said: the premise the decision still holds after, and why. Carried
/// on the history row of a [`HistoryChangeKind::Reaffirmed`] change next to the row's own
/// `actor_id` and `ts`, so "reaffirmed after <premise> changed, by <actor> on <date>: <reason>" is
/// answerable from the row alone. `None` on every other kind of change.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ReaffirmedPremise {
    /// A decision or a hypothesis id.
    pub premise_id: String,
    pub reason: String,
}

/// Where a `decision.retitled` took a decision's title. Carried on the history row of a
/// [`HistoryChangeKind::TitleChanged`] change next to the row's own `actor_id` and `ts`
/// (mirrors [`ProjectMove`] exactly; hivemind-ydmp). `None` on every other kind of change.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TitleChange {
    pub from: String,
    pub to: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LedgerRange {
    pub from_offset_exclusive: EventId,
    pub to_offset_inclusive: EventId,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct EventCitation {
    pub citation_id: String,
    pub event_id: EventId,
    pub event_uuid: String,
    pub event_type: EventType,
    pub actor_id: String,
    pub source: EventSource,
    pub source_ref: Option<String>,
    pub ts: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ActivityRow {
    pub event_origin: EventId,
    pub event_uuid: String,
    pub event_type: EventType,
    pub change_kind: HistoryChangeKind,
    pub actor_id: String,
    pub source: EventSource,
    pub source_ref: Option<String>,
    pub ts: Option<DateTime<Utc>>,
    pub decision_ids: Vec<String>,
    pub affected_nodes: Vec<AffectedNode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_move: Option<ProjectMove>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title_change: Option<TitleChange>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reaffirmed_premise: Option<ReaffirmedPremise>,
    pub citation_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RecentActivityResults {
    pub filters: HistoryFilters,
    pub limit: usize,
    pub cursor: Option<String>,
    pub next_cursor: Option<String>,
    pub total_matches: usize,
    pub ledger_range: LedgerRange,
    pub items: Vec<ActivityRow>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct RecentDecisionFilters {
    pub actor_patterns: Vec<String>,
    pub sources: Vec<String>,
    pub topic_keys: Vec<String>,
    pub statuses: Vec<DecisionStatus>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RecentDecisionEntry {
    pub decision_id: String,
    pub title: String,
    pub rationale: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    pub status: DecisionStatus,
    /// Address of the project the decision is filed under: where its latest move took it, else
    /// the handle stated on the proposal, else the recorder's derived personal address (see
    /// `DecisionView::project`).
    pub project: String,
    /// What a person calls that project (see `DecisionView::project_label`).
    pub project_label: String,
    pub topic_keys: Vec<String>,
    pub actor_ids: Vec<String>,
    /// When it was decided, the time `since` / `until` window on: the proposal's own time, or for
    /// a decision a classifier drafted from a transcript (`capture:<event>:<index>`) the time of
    /// the turn it was said in, as `GET /v1/graph` dates it. `creation` is the event that
    /// recorded it, which for a draft is the classification, possibly days later.
    pub decided_at: Option<DateTime<Utc>>,
    pub creation: DecisionEventProvenance,
    /// What the capturer said the decision binds and what undoing it costs, and the floor those
    /// answers reach. Serialized as `binds`, `undo_cost` and `floor` (hivemind-ekmp4).
    #[serde(flatten)]
    pub floor: RecordedFloor,
    pub chosen_option_id: Option<String>,
    /// The options as a person reads them (`OptionLabel::label`), each with the id the choice
    /// names, so a reader shows `Async (chosen)` and never `option-<uuid>`.
    pub options: Vec<OptionLabel>,
    pub evidence_ids: Vec<String>,
    pub hypothesis_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RecentDecisionsResults {
    pub resolved_since: ChangeBoundary,
    pub resolved_until: ChangeBoundary,
    pub filters: RecentDecisionFilters,
    pub limit: usize,
    pub cursor: Option<String>,
    pub next_cursor: Option<String>,
    pub total_matches: usize,
    pub ledger_range: LedgerRange,
    pub items: Vec<RecentDecisionEntry>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ChangeBoundary {
    pub offset: EventId,
    pub timestamp: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DecisionChangeRow {
    pub event_origin: EventId,
    pub event_uuid: String,
    pub event_type: EventType,
    pub change_kind: HistoryChangeKind,
    pub actor_id: String,
    pub source: EventSource,
    pub source_ref: Option<String>,
    pub ts: Option<DateTime<Utc>>,
    pub decision_ids: Vec<String>,
    pub affected_nodes: Vec<AffectedNode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_move: Option<ProjectMove>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title_change: Option<TitleChange>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reaffirmed_premise: Option<ReaffirmedPremise>,
    pub citation_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DecisionsChangedSinceResults {
    pub resolved_since: ChangeBoundary,
    pub resolved_until: ChangeBoundary,
    pub filters: HistoryFilters,
    pub limit: usize,
    pub cursor: Option<String>,
    pub next_cursor: Option<String>,
    pub total_matches: usize,
    pub ledger_range: LedgerRange,
    pub items: Vec<DecisionChangeRow>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DecisionsAddedSinceFilterRequest {
    pub actor_ids: Vec<String>,
    pub sources: Vec<String>,
    pub source_refs: Vec<String>,
    pub import_run_ids: Vec<String>,
    pub topic_keys: Vec<String>,
    pub statuses: Vec<DecisionStatus>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DecisionsAddedSinceRequest {
    pub since_offset: Option<EventId>,
    pub since_timestamp: Option<DateTime<Utc>>,
    pub until_offset: Option<EventId>,
    pub until_timestamp: Option<DateTime<Utc>>,
    pub filters: DecisionsAddedSinceFilterRequest,
    pub limit: usize,
    pub cursor: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct DecisionsAddedSinceFilters {
    pub actor_ids: Vec<String>,
    pub sources: Vec<String>,
    pub source_refs: Vec<String>,
    pub import_run_ids: Vec<String>,
    pub topic_keys: Vec<String>,
    pub statuses: Vec<DecisionStatus>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DecisionEventProvenance {
    pub event_origin: EventId,
    pub event_uuid: String,
    pub event_type: EventType,
    pub actor_id: String,
    pub source: EventSource,
    pub source_ref: Option<String>,
    pub import_run_id: Option<String>,
    pub ts: Option<DateTime<Utc>>,
    pub citation_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DecisionChange {
    pub change_kind: HistoryChangeKind,
    pub provenance: DecisionEventProvenance,
    pub affected_nodes: Vec<AffectedNode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_move: Option<ProjectMove>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title_change: Option<TitleChange>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reaffirmed_premise: Option<ReaffirmedPremise>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AddedDecisionEntry {
    pub decision_id: String,
    pub status: DecisionStatus,
    pub topic_keys: Vec<String>,
    /// When it was decided (see `RecentDecisionEntry::decided_at`): what the timestamp bounds of
    /// the request window on for a decision a classifier drafted.
    pub decided_at: Option<DateTime<Utc>>,
    /// What the capturer said the decision binds and what undoing it costs, and the floor those
    /// answers reach. Serialized as `binds`, `undo_cost` and `floor` (hivemind-ekmp4).
    #[serde(flatten)]
    pub floor: RecordedFloor,
    pub creation: DecisionEventProvenance,
    pub option_ids: Vec<String>,
    pub evidence_ids: Vec<String>,
    pub hypothesis_ids: Vec<String>,
    pub changes_in_window: Vec<DecisionChange>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ChangedDecisionEntry {
    pub decision_id: String,
    pub status: DecisionStatus,
    pub topic_keys: Vec<String>,
    /// What the capturer said the decision binds and what undoing it costs, and the floor those
    /// answers reach. Serialized as `binds`, `undo_cost` and `floor` (hivemind-ekmp4).
    #[serde(flatten)]
    pub floor: RecordedFloor,
    pub creation: Option<DecisionEventProvenance>,
    pub option_ids: Vec<String>,
    pub evidence_ids: Vec<String>,
    pub hypothesis_ids: Vec<String>,
    pub changes_in_window: Vec<DecisionChange>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DecisionsAddedSinceResults {
    pub resolved_since: ChangeBoundary,
    pub resolved_until: ChangeBoundary,
    pub filters: DecisionsAddedSinceFilters,
    pub limit: usize,
    pub cursor: Option<String>,
    pub next_cursor: Option<String>,
    pub total_matches: usize,
    pub total_added: usize,
    pub total_changed_existing: usize,
    pub ledger_range: LedgerRange,
    pub added_decisions: Vec<AddedDecisionEntry>,
    pub changed_existing_decisions: Vec<ChangedDecisionEntry>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReadOnlyExport {
    pub query: ReadOnlyExportQueryKind,
    pub format: ReadOnlyExportFormat,
    pub query_params: BTreeMap<String, Value>,
    pub ledger_range: LedgerRange,
    pub generated_at: DateTime<Utc>,
    pub result_count: usize,
    pub truncated: bool,
    pub continuation_cursor: Option<String>,
    pub citation_map: BTreeMap<String, EventCitation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub markdown: Option<String>,
}

pub fn get_recent_activity(
    ledger: &impl EventLedger,
    graph: &impl GraphView,
    request: &RecentActivityRequest,
) -> Result<QueryResponse<RecentActivityResults>> {
    let started = Instant::now();
    let events = read_history_events(ledger, FEED_EVENT_TYPES)?;
    let latest_offset = ledger.latest_offset()?;
    let captured = CapturedStandings::read_for_feed(graph, &events)?;
    let mut index = DecisionIndex::from_events(&events)?;
    index.note_captured(&captured);
    let standings = DecisionStandings::load(graph)?;
    let mut unmet = captured.by_origin();
    let filters = normalized_history_filters(&request.filters);
    let limit = normalized_history_limit(request.limit);
    let cursor = normalized_query(request.cursor.as_deref());
    let offset = parse_history_cursor(cursor.as_deref())?;

    let mut rows = Vec::new();
    for event in &events {
        let mut event_rows = vec![activity_row(event, &index)?];
        event_rows.extend(stale_premise_activity_row(event, &index)?);
        for payload in unmet.remove(&event_id(event)?).unwrap_or_default() {
            event_rows.push(activity_row_of(event, payload, &index)?);
            event_rows.extend(stale_premise_activity_row_of(event, payload, &index)?);
        }
        for row in event_rows {
            if matches_history_filters(
                &row.actor_id,
                row.source,
                row.source_ref.as_deref(),
                &row.decision_ids,
                &filters,
                &index,
                &standings,
            ) {
                rows.push(row);
            }
        }
    }
    refuse_unmet_standings(&unmet)?;
    rows.sort_by_key(|row| Reverse(row.event_origin));

    let total_matches = rows.len();
    let items: Vec<ActivityRow> = rows.into_iter().skip(offset).take(limit).collect();
    let next_offset = offset.saturating_add(items.len());
    let next_cursor = (next_offset < total_matches).then(|| next_offset.to_string());

    Ok(QueryResponse {
        result_count: items.len(),
        truncated: next_cursor.is_some(),
        latency_ms: started.elapsed().as_millis(),
        data: RecentActivityResults {
            filters,
            limit,
            cursor,
            next_cursor,
            total_matches,
            ledger_range: LedgerRange {
                from_offset_exclusive: 0,
                to_offset_inclusive: latest_offset,
            },
            items,
        },
    })
}

/// The decisions decided in the window, newest record first: the ones a `decision.proposed`
/// recorded, windowed by their own time (`ts`), and the ones a classifier drafted from a
/// transcript, which the graph holds and no proposal records, windowed by the time they were said
/// (`RecentDecisionEntry::decided_at`).
pub fn get_recent_decisions(
    ledger: &impl EventLedger,
    graph: &impl GraphView,
    request: &RecentDecisionsRequest,
) -> Result<QueryResponse<RecentDecisionsResults>> {
    let started = Instant::now();
    let events = read_history_events(ledger, DECISION_EVENT_TYPES)?;
    let latest_offset = ledger.latest_offset()?;
    let window = HistoryWindow::new(
        latest_offset,
        None,
        None,
        Some(request.since_timestamp),
        request.until_timestamp,
    );
    let index = DecisionIndex::from_events(&events)?;
    let decision_rows = node_rows(graph, NodeKind::Decision)?;
    let option_rows = node_rows(graph, NodeKind::Option)?;
    // Where a decision stands is the graph's word, which holds what a classified capture states
    // inside itself (accepted, rejected, replaces) as well as the status events.
    let standings = DecisionStandings::load(graph)?;
    let labels = ProjectLabels::from_events(&events);
    let filters = normalized_recent_decision_filters(&request.filters);
    let limit = normalized_history_limit(request.limit);
    let cursor = normalized_query(request.cursor.as_deref());
    let offset = parse_history_cursor(cursor.as_deref())?;

    let mut items = Vec::new();
    for event in &events {
        if !window.contains(event_id(event)?, event.ts) {
            continue;
        }

        let payload = events::validate(event).map_err(query_validation_error)?;
        let EventPayload::DecisionProposed(payload) = payload else {
            continue;
        };

        let decision_id = payload.decision_id.clone();
        let entry = index.decisions.get(&decision_id);
        let status = standings.status_of(&decision_id);
        let topic_keys = entry
            .map(|entry| entry.topic_keys.clone())
            .unwrap_or_else(|| normalized_string_values(payload.topic_keys.clone()));
        let actor_ids = entry
            .map(|entry| entry.actor_ids.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_else(|| vec![event.actor_id.clone()]);

        if !decision_matches_recent_filters(&topic_keys, status, &actor_ids, event.source, &filters)
        {
            continue;
        }

        let option_ids = entry
            .map(|entry| entry.option_ids.iter().cloned().collect::<Vec<_>>())
            .unwrap_or(payload.option_ids.clone());
        let evidence_ids = entry
            .map(|entry| entry.evidence_ids.iter().cloned().collect::<Vec<_>>())
            .unwrap_or(payload.evidence_ids.clone());
        let hypothesis_ids = entry
            .map(|entry| entry.hypothesis_ids.iter().cloned().collect::<Vec<_>>())
            .unwrap_or(payload.hypothesis_ids.clone());

        // The same rule the projector applies: where the latest `decision.moved` took the
        // decision, else the handle on the proposal, else (no handle, or an event that
        // predates the field) the recorder's personal project.
        let moved_to = entry.and_then(|entry| entry.moved_to.as_deref());
        let project = match (moved_to, payload.project) {
            (Some(moved_to), _) => moved_to.to_owned(), // ubs:ignore: entry owns its handle
            (None, Some(handle)) => handle,
            (None, None) => personal_project_handle(&event.actor_id),
        };
        // The same rule the projector applies to `title`: the latest `decision.retitled`
        // wins over the proposal's own title (hivemind-ydmp) — without this override,
        // `query recent` / `recent_decisions` stay stale forever after a retitle even
        // though recall/why/digest/search read straight off the projected node.
        let retitled_to = entry.and_then(|entry| entry.retitled_to.as_deref());
        let title = match retitled_to {
            Some(to) => to.to_owned(), // ubs:ignore: entry owns its handle
            None => payload.title,
        };
        items.push(RecentDecisionEntry {
            floor: floor_of(&decision_rows, &decision_id),
            decision_id,
            title,
            rationale: payload.rationale,
            quote: payload.quote,
            question: payload.question,
            status,
            project_label: labels.label(&project),
            project,
            topic_keys,
            actor_ids,
            decided_at: event.ts,
            creation: decision_event_provenance(event)?,
            chosen_option_id: payload.chosen_option_id,
            options: option_labels_of(&option_rows, &option_ids),
            evidence_ids,
            hypothesis_ids,
        });
    }
    for (decision_id, draft) in ClassifiedDrafts::load(graph, ledger, &decision_rows, &standings)? {
        if !draft.decided_between(Some(request.since_timestamp), request.until_timestamp)
            || !decision_matches_recent_filters(
                &draft.topic_keys,
                draft.status,
                &draft.actor_ids,
                draft.creation.source,
                &filters,
            )
        {
            continue;
        }
        let floor = floor_of(&decision_rows, &decision_id);
        let options = option_labels_of(&option_rows, &draft.option_ids);
        items.push(recent_entry_of_draft(
            decision_id,
            draft,
            &labels,
            floor,
            options,
        ));
    }
    items.sort_by(|left, right| {
        Reverse(left.creation.event_origin)
            .cmp(&Reverse(right.creation.event_origin))
            .then_with(|| left.decision_id.cmp(&right.decision_id))
    });

    let total_matches = items.len();
    let page: Vec<RecentDecisionEntry> = items.into_iter().skip(offset).take(limit).collect();
    let next_offset = offset.saturating_add(page.len());
    let next_cursor = (next_offset < total_matches).then(|| next_offset.to_string());

    Ok(QueryResponse {
        result_count: page.len(),
        truncated: next_cursor.is_some(),
        latency_ms: started.elapsed().as_millis(),
        data: RecentDecisionsResults {
            resolved_since: window.since(),
            resolved_until: window.until(),
            filters: filters.into_view(),
            limit,
            cursor,
            next_cursor,
            total_matches,
            ledger_range: window.ledger_range(),
            items: page,
        },
    })
}

/// The floor answers the decision's node states. A decision with no node, or a node without the
/// answers, reads as never asked.
fn floor_of(decisions: &BTreeMap<String, GraphRow>, decision_id: &str) -> RecordedFloor {
    decisions
        .get(decision_id)
        .map_or_else(RecordedFloor::not_asked, recorded_floor_of)
}

/// The labels of a decision's options, in the order its option ids are given.
fn option_labels_of(
    option_rows: &BTreeMap<String, GraphRow>,
    option_ids: &[String],
) -> Vec<OptionLabel> {
    option_ids
        .iter()
        .map(|option_id| option_label_of(option_id, option_rows.get(option_id)))
        .collect()
}

fn recent_entry_of_draft(
    decision_id: String,
    draft: ClassifiedDraft,
    labels: &ProjectLabels,
    floor: RecordedFloor,
    options: Vec<OptionLabel>,
) -> RecentDecisionEntry {
    RecentDecisionEntry {
        floor,
        decision_id,
        title: draft.title,
        rationale: draft.rationale,
        quote: draft.quote,
        question: draft.question,
        status: draft.status,
        project_label: labels.label(&draft.project),
        project: draft.project,
        topic_keys: draft.topic_keys,
        actor_ids: draft.actor_ids,
        decided_at: draft.decided_at,
        creation: draft.creation,
        chosen_option_id: draft.chosen_option_id,
        options,
        evidence_ids: draft.evidence_ids,
        hypothesis_ids: draft.hypothesis_ids,
    }
}

/// Every change row in the request's window that passes its filters, oldest event first. The
/// request's `limit` and `cursor` do not cut it: the pager and the id-set reader below each
/// decide what to keep of the whole.
fn changed_window_rows(
    ledger: &impl EventLedger,
    graph: &impl GraphView,
    request: &ChangedSinceRequest,
) -> Result<(HistoryWindow, HistoryFilters, Vec<DecisionChangeRow>)> {
    let events = read_history_events(ledger, FEED_EVENT_TYPES)?;
    let latest_offset = ledger.latest_offset()?;
    let window = HistoryWindow::new(
        latest_offset,
        request.since_offset,
        request.until_offset,
        request.since_timestamp,
        request.until_timestamp,
    );
    let captured = CapturedStandings::read_for_feed(graph, &events)?;
    let mut index = DecisionIndex::from_events(&events)?;
    index.note_captured(&captured);
    let standings = DecisionStandings::load(graph)?;
    let mut unmet = captured.by_origin();
    let filters = normalized_history_filters(&request.filters);

    let mut rows = Vec::new();
    for event in &events {
        let event_id = event_id(event)?;
        let stated = unmet.remove(&event_id).unwrap_or_default();
        if !window.contains(event_id, event.ts) {
            continue;
        }
        let mut event_rows = vec![decision_change_row(event, &index)?];
        event_rows.extend(stale_premise_change_row(event, &index)?);
        for payload in stated {
            event_rows.push(decision_change_row_of(event, payload, &index)?);
            event_rows.extend(stale_premise_change_row_of(event, payload, &index)?);
        }
        for row in event_rows {
            if matches_history_filters(
                &row.actor_id,
                row.source,
                row.source_ref.as_deref(),
                &row.decision_ids,
                &filters,
                &index,
                &standings,
            ) {
                rows.push(row);
            }
        }
    }
    refuse_unmet_standings(&unmet)?;
    rows.sort_by_key(|row| row.event_origin);
    Ok((window, filters, rows))
}

/// The ids of every decision the window changed, and the boundary it resolved to. One pass over
/// the window, not a page of it: a window of more than `MAX_QUERY_RESULTS` events, most of them
/// naming no decision, still reports each decision it changed.
pub fn get_changed_decision_ids_since(
    ledger: &impl EventLedger,
    graph: &impl GraphView,
    since_offset: Option<EventId>,
    since_timestamp: Option<DateTime<Utc>>,
) -> Result<(ChangeBoundary, BTreeSet<String>)> {
    let (window, _, rows) = changed_window_rows(
        ledger,
        graph,
        &ChangedSinceRequest {
            since_offset,
            since_timestamp,
            ..ChangedSinceRequest::default()
        },
    )?;
    let ids = rows.into_iter().flat_map(|row| row.decision_ids).collect();
    Ok((window.since(), ids))
}

pub fn get_decisions_changed_since(
    ledger: &impl EventLedger,
    graph: &impl GraphView,
    request: &ChangedSinceRequest,
) -> Result<QueryResponse<DecisionsChangedSinceResults>> {
    let started = Instant::now();
    let (window, filters, rows) = changed_window_rows(ledger, graph, request)?;
    let limit = normalized_history_limit(request.limit);
    let cursor = normalized_query(request.cursor.as_deref());
    let offset = parse_history_cursor(cursor.as_deref())?;

    let total_matches = rows.len();
    let items: Vec<DecisionChangeRow> = rows.into_iter().skip(offset).take(limit).collect();
    let next_offset = offset.saturating_add(items.len());
    let next_cursor = (next_offset < total_matches).then(|| next_offset.to_string());

    Ok(QueryResponse {
        result_count: items.len(),
        truncated: next_cursor.is_some(),
        latency_ms: started.elapsed().as_millis(),
        data: DecisionsChangedSinceResults {
            resolved_since: window.since(),
            resolved_until: window.until(),
            filters,
            limit,
            cursor,
            next_cursor,
            total_matches,
            ledger_range: window.ledger_range(),
            items,
        },
    })
}

/// The decisions added in the window, and the ones it changed. A decision a classifier drafted
/// from a transcript is added when the classification that recorded it falls inside the request's
/// offset bounds and the turn time it was said at (`AddedDecisionEntry::decided_at`) inside its
/// timestamp bounds; a proposal is windowed by its own time (`ts`) and the caller's offset bounds.
pub fn get_decisions_added_since(
    ledger: &impl EventLedger,
    graph: &impl GraphView,
    request: &DecisionsAddedSinceRequest,
) -> Result<QueryResponse<DecisionsAddedSinceResults>> {
    let started = Instant::now();
    let events = read_history_events(ledger, DECISION_EVENT_TYPES)?;
    let latest_offset = ledger.latest_offset()?;
    let window = HistoryWindow::new(
        latest_offset,
        request.since_offset,
        request.until_offset,
        request.since_timestamp,
        request.until_timestamp,
    );
    let captured = CapturedStandings::read(graph, &events)?;
    let mut index = DecisionIndex::from_events(&events)?;
    index.note_captured(&captured);
    let decision_rows = node_rows(graph, NodeKind::Decision)?;
    let standings = DecisionStandings::load(graph)?;
    let mut drafts = ClassifiedDrafts::load(graph, ledger, &decision_rows, &standings)?;
    let classifications = classification_events(ledger, &captured)?;
    let filters = normalized_added_since_filters(&request.filters);
    let limit = normalized_history_limit(request.limit);
    let cursor = normalized_query(request.cursor.as_deref());
    let offset = parse_history_cursor(cursor.as_deref())?;

    let mut creation_events: BTreeMap<String, &Event> = BTreeMap::new();
    // Each change names the event that recorded it and, for a standing a capture states inside its
    // classification event, the payload it stands for; the event's own payload otherwise.
    let mut per_decision_changes: BTreeMap<String, Vec<WindowChange>> = BTreeMap::new();

    for event in &events {
        let event_id = event_id(event)?;
        let payload = events::validate(event).map_err(query_validation_error)?;
        let in_window = window.contains(event_id, event.ts);
        if matches!(payload, EventPayload::DecisionProposed(_)) {
            for decision_id in decision_ids_for_payload(&payload, &index) {
                creation_events.entry(decision_id).or_insert(event);
            }
        }
        if !in_window {
            continue;
        }
        let change_kind = change_kind_for_payload(&payload);
        for decision_id in decision_ids_for_payload(&payload, &index) {
            per_decision_changes.entry(decision_id).or_default().push((
                event_id,
                event,
                change_kind,
                None,
            ));
        }
        for decision_id in stale_premise_dependents(&payload, &index, event_id) {
            per_decision_changes.entry(decision_id).or_default().push((
                event_id,
                event,
                HistoryChangeKind::StalePremise,
                None,
            ));
        }
    }
    for standing in &captured.standings {
        let event = classifications.get(&standing.origin).ok_or_else(|| {
            query_error(format!(
                "a capture's standing was recorded by event {}, which the ledger does not hold",
                standing.origin
            ))
        })?;
        if !window.contains(standing.origin, event.ts) {
            continue;
        }
        let payload = Some(&standing.payload);
        let change_kind = change_kind_for_payload(&standing.payload);
        for decision_id in decision_ids_for_payload(&standing.payload, &index) {
            per_decision_changes.entry(decision_id).or_default().push((
                standing.origin,
                event,
                change_kind,
                payload,
            ));
        }
        for decision_id in stale_premise_dependents(&standing.payload, &index, standing.origin) {
            per_decision_changes.entry(decision_id).or_default().push((
                standing.origin,
                event,
                HistoryChangeKind::StalePremise,
                payload,
            ));
        }
    }
    // Oldest first, as the events were read: the standings were met after them.
    for changes in per_decision_changes.values_mut() {
        changes.sort_by_key(|change| change.0);
    }

    let mut added: Vec<AddedDecisionEntry> = Vec::new();
    let mut changed: Vec<ChangedDecisionEntry> = Vec::new();

    for (decision_id, change_events) in &per_decision_changes {
        // A draft is known to the graph alone: the ledger's index holds nothing of it. It leaves
        // the set here, so the drafts the ledger never touched are the ones the loop below meets.
        let AddedFacts {
            topic_keys,
            status,
            option_ids,
            evidence_ids,
            hypothesis_ids,
            decided_at,
            created_in_window,
            creation,
        } = match drafts.take(decision_id) {
            Some(draft) => AddedFacts::of_draft(draft, request),
            None => {
                let entry = index.decisions.get(decision_id);
                let creation = creation_events
                    .get(decision_id)
                    .copied()
                    .map(decision_event_provenance)
                    .transpose()?;
                AddedFacts {
                    topic_keys: entry
                        .map(|entry| entry.topic_keys.clone())
                        .unwrap_or_default(),
                    status: standings.status_of(decision_id),
                    option_ids: entry
                        .map(|entry| entry.option_ids.iter().cloned().collect::<Vec<_>>())
                        .unwrap_or_default(),
                    evidence_ids: entry
                        .map(|entry| entry.evidence_ids.iter().cloned().collect::<Vec<_>>())
                        .unwrap_or_default(),
                    hypothesis_ids: entry
                        .map(|entry| entry.hypothesis_ids.iter().cloned().collect::<Vec<_>>())
                        .unwrap_or_default(),
                    decided_at: creation.as_ref().and_then(|prov| prov.ts),
                    created_in_window: creation
                        .as_ref()
                        .is_some_and(|prov| window.contains(prov.event_origin, prov.ts)),
                    creation,
                }
            }
        };

        let mut changes = Vec::with_capacity(change_events.len());
        for (event_id_value, event, change_kind, stated) in change_events {
            if created_in_window
                && creation
                    .as_ref()
                    .is_some_and(|prov| prov.event_origin == *event_id_value)
            {
                continue;
            }
            let validated;
            let payload = match stated {
                Some(payload) => payload,
                None => {
                    validated = events::validate(event).map_err(query_validation_error)?;
                    &validated
                }
            };
            let affected_nodes = affected_nodes_for_event(event, payload);
            let provenance = decision_event_provenance(event)?;
            changes.push(DecisionChange {
                change_kind: *change_kind,
                provenance,
                affected_nodes,
                project_move: project_move_for_payload(payload),
                title_change: title_change_for_payload(payload),
                reaffirmed_premise: reaffirmed_premise_for_payload(payload),
            });
        }

        let decision_matches_filters = decision_matches_added_since_filters(
            decision_id,
            &topic_keys,
            status,
            &filters,
            creation.as_ref(),
            &changes,
        );
        if !decision_matches_filters {
            continue;
        }

        match creation {
            Some(creation) if created_in_window => added.push(AddedDecisionEntry {
                floor: floor_of(&decision_rows, decision_id),
                decision_id: decision_id.clone(),
                status,
                topic_keys,
                decided_at,
                creation,
                option_ids,
                evidence_ids,
                hypothesis_ids,
                changes_in_window: changes,
            }),
            creation => changed.push(ChangedDecisionEntry {
                floor: floor_of(&decision_rows, decision_id),
                decision_id: decision_id.clone(),
                status,
                topic_keys,
                creation,
                option_ids,
                evidence_ids,
                hypothesis_ids,
                changes_in_window: changes,
            }),
        }
    }

    for (decision_id, draft) in drafts {
        if !draft_added_in_window(&draft, request)
            || !decision_matches_added_since_filters(
                &decision_id,
                &draft.topic_keys,
                draft.status,
                &filters,
                Some(&draft.creation),
                &[],
            )
        {
            continue;
        }
        added.push(AddedDecisionEntry {
            floor: floor_of(&decision_rows, &decision_id),
            decision_id,
            status: draft.status,
            topic_keys: draft.topic_keys,
            decided_at: draft.decided_at,
            creation: draft.creation,
            option_ids: draft.option_ids,
            evidence_ids: draft.evidence_ids,
            hypothesis_ids: draft.hypothesis_ids,
            changes_in_window: Vec::new(),
        });
    }

    added.sort_by(|left, right| {
        (left.creation.event_origin, &left.decision_id)
            .cmp(&(right.creation.event_origin, &right.decision_id))
    });
    changed.sort_by(|left, right| {
        let left_origin = changed_primary_origin(&left.changes_in_window);
        let right_origin = changed_primary_origin(&right.changes_in_window);
        (left_origin, &left.decision_id).cmp(&(right_origin, &right.decision_id))
    });

    let total_added = added.len();
    let total_changed = changed.len();
    let total_matches = total_added.saturating_add(total_changed);

    let mut combined: Vec<DiffEntry> = Vec::with_capacity(total_matches);
    for entry in added.into_iter() {
        combined.push(DiffEntry::Added(entry));
    }
    for entry in changed.into_iter() {
        combined.push(DiffEntry::Changed(entry));
    }

    let paginated: Vec<DiffEntry> = combined.into_iter().skip(offset).take(limit).collect();
    let next_offset = offset.saturating_add(paginated.len());
    let next_cursor = (next_offset < total_matches).then(|| next_offset.to_string());

    let mut added_page: Vec<AddedDecisionEntry> = Vec::new();
    let mut changed_page: Vec<ChangedDecisionEntry> = Vec::new();
    for entry in paginated {
        match entry {
            DiffEntry::Added(entry) => added_page.push(entry),
            DiffEntry::Changed(entry) => changed_page.push(entry),
        }
    }

    Ok(QueryResponse {
        result_count: added_page.len() + changed_page.len(),
        truncated: next_cursor.is_some(),
        latency_ms: started.elapsed().as_millis(),
        data: DecisionsAddedSinceResults {
            resolved_since: window.since(),
            resolved_until: window.until(),
            filters: filters.into_view(),
            limit,
            cursor,
            next_cursor,
            total_matches,
            total_added,
            total_changed_existing: total_changed,
            ledger_range: window.ledger_range(),
            added_decisions: added_page,
            changed_existing_decisions: changed_page,
        },
    })
}

/// One change in the window to one decision: its ledger offset, the event that recorded it, how it
/// changed the decision, and the payload it stands for when that is not the event's own.
type WindowChange<'a> = (
    EventId,
    &'a Event,
    HistoryChangeKind,
    Option<&'a EventPayload>,
);

/// The classification events that recorded the standings `captured` holds, by offset. The decision
/// lists read no classification event themselves, so they are asked for by id.
fn classification_events(
    ledger: &impl EventLedger,
    captured: &CapturedStandings,
) -> Result<HashMap<EventId, Event>> {
    let origins: Vec<EventId> = captured
        .standings
        .iter()
        .map(|standing| standing.origin)
        .collect();
    if origins.is_empty() {
        return Ok(HashMap::new());
    }
    Ok(ledger
        .read_ids_for_tenant(&ledger.bound_tenant(), &origins)?
        .into_iter()
        .filter_map(|event| Some((event.event_id?, event)))
        .collect())
}

/// What an added-since entry says of one decision, whether the ledger's proposal or the graph's
/// draft is where it was read from.
struct AddedFacts {
    topic_keys: Vec<String>,
    status: DecisionStatus,
    option_ids: Vec<String>,
    evidence_ids: Vec<String>,
    hypothesis_ids: Vec<String>,
    decided_at: Option<DateTime<Utc>>,
    /// Whether the decision was added, not only changed, in the request's window.
    created_in_window: bool,
    creation: Option<DecisionEventProvenance>,
}

impl AddedFacts {
    fn of_draft(draft: ClassifiedDraft, request: &DecisionsAddedSinceRequest) -> Self {
        Self {
            created_in_window: draft_added_in_window(&draft, request),
            topic_keys: draft.topic_keys,
            status: draft.status,
            option_ids: draft.option_ids,
            evidence_ids: draft.evidence_ids,
            hypothesis_ids: draft.hypothesis_ids,
            decided_at: draft.decided_at,
            creation: Some(draft.creation),
        }
    }
}

/// Whether a classifier draft is added in the request's window: the classification that recorded
/// it inside the offset bounds, as for any decision, and the time it was said inside the timestamp
/// bounds, because the classification can come days after the conversation.
fn draft_added_in_window(draft: &ClassifiedDraft, request: &DecisionsAddedSinceRequest) -> bool {
    let recorded_at = draft.creation.event_origin;
    recorded_at > request.since_offset.unwrap_or_default()
        && request
            .until_offset
            .is_none_or(|until_offset| recorded_at <= until_offset)
        && draft.decided_between(request.since_timestamp, request.until_timestamp)
}

enum DiffEntry {
    Added(AddedDecisionEntry),
    Changed(ChangedDecisionEntry),
}

fn changed_primary_origin(changes: &[DecisionChange]) -> EventId {
    changes
        .iter()
        .map(|change| change.provenance.event_origin)
        .min()
        .unwrap_or_default()
}

#[derive(Clone, Debug, Default)]
struct NormalizedRecentDecisionFilters {
    actor_patterns: Vec<String>,
    sources: Vec<String>,
    topic_keys: Vec<String>,
    statuses: Vec<DecisionStatus>,
}

impl NormalizedRecentDecisionFilters {
    fn into_view(self) -> RecentDecisionFilters {
        RecentDecisionFilters {
            actor_patterns: self.actor_patterns,
            sources: self.sources,
            topic_keys: self.topic_keys,
            statuses: self.statuses,
        }
    }
}

fn normalized_recent_decision_filters(
    request: &RecentDecisionFilterRequest,
) -> NormalizedRecentDecisionFilters {
    NormalizedRecentDecisionFilters {
        actor_patterns: normalized_string_values(request.actor_patterns.clone()),
        sources: normalized_string_values(request.sources.clone()),
        topic_keys: normalized_topic_filter_values(&request.topic_keys),
        statuses: request
            .statuses
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
    }
}

fn decision_matches_recent_filters(
    topic_keys: &[String],
    status: DecisionStatus,
    actor_ids: &[String],
    source: EventSource,
    filters: &NormalizedRecentDecisionFilters,
) -> bool {
    if !filters.statuses.is_empty() && !filters.statuses.contains(&status) {
        return false;
    }
    if !filters.topic_keys.is_empty()
        && !filters
            .topic_keys
            .iter()
            .all(|topic| stored_topics_include(topic_keys, topic))
    {
        return false;
    }
    if !filters.actor_patterns.is_empty()
        && !filters.actor_patterns.iter().any(|pattern| {
            actor_ids
                .iter()
                .any(|actor_id| glob_matches(pattern, actor_id))
        })
    {
        return false;
    }
    if !filters.sources.is_empty()
        && !filters
            .sources
            .iter()
            .any(|expected| source.as_str().eq_ignore_ascii_case(expected))
    {
        return false;
    }

    true
}

fn glob_matches(pattern: &str, value: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if !pattern.contains('*') {
        return pattern == value;
    }

    let parts: Vec<&str> = pattern.split('*').filter(|part| !part.is_empty()).collect();
    if parts.is_empty() {
        return true;
    }

    let mut position = 0;
    let mut part_index = 0;
    if !pattern.starts_with('*') {
        let first = parts[0];
        if !value.starts_with(first) {
            return false;
        }
        position = first.len();
        part_index = 1;
    }

    for part in &parts[part_index..] {
        let Some(found_at) = value[position..].find(part) else {
            return false;
        };
        position += found_at + part.len();
    }

    pattern.ends_with('*') || value.ends_with(parts.last().copied().unwrap_or_default())
}

#[derive(Clone, Debug, Default)]
struct NormalizedAddedSinceFilters {
    actor_ids: Vec<String>,
    sources: Vec<String>,
    source_refs: Vec<String>,
    import_run_ids: Vec<String>,
    topic_keys: Vec<String>,
    statuses: Vec<DecisionStatus>,
}

impl NormalizedAddedSinceFilters {
    fn into_view(self) -> DecisionsAddedSinceFilters {
        DecisionsAddedSinceFilters {
            actor_ids: self.actor_ids,
            sources: self.sources,
            source_refs: self.source_refs,
            import_run_ids: self.import_run_ids,
            topic_keys: self.topic_keys,
            statuses: self.statuses,
        }
    }
}

fn normalized_added_since_filters(
    request: &DecisionsAddedSinceFilterRequest,
) -> NormalizedAddedSinceFilters {
    NormalizedAddedSinceFilters {
        actor_ids: normalized_string_values(request.actor_ids.clone()),
        sources: normalized_string_values(request.sources.clone()),
        source_refs: normalized_string_values(request.source_refs.clone()),
        import_run_ids: normalized_string_values(request.import_run_ids.clone()),
        topic_keys: normalized_topic_filter_values(&request.topic_keys),
        statuses: request
            .statuses
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
    }
}

fn decision_matches_added_since_filters(
    _decision_id: &str,
    topic_keys: &[String],
    status: DecisionStatus,
    filters: &NormalizedAddedSinceFilters,
    creation: Option<&DecisionEventProvenance>,
    changes: &[DecisionChange],
) -> bool {
    if !filters.statuses.is_empty() && !filters.statuses.contains(&status) {
        return false;
    }
    if !filters.topic_keys.is_empty()
        && !filters
            .topic_keys
            .iter()
            .all(|topic| stored_topics_include(topic_keys, topic))
    {
        return false;
    }

    let mut provenances: Vec<&DecisionEventProvenance> =
        changes.iter().map(|change| &change.provenance).collect();
    if let Some(creation) = creation {
        provenances.push(creation);
    }

    if !filters.actor_ids.is_empty()
        && !provenances
            .iter()
            .any(|prov| filters.actor_ids.iter().any(|id| id == &prov.actor_id))
    {
        return false;
    }
    if !filters.sources.is_empty()
        && !provenances.iter().any(|prov| {
            filters
                .sources
                .iter()
                .any(|expected| prov.source.as_str().eq_ignore_ascii_case(expected))
        })
    {
        return false;
    }
    if !filters.source_refs.is_empty()
        && !provenances.iter().any(|prov| {
            prov.source_ref.as_deref().is_some_and(|source_ref| {
                filters
                    .source_refs
                    .iter()
                    .any(|expected| expected == source_ref)
            })
        })
    {
        return false;
    }
    if !filters.import_run_ids.is_empty()
        && !provenances.iter().any(|prov| {
            prov.import_run_id.as_deref().is_some_and(|run_id| {
                filters
                    .import_run_ids
                    .iter()
                    .any(|expected| expected == run_id)
            })
        })
    {
        return false;
    }

    true
}

fn decision_event_provenance(event: &Event) -> Result<DecisionEventProvenance> {
    let event_id = event_id(event)?;
    Ok(DecisionEventProvenance {
        event_origin: event_id,
        event_uuid: event.event_uuid.to_string(),
        event_type: event.event_type,
        actor_id: event.actor_id.clone(),
        source: event.source,
        source_ref: event.source_ref.clone(),
        import_run_id: extract_import_run_id(event.source_ref.as_deref()),
        ts: event.ts,
        citation_id: citation_id(event_id),
    })
}

fn extract_import_run_id(source_ref: Option<&str>) -> Option<String> {
    let raw = source_ref?;
    let value: Value = serde_json::from_str(raw).ok()?;
    value
        .get("import_run_id")
        .and_then(|value| value.as_str())
        .map(str::to_owned)
}

/// The window a history request cuts the ledger to. Offset bounds bound it only when the caller
/// gave them; a timestamp bound is tested against each event's own `ts`, never turned into an
/// offset, because event time does not rise with ledger offset: an importer or a classified
/// transcript answer is dated at its source's time, so an event appended now can carry a `ts`
/// from last week. Cutting at "the last offset dated before the bound" would drop every
/// proposal appended before such an event.
#[derive(Clone, Copy, Debug)]
struct HistoryWindow {
    /// Exclusive; 0 when the caller gave no offset.
    since_offset: EventId,
    /// Inclusive; the ledger's latest offset when the caller gave no offset.
    until_offset: EventId,
    /// Exclusive.
    since_timestamp: Option<DateTime<Utc>>,
    /// Inclusive.
    until_timestamp: Option<DateTime<Utc>>,
}

impl HistoryWindow {
    fn new(
        latest_offset: EventId,
        since_offset: Option<EventId>,
        until_offset: Option<EventId>,
        since_timestamp: Option<DateTime<Utc>>,
        until_timestamp: Option<DateTime<Utc>>,
    ) -> Self {
        Self {
            since_offset: since_offset.unwrap_or_default(),
            until_offset: until_offset.unwrap_or(latest_offset),
            since_timestamp,
            until_timestamp,
        }
    }

    /// Whether the event at `offset`, dated `ts`, is in the window: inside the offset bounds and
    /// after `since_timestamp` and no later than `until_timestamp`, the same half-open cut
    /// `ClassifiedDraft::decided_between` makes. With a timestamp bound, an event with no `ts`
    /// is not in the window.
    fn contains(&self, offset: EventId, ts: Option<DateTime<Utc>>) -> bool {
        if offset <= self.since_offset || offset > self.until_offset {
            return false;
        }
        if self.since_timestamp.is_none() && self.until_timestamp.is_none() {
            return true;
        }
        let Some(ts) = ts else {
            return false;
        };
        self.since_timestamp.is_none_or(|since| ts > since)
            && self.until_timestamp.is_none_or(|until| ts <= until)
    }

    fn since(&self) -> ChangeBoundary {
        ChangeBoundary {
            offset: self.since_offset,
            timestamp: self.since_timestamp,
        }
    }

    fn until(&self) -> ChangeBoundary {
        ChangeBoundary {
            offset: self.until_offset,
            timestamp: self.until_timestamp,
        }
    }

    fn ledger_range(&self) -> LedgerRange {
        LedgerRange {
            from_offset_exclusive: self.since_offset,
            to_offset_inclusive: self.until_offset,
        }
    }
}

pub fn export_read_only_summary(
    ledger: &impl EventLedger,
    graph: &impl GraphView,
    request: &ReadOnlyExportRequest,
) -> Result<QueryResponse<ReadOnlyExport>> {
    let started = Instant::now();

    let export = match &request.query {
        ReadOnlyExportQuery::RecentActivity(activity_request) => {
            let response = get_recent_activity(ledger, graph, activity_request)?;
            let query_params = recent_activity_query_params(&response.data);
            let citation_map = activity_citation_map(&response.data.items);
            let json_body = serde_json::to_value(&response.data).map_err(json_query_error)?;
            let markdown = render_recent_activity_markdown(&response.data, request.generated_at);
            export_from_parts(
                ReadOnlyExportQueryKind::RecentActivity,
                request.format,
                query_params,
                response.data.ledger_range.clone(),
                request.generated_at,
                response.result_count,
                response.truncated,
                response.data.next_cursor.clone(),
                citation_map,
                json_body,
                markdown,
            )
        }
        ReadOnlyExportQuery::DecisionsChangedSince(changed_request) => {
            let response = get_decisions_changed_since(ledger, graph, changed_request)?;
            let query_params = changed_since_query_params(&response.data);
            let citation_map = change_citation_map(&response.data.items);
            let json_body = serde_json::to_value(&response.data).map_err(json_query_error)?;
            let markdown = render_changed_since_markdown(&response.data, request.generated_at);
            export_from_parts(
                ReadOnlyExportQueryKind::DecisionsChangedSince,
                request.format,
                query_params,
                response.data.ledger_range.clone(),
                request.generated_at,
                response.result_count,
                response.truncated,
                response.data.next_cursor.clone(),
                citation_map,
                json_body,
                markdown,
            )
        }
    };

    Ok(QueryResponse {
        result_count: export.result_count,
        truncated: export.truncated,
        latency_ms: started.elapsed().as_millis(),
        data: export,
    })
}

#[allow(clippy::too_many_arguments)]
fn export_from_parts(
    query: ReadOnlyExportQueryKind,
    format: ReadOnlyExportFormat,
    query_params: BTreeMap<String, Value>,
    ledger_range: LedgerRange,
    generated_at: DateTime<Utc>,
    result_count: usize,
    truncated: bool,
    continuation_cursor: Option<String>,
    citation_map: BTreeMap<String, EventCitation>,
    json_body: Value,
    markdown: String,
) -> ReadOnlyExport {
    let (json, markdown) = match format {
        ReadOnlyExportFormat::Json => (Some(json_body), None),
        ReadOnlyExportFormat::Markdown => (None, Some(markdown)),
    };

    ReadOnlyExport {
        query,
        format,
        query_params,
        ledger_range,
        generated_at,
        result_count,
        truncated,
        continuation_cursor,
        citation_map,
        json,
        markdown,
    }
}

/// The event types the feeds (`get_recent_activity`, `get_decisions_changed_since`) list a row
/// for: every type but `ingest.batch_received`. A received batch is a raw transcript. It names no
/// decision and changes nothing in the graph, and a cell holds them by the hundred thousand, so
/// reading them to list them made a feed cost what the transcripts weigh, not what the decisions
/// do (hivemind-gxypz). The classification that records what a batch decided is listed.
const FEED_EVENT_TYPES: &[EventType] = &[
    EventType::DecisionProposed,
    EventType::DecisionRequested,
    EventType::DecisionAccepted,
    EventType::DecisionRejected,
    EventType::DecisionSuperseded,
    EventType::EvidenceRecorded,
    EventType::HypothesisRecorded,
    EventType::QuestionRecorded,
    EventType::QuestionAsked,
    EventType::RelationAdded,
    EventType::RelationRemoved,
    EventType::BlockerReported,
    EventType::BlockerResolved,
    EventType::NotificationSent,
    EventType::NotificationAcknowledged,
    EventType::SuggestionSurfaced,
    EventType::IngestBatchClassified,
    EventType::DecisionScored,
    EventType::DecisionMetadataDerived,
    EventType::DecisionMoved,
    EventType::DecisionRetitled,
    EventType::DecisionReaffirmed,
    EventType::ProjectRegistered,
    EventType::ProjectLinked,
    EventType::ProjectUnlinked,
    EventType::ProjectAnchored,
    EventType::ProjectUnanchored,
    EventType::ProjectTopicDeclared,
];

/// The event types the decision lists (`get_recent_decisions`, `get_decisions_added_since`) read:
/// the events that carry or change a decision (`DecisionIndex`, `decision_ids_for_payload`) and
/// the project registrations that name a project. Nothing else changes what they answer.
const DECISION_EVENT_TYPES: &[EventType] = &[
    EventType::DecisionProposed,
    EventType::DecisionRequested,
    EventType::DecisionAccepted,
    EventType::DecisionRejected,
    EventType::DecisionSuperseded,
    EventType::RelationAdded,
    EventType::BlockerReported,
    EventType::DecisionMoved,
    EventType::DecisionRetitled,
    EventType::DecisionReaffirmed,
    EventType::ProjectRegistered,
];

/// Every event of `types` in ledger order. An annotation row nobody can parse is skipped, not a
/// reason to refuse every history read; the caller that wants to tell the reader scans for it
/// (`read_notice`).
fn read_history_events(ledger: &impl EventLedger, types: &[EventType]) -> Result<Vec<Event>> {
    let mut events = ledger.read_all_types_for_tenant(&ledger.bound_tenant(), types)?;
    events.retain(|event| !events::is_unreadable_annotation(event));
    Ok(events)
}

/// What classified captures state about where decisions stand, for the feeds to list
/// (hivemind-pf6xi). A capture's `rejected_by` and `supersedes_id` are edges of the graph and no
/// `decision.rejected` or `decision.superseded` of the ledger, so the events a feed lists never
/// said them, and a replaced decision stayed unchanged in it. Each standing is listed as the event
/// it stands for, cited by the classification event that recorded it, as a hand-recorded one is.
struct CapturedStandings {
    standings: Vec<CapturedStanding>,
    /// The topics of the decisions the standings name that a capture recorded, which the graph's
    /// node holds and no proposal does.
    topic_keys: BTreeMap<String, Vec<String>>,
}

impl CapturedStandings {
    /// The standings of `graph` that none of the `decision.rejected` and `decision.superseded`
    /// among `events` states.
    fn read(graph: &impl GraphView, events: &[Event]) -> Result<Self> {
        Self::of(graph, captured_standings(graph, events)?)
    }

    /// [`Self::read`], and the decisions captures record as the `decision.proposed` each stands
    /// for (hivemind-svqxz), ahead of what the same classification states of them. A feed lists a
    /// decision as new when it first appears, and a capture's decision has no proposal to say so.
    /// The decision lists leave them out: they list a draft as added from the graph's own record.
    fn read_for_feed(graph: &impl GraphView, events: &[Event]) -> Result<Self> {
        let mut standings = captured_creations(graph)?;
        standings.extend(captured_standings(graph, events)?);
        Self::of(graph, standings)
    }

    fn of(graph: &impl GraphView, standings: Vec<CapturedStanding>) -> Result<Self> {
        let mut topic_keys = BTreeMap::new();
        for standing in &standings {
            let named = match &standing.payload {
                EventPayload::DecisionProposed(proposed) => {
                    // The recorded decision's own topics; no later standing names it again.
                    topic_keys.insert(proposed.decision_id.clone(), proposed.topic_keys.clone()); // ubs:ignore: the map owns its ids
                    continue;
                }
                EventPayload::DecisionRejected(rejected) => &rejected.decision_id,
                EventPayload::DecisionSuperseded(superseded) => &superseded.new_decision_id,
                _ => continue,
            };
            if !is_classified_capture(named) || topic_keys.contains_key(named) {
                continue;
            }
            if let Some(row) = node_row(graph, NodeKind::Decision, named)? {
                let keys = optional_string_list(&row, "topic_keys");
                topic_keys.insert(named.clone(), keys); // ubs:ignore: the map owns its ids
            }
        }
        Ok(Self {
            standings,
            topic_keys,
        })
    }

    /// The payloads, by the offset of the classification event that recorded them. A feed takes
    /// each away as it meets the event, so what is left names an event it never met.
    fn by_origin(&self) -> BTreeMap<EventId, Vec<&EventPayload>> {
        let mut by_origin: BTreeMap<EventId, Vec<&EventPayload>> = BTreeMap::new();
        for standing in &self.standings {
            by_origin
                .entry(standing.origin)
                .or_default()
                .push(&standing.payload);
        }
        by_origin
    }
}

/// Refuses the standings a feed did not meet the event of: a window that quietly lost one would
/// look complete.
fn refuse_unmet_standings(unmet: &BTreeMap<EventId, Vec<&EventPayload>>) -> Result<()> {
    match unmet.keys().next() {
        None => Ok(()),
        Some(origin) => Err(query_error(format!(
            "a capture's standing was recorded by event {origin}, which the ledger does not hold"
        ))
        .into()),
    }
}

#[derive(Clone, Debug, Default)]
struct DecisionIndex {
    decisions: BTreeMap<String, DecisionIndexEntry>,
    assumed_by_hypothesis: BTreeMap<String, BTreeSet<String>>,
    option_to_decision: BTreeMap<String, String>,
    /// Premise decision id -> the decisions that follow from it (`FOLLOWS_FROM`), each with
    /// the ledger offset of the event that first linked it.
    followers_of: BTreeMap<String, BTreeMap<String, EventId>>,
}

#[derive(Clone, Debug, Default)]
struct DecisionIndexEntry {
    topic_keys: Vec<String>,
    actor_ids: BTreeSet<String>,
    superseded_by: BTreeSet<String>,
    option_ids: BTreeSet<String>,
    evidence_ids: BTreeSet<String>,
    hypothesis_ids: BTreeSet<String>,
    /// Where the latest `decision.moved` took the decision; absent when it never moved.
    moved_to: Option<String>,
    /// The title the latest `decision.retitled` gave the decision; absent when it was never
    /// retitled (same pattern as `moved_to`; hivemind-ydmp).
    retitled_to: Option<String>,
}

impl DecisionIndex {
    fn from_events(events: &[Event]) -> Result<Self> {
        let mut index = Self::default();
        for event in events {
            match events::validate(event).map_err(query_validation_error)? {
                EventPayload::DecisionProposed(payload) => {
                    let entry = index
                        .decisions
                        .entry(payload.decision_id.clone())
                        .or_default();
                    entry.topic_keys = normalized_string_values(payload.topic_keys);
                    entry.actor_ids.insert(event.actor_id.clone());
                    entry.option_ids.extend(payload.option_ids);
                    entry.evidence_ids.extend(payload.evidence_ids);
                    entry
                        .hypothesis_ids
                        .extend(payload.hypothesis_ids.iter().cloned());
                    for hypothesis_id in payload.hypothesis_ids {
                        index
                            .assumed_by_hypothesis
                            .entry(hypothesis_id)
                            .or_default()
                            .insert(payload.decision_id.clone());
                    }
                }
                EventPayload::DecisionRequested(payload) => {
                    if let Some(decision_id) = payload.decision_id {
                        index.decisions.entry(decision_id).or_default().topic_keys =
                            normalized_string_values(payload.topic_keys);
                    }
                }
                EventPayload::BlockerReported(payload) => {
                    if let Some(decision_id) = payload.decision_id {
                        index.decisions.entry(decision_id).or_default().topic_keys =
                            normalized_string_values(payload.topic_keys);
                    }
                }
                EventPayload::DecisionAccepted(payload) => {
                    let entry = index.decisions.entry(payload.decision_id).or_default();
                    entry.actor_ids.insert(event.actor_id.clone());
                }
                EventPayload::DecisionRejected(payload) => {
                    let entry = index.decisions.entry(payload.decision_id).or_default();
                    entry.actor_ids.insert(event.actor_id.clone());
                }
                EventPayload::DecisionSuperseded(payload) => {
                    index
                        .decisions
                        .entry(payload.old_decision_id)
                        .or_default()
                        .superseded_by
                        .insert(payload.new_decision_id);
                }
                EventPayload::RelationAdded(payload) => match payload.relation {
                    EventRelationKind::Assumes => {
                        let decision_id = index
                            .option_to_decision
                            .get(&payload.from_id)
                            .cloned()
                            .unwrap_or_else(|| payload.from_id.clone()); // ubs:ignore:
                        index
                            .decisions
                            .entry(decision_id.clone()) // ubs:ignore:
                            .or_default()
                            .hypothesis_ids
                            .insert(payload.to_id.clone()); // ubs:ignore:
                        index
                            .assumed_by_hypothesis
                            .entry(payload.to_id)
                            .or_default()
                            .insert(decision_id);
                    }
                    EventRelationKind::BasedOn => {
                        index
                            .decisions
                            .entry(payload.from_id)
                            .or_default()
                            .evidence_ids
                            .insert(payload.to_id);
                    }
                    EventRelationKind::HasOption => {
                        index
                            .decisions
                            .entry(payload.from_id)
                            .or_default()
                            .option_ids
                            .insert(payload.to_id);
                    }
                    EventRelationKind::Chose => {
                        index
                            .decisions
                            .entry(payload.from_id.clone()) // ubs:ignore:
                            .or_default()
                            .option_ids
                            .insert(payload.to_id.clone()); // ubs:ignore:
                        index
                            .option_to_decision
                            .insert(payload.to_id, payload.from_id);
                    }
                    EventRelationKind::FollowsFrom => {
                        // Like the graph projection, a later relation.removed does not retract
                        // this: retraction is recorded in the ledger only.
                        index
                            .followers_of
                            .entry(payload.to_id)
                            .or_default()
                            .entry(payload.from_id)
                            .or_insert_with(|| event.event_id.unwrap_or_default());
                    }
                    EventRelationKind::Supports
                    | EventRelationKind::Refutes
                    | EventRelationKind::SameAs
                    | EventRelationKind::Answers => {}
                },
                EventPayload::RelationRemoved(_)
                | EventPayload::EvidenceRecorded(_)
                | EventPayload::HypothesisRecorded(_)
                | EventPayload::QuestionRecorded(_)
                | EventPayload::QuestionAsked(_)
                | EventPayload::BlockerResolved(_)
                | EventPayload::NotificationSent(_)
                | EventPayload::NotificationAcknowledged(_)
                | EventPayload::SuggestionSurfaced(_)
                | EventPayload::IngestBatchReceived(_)
                | EventPayload::IngestBatchClassified(_)
                | EventPayload::DecisionScored(_)
                | EventPayload::DecisionAssessed(_)
                | EventPayload::DecisionMetadataDerived(_)
                | EventPayload::ProjectRegistered(_)
                | EventPayload::ProjectLinked(_)
                | EventPayload::ProjectUnlinked(_)
                | EventPayload::ProjectAnchored(_)
                | EventPayload::ProjectUnanchored(_)
                | EventPayload::ProjectTopicDeclared(_)
                | EventPayload::DecisionReaffirmed(_) => {}
                EventPayload::DecisionMoved(payload) => {
                    index
                        .decisions
                        .entry(payload.decision_id)
                        .or_default()
                        .moved_to = Some(payload.to);
                }
                EventPayload::DecisionRetitled(payload) => {
                    index
                        .decisions
                        .entry(payload.decision_id)
                        .or_default()
                        .retitled_to = Some(payload.to);
                }
            }
        }
        Ok(index)
    }

    /// Folds in what classified captures state: a decision a capture replaces was superseded by
    /// the capturing decision, which matters to who else replaced it (`solely_supersedes`), and a
    /// decision a capture records answers the topics its node names, since no proposal carries
    /// them (hivemind-pf6xi).
    fn note_captured(&mut self, captured: &CapturedStandings) {
        for standing in &captured.standings {
            if let EventPayload::DecisionSuperseded(payload) = &standing.payload {
                self.decisions
                    .entry(payload.old_decision_id.clone()) // ubs:ignore: the index owns its ids
                    .or_default()
                    .superseded_by
                    .insert(payload.new_decision_id.clone()); // ubs:ignore: the index owns its ids
            }
        }
        for (decision_id, topic_keys) in &captured.topic_keys {
            self.decisions
                .entry(decision_id.clone()) // ubs:ignore: the index owns its ids
                .or_default()
                .topic_keys
                .clone_from(topic_keys);
        }
    }

    fn topic_matches(&self, decision_id: &str, topic_keys: &[String]) -> bool {
        topic_keys.is_empty()
            || self.decisions.get(decision_id).is_some_and(|entry| {
                topic_keys
                    .iter()
                    .all(|topic| stored_topics_include(&entry.topic_keys, topic))
            })
    }

    fn decisions_assuming(&self, hypothesis_id: &str) -> Vec<String> {
        self.assumed_by_hypothesis
            .get(hypothesis_id)
            .map(|ids| ids.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Whether `decision_id` is the only decision to have superseded `premise_id`, over the whole
    /// ledger (as the re-examine items read it).
    fn solely_supersedes(&self, decision_id: &str, premise_id: &str) -> bool {
        self.decisions
            .get(premise_id)
            .is_some_and(|entry| reexamine::solely_supersedes(&entry.superseded_by, decision_id))
    }

    /// The decisions that followed from `premise_decision_id` before ledger offset `before`.
    fn decisions_following(&self, premise_decision_id: &str, before: EventId) -> Vec<String> {
        self.followers_of
            .get(premise_decision_id)
            .map(|followers| {
                followers
                    .iter()
                    .filter(|(_, linked_at)| **linked_at < before)
                    .map(|(follower, _)| follower.clone())
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// The decisions that already followed from a prior decision when this event (at ledger offset
/// `event_offset`) superseded or rejected it: their premise no longer stands (superseded), or
/// is in doubt (rejected). Empty for every other event. The event's own row is unchanged; these
/// decisions get a `StalePremise` row of their own, so a premise going stale is never silent on
/// the decisions that rested on it. A decision that only started following the premise later
/// was stale from birth; its link is its own `ContextChange` row.
///
/// The decision that follows its premise and is the only one to supersede it is the change
/// itself, not a dependent of it, so it is not listed (the rule re-examine items follow, see
/// `PremiseChange::is_made_by_alone`). One of several superseders stays listed: it is told about
/// the others.
fn stale_premise_dependents(
    payload: &EventPayload,
    index: &DecisionIndex,
    event_offset: EventId,
) -> Vec<String> {
    match payload {
        EventPayload::DecisionSuperseded(DecisionSupersededPayload {
            old_decision_id,
            new_decision_id,
            ..
        }) => {
            let mut dependents = index.decisions_following(old_decision_id, event_offset);
            if index.solely_supersedes(new_decision_id, old_decision_id) {
                dependents.retain(|dependent| dependent != new_decision_id);
            }
            dependents
        }
        EventPayload::DecisionRejected(DecisionRejectedPayload { decision_id, .. }) => {
            index.decisions_following(decision_id, event_offset)
        }
        _ => Vec::new(),
    }
}

fn activity_row(event: &Event, index: &DecisionIndex) -> Result<ActivityRow> {
    let payload = events::validate(event).map_err(query_validation_error)?;
    activity_row_of(event, &payload, index)
}

/// The row `payload` makes, cited by `event`: the event's own payload, or the standing a
/// classified capture states inside it (`CapturedStandings`).
fn activity_row_of(
    event: &Event,
    payload: &EventPayload,
    index: &DecisionIndex,
) -> Result<ActivityRow> {
    let event_id = event_id(event)?;
    let change_kind = change_kind_for_payload(payload);
    let decision_ids = decision_ids_for_payload(payload, index);
    let affected_nodes = affected_nodes_for_event(event, payload);
    let citation = citation_for_event(event)?;

    Ok(ActivityRow {
        event_origin: event_id,
        event_uuid: event.event_uuid.to_string(),
        event_type: event.event_type,
        change_kind,
        actor_id: event.actor_id.clone(),
        source: event.source,
        source_ref: event.source_ref.clone(),
        ts: event.ts,
        decision_ids,
        affected_nodes,
        project_move: project_move_for_payload(payload),
        title_change: title_change_for_payload(payload),
        reaffirmed_premise: reaffirmed_premise_for_payload(payload),
        citation_id: citation.citation_id,
    })
}

fn decision_change_row(event: &Event, index: &DecisionIndex) -> Result<DecisionChangeRow> {
    let payload = events::validate(event).map_err(query_validation_error)?;
    decision_change_row_of(event, &payload, index)
}

/// `activity_row_of` for the changed-since listing.
fn decision_change_row_of(
    event: &Event,
    payload: &EventPayload,
    index: &DecisionIndex,
) -> Result<DecisionChangeRow> {
    let event_id = event_id(event)?;
    let change_kind = change_kind_for_payload(payload);
    let decision_ids = decision_ids_for_payload(payload, index);
    let affected_nodes = affected_nodes_for_event(event, payload);
    let citation = citation_for_event(event)?;

    Ok(DecisionChangeRow {
        event_origin: event_id,
        event_uuid: event.event_uuid.to_string(),
        event_type: event.event_type,
        change_kind,
        actor_id: event.actor_id.clone(),
        source: event.source,
        source_ref: event.source_ref.clone(),
        ts: event.ts,
        decision_ids,
        affected_nodes,
        project_move: project_move_for_payload(payload),
        title_change: title_change_for_payload(payload),
        reaffirmed_premise: reaffirmed_premise_for_payload(payload),
        citation_id: citation.citation_id,
    })
}

/// The second row a premise's supersession/rejection produces: the same event seen from the
/// decisions that follow from the premise. `None` when nothing follows from it.
fn stale_premise_activity_row(event: &Event, index: &DecisionIndex) -> Result<Option<ActivityRow>> {
    let payload = events::validate(event).map_err(query_validation_error)?;
    stale_premise_activity_row_of(event, &payload, index)
}

fn stale_premise_activity_row_of(
    event: &Event,
    payload: &EventPayload,
    index: &DecisionIndex,
) -> Result<Option<ActivityRow>> {
    let dependents = stale_premise_dependents(payload, index, event_id(event)?);
    if dependents.is_empty() {
        return Ok(None);
    }
    let mut row = activity_row_of(event, payload, index)?;
    row.change_kind = HistoryChangeKind::StalePremise;
    row.decision_ids = dependents;
    Ok(Some(row))
}

/// `stale_premise_activity_row` for the changed-since listing.
fn stale_premise_change_row(
    event: &Event,
    index: &DecisionIndex,
) -> Result<Option<DecisionChangeRow>> {
    let payload = events::validate(event).map_err(query_validation_error)?;
    stale_premise_change_row_of(event, &payload, index)
}

fn stale_premise_change_row_of(
    event: &Event,
    payload: &EventPayload,
    index: &DecisionIndex,
) -> Result<Option<DecisionChangeRow>> {
    let dependents = stale_premise_dependents(payload, index, event_id(event)?);
    if dependents.is_empty() {
        return Ok(None);
    }
    let mut row = decision_change_row_of(event, payload, index)?;
    row.change_kind = HistoryChangeKind::StalePremise;
    row.decision_ids = dependents;
    Ok(Some(row))
}

fn project_move_for_payload(payload: &EventPayload) -> Option<ProjectMove> {
    match payload {
        EventPayload::DecisionMoved(DecisionMovedPayload {
            from, to, reason, ..
        }) => Some(ProjectMove {
            from: from.clone(),
            to: to.clone(),
            reason: reason.clone(),
        }),
        _ => None,
    }
}

fn title_change_for_payload(payload: &EventPayload) -> Option<TitleChange> {
    match payload {
        EventPayload::DecisionRetitled(DecisionRetitledPayload {
            from, to, reason, ..
        }) => Some(TitleChange {
            from: from.clone(),
            to: to.clone(),
            reason: reason.clone(),
        }),
        _ => None,
    }
}

fn reaffirmed_premise_for_payload(payload: &EventPayload) -> Option<ReaffirmedPremise> {
    match payload {
        EventPayload::DecisionReaffirmed(DecisionReaffirmedPayload {
            premise_id, reason, ..
        }) => Some(ReaffirmedPremise {
            premise_id: premise_id.clone(),
            reason: reason.clone(),
        }),
        _ => None,
    }
}

fn change_kind_for_payload(payload: &EventPayload) -> HistoryChangeKind {
    match payload {
        EventPayload::DecisionProposed(_) => HistoryChangeKind::NewDecision,
        EventPayload::DecisionAccepted(_) | EventPayload::DecisionRejected(_) => {
            HistoryChangeKind::StatusChange
        }
        EventPayload::DecisionSuperseded(_) => HistoryChangeKind::Supersession,
        EventPayload::DecisionMoved(_) => HistoryChangeKind::ProjectMoved,
        EventPayload::DecisionRetitled(_) => HistoryChangeKind::TitleChanged,
        EventPayload::DecisionReaffirmed(_) => HistoryChangeKind::Reaffirmed,
        EventPayload::EvidenceRecorded(_) => HistoryChangeKind::NewEvidence,
        EventPayload::RelationAdded(payload) => match payload.relation {
            EventRelationKind::BasedOn | EventRelationKind::Supports => {
                HistoryChangeKind::NewEvidence
            }
            EventRelationKind::Refutes => HistoryChangeKind::StalePremise,
            EventRelationKind::HasOption
            | EventRelationKind::Chose
            | EventRelationKind::Assumes
            | EventRelationKind::SameAs
            // A premise link is a context change. A premise later going stale is a separate
            // signal: `HistoryChangeKind::StalePremise`, driven by decision.superseded /
            // decision.rejected on the premise (see `stale_premise_dependents`), not by this
            // relation.added event itself.
            | EventRelationKind::FollowsFrom
            // Naming the question a decision answers adds context to it, nothing more.
            | EventRelationKind::Answers => HistoryChangeKind::ContextChange,
        },
        EventPayload::RelationRemoved(_)
        | EventPayload::HypothesisRecorded(_)
        | EventPayload::QuestionRecorded(_)
        | EventPayload::QuestionAsked(_)
        | EventPayload::DecisionRequested(_)
        | EventPayload::BlockerReported(_)
        | EventPayload::BlockerResolved(_)
        | EventPayload::NotificationSent(_)
        | EventPayload::NotificationAcknowledged(_)
        | EventPayload::SuggestionSurfaced(_)
        | EventPayload::IngestBatchReceived(_)
        | EventPayload::IngestBatchClassified(_)
        | EventPayload::DecisionScored(_)
        | EventPayload::DecisionAssessed(_)
        | EventPayload::DecisionMetadataDerived(_)
        | EventPayload::ProjectRegistered(_)
        | EventPayload::ProjectLinked(_)
        | EventPayload::ProjectUnlinked(_)
        | EventPayload::ProjectAnchored(_)
        | EventPayload::ProjectUnanchored(_)
        | EventPayload::ProjectTopicDeclared(_) => HistoryChangeKind::ContextChange,
    }
}

fn decision_ids_for_payload(payload: &EventPayload, index: &DecisionIndex) -> Vec<String> {
    let mut ids = BTreeSet::new();
    match payload {
        EventPayload::DecisionProposed(DecisionProposedPayload { decision_id, .. })
        | EventPayload::DecisionAccepted(DecisionAcceptedPayload { decision_id, .. }) => {
            ids.insert(decision_id.clone());
        }
        EventPayload::DecisionRejected(DecisionRejectedPayload { decision_id, .. }) => {
            ids.insert(decision_id.clone());
        }
        EventPayload::DecisionSuperseded(DecisionSupersededPayload {
            old_decision_id,
            new_decision_id,
        }) => {
            ids.insert(old_decision_id.clone());
            ids.insert(new_decision_id.clone());
        }
        EventPayload::DecisionMoved(DecisionMovedPayload { decision_id, .. }) => {
            ids.insert(decision_id.clone());
        }
        EventPayload::DecisionRetitled(DecisionRetitledPayload { decision_id, .. }) => {
            ids.insert(decision_id.clone());
        }
        EventPayload::DecisionReaffirmed(DecisionReaffirmedPayload { decision_id, .. }) => {
            ids.insert(decision_id.clone());
        }
        EventPayload::DecisionRequested(payload) => {
            if let Some(decision_id) = &payload.decision_id {
                ids.insert(decision_id.clone());
            }
        }
        EventPayload::RelationAdded(RelationAddedPayload {
            relation,
            from_id,
            to_id,
        }) => match relation {
            EventRelationKind::BasedOn
            | EventRelationKind::HasOption
            | EventRelationKind::Chose
            | EventRelationKind::SameAs
            | EventRelationKind::FollowsFrom
            | EventRelationKind::Answers => {
                ids.insert(from_id.clone());
            }
            EventRelationKind::Assumes => {
                let decision_id = index
                    .option_to_decision
                    .get(from_id)
                    .cloned()
                    .unwrap_or_else(|| from_id.clone()); // ubs:ignore:
                ids.insert(decision_id);
            }
            EventRelationKind::Supports => {}
            EventRelationKind::Refutes => {
                ids.extend(index.decisions_assuming(to_id));
            }
        },
        EventPayload::RelationRemoved(_) => {}
        EventPayload::BlockerReported(payload) => {
            if let Some(decision_id) = &payload.decision_id {
                ids.insert(decision_id.clone());
            }
        }
        EventPayload::EvidenceRecorded(_)
        | EventPayload::HypothesisRecorded(_)
        | EventPayload::QuestionRecorded(_)
        | EventPayload::QuestionAsked(_)
        | EventPayload::BlockerResolved(_)
        | EventPayload::NotificationSent(_)
        | EventPayload::NotificationAcknowledged(_)
        | EventPayload::SuggestionSurfaced(_)
        | EventPayload::IngestBatchReceived(_)
        | EventPayload::IngestBatchClassified(_)
        | EventPayload::DecisionScored(_)
        | EventPayload::DecisionAssessed(_)
        | EventPayload::DecisionMetadataDerived(_)
        | EventPayload::ProjectRegistered(_)
        | EventPayload::ProjectLinked(_)
        | EventPayload::ProjectUnlinked(_)
        | EventPayload::ProjectAnchored(_)
        | EventPayload::ProjectUnanchored(_)
        | EventPayload::ProjectTopicDeclared(_) => {}
    }
    ids.into_iter().collect()
}

fn affected_nodes_for_event(event: &Event, payload: &EventPayload) -> Vec<AffectedNode> {
    let mut nodes = BTreeSet::new();
    match payload {
        EventPayload::DecisionProposed(payload) => {
            nodes.insert(affected_node(&payload.decision_id, NodeKind::Decision));
            nodes.extend(
                payload
                    .option_ids
                    .iter()
                    .map(|id| affected_node(id, NodeKind::Option)),
            );
            nodes.extend(
                payload
                    .hypothesis_ids
                    .iter()
                    .map(|id| affected_node(id, NodeKind::Hypothesis)),
            );
            nodes.extend(
                payload
                    .evidence_ids
                    .iter()
                    .map(|id| affected_node(id, NodeKind::Evidence)),
            );
        }
        EventPayload::DecisionAccepted(payload) => {
            nodes.insert(affected_node(&payload.decision_id, NodeKind::Decision));
        }
        EventPayload::DecisionRejected(payload) => {
            nodes.insert(affected_node(&payload.decision_id, NodeKind::Decision));
        }
        EventPayload::DecisionRequested(payload) => {
            nodes.insert(affected_node(
                &event.event_uuid.to_string(),
                NodeKind::DecisionRequest,
            ));
            nodes.insert(affected_node(&payload.requested_by, NodeKind::Actor));
            if let Some(decision_id) = &payload.decision_id {
                nodes.insert(affected_node(decision_id, NodeKind::Decision));
            }
            if let Some(required_owner_id) = &payload.required_owner_id {
                nodes.insert(affected_node(required_owner_id, NodeKind::Actor));
            }
        }
        EventPayload::DecisionSuperseded(payload) => {
            nodes.insert(affected_node(&payload.old_decision_id, NodeKind::Decision));
            nodes.insert(affected_node(&payload.new_decision_id, NodeKind::Decision));
        }
        EventPayload::DecisionMoved(payload) => {
            nodes.insert(affected_node(&payload.decision_id, NodeKind::Decision));
        }
        EventPayload::DecisionRetitled(payload) => {
            nodes.insert(affected_node(&payload.decision_id, NodeKind::Decision));
        }
        EventPayload::DecisionReaffirmed(payload) => {
            nodes.insert(affected_node(
                &event.event_uuid.to_string(),
                NodeKind::Reaffirmation,
            ));
            nodes.insert(affected_node(&payload.decision_id, NodeKind::Decision));
        }
        EventPayload::EvidenceRecorded(payload) => {
            nodes.insert(affected_node(&payload.evidence_id, NodeKind::Evidence));
        }
        EventPayload::HypothesisRecorded(payload) => {
            nodes.insert(affected_node(&payload.hypothesis_id, NodeKind::Hypothesis));
        }
        EventPayload::QuestionRecorded(payload) => {
            nodes.insert(affected_node(&payload.question_id, NodeKind::Question));
        }
        EventPayload::QuestionAsked(payload) => {
            nodes.insert(affected_node(&event.event_uuid.to_string(), NodeKind::Ask));
            nodes.insert(affected_node(&payload.question_id, NodeKind::Question));
        }
        EventPayload::RelationAdded(payload) => {
            if payload.relation == EventRelationKind::Assumes {
                let from_kind = if payload.from_id.starts_with("option-")
                    || payload.from_id.starts_with("option:")
                {
                    NodeKind::Option
                } else {
                    NodeKind::Decision
                };
                nodes.insert(affected_node(&payload.from_id, from_kind));
                nodes.insert(affected_node(&payload.to_id, NodeKind::Hypothesis));
            } else {
                let (from_kind, to_kind) = event_relation_endpoints(payload.relation);
                nodes.insert(affected_node(&payload.from_id, from_kind));
                nodes.insert(affected_node(&payload.to_id, to_kind));
            }
        }
        EventPayload::RelationRemoved(payload) => {
            if payload.relation == EventRelationKind::Assumes {
                let from_kind = if payload.from_id.starts_with("option-")
                    || payload.from_id.starts_with("option:")
                {
                    NodeKind::Option
                } else {
                    NodeKind::Decision
                };
                nodes.insert(affected_node(&payload.from_id, from_kind));
                nodes.insert(affected_node(&payload.to_id, NodeKind::Hypothesis));
            } else {
                let (from_kind, to_kind) = event_relation_endpoints(payload.relation);
                nodes.insert(affected_node(&payload.from_id, from_kind));
                nodes.insert(affected_node(&payload.to_id, to_kind));
            }
        }
        EventPayload::BlockerReported(payload) => {
            nodes.insert(affected_node(&payload.blocker_id, NodeKind::Blocker));
            nodes.insert(affected_node(&payload.blocked_actor_id, NodeKind::Actor));
            if let Some(decision_id) = &payload.decision_id {
                nodes.insert(affected_node(decision_id, NodeKind::Decision));
            }
            if let Some(required_owner_id) = &payload.required_owner_id {
                nodes.insert(affected_node(required_owner_id, NodeKind::Actor));
            }
        }
        EventPayload::NotificationSent(payload) => {
            nodes.insert(affected_node(
                &event.event_uuid.to_string(),
                NodeKind::Notification,
            ));
            nodes.insert(affected_node(&payload.blocker_id, NodeKind::Blocker));
            nodes.insert(affected_node(&payload.recipient_actor_id, NodeKind::Actor));
        }
        EventPayload::BlockerResolved(payload) => {
            nodes.insert(affected_node(&payload.blocker_id, NodeKind::Blocker));
        }
        EventPayload::NotificationAcknowledged(payload) => {
            nodes.insert(affected_node(
                &payload.notification_id,
                NodeKind::Notification,
            ));
        }
        EventPayload::SuggestionSurfaced(payload) => {
            nodes.insert(affected_node(
                &event.event_uuid.to_string(),
                NodeKind::Notification,
            ));
            nodes.insert(affected_node(&payload.recipient_actor_id, NodeKind::Actor));
        }
        EventPayload::IngestBatchReceived(_)
        | EventPayload::IngestBatchClassified(_)
        | EventPayload::DecisionScored(_)
        | EventPayload::DecisionAssessed(_)
        | EventPayload::DecisionMetadataDerived(_)
        | EventPayload::ProjectRegistered(_)
        | EventPayload::ProjectLinked(_)
        | EventPayload::ProjectUnlinked(_)
        | EventPayload::ProjectAnchored(_)
        | EventPayload::ProjectUnanchored(_)
        | EventPayload::ProjectTopicDeclared(_) => {}
    }
    nodes.into_iter().collect()
}

fn affected_node(id: &str, kind: NodeKind) -> AffectedNode {
    AffectedNode {
        id: id.to_owned(),
        kind,
    }
}

fn event_relation_endpoints(relation: EventRelationKind) -> (NodeKind, NodeKind) {
    match relation {
        EventRelationKind::BasedOn => (NodeKind::Decision, NodeKind::Evidence),
        EventRelationKind::HasOption | EventRelationKind::Chose => {
            (NodeKind::Decision, NodeKind::Option)
        }
        EventRelationKind::Assumes => (NodeKind::Decision, NodeKind::Hypothesis),
        EventRelationKind::Supports | EventRelationKind::Refutes => {
            (NodeKind::Evidence, NodeKind::Hypothesis)
        }
        EventRelationKind::SameAs | EventRelationKind::FollowsFrom => {
            (NodeKind::Decision, NodeKind::Decision)
        }
        EventRelationKind::Answers => (NodeKind::Decision, NodeKind::Question),
    }
}

impl Ord for AffectedNode {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.kind, &self.id).cmp(&(other.kind, &other.id))
    }
}

impl PartialOrd for AffectedNode {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

fn matches_history_filters(
    actor_id: &str,
    source: EventSource,
    source_ref: Option<&str>,
    decision_ids: &[String],
    filters: &HistoryFilters,
    index: &DecisionIndex,
    standings: &DecisionStandings,
) -> bool {
    if !filters.actor_ids.is_empty()
        && !filters
            .actor_ids
            .iter()
            .any(|expected| expected == actor_id)
    {
        return false;
    }
    if !filters.sources.is_empty()
        && !filters
            .sources
            .iter()
            .any(|expected| source.as_str().eq_ignore_ascii_case(expected))
    {
        return false;
    }
    if !filters.source_refs.is_empty()
        && !source_ref.is_some_and(|source_ref| {
            filters
                .source_refs
                .iter()
                .any(|expected| expected == source_ref)
        })
    {
        return false;
    }

    if filters.topic_keys.is_empty() && filters.statuses.is_empty() {
        return true;
    }

    decision_ids.iter().any(|decision_id| {
        index.topic_matches(decision_id, &filters.topic_keys)
            && (filters.statuses.is_empty()
                || filters.statuses.contains(&standings.status_of(decision_id)))
    })
}

fn normalized_history_filters(request: &HistoryFilterRequest) -> HistoryFilters {
    HistoryFilters {
        actor_ids: normalized_string_values(request.actor_ids.clone()),
        sources: normalized_string_values(request.sources.clone()),
        source_refs: normalized_string_values(request.source_refs.clone()),
        topic_keys: normalized_topic_filter_values(&request.topic_keys),
        statuses: request
            .statuses
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
    }
}

fn normalized_string_values(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn normalized_history_limit(limit: usize) -> usize {
    if limit == 0 {
        DEFAULT_HISTORY_LIMIT
    } else {
        limit.min(MAX_QUERY_RESULTS)
    }
}

fn parse_history_cursor(cursor: Option<&str>) -> Result<usize> {
    match cursor {
        None => Ok(0),
        Some(cursor) => cursor.parse::<usize>().map_err(|error| {
            query_error(format!("cursor must be a non-negative offset: {error}")).into()
        }),
    }
}

fn event_id(event: &Event) -> Result<EventId> {
    event
        .event_id
        .ok_or_else(|| query_error("event_id is required for history queries").into())
}

fn citation_id(event_id: EventId) -> String {
    format!("event:{event_id}")
}

fn citation_for_event(event: &Event) -> Result<EventCitation> {
    let event_id = event_id(event)?;
    Ok(EventCitation {
        citation_id: citation_id(event_id),
        event_id,
        event_uuid: event.event_uuid.to_string(),
        event_type: event.event_type,
        actor_id: event.actor_id.clone(),
        source: event.source,
        source_ref: event.source_ref.clone(),
        ts: event.ts,
    })
}

fn activity_citation_map(rows: &[ActivityRow]) -> BTreeMap<String, EventCitation> {
    rows.iter()
        .map(|row| {
            let citation = EventCitation {
                citation_id: row.citation_id.clone(),
                event_id: row.event_origin,
                event_uuid: row.event_uuid.clone(),
                event_type: row.event_type,
                actor_id: row.actor_id.clone(),
                source: row.source,
                source_ref: row.source_ref.clone(),
                ts: row.ts,
            };
            (citation.citation_id.clone(), citation)
        })
        .collect()
}

fn change_citation_map(rows: &[DecisionChangeRow]) -> BTreeMap<String, EventCitation> {
    rows.iter()
        .map(|row| {
            let citation = EventCitation {
                citation_id: row.citation_id.clone(),
                event_id: row.event_origin,
                event_uuid: row.event_uuid.clone(),
                event_type: row.event_type,
                actor_id: row.actor_id.clone(),
                source: row.source,
                source_ref: row.source_ref.clone(),
                ts: row.ts,
            };
            (citation.citation_id.clone(), citation)
        })
        .collect()
}

fn recent_activity_query_params(results: &RecentActivityResults) -> BTreeMap<String, Value> {
    BTreeMap::from([
        ("cursor".to_owned(), option_json(results.cursor.as_deref())),
        (
            "filters".to_owned(),
            serde_json::to_value(&results.filters).unwrap_or(Value::Null),
        ),
        ("limit".to_owned(), json!(results.limit)),
        ("query".to_owned(), json!("recent_activity")),
    ])
}

fn changed_since_query_params(results: &DecisionsChangedSinceResults) -> BTreeMap<String, Value> {
    BTreeMap::from([
        ("cursor".to_owned(), option_json(results.cursor.as_deref())),
        (
            "filters".to_owned(),
            serde_json::to_value(&results.filters).unwrap_or(Value::Null),
        ),
        ("limit".to_owned(), json!(results.limit)),
        ("query".to_owned(), json!("decisions_changed_since")),
        (
            "since_offset".to_owned(),
            json!(results.resolved_since.offset),
        ),
        (
            "since_timestamp".to_owned(),
            option_json(
                results
                    .resolved_since
                    .timestamp
                    .as_ref()
                    .map(DateTime::<Utc>::to_rfc3339)
                    .as_deref(),
            ),
        ),
        (
            "until_offset".to_owned(),
            json!(results.resolved_until.offset),
        ),
        (
            "until_timestamp".to_owned(),
            option_json(
                results
                    .resolved_until
                    .timestamp
                    .as_ref()
                    .map(DateTime::<Utc>::to_rfc3339)
                    .as_deref(),
            ),
        ),
    ])
}

fn option_json(value: Option<&str>) -> Value {
    value.map_or(Value::Null, |value| json!(value))
}

fn render_recent_activity_markdown(
    results: &RecentActivityResults,
    generated_at: DateTime<Utc>,
) -> String {
    let mut output = render_markdown_header(
        "recent_activity",
        generated_at,
        &results.ledger_range,
        results.items.len(),
        results.next_cursor.as_deref(),
    );

    for row in &results.items {
        let _ = writeln!(
            output,
            "- event {} {:?} {:?} actor={} source={} citation={}",
            row.event_origin,
            row.event_type,
            row.change_kind,
            row.actor_id,
            row.source.as_str(),
            row.citation_id
        );
        if !row.decision_ids.is_empty() {
            let _ = writeln!(output, "  decisions: {}", row.decision_ids.join(", "));
        }
    }

    output
}

fn render_changed_since_markdown(
    results: &DecisionsChangedSinceResults,
    generated_at: DateTime<Utc>,
) -> String {
    let mut output = render_markdown_header(
        "decisions_changed_since",
        generated_at,
        &results.ledger_range,
        results.items.len(),
        results.next_cursor.as_deref(),
    );
    let _ = write!(
        output,
        "Resolved since: offset {}\nResolved until: offset {}\n\n",
        results.resolved_since.offset, results.resolved_until.offset
    );

    for row in &results.items {
        let _ = writeln!(
            output,
            "- event {} {:?} {:?} actor={} source={} citation={}",
            row.event_origin,
            row.event_type,
            row.change_kind,
            row.actor_id,
            row.source.as_str(),
            row.citation_id
        );
        if !row.decision_ids.is_empty() {
            let _ = writeln!(output, "  decisions: {}", row.decision_ids.join(", "));
        }
    }

    output
}

fn render_markdown_header(
    query: &str,
    generated_at: DateTime<Utc>,
    ledger_range: &LedgerRange,
    result_count: usize,
    continuation_cursor: Option<&str>,
) -> String {
    format!(
        "# Loomtracer Read-Only Summary\n\nQuery: {query}\nGenerated: {}\nLedger range: >{} through {}\nResults: {result_count}\nTruncated: {}\nContinuation cursor: {}\n\n",
        generated_at.to_rfc3339(),
        ledger_range.from_offset_exclusive,
        ledger_range.to_offset_inclusive,
        continuation_cursor.is_some(),
        continuation_cursor.unwrap_or("none"),
    )
}

fn query_validation_error(error: impl std::fmt::Display) -> QueryError {
    query_error(format!("invalid ledger event in history query: {error}"))
}

fn json_query_error(error: serde_json::Error) -> QueryError {
    query_error(format!("json serialization failed: {error}"))
}

fn query_error(error: impl std::fmt::Display) -> QueryError {
    QueryError::Execution(error.to_string())
}

#[cfg(test)]
mod tests;
