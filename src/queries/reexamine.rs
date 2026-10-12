//! Re-examine items: which decisions rest on something that changed, and whether anyone has looked
//! again. Design: the `hivemind-ottw` design field (sections 1, 4, 6).
//!
//! Propagation is visible state, not a score on a person. A decision that rests on a premise (a
//! prior decision it follows from, or an assumption or bet it rests on) gets an *item* when that
//! premise changes. Nothing stores an item as open: it is derived on every read from facts the
//! ledger already holds, plus the `decision.reaffirmed` events ("still holds") that close it.
//! Pure Layer-2 reads: no model, no ranking, no write.
//!
//! # What an item is
//! One item per (dependent D, premise P) where D rests on P directly (one hop; a changed D opens
//! its own dependents' items, a D that still holds stops the wave) and P has *changed*:
//!
//! | Premise | Change | The change happened at |
//! | --- | --- | --- |
//! | a decision | `superseded` | the earliest superseding event |
//! | a decision | `rejected` (rejected, never accepted) | the first rejection |
//! | a decision | `contested` (accepted and rejected) | the moment the second side spoke |
//! | a hypothesis | `refuted` | the earliest refutation |
//!
//! The moment is a *ledger offset*, never a wall-clock time. An item is **open** until
//!
//! - a `decision.reaffirmed` for (D, P) lands at an offset after the change's offset (several can
//!   exist for one pair and each is kept), or
//! - D is itself superseded or rejected after the change (it no longer stands, so there is
//!   nothing to re-examine).
//!
//! A D that was already superseded or rejected *before* the change has no item. A later change to
//! P (contested, then superseded) is a new change at a later offset, so an earlier reaffirmation
//! no longer closes the item: it reopens, and the earlier reaffirmations stay visible on it. A
//! reaffirmation whose offset is unknown, or that cannot be placed against a change whose offset
//! is unknown, does not close anything: an item is never closed on a guess. A contested premise
//! opens an item but is not a verdict, so it never flips `held_up` (see `outcome.rs`).
//!
//! A reaffirmation that matches no changed premise (nothing to re-examine) is ignored. Project
//! links create no items; they decide which items a question asked from one project sees.
//!
//! # Completeness
//! For a changed premise, [`Propagation`] says "N of M dependents re-examined": M counts the
//! dependents that stood when the premise changed, N those since reaffirmed or no longer
//! standing. It is `unfinished` when an item is still open more than `propagation_window_days`
//! (default [`DEFAULT_PROPAGATION_WINDOW_DAYS`]) after the change. The age is the change's own
//! time, which the graph states for a supersession (when the superseding decision was made) and
//! a refutation (when the earliest refuting evidence was recorded) and does not state for a
//! rejection or a contest: those can be open but are never counted unfinished by age. Wording
//! never names an actor.
//!
//! # Scope, paging and cost
//! A request may ask from a project: only items on decisions in the project, the project it is
//! part of and the projects it depends on are named (one hop, `project_scope`); the rest are
//! counted in `not_shown`, never named, and an open item outside the scope still counts toward
//! `unfinished`. A page holds at most `limit` items in `(dependent, premise, change offset)`
//! order; when more remain `truncated` is true and `next_cursor` resumes after the last one
//! returned. The cursor is a position, not an offset, so items that appear or close between two
//! pages never make a reader skip or repeat one that stood throughout.
//!
//! A page issues [`GROUNDING_FACT_READS`](super::GROUNDING_FACT_READS) bulk reads and one project
//! label read however many decisions there are (plus one bulk decision read when scoped), and
//! then anchored reads that word each item: the dependent, the premise and the decision or
//! evidence that dates the change. Nothing walks the premise graph: a link is read once, so a
//! `FOLLOWS_FROM` cycle costs nothing extra and cannot loop.
//!
//! One decision's own entries ([`ReexamineEntry`], carried on its outcome) are derived from the
//! same rules with anchored reads, and only when one of its premises has changed.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::error::QueryError;
use crate::ledger::{EventLedger, TenantScopedLedger};
use crate::projector::{GraphRow, GraphView, NodeKind, RelationKind};
use crate::Result;

use super::decision::get_hypothesis_statement;
use super::grounding_facts::{
    get_grounding_facts, reaffirmation_records, GroundingFacts, ReaffirmationRecord,
};
use super::project_label::ProjectLabels;
use super::project_scope::{project_scope, ProjectScope, ScopeNote};
use super::shared::{
    neighbor_edges_with_origin, node_row, node_rows, optional_datetime, optional_string,
    query_timer_start, Direction, MAX_QUERY_RESULTS,
};
use super::status::{DecisionStatus, StandingSummary};
use super::{QueryContext, QueryResponse};

/// How long an item may stay open after its premise changed before the change reads as
/// unfinished, when nobody says otherwise: two weeks.
pub const DEFAULT_PROPAGATION_WINDOW_DAYS: u32 = 14;

/// Items per page when the request does not say.
pub const DEFAULT_REEXAMINE_PAGE_SIZE: usize = 50;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// How a premise changed.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    /// A decision was superseded.
    Superseded,
    /// A decision was rejected and nobody accepted it.
    Rejected,
    /// A decision has both accepting and rejecting actors: a live disagreement, not a verdict.
    Contested,
    /// A hypothesis (an assumption or a bet) was refuted by evidence.
    Refuted,
}

/// What a decision rests on, as far as an item is concerned.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PremiseKind {
    Decision,
    Hypothesis,
}

/// A premise's change: what happened, who or what did it, and when.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PremiseChange {
    pub kind: ChangeKind,
    /// The superseding decisions, the rejecting (or, for a contest, dissenting) actors, or the
    /// refuting evidence, sorted by id.
    pub by_ids: Vec<String>,
    /// The ledger offset of the change; what a reaffirmation is ordered against. Absent when the
    /// edge states none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at_offset: Option<i64>,
    /// When the change was made, where the graph states it (a superseding decision's time, the
    /// earliest refuting evidence's time); absent for a rejection or a contest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at_time: Option<DateTime<Utc>>,
    /// The decision whose own time dates a supersession: the one superseded first.
    #[serde(skip)]
    dated_by: Option<String>,
}

/// One `decision.reaffirmed` event, as an item shows it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReaffirmationSummary {
    /// The event's own uuid.
    pub id: String,
    pub actor_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<DateTime<Utc>>,
    /// The one-line reason the actor gave.
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_origin: Option<i64>,
}

/// Where an item stands.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ItemState {
    /// Nobody has said the dependent still holds, and it still stands.
    Open,
    /// Someone said it still holds, after the change. Each such reaffirmation, in ledger order.
    Reaffirmed {
        reaffirmations: Vec<ReaffirmationSummary>,
    },
    /// The dependent was itself superseded (by `by_id`, the supersession recorded first) after
    /// the change.
    DependentSuperseded { by_id: String },
    /// The dependent was itself rejected, and never accepted, after the change.
    DependentRejected,
}

impl ItemState {
    pub fn is_open(&self) -> bool {
        matches!(self, Self::Open)
    }
}

/// What a decision's re-examination of one premise comes to: the item minus the words that name
/// the dependent. It is what a decision's outcome carries.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReexamineEntry {
    pub premise_id: String,
    pub premise_kind: PremiseKind,
    /// The premise's own words: a decision's title, a hypothesis's statement (its id when the
    /// node states none).
    pub premise_label: String,
    pub change: PremiseChange,
    #[serde(flatten)]
    pub state: ItemState,
    /// Reaffirmations recorded *before* the latest change: history that no longer closes it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub earlier_reaffirmations: Vec<ReaffirmationSummary>,
}

/// The decision that has to be re-examined.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ItemDependent {
    pub id: String,
    /// Its title (its id when the node states none).
    pub title: String,
    pub status: DecisionStatus,
    /// Address of the project it is filed under (see `DecisionView::project`).
    pub project: Option<String>,
    /// What a person calls that project.
    pub project_label: String,
}

/// One open or closed re-examination: a dependent, the premise that changed, how.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReexamineItem {
    pub dependent: ItemDependent,
    /// Where the premise decision is filed, when it is one: an item that crossed a project link
    /// names where the change came from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub premise_project: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub premise_project_label: Option<String>,
    #[serde(flatten)]
    pub entry: ReexamineEntry,
}

/// "N of M dependents re-examined" for one changed premise.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Propagation {
    pub premise_id: String,
    pub premise_kind: PremiseKind,
    pub premise_label: String,
    pub change: PremiseChange,
    /// M: the dependents that stood when the premise changed (plus those linked since). A
    /// dependent already out before the change is not counted: nothing to re-examine.
    pub dependents_total: usize,
    /// N: those reaffirmed, plus those since superseded or rejected.
    pub reexamined: usize,
    /// M - N: those still open.
    pub open: usize,
    /// When the oldest open item opened: the change's own time, when the graph states it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oldest_open_since: Option<DateTime<Utc>>,
    /// An item is still open more than `window_days` after the change. Never a judgement of
    /// anyone: a fact about the age of an open item.
    pub unfinished: bool,
    pub window_days: u32,
    /// Dependents in projects outside the asker's scope: counted here, never named. Absent when
    /// the request named no project.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_shown: Option<usize>,
    /// Of those, how many are still open. Absent when the request named no project.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_shown_open: Option<usize>,
}

/// Which items to page through.
#[derive(Clone, Debug, Default)]
pub struct ReexamineRequest {
    /// Only items on this decision.
    pub dependent_id: Option<String>,
    /// Only items resting on this premise (a decision or a hypothesis). Also fills
    /// [`ReexamineResults::propagation`].
    pub premise_id: Option<String>,
    /// Ask from this project: a registered handle or a personal address. Items on decisions in
    /// the project, the project it is part of and the projects it depends on are named; the rest
    /// are counted. `None` is the whole tenant. An unregistered handle is refused.
    pub project: Option<String>,
    /// Closed items too. Open ones only when false.
    pub include_closed: bool,
    /// Items per page; [`DEFAULT_REEXAMINE_PAGE_SIZE`] when 0, never more than `MAX_QUERY_RESULTS`.
    pub limit: usize,
    /// The `next_cursor` of the previous page.
    pub cursor: Option<String>,
    /// The unfinished window in days; [`DEFAULT_PROPAGATION_WINDOW_DAYS`] when unset.
    pub propagation_window_days: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReexamineResults {
    pub items: Vec<ReexamineItem>,
    pub next_cursor: Option<String>,
    /// Items that match the request but sit on decisions outside the asker's scope: counted,
    /// never named. Absent when the request named no project.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_shown: Option<usize>,
    /// Where a project-scoped answer looked and where it stopped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<ScopeNote>,
    /// Completeness of the premise the request names, when it names one that has changed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub propagation: Option<Propagation>,
}

// ---------------------------------------------------------------------------
// The rules (pure)
// ---------------------------------------------------------------------------

/// A premise's change, judged against one dependent.
#[derive(Clone)]
struct Judged {
    premise_kind: PremiseKind,
    change: PremiseChange,
    state: ItemState,
    earlier: Vec<ReaffirmationSummary>,
}

/// How a premise decision changed, from the position edges it carries.
fn decision_change(standing: &StandingSummary) -> Option<PremiseChange> {
    let earliest = standing.earliest_superseder.as_ref();
    let (kind, by_ids, at_offset, dated_by) = match standing.status() {
        DecisionStatus::Superseded => (
            ChangeKind::Superseded,
            standing.superseders.clone(),
            earliest.and_then(|(_, origin)| *origin),
            earliest.map(|(superseder, _)| superseder.clone()),
        ),
        DecisionStatus::Rejected => (
            ChangeKind::Rejected,
            standing.rejecters.clone(),
            standing.first_rejected_at,
            None,
        ),
        DecisionStatus::Contested => (
            ChangeKind::Contested,
            standing.rejecters.clone(),
            standing.contested_at(),
            None,
        ),
        DecisionStatus::Proposed | DecisionStatus::Accepted => return None,
    };
    Some(PremiseChange {
        kind,
        by_ids,
        at_offset,
        at_time: None,
        dated_by,
    })
}

/// How a hypothesis changed: refuted, or not at all.
fn hypothesis_change(refuted_by: &[String], refuted_at: Option<i64>) -> Option<PremiseChange> {
    if refuted_by.is_empty() {
        return None;
    }
    Some(PremiseChange {
        kind: ChangeKind::Refuted,
        by_ids: refuted_by.to_vec(),
        at_offset: refuted_at,
        at_time: None,
        dated_by: None,
    })
}

/// [`hypothesis_change`] for the refuting edges read anchored on one hypothesis:
/// `(evidence, offset)`; the change is dated by the earliest offset an edge states.
fn hypothesis_change_from_edges(refuters: &[(String, Option<i64>)]) -> Option<PremiseChange> {
    let refuted_by: Vec<String> = refuters.iter().map(|(id, _)| id.clone()).collect();
    let refuted_at = refuters.iter().filter_map(|(_, origin)| *origin).min();
    hypothesis_change(&refuted_by, refuted_at)
}

/// Where the dependent stands against a change at `change_offset`.
enum DependentPosition {
    /// It stands: not superseded, not rejected.
    Stands,
    /// It was already out before the change; nothing to re-examine.
    OutBefore,
    /// It went out at or after the change (or when it did cannot be told).
    Out(ItemState),
}

fn dependent_position(
    dependent: &StandingSummary,
    change_offset: Option<i64>,
) -> DependentPosition {
    let (out_at, state) = match dependent.status() {
        DecisionStatus::Superseded => (
            dependent.earliest_superseder.as_ref().and_then(|(_, o)| *o),
            ItemState::DependentSuperseded {
                by_id: dependent
                    .earliest_superseder
                    .as_ref()
                    .map(|(superseder, _)| superseder.clone())
                    .unwrap_or_default(),
            },
        ),
        DecisionStatus::Rejected => (dependent.first_rejected_at, ItemState::DependentRejected),
        DecisionStatus::Proposed | DecisionStatus::Accepted | DecisionStatus::Contested => {
            return DependentPosition::Stands
        }
    };
    match (out_at, change_offset) {
        (Some(out), Some(change)) if out < change => DependentPosition::OutBefore,
        _ => DependentPosition::Out(state),
    }
}

fn summarize_reaffirmation(record: &ReaffirmationRecord) -> ReaffirmationSummary {
    ReaffirmationSummary {
        id: record.id.clone(),
        actor_id: record.reaffirmed_by.clone(),
        at: record.reaffirmed_at,
        reason: record.reason.clone(),
        event_origin: record.event_origin,
    }
}

/// The state of one (dependent, premise) pair after `change`, or `None` when the dependent was
/// already out before the change. `reaffirmations` are every reaffirmation of the pair.
fn judge(
    dependent: &StandingSummary,
    premise_kind: PremiseKind,
    change: PremiseChange,
    reaffirmations: &[&ReaffirmationRecord],
) -> Option<Judged> {
    let position = dependent_position(dependent, change.at_offset);
    if matches!(position, DependentPosition::OutBefore) {
        return None;
    }
    let mut after = Vec::new();
    let mut earlier = Vec::new();
    for record in reaffirmations {
        let summary = summarize_reaffirmation(record);
        match (record.event_origin, change.at_offset) {
            (Some(reaffirmed), Some(changed)) if reaffirmed > changed => after.push(summary),
            _ => earlier.push(summary),
        }
    }
    let state = if !after.is_empty() {
        ItemState::Reaffirmed {
            reaffirmations: after,
        }
    } else {
        match position {
            DependentPosition::Out(state) => state,
            DependentPosition::Stands | DependentPosition::OutBefore => ItemState::Open,
        }
    };
    Some(Judged {
        premise_kind,
        change,
        state,
        earlier,
    })
}

// ---------------------------------------------------------------------------
// The whole graph at once
// ---------------------------------------------------------------------------

/// One derived item before it is worded.
struct IndexedItem {
    dependent: String,
    premise: String,
    judged: Judged,
}

impl IndexedItem {
    /// The position a cursor is a point in: `(dependent, premise, change offset)`.
    fn position(&self) -> (&str, &str, i64) {
        (
            self.dependent.as_str(),
            self.premise.as_str(),
            self.judged.change.at_offset.unwrap_or(i64::MIN),
        )
    }
}

/// Every item of the graph, derived from the grounding facts alone.
struct ReexamineIndex {
    /// In `(dependent, premise, change offset)` order.
    items: Vec<IndexedItem>,
}

impl ReexamineIndex {
    fn from_facts(facts: &GroundingFacts) -> Self {
        let mut reaffirmed: BTreeMap<(&str, &str), Vec<&ReaffirmationRecord>> = BTreeMap::new();
        for record in &facts.reaffirmations {
            reaffirmed
                .entry((record.decision_id.as_str(), record.premise_id.as_str()))
                .or_default()
                .push(record);
        }

        let mut dependents: BTreeMap<&str, StandingSummary> = BTreeMap::new();
        let mut decision_changes: BTreeMap<&str, Option<PremiseChange>> = BTreeMap::new();
        let mut items = Vec::new();

        items.extend(
            facts
                .premise_links
                .iter()
                // A decision that follows from itself rests on nothing but itself.
                .filter(|(dependent, premise)| dependent != premise)
                .filter_map(|(dependent, premise)| {
                    let change = decision_changes
                        .entry(premise.as_str())
                        .or_insert_with(|| decision_change(&facts.standings.summary_of(premise)))
                        .as_ref()?;
                    let standing = dependents
                        .entry(dependent.as_str())
                        .or_insert_with(|| facts.standings.summary_of(dependent));
                    let pair = reaffirmed
                        .get(&(dependent.as_str(), premise.as_str()))
                        .map(Vec::as_slice)
                        .unwrap_or_default();
                    let judged = judge(standing, PremiseKind::Decision, change.clone(), pair)?;
                    Some(IndexedItem {
                        dependent: dependent.clone(),
                        premise: premise.clone(),
                        judged,
                    })
                }),
        );

        items.extend(
            facts
                .hypothesis_links
                .iter()
                .filter_map(|(dependent, hypothesis)| {
                    let record = facts.hypotheses.get(hypothesis)?;
                    let change = hypothesis_change(&record.refuted_by, record.refuted_at)?;
                    let standing = dependents
                        .entry(dependent.as_str())
                        .or_insert_with(|| facts.standings.summary_of(dependent));
                    let pair = reaffirmed
                        .get(&(dependent.as_str(), hypothesis.as_str()))
                        .map(Vec::as_slice)
                        .unwrap_or_default();
                    let judged = judge(standing, PremiseKind::Hypothesis, change, pair)?;
                    Some(IndexedItem {
                        dependent: dependent.clone(),
                        premise: hypothesis.clone(),
                        judged,
                    })
                }),
        );

        items.sort_by(|left, right| left.position().cmp(&right.position()));
        Self { items }
    }
}

// ---------------------------------------------------------------------------
// Wording: titles, projects, times
// ---------------------------------------------------------------------------

/// What a decision node states about itself that an item needs.
struct DecisionFacts {
    title: Option<String>,
    project: Option<String>,
}

fn decision_facts_of(row: &GraphRow) -> DecisionFacts {
    DecisionFacts {
        title: optional_string(row, "title"),
        project: optional_string(row, "project"),
    }
}

/// The title and the project a decision states about itself, owned; both absent for a decision
/// the graph does not hold.
fn wording_of(facts: Option<&DecisionFacts>) -> (Option<String>, Option<String>) {
    facts.map_or((None, None), |facts| {
        (facts.title.clone(), facts.project.clone())
    })
}

/// Decision nodes by id: one bulk read when the whole set is wanted (a scoped request needs every
/// dependent's project to count what it does not name), anchored lookups otherwise.
struct DecisionLookup<'g, G: GraphView> {
    graph: &'g G,
    bulk: Option<BTreeMap<String, GraphRow>>,
    cache: BTreeMap<String, Option<DecisionFacts>>,
}

impl<'g, G: GraphView> DecisionLookup<'g, G> {
    fn anchored(graph: &'g G) -> Self {
        Self {
            graph,
            bulk: None,
            cache: BTreeMap::new(),
        }
    }

    fn bulk(graph: &'g G) -> Result<Self> {
        Ok(Self {
            graph,
            bulk: Some(node_rows(graph, NodeKind::Decision)?),
            cache: BTreeMap::new(),
        })
    }

    fn facts(&mut self, decision_id: &str) -> Result<Option<&DecisionFacts>> {
        if !self.cache.contains_key(decision_id) {
            let facts = match &self.bulk {
                Some(rows) => rows.get(decision_id).map(decision_facts_of),
                None => node_row(self.graph, NodeKind::Decision, decision_id)?
                    .as_ref()
                    .map(decision_facts_of),
            };
            self.cache.insert(decision_id.to_owned(), facts);
        }
        Ok(self.cache.get(decision_id).and_then(Option::as_ref))
    }

    fn title(&mut self, decision_id: &str) -> Result<Option<String>> {
        Ok(self
            .facts(decision_id)?
            .and_then(|facts| facts.title.clone()))
    }

    fn project(&mut self, decision_id: &str) -> Result<Option<String>> {
        Ok(self
            .facts(decision_id)?
            .and_then(|facts| facts.project.clone()))
    }
}

/// When a change was made, where the graph states it: the superseding decision's time, the
/// earliest refuting evidence's time. A rejection or a contest has none.
fn change_time(graph: &impl GraphView, change: &PremiseChange) -> Result<Option<DateTime<Utc>>> {
    match change.kind {
        ChangeKind::Superseded => {
            let Some(by) = change.dated_by.as_deref() else {
                return Ok(None);
            };
            let row = node_row(graph, NodeKind::Decision, by)?;
            row.map(|row| optional_datetime(&row, "occurred_at"))
                .transpose()
                .map(Option::flatten)
        }
        ChangeKind::Refuted => {
            let mut earliest: Option<DateTime<Utc>> = None;
            for evidence_id in &change.by_ids {
                let Some(row) = node_row(graph, NodeKind::Evidence, evidence_id)? else {
                    continue;
                };
                if let Some(recorded_at) = optional_datetime(&row, "recorded_at")? {
                    earliest = Some(earliest.map_or(recorded_at, |known| known.min(recorded_at)));
                }
            }
            Ok(earliest)
        }
        ChangeKind::Rejected | ChangeKind::Contested => Ok(None),
    }
}

/// A premise in its own words: a decision's title, a hypothesis's statement, else its id.
fn premise_label(
    graph: &impl GraphView,
    decisions: &mut DecisionLookup<'_, impl GraphView>,
    premise_id: &str,
    kind: PremiseKind,
) -> Result<String> {
    let label = match kind {
        PremiseKind::Decision => decisions.title(premise_id)?,
        PremiseKind::Hypothesis => get_hypothesis_statement(graph, premise_id)?,
    };
    Ok(label.unwrap_or_else(|| premise_id.to_owned()))
}

/// A judged pair in words, with the change dated.
fn entry_of(
    graph: &impl GraphView,
    decisions: &mut DecisionLookup<'_, impl GraphView>,
    premise_id: &str,
    judged: Judged,
) -> Result<ReexamineEntry> {
    let mut change = judged.change;
    change.at_time = change_time(graph, &change)?;
    Ok(ReexamineEntry {
        premise_id: premise_id.to_owned(),
        premise_kind: judged.premise_kind,
        premise_label: premise_label(graph, decisions, premise_id, judged.premise_kind)?,
        change,
        state: judged.state,
        earlier_reaffirmations: judged.earlier,
    })
}

// ---------------------------------------------------------------------------
// One decision's own entries (anchored)
// ---------------------------------------------------------------------------

/// What one decision's position edges state, read anchored on it.
fn anchored_standing(graph: &impl GraphView, decision_id: &str) -> Result<StandingSummary> {
    let superseders = neighbor_edges_with_origin(
        graph,
        NodeKind::Decision,
        decision_id,
        RelationKind::Supersedes,
        NodeKind::Decision,
        Direction::Incoming,
    )?;
    let accepted: Vec<Option<i64>> = neighbor_edges_with_origin(
        graph,
        NodeKind::Decision,
        decision_id,
        RelationKind::AcceptedBy,
        NodeKind::Actor,
        Direction::Outgoing,
    )?
    .into_iter()
    .map(|(_, origin)| origin)
    .collect();
    let rejected = neighbor_edges_with_origin(
        graph,
        NodeKind::Decision,
        decision_id,
        RelationKind::RejectedBy,
        NodeKind::Actor,
        Direction::Outgoing,
    )?;
    Ok(StandingSummary::from_edges(
        &superseders,
        &accepted,
        &rejected,
    ))
}

/// The reaffirmations among `records` that name this premise.
fn reaffirmations_of<'a>(
    records: &'a [ReaffirmationRecord],
    premise_id: &str,
) -> Vec<&'a ReaffirmationRecord> {
    records
        .iter()
        .filter(|record| record.premise_id == premise_id)
        .collect()
}

/// The re-examination entries of one decision: one per premise it rests on that has changed.
/// `changed_decision_premises` are the prior decisions it follows from that no longer stand or
/// are contested, `refuted_hypotheses` the assumptions and bets it rests on that were refuted;
/// the caller has found them already, and with none of either this reads nothing. Anchored reads
/// only (plus the one scan of the reaffirmation nodes, which have no edge to anchor on), so it
/// costs the same on a graph of ten decisions or ten thousand.
pub(crate) fn reexamine_entries_for(
    graph: &impl GraphView,
    decision_id: &str,
    changed_decision_premises: &[String],
    refuted_hypotheses: &[String],
) -> Result<Vec<ReexamineEntry>> {
    if changed_decision_premises.is_empty() && refuted_hypotheses.is_empty() {
        return Ok(Vec::new());
    }
    let reaffirmations: Vec<ReaffirmationRecord> = reaffirmation_records(graph)?
        .into_iter()
        .filter(|record| record.decision_id == decision_id)
        .collect();
    let dependent = anchored_standing(graph, decision_id)?;
    let mut decisions = DecisionLookup::anchored(graph);

    let mut entries = Vec::new();
    let mut premises: Vec<&String> = changed_decision_premises.iter().collect();
    premises.sort();
    premises.dedup();
    for premise_id in premises {
        // A decision never rests on itself (see `ReexamineIndex::from_facts`).
        if premise_id == decision_id {
            continue;
        }
        let Some(change) = decision_change(&anchored_standing(graph, premise_id)?) else {
            continue;
        };
        if let Some(judged) = judge(
            &dependent,
            PremiseKind::Decision,
            change,
            &reaffirmations_of(&reaffirmations, premise_id),
        ) {
            entries.push(entry_of(graph, &mut decisions, premise_id, judged)?);
        }
    }

    let mut hypotheses: Vec<&String> = refuted_hypotheses.iter().collect();
    hypotheses.sort();
    hypotheses.dedup();
    for hypothesis_id in hypotheses {
        let refuters = neighbor_edges_with_origin(
            graph,
            NodeKind::Hypothesis,
            hypothesis_id,
            RelationKind::Refutes,
            NodeKind::Evidence,
            Direction::Incoming,
        )?;
        let Some(change) = hypothesis_change_from_edges(&refuters) else {
            continue;
        };
        if let Some(judged) = judge(
            &dependent,
            PremiseKind::Hypothesis,
            change,
            &reaffirmations_of(&reaffirmations, hypothesis_id),
        ) {
            entries.push(entry_of(graph, &mut decisions, hypothesis_id, judged)?);
        }
    }
    Ok(entries)
}

// ---------------------------------------------------------------------------
// Completeness
// ---------------------------------------------------------------------------

/// "N of M" for the items of one changed premise. `members` are `(shown, state)`: whether the
/// dependent is in the asker's scope, and where its item stands; `scoped` says whether the
/// request named a project at all.
fn summarize(
    premise_id: &str,
    entry_view: &ReexamineEntry,
    members: &[(bool, &ItemState)],
    scoped: bool,
    now: DateTime<Utc>,
    window_days: u32,
) -> Propagation {
    let shown = members.iter().filter(|(shown, _)| *shown);
    let hidden = members.iter().filter(|(shown, _)| !*shown);
    let dependents_total = shown.clone().count();
    let open = shown.filter(|(_, state)| state.is_open()).count();
    let not_shown = hidden.clone().count();
    let not_shown_open = hidden.filter(|(_, state)| state.is_open()).count();

    // An open item anywhere counts toward unfinished: hiding it behind a scope would hide
    // unfinished propagation.
    let any_open = open + not_shown_open > 0;
    let at_time = entry_view.change.at_time;
    let unfinished =
        any_open && at_time.is_some_and(|at| now - at > Duration::days(i64::from(window_days)));
    Propagation {
        premise_id: premise_id.to_owned(),
        premise_kind: entry_view.premise_kind,
        premise_label: entry_view.premise_label.clone(),
        change: entry_view.change.clone(),
        dependents_total,
        reexamined: dependents_total - open,
        open,
        oldest_open_since: if any_open { at_time } else { None },
        unfinished,
        window_days,
        not_shown: scoped.then_some(not_shown),
        not_shown_open: scoped.then_some(not_shown_open),
    }
}

/// How complete the propagation of one changed premise is, across the whole tenant, as of `now`:
/// `None` when the premise has not changed (there is nothing to propagate) or has no dependents
/// that stood when it changed. Reads the grounding facts in bulk.
pub fn get_propagation_at(
    graph: &impl GraphView,
    premise_id: &str,
    window_days: Option<u32>,
    now: DateTime<Utc>,
) -> Result<Option<Propagation>> {
    let facts = get_grounding_facts(graph)?;
    let index = ReexamineIndex::from_facts(&facts);
    let mut decisions = DecisionLookup::anchored(graph);
    propagation_of(
        graph,
        &mut decisions,
        &index,
        premise_id,
        &|_: &str| Ok(true),
        false,
        now,
        window_days.unwrap_or(DEFAULT_PROPAGATION_WINDOW_DAYS),
    )
}

/// [`get_propagation_at`] as of the current time.
pub fn get_propagation(
    graph: &impl GraphView,
    premise_id: &str,
    window_days: Option<u32>,
) -> Result<Option<Propagation>> {
    get_propagation_at(graph, premise_id, window_days, Utc::now())
}

#[allow(clippy::too_many_arguments)]
fn propagation_of(
    graph: &impl GraphView,
    decisions: &mut DecisionLookup<'_, impl GraphView>,
    index: &ReexamineIndex,
    premise_id: &str,
    in_scope: &dyn Fn(&str) -> Result<bool>,
    scoped: bool,
    now: DateTime<Utc>,
    window_days: u32,
) -> Result<Option<Propagation>> {
    let of_premise: Vec<&IndexedItem> = index
        .items
        .iter()
        .filter(|item| item.premise == premise_id)
        .collect();
    let Some(first) = of_premise.first() else {
        return Ok(None);
    };
    let mut members = Vec::with_capacity(of_premise.len());
    for item in &of_premise {
        members.push((in_scope(&item.dependent)?, &item.judged.state));
    }
    // Every item of one premise shares the change; word it from the first.
    let entry_view = entry_of(
        graph,
        decisions,
        premise_id,
        Judged {
            state: ItemState::Open,
            earlier: Vec::new(),
            ..first.judged.clone()
        },
    )?;
    Ok(Some(summarize(
        premise_id,
        &entry_view,
        &members,
        scoped,
        now,
        window_days,
    )))
}

// ---------------------------------------------------------------------------
// The paged list
// ---------------------------------------------------------------------------

/// A position in the item order, as a cursor carries it.
#[derive(Debug, Deserialize, Serialize)]
struct Cursor {
    dependent: String,
    premise: String,
    offset: Option<i64>,
}

fn parse_item_cursor(cursor: &str) -> Result<Cursor> {
    serde_json::from_str(cursor).map_err(|_| {
        QueryError::Execution("cursor is not one a re-examine query returned".to_owned()).into()
    })
}

fn item_cursor(item: &IndexedItem) -> Result<String> {
    serde_json::to_string(&Cursor {
        dependent: item.dependent.clone(),
        premise: item.premise.clone(),
        offset: item.judged.change.at_offset,
    })
    .map_err(|error| QueryError::Execution(format!("cursor could not be written: {error}")).into())
}

fn page_limit(limit: usize) -> usize {
    if limit == 0 {
        DEFAULT_REEXAMINE_PAGE_SIZE
    } else {
        limit.min(MAX_QUERY_RESULTS)
    }
}

/// The re-examine items, one page at a time, as of the current time.
pub fn get_reexamine_items(
    context: &QueryContext,
    graph: &impl GraphView,
    ledger: &impl EventLedger,
    request: &ReexamineRequest,
) -> Result<QueryResponse<ReexamineResults>> {
    get_reexamine_items_at(context, graph, ledger, request, Utc::now())
}

/// [`get_reexamine_items`] as of `now`, which only decides what reads as unfinished.
pub fn get_reexamine_items_at(
    context: &QueryContext,
    graph: &impl GraphView,
    ledger: &impl EventLedger,
    request: &ReexamineRequest,
    now: DateTime<Utc>,
) -> Result<QueryResponse<ReexamineResults>> {
    let started = query_timer_start();
    let limit = page_limit(request.limit);
    let after = request
        .cursor
        .as_deref()
        .map(parse_item_cursor)
        .transpose()?;
    let window_days = request
        .propagation_window_days
        .unwrap_or(DEFAULT_PROPAGATION_WINDOW_DAYS);
    let scope: Option<ProjectScope> = match request.project.as_deref() {
        Some(project) => {
            let scoped_ledger = TenantScopedLedger::new(ledger, context.tenant_id.clone());
            Some(project_scope(&scoped_ledger, project)?)
        }
        None => None,
    };

    let facts = get_grounding_facts(graph)?;
    let index = ReexamineIndex::from_facts(&facts);
    let labels = ProjectLabels::from_graph(graph)?;
    let mut decisions = if scope.is_some() {
        DecisionLookup::bulk(graph)?
    } else {
        DecisionLookup::anchored(graph)
    };

    // Which dependents the asker sees: every one, or those filed in the scope.
    let mut seen: BTreeMap<&str, bool> = BTreeMap::new();
    if let Some(scope) = &scope {
        for item in &index.items {
            let dependent = item.dependent.as_str();
            if seen.contains_key(dependent) {
                continue;
            }
            let project = decisions.project(dependent)?;
            seen.insert(dependent, scope.relation_of(project.as_deref()).is_some());
        }
    }
    let in_scope = |dependent: &str| -> Result<bool> {
        Ok(scope.is_none() || seen.get(dependent).copied().unwrap_or(false))
    };

    let propagation = match request.premise_id.as_deref() {
        Some(premise_id) => propagation_of(
            graph,
            &mut decisions,
            &index,
            premise_id,
            &in_scope,
            scope.is_some(),
            now,
            window_days,
        )?,
        None => None,
    };

    let mut not_shown = 0_usize;
    let mut page: Vec<&IndexedItem> = Vec::new();
    let mut more = false;
    for item in &index.items {
        if request
            .dependent_id
            .as_deref()
            .is_some_and(|dependent| dependent != item.dependent)
            || request
                .premise_id
                .as_deref()
                .is_some_and(|premise| premise != item.premise)
            || !(request.include_closed || item.judged.state.is_open())
        {
            continue;
        }
        if !in_scope(&item.dependent)? {
            not_shown += 1;
            continue;
        }
        if let Some(after) = &after {
            let resume = (
                after.dependent.as_str(),
                after.premise.as_str(),
                after.offset.unwrap_or(i64::MIN),
            );
            if item.position() <= resume {
                continue;
            }
        }
        if page.len() == limit {
            more = true;
            continue;
        }
        page.push(item);
    }
    let next_cursor = match (more, page.last()) {
        (true, Some(last)) => Some(item_cursor(last)?),
        _ => None,
    };

    let mut items = Vec::with_capacity(page.len());
    for item in page {
        let (title, project) = wording_of(decisions.facts(&item.dependent)?);
        items.push(word_item(
            graph,
            &mut decisions,
            &labels,
            &facts,
            item,
            title,
            project,
        )?);
    }

    Ok(QueryResponse {
        result_count: items.len(),
        truncated: next_cursor.is_some(),
        latency_ms: started.elapsed().as_millis(),
        data: ReexamineResults {
            items,
            next_cursor,
            not_shown: scope.is_some().then_some(not_shown),
            scope: scope.as_ref().map(|scope| scope.note(&labels)),
            propagation,
        },
    })
}

fn word_item(
    graph: &impl GraphView,
    decisions: &mut DecisionLookup<'_, impl GraphView>,
    labels: &ProjectLabels,
    facts: &GroundingFacts,
    item: &IndexedItem,
    title: Option<String>,
    project: Option<String>,
) -> Result<ReexamineItem> {
    let entry = entry_of(graph, decisions, &item.premise, item.judged.clone())?;
    let (premise_project, premise_project_label) = match item.judged.premise_kind {
        PremiseKind::Decision => {
            let project = decisions.project(&item.premise)?;
            let label = labels.label_of(project.as_deref());
            (project, Some(label))
        }
        PremiseKind::Hypothesis => (None, None),
    };
    Ok(ReexamineItem {
        dependent: ItemDependent {
            title: title.unwrap_or_else(|| item.dependent.clone()),
            status: facts.standings.status_of(&item.dependent),
            project_label: labels.label_of(project.as_deref()),
            project,
            id: item.dependent.clone(),
        },
        premise_project,
        premise_project_label,
        entry,
    })
}

#[cfg(test)]
mod tests;
