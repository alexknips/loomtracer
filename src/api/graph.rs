//! `GET /v1/graph` — the decision graph of a tenant, with `?project=` one page of a project's
//! decisions and what hangs off them, or with `?decision=` one decision and what hangs off it,
//! plus the projected [`MemoryGraph`] cache shared by every read handler across the API (avoids
//! replaying the whole ledger on each request).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::events::{EventId, TenantId};
use crate::ledger::EventLedger;
use crate::projector::arrow::Arrow;
use crate::projector::{
    memory::MemoryGraph, project_from_ledger_for_tenant_reporting, GraphParams, GraphRow,
    GraphValue, GraphView, NodeKind,
};
use crate::queries::{
    decision_graph, oriented_edges, project_graph_page, reaffirmation_arrows, DecisionStandings,
    DerivedArrow, ProjectGraphCursor, ProjectGraphRequest, MAX_QUERY_RESULTS,
};

use super::auth::extract_ctx;
use super::projects::named_project;
use super::{respond, ApiBackend, ApiError, ApiLedger, ApiRequestCtx, ApiResult, AppState};

/// How many of a project's decisions `GET /v1/graph?project=` returns when it is not asked for a
/// number: a page a UI draws at once. The most it returns is [`MAX_QUERY_RESULTS`].
const PROJECT_GRAPH_DEFAULT_LIMIT: usize = 200;

/// Query parameters of `GET /v1/graph`. Without `project` or `decision` it answers for the whole
/// tenant and takes no `limit` or `cursor`; `decision` takes none of the others.
#[derive(Debug, serde::Deserialize)]
pub(super) struct GraphQuery {
    /// A project: a registered handle or a personal address (`personal:agent:claude`).
    project: Option<String>,
    /// How many of the project's decisions the page holds (default 200, at most 1000).
    limit: Option<usize>,
    /// The `next_cursor` of the previous page.
    cursor: Option<String>,
    /// One decision's id: it and what hangs off it, not a project's page.
    decision: Option<String>,
}

/// What a request to `GET /v1/graph` asks the graph for.
enum GraphScope {
    Tenant,
    Project {
        project: String,
        limit: usize,
        cursor: Option<ProjectGraphCursor>,
    },
    Decision(String),
}

/// A single node in the decision graph, as returned by GET /v1/graph.
#[derive(Debug, serde::Serialize)]
struct GraphNode {
    id: String,
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    /// On an Option: its own label, else its id, so an option always has something to show.
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    /// On an Option: what the capture recorded, when that reads differently from its label
    /// (`name-a-upheld` for `Upheld`). The ledger is immutable and the label is a reading of it.
    #[serde(skip_serializing_if = "Option::is_none")]
    recorded_as: Option<String>,
    /// On a Decision: when the capture event recorded it, ISO-8601 UTC. `None` only for a
    /// decision from an event predating the ledger's `ts` backfill.
    #[serde(skip_serializing_if = "Option::is_none")]
    decided_at: Option<String>,
    /// On a Decision: its stable link segment (`/decisions/<slug>`), assigned once at
    /// proposal and unchanged by anything projected afterwards (docs/GRAPH_CONTRACT.md).
    /// `None` only for a decision from an event predating this field.
    #[serde(skip_serializing_if = "Option::is_none")]
    slug: Option<String>,
}

/// A directed edge in the decision graph: an arrow from the newer node to the older node.
#[derive(Clone, Debug, serde::Serialize)]
struct GraphEdge {
    /// The newer node (or, for equal ages, the stored source).
    from: String,
    /// The older node.
    to: String,
    /// What the edge means, unchanged whichever way the arrow runs.
    relation: String,
    /// The relation read along the arrow, an active phrase: `based on`, `informs`.
    label: &'static str,
    /// True when the arrow runs against the relation's stored direction.
    reversed: bool,
    /// True for a line drawn from a node's own properties that the graph does not store as an
    /// edge (a `Reaffirmation` to the decision it concerns and the premise that changed).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    derived: bool,
}

/// Decision graph in the shape expected by the SPA: a tenant's whole graph, or one page of a
/// project's. Each entry of `decisions` is the decision's stored row plus its derived `status`
/// (`proposed`, `accepted`, `rejected`, `contested` or `superseded`), `deciders` (the
/// `ACCEPTED_BY` actors, each with a `kind` of `human`, `agent` or `unknown`; empty until someone
/// accepts) and `decided_at` (the capture event's timestamp, ISO-8601 UTC).
#[derive(Debug, serde::Serialize)]
struct GraphData {
    decisions: Vec<serde_json::Value>,
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
    /// Present on a project's page only.
    #[serde(flatten)]
    page: Option<GraphPage>,
}

/// Where a page of a project's graph sits in the project (docs/GRAPH_CONTRACT.md).
#[derive(Debug, serde::Serialize)]
struct GraphPage {
    /// The address the project's decisions are filed under (a personal address is canonical:
    /// every session of one agent tool is one project).
    project: String,
    /// How many of the project's decisions this page holds at most.
    limit: usize,
    /// Every decision the project holds, on this page or not.
    total_matches: usize,
    /// True when older decisions remain; `next_cursor` is where they start.
    truncated: bool,
    next_cursor: Option<String>,
}

// ---------------------------------------------------------------------------
// Graph handler
// ---------------------------------------------------------------------------

pub(super) async fn graph_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<GraphQuery>,
) -> Response {
    let ctx = match extract_ctx(&state, &headers).await {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };
    let backend = Arc::clone(&state.backend);
    let cache = Arc::clone(&state.graph_cache);
    let result =
        tokio::task::spawn_blocking(move || graph_blocking(&backend, &ctx, &cache, query)).await;
    respond(result, StatusCode::OK)
}

fn graph_err(e: impl std::fmt::Display) -> ApiError {
    ApiError::internal(e.to_string())
}

fn graph_blocking(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    cache: &GraphCache,
    query: GraphQuery,
) -> ApiResult<serde_json::Value> {
    let scope = match query {
        GraphQuery {
            project: None,
            limit: None,
            cursor: None,
            decision: None,
        } => GraphScope::Tenant,
        GraphQuery {
            project: None,
            limit: None,
            cursor: None,
            decision: Some(decision),
        } => {
            let decision = decision.trim();
            if decision.is_empty() {
                return Err(ApiError::validation("`decision` must name a decision"));
            }
            GraphScope::Decision(decision.to_owned())
        }
        GraphQuery {
            decision: Some(_), ..
        } => {
            return Err(ApiError::validation(
                "`decision` reads one decision's graph on its own: `project`, `limit` and `cursor` page a project's graph",
            ))
        }
        GraphQuery { project: None, .. } => {
            return Err(ApiError::validation(
                "`limit` and `cursor` page one project's graph: name it with `project`",
            ))
        }
        GraphQuery {
            project: Some(project),
            limit,
            cursor,
            decision: None,
        } => {
            let cursor = cursor
                .map(|text| {
                    ProjectGraphCursor::parse(&text).ok_or_else(|| {
                        ApiError::validation(format!(
                            "cursor is not one this endpoint gave: {text}"
                        ))
                    })
                })
                .transpose()?;
            let limit = match limit {
                None | Some(0) => PROJECT_GRAPH_DEFAULT_LIMIT,
                Some(limit) => limit.min(MAX_QUERY_RESULTS),
            };
            GraphScope::Project {
                project,
                limit,
                cursor,
            }
        }
    };

    let ledger = backend.open_ledger_for_tenant(&ctx.tenant_id)?;
    let graph = get_cached_graph(&ledger, &ctx.tenant_id, cache)?;
    let standings = DecisionStandings::load(graph.as_ref()).map_err(graph_err)?;
    let data = match scope {
        GraphScope::Tenant => whole_graph(graph.as_ref(), &standings)?,
        GraphScope::Project {
            project,
            limit,
            cursor,
        } => {
            let address = named_project(graph.as_ref(), &project)?;
            project_graph(graph.as_ref(), &standings, address, limit, cursor)?
        }
        GraphScope::Decision(decision_id) => {
            decision_graph_data(graph.as_ref(), &standings, &decision_id)?
        }
    };
    serde_json::to_value(data).map_err(graph_err)
}

/// Every node, decision and edge of the tenant.
fn whole_graph(graph: &impl GraphView, standings: &DecisionStandings) -> ApiResult<GraphData> {
    let mut nodes: Vec<GraphNode> = Vec::new();
    let mut decisions: Vec<serde_json::Value> = Vec::new();

    for kind in NodeKind::ALL {
        let rows = graph
            .query(&graph_node_query(kind), &GraphParams::new())
            .map_err(graph_err)?;
        for row in rows {
            let (node, decision) = graph_entry(kind, &row, standings)?;
            nodes.push(node);
            decisions.extend(decision);
        }
    }

    // Every arrow runs from the newer node to the older node (docs/GRAPH_CONTRACT.md); the
    // label reads along that direction, so the SPA draws `from -> to` as given.
    let edges: Vec<GraphEdge> = oriented_edges(graph)
        .map_err(graph_err)?
        .iter()
        .map(graph_edge)
        .chain(
            // A reaffirmation has no stored edge; drawn alone it would float free of the
            // decision it is about. These lines are read from its own properties.
            reaffirmation_arrows(graph)
                .map_err(graph_err)?
                .iter()
                .map(derived_edge),
        )
        .collect();

    Ok(GraphData {
        decisions,
        nodes,
        edges,
        page: None,
    })
}

/// One page of a project's graph: the same nodes, decisions and edges as the whole graph, for
/// the project's newest `limit` decisions (older than `cursor`) and what hangs off them.
fn project_graph(
    graph: &impl GraphView,
    standings: &DecisionStandings,
    address: String,
    limit: usize,
    cursor: Option<ProjectGraphCursor>,
) -> ApiResult<GraphData> {
    let page = project_graph_page(
        graph,
        &ProjectGraphRequest {
            address: address.clone(), // ubs:ignore: the response names the project; the request owns its copy
            limit,
            cursor,
        },
    )
    .map_err(graph_err)?;

    shaped(
        graph,
        standings,
        &page.nodes,
        &page.arrows,
        &page.derived,
        Some(GraphPage {
            project: address,
            limit,
            total_matches: page.total_matches,
            truncated: page.next_cursor.is_some(),
            next_cursor: page.next_cursor,
        }),
    )
}

/// One decision and what hangs off it: the same nodes, decisions and edges as the whole graph,
/// for the decision and the nodes it is linked to. 404 when the graph holds no such decision.
fn decision_graph_data(
    graph: &impl GraphView,
    standings: &DecisionStandings,
    decision_id: &str,
) -> ApiResult<GraphData> {
    let slice = decision_graph(graph, decision_id)
        .map_err(graph_err)?
        .ok_or_else(|| ApiError::not_found(format!("decision not found: {decision_id}")))?;
    shaped(
        graph,
        standings,
        &slice.nodes,
        &slice.arrows,
        &slice.derived,
        None,
    )
}

/// The response for a selection of the graph: each node read from the graph, a decision also as
/// an entry of `decisions`, each arrow as an edge.
fn shaped(
    graph: &impl GraphView,
    standings: &DecisionStandings,
    page_nodes: &[(NodeKind, String)],
    arrows: &[Arrow],
    derived: &[DerivedArrow],
    page: Option<GraphPage>,
) -> ApiResult<GraphData> {
    let mut nodes: Vec<GraphNode> = Vec::with_capacity(page_nodes.len());
    let mut decisions: Vec<serde_json::Value> = Vec::new();
    for (kind, id) in page_nodes {
        let rows = graph
            .query(
                &graph_node_lookup(*kind),
                &GraphParams::from([("id".to_owned(), GraphValue::String(id.clone()))]), // ubs:ignore: the query owns its parameter
            )
            .map_err(graph_err)?;
        // A node the graph does not hold is left out, as it is from the whole graph's `nodes`.
        let Some(row) = rows
            .into_iter()
            .find(|row| row_string(row, "id").as_deref() == Some(id.as_str()))
        else {
            continue;
        };
        let (node, decision) = graph_entry(*kind, &row, standings)?;
        nodes.push(node);
        decisions.extend(decision);
    }
    let edges: Vec<GraphEdge> = arrows
        .iter()
        .map(graph_edge)
        .chain(derived.iter().map(derived_edge))
        .collect();

    Ok(GraphData {
        decisions,
        nodes,
        edges,
        page,
    })
}

/// The graph node a row is and, for a Decision, its entry in `decisions`.
fn graph_entry(
    kind: NodeKind,
    row: &GraphRow,
    standings: &DecisionStandings,
) -> ApiResult<(GraphNode, Option<serde_json::Value>)> {
    let kind_name = kind.table_name();
    let id = row_string(row, "id").unwrap_or_default();
    // `.get(key)` on a Null-valued key (e.g. an unset "display_name") returns
    // `Some(GraphValue::Null)`, which would short-circuit `.or_else` before it ever
    // tries the next candidate key — so filter to String values before chaining.
    let string_field = |key: &str| match row.get(key) {
        Some(GraphValue::String(s)) => Some(s.clone()), // ubs:ignore: clone necessary — GraphValue holds borrowed ref, GraphNode needs owned String
        _ => None,
    };
    let label = string_field("title")
        .or_else(|| string_field("label"))
        .or_else(|| string_field("statement"))
        .or_else(|| string_field("content"))
        .or_else(|| string_field("display_name"))
        .or_else(|| string_field("handle"))
        // A question and an ask read as the words that were asked.
        .or_else(|| string_field("text"))
        // A reaffirmation, a blocker and a decision request read as their one-line reason.
        .or_else(|| {
            matches!(
                kind,
                NodeKind::Reaffirmation | NodeKind::Blocker | NodeKind::DecisionRequest
            )
            .then(|| string_field("reason"))
            .flatten()
        });
    // The capture event's timestamp (`occurred_at` on the projected node) read
    // under the response's `decided_at` name (docs/GRAPH_CONTRACT.md).
    let decided_at = matches!(kind, NodeKind::Decision)
        .then(|| string_field("occurred_at"))
        .flatten();
    let slug = matches!(kind, NodeKind::Decision)
        .then(|| string_field("slug"))
        .flatten();
    let decision = if matches!(kind, NodeKind::Decision) {
        let mut obj: serde_json::Map<String, serde_json::Value> = row
            .iter()
            .map(|(k, v)| (k.into(), graph_value_to_json(v)))
            .collect();
        obj.insert(
            "status".into(),
            serde_json::to_value(standings.status_of(&id)).map_err(graph_err)?,
        );
        obj.insert(
            "deciders".into(),
            serde_json::to_value(standings.deciders_of(&id)).map_err(graph_err)?,
        );
        // Raw column selected only to compute `decided_at` below; drop it so the
        // response carries one timestamp field, not two names for the same value.
        obj.remove("occurred_at");
        obj.insert(
            "decided_at".into(),
            decided_at
                .clone()
                .map_or(serde_json::Value::Null, serde_json::Value::String),
        );
        Some(serde_json::Value::Object(obj))
    } else {
        None
    };
    // Same fallback every other reader of an option uses (`resolve_option_label`).
    let title =
        matches!(kind, NodeKind::Option).then(|| label.clone().unwrap_or_else(|| id.clone())); // ubs:ignore: one owned title per option node
    let recorded_as = matches!(kind, NodeKind::Option)
        .then(|| string_field("recorded_label"))
        .flatten()
        .filter(|recorded| Some(recorded) != label.as_ref());
    let node = GraphNode {
        id: [kind_name, ":", &id].concat(),
        kind: kind_name.into(),
        label,
        title,
        recorded_as,
        decided_at,
        slug,
    };
    Ok((node, decision))
}

fn graph_edge(arrow: &Arrow) -> GraphEdge {
    GraphEdge {
        from: [arrow.from_kind.table_name(), ":", &arrow.from_id].concat(),
        to: [arrow.to_kind.table_name(), ":", &arrow.to_id].concat(),
        relation: arrow.relation.table_name().into(),
        label: arrow.label,
        reversed: arrow.reversed,
        derived: false,
    }
}

fn derived_edge(arrow: &DerivedArrow) -> GraphEdge {
    GraphEdge {
        from: [arrow.from_kind.table_name(), ":", &arrow.from_id].concat(),
        to: [arrow.to_kind.table_name(), ":", &arrow.to_id].concat(),
        relation: arrow.relation.into(),
        label: arrow.label,
        reversed: false,
        derived: true,
    }
}

fn row_string(row: &GraphRow, key: &str) -> Option<String> {
    row.get(key).and_then(|v| match v {
        GraphValue::String(s) => Some(s.to_owned()),
        _ => None,
    })
}

fn graph_node_projection(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Decision => {
            "node.id AS id, node.title AS title, node.rationale AS rationale, node.topic_keys AS topic_keys, node.occurred_at AS occurred_at, node.slug AS slug"
        }
        NodeKind::DecisionRequest => {
            "node.id AS id, node.topic_keys AS topic_keys, node.reason AS reason"
        }
        NodeKind::Actor => "node.id AS id",
        NodeKind::Blocker => "node.id AS id, node.reason AS reason",
        NodeKind::Evidence => "node.id AS id, node.content AS content",
        NodeKind::Notification => "node.id AS id",
        NodeKind::Option => {
            "node.id AS id, node.label AS label, node.recorded_label AS recorded_label, node.description AS description"
        }
        NodeKind::Hypothesis => "node.id AS id, node.statement AS statement",
        NodeKind::Question => "node.id AS id, node.text AS text",
        NodeKind::Project => "node.id AS id, node.handle AS handle, node.display_name AS display_name",
        NodeKind::Ask => "node.id AS id, node.text AS text, node.question_id AS question_id",
        NodeKind::Reaffirmation => "node.id AS id, node.decision_id AS decision_id, node.premise_id AS premise_id, node.reason AS reason, node.reaffirmed_by AS reaffirmed_by, node.reaffirmed_at AS reaffirmed_at",
    }
}

fn graph_node_query(kind: NodeKind) -> String {
    format!(
        "MATCH (node:`{}`) RETURN {} ORDER BY node.id;",
        kind.table_name(),
        graph_node_projection(kind)
    )
}

/// The same projection for one node, by `$id`.
fn graph_node_lookup(kind: NodeKind) -> String {
    format!(
        "MATCH (node:`{}` {{id: $id}}) RETURN {} LIMIT 1;",
        kind.table_name(),
        graph_node_projection(kind)
    )
}

fn graph_value_to_json(v: &GraphValue) -> serde_json::Value {
    match v {
        GraphValue::Null => serde_json::Value::Null,
        GraphValue::Bool(b) => serde_json::Value::Bool(*b),
        GraphValue::Int(i) => serde_json::Value::Number(serde_json::Number::from(*i)),
        GraphValue::Float(f) => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        GraphValue::String(s) => serde_json::Value::String(s.to_owned()),
        GraphValue::StringList(list) => serde_json::Value::Array(
            list.iter()
                .map(|s| serde_json::Value::String(s.to_owned()))
                .collect(),
        ),
    }
}

// ---------------------------------------------------------------------------
// Graph cache — shared by every handler that needs a projected MemoryGraph.
// ---------------------------------------------------------------------------

/// The projected graph of every tenant that has been read, shared by every read handler.
///
/// Each tenant has a slot of its own, so one tenant's refresh never holds up another tenant's
/// reads: the map of slots is locked for a lookup, and a slot is locked to read or swap the graph
/// it publishes, never while a graph is cloned, replayed into or freed (hivemind-cfiff).
#[derive(Default)]
pub(super) struct GraphCache {
    slots: RwLock<HashMap<TenantId, Arc<TenantGraph>>>,
}

#[derive(Default)]
struct TenantGraph {
    /// Held for a whole refresh, so the refreshes of one tenant run one after another and each
    /// starts from the graph the one before it published: no event is applied twice.
    refresh: Mutex<()>,
    /// The graph the last refresh published and the last event it applied.
    published: RwLock<Option<(EventId, Arc<MemoryGraph>)>>,
}

impl GraphCache {
    fn slot(&self, tenant_id: &TenantId) -> ApiResult<Arc<TenantGraph>> {
        {
            let slots = self.slots.read().map_err(|_| cache_poisoned())?;
            if let Some(slot) = slots.get(tenant_id) {
                return Ok(Arc::clone(slot));
            }
        }
        let mut slots = self.slots.write().map_err(|_| cache_poisoned())?;
        Ok(Arc::clone(slots.entry(tenant_id.clone()).or_default()))
    }
}

impl TenantGraph {
    fn published(&self) -> ApiResult<Option<(EventId, Arc<MemoryGraph>)>> {
        let published = self.published.read().map_err(|_| cache_poisoned())?;
        Ok(published
            .as_ref()
            .map(|(offset, graph)| (*offset, Arc::clone(graph))))
    }

    /// The published graph when it has applied every event up to `latest`.
    fn published_through(&self, latest: EventId) -> ApiResult<Option<Arc<MemoryGraph>>> {
        let published = self.published.read().map_err(|_| cache_poisoned())?;
        Ok(published
            .as_ref()
            .filter(|(offset, _)| *offset >= latest)
            .map(|(_, graph)| Arc::clone(graph)))
    }

    /// Swaps in `graph`. The write lock is held for the swap only, and what it replaces is not
    /// freed here: the caller still holds it and frees it after the refresh lock is released.
    fn publish(&self, applied_through: EventId, graph: Arc<MemoryGraph>) -> ApiResult<()> {
        let mut published = self.published.write().map_err(|_| cache_poisoned())?;
        *published = Some((applied_through, graph));
        Ok(())
    }
}

fn cache_poisoned() -> ApiError {
    graph_err("graph cache poisoned")
}

/// Return a projected graph for `tenant_id`, served from the in-memory cache
/// when the ledger offset is unchanged, or rebuilt incrementally otherwise.
pub(super) fn get_cached_graph(
    ledger: &impl EventLedger,
    tenant_id: &TenantId,
    cache: &GraphCache,
) -> ApiResult<Arc<MemoryGraph>> {
    let latest = ledger
        .latest_offset_for_tenant(tenant_id)
        .map_err(graph_err)?;
    let slot = cache.slot(tenant_id)?;

    // Fast path: no allocation when the offset matches.
    if let Some(graph) = slot.published_through(latest)? {
        return Ok(graph);
    }

    // Slow path: one refresh per tenant at a time. The replaced graph is freed once the refresh
    // lock is released too, so the next refresh of this tenant does not wait for the free.
    let (new_graph, _replaced) = {
        let _refreshing = slot.refresh.lock().map_err(|_| cache_poisoned())?;

        // The refresh that held the lock before this one may have applied what this one is for.
        let published = slot.published()?;
        if let Some((cached_offset, graph)) = &published {
            if *cached_offset >= latest {
                return Ok(Arc::clone(graph));
            }
        }

        let (base_graph, base_offset) = match &published {
            Some((cached_offset, cached_graph)) => (cached_graph.as_ref().clone(), *cached_offset),
            None => (MemoryGraph::default(), 0),
        };
        let report =
            project_from_ledger_for_tenant_reporting(ledger, tenant_id, &base_graph, base_offset)
                .map_err(graph_err)?;
        base_graph.note_unreadable_annotations(report.unreadable_annotations);
        let new_graph = Arc::new(base_graph);
        // The last event the replay applied, not the `latest` read before it: the replay runs to
        // the end of the ledger, so an event appended since that read is in this graph and the
        // next refresh must not apply it again.
        let applied_through = report.last_event_id.unwrap_or(base_offset);
        slot.publish(applied_through, Arc::clone(&new_graph))?;
        (new_graph, published)
    };
    Ok(new_graph)
}

/// [`get_cached_graph`] for a caller outside the HTTP layer that wants the failure as the crate's
/// own error: the MCP core and the grounding resolver, which look a decision up while a write
/// runs and must not replay the whole ledger to do it (hivemind-f38ka).
pub(super) fn get_cached_graph_for_core(
    ledger: &ApiLedger,
    tenant_id: &TenantId,
    cache: &GraphCache,
) -> crate::Result<Arc<MemoryGraph>> {
    get_cached_graph(ledger, tenant_id, cache)
        .map_err(|e| crate::error::ProjectorError::Projection(e.to_string()).into())
}

/// Project the graph of every registered tenant into `cache`, so the first read after a start
/// finds it built instead of replaying the ledger.
///
/// A tenant whose graph cannot be built is logged and skipped: the server still starts, and that
/// tenant's first read retries the build and reports the error the way it always did.
pub(super) fn warm_graph_cache(backend: &ApiBackend, cache: &GraphCache) {
    let started = std::time::Instant::now();
    let tenants = match backend.known_tenant_ids() {
        Ok(tenants) => tenants,
        Err(e) => {
            tracing::warn!(target: "loomtracer::api", error = %e, "graph cache warm-up could not list tenants; the first read of each tenant builds its graph");
            return;
        }
    };
    let mut warmed = 0_usize;
    for tenant_id in &tenants {
        let result = backend
            .open_ledger_for_tenant(tenant_id)
            .and_then(|ledger| get_cached_graph(&ledger, tenant_id, cache));
        match result {
            Ok(_) => warmed += 1,
            Err(e) => {
                tracing::warn!(target: "loomtracer::api", tenant = %tenant_id, error = %e, "graph cache warm-up failed for a tenant; its first read builds the graph");
            }
        }
    }
    tracing::info!(
        target: "loomtracer::api",
        tenants = tenants.len(),
        warmed,
        elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        "graph cache warmed"
    );
}

pub(super) fn open_graph_from_ledger(
    ledger: &ApiLedger,
    tenant_id: &TenantId,
    cache: &GraphCache,
) -> ApiResult<Arc<MemoryGraph>> {
    get_cached_graph(ledger, tenant_id, cache)
}

#[cfg(test)]
mod tests;
