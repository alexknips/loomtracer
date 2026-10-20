//! Linking two decisions that are already recorded as superseded-by over HTTP with the caller's
//! token (hivemind-4p0xd), so a person's own answer can replace an earlier record without
//! being recorded again under the caller's name.
//!
//! `POST /v1/decisions/{id}/supersession-links` records that `{id}` (the newer decision)
//! supersedes the body's `supersedes_id` (the older one), as `decision.superseded`, attributed
//! to the bearer token's actor. It creates nothing: both decisions must already be recorded, and
//! the newer one keeps whoever decided it. `POST /v1/decisions/{id}/supersessions` is the other
//! way round and the other thing: there `{id}` is the OLD decision and the call records a new
//! replacement, decided by the caller.
//!
//! The rules are `crate::supersession_link`'s, shared with the `link_supersession` MCP tool: an
//! id that is not a recorded decision, the same id twice and a link that would close a loop are
//! refused with 400 and nothing written; a pair already linked writes nothing and replies
//! `linked: false`.

use std::sync::Arc;

use axum::extract::rejection::JsonRejection;
use axum::extract::{Json, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::events::EventProvenance;
use crate::supersession_link::{link_supersession, SupersessionLinkCall, SupersessionLinkOutcome};

use super::auth::extract_ctx;
use super::graph::open_graph_from_ledger;
use super::{respond, to_api_error, ApiError, ApiResult, AppState};

/// The body of `POST /v1/decisions/{id}/supersession-links`: the one decision `{id}` supersedes.
/// Any other field is refused, so a client cannot mistake this for the route that records a new
/// replacement.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LinkSupersessionRequest {
    /// The decision recorded earlier that `{id}` supersedes.
    supersedes_id: String,
}

/// `POST /v1/decisions/{id}/supersession-links`: records that `{id}` supersedes `supersedes_id`,
/// as `decision.superseded`, attributed to the caller. Replies with the event id, or `null` and
/// `linked: false` when `{id}` already superseded it.
pub(super) async fn link_supersession_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(decision_id): Path<String>,
    payload: std::result::Result<Json<LinkSupersessionRequest>, JsonRejection>,
) -> Response {
    let ctx = match extract_ctx(&state, &headers).await {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };
    let req = match payload {
        Ok(Json(r)) => r,
        Err(e) => return ApiError::validation(e.to_string()).into_response(),
    };

    let backend = Arc::clone(&state.backend);
    let cache = Arc::clone(&state.graph_cache);
    let result = tokio::task::spawn_blocking(move || -> ApiResult<_> {
        let ledger = backend.open_ledger_for_tenant(&ctx.tenant_id)?;
        let graph = open_graph_from_ledger(&ledger, &ctx.tenant_id, &cache)?;
        let outcome = link_supersession(
            &ledger,
            &ctx.tenant_id,
            EventProvenance::api(Some(ctx.actor_id.clone())),
            &*graph,
            SupersessionLinkCall {
                actor_id: &ctx.actor_id,
                old_decision_id: req.supersedes_id.trim(),
                new_decision_id: decision_id.trim(),
            },
        )
        .map_err(to_api_error)?;
        match outcome {
            SupersessionLinkOutcome::Linked(reply) => {
                serde_json::to_value(reply).map_err(|error| ApiError::internal(error.to_string()))
            }
            SupersessionLinkOutcome::Refused(reason) => Err(ApiError::validation(reason)),
        }
    })
    .await;

    respond(result, StatusCode::OK)
}
