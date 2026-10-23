//! Full-text and filter search over decisions via SQLite FTS and graph predicate evaluation.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Instant;

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::Serialize;

use crate::ledger::{AnyLedger, EventLedger, SqliteEventLedger, TenantScopedLedger};
use crate::projector::{GraphRow, GraphView, NodeKind, RelationKind};
use crate::Result;

use super::decision::{DecisionView, HypothesisContext};
use super::grounding::{hypothesis_facts_from_row, GroundingState};
use super::project_label::ProjectLabels;
use super::project_scope::{project_scope, MatchScope, ProjectScope, ScopeNote, ScopeRelation};
use super::reexamine::{ReexamineMarker, ReexamineMarkers};
use super::same_as::{fold_linked, RecordedCopy};
use super::shared::{
    node_rows, normalized_filter_values, normalized_limit, normalized_query, normalized_statuses,
    normalized_topic_filter_values, optional_datetime, optional_int, optional_string,
    optional_string_list, parse_cursor, query_error, query_terms, recorded_floor_of,
    relation_edges_by_kind, relation_sources, relation_targets, stored_topics_include,
};
use super::status::{hypothesis_status_from_evidence, DecisionStandings, DecisionStatus};
use super::terms::{
    content_query, outside_negation, resolver_question, spelled_together, stem, word_stems,
    RelatedWord, WordMatch,
};
use super::{QueryContext, QueryResponse};

const MAX_SNIPPETS_PER_RESULT: usize = 5;
const SNIPPET_MAX_CHARS: usize = 160;

#[derive(Clone, Debug, PartialEq)]
pub struct SearchDecisionRequest {
    pub query: Option<String>,
    pub topic_keys: Vec<String>,
    pub statuses: Vec<DecisionStatus>,
    pub actor_ids: Vec<String>,
    pub sources: Vec<String>,
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
    pub limit: usize,
    pub cursor: Option<String>,
    /// Ask from this project: a registered handle or a personal address. Results are limited to
    /// decisions filed under the project, the project it is part of, and the projects it depends
    /// on, in that order (own first, then the parent's, then a dependency's; see
    /// `project_scope`), each in the usual rank order. `None` searches the whole tenant, as
    /// before. An unregistered handle is refused, never an empty answer. Only the ledger-backed
    /// searches resolve a project; the graph-only `search_decisions` refuses one.
    pub project: Option<String>,
}

impl Default for SearchDecisionRequest {
    fn default() -> Self {
        Self {
            query: None,
            topic_keys: Vec::new(),
            statuses: Vec::new(),
            actor_ids: Vec::new(),
            sources: Vec::new(),
            since: None,
            until: None,
            limit: super::shared::DEFAULT_SEARCH_LIMIT,
            cursor: None,
            project: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DecisionSearchResults {
    pub query: Option<String>,
    pub filters: SearchDecisionFilters,
    pub limit: usize,
    pub cursor: Option<String>,
    pub next_cursor: Option<String>,
    pub total_matches: usize,
    pub items: Vec<DecisionSearchResult>,
    /// Where a project-scoped answer looked and where it stopped. Absent when the request named
    /// no project.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<ScopeNote>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct SearchDecisionFilters {
    pub topic_keys: Vec<String>,
    pub statuses: Vec<DecisionStatus>,
    pub actor_ids: Vec<String>,
    pub sources: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DecisionSearchResult {
    pub decision: DecisionView,
    pub rank: u8,
    pub matched_fields: Vec<String>,
    /// Terms of the question this decision does not contain. Empty for a full match; non-empty
    /// only for the close matches `recall` returns after the full ones, so a partial answer is
    /// never presented as a complete one.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub missing_terms: Vec<String>,
    pub snippets: Vec<SearchSnippet>,
    pub graph_context: SearchGraphContext,
    /// How this decision reached a project-scoped answer (own project, inherited from the
    /// parent, or from a dependency). Absent when the request named no project.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<MatchScope>,
    /// Other records of this same decision, linked `SAME_AS`: set only by `recall`, which shows a
    /// decision recorded more than once as one item (hivemind-83cj). `search` lists records.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub also_recorded_as: Vec<RecordedCopy>,
    /// When it was decided, as the graph dates it (see `ScoredDecisionSearchResult::decided_at`):
    /// what a caller that orders a window by time sorts on. Not on the wire; `GET /v1/graph` and
    /// `recent_decisions` already report it.
    #[serde(skip)]
    pub decided_at: Option<DateTime<Utc>>,
}

impl DecisionSearchResult {
    /// Takes the match of another record of the same decision onto this one: the best rank, the
    /// fields either matched, and only the words neither record lacks.
    fn absorb_record(&mut self, other: Self) {
        self.rank = self.rank.min(other.rank);
        for field in other.matched_fields {
            if !self.matched_fields.contains(&field) {
                self.matched_fields.push(field);
            }
        }
        self.missing_terms
            .retain(|term| other.missing_terms.contains(term));
    }
}

/// Shows each decision recorded more than once, linked `SAME_AS`, as one result: the record made
/// first among those that matched, carrying the best match of any of them and naming the other
/// records in `also_recorded_as` (hivemind-83cj). Results with no link pass through unchanged,
/// and nothing is folded on closeness.
pub(crate) fn fold_linked_results(
    graph: &impl GraphView,
    results: Vec<DecisionSearchResult>,
) -> Result<Vec<DecisionSearchResult>> {
    Ok(fold_linked(
        graph,
        results,
        |result| result.decision.id.as_str(),
        DecisionSearchResult::absorb_record,
    )?
    .into_iter()
    .map(|folded| DecisionSearchResult {
        also_recorded_as: folded.also_recorded_as,
        ..folded.item
    })
    .collect())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SearchSnippet {
    pub field: String,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SearchGraphContext {
    pub actor_ids: Vec<String>,
    /// The human whose delegated scope an agent's self-acceptance fell within
    /// (hivemind-zdsh.6), read off the Decision node's `delegated_by` property.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delegated_by: Option<String>,
    pub supersedes_decision_ids: Vec<String>,
    pub superseded_by_decision_ids: Vec<String>,
    pub option_ids: Vec<String>,
    pub evidence_ids: Vec<String>,
    pub hypotheses: Vec<HypothesisContext>,
    /// Whether anything was declared about what this decision rests on.
    pub grounding_state: GroundingState,
    /// Whether what this decision rests on changed, and whether anyone has said it still holds:
    /// the stale marker and the count of open re-examine items (hivemind-ottw.3). Absent when no
    /// premise changed under it. Derived by the same rules as `DecisionOutcome::reexamine`, which
    /// holds the detail (`verify` shows who looked again, when and why).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reexamine: Option<ReexamineMarker>,
    pub matched_nodes: Vec<SearchMatchedNode>,
}

/// Fills in each result's re-examine marker. Done on the page that is returned, never on the
/// decisions that were scored: a page reads the grounding facts in bulk once, however many
/// decisions the ledger holds, and a request that returns nothing reads nothing extra.
fn attach_reexamine_markers(
    graph: &impl GraphView,
    items: &mut [DecisionSearchResult],
) -> Result<()> {
    if items.is_empty() {
        return Ok(());
    }
    let markers = ReexamineMarkers::load(graph)?;
    for item in items {
        item.graph_context.reexamine = markers.of(&item.decision.id);
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct SearchMatchedNode {
    pub id: String,
    pub kind: NodeKind,
    pub field: String,
}

pub fn search_decisions(
    graph: &impl GraphView,
    request: &SearchDecisionRequest,
) -> Result<QueryResponse<DecisionSearchResults>> {
    let started = Instant::now();
    let query = normalized_query(request.query.as_deref());
    let terms = query_terms(query.as_deref());
    let topic_keys = normalized_topic_filter_values(&request.topic_keys);
    let statuses = normalized_statuses(&request.statuses);
    let actor_ids = normalized_filter_values(&request.actor_ids);
    let sources = normalized_filter_values(&request.sources);
    let limit = normalized_limit(request.limit);
    let cursor = normalized_query(request.cursor.as_deref());
    let offset = parse_cursor(cursor.as_deref())?;
    if request.since.is_some() || request.until.is_some() {
        return Err(query_error("timestamp filters require FTS-backed decision search").into());
    }
    // The project structure lives in the ledger, which this graph-only search never sees;
    // refuse rather than answer as though the whole tenant were the project.
    if request.project.is_some() {
        return Err(query_error("project scoping requires ledger-backed decision search").into());
    }

    let mut scored = collect_graph_search_results(
        graph,
        query.as_deref(),
        &SearchTerms::literal(&terms),
        &topic_keys,
        &statuses,
        &actor_ids,
        &sources,
    )?;

    scored.sort_by(|left, right| (left.rank, &left.id).cmp(&(right.rank, &right.id)));

    let total_matches = scored.len();
    let mut items: Vec<DecisionSearchResult> = scored
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|scored| scored.result)
        .collect();
    attach_reexamine_markers(graph, &mut items)?;
    let next_offset = offset.saturating_add(items.len());
    let next_cursor = (next_offset < total_matches).then(|| next_offset.to_string());

    Ok(QueryResponse {
        result_count: items.len(),
        truncated: next_cursor.is_some(),
        latency_ms: started.elapsed().as_millis(),
        data: DecisionSearchResults {
            query,
            filters: SearchDecisionFilters {
                topic_keys,
                statuses,
                actor_ids,
                sources,
                since: request.since,
                until: request.until,
            },
            limit,
            cursor,
            next_cursor,
            total_matches,
            items,
            scope: None,
        },
    })
}

pub fn search_decisions_fts(
    ledger: &SqliteEventLedger,
    graph: &impl GraphView,
    request: &SearchDecisionRequest,
) -> Result<QueryResponse<DecisionSearchResults>> {
    search_decisions_fts_with_context(&QueryContext::local(), ledger, graph, request)
}

/// Search decisions using in-memory graph matching with optional ledger-backed timestamp filtering.
///
/// This is the backend-agnostic search path: it works with any `EventLedger` (SQLite or Postgres)
/// and uses in-memory term matching rather than SQLite FTS. Ranking is by match rank + decision id —
/// the same ordinal scheme `search_decisions_fts_with_context` uses, so the two backends return
/// identical order for identical fixtures (see docs/SEARCH_DESIGN.md's Ordering Guarantees).
pub fn search_decisions_with_ledger(
    context: &QueryContext,
    ledger: &impl EventLedger,
    graph: &impl GraphView,
    request: &SearchDecisionRequest,
) -> Result<QueryResponse<DecisionSearchResults>> {
    search_with_ledger(context, ledger, graph, request, Matching::Literal)
}

/// Search for what a question asks about (the `recall` path): the request's text is already
/// stripped of question words, and a term matches a field word with the same stem as well as a
/// substring. A decision matching every term comes first; then, fewest missing terms first, the
/// ones matching at least half of them, each carrying `missing_terms` so a partial answer is
/// never presented as a complete one. Filters, project scope, ordering ties and pagination are
/// those of `search_decisions_with_ledger`.
///
/// `negated` says the question was asked with a negation ("why doesn't ..."), which the request's
/// text no longer carries. It never narrows the answer and is never a missing term: among
/// decisions that match equally, one whose own title does not say every term outright comes first.
///
/// One in-memory path for every backend: SQLite's FTS5 only matches whole tokens (every term,
/// exactly as written), which is the strictness this exists to relax.
pub fn search_decisions_fluent(
    context: &QueryContext,
    ledger: &impl EventLedger,
    graph: &impl GraphView,
    request: &SearchDecisionRequest,
    negated: bool,
) -> Result<QueryResponse<DecisionSearchResults>> {
    search_with_ledger(
        context,
        ledger,
        graph,
        request,
        Matching::Fluent { negated },
    )
}

/// How a search request's free text is matched against a decision.
#[derive(Clone, Copy)]
enum Matching {
    /// Every term as a substring (`search`).
    Literal,
    /// Stemmed terms, close matches after the full ones (`recall`); `negated` is the polarity of
    /// the question.
    Fluent { negated: bool },
}

fn search_with_ledger(
    context: &QueryContext,
    ledger: &impl EventLedger,
    graph: &impl GraphView,
    request: &SearchDecisionRequest,
    matching: Matching,
) -> Result<QueryResponse<DecisionSearchResults>> {
    let started = Instant::now();
    let query = normalized_query(request.query.as_deref());
    let terms = query_terms(query.as_deref());
    let search_terms = match matching {
        Matching::Literal => SearchTerms::literal(&terms),
        Matching::Fluent { negated } => SearchTerms::fluent(&terms, negated),
    };
    let topic_keys = normalized_topic_filter_values(&request.topic_keys);
    let statuses = normalized_statuses(&request.statuses);
    let actor_ids = normalized_filter_values(&request.actor_ids);
    let sources = normalized_filter_values(&request.sources);
    let since = request.since;
    let until = request.until;
    if let (Some(since), Some(until)) = (since, until) {
        if since > until {
            return Err(query_error("--since must be earlier than or equal to --until").into());
        }
    }
    let limit = normalized_limit(request.limit);
    let cursor = normalized_query(request.cursor.as_deref());
    let offset = parse_cursor(cursor.as_deref())?;
    let scope = resolve_scope(context, ledger, request.project.as_deref())?;

    let mut scored = collect_graph_search_results(
        graph,
        query.as_deref(),
        &search_terms,
        &topic_keys,
        &statuses,
        &actor_ids,
        &sources,
    )?;

    scored.retain(|doc| date_in_range(doc.decided_at, since, until));

    let scope_note = match &scope {
        Some(scope) => Some(narrow_to_scope(&mut scored, scope, graph)?),
        None => None,
    };
    sort_scored(&mut scored);

    let total_matches = scored.len();
    let mut items: Vec<DecisionSearchResult> = scored
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|s| s.result)
        .collect();
    attach_reexamine_markers(graph, &mut items)?;
    let next_offset = offset.saturating_add(items.len());
    let next_cursor = (next_offset < total_matches).then(|| next_offset.to_string());

    Ok(QueryResponse {
        result_count: items.len(),
        truncated: next_cursor.is_some(),
        latency_ms: started.elapsed().as_millis(),
        data: DecisionSearchResults {
            query,
            filters: SearchDecisionFilters {
                topic_keys,
                statuses,
                actor_ids,
                sources,
                since,
                until,
            },
            limit,
            cursor,
            next_cursor,
            total_matches,
            items,
            scope: scope_note,
        },
    })
}

/// How many words make a query a phrase (`search_decisions_any`).
const PHRASE_WORDS: usize = 3;

/// Backend-dispatching search: SQLite goes through FTS
/// (`search_decisions_fts_with_context`); Postgres goes through the
/// backend-agnostic in-memory path (`search_decisions_with_ledger`). Both
/// return identical order for identical fixtures (see docs/SEARCH_DESIGN.md's
/// Ordering Guarantees), so callers that only hold an `AnyLedger` — the CLI,
/// stdio MCP, and one-shot HTTP paths — never need to know which backend they
/// are on.
///
/// A phrase of at least `PHRASE_WORDS` words that no decision matches in full is asked again the
/// way `recall` asks (`search_decisions_fluent`), so what an agent wrote in its own words still
/// finds the decision that holds some of them. The words are counted as the query was given:
/// "where does the website live" is a phrase of five, though only "website live" is searched for
/// once the question words are dropped. A phrase that is nothing but question words ("what did we
/// decide") has nothing to search for and stays exact. That answer says what each decision lacks
/// (`DecisionSearchResult::missing_terms`) and echoes the query as it was given; a query some
/// decision matches in full, or of fewer words (a short query stays exact), is answered exactly
/// as before.
pub fn search_decisions_any(
    context: &QueryContext,
    ledger: &AnyLedger,
    graph: &impl GraphView,
    request: &SearchDecisionRequest,
) -> Result<QueryResponse<DecisionSearchResults>> {
    let strict = match ledger {
        AnyLedger::Sqlite(inner) => {
            search_decisions_fts_with_context(context, inner, graph, request)?
        }
        #[cfg(feature = "shared-backend-postgres")]
        AnyLedger::Postgres(inner) => search_decisions_with_ledger(context, inner, graph, request)?,
    };
    if strict.data.total_matches > 0 {
        return Ok(strict);
    }
    let Some(content) = request.query.as_deref().map(content_query) else {
        return Ok(strict);
    };
    if content.query.is_none() || query_terms(request.query.as_deref()).len() < PHRASE_WORDS {
        return Ok(strict);
    }
    let fluent_request = SearchDecisionRequest {
        query: content.query,
        ..request.clone()
    };
    let mut fluent =
        search_decisions_fluent(context, ledger, graph, &fluent_request, content.negated)?;
    fluent.data.query = strict.data.query;
    Ok(fluent)
}

pub fn search_decisions_fts_with_context(
    context: &QueryContext,
    ledger: &SqliteEventLedger,
    graph: &impl GraphView,
    request: &SearchDecisionRequest,
) -> Result<QueryResponse<DecisionSearchResults>> {
    let started = Instant::now();
    let query = normalized_query(request.query.as_deref());
    let terms = query_terms(query.as_deref());
    let topic_keys = normalized_topic_filter_values(&request.topic_keys);
    let statuses = normalized_statuses(&request.statuses);
    let actor_ids = normalized_filter_values(&request.actor_ids);
    let sources = normalized_filter_values(&request.sources);
    let since = request.since;
    let until = request.until;
    if let (Some(since), Some(until)) = (since, until) {
        if since > until {
            return Err(query_error("--since must be earlier than or equal to --until").into());
        }
    }
    let limit = normalized_limit(request.limit);
    let cursor = normalized_query(request.cursor.as_deref());
    let offset = parse_cursor(cursor.as_deref())?;
    let scope = resolve_scope(context, ledger, request.project.as_deref())?;

    let documents =
        collect_graph_search_results(graph, None, &SearchTerms::literal(&[]), &[], &[], &[], &[])?;
    let index = build_decision_search_fts(&documents)?;
    let fts_decision_ids = query_decision_search_fts(&index, query.as_deref())?;

    let mut documents_by_id = documents
        .into_iter()
        .map(|document| (document.id.clone(), document))
        .collect::<BTreeMap<_, _>>();
    let mut scored = Vec::new();
    for decision_id in fts_decision_ids {
        let Some(mut document) = documents_by_id.remove(&decision_id) else {
            continue;
        };
        if !date_in_range(document.decided_at, since, until) {
            continue;
        }
        if !document_matches_filters(&document, &topic_keys, &statuses, &actor_ids, &sources) {
            continue;
        }

        let match_info = match query.as_deref() {
            Some(_) => evaluate_search_match(
                query.as_deref(),
                &SearchTerms::literal(&terms),
                &document.fields,
            )
            .unwrap_or_else(|| SearchMatchInfo {
                rank: 3,
                ..SearchMatchInfo::default()
            }),
            None => SearchMatchInfo {
                rank: 4,
                ..SearchMatchInfo::default()
            },
        };
        document.rank = match_info.rank;
        document.result.rank = match_info.rank;
        document.result.matched_fields = match_info.matched_fields;
        document.result.snippets = match_info.snippets;
        document.result.graph_context.matched_nodes = match_info.matched_nodes;
        scored.push(document);
    }

    // Same ordinal (rank, id) ordering as search_decisions_with_ledger (the Postgres/generic
    // graph path): FTS5's MATCH clause above only selects the candidate set, never the order.
    // BM25 values are a SQLite-internal retrieval detail (docs/SEARCH_DESIGN.md's
    // Storage-Bound Behaviors) — keeping them out of the sort key is what gives both backends
    // identical order for identical fixtures. A project scope is the same filter on the
    // decision's project after FTS, so both backends narrow and group identically.
    let scope_note = match &scope {
        Some(scope) => Some(narrow_to_scope(&mut scored, scope, graph)?),
        None => None,
    };
    sort_scored(&mut scored);

    let total_matches = scored.len();
    let mut items: Vec<DecisionSearchResult> = scored
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|scored| scored.result)
        .collect();
    attach_reexamine_markers(graph, &mut items)?;
    let next_offset = offset.saturating_add(items.len());
    let next_cursor = (next_offset < total_matches).then(|| next_offset.to_string());

    Ok(QueryResponse {
        result_count: items.len(),
        truncated: next_cursor.is_some(),
        latency_ms: started.elapsed().as_millis(),
        data: DecisionSearchResults {
            query,
            filters: SearchDecisionFilters {
                topic_keys,
                statuses,
                actor_ids,
                sources,
                since,
                until,
            },
            limit,
            cursor,
            next_cursor,
            total_matches,
            items,
            scope: scope_note,
        },
    })
}

/// The project structure a scoped request is answered from, read once from the ledger; `None`
/// for an unscoped request. An unregistered handle is refused here, before any search work.
fn resolve_scope(
    context: &QueryContext,
    ledger: &impl EventLedger,
    project: Option<&str>,
) -> Result<Option<ProjectScope>> {
    let Some(project) = project else {
        return Ok(None);
    };
    let scoped_ledger = TenantScopedLedger::new(ledger, context.tenant_id.clone());
    project_scope(&scoped_ledger, project).map(Some)
}

/// Keep the documents filed under a project in `scope` and record how each one got there. A
/// decision with no project, or in a project the scope did not follow, is left out; the returned
/// note says which projects were looked in and where the walk stopped, so a short answer never
/// reads as a complete one. This is a filter on the decision's own `project`: one lookup per
/// document, no walk.
fn narrow_to_scope(
    scored: &mut Vec<ScoredDecisionSearchResult>,
    scope: &ProjectScope,
    graph: &impl GraphView,
) -> Result<ScopeNote> {
    let labels = ProjectLabels::from_graph(graph)?;
    scored.retain_mut(|document| {
        let decision = &document.result.decision;
        let Some(relation) = scope.relation_of(decision.project.as_deref()) else {
            return false;
        };
        let match_scope = scope.match_scope(relation, &decision.project_label, &labels);
        document.relation = Some(relation);
        document.result.scope = Some(match_scope);
        true
    });
    Ok(scope.note(&labels))
}

/// Own project first, then the parent's, then a dependency's; within each, first the close
/// matches that hold more of what the question is about than every decision that holds all of its
/// words (`out_heading`), then the decisions that hold every term, then the close matches. Within
/// the full matches and within the close matches, the one whose title, topic keys and recorded
/// question hold more of the question comes first (`About`); a close match that is no less about
/// it than another but lacks fewer of the terms comes before that one (a decision that holds all
/// but one of the words only in a long rationale is not ahead of the one whose title says them),
/// then the ones whose title or topic keys carry more of the terms they did match. Then (for a
/// decision below the bar, see `admit_below_bar`) the one that names a word it holds in both its
/// title and its topic keys, then the ones that match fewer of the terms only through a stand-in
/// word (a decision that has the word asked for comes before one that has a synonym), then
/// (rank, id) order. A negated question does not change that order: among decisions tied on all
/// of it, the ones whose title does not say every term outright come before the ones whose
/// title does. Among decisions tied on all of that, an accepted decision comes before one that is
/// still proposed (`ScoredDecisionSearchResult::accepted`), so a draft that discusses a decision
/// is not listed above it for the sake of an id. An unscoped document has no relation, a literal
/// match lacks nothing, is never negated, is about nothing and is never `accepted`, so an
/// unscoped `search` keeps the plain (rank, id) order.
fn sort_scored(scored: &mut [ScoredDecisionSearchResult]) {
    mark_out_heading(scored);
    scored.sort_by(|left, right| {
        left.relation
            .cmp(&right.relation)
            .then_with(|| right.promoted.cmp(&left.promoted))
            .then_with(|| {
                right
                    .result
                    .missing_terms
                    .is_empty()
                    .cmp(&left.result.missing_terms.is_empty())
            })
            .then_with(|| right.about.cmp_about(&left.about))
            .then_with(|| {
                left.result
                    .missing_terms
                    .len()
                    .cmp(&right.result.missing_terms.len())
            })
            .then_with(|| right.headline_terms.cmp(&left.headline_terms))
            .then_with(|| right.headline_hits.cmp(&left.headline_hits))
            .then_with(|| left.stand_ins.cmp(&right.stand_ins))
            .then_with(|| left.rank.cmp(&right.rank))
            .then_with(|| left.polarity_mismatch.cmp(&right.polarity_mismatch))
            .then_with(|| right.accepted.cmp(&left.accepted))
            .then_with(|| left.id.cmp(&right.id))
    });
}

/// Sets `promoted` on the close matches that `out_heading` puts ahead of the full ones, within
/// each project relation (an own-project decision is never weighed against a parent's).
fn mark_out_heading(scored: &mut [ScoredDecisionSearchResult]) {
    let mut groups: BTreeMap<Option<ScopeRelation>, Vec<usize>> = BTreeMap::new();
    for (index, document) in scored.iter().enumerate() {
        groups.entry(document.relation).or_default().push(index);
    }
    let mut promoted = vec![false; scored.len()];
    for indices in groups.values() {
        let standings: Vec<Standing> = indices
            .iter()
            .filter_map(|&index| scored.get(index))
            .map(|document| Standing {
                missing: document.result.missing_terms.len(),
                opposite: document.polarity_mismatch,
                about: document.about,
            })
            .collect();
        for (&index, flag) in indices.iter().zip(out_heading(&standings)) {
            if let Some(slot) = promoted.get_mut(index) {
                *slot = flag;
            }
        }
    }
    for (document, flag) in scored.iter_mut().zip(promoted) {
        document.promoted = flag;
    }
}

/// What a question is about, for one decision: how much of the question its own title, topic keys
/// and recorded question carry. A decision about a thing says it in its headline; one that merely
/// holds the same words somewhere in a long rationale, its evidence or the fields of whoever
/// recorded it is a weaker answer, however many of the words it holds. A word counts for what it
/// is worth (`TermWeights`), so the one rare word of a question outweighs the generic ones around
/// it, and a word the question says in two forms ("job", "jobs") is one word of a decision, counted
/// once (`TermWeights::once`). A title or topic key that says a word only in other words (`WORD_GROUPS`: "graphs" for the
/// "diagrams" asked about, an "edge" for the "arrow") is about the question too, for
/// `STAND_IN_CREDIT` of what the word itself would be worth (hivemind-ctok).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct About {
    /// The weight of the question's words that the title, a topic key or the question the
    /// decision records holds as a word or a form of one, never as part of a longer word, plus
    /// `STAND_IN_CREDIT` of the weight of each word its title or a topic key says only through a
    /// stand-in.
    weight: f64,
    /// The part of `weight` the title itself holds: the title states what was decided, a topic
    /// key only files it, so between equal weights the decision that says it in its title is the
    /// one about it.
    title: f64,
}

impl About {
    /// The decision the question names: its id or title is the question, or it records the
    /// question as asked. Nothing is more about the question than that.
    const NAMED: Self = Self {
        weight: f64::INFINITY,
        title: f64::INFINITY,
    };

    pub(crate) fn cmp_about(&self, other: &Self) -> Ordering {
        self.weight
            .total_cmp(&other.weight)
            .then_with(|| self.title.total_cmp(&other.title))
    }

    /// Whether the question names this decision outright (see `NAMED`), so nothing about where
    /// its words sit is left to weigh.
    pub(crate) fn is_named(&self) -> bool {
        self.weight.is_infinite()
    }

    /// This much of the question, but no more than `ceiling`.
    fn capped_at(self, ceiling: Self) -> Self {
        if self.cmp_about(&ceiling).is_gt() {
            ceiling
        } else {
            self
        }
    }

    /// Whether this is at least `ratio` of what `other` weighs.
    pub(crate) fn weighs_at_least(&self, ratio: f64, other: &Self) -> bool {
        self.weight >= ratio * other.weight
    }
}

/// What `out_heading` needs of one candidate.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Standing {
    /// How many of the question's terms the candidate lacks.
    pub(crate) missing: usize,
    /// The question is negated and the candidate's title says every term of it outright.
    pub(crate) opposite: bool,
    pub(crate) about: About,
}

/// Which of `candidates` are put ahead of the ones that hold every word asked. A decision that
/// holds every word only somewhere in a long rationale ("is the product still called HiveMind",
/// answered by a decision on a triage gate whose rationale says all three) is not the one the
/// question is about when another decision carries more of those words in its title, topic keys
/// and recorded question and lacks only one. The candidates that hold every term (of the right
/// polarity, when any do) say how much is about the question at best; a close match that carries
/// strictly more of it is promoted: listed before them, and for a verb that only reads, answered
/// with, saying what it lacks. Nothing else is moved, and a candidate of the opposite polarity is
/// never promoted. The basis is where the words sit and how many decisions hold each; nothing is
/// learned.
pub(crate) fn out_heading(candidates: &[Standing]) -> Vec<bool> {
    let best = |eligible_only: bool| {
        candidates
            .iter()
            .filter(|candidate| candidate.missing == 0 && !(eligible_only && candidate.opposite))
            .map(|candidate| candidate.about)
            .max_by(About::cmp_about)
    };
    let reference = best(true).or_else(|| best(false));
    candidates
        .iter()
        .map(|candidate| {
            candidate.missing > 0
                && !candidate.opposite
                && reference.is_some_and(|reference| candidate.about.cmp_about(&reference).is_gt())
        })
        .collect()
}

/// Builds the FTS5 index for one search in a private in-memory database and hands back the
/// connection that holds it. The index is a per-request scratch table: nothing is written to the
/// ledger file, so a search never takes the ledger's write lock and two concurrent searches
/// (including ones for different tenants) never see each other's documents.
// ubs:ignore: This helper only executes static FTS SQL and uses rusqlite params! for document values.
fn build_decision_search_fts(
    // ubs:ignore: FTS build uses static SQL plus rusqlite parameter binding; documents are bound as parameters.
    documents: &[ScoredDecisionSearchResult],
    // ubs:ignore: FTS build uses static SQL plus rusqlite parameter binding.
) -> Result<Connection> {
    let mut connection = Connection::open_in_memory()
        .map_err(|error| query_error(format!("open decision search index: {error}")))?;
    let transaction = connection
        .transaction()
        .map_err(|error| query_error(format!("begin decision search index rebuild: {error}")))?;
    // ubs:ignore: FTS schema SQL is static; no user input is interpolated into this statement.
    transaction
        // ubs:ignore: static FTS schema SQL contains no request-controlled interpolation.
        .execute_batch(
            "DROP TABLE IF EXISTS decision_search_fts;
             CREATE VIRTUAL TABLE decision_search_fts USING fts5(
                 decision_id,
                 title,
                 rationale,
                 topic_keys,
                 status,
                 actor_text,
                 source,
                 option_text,
                 evidence_text,
                 hypothesis_text,
                 supersession_text,
                 question_text,
                 tokenize = 'unicode61'
             );",
        )
        .map_err(|error| query_error(format!("initialize decision search index: {error}")))?;
    {
        // ubs:ignore: FTS insert SQL is static and dynamic values are bound with rusqlite params!.
        let mut statement = transaction
            // ubs:ignore: static INSERT statement; values are bound via rusqlite params.
            .prepare(
                "INSERT INTO decision_search_fts (
                    decision_id,
                    title,
                    rationale,
                    topic_keys,
                    status,
                    actor_text,
                    source,
                    option_text,
                    evidence_text,
                    hypothesis_text,
                    supersession_text,
                    question_text
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            )
            .map_err(|error| {
                query_error(format!("prepare decision search index insert: {error}"))
            })?;
        for document in documents {
            statement
                // ubs:ignore: document fields are bound parameters, not interpolated SQL.
                .execute(params![
                    field_text(&document.fields, &["decision.id"]),
                    field_text(&document.fields, &["decision.title"]),
                    field_text(&document.fields, &["decision.rationale"]),
                    field_text(&document.fields, &["decision.topic"]),
                    field_text(&document.fields, &["decision.status"]),
                    field_text(&document.fields, &["actor.id", "actor.source_ref"]),
                    field_text(&document.fields, &["decision.source"]),
                    field_text(
                        &document.fields,
                        &["option.id", "option.label", "option.description"],
                    ),
                    field_text(&document.fields, &["evidence.id", "evidence.content"]),
                    field_text(&document.fields, &["hypothesis.id", "hypothesis.statement"]),
                    field_text(&document.fields, &["supersedes.id", "superseded_by.id"]),
                    field_text(&document.fields, &["decision.question"]),
                ])
                .map_err(|error| {
                    query_error(format!(
                        "insert decision search index row for {}: {error}",
                        document.id
                    ))
                })?;
        }
    }
    transaction
        .commit()
        .map_err(|error| query_error(format!("commit decision search index build: {error}")))?;
    Ok(connection)
}

/// Returns the decision ids FTS5 considers a match for `query` (or every indexed decision id
/// when `query` is `None`), in no particular order — the caller re-ranks by the shared
/// (rank, id) ordinal scheme, so FTS5 here only selects the candidate set.
fn query_decision_search_fts(
    // ubs:ignore: FTS query uses static SQL with MATCH bound via rusqlite params.
    connection: &Connection,
    // ubs:ignore: Caller text is converted to an FTS expression and bound as ?1.
    query: Option<&str>,
    // ubs:ignore: SQL text in this function is static and parameterized.
) -> Result<Vec<String>> {
    let mut decision_ids = Vec::new();
    // ubs:ignore: Query text is converted to an FTS expression and bound as ?1 below.
    if let Some(query) = query {
        let Some(fts_query) = fts5_query(query) else {
            return Ok(Vec::new());
        };
        // ubs:ignore: SQL statement is static; the FTS expression is passed via params![fts_query].
        let mut statement = connection
            // ubs:ignore: static SELECT statement; FTS query is bound as parameter ?1.
            .prepare(
                "SELECT decision_id
                   FROM decision_search_fts
                  WHERE decision_search_fts MATCH ?1
                  ORDER BY decision_id ASC",
            )
            .map_err(|error| query_error(format!("prepare decision search query: {error}")))?;
        let rows = statement
            // ubs:ignore: FTS query value is bound through rusqlite params.
            .query_map(params![fts_query], |row| row.get::<_, String>(0))
            .map_err(|error| query_error(format!("execute decision search query: {error}")))?;
        for row in rows {
            decision_ids
                .push(row.map_err(|error| query_error(format!("read search row: {error}")))?);
        }
    } else {
        // ubs:ignore: SQL statement is static and has no caller-controlled interpolation.
        let mut statement = connection
            // ubs:ignore: static unfiltered SELECT contains no request-controlled interpolation.
            .prepare("SELECT decision_id FROM decision_search_fts ORDER BY decision_id ASC")
            .map_err(|error| query_error(format!("prepare unfiltered decision search: {error}")))?;
        let rows = statement
            // ubs:ignore: static unfiltered SELECT has no user-supplied SQL fragments.
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| query_error(format!("execute unfiltered decision search: {error}")))?;
        for row in rows {
            decision_ids
                .push(row.map_err(|error| query_error(format!("read search row: {error}")))?);
        }
    }
    Ok(decision_ids)
}

/// Whether a decision decided at `decided_at` falls in `[since, until]`. With no bound every
/// decision does; with one, a decision the graph has no time for does not.
fn date_in_range(
    decided_at: Option<DateTime<Utc>>,
    since: Option<DateTime<Utc>>,
    until: Option<DateTime<Utc>>,
) -> bool {
    if since.is_none() && until.is_none() {
        return true;
    }
    let Some(decided_at) = decided_at else {
        return false;
    };
    since.is_none_or(|since| decided_at >= since) && until.is_none_or(|until| decided_at <= until)
}

fn document_matches_filters(
    document: &ScoredDecisionSearchResult,
    topic_keys: &[String],
    statuses: &[DecisionStatus],
    actor_ids: &[String],
    sources: &[String],
) -> bool {
    let decision = &document.result.decision;
    if !topic_keys.is_empty()
        && !topic_keys
            .iter()
            .all(|topic| stored_topics_include(&decision.topic_keys, topic))
    {
        return false;
    }
    if !statuses.is_empty() && !statuses.contains(&decision.status) {
        return false;
    }
    if !actor_ids.is_empty()
        && !actor_ids.iter().any(|actor_id| {
            document
                .result
                .graph_context
                .actor_ids
                .iter()
                .any(|candidate| candidate == actor_id)
        })
    {
        return false;
    }
    if !sources.is_empty() {
        let source = field_text(&document.fields, &["decision.source"]);
        if !sources
            .iter()
            .any(|expected| source.eq_ignore_ascii_case(expected))
        {
            return false;
        }
    }
    true
}

fn field_text(fields: &[SearchField], field_names: &[&str]) -> String {
    fields
        .iter()
        .filter(|field| field_names.iter().any(|name| field.field == *name))
        .map(|field| field.value.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

fn fts5_query(query: &str) -> Option<String> {
    let terms = query
        .split(|character: char| !character.is_alphanumeric())
        .map(str::trim)
        .filter(|term| !term.is_empty())
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>();
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" AND "))
    }
}

struct ScoredDecisionSearchResult {
    /// How the decision's project relates to the asked project; `None` for an unscoped request.
    relation: Option<ScopeRelation>,
    rank: u8,
    id: String,
    event_origin: i64,
    /// The decision is accepted, and the asker matches words (a fluent or resolver ask; a literal
    /// search never sets it, so it keeps its plain (rank, id) order on every backend). Among
    /// decisions that tie on everything else that says how fully a question matches, the accepted
    /// one is listed before a proposed draft, which the id or recency would otherwise place first
    /// (`capture:` sorts before `decision-`, and a draft is newer than the decision it discusses).
    accepted: bool,
    /// When the decision was decided, as the graph dates it (the Decision node's `occurred_at`,
    /// which `GET /v1/graph` reports as `decided_at`): a proposal's own time, a classifier
    /// draft's turn time. What `since`/`until` window on; `None` for a decision the graph has no
    /// time for.
    decided_at: Option<DateTime<Utc>>,
    /// `SearchMatchInfo::polarity_mismatch`.
    polarity_mismatch: bool,
    /// `SearchMatchInfo::headline_terms`: for a close match, how many of the terms its title or
    /// topic keys contain.
    headline_terms: usize,
    /// `SearchMatchInfo::headline_hits` for a decision below the bar, else 0.
    headline_hits: usize,
    /// `SearchMatchInfo::headline_hits` for every decision: what orders the resolver's ties.
    headline_hit_count: usize,
    /// `SearchMatchInfo::stand_in_terms`.
    stand_ins: usize,
    /// How many of the question's words the decision's own title and topic keys hold as a word or
    /// a form of one (`SearchMatchInfo::headline_words`); 0 for a literal match.
    headline_words: usize,
    /// `SearchMatchInfo::headline_matched`.
    headline_matched: usize,
    /// Which of the question's terms (by position, bit `n` for term `n`) the decision's own title
    /// and topic keys hold as a word or a form of one: the set `headline_words` counts.
    headline_mask: u64,
    /// Which of the question's terms its title and topic keys hold in any form
    /// (`SearchMatchInfo::headline_any`).
    headline_any_mask: u64,
    /// How much of the question the decision's title, topic keys and recorded question carry
    /// (`About`); nothing for a literal match, which has no words to weigh.
    about: About,
    /// A close match that `out_heading` puts ahead of the full ones (set when sorting).
    promoted: bool,
    /// Admitted below the bar as a peer of the answer's best-titled decision
    /// (`peers_below_bar`): listed with the answer, and never named alone.
    peer: bool,
    /// Admitted below the bar, while a decision left out says more of the question
    /// (`outweighed_below_bar`): listed with the answer, and never named alone.
    outweighed: bool,
    result: DecisionSearchResult,
    fields: Vec<SearchField>,
}

fn collect_graph_search_results(
    graph: &impl GraphView,
    query: Option<&str>,
    terms: &SearchTerms<'_>,
    topic_keys: &[String],
    statuses: &[DecisionStatus],
    actor_ids: &[String],
    sources: &[String],
) -> Result<Vec<ScoredDecisionSearchResult>> {
    let decision_rows = node_rows(graph, NodeKind::Decision)?;
    let actor_rows = node_rows(graph, NodeKind::Actor)?;
    let evidence_rows = node_rows(graph, NodeKind::Evidence)?;
    let option_rows = node_rows(graph, NodeKind::Option)?;
    let hypothesis_rows = node_rows(graph, NodeKind::Hypothesis)?;
    let question_rows = node_rows(graph, NodeKind::Question)?;
    let edges = relation_edges_by_kind(graph)?;
    let labels = ProjectLabels::from_graph(graph)?;
    // What stands for or against a decision is read once for the whole ledger, never asked per
    // decision: a lookup each walks the graph once for every decision searched (hivemind-7jon8).
    let standings = DecisionStandings::load(graph)?;
    let targets_of = |relation: RelationKind| -> BTreeSet<&str> {
        edges
            .get(&relation)
            .into_iter()
            .flatten()
            .map(|(_, to)| to.as_str())
            .collect()
    };
    let refuted_hypotheses = targets_of(RelationKind::Refutes);
    let supported_hypotheses = targets_of(RelationKind::Supports);

    let mut scored = Vec::new();
    // What `admit_below_bar` needs of each decision in `scored`, in the same order.
    let mut holdings: Vec<Holding> = Vec::new();
    // The decisions that passed the filters: the ledger the terms are weighed against.
    let mut searched = 0_usize;
    for (id, row) in decision_rows {
        let project = optional_string(&row, "project");
        let project_label = labels.label_of(project.as_deref());
        let title = optional_string(&row, "title").unwrap_or_default();
        let rationale = optional_string(&row, "rationale").unwrap_or_default();
        let quote = optional_string(&row, "quote");
        let question = optional_string(&row, "question");
        let delegated_by = optional_string(&row, "delegated_by");
        let event_origin = optional_int(&row, "event_origin").unwrap_or(0);
        let decided_at = optional_datetime(&row, "occurred_at")?;
        let decision_topic_keys = optional_string_list(&row, "topic_keys");
        if !topic_keys.is_empty()
            && !topic_keys
                .iter()
                .all(|topic| stored_topics_include(&decision_topic_keys, topic))
        {
            continue;
        }

        let status = standings.status_of(&id);
        if !statuses.is_empty() && !statuses.contains(&status) {
            continue;
        }

        let actor_ids_for_decision = relation_targets(
            &edges,
            &[
                RelationKind::ProposedBy,
                RelationKind::AcceptedBy,
                RelationKind::RejectedBy,
            ],
            &id,
        );
        if !actor_ids.is_empty()
            && !actor_ids
                .iter()
                .any(|actor_id| actor_ids_for_decision.iter().any(|id| id == actor_id))
        {
            continue;
        }

        let source = optional_string(&row, "source").unwrap_or_default();
        if !sources.is_empty()
            && !sources
                .iter()
                .any(|expected| source.eq_ignore_ascii_case(expected))
        {
            continue;
        }

        let option_ids = relation_targets(&edges, &[RelationKind::HasOption], &id);
        let chosen_option_id = relation_targets(&edges, &[RelationKind::Chose], &id)
            .into_iter()
            .next();
        let evidence_ids = relation_targets(&edges, &[RelationKind::BasedOn], &id);
        let mut hypothesis_ids = relation_targets(&edges, &[RelationKind::PremisedOnDirect], &id);
        if let Some(opt_id) = &chosen_option_id {
            hypothesis_ids.extend(relation_targets(
                &edges,
                &[RelationKind::PremisedOn],
                opt_id,
            ));
            hypothesis_ids.sort();
            hypothesis_ids.dedup();
        }
        let supersedes_decision_ids = relation_targets(&edges, &[RelationKind::Supersedes], &id);
        let superseded_by_decision_ids = relation_sources(&edges, RelationKind::Supersedes, &id);
        let premise_decision_ids = relation_targets(&edges, &[RelationKind::FollowsFrom], &id);
        let question_id = relation_targets(&edges, &[RelationKind::Answers], &id)
            .into_iter()
            .next();

        let mut hypotheses = Vec::with_capacity(hypothesis_ids.len());
        for hypothesis_id in &hypothesis_ids {
            let facts = match hypothesis_rows.get(hypothesis_id) {
                Some(row) => hypothesis_facts_from_row(row)?,
                None => hypothesis_facts_from_row(&GraphRow::new())?,
            };
            hypotheses.push(HypothesisContext {
                id: hypothesis_id.clone(),
                status: hypothesis_status_from_evidence(
                    refuted_hypotheses.contains(hypothesis_id.as_str()),
                    supported_hypotheses.contains(hypothesis_id.as_str()),
                ),
                kind: facts.kind,
                check_by: facts.check_by,
                would_change_if: facts.would_change_if,
            });
        }

        // A decision linked to a question after the fact has no question text of its own.
        let question = question.or_else(|| {
            question_id
                .as_ref()
                .and_then(|question_id| question_rows.get(question_id))
                .and_then(|row| optional_string(row, "text"))
        });

        let mut fields = Vec::new();
        fields.push(SearchField::decision("decision.id", &id, 0));
        fields.push(SearchField::decision("decision.title", &title, 1));
        fields.push(SearchField::decision("decision.rationale", &rationale, 2));
        // The question a decision answers is often the only text a person would ask with: a
        // decision the ask hooks write is titled "<header>: <choice>" with a fixed rationale.
        if let Some(question) = &question {
            fields.push(SearchField::decision("decision.question", question, 2));
        }
        for topic_key in &decision_topic_keys {
            fields.push(SearchField::decision("decision.topic", topic_key, 3));
        }
        fields.push(SearchField::decision(
            "decision.status",
            decision_status_label(status),
            3,
        ));
        if !source.is_empty() {
            fields.push(SearchField::decision("decision.source", &source, 3));
        }
        for actor_id in &actor_ids_for_decision {
            fields.push(SearchField::node(
                "actor.id",
                actor_id,
                3,
                NodeKind::Actor,
                actor_id,
            ));
            if let Some(actor) = actor_rows.get(actor_id) {
                if let Some(source_ref) = optional_string(actor, "source_ref") {
                    fields.push(SearchField::node(
                        "actor.source_ref",
                        &source_ref,
                        3,
                        NodeKind::Actor,
                        actor_id,
                    ));
                }
            }
        }
        for option_id in &option_ids {
            add_node_search_fields(
                &mut fields,
                &option_rows,
                NodeKind::Option,
                option_id,
                &[
                    ("option.id", "id"),
                    ("option.label", "label"),
                    ("option.description", "description"),
                ],
            );
        }
        for evidence_id in &evidence_ids {
            add_node_search_fields(
                &mut fields,
                &evidence_rows,
                NodeKind::Evidence,
                evidence_id,
                &[("evidence.id", "id"), ("evidence.content", "content")],
            );
        }
        for hypothesis_id in &hypothesis_ids {
            add_node_search_fields(
                &mut fields,
                &hypothesis_rows,
                NodeKind::Hypothesis,
                hypothesis_id,
                &[
                    ("hypothesis.id", "id"),
                    ("hypothesis.statement", "statement"),
                ],
            );
        }
        for decision_id in &supersedes_decision_ids {
            fields.push(SearchField::node(
                "supersedes.id",
                decision_id,
                3,
                NodeKind::Decision,
                decision_id,
            ));
        }
        for decision_id in &superseded_by_decision_ids {
            fields.push(SearchField::node(
                "superseded_by.id",
                decision_id,
                3,
                NodeKind::Decision,
                decision_id,
            ));
        }

        searched += 1;
        let Some(match_info) = evaluate_search_match(query, terms, &fields) else {
            continue;
        };
        let headline_words = match_info.headline_words.len();
        let headline_mask = term_mask(&match_info.headline_words);
        let headline_any_mask = term_mask(&match_info.headline_any);
        let reached: BTreeSet<usize> = match_info
            .held_words
            .union(&match_info.stand_in_reached)
            .copied()
            .collect();
        holdings.push(Holding {
            below_bar: match_info.below_bar,
            held: match_info.held_words,
            headline: match_info.headline_words,
            title: match_info.title_words,
            about: match_info.about_words,
            spelled: match_info.spelled_words,
            stand_in_headline: match_info.stand_in_headline,
            stand_in_title: match_info.stand_in_title,
            reached,
            named: match_info.named && terms.close != CloseMatch::Never,
        });

        let decision = DecisionView {
            id: id.clone(),
            title,
            rationale,
            topic_keys: decision_topic_keys,
            status,
            project,
            project_label,
            chosen_option_id,
            option_ids: option_ids.clone(),
            evidence_ids: evidence_ids.clone(),
            hypotheses: hypotheses.clone(),
            premise_decision_ids,
            quote: quote.clone(), // ubs:ignore: clone necessary — building owned DecisionView
            question,
            question_id,
            floor: recorded_floor_of(&row),
        };
        let grounding_state = decision.grounding_state();

        scored.push(ScoredDecisionSearchResult {
            relation: None,
            rank: match_info.rank,
            id,
            event_origin,
            accepted: terms.stemmed && status == DecisionStatus::Accepted,
            decided_at,
            polarity_mismatch: match_info.polarity_mismatch,
            headline_terms: match_info.headline_terms,
            headline_hits: if match_info.below_bar {
                match_info.headline_hits
            } else {
                0
            },
            headline_hit_count: match_info.headline_hits,
            stand_ins: match_info.stand_in_terms,
            headline_words,
            headline_matched: match_info.headline_matched,
            headline_mask,
            headline_any_mask,
            about: About::default(),
            promoted: false,
            peer: false,
            outweighed: false,
            fields,
            result: DecisionSearchResult {
                decision,
                rank: match_info.rank,
                matched_fields: match_info.matched_fields,
                missing_terms: match_info.missing_terms,
                snippets: match_info.snippets,
                graph_context: SearchGraphContext {
                    actor_ids: actor_ids_for_decision,
                    delegated_by,
                    supersedes_decision_ids,
                    superseded_by_decision_ids,
                    option_ids,
                    evidence_ids,
                    hypotheses,
                    grounding_state,
                    reexamine: None,
                    matched_nodes: match_info.matched_nodes,
                },
                scope: None,
                also_recorded_as: Vec::new(),
                decided_at,
            },
        });
    }

    let weights = TermWeights::new(&holdings, searched, terms.terms);
    for (decision, holding) in scored.iter_mut().zip(&holdings) {
        decision.about = holding.about(&weights);
    }
    Ok(admit_below_bar(
        scored,
        holdings,
        &weights,
        terms.terms.len(),
    ))
}

/// The terms of a question, by position, as bits of one number: bit `n` is term `n`. A question
/// has far fewer than 64 terms; a term past that is left out.
fn term_mask(terms: &BTreeSet<usize>) -> u64 {
    terms
        .iter()
        .filter(|term| **term < u64::BITS as usize)
        .fold(0, |mask, term| mask | (1 << term))
}

/// What `admit_below_bar` and `About` know of a matched decision.
struct Holding {
    /// `SearchMatchInfo::below_bar`.
    below_bar: bool,
    /// `SearchMatchInfo::held_words`.
    held: BTreeSet<usize>,
    /// `SearchMatchInfo::headline_words`.
    headline: BTreeSet<usize>,
    /// `SearchMatchInfo::title_words`.
    title: BTreeSet<usize>,
    /// `SearchMatchInfo::about_words`.
    about: BTreeSet<usize>,
    /// `SearchMatchInfo::spelled_words`.
    spelled: BTreeSet<usize>,
    /// `SearchMatchInfo::stand_in_headline`.
    stand_in_headline: BTreeSet<usize>,
    /// `SearchMatchInfo::stand_in_title`.
    stand_in_title: BTreeSet<usize>,
    /// The terms some field holds as a word, a form of one or a stand-in.
    reached: BTreeSet<usize>,
    /// The question names this decision (`SearchMatchInfo::named`), for a request that matches
    /// words at all.
    named: bool,
}

impl Holding {
    /// How much of the question this decision's title, topic keys and recorded question carry.
    fn about(&self, weights: &TermWeights) -> About {
        if self.named {
            return About::NAMED;
        }
        let sum = |terms: &BTreeSet<usize>| -> f64 {
            weights
                .once(terms)
                .iter()
                .map(|term| weights.weight_of(*term))
                .sum()
        };
        let stand_in = |terms: &BTreeSet<usize>, held: &BTreeSet<usize>| -> f64 {
            let held = weights.once(held);
            weights
                .once(terms)
                .iter()
                .filter(|term| !held.contains(term))
                .map(|term| weights.stand_in_weight(*term))
                .sum()
        };
        About {
            weight: sum(&self.about) + stand_in(&self.stand_in_headline, &self.about),
            title: sum(&self.title) + stand_in(&self.stand_in_title, &self.title),
        }
    }
}

/// How rare each word of a question is among the decisions searched: a word weighs
/// `ln((searched + 1) / (holders + 1/2))`, `holders` being the matched decisions that hold it as a
/// word or a form of one, so a word few decisions hold weighs the most and a word nobody holds
/// weighs more still. The counts are the ledger's own and nothing is learned.
struct TermWeights {
    /// How many matched decisions hold each term (by position in the question's terms).
    holders: HashMap<usize, usize>,
    /// How many matched decisions hold each term as a word, a form of one or a stand-in.
    reachers: HashMap<usize, usize>,
    /// How many matched decisions spell each term together with the term after it in their title
    /// or topic keys (`Holding::spelled`).
    spellers: HashMap<usize, usize>,
    /// How many decisions were searched.
    searched: usize,
    /// For each term (by position), the first term that has its stem: "job" and "jobs" are two
    /// terms of a question and one word of a decision (see `TermWeights::once`).
    first_of_stem: Vec<usize>,
}

impl TermWeights {
    fn new(holdings: &[Holding], searched: usize, terms: &[String]) -> Self {
        let mut holders: HashMap<usize, usize> = HashMap::new();
        let mut reachers: HashMap<usize, usize> = HashMap::new();
        let mut spellers: HashMap<usize, usize> = HashMap::new();
        for holding in holdings {
            for term in &holding.spelled {
                *spellers.entry(*term).or_default() += 1;
            }
            for term in &holding.held {
                *holders.entry(*term).or_default() += 1;
            }
            for term in &holding.reached {
                *reachers.entry(*term).or_default() += 1;
            }
        }
        let first_of_stem = terms
            .iter()
            .map(|term| {
                terms
                    .iter()
                    .position(|other| stem(other) == stem(term))
                    .unwrap_or_default()
            })
            .collect();
        Self {
            holders,
            reachers,
            spellers,
            searched,
            first_of_stem,
        }
    }

    /// The words of the question that `terms` hold, each stem once: a decision that says "job"
    /// holds the question's "job" and its "jobs" alike, and that is one word it shares with the
    /// question, not two (hivemind-otruj).
    fn once(&self, terms: &BTreeSet<usize>) -> BTreeSet<usize> {
        terms
            .iter()
            .map(|term| self.first_of_stem.get(*term).copied().unwrap_or(*term))
            .collect()
    }

    /// The decisions searched, plus one.
    fn ledger(&self) -> f64 {
        self.searched as f64 + 1.0
    }

    /// The weight of a word that `count` decisions hold.
    fn weight_for(&self, count: usize) -> f64 {
        (self.ledger() / (count as f64 + 0.5)).ln()
    }

    /// What a term is worth to a decision whose title or topic keys hold it only through a
    /// stand-in word: `STAND_IN_CREDIT` of what the term weighs when rarity counts every decision
    /// that reaches it, as the word, a form of one or a stand-in. Nobody says "diagram" as the
    /// word when the ledger says "graph", so counting only the decisions that hold the word
    /// itself would weigh it as the rarest word there is.
    fn stand_in_weight(&self, term: usize) -> f64 {
        let reached = self.reachers.get(&term).copied().unwrap_or(0);
        STAND_IN_CREDIT * self.weight_for(reached)
    }

    /// The weight of a term some decision holds; nothing for one that none does.
    fn weight_of(&self, term: usize) -> f64 {
        self.holders
            .get(&term)
            .map_or(0.0, |count| self.weight_for(*count))
    }
}

/// What a title or topic key that says a word of the question only through a stand-in word is worth,
/// next to one that says the word itself: half. A decision that puts the question in other words
/// ("graphs" for "diagrams", "edge" for "arrow") is about it, but the word asked for says it better
/// (`WordMatch`).
const STAND_IN_CREDIT: f64 = 0.5;

/// Fewer decisions than this say too little about which words are rare, so a lone word is trusted
/// when it is the one thing a decision's own title or topic keys name (see `admit_below_bar`).
const SMALL_LEDGER: usize = 16;
/// The words a decision shares with the question must be, together, this many times rarer than a
/// word held by one decision of the `searched + 1` (natural log of the odds), see `admit_below_bar`.
const SHARED_WORDS_RARITY: f64 = 1.25;
/// ... and carry at least this share of the weight of all the question's words.
const SHARED_WORDS_SHARE: f64 = 0.2;
/// ... and be at least this many words, or ...
const SHARED_WORDS_MIN: usize = 3;
/// ... this many of them words the decision's own title or topic keys hold.
const SHARED_HEADLINE_WORDS_MIN: usize = 2;
/// The words a decision's own title or topic keys hold must be, together, this many times rarer
/// than a word held by one decision of the `searched + 1`, see `admit_below_bar`: one word held by
/// four of sixty-nine decisions or by thirty-nine of ten thousand ...
const SUBJECT_RARITY: f64 = 0.6;
/// ... and carry at least this share of the weight of all the question's words.
const SUBJECT_SHARE: f64 = 0.4;
/// The question's subject word, the rarest word any decision holds, rules only when the words
/// some decision holds carry at least this share of the weight of all the question's words (the
/// rest is words nobody holds), see `subject_below_bar`.
const SUBJECT_WORD_KNOWN_SHARE: f64 = 2.0 / 3.0;
/// A pair of words is rare when no more than this many decisions hold each of them, see
/// `admit_below_bar`: a decision holding two words of the question that nearly nobody else holds,
/// one of them in its title or topic keys.
const RARE_PAIR_HOLDERS: usize = 2;
/// At most this many decisions below the bar are added to an answer.
const BELOW_BAR_LIMIT: usize = 3;
/// Two words of a title that the question does not say side by side are a pair for
/// `titled_below_bar` only when the decision holds this many of the question's words in all.
const FAR_PAIR_WORDS_MIN: usize = 5;
/// A decision left out below the bar outweighs one admitted there (`outweighed_below_bar`) only
/// when it holds this many of the question's words, one of them in its title or topic keys.
const OUTWEIGHING_WORDS_MIN: usize = 2;
/// The decision that owns the question's one rare word (`lone_below_bar`) is added only when it
/// holds this many of the question's words.
const LONE_SUBJECT_WORDS_MIN: usize = 2;
/// The question's one rare word may be held by this many decisions at most when every one of them
/// says it in its title or topic keys (`lone_owners_below_bar`).
const LONE_OWNERS_HOLDERS_MAX: usize = 3;
/// ... and the words some decision holds must carry this share of the weight of all the question's
/// words: a question with words nobody holds is mostly about something the ledger lacks.
const LONE_OWNERS_KNOWN_SHARE: f64 = 0.85;
/// The word that a decision is filed under (`dominant_owner_below_bar`) weighs at least this times
/// `ln(searched + 1)`: a word four decisions hold, in a ledger of eighteen hundred.
const DOMINANT_WORD_RARITY: f64 = 0.78;
/// ... and the answer is at least this many decisions: a question whose everyday words many
/// decisions hold, and whose rare word none of them titles.
const DOMINANT_ANSWER_MIN: usize = 4;
/// ... and the decision is filed under the word: its title and its topic keys say it, which is
/// `SearchMatchInfo::headline_hits` of at least this many.
const DOMINANT_FILED_HITS_MIN: usize = 2;
/// The word that exactly one decision says in its title (`title_owner_below_bar`) weighs at least
/// this times `ln(searched + 1)`: a word five or six decisions of two thousand hold, whereas the
/// dominant word of `DOMINANT_WORD_RARITY` is held by four, so that one more decision saying it in
/// passing does not take the title's owner out of the answer.
const TITLE_OWNER_RARITY: f64 = 0.75;

/// Which decisions that lack too many of the terms (`SearchMatchInfo::below_bar`) are still
/// answered. A long question names one thing among generic words ("CLI query xpath css text
/// extraction"), and the half-of-the-terms bar loses it. A word weighs the rarer it is among the
/// `searched` decisions: `ln((searched + 1) / (holders + 1/2))`, and a word nobody holds weighs the
/// most of all, so a question mostly about what the ledger lacks never passes. A decision below the
/// bar is answered when
///
/// - it holds the terms `SHARED_WORDS_MIN` or more of them, or `SHARED_HEADLINE_WORDS_MIN` that its
///   title or topic keys hold, as words that together weigh at least `SHARED_WORDS_RARITY` times
///   `ln(searched + 1)` and `SHARED_WORDS_SHARE` of all the terms' weight: that many words few
///   decisions hold do not meet by chance, whereas two that sit in a long rationale do ("load" and
///   "timeout" of a load balancer question), or
/// - it holds two of the terms, one in its title or topic keys, that `RARE_PAIR_HOLDERS` decisions
///   or fewer each hold, with the weight tests of the first ("how far does the forecast's leftover
///   tail reach?" against the decision titled for the leftover that says "tail" in its rationale,
///   whereas "forecast" is held by six decisions and "far" and "reach" by one each), or
/// - the ledger is small (`SMALL_LEDGER`), so counts say little, and a term the decision's title or
///   topic keys hold is held by no other decision: its capturer named it as the subject, or
/// - the words its title or topic keys hold weigh, together, at least `SUBJECT_RARITY` times
///   `ln(searched + 1)` and `SUBJECT_SHARE` of all the terms' weight: the question is mostly about
///   what few decisions say, and this decision says it in its title ("what licence is the code
///   under?" against the one decision titled for the licence, whereas "code" and "under" are held by
///   a decision in five). Words the ledger lacks weigh in the total, so a question that adds to the
///   rare word several that nobody holds ("do we need a licence to play music in the office?")
///   never reaches the share, or
/// - its title or topic keys spell two of the question's words together ("file store" against
///   `FileStore`), and the decisions that spell that pair are as few as a subject word's holders
///   (`SUBJECT_RARITY`): two words that join into a word of the ledger are not chance, so the
///   share of the other words does not matter ("how does the file store throw away expired
///   entries?").
///
/// At most `BELOW_BAR_LIMIT` of them, those holding the most weight, are added; they all lack more
/// terms than any decision at the bar and each is labelled with the terms it lacks. Where they
/// stand among the others is the search's own order, by where the question's words sit. One more
/// is added when it is the question's subject (`subject_below_bar`) or, failing that, the one its
/// title or topic keys hold two words of (`titled_below_bar`) or, failing that, the one that owns
/// the question's one rare word (`lone_below_bar`). A decision that owns a rare word several others
/// hold is listed beside the answer, never named alone, by the first of `lone_owners_below_bar`,
/// `dominant_owner_below_bar` and `title_owner_below_bar` that finds one. The words a decision shares with the
/// question are counted once per stem: it says "job" or "jobs" and holds the question's "job" and
/// its "jobs" alike, which is one word, not two (`TermWeights::once`). An admitted decision that a
/// left-out one outweighs is flagged (`outweighed_below_bar`). No word list, nothing learned: the
/// counts are the ledger's own.
/// `scored` and `holdings` are in the same order; the terms are the question's, `term_count` of
/// them, and the holdings name them by position.
fn admit_below_bar(
    scored: Vec<ScoredDecisionSearchResult>,
    holdings: Vec<Holding>,
    weights: &TermWeights,
    term_count: usize,
) -> Vec<ScoredDecisionSearchResult> {
    if !holdings.iter().any(|holding| holding.below_bar) {
        return scored;
    }
    let total: f64 = (0..term_count)
        .map(|term| match weights.weight_of(term) {
            held if held > 0.0 => held,
            _ => weights.weight_for(0),
        })
        .sum();

    let mut below: Vec<(f64, &str, usize)> = Vec::new();
    for (index, (decision, holding)) in scored.iter().zip(&holdings).enumerate() {
        if !holding.below_bar {
            continue;
        }
        let held = weights.once(&holding.held);
        let headlined = weights.once(&holding.headline);
        let shared: f64 = held.iter().map(|term| weights.weight_of(*term)).sum();
        let rare_pair = held.len() >= 2
            && !headlined.is_empty()
            && held.iter().all(|term| {
                weights
                    .holders
                    .get(term)
                    .is_some_and(|n| *n <= RARE_PAIR_HOLDERS)
            });
        let pair = (held.len() >= SHARED_WORDS_MIN
            || headlined.len() >= SHARED_HEADLINE_WORDS_MIN
            || rare_pair)
            && shared >= SHARED_WORDS_RARITY * weights.ledger().ln()
            && shared >= SHARED_WORDS_SHARE * total;
        let spelled = holding.spelled.iter().any(|term| {
            let spellers = weights.spellers.get(term).copied().unwrap_or(0);
            weights.weight_for(spellers) >= SUBJECT_RARITY * weights.ledger().ln()
        });
        let named = weights.searched <= SMALL_LEDGER
            && holding
                .headline
                .iter()
                .any(|term| weights.holders.get(term) == Some(&1));
        let headline: f64 = headlined.iter().map(|term| weights.weight_of(*term)).sum();
        let subject =
            headline >= SUBJECT_RARITY * weights.ledger().ln() && headline >= SUBJECT_SHARE * total;
        if pair || named || subject || spelled {
            below.push((shared, decision.id.as_str(), index));
        }
    }
    let below = order_below_bar(below, &scored, &holdings, weights);
    let mut admitted: BTreeSet<usize> = below
        .into_iter()
        .take(BELOW_BAR_LIMIT)
        .map(|(_, _, index)| index)
        .collect();
    if let Some(extra) =
        subject_below_bar(&scored, &holdings, &admitted, weights, term_count, total)
            .or_else(|| titled_below_bar(&scored, &holdings, &admitted, weights))
            .or_else(|| lone_below_bar(&holdings, &admitted, weights))
    {
        admitted.insert(extra);
    }
    let ceiling = answer_about(&holdings, &admitted, weights);
    let peers = peers_below_bar(&scored, &holdings, &admitted, weights);
    admitted.extend(peers.iter().copied());
    let mut outweighed = outweighed_below_bar(&holdings, &admitted, weights);
    let mut listed: BTreeSet<usize> = BTreeSet::new();
    let owners = lone_owners_below_bar(&scored, &holdings, &admitted, weights, term_count, total);
    if owners.is_empty() {
        if let Some((owner, outweighed_leaders)) =
            dominant_owner_below_bar(&scored, &holdings, &admitted, weights, term_count, total)
                .or_else(|| title_owner_below_bar(&holdings, &admitted, weights, term_count, total))
        {
            listed.insert(owner);
            outweighed.extend(outweighed_leaders);
        }
    } else {
        listed.extend(owners);
    }
    let listed_ceiling = answer_about(&holdings, &admitted, weights);
    admitted.extend(listed.iter().copied());
    scored
        .into_iter()
        .zip(holdings)
        .enumerate()
        .filter_map(|(index, (mut decision, holding))| {
            decision.peer = peers.contains(&index) || listed.contains(&index);
            if let Some(ceiling) = ceiling.filter(|ceiling| {
                peers.contains(&index)
                    && ceiling.weighs_at_least(SAYS_AS_MUCH_SHARE, &decision.about)
            }) {
                decision.about = decision.about.capped_at(ceiling);
            }
            if let Some(ceiling) = listed_ceiling.filter(|_| listed.contains(&index)) {
                decision.about = decision.about.capped_at(ceiling);
            }
            decision.outweighed = outweighed.contains(&index);
            (!holding.below_bar || admitted.contains(&index)).then_some(decision)
        })
        .collect()
}

/// The decision below the bar that is the question's subject, if it has one: a person who asks
/// "which shelf do we want people to put us on?" names one thing and adds everyday words that
/// several decisions hold, so the decisions that hold "people" and "put" outrank the one titled for
/// the shelf, and the half-of-the-terms bar and the share of `SUBJECT_SHARE` both lose it.
///
/// The subject word is the rarest word that any decision holds. A decision below the bar, not
/// already admitted, is the subject when
///
/// - its title or topic keys hold that word, and the word weighs at least `SUBJECT_RARITY` times
///   `ln(searched + 1)`, and
/// - no decision of the answer holds it in its title or topic keys (one that does is the subject
///   already), and
/// - the decision says at least as many of the question's words in its title or topic keys as any
///   decision of the answer does: two words that sit in a title are not chance, so a word that few
///   decisions hold does not outweigh them ("does the command-line binary get a new name along
///   with the new company?" is answered by the decision titled for the new name, not by the one
///   that says "along", which seven decisions hold and one of them in a title), and
/// - the decision is more about the question (`About`) than every decision of the answer: it says
///   the word in its title or topic keys, whereas they hold the question's other words in a
///   rationale or say more common ones, and
/// - the words some decision holds carry at least `SUBJECT_WORD_KNOWN_SHARE` of the weight of all
///   the question's words: a question mostly about what the ledger lacks ("did we decide to move
///   the UI from React to Svelte?") has no subject here, and nothing is added to what it answers.
///
/// A question whose answer is empty has no decision to outrank, so nothing is added either. At most
/// one decision is the subject; of two, the one more about the question.
fn subject_below_bar(
    scored: &[ScoredDecisionSearchResult],
    holdings: &[Holding],
    admitted: &BTreeSet<usize>,
    weights: &TermWeights,
    term_count: usize,
    total: f64,
) -> Option<usize> {
    let known: f64 = (0..term_count).map(|term| weights.weight_of(term)).sum();
    if known < SUBJECT_WORD_KNOWN_SHARE * total {
        return None;
    }
    let subject_word = (0..term_count)
        .filter(|term| weights.weight_of(*term) > 0.0)
        .max_by(|left, right| {
            weights
                .weight_of(*left)
                .total_cmp(&weights.weight_of(*right))
        })?;
    if weights.weight_of(subject_word) < SUBJECT_RARITY * weights.ledger().ln() {
        return None;
    }
    if holdings.iter().enumerate().any(|(index, holding)| {
        answered(holding, index, admitted) && holding.headline.contains(&subject_word)
    }) {
        return None;
    }
    let titled = holdings
        .iter()
        .enumerate()
        .filter(|(index, holding)| answered(holding, *index, admitted))
        .map(|(_, holding)| holding.headline.len())
        .max()?;
    let outranked = answer_about(holdings, admitted, weights)?;
    outranking_below_bar(scored, holdings, admitted, weights, outranked, |holding| {
        holding.headline.contains(&subject_word) && holding.headline.len() >= titled
    })
}

/// The decision below the bar that the question's words sit in the title of, if there is one: a
/// person who asks "for jobs that run longer than anything we have seen finish, how does beadline
/// draw the time they still need?" adds to "beadline" and "draw", which the decision's title holds,
/// everyday words that other decisions hold, and the share test of the pair (`SHARED_WORDS_SHARE`)
/// loses it by a hair because two words nobody holds weigh in the total.
///
/// A decision below the bar, not already admitted, is it when
///
/// - its title or topic keys hold `SHARED_HEADLINE_WORDS_MIN` or more of the question's words, two
///   of them side by side in the question (`side_by_side`: "beadline draw"), or the decision holds
///   `FAR_PAIR_WORDS_MIN` of the words in all: two words that sit in a title and that the question
///   says far apart, with fewer other words to support them, are chance ("lint" and "sizes" of "is
///   there a lint that checks nobody makes up their own colours and sizes?" in the title "Box the
///   large Payload enum variant to fix clippy size lint"), and
/// - the words it holds weigh, together, at least `SHARED_WORDS_RARITY` times `ln(searched + 1)`,
///   as the pair asks, and
/// - it is more about the question (`About`) than every decision of the answer, which is what the
///   share test is for: two rare words that sit in a title are not chance when nothing the
///   question's other words found says as much of it.
///
/// A question whose answer is empty has no decision to outrank, so nothing is added. At most one
/// decision is added; of two, the one more about the question.
fn titled_below_bar(
    scored: &[ScoredDecisionSearchResult],
    holdings: &[Holding],
    admitted: &BTreeSet<usize>,
    weights: &TermWeights,
) -> Option<usize> {
    let outranked = answer_about(holdings, admitted, weights)?;
    outranking_below_bar(scored, holdings, admitted, weights, outranked, |holding| {
        let held = weights.once(&holding.held);
        let shared: f64 = held.iter().map(|term| weights.weight_of(*term)).sum();
        weights.once(&holding.headline).len() >= SHARED_HEADLINE_WORDS_MIN
            && (side_by_side(&holding.headline) || held.len() >= FAR_PAIR_WORDS_MIN)
            && shared >= SHARED_WORDS_RARITY * weights.ledger().ln()
    })
}

/// The decision below the bar that owns the question's one rare word, if there is one: a person who
/// asks "is anything paywalled before people try it?" names one thing and adds everyday words, and
/// the ledger holds "paywalled" in one decision's title and not in any other. That decision holds
/// one more of the words ("before", in its rationale), which is not chance, but the other rules
/// lose it: the pair test (`SHARED_WORDS_RARITY`) by a hair, because "before" weighs little, the
/// share of the words some decision holds (`SUBJECT_WORD_KNOWN_SHARE`) on a ledger that holds
/// neither "anything" nor "try", and the subject rule wants a decision of the answer to outrank.
///
/// A decision below the bar, not already admitted, is it when
///
/// - its title or topic keys hold a word of the question that no other decision holds, and it is
///   the only decision whose title or topic keys hold such a word: where two decisions each own a
///   word, the question's rare words are scattered and none is its subject ("which cdn provider
///   serves static assets for the marketing site?" meets "static" in one title and "marketing" in
///   another), and
/// - it holds `LONE_SUBJECT_WORDS_MIN` of the question's words, each stem once, wherever: a rare
///   word on its own is what a stray everyday question meets (the standing controls prove it), and
/// - it is more about the question (`About`) than every decision of the answer, when the answer
///   has one.
///
/// At most one decision is added, since only one decision holds the word.
fn lone_below_bar(
    holdings: &[Holding],
    admitted: &BTreeSet<usize>,
    weights: &TermWeights,
) -> Option<usize> {
    let owns = |holding: &Holding| {
        holding
            .headline
            .iter()
            .any(|term| weights.holders.get(term) == Some(&1))
    };
    let mut owners = holdings
        .iter()
        .enumerate()
        .filter(|(_, holding)| owns(holding))
        .map(|(index, _)| index);
    let (Some(owner), None) = (owners.next(), owners.next()) else {
        return None;
    };
    let holding = holdings.get(owner)?;
    if !holding.below_bar
        || admitted.contains(&owner)
        || weights.once(&holding.held).len() < LONE_SUBJECT_WORDS_MIN
    {
        return None;
    }
    let outranks = answer_about(holdings, admitted, weights)
        .is_none_or(|answer| holding.about(weights).cmp_about(&answer).is_gt());
    outranks.then_some(owner)
}

/// The decisions that pass the first test below the bar, in the order the first `BELOW_BAR_LIMIT` of
/// them are added: the ones holding the most weight first, then the lower id. Where several hold the
/// same weight and are equally about the question (`About`), the one filed under the word comes first
/// (`SearchMatchInfo::headline_hits`: its title and its topic keys both say it), since the id says
/// nothing of which was meant: a decision and the drafts that restate its title tie exactly ("what
/// goes in the timeline for a single decision?" meets the decision filed under "timeline" and three
/// drafts that say it in their titles, and the three drafts' ids come first).
fn order_below_bar<'a>(
    mut below: Vec<(f64, &'a str, usize)>,
    scored: &[ScoredDecisionSearchResult],
    holdings: &[Holding],
    weights: &TermWeights,
) -> Vec<(f64, &'a str, usize)> {
    below.sort_by(|left, right| right.0.total_cmp(&left.0).then_with(|| left.1.cmp(right.1)));
    let mut placed = vec![false; below.len()];
    let mut ordered = Vec::with_capacity(below.len());
    for first in 0..below.len() {
        if placed.get(first).copied().unwrap_or(true) {
            continue;
        }
        let Some(&(shared, _, lead)) = below.get(first) else {
            continue;
        };
        let Some(lead_about) = holdings.get(lead).map(|holding| holding.about(weights)) else {
            continue;
        };
        let mut tied: Vec<(usize, &str, usize)> = Vec::new();
        for (position, &(weight, id, index)) in below.iter().enumerate().skip(first) {
            let equally_about = holdings
                .get(index)
                .is_some_and(|holding| holding.about(weights).cmp_about(&lead_about).is_eq());
            if !placed.get(position).copied().unwrap_or(true)
                && weight.total_cmp(&shared).is_eq()
                && equally_about
            {
                if let Some(slot) = placed.get_mut(position) {
                    *slot = true;
                }
                tied.push((scored.get(index).map_or(0, |d| d.headline_hits), id, index));
            }
        }
        tied.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(right.1)));
        ordered.extend(tied.into_iter().map(|(_, id, index)| (shared, id, index)));
    }
    ordered
}

/// The decisions below the bar that own the question's one rare word when nothing answers it:
/// a person who asks "at what number of pixels do we stop putting the names in?" names one thing and
/// adds everyday words, and the ledger holds "pixels" in the titles of three decisions and in
/// nothing else, so none of them is the only one (`lone_below_bar`) and each of them holds the
/// everyday "names" too. The answer is empty, or a decision that holds "number", "stop" and "names"
/// in a rationale (a review of a hundred and twelve decisions, say), so nothing in it is about the
/// question more than they are.
///
/// Up to `BELOW_BAR_LIMIT` decisions below the bar, the most about the question first, are added when
///
/// - the answer is empty or none of its decisions says a word of the question in its title or topic
///   keys (`About` of nothing), and
/// - the words some decision holds carry at least `LONE_OWNERS_KNOWN_SHARE` of the weight of all the
///   question's words, and
/// - exactly one word of the question is held by `LONE_OWNERS_HOLDERS_MAX` decisions or fewer and said
///   in the title or topic keys of one of them, and
/// - every decision that holds that word says it in its title or topic keys: nobody mentions it in
///   passing, and
/// - the decision holds `LONE_SUBJECT_WORDS_MIN` of the question's words, each stem once.
///
/// The word held by one decision only is `lone_below_bar`'s, which also answers a question with an
/// answer that says more in its title; this is its form for a word that a few decisions each title.
fn lone_owners_below_bar(
    scored: &[ScoredDecisionSearchResult],
    holdings: &[Holding],
    admitted: &BTreeSet<usize>,
    weights: &TermWeights,
    term_count: usize,
    total: f64,
) -> Vec<usize> {
    if answer_about(holdings, admitted, weights).is_some_and(|answer| answer.weight > 0.0) {
        return Vec::new();
    }
    let known: f64 = (0..term_count).map(|term| weights.weight_of(term)).sum();
    if known < LONE_OWNERS_KNOWN_SHARE * total {
        return Vec::new();
    }
    let mut rare = (0..term_count).filter(|term| {
        weights
            .holders
            .get(term)
            .is_some_and(|n| *n <= LONE_OWNERS_HOLDERS_MAX)
            && holdings
                .iter()
                .any(|holding| holding.headline.contains(term))
    });
    let (Some(word), None) = (rare.next(), rare.next()) else {
        return Vec::new();
    };
    if holdings
        .iter()
        .any(|holding| holding.held.contains(&word) != holding.headline.contains(&word))
    {
        return Vec::new();
    }
    let mut owners: Vec<usize> = holdings
        .iter()
        .enumerate()
        .filter(|(index, holding)| {
            holding.headline.contains(&word)
                && holding.below_bar
                && !admitted.contains(index)
                && weights.once(&holding.held).len() >= LONE_SUBJECT_WORDS_MIN
        })
        .map(|(index, _)| index)
        .collect();
    owners.sort_by(|left, right| {
        let about = |index: &usize| {
            holdings
                .get(*index)
                .map_or_else(About::default, |holding| holding.about(weights))
        };
        let hits = |index: &usize| scored.get(*index).map_or(0, |d| d.headline_hits);
        let id = |index: &usize| scored.get(*index).map(|d| d.id.as_str());
        about(right)
            .cmp_about(&about(left))
            .then_with(|| hits(right).cmp(&hits(left)))
            .then_with(|| id(left).cmp(&id(right)))
    });
    owners.truncate(BELOW_BAR_LIMIT);
    owners
}

/// The decision below the bar that is filed under the question's dominant word, listed beside an answer
/// that never says it: a person who asks "which shelf do we point people to when they ask what kind
/// of thing we are?" adds to "shelf", which four decisions of eighteen hundred hold, everyday words
/// that dozens of decisions hold each, so the answer is the decisions that say "people" and "point"
/// in their titles, and the decision filed under the shelf is one word of six. The subject rule
/// (`subject_below_bar`) wants it more about the question than the answer and as titled, and neither
/// holds: the answer's title says two everyday words and weighs more.
///
/// A decision below the bar, not already admitted, is it when
///
/// - the answer is `DOMINANT_ANSWER_MIN` decisions or more, and
/// - the words some decision holds carry at least `SUBJECT_WORD_KNOWN_SHARE` of the weight of all the
///   question's words, and
/// - the rarest word of the question that a decision below the bar says in its title or topic keys
///   weighs at least `DOMINANT_WORD_RARITY` times `ln(searched + 1)`, and
/// - no decision of the answer says that word in its title or topic keys, and
/// - the decision that is the most about the question of those that do is filed under a word of the
///   question (`DOMINANT_FILED_HITS_MIN`).
///
/// It is listed with the answer and never named alone, and it comes no higher than the answer's best.
/// The decisions of the answer that are more about the question than it is and do not hold the word
/// are not named alone either, since the question is as much about the word they lack as about the
/// words they hold. At most one decision is added.
fn dominant_owner_below_bar(
    scored: &[ScoredDecisionSearchResult],
    holdings: &[Holding],
    admitted: &BTreeSet<usize>,
    weights: &TermWeights,
    term_count: usize,
    total: f64,
) -> Option<(usize, Vec<usize>)> {
    let known: f64 = (0..term_count).map(|term| weights.weight_of(term)).sum();
    if known < SUBJECT_WORD_KNOWN_SHARE * total {
        return None;
    }
    answer_about(holdings, admitted, weights)?;
    let answers = holdings
        .iter()
        .enumerate()
        .filter(|(index, holding)| answered(holding, *index, admitted))
        .count();
    if answers < DOMINANT_ANSWER_MIN {
        return None;
    }
    let word = (0..term_count)
        .filter(|term| {
            weights.weight_of(*term) > 0.0
                && holdings.iter().enumerate().any(|(index, holding)| {
                    holding.below_bar
                        && !admitted.contains(&index)
                        && holding.headline.contains(term)
                })
        })
        .max_by(|left, right| {
            weights
                .weight_of(*left)
                .total_cmp(&weights.weight_of(*right))
        })?;
    if weights.weight_of(word) < DOMINANT_WORD_RARITY * weights.ledger().ln() {
        return None;
    }
    if holdings.iter().enumerate().any(|(index, holding)| {
        answered(holding, index, admitted) && holding.headline.contains(&word)
    }) {
        return None;
    }
    let owner = scored
        .iter()
        .zip(holdings)
        .enumerate()
        .filter(|(index, (_, holding))| {
            holding.below_bar && !admitted.contains(index) && holding.headline.contains(&word)
        })
        .max_by(|(_, (left, left_holding)), (_, (right, right_holding))| {
            left_holding
                .about(weights)
                .cmp_about(&right_holding.about(weights))
                .then_with(|| left.headline_hits.cmp(&right.headline_hits))
                .then_with(|| right.id.cmp(&left.id))
        })?;
    let (index, (decision, holding)) = owner;
    if decision.headline_hits < DOMINANT_FILED_HITS_MIN {
        return None;
    }
    let about = holding.about(weights);
    let leaders = holdings
        .iter()
        .enumerate()
        .filter(|(other, holding)| {
            answered(holding, *other, admitted)
                && !holding.held.contains(&word)
                && holding.about(weights).cmp_about(&about).is_gt()
        })
        .map(|(other, _)| other)
        .collect();
    Some((index, leaders))
}

/// The decision below the bar that is the only one to say the question's rare word in its title,
/// listed beside an answer in which another decision holds the word. A person who asks "is anything
/// paywalled before people try it?" names one thing. The ledger holds "paywalled" in a handful of
/// decisions, one of them titled for it, and the others say it in passing: a draft that restates the
/// question holds every word of it and is named alone, though nothing in the question sets it above
/// the decision titled for the word. `lone_below_bar` adds that owner only while it is the word's one
/// holder, and `dominant_owner_below_bar` only for a long answer, so a second holder turns both off.
///
/// A decision below the bar, not already admitted, is it when
///
/// - the words some decision holds carry at least `SUBJECT_WORD_KNOWN_SHARE` of the weight of all the
///   question's words, and
/// - the answer is not empty and a decision of it holds the word, and
/// - the word weighs at least `TITLE_OWNER_RARITY` times `ln(searched + 1)` and no other decision
///   says it in its title; where two decisions each own such a word, the question's rare words are
///   scattered and none is its subject, and
/// - the decision holds `LONE_SUBJECT_WORDS_MIN` of the question's words, each stem once.
///
/// It is listed with the answer and never named alone, and it comes no higher than the answer's
/// best. The decisions of the answer that hold the word and are more about the question than it is
/// are not named alone either: nothing in the question tells them from the decision titled for the
/// word. At most one decision is added.
fn title_owner_below_bar(
    holdings: &[Holding],
    admitted: &BTreeSet<usize>,
    weights: &TermWeights,
    term_count: usize,
    total: f64,
) -> Option<(usize, Vec<usize>)> {
    let known: f64 = (0..term_count).map(|term| weights.weight_of(term)).sum();
    if known < SUBJECT_WORD_KNOWN_SHARE * total {
        return None;
    }
    answer_about(holdings, admitted, weights)?;
    let terms: BTreeSet<usize> = (0..term_count).collect();
    let mut owner: Option<usize> = None;
    let mut words: BTreeSet<usize> = BTreeSet::new();
    for term in weights.once(&terms) {
        if weights.weight_of(term) < TITLE_OWNER_RARITY * weights.ledger().ln() {
            continue;
        }
        let mut titles = holdings
            .iter()
            .enumerate()
            .filter(|(_, holding)| weights.once(&holding.title).contains(&term))
            .map(|(index, _)| index);
        let (Some(only), None) = (titles.next(), titles.next()) else {
            continue;
        };
        if owner.is_some_and(|other| other != only) {
            return None;
        }
        owner = Some(only);
        words.insert(term);
    }
    let owner = owner?;
    let holding = holdings.get(owner)?;
    if !holding.below_bar
        || admitted.contains(&owner)
        || weights.once(&holding.held).len() < LONE_SUBJECT_WORDS_MIN
    {
        return None;
    }
    let holders: Vec<usize> = holdings
        .iter()
        .enumerate()
        .filter(|(index, other)| {
            answered(other, *index, admitted)
                && weights
                    .once(&other.held)
                    .iter()
                    .any(|term| words.contains(term))
        })
        .map(|(index, _)| index)
        .collect();
    if holders.is_empty() {
        return None;
    }
    let about = holding.about(weights);
    let leaders = holders
        .into_iter()
        .filter(|index| {
            holdings
                .get(*index)
                .is_some_and(|other| other.about(weights).cmp_about(&about).is_gt())
        })
        .collect();
    Some((owner, leaders))
}

/// One decision is about the question as much as another when it weighs (`About`) at least this
/// share of what the other does, see `peers_below_bar` and `close_candidate_leads`.
pub(crate) const SAYS_AS_MUCH_SHARE: f64 = 0.9;

/// The decisions below the bar that are peers of the answer's best-titled decision: their title or
/// topic keys say as much of the question as its does, `SHARED_HEADLINE_WORDS_MIN` words at least
/// (a word the question says in two forms counted once), two of them the words the question says
/// side by side, and they are about the question as much
/// (`SAYS_AS_MUCH_SHARE`). "does the graph draw anyone in it at all, or do we tell you who made the
/// call?" is answered by the decision whose title says "graph" and "made", whereas the two
/// decisions whose titles say "graph draw" are the ones asked about: one rare word does not tell
/// them apart when "graph" is held by a decision in three, so the weight test of `titled_below_bar`
/// (that many decisions do not say two such words by chance) cannot hold for them. A pair the
/// question says as a phrase ("graph draw", "link ... graph") and a title says is not chance either
/// way, which two words the question says far apart ("people" and "graph" of "what do people get told
/// when a link to the graph points at nothing?") are. They join the answer, most about the question
/// first, so a search that lists them does not leave them for a decision that holds more of the
/// words in a rationale. A peer does not outrank the decision the answer was found by when that
/// decision is about the question as much as the peer is (`SAYS_AS_MUCH_SHARE`: `admit_below_bar`
/// caps the peer's weight there, and the search's tie-break, the fewer words missing, puts the
/// decision first): "does the command-line binary get a new name along with the new company?" is
/// answered by the decision titled for the new name, which holds four of the words, and not by the
/// two drafts that say "get" and "new" in theirs, which hold two and weigh a hair more because
/// "get" is held by fewer decisions than "name". A peer that weighs clearly more keeps its place.
/// An answer none of whose decisions says a word of the question in its title or topic keys has
/// no best-titled decision to be a peer of (it found the decision by a rare word in a rationale,
/// "how do visitors know the animated graph is showing sample data?"), and an empty one has
/// nothing, so nothing is added. At most `BELOW_BAR_LIMIT` are added; of two, the one more about
/// the question, then the one that names a word it holds in both its title and its topic keys,
/// then the lower id.
fn peers_below_bar(
    scored: &[ScoredDecisionSearchResult],
    holdings: &[Holding],
    admitted: &BTreeSet<usize>,
    weights: &TermWeights,
) -> Vec<usize> {
    let Some(best) = answer_about(holdings, admitted, weights) else {
        return Vec::new();
    };
    let titled = holdings
        .iter()
        .enumerate()
        .filter(|(index, holding)| answered(holding, *index, admitted))
        .map(|(_, holding)| weights.once(&holding.headline).len())
        .max()
        .filter(|titled| *titled > 0)
        .map_or(0, |titled| titled.max(SHARED_HEADLINE_WORDS_MIN));
    if titled == 0 {
        return Vec::new();
    }
    let mut peers: Vec<(About, usize, &str, usize)> = scored
        .iter()
        .zip(holdings)
        .enumerate()
        .filter(|(index, (_, holding))| {
            holding.below_bar
                && !admitted.contains(index)
                && weights.once(&holding.headline).len() >= titled
                && side_by_side(&holding.headline)
                && holding
                    .about(weights)
                    .weighs_at_least(SAYS_AS_MUCH_SHARE, &best)
        })
        .map(|(index, (decision, holding))| {
            (
                holding.about(weights),
                decision.headline_hits,
                decision.id.as_str(),
                index,
            )
        })
        .collect();
    peers.sort_by(|left, right| {
        right
            .0
            .cmp_about(&left.0)
            .then_with(|| right.1.cmp(&left.1))
            .then_with(|| left.2.cmp(right.2))
    });
    peers
        .into_iter()
        .take(BELOW_BAR_LIMIT)
        .map(|(.., index)| index)
        .collect()
}

/// The decisions admitted below the bar while a decision left out below it is more about the
/// question (`About`) and holds `OUTWEIGHING_WORDS_MIN` of its words, each stem once, one of them in
/// its title or topic keys. The half-of-the-terms bar (`admit_below_bar`) and its pair rule are
/// counts, and a decision that holds three of the words in a rationale, two of them in a title,
/// passes them by a hair while a decision that says the question's rarer words in its title falls
/// short of them by another: "why did the drawings in the animation stop using a fixed-size canvas
/// that got shrunk on phones?" admits the decision titled for the frame loop that stops with the
/// canvas, whereas the decision that says "canvas" and "phones" in its title and is more about the
/// question is left out. Which of the two was meant is not told by the counts, so the admitted
/// one is listed and never named alone (`close_candidate_leads`). A decision at the bar is not
/// outweighed: it holds half of the question, as the one left out does not.
fn outweighed_below_bar(
    holdings: &[Holding],
    admitted: &BTreeSet<usize>,
    weights: &TermWeights,
) -> BTreeSet<usize> {
    let rivals: Vec<About> = holdings
        .iter()
        .enumerate()
        .filter(|(index, holding)| {
            holding.below_bar
                && !admitted.contains(index)
                && !holding.headline.is_empty()
                && weights.once(&holding.held).len() >= OUTWEIGHING_WORDS_MIN
        })
        .map(|(_, holding)| holding.about(weights))
        .collect();
    admitted
        .iter()
        .copied()
        .filter(|index| {
            holdings.get(*index).is_some_and(|holding| {
                let about = holding.about(weights);
                holding.below_bar && rivals.iter().any(|rival| rival.cmp_about(&about).is_gt())
            })
        })
        .collect()
}

/// Whether two of the question's terms in `terms` are neighbours in the question: it says them as
/// a phrase, whatever small words it puts between them.
fn side_by_side(terms: &BTreeSet<usize>) -> bool {
    terms
        .iter()
        .zip(terms.iter().skip(1))
        .any(|(left, right)| right - left == 1)
}

/// Whether the decision at `index` is in the answer: at the bar, or admitted below it.
fn answered(holding: &Holding, index: usize, admitted: &BTreeSet<usize>) -> bool {
    !holding.below_bar || admitted.contains(&index)
}

/// How much the answer is about the question: what its most-about decision is (`About`); nothing
/// when the answer is empty.
fn answer_about(
    holdings: &[Holding],
    admitted: &BTreeSet<usize>,
    weights: &TermWeights,
) -> Option<About> {
    holdings
        .iter()
        .enumerate()
        .filter(|(index, holding)| answered(holding, *index, admitted))
        .map(|(_, holding)| holding.about(weights))
        .max_by(About::cmp_about)
}

/// Of the decisions below the bar, not admitted, that `fits`, the one more about the question than
/// `outranked`; of two, the more about it, then the one that names a word it holds in both its
/// title and its topic keys (the search's own tie-break, `sort_scored`: a proposal that says
/// "shelf" in its title is not the decision filed under the public shelf), then the lower id.
fn outranking_below_bar(
    scored: &[ScoredDecisionSearchResult],
    holdings: &[Holding],
    admitted: &BTreeSet<usize>,
    weights: &TermWeights,
    outranked: About,
    fits: impl Fn(&Holding) -> bool,
) -> Option<usize> {
    scored
        .iter()
        .zip(holdings)
        .enumerate()
        .filter(|(index, (_, holding))| {
            holding.below_bar && !admitted.contains(index) && fits(holding)
        })
        .map(|(index, (decision, holding))| {
            (
                holding.about(weights),
                decision.headline_hits,
                decision.id.as_str(),
                index,
            )
        })
        .filter(|(about, ..)| about.cmp_about(&outranked).is_gt())
        .max_by(|left, right| {
            left.0
                .cmp_about(&right.0)
                .then_with(|| left.1.cmp(&right.1))
                .then_with(|| right.2.cmp(left.2))
        })
        .map(|(.., index)| index)
}

/// One decision candidate for the resolve-by-description primitive (`resolve.rs`): the same
/// deterministic tier ranking `collect_graph_search_results` already computes, plus the
/// `event_origin` recency tiebreak. No new ranking logic — see docs/AGENT_FLUENT_QUERYING.md §1.1.
pub(crate) struct ResolverCandidateRow {
    pub(crate) decision_id: String,
    pub(crate) title: String,
    pub(crate) rank: u8,
    pub(crate) event_origin: i64,
    /// The decision is accepted and the asker matches words (`ScoredDecisionSearchResult::
    /// accepted`): among candidates that tie on everything that says how fully the question
    /// matches, it comes before a proposed draft, which recency alone would put first.
    pub(crate) accepted: bool,
    pub(crate) matched_fields: Vec<String>,
    /// Description terms this decision does not contain; empty for a full match.
    pub(crate) missing_terms: Vec<String>,
    /// The description is negated and this decision's title says every term of it outright: it
    /// is a close candidate even when it contains every term, and is never resolved to.
    pub(crate) polarity_mismatch: bool,
    /// For a close candidate, how many description terms its title or topic keys contain; 0 for a
    /// full match.
    pub(crate) headline_terms: usize,
    /// How many of the terms it matched only through a stand-in word (`SearchMatchInfo::
    /// stand_in_terms`); 0 for an asker that writes, which never reads stand-ins.
    pub(crate) stand_in_terms: usize,
    /// How many of the question's words the decision's own title and topic keys hold as a word or
    /// a form of one. Not `About`'s count: the question a decision records can be as long as a
    /// paragraph and holds a great many words, so it orders a decision but is never what names it.
    pub(crate) headline_words: usize,
    /// How many of the question's words the decision's own title and topic keys hold in any form:
    /// as a word, a form of one, a stand-in or a part of a longer word. Unlike `headline_terms`
    /// it is counted for a full match too.
    pub(crate) headline_matched: usize,
    /// Which of the question's terms its own title and topic keys hold as a word or a form of one
    /// (bit `n` for term `n`); `headline_words` is its size.
    pub(crate) headline_mask: u64,
    /// Which of the question's terms its title and topic keys hold in any form: as a word, a form
    /// of one, a stand-in or a part of a longer word (`headline_matched` is its size).
    pub(crate) headline_any_mask: u64,
    /// How many times its title and topic keys hold a term as a word (a term both hold counts
    /// twice). Among candidates that tie on everything else that says where the words sit, the
    /// decision filed under the word its title says is the one about it, before recency decides.
    pub(crate) headline_hits: usize,
    /// How much of the question the decision's title, topic keys and recorded question carry
    /// (`About`).
    pub(crate) about: About,
    /// A close match that `out_heading` puts ahead of the full ones; set by the resolver once
    /// the records of one decision are folded together.
    pub(crate) promoted: bool,
    /// Admitted below the bar as a peer of the answer's best-titled decision, see
    /// `peers_below_bar`; it is listed with the answer and is never the one named alone.
    pub(crate) peer: bool,
    /// Admitted below the bar while a decision left out says more of the question, see
    /// `outweighed_below_bar`; it is listed with the answer and is never the one named alone.
    pub(crate) outweighed: bool,
}

impl ResolverCandidateRow {
    /// Whether this is a close candidate rather than a full match: it lacks some of the terms, or
    /// it is the opposite of what a negated description asked for.
    pub(crate) fn is_close(&self) -> bool {
        !self.missing_terms.is_empty() || self.polarity_mismatch
    }
}

/// Backend-agnostic candidate rows for resolve-by-description: reuses
/// `collect_graph_search_results`'s tier system unmodified (§1.1 of the design), narrowed by an
/// optional topic hint. `description` is required — an empty resolver query is the caller's bug.
/// `related` reads the description as `recall` does (word forms and stand-in words); a verb that
/// writes asks with `false` and matches a word and its inflections only.
pub(crate) fn collect_resolver_candidates(
    graph: &impl GraphView,
    description: &str,
    topic_keys: &[String],
    close: CloseMatch,
    related: bool,
) -> Result<Vec<ResolverCandidateRow>> {
    let question = resolver_question(description);
    let scored = collect_graph_search_results(
        graph,
        Some(description),
        &SearchTerms::resolver(&question.terms, close, question.negated, related),
        topic_keys,
        &[],
        &[],
        &[],
    )?;
    Ok(scored
        .into_iter()
        .map(|scored| ResolverCandidateRow {
            decision_id: scored.id,
            title: scored.result.decision.title,
            rank: scored.rank,
            event_origin: scored.event_origin,
            accepted: scored.accepted,
            matched_fields: scored.result.matched_fields,
            missing_terms: scored.result.missing_terms,
            polarity_mismatch: scored.polarity_mismatch,
            headline_terms: scored.headline_terms,
            stand_in_terms: scored.stand_ins,
            headline_words: scored.headline_words,
            headline_matched: scored.headline_matched,
            headline_mask: scored.headline_mask,
            headline_any_mask: scored.headline_any_mask,
            headline_hits: scored.headline_hit_count,
            about: scored.about,
            promoted: false,
            peer: scored.peer,
            outweighed: scored.outweighed,
        })
        .collect())
}

#[derive(Clone, Debug)]
struct SearchField {
    field: String,
    value: String,
    rank: u8,
    node: Option<(NodeKind, String)>,
}

impl SearchField {
    fn decision(field: &str, value: &str, rank: u8) -> Self {
        Self {
            field: field.to_owned(),
            value: value.to_owned(),
            rank,
            node: None,
        }
    }

    fn node(field: &str, value: &str, rank: u8, kind: NodeKind, id: &str) -> Self {
        Self {
            field: field.to_owned(),
            value: value.to_owned(),
            rank,
            node: Some((kind, id.to_owned())),
        }
    }
}

#[derive(Clone, Debug, Default)]
struct SearchMatchInfo {
    rank: u8,
    /// The decision lacks too many terms to pass `CloseMatch::accepts`, but holds some term as a
    /// word: `collect_graph_search_results` keeps it only if the words it holds are rare among the
    /// decisions searched (`admit_below_bar`).
    below_bar: bool,
    /// The terms (by position in the question's terms) some field holds as a whole word or a form
    /// of one, never as part of a longer word; set for every asker that matches words
    /// (`CloseMatch::Majority` and `CloseMatch::Half`).
    held_words: BTreeSet<usize>,
    /// The subset of `held_words` the decision's title or topic keys hold.
    headline_words: BTreeSet<usize>,
    /// The subset of `headline_words` the title itself holds.
    title_words: BTreeSet<usize>,
    /// What `About` weighs: `headline_words`, and the terms the decision's recorded question
    /// holds as words. The question a decision answers is as much what it is about as its title.
    about_words: BTreeSet<usize>,
    /// The terms (by position) that the title or a topic key holds spelled together with the term
    /// after it (`SearchTerms::spelled`): `FileStore` for "file store". Both words are then in
    /// `held_words`, `headline_words` and `about_words`.
    spelled_words: BTreeSet<usize>,
    /// The terms the title or a topic key holds only through a stand-in word, never as the term
    /// or a form of it.
    stand_in_headline: BTreeSet<usize>,
    /// The subset of `stand_in_headline` the title itself holds.
    stand_in_title: BTreeSet<usize>,
    /// The terms some field holds through a stand-in word.
    stand_in_reached: BTreeSet<usize>,
    /// The query is the decision's id or title, or the question it records, as written.
    named: bool,
    /// How many of the terms the title or topic keys contain in any form (a word, a form of one, a
    /// stand-in or a part of a longer word), for a full match as well as a close one.
    headline_matched: usize,
    /// Which terms (by position) those are: the set `headline_matched` counts.
    headline_any: BTreeSet<usize>,
    /// How many times a field of the title or topic keys holds a term as a word (a term the title
    /// and a topic key both hold counts twice). Among decisions below the bar that hold a word
    /// each, the one whose title and topic keys both name it is the one about it.
    headline_hits: usize,
    /// Query terms no field matched; non-empty only for a close match.
    missing_terms: Vec<String>,
    /// The question is negated and the decision's title says every term of it outright (see
    /// `SearchTerms::negated`).
    polarity_mismatch: bool,
    /// For a close match, how many of the terms the decision's title or topic keys contain; 0 for
    /// a full match, which `rank` already orders. A decision about a thing says it in its
    /// headline, so among close matches lacking the same number of terms, the one whose headline
    /// carries more of the words it did match is the likelier answer than one that only holds
    /// them somewhere in a long rationale or in evidence.
    headline_terms: usize,
    /// How many of the matched terms the decision has only as a stand-in word (`WORD_GROUPS`),
    /// never as the term or a form of it. A decision that has the word asked for is a better
    /// answer than one that has a synonym of it, so among decisions lacking the same number of
    /// terms the one that leans on fewer stand-ins comes first.
    stand_in_terms: usize,
    matched_fields: Vec<String>,
    snippets: Vec<SearchSnippet>,
    matched_nodes: Vec<SearchMatchedNode>,
}

fn add_node_search_fields(
    fields: &mut Vec<SearchField>,
    rows: &BTreeMap<String, GraphRow>,
    kind: NodeKind,
    id: &str,
    field_specs: &[(&str, &str)],
) {
    for (field_name, property) in field_specs {
        let value = if *property == "id" {
            Some(id.to_owned())
        } else {
            rows.get(id).and_then(|row| optional_string(row, property))
        };
        if let Some(value) = value {
            fields.push(SearchField::node(field_name, &value, 3, kind, id));
        }
    }
}

/// When a decision that lacks some of a query's terms is still returned, and with how few.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CloseMatch {
    /// Every term must match: `search`, which keeps literal matching.
    Never,
    /// More than half of the terms, and at least two. A resolver's close candidates feed the
    /// fluent follow-up verbs that can write (`disagree`, `supersede`), so one shared word is
    /// never enough.
    Majority,
    /// At least half of the terms. `recall` only reads and labels every item with what it lacks,
    /// so a question naming two things ("sign-in and pricing") still finds a decision about
    /// either, after the ones about both. The fluent verbs that only read (`why`, `verify`)
    /// resolve with the same bar, so they name what `recall` just named.
    Half,
}

impl CloseMatch {
    /// Whether a decision matching `matched` of `total` terms (`matched < total`) is returned. Both
    /// count half words: a term a field holds as the word is two, one only a stand-in reaches is
    /// two or one (`evaluate_search_match`), so the bar is the same in whole words.
    fn accepts(self, matched: usize, total: usize) -> bool {
        match self {
            Self::Never => false,
            Self::Majority => matched >= 4 && matched * 2 > total,
            Self::Half => matched >= 2 && matched * 2 >= total,
        }
    }
}

/// The terms a query must find, and how. Every term must match some field (AND) unless `close`
/// admits a decision that lacks some. `literal` terms match as a substring. Fluent terms
/// (`resolver`, `fluent`) also match a field word with the same stem ("move" finds "moves" and
/// "moved"), and when no decision matches every term, a decision matching most of them is
/// returned with the terms it lacks. Related terms (`fluent`, and the resolver of a verb that only
/// reads) also match the other forms of a word ("superseded" finds "supersession") and the words
/// `WORD_GROUPS` lists as standing in for it ("interface" finds "UI").
struct SearchTerms<'a> {
    terms: &'a [String],
    stemmed: bool,
    /// One per term, in the same order, for a request that reads related words; empty otherwise.
    related: Vec<RelatedWord<'a>>,
    close: CloseMatch,
    /// One per term, in the same order, for an asker that reads: the word the term makes with the
    /// term after it, written together (`spelled_together`); empty otherwise.
    spelled: Vec<String>,
    /// The question was asked negated ("don't adopt Kafka"). The negation is not one of `terms`:
    /// it never makes a decision lack a word. It only marks a decision whose title says every
    /// term outright as the opposite of what was asked (`SearchMatchInfo::polarity_mismatch`).
    negated: bool,
}

impl<'a> SearchTerms<'a> {
    fn literal(terms: &'a [String]) -> Self {
        Self {
            terms,
            stemmed: false,
            related: Vec::new(),
            close: CloseMatch::Never,
            spelled: Vec::new(),
            negated: false,
        }
    }

    fn resolver(terms: &'a [String], close: CloseMatch, negated: bool, related: bool) -> Self {
        Self {
            terms,
            stemmed: true,
            related: Self::related_words(terms, related),
            close,
            spelled: Self::spelled_words(terms, close),
            negated,
        }
    }

    fn fluent(terms: &'a [String], negated: bool) -> Self {
        Self {
            terms,
            stemmed: true,
            related: Self::related_words(terms, true),
            close: CloseMatch::Half,
            spelled: Self::spelled_words(terms, CloseMatch::Half),
            negated,
        }
    }

    /// Only the asker that reads (`CloseMatch::Half`) admits a decision on a pair of words spelled
    /// together.
    fn spelled_words(terms: &[String], close: CloseMatch) -> Vec<String> {
        if close == CloseMatch::Half {
            spelled_together(terms)
        } else {
            Vec::new()
        }
    }

    fn related_words(terms: &'a [String], related: bool) -> Vec<RelatedWord<'a>> {
        if related {
            terms.iter().map(|term| RelatedWord::new(term)).collect()
        } else {
            Vec::new()
        }
    }
}

fn evaluate_search_match(
    query: Option<&str>,
    search_terms: &SearchTerms<'_>,
    fields: &[SearchField],
) -> Option<SearchMatchInfo> {
    let terms = search_terms.terms;
    let Some(query) = query else {
        return Some(SearchMatchInfo {
            rank: 4,
            ..SearchMatchInfo::default()
        });
    };

    let mut matched_terms = BTreeSet::new();
    // The matched terms that some field holds as the word itself or a form of it.
    let mut worded_terms: BTreeSet<&String> = BTreeSet::new();
    let mut headline_terms = BTreeSet::new();
    let mut headline_any = BTreeSet::new();
    // What `admit_below_bar` and `About` need from an asker that matches words: which terms a
    // field holds as a whole word, which of those the title or topic keys hold, and which of those
    // the title holds. Only the asker that reads admits a decision below the bar.
    let track_words = search_terms.close != CloseMatch::Never;
    let reads = search_terms.close == CloseMatch::Half;
    let mut held_words = BTreeSet::new();
    let mut headline_words = BTreeSet::new();
    let mut title_words = BTreeSet::new();
    let mut about_words = BTreeSet::new();
    let mut spelled_words = BTreeSet::new();
    let mut stand_in_headline = BTreeSet::new();
    let mut stand_in_title = BTreeSet::new();
    let mut stand_in_about = BTreeSet::new();
    let mut reached = BTreeSet::new();
    let mut headline_hits = 0_usize;
    let mut matched_fields = BTreeSet::new();
    let mut snippets = Vec::new();
    let mut matched_nodes = BTreeSet::new();
    // The question a decision answers, asked back as recorded, names that decision as an exact
    // title does.
    let quotes_question = exact_question_match(query, fields);
    let named = quotes_question || exact_id_or_title_match(query, fields);
    let mut rank = if named { 0 } else { u8::MAX };

    for field in fields {
        let value_lower = field.value.to_ascii_lowercase();
        let mut field_stems: Option<BTreeSet<&str>> = None;
        let mut field_matched = false;
        let in_headline = matches!(field.field.as_str(), "decision.title" | "decision.topic");
        for (index, term) in terms.iter().enumerate() {
            let found = if value_lower.contains(term) {
                Some(WordMatch::Word)
            } else if let Some(related) = search_terms.related.get(index) {
                related.find_in(&value_lower)
            } else if search_terms.stemmed
                && field_stems
                    .get_or_insert_with(|| word_stems(&value_lower))
                    .contains(stem(term))
            {
                Some(WordMatch::Word)
            } else {
                None
            };
            if let Some(how) = found {
                matched_terms.insert(term.clone());
                if how == WordMatch::Word {
                    worded_terms.insert(term);
                }
                if in_headline {
                    headline_terms.insert(term.clone());
                    headline_any.insert(index);
                }
                if track_words
                    && holds_whole_word(search_terms.related.get(index), &value_lower, term)
                {
                    held_words.insert(index);
                    if in_headline {
                        headline_words.insert(index);
                        headline_hits += 1;
                        if field.field == "decision.title" {
                            title_words.insert(index);
                        }
                    }
                    if in_headline || field.field == "decision.question" {
                        about_words.insert(index);
                    }
                }
                if track_words && how == WordMatch::StandIn {
                    if matches!(field.field.as_str(), "decision.topic" | "decision.question") {
                        stand_in_about.insert(index);
                    }
                    reached.insert(index);
                    if in_headline {
                        stand_in_headline.insert(index);
                        if field.field == "decision.title" {
                            stand_in_title.insert(index);
                        }
                    }
                }
                field_matched = true;
            }
        }

        // "file store" is what a person asks and `FileStore` is what the title says: neither word
        // is a word of the title, the two together are. The title or topic key that spells a pair
        // of the question's words together holds both, as it would had it written them apart.
        if reads && in_headline {
            for (index, spelled) in search_terms.spelled.iter().enumerate() {
                if spelled.is_empty() || !value_lower.contains(spelled.as_str()) {
                    continue;
                }
                let stems = field_stems.get_or_insert_with(|| word_stems(&value_lower));
                if !stems.contains(stem(spelled)) {
                    continue;
                }
                spelled_words.insert(index);
                for term in [index, index + 1] {
                    held_words.insert(term);
                    headline_words.insert(term);
                    headline_hits += 1;
                    about_words.insert(term);
                    if field.field == "decision.title" {
                        title_words.insert(term);
                    }
                }
            }
        }

        if !field_matched {
            continue;
        }

        rank = rank.min(field.rank);
        matched_fields.insert(field.field.clone());
        if snippets.len() < MAX_SNIPPETS_PER_RESULT {
            snippets.push(SearchSnippet {
                field: field.field.clone(),
                value: snippet_value(&field.value),
            });
        }
        if let Some((kind, id)) = &field.node {
            matched_nodes.insert(SearchMatchedNode {
                id: id.clone(),
                kind: *kind,
                field: field.field.clone(),
            });
        }
    }

    let missing_terms: Vec<String> = terms
        .iter()
        .filter(|term| !matched_terms.contains(*term))
        .cloned()
        .collect();
    let mut below_bar = false;
    // What each term is worth toward the close bar, in half words. A word a field holds is two, and
    // so is a word of the product's own vocabulary that a stand-in reaches. A word only an
    // everyday stand-in reaches (`FIRST_EVERYDAY_GROUP`) is worth something only beside company:
    // a second word of the question that the title or topic keys hold, or two more words the
    // decision holds anywhere. Else one everyday word ("rebrand") meets the bar of a question of
    // two or three words, and the cafe's rebrand lists the decision about renaming the website
    // (hivemind-4fzjf). With company it is two when the title says the stand-in and the topic or
    // question says it too, or the title also holds two of the question's own words: the decision
    // is about that thing. Anywhere else (a rationale, a title that stands alone) it is one. Else
    // a question about the cake at a retirement presentation lists the pitch-deck decisions,
    // whose rationale says "presentation", for a word the question uses in its everyday sense
    // (hivemind-k25y8).
    let company = !headline_words.is_empty() || held_words.len() >= 2;
    let halves: usize = terms
        .iter()
        .enumerate()
        .map(|(index, term)| {
            if worded_terms.contains(term) {
                2
            } else if !matched_terms.contains(term) {
                0
            } else if !search_terms
                .related
                .get(index)
                .is_some_and(RelatedWord::is_everyday)
            {
                2
            } else if !company {
                0
            } else if stand_in_title.contains(&index)
                && (stand_in_about.contains(&index) || title_words.len() >= 2)
            {
                2
            } else {
                1
            }
        })
        .sum();
    if !missing_terms.is_empty() && !search_terms.close.accepts(halves, 2 * terms.len()) {
        // Too few of the words for the bar. A decision holding one of them as a word is not
        // dropped yet: whether the words it holds are rare is known only across all decisions
        // (`admit_below_bar`).
        if reads && !held_words.is_empty() {
            below_bar = true;
        } else {
            return None;
        }
    }

    // The title is the sentence that states what was decided; a rationale says "not" for a dozen
    // reasons that leave the decision itself positive. A decision is the opposite of a negated
    // question when its title says everything the question denies, outright: "Adopt Kafka" for
    // "why didn't we adopt Kafka". A title that leaves a word of the question out, or denies it,
    // states something else, and is ranked by how much of the question it carries like any other.
    // A question quoted as recorded is the one the decision answers, whatever its words ("... or
    // no Jev?", "... without judging"): its polarity is the question's own, never the opposite of
    // the decision.
    let polarity_mismatch = search_terms.negated
        && !quotes_question
        && fields.iter().any(|field| {
            field.field == "decision.title" && says_every_term(search_terms, &field.value)
        });

    let headline_matched = headline_terms.len();
    Some(SearchMatchInfo {
        rank,
        below_bar,
        held_words,
        headline_words,
        headline_matched,
        headline_any,
        title_words,
        about_words,
        spelled_words,
        stand_in_headline,
        stand_in_title,
        stand_in_reached: reached,
        named,
        headline_hits,
        polarity_mismatch,
        headline_terms: if missing_terms.is_empty() {
            0
        } else {
            headline_terms.len()
        },
        stand_in_terms: matched_terms.len() - worded_terms.len(),
        missing_terms,
        matched_fields: matched_fields.into_iter().collect(),
        snippets,
        matched_nodes: matched_nodes.into_iter().collect(),
    })
}

/// Whether `title` says every one of the question's terms outright: each is in it, as a word, a
/// form of one or a part of a longer word, outside any negation ("Do not adopt Kafka" says neither
/// "adopt" nor "kafka" outright). A word a stand-in only stands in for is not said.
fn says_every_term(search_terms: &SearchTerms<'_>, title: &str) -> bool {
    let outright = outside_negation(title).to_ascii_lowercase();
    !search_terms.terms.is_empty()
        && search_terms.terms.iter().enumerate().all(|(index, term)| {
            outright.contains(term.as_str())
                || holds_whole_word(search_terms.related.get(index), &outright, term)
        })
}

/// Whether `text` (lowercase) holds `term` as a word or a form of one, never as a part of a longer
/// word: "off" is not in "offset". `related` is the term's forms when the asker reads them.
fn holds_whole_word(related: Option<&RelatedWord<'_>>, text: &str, term: &str) -> bool {
    match related {
        Some(related) => related.find_in(text) == Some(WordMatch::Word),
        None => word_stems(text).contains(stem(term)),
    }
}

fn exact_id_or_title_match(query: &str, fields: &[SearchField]) -> bool {
    let query = query.to_ascii_lowercase();
    fields.iter().any(|field| {
        matches!(field.field.as_str(), "decision.id" | "decision.title")
            && field.value.to_ascii_lowercase() == query
    })
}

/// Whether the query is the question the decision answers, as recorded: matched without case and
/// without a closing "?", since a person asking it back rarely types them the way it was recorded.
fn exact_question_match(query: &str, fields: &[SearchField]) -> bool {
    let asked = without_closing_marks(query);
    !asked.is_empty()
        && fields.iter().any(|field| {
            field.field == "decision.question"
                && without_closing_marks(&field.value).eq_ignore_ascii_case(asked)
        })
}

/// `text` without the whitespace and question marks that close it.
fn without_closing_marks(text: &str) -> &str {
    text.trim_end_matches(|character: char| character == '?' || character.is_whitespace())
        .trim_start()
}

fn snippet_value(value: &str) -> String {
    let mut snippet = value.chars().take(SNIPPET_MAX_CHARS).collect::<String>();
    if value.chars().count() > SNIPPET_MAX_CHARS {
        snippet.push_str("...");
    }
    snippet
}

fn decision_status_label(status: DecisionStatus) -> &'static str {
    match status {
        DecisionStatus::Proposed => "proposed",
        DecisionStatus::Accepted => "accepted",
        DecisionStatus::Rejected => "rejected",
        DecisionStatus::Contested => "contested",
        DecisionStatus::Superseded => "superseded",
    }
}
