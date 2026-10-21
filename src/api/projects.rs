//! `GET /v1/projects` — the projects a tenant holds, each with its decision counts by status —
//! and [`named_project`], how `GET /v1/graph?project=` and `GET /v1/decisions/search?project=`
//! read the project a caller names. Read only: registering, renaming and moving a project stay
//! with the CLI.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::commands::PERSONAL_PROJECT_HANDLE_PREFIX;
use crate::projector::GraphView;
use crate::queries::{project_address, project_overview, ProjectListRequest};

use super::auth::extract_ctx;
use super::graph::get_cached_graph;
use super::{respond_envelope, to_api_error, ApiError, ApiResult, AppState};

#[derive(Debug, Deserialize)]
pub(super) struct ProjectsParams {
    limit: Option<usize>,
    /// The `next_cursor` of the previous page.
    cursor: Option<String>,
}

/// `GET /v1/projects[?limit=&cursor=]`: every project with a registration or a decision in it,
/// by handle, paged (`truncated` and `data.next_cursor` continue), with its name, its parent
/// (`PART_OF`) and its decision counts by status. A personal project appears once a decision
/// is filed in it.
pub(super) async fn projects_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<ProjectsParams>,
) -> Response {
    let ctx = match extract_ctx(&state, &headers).await {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };

    let backend = Arc::clone(&state.backend);
    let cache = Arc::clone(&state.graph_cache);
    let result = tokio::task::spawn_blocking(move || -> ApiResult<_> {
        if let Some(cursor) = params.cursor.as_deref() {
            cursor.parse::<usize>().map_err(|error| {
                ApiError::validation(format!("cursor must be a non-negative offset: {error}"))
            })?;
        }
        let ledger = backend.open_ledger_for_tenant(&ctx.tenant_id)?;
        let graph = get_cached_graph(&ledger, &ctx.tenant_id, &cache)?;
        project_overview(
            &ledger,
            graph.as_ref(),
            &ProjectListRequest {
                limit: params.limit.unwrap_or(0),
                cursor: params.cursor,
            },
        )
        .map_err(to_api_error)
    })
    .await;

    respond_envelope(result)
}

/// The address the decisions of the project a caller named are filed under. An empty name, or a
/// personal address that names no actor, is refused as a bad request; a shared project nobody
/// registered is not found, so a typo never reads as an empty project.
pub(super) fn named_project(graph: &impl GraphView, requested: &str) -> ApiResult<String> {
    let requested = requested.trim();
    if requested.is_empty() {
        return Err(ApiError::validation("`project` must not be empty"));
    }
    if requested
        .strip_prefix(PERSONAL_PROJECT_HANDLE_PREFIX)
        .is_some_and(|actor| actor.trim().is_empty())
    {
        return Err(ApiError::validation(
            "a personal project address must name an actor, like personal:agent:claude",
        ));
    }
    project_address(graph, requested)
        .map_err(to_api_error)?
        .ok_or_else(|| ApiError::not_found(format!("project not registered: {requested}")))
}
