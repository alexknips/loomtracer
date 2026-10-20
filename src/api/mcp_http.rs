//! MCP Streamable HTTP transport (`POST /mcp`, 2025-03-26 spec).
//!
//! Each POST carries a single JSON-RPC 2.0 request and receives a single
//! JSON-RPC 2.0 response (no SSE in this implementation — MCP allows plain
//! JSON for non-streaming tools). Auth uses the same bearer-token path as
//! the REST API ([`super::auth::extract_ctx`]). The `Mcp-Session-Id` header
//! is issued on `initialize` and accepted (but not enforced) on subsequent
//! requests; under a person's credential it seeds the default actor_id for
//! write operations (an agent token's own actor wins, see [`mcp_resolve_actor`]).
//!
//! A read tool is cut off after the server's read timeout, like a `GET`, and answers a tool
//! error (`isError: true`) saying so. A write tool is never cut off: it runs to its result
//! (hivemind-s9cx). Which is which comes from `TOOL_KINDS` (hivemind-g3vd0).

use std::sync::Arc;

use axum::extract::{Json, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::events::TenantId;
use crate::mcp::core::{
    AcknowledgeSuggestionArgs, AnalyzeFailureModesArgs, CaptureDecisionArgs, CaptureEvidenceArgs,
    CaptureHypothesisArgs, ClassifyQueueListArgs, ClassifyQueueSubmitArgs, CompactViewArgs,
    CoreError, DecisionContextCandidatesArgs, DecisionQualityCandidatesArgs, DisagreeArgs,
    GetChangedDecisionsArgs, GetContestedDecisionsArgs, GetDecisionArgs, GetDecisionContextArgs,
    GetDecisionNeighborhoodArgs, GetDecisionOutcomeArgs, GetOpenWhyDecisionsArgs,
    GetReexamineItemsArgs, GetRelevantDecisionsArgs, GetSituationalDecisionsArgs,
    GetSuggestionsArgs, GetSupersessionChainArgs, GetWaitingRequestsArgs, GraphSource,
    GroundDecisionArgs, LedgerHandle, LedgerProvider, LinkSupersessionArgs, MoveDecisionArgs,
    RankDecisionsByImportanceArgs, ReaffirmDecisionArgs, RecallDecisionsArgs, RecentDecisionsArgs,
    RequestDecisionArgs, RetitleDecisionArgs, ScanDecisionQualityArgs, ScanMisfiledDecisionsArgs,
    ScoreDecisionArgs, SearchDecisionsArgs, SummarizeDecisionsArgs, SupersedeDecisionArgs,
};
use crate::projector::memory::MemoryGraph;

use super::auth::extract_ctx;
use super::graph::{get_cached_graph, get_cached_graph_for_core};
use super::{ApiBackend, ApiLedger, ApiRequestCtx, AppState, GraphCache, HEADER_FORWARDED_PROTO};

pub(super) async fn mcp_http_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let ctx = match extract_ctx(&state, &headers).await {
        Ok(c) => c,
        Err(e) => {
            let mut resp = e.into_response();
            // MCP clients use WWW-Authenticate to discover the PRM endpoint (RFC 9728 §5).
            if resp.status() == StatusCode::UNAUTHORIZED {
                let scheme = headers
                    .get(HEADER_FORWARDED_PROTO)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("https");
                let host = headers
                    .get("host")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("localhost");
                let prm_url = format!("{scheme}://{host}/.well-known/oauth-protected-resource");
                let www_auth = format!("Bearer resource_metadata=\"{prm_url}\"");
                if let Ok(v) = www_auth.parse::<axum::http::HeaderValue>() {
                    resp.headers_mut().insert("www-authenticate", v);
                }
            }
            return resp;
        }
    };

    let session_id = headers
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_owned())
        .unwrap_or_default();

    let parsed: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return mcp_error_response(
                serde_json::Value::Null,
                -32700,
                format!("invalid JSON: {e}"),
            )
        }
    };

    let obj = match parsed.as_object() {
        Some(o) => o,
        None => {
            return mcp_error_response(
                serde_json::Value::Null,
                -32600,
                "request must be a JSON object".into(),
            )
        }
    };

    // MCP notifications carry no `id` — acknowledge with 202 and no body.
    if !obj.contains_key("id") {
        return (StatusCode::ACCEPTED, "").into_response();
    }

    let id = obj.get("id").cloned().unwrap_or(serde_json::Value::Null);
    let method = match obj.get("method").and_then(|v| v.as_str()) {
        Some(m) => m.to_owned(),
        None => return mcp_error_response(id, -32600, "missing `method`".into()),
    };
    let params = obj
        .get("params")
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    match method.as_str() {
        "initialize" => {
            let new_session_id = uuid::Uuid::new_v4().to_string();
            let mut resp = mcp_success_response(id, crate::mcp::initialize_result());
            if let Ok(v) = new_session_id.parse::<axum::http::HeaderValue>() {
                resp.headers_mut().insert("mcp-session-id", v);
            }
            resp
        }
        "ping" => mcp_success_response(id, serde_json::json!({})),
        "tools/list" => mcp_success_response(id, crate::mcp::tools_list_result()),
        "tools/call" => {
            let backend = Arc::clone(&state.backend);
            let cache = Arc::clone(&state.graph_cache);
            let tool = params
                .get("name")
                .and_then(|name| name.as_str())
                .unwrap_or_default()
                .to_owned();
            let call = tokio::task::spawn_blocking(move || {
                mcp_tools_call_blocking(&backend, &ctx, &session_id, params, &cache)
            });
            // A read has nothing to commit, so it is cut off like a GET. A write is not: its
            // ledger work runs on after the answer is dropped, and "timed out" would be false
            // about an event that is recorded (hivemind-s9cx).
            let result = if tool_is_read(&tool) {
                match tokio::time::timeout(state.read_timeout, call).await {
                    Ok(result) => result,
                    Err(_elapsed) => {
                        return mcp_success_response(
                            id,
                            mcp_tool_err(format!(
                                "request timed out after {:?}: `{tool}` only reads and was cut off, \
                                 so nothing was recorded. Narrow the query or try again.",
                                state.read_timeout
                            )),
                        );
                    }
                }
            } else {
                call.await
            };
            match result {
                Ok(Ok(v)) => mcp_success_response(id, v),
                Ok(Err((code, msg))) => mcp_error_response(id, code, msg),
                Err(e) => mcp_error_response(id, -32603, e.to_string()),
            }
        }
        other => mcp_error_response(id, -32601, format!("unknown method: {other}")),
    }
}

/// True for a tool that only reads, so the server may stop waiting for it. The kind comes from
/// `TOOL_KINDS`; a name that is not a tool answers its error at once and needs no deadline.
fn tool_is_read(name: &str) -> bool {
    crate::mcp::core::tool_kind(name).is_some_and(|kind| kind.is_read())
}

fn mcp_success_response(id: serde_json::Value, result: serde_json::Value) -> Response {
    let body = serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result });
    (StatusCode::OK, Json(body)).into_response()
}

fn mcp_error_response(id: serde_json::Value, code: i32, message: String) -> Response {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message },
    });
    (StatusCode::OK, Json(body)).into_response()
}

type McpToolResult = std::result::Result<serde_json::Value, (i32, String)>;

fn mcp_tools_call_blocking(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    session_id: &str,
    params: serde_json::Value,
    cache: &Arc<GraphCache>,
) -> McpToolResult {
    let mut obj = match params {
        serde_json::Value::Object(map) => map,
        _ => return Err((-32602, "params must be an object".into())),
    };
    let name = match obj.remove("name") {
        Some(serde_json::Value::String(s)) => s,
        _ => return Err((-32602, "missing `name`".into())),
    };
    let arguments = obj
        .remove("arguments")
        .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
    let args = match arguments {
        serde_json::Value::Object(map) => map,
        _ => return Err((-32602, "`arguments` must be an object".into())),
    };

    if let Err(refused) = crate::mcp::core::reject_unknown_arguments(&name, &args) {
        return Ok(mcp_tool_err(refused.into_message()));
    }

    let actor_id = mcp_resolve_actor(&args, &ctx.actor_id, session_id);

    let outcome: McpToolResult = match name.as_str() {
        "capture_decision" => mcp_capture_decision(backend, ctx, &actor_id, args, cache),
        "capture_evidence" => mcp_capture_evidence(backend, ctx, &actor_id, args),
        "capture_hypothesis" => mcp_capture_hypothesis(backend, ctx, &actor_id, args),
        "disagree_decision" => mcp_disagree(backend, ctx, &actor_id, args, cache),
        "supersede_decision" => mcp_supersede(backend, ctx, &actor_id, args, cache),
        "link_supersession" => mcp_link_supersession(backend, ctx, &actor_id, args, cache),
        "move_decision" => mcp_move(backend, ctx, &actor_id, args, cache),
        "retitle_decision" => mcp_retitle(backend, ctx, &actor_id, args, cache),
        "acknowledge_suggestion" => mcp_acknowledge_suggestion(backend, ctx, &actor_id, args),
        "ground_decision" => mcp_ground(backend, ctx, &actor_id, args, cache),
        "reaffirm_decision" => mcp_reaffirm(backend, ctx, &actor_id, args, cache),
        "request_decision" => mcp_request_decision(backend, ctx, &actor_id, args),
        "get_waiting_requests" => mcp_get_waiting_requests(backend, ctx, args, cache),
        "get_contested_decisions" => mcp_get_contested_decisions(backend, ctx, args, cache),
        "get_open_why_decisions" => mcp_get_open_why_decisions(backend, ctx, args, cache),
        "get_reexamine_items" => mcp_get_reexamine_items(backend, ctx, args, cache),
        "get_changed_decisions" => mcp_get_changed_decisions(backend, ctx, args, cache),
        "get_decision" => mcp_get_decision(backend, ctx, args, cache),
        "get_decision_outcome" => mcp_get_decision_outcome(backend, ctx, args, cache),
        "decision_quality_candidates" => mcp_decision_quality_candidates(backend, ctx, args, cache),
        "get_decision_context" => mcp_get_decision_context(backend, ctx, args, cache),
        "decision_context_candidates" => mcp_decision_context_candidates(backend, ctx, args, cache),
        "get_relevant_decisions" => mcp_get_relevant_decisions(backend, ctx, args, cache),
        "get_situational_decisions" => mcp_get_situational_decisions(backend, ctx, args, cache),
        "get_supersession_chain" => mcp_get_supersession_chain(backend, ctx, args, cache),
        "get_decision_neighborhood" => mcp_get_decision_neighborhood(backend, ctx, args, cache),
        "recall_decisions" => mcp_recall_decisions(backend, ctx, args, cache),
        "search_decisions" => mcp_search_decisions(backend, ctx, args, cache),
        "recent_decisions" => mcp_recent_decisions(backend, ctx, args, cache),
        "score_decision" => mcp_score_decision(backend, ctx, args, cache),
        "scan_decision_quality" => mcp_scan_decision_quality(backend, ctx, args, cache),
        "get_suggestions" => mcp_get_suggestions(backend, ctx, args, cache),
        "rank_decisions_by_importance" => {
            mcp_rank_decisions_by_importance(backend, ctx, args, cache)
        }
        "scan_misfiled_decisions" => mcp_scan_misfiled_decisions(backend, ctx, args, cache),
        "analyze_failure_modes" => mcp_analyze_failure_modes(backend, ctx, args, cache),
        "dump_graph" => mcp_dump_graph(backend, ctx, cache),
        "hivemind_compact_view" => mcp_compact_view(backend, ctx, args, cache),
        "summarize_decisions" => mcp_summarize(backend, ctx, args, cache),
        "classify_queue_list" => mcp_classify_queue_list(backend, ctx, args),
        "classify_queue_submit" => mcp_classify_queue_submit(backend, ctx, args),
        other => return Err((-32602, format!("unknown tool: {other}"))),
    };

    // Per MCP spec: tool-level errors are returned as success responses with
    // `isError: true` rather than as JSON-RPC error objects.
    match outcome {
        Ok(mut payload) => {
            if crate::mcp::core::tool_reads_decisions(&name) {
                // The cached graph carries what its replays skipped, so the notice costs no
                // second pass over the ledger.
                let graph = mcp_open_graph(backend, ctx, cache)?;
                if let Some(notice) =
                    crate::read_notice::notice_for(&graph.unreadable_annotations())
                {
                    crate::read_notice::annotate_json(&mut payload, &notice);
                }
            }
            Ok(mcp_tool_ok(payload))
        }
        Err((_, msg)) => Ok(mcp_tool_err(msg)),
    }
}

/// Who a write is attributed to, first match wins:
///
/// 1. an explicit `actor_id` argument;
/// 2. the bearer token's own actor when it is an agent (`agent:<tool>:<name>`, an agent token,
///    hivemind-zdsh.19): the token already names the calling agent, and Claude Code sends a
///    fresh `Mcp-Session-Id` per connection, so keying on that filed every write under an id
///    nobody could recognise;
/// 3. `agent:mcp-http:<session>` when there is a session id. Under a person's credential (user
///    token, WorkOS JWT) the token's actor is `human:<email>`, and an agent's write must not
///    be recorded as that person: with a chosen option it would self-accept as them
///    (hivemind-zdsh.3/.6 attribution ruling);
/// 4. the token's actor (no session id).
fn mcp_resolve_actor(
    args: &serde_json::Map<String, serde_json::Value>,
    auth_actor: &str,
    session_id: &str,
) -> String {
    args.get("actor_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_owned())
        .unwrap_or_else(|| {
            if auth_actor.starts_with("agent:") || session_id.is_empty() {
                auth_actor.to_owned()
            } else {
                format!("agent:mcp-http:{session_id}")
            }
        })
}

fn mcp_open_graph(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    cache: &Arc<GraphCache>,
) -> std::result::Result<Arc<MemoryGraph>, (i32, String)> {
    let ledger = backend
        .open_ledger_for_tenant(&ctx.tenant_id)
        .map_err(|e| (-32603i32, e.to_string()))?;
    get_cached_graph(&ledger, &ctx.tenant_id, cache).map_err(|e| (-32603i32, e.to_string()))
}

fn mcp_tool_ok(payload: serde_json::Value) -> serde_json::Value {
    let text = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".into());
    serde_json::json!({
        "content": [{ "type": "text", "text": text }],
        "isError": false,
        "structuredContent": payload,
    })
}

fn mcp_tool_err(message: String) -> serde_json::Value {
    serde_json::json!({
        "content": [{ "type": "text", "text": message }],
        "isError": true,
    })
}

// ---------------------------------------------------------------------------
// Per-tool implementations
// ---------------------------------------------------------------------------

/// HTTP's [`LedgerProvider`]: resolves a tenant-scoped ledger from the
/// tenant already established by [`extract_ctx`]'s WorkOS/bearer-token
/// validation. Auth happened before this is ever called — the core this
/// feeds never sees the tenant negotiation, only its result.
struct HttpLedgerProvider<'a> {
    backend: &'a ApiBackend,
    tenant_id: &'a TenantId,
    /// The server's graph cache, for a tool that looks a decision up while it runs. With it the
    /// call reads the graph the cache keeps current (it replays only what was appended since its
    /// last offset); without it the core replays the whole ledger into a fresh graph, which on
    /// the city cell's 130k events is minutes per write (hivemind-f38ka).
    graph_cache: Option<&'a Arc<GraphCache>>,
}

impl LedgerProvider for HttpLedgerProvider<'_> {
    type Ledger = ApiLedger;

    fn ledger(&self) -> std::result::Result<LedgerHandle<Self::Ledger>, CoreError> {
        let ledger = self
            .backend
            .open_ledger_for_tenant(self.tenant_id)
            .map_err(|e| CoreError::Internal(e.to_string()))?;
        let graph_source = self.graph_cache.map(|cache| {
            let cache = Arc::clone(cache);
            Box::new(move |ledger: &ApiLedger, tenant_id: &TenantId| {
                get_cached_graph_for_core(ledger, tenant_id, &cache)
            }) as GraphSource<ApiLedger>
        });
        Ok(LedgerHandle {
            ledger,
            tenant_id: self.tenant_id.clone(),
            graph_source,
        })
    }
}

impl From<CoreError> for (i32, String) {
    fn from(error: CoreError) -> Self {
        let code = match &error {
            CoreError::InvalidArgument(_) => -32602,
            CoreError::Internal(_) => -32603,
        };
        (code, error.into_message())
    }
}

fn mcp_capture_decision(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    actor_id: &str,
    args: serde_json::Map<String, serde_json::Value>,
    cache: &Arc<GraphCache>,
) -> McpToolResult {
    let core_args = CaptureDecisionArgs::from_json(&args, actor_id.to_owned())?;
    let provider = HttpLedgerProvider {
        backend,
        tenant_id: &ctx.tenant_id,
        graph_cache: Some(cache),
    };
    let output = crate::mcp::core::capture_decision(&provider, core_args)?;
    Ok(output.into_value())
}

fn mcp_request_decision(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    actor_id: &str,
    args: serde_json::Map<String, serde_json::Value>,
) -> McpToolResult {
    let core_args = RequestDecisionArgs::from_json(&args, actor_id.to_owned())?;
    let provider = HttpLedgerProvider {
        backend,
        tenant_id: &ctx.tenant_id,
        graph_cache: None,
    };
    let output = crate::mcp::core::request_decision(&provider, core_args)?;
    Ok(output.into_value())
}

fn mcp_capture_evidence(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    actor_id: &str,
    args: serde_json::Map<String, serde_json::Value>,
) -> McpToolResult {
    let core_args = CaptureEvidenceArgs::from_json(&args, actor_id.to_owned())?;
    let provider = HttpLedgerProvider {
        backend,
        tenant_id: &ctx.tenant_id,
        graph_cache: None,
    };
    let output = crate::mcp::core::capture_evidence(&provider, core_args)?;
    Ok(output.into_value())
}

fn mcp_capture_hypothesis(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    actor_id: &str,
    args: serde_json::Map<String, serde_json::Value>,
) -> McpToolResult {
    let core_args = CaptureHypothesisArgs::from_json(&args, actor_id.to_owned())?;
    let provider = HttpLedgerProvider {
        backend,
        tenant_id: &ctx.tenant_id,
        graph_cache: None,
    };
    let output = crate::mcp::core::capture_hypothesis(&provider, core_args)?;
    Ok(output.into_value())
}

fn mcp_disagree(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    actor_id: &str,
    args: serde_json::Map<String, serde_json::Value>,
    cache: &Arc<GraphCache>,
) -> McpToolResult {
    let core_args = DisagreeArgs::from_json(&args, actor_id.to_owned())?;
    let provider = HttpLedgerProvider {
        backend,
        tenant_id: &ctx.tenant_id,
        graph_cache: Some(cache),
    };
    let output = crate::mcp::core::disagree_decision(&provider, core_args)?;
    Ok(output.into_value())
}

fn mcp_supersede(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    actor_id: &str,
    args: serde_json::Map<String, serde_json::Value>,
    cache: &Arc<GraphCache>,
) -> McpToolResult {
    let core_args = SupersedeDecisionArgs::from_json(&args, actor_id.to_owned())?;
    let provider = HttpLedgerProvider {
        backend,
        tenant_id: &ctx.tenant_id,
        graph_cache: Some(cache),
    };
    let output = crate::mcp::core::supersede_decision(&provider, core_args)?;
    Ok(output.into_value())
}

fn mcp_link_supersession(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    actor_id: &str,
    args: serde_json::Map<String, serde_json::Value>,
    cache: &Arc<GraphCache>,
) -> McpToolResult {
    let core_args = LinkSupersessionArgs::from_json(&args, actor_id.to_owned())?;
    let provider = HttpLedgerProvider {
        backend,
        tenant_id: &ctx.tenant_id,
        graph_cache: Some(cache),
    };
    let output = crate::mcp::core::link_supersession(&provider, core_args)?;
    Ok(output.into_value())
}

fn mcp_move(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    actor_id: &str,
    args: serde_json::Map<String, serde_json::Value>,
    cache: &Arc<GraphCache>,
) -> McpToolResult {
    let core_args = MoveDecisionArgs::from_json(&args, actor_id.to_owned())?;
    let provider = HttpLedgerProvider {
        backend,
        tenant_id: &ctx.tenant_id,
        graph_cache: Some(cache),
    };
    let output = crate::mcp::core::move_decision(&provider, core_args)?;
    Ok(output.into_value())
}

fn mcp_retitle(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    actor_id: &str,
    args: serde_json::Map<String, serde_json::Value>,
    cache: &Arc<GraphCache>,
) -> McpToolResult {
    let core_args = RetitleDecisionArgs::from_json(&args, actor_id.to_owned())?;
    let provider = HttpLedgerProvider {
        backend,
        tenant_id: &ctx.tenant_id,
        graph_cache: Some(cache),
    };
    let output = crate::mcp::core::retitle_decision(&provider, core_args)?;
    Ok(output.into_value())
}

fn mcp_acknowledge_suggestion(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    actor_id: &str,
    args: serde_json::Map<String, serde_json::Value>,
) -> McpToolResult {
    let core_args = AcknowledgeSuggestionArgs::from_json(&args, actor_id.to_owned())?;
    let provider = HttpLedgerProvider {
        backend,
        tenant_id: &ctx.tenant_id,
        graph_cache: None,
    };
    let output = crate::mcp::core::acknowledge_suggestion(&provider, core_args)?;
    Ok(output.into_value())
}

fn mcp_ground(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    actor_id: &str,
    args: serde_json::Map<String, serde_json::Value>,
    cache: &Arc<GraphCache>,
) -> McpToolResult {
    let core_args = GroundDecisionArgs::from_json(&args, actor_id.to_owned())?;
    let provider = HttpLedgerProvider {
        backend,
        tenant_id: &ctx.tenant_id,
        graph_cache: Some(cache),
    };
    let output = crate::mcp::core::ground_decision(&provider, core_args)?;
    Ok(output.into_value())
}

fn mcp_reaffirm(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    actor_id: &str,
    args: serde_json::Map<String, serde_json::Value>,
    cache: &Arc<GraphCache>,
) -> McpToolResult {
    let core_args = ReaffirmDecisionArgs::from_json(&args, actor_id.to_owned())?;
    let provider = HttpLedgerProvider {
        backend,
        tenant_id: &ctx.tenant_id,
        graph_cache: Some(cache),
    };
    let output = crate::mcp::core::reaffirm_decision(&provider, core_args)?;
    Ok(output.into_value())
}

fn mcp_get_reexamine_items(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    args: serde_json::Map<String, serde_json::Value>,
    cache: &Arc<GraphCache>,
) -> McpToolResult {
    let core_args = GetReexamineItemsArgs::from_json(&args)?;
    let graph = mcp_open_graph(backend, ctx, cache)?;
    let provider = HttpLedgerProvider {
        backend,
        tenant_id: &ctx.tenant_id,
        graph_cache: None,
    };
    let output = crate::mcp::core::get_reexamine_items(&provider, &*graph, core_args)?;
    Ok(output.into_value())
}

fn mcp_get_decision(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    args: serde_json::Map<String, serde_json::Value>,
    cache: &Arc<GraphCache>,
) -> McpToolResult {
    let core_args = GetDecisionArgs::from_json(&args)?;
    let graph = mcp_open_graph(backend, ctx, cache)?;
    let output = crate::mcp::core::get_decision(&*graph, core_args)?;
    Ok(output.into_value())
}

fn mcp_get_decision_outcome(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    args: serde_json::Map<String, serde_json::Value>,
    cache: &Arc<GraphCache>,
) -> McpToolResult {
    let core_args = GetDecisionOutcomeArgs::from_json(&args)?;
    let provider = HttpLedgerProvider {
        backend,
        tenant_id: &ctx.tenant_id,
        graph_cache: Some(cache),
    };
    let output = crate::mcp::core::get_decision_outcome(&provider, core_args)?;
    Ok(output.into_value())
}

fn mcp_get_relevant_decisions(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    args: serde_json::Map<String, serde_json::Value>,
    cache: &Arc<GraphCache>,
) -> McpToolResult {
    let core_args = GetRelevantDecisionsArgs::from_json(&args)?;
    let graph = mcp_open_graph(backend, ctx, cache)?;
    let output = crate::mcp::core::get_relevant_decisions(&*graph, core_args)?;
    Ok(output.into_value())
}

fn mcp_get_situational_decisions(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    args: serde_json::Map<String, serde_json::Value>,
    cache: &Arc<GraphCache>,
) -> McpToolResult {
    let core_args = GetSituationalDecisionsArgs::from_json(&args)?;
    let graph = mcp_open_graph(backend, ctx, cache)?;
    let provider = HttpLedgerProvider {
        backend,
        tenant_id: &ctx.tenant_id,
        graph_cache: None,
    };
    let output = crate::mcp::core::get_situational_decisions(&provider, &*graph, core_args)?;
    Ok(output.into_value())
}

fn mcp_get_waiting_requests(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    args: serde_json::Map<String, serde_json::Value>,
    cache: &Arc<GraphCache>,
) -> McpToolResult {
    let core_args = GetWaitingRequestsArgs::from_json(&args)?;
    let graph = mcp_open_graph(backend, ctx, cache)?;
    let output = crate::mcp::core::get_waiting_requests(&*graph, core_args)?;
    Ok(output.into_value())
}

fn mcp_get_contested_decisions(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    args: serde_json::Map<String, serde_json::Value>,
    cache: &Arc<GraphCache>,
) -> McpToolResult {
    let core_args = GetContestedDecisionsArgs::from_json(&args)?;
    let graph = mcp_open_graph(backend, ctx, cache)?;
    let output = crate::mcp::core::get_contested_decisions(&*graph, core_args)?;
    Ok(output.into_value())
}

fn mcp_get_open_why_decisions(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    args: serde_json::Map<String, serde_json::Value>,
    cache: &Arc<GraphCache>,
) -> McpToolResult {
    let core_args = GetOpenWhyDecisionsArgs::from_json(&args)?;
    let graph = mcp_open_graph(backend, ctx, cache)?;
    let output = crate::mcp::core::get_open_why_decisions(&*graph, core_args)?;
    Ok(output.into_value())
}

fn mcp_get_changed_decisions(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    args: serde_json::Map<String, serde_json::Value>,
    cache: &Arc<GraphCache>,
) -> McpToolResult {
    let core_args = GetChangedDecisionsArgs::from_json(&args)?;
    let graph = mcp_open_graph(backend, ctx, cache)?;
    let provider = HttpLedgerProvider {
        backend,
        tenant_id: &ctx.tenant_id,
        graph_cache: None,
    };
    let output = crate::mcp::core::get_changed_decisions(&provider, &*graph, core_args)?;
    Ok(output.into_value())
}

fn mcp_recall_decisions(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    args: serde_json::Map<String, serde_json::Value>,
    cache: &Arc<GraphCache>,
) -> McpToolResult {
    let core_args = RecallDecisionsArgs::from_json(&args)?;
    let graph = mcp_open_graph(backend, ctx, cache)?;
    let provider = HttpLedgerProvider {
        backend,
        tenant_id: &ctx.tenant_id,
        graph_cache: None,
    };
    let output = crate::mcp::core::recall_decisions(&provider, &*graph, core_args)?;
    Ok(output.into_value())
}

fn mcp_get_supersession_chain(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    args: serde_json::Map<String, serde_json::Value>,
    cache: &Arc<GraphCache>,
) -> McpToolResult {
    let core_args = GetSupersessionChainArgs::from_json(&args)?;
    let provider = HttpLedgerProvider {
        backend,
        tenant_id: &ctx.tenant_id,
        graph_cache: Some(cache),
    };
    let output = crate::mcp::core::get_supersession_chain(&provider, core_args)?;
    Ok(output.into_value())
}

fn mcp_get_decision_neighborhood(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    args: serde_json::Map<String, serde_json::Value>,
    cache: &Arc<GraphCache>,
) -> McpToolResult {
    let core_args = GetDecisionNeighborhoodArgs::from_json(&args)?;
    let provider = HttpLedgerProvider {
        backend,
        tenant_id: &ctx.tenant_id,
        graph_cache: Some(cache),
    };
    let output = crate::mcp::core::get_decision_neighborhood(&provider, core_args)?;
    Ok(output.into_value())
}

fn mcp_search_decisions(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    args: serde_json::Map<String, serde_json::Value>,
    cache: &Arc<GraphCache>,
) -> McpToolResult {
    let core_args = SearchDecisionsArgs::from_json(&args)?;
    let graph = mcp_open_graph(backend, ctx, cache)?;
    let provider = HttpLedgerProvider {
        backend,
        tenant_id: &ctx.tenant_id,
        graph_cache: None,
    };
    let output = crate::mcp::core::search_decisions(&provider, &*graph, core_args)?;
    Ok(output.into_value())
}

fn mcp_score_decision(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    args: serde_json::Map<String, serde_json::Value>,
    cache: &Arc<GraphCache>,
) -> McpToolResult {
    let core_args = ScoreDecisionArgs::from_json(&args)?;
    let graph = mcp_open_graph(backend, ctx, cache)?;
    let output = crate::mcp::core::score_decision(&*graph, core_args)?;
    Ok(output.into_value())
}

fn mcp_scan_decision_quality(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    args: serde_json::Map<String, serde_json::Value>,
    cache: &Arc<GraphCache>,
) -> McpToolResult {
    let core_args = ScanDecisionQualityArgs::from_json(&args)?;
    let graph = mcp_open_graph(backend, ctx, cache)?;
    let output = crate::mcp::core::scan_decision_quality(&*graph, core_args)?;
    Ok(output.into_value())
}

fn mcp_get_suggestions(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    args: serde_json::Map<String, serde_json::Value>,
    cache: &Arc<GraphCache>,
) -> McpToolResult {
    let core_args = GetSuggestionsArgs::from_json(&args)?;
    let graph = mcp_open_graph(backend, ctx, cache)?;
    let output = crate::mcp::core::get_suggestions(&*graph, core_args)?;
    Ok(output.into_value())
}

fn mcp_rank_decisions_by_importance(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    args: serde_json::Map<String, serde_json::Value>,
    cache: &Arc<GraphCache>,
) -> McpToolResult {
    let core_args = RankDecisionsByImportanceArgs::from_json(&args)?;
    let graph = mcp_open_graph(backend, ctx, cache)?;
    let output = crate::mcp::core::rank_decisions_by_importance(&*graph, core_args)?;
    Ok(output.into_value())
}

fn mcp_recent_decisions(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    args: serde_json::Map<String, serde_json::Value>,
    cache: &Arc<GraphCache>,
) -> McpToolResult {
    let core_args = RecentDecisionsArgs::from_json(&args)?;
    let provider = HttpLedgerProvider {
        backend,
        tenant_id: &ctx.tenant_id,
        graph_cache: None,
    };
    let graph = mcp_open_graph(backend, ctx, cache)?;
    let output = crate::mcp::core::recent_decisions(&provider, &*graph, core_args)?;
    Ok(output.into_value())
}

fn mcp_decision_quality_candidates(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    args: serde_json::Map<String, serde_json::Value>,
    cache: &Arc<GraphCache>,
) -> McpToolResult {
    let core_args = DecisionQualityCandidatesArgs::from_json(&args)?;
    let graph = mcp_open_graph(backend, ctx, cache)?;
    let output = crate::mcp::core::decision_quality_candidates(&*graph, core_args)?;
    Ok(output.into_value())
}

fn mcp_get_decision_context(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    args: serde_json::Map<String, serde_json::Value>,
    cache: &Arc<GraphCache>,
) -> McpToolResult {
    let core_args = GetDecisionContextArgs::from_json(&args)?;
    let graph = mcp_open_graph(backend, ctx, cache)?;
    let output = crate::mcp::core::get_decision_context(&*graph, core_args)?;
    Ok(output.into_value())
}

fn mcp_decision_context_candidates(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    args: serde_json::Map<String, serde_json::Value>,
    cache: &Arc<GraphCache>,
) -> McpToolResult {
    let core_args = DecisionContextCandidatesArgs::from_json(&args)?;
    let graph = mcp_open_graph(backend, ctx, cache)?;
    let output = crate::mcp::core::decision_context_candidates(&*graph, core_args)?;
    Ok(output.into_value())
}

fn mcp_scan_misfiled_decisions(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    args: serde_json::Map<String, serde_json::Value>,
    cache: &Arc<GraphCache>,
) -> McpToolResult {
    let core_args = ScanMisfiledDecisionsArgs::from_json(&args)?;
    let provider = HttpLedgerProvider {
        backend,
        tenant_id: &ctx.tenant_id,
        graph_cache: None,
    };
    let graph = mcp_open_graph(backend, ctx, cache)?;
    let output = crate::mcp::core::scan_misfiled_decisions(&provider, &*graph, core_args)?;
    Ok(output.into_value())
}

fn mcp_analyze_failure_modes(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    args: serde_json::Map<String, serde_json::Value>,
    cache: &Arc<GraphCache>,
) -> McpToolResult {
    let core_args = AnalyzeFailureModesArgs::from_json(&args)?;
    let graph = mcp_open_graph(backend, ctx, cache)?;
    let output = crate::mcp::core::analyze_failure_modes(&*graph, core_args)?;
    Ok(output.into_value())
}

fn mcp_dump_graph(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    cache: &Arc<GraphCache>,
) -> McpToolResult {
    let graph = mcp_open_graph(backend, ctx, cache)?;
    let output = crate::mcp::core::dump_graph(&*graph)?;
    Ok(output.into_value())
}

fn mcp_compact_view(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    args: serde_json::Map<String, serde_json::Value>,
    cache: &Arc<GraphCache>,
) -> McpToolResult {
    let core_args = CompactViewArgs::from_json(&args)?;
    let provider = HttpLedgerProvider {
        backend,
        tenant_id: &ctx.tenant_id,
        graph_cache: Some(cache),
    };
    let output = crate::mcp::core::compact_view(&provider, core_args)?;
    Ok(output.into_value())
}

fn mcp_summarize(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    args: serde_json::Map<String, serde_json::Value>,
    cache: &Arc<GraphCache>,
) -> McpToolResult {
    let core_args = SummarizeDecisionsArgs::from_json(&args)?;
    let graph = mcp_open_graph(backend, ctx, cache)?;
    let output = crate::mcp::core::summarize_decisions(&*graph, core_args)?;
    Ok(output.into_value())
}

fn mcp_classify_queue_list(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    args: serde_json::Map<String, serde_json::Value>,
) -> McpToolResult {
    let core_args = ClassifyQueueListArgs::from_json(&args)?;
    let provider = HttpLedgerProvider {
        backend,
        tenant_id: &ctx.tenant_id,
        graph_cache: None,
    };
    let output = crate::mcp::core::classify_queue_list(&provider, core_args)?;
    Ok(output.into_value())
}

/// Recorded as the token's own actor, whatever `actor_id` or session the call carries: a
/// classification is the worker's, not a session's.
fn mcp_classify_queue_submit(
    backend: &ApiBackend,
    ctx: &ApiRequestCtx,
    args: serde_json::Map<String, serde_json::Value>,
) -> McpToolResult {
    let core_args = ClassifyQueueSubmitArgs::from_json(&args, ctx.actor_id.clone())?;
    let provider = HttpLedgerProvider {
        backend,
        tenant_id: &ctx.tenant_id,
        graph_cache: None,
    };
    let output = crate::mcp::core::classify_queue_submit(&provider, core_args)?;
    Ok(output.into_value())
}
