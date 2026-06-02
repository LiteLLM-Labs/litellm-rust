use std::{env, path::PathBuf, sync::Arc};

use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Redirect},
    Json,
};
use serde_json::json;
use tower_http::services::{ServeDir, ServeFile};

use crate::proxy::state::AppState;

pub fn static_files() -> ServeDir<ServeFile> {
    let dir = ui_dir();
    ServeDir::new(&dir)
        .append_index_html_on_directories(true)
        .fallback(ServeFile::new(dir.join("404.html")))
}

pub async fn redirect_to_sessions() -> Redirect {
    Redirect::temporary("/sessions/")
}

pub async fn whoami(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> Result<impl IntoResponse, crate::errors::GatewayError> {
    crate::proxy::auth::master_key::require_master_key(
        &headers,
        state.config.general_settings.master_key.as_deref(),
    )?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn litellm_health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(json!({
        "ok": true,
        "modelCount": state.config.model_list.len(),
        "status": 200,
        "base": "/",
        "modelsUrl": "/v1/models"
    }))
}

pub async fn models(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(json!({
        "object": "list",
        "data": state.config.model_list.iter().map(|entry| {
            json!({
                "id": entry.model_name,
                "object": "model",
                "owned_by": "litellm-rust"
            })
        }).collect::<Vec<_>>()
    }))
}

pub async fn sessions() -> impl IntoResponse {
    Json(json!([]))
}

pub async fn create_session() -> impl IntoResponse {
    Json(session_json("gateway"))
}

pub async fn session(Path(id): Path<String>) -> impl IntoResponse {
    Json(session_json(&id))
}

pub async fn session_messages() -> impl IntoResponse {
    Json(json!([]))
}

pub async fn prompt_async() -> impl IntoResponse {
    StatusCode::NO_CONTENT
}

pub async fn delete_session() -> impl IntoResponse {
    StatusCode::NO_CONTENT
}

pub async fn abort_session() -> impl IntoResponse {
    StatusCode::NO_CONTENT
}

pub async fn agents() -> impl IntoResponse {
    Json(json!({ "agents": [] }))
}

pub async fn approvals() -> impl IntoResponse {
    Json(json!({ "approvals": [] }))
}

pub async fn inbox() -> impl IntoResponse {
    Json(json!({ "items": [] }))
}

pub async fn skills() -> impl IntoResponse {
    Json(json!({ "skills": [] }))
}

pub async fn vault() -> impl IntoResponse {
    Json(json!({ "keys": [] }))
}

pub async fn events() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/event-stream; charset=utf-8")],
        "event: ready\ndata: {}\n\n",
    )
}

fn ui_dir() -> PathBuf {
    env::var_os("LITELLM_UI_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("src/ui/out"))
}

fn session_json(id: &str) -> serde_json::Value {
    json!({
        "id": id,
        "title": "LiteLLM Gateway",
        "time": { "created": 0 },
        "harness": "litellm"
    })
}
