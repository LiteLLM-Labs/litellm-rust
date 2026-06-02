use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::HeaderMap,
    Json,
};

use crate::{db::managed_agents::inbox::repository, errors::GatewayError, proxy::state::AppState};

use super::types::{AcceptRequest, ApprovalsResponse, DecisionResponse, RejectRequest};

pub async fn list_pending(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<ApprovalsResponse>, GatewayError> {
    let pool = super::super::db(&state, &headers)?;
    Ok(Json(ApprovalsResponse {
        approvals: repository::pending_approvals(pool).await?,
    }))
}

pub async fn accept(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(item_id): Path<String>,
    Json(input): Json<AcceptRequest>,
) -> Result<Json<DecisionResponse>, GatewayError> {
    let pool = super::super::db(&state, &headers)?;
    let live = repository::decide_approval(pool, &item_id, "accept", None, input.arguments).await?;
    Ok(Json(DecisionResponse { ok: true, live }))
}

pub async fn reject(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(item_id): Path<String>,
    Json(input): Json<RejectRequest>,
) -> Result<Json<DecisionResponse>, GatewayError> {
    let pool = super::super::db(&state, &headers)?;
    let live = repository::decide_approval(pool, &item_id, "reject", input.feedback, None).await?;
    Ok(Json(DecisionResponse { ok: true, live }))
}
