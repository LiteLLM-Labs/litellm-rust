use std::sync::Arc;

use axum::{
    extract::State,
    response::{Html, IntoResponse, Redirect},
    Json,
};
use serde_json::{json, Value};

use crate::proxy::state::AppState;

pub async fn swagger_ui() -> Html<&'static str> {
    Html(include_str!("swagger.html"))
}

pub async fn openapi_json(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let models = configured_models(&state);
    let model_enum_desc = model_enum_description(&models);

    Json(openapi_spec(&models, &model_enum_desc))
}

fn configured_models(state: &AppState) -> Vec<Value> {
    state
        .config
        .model_list
        .iter()
        .map(|m| json!(m.model_name))
        .collect()
}

fn model_enum_description(models: &[Value]) -> String {
    models
        .iter()
        .filter_map(|v| v.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn openapi_spec(models: &[Value], model_enum_desc: &str) -> Value {
    json!({
        "openapi": "3.0.3",
        "info": {
            "title": "LiteLLM API",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Low-overhead LiteLLM-compatible gateway"
        },
        "paths": {
            "/health": health_path(),
            "/v1/messages": messages_path(models, model_enum_desc)
        },
        "components": components()
    })
}

fn health_path() -> Value {
    json!({
        "get": {
            "summary": "Health check",
            "operationId": "health",
            "tags": ["System"],
            "responses": {
                "200": {
                    "description": "Server is healthy",
                    "content": {
                        "application/json": {
                            "schema": {
                                "type": "object",
                                "properties": {
                                    "status": { "type": "string", "example": "ok" }
                                }
                            }
                        }
                    }
                }
            }
        }
    })
}

fn messages_path(models: &[Value], model_enum_desc: &str) -> Value {
    json!({
        "post": {
            "summary": "Create a message (Anthropic-compatible)",
            "operationId": "createMessage",
            "tags": ["Messages"],
            "security": [{ "BearerAuth": [] }],
            "requestBody": {
                "required": true,
                "content": {
                    "application/json": {
                        "schema": messages_schema(models, model_enum_desc)
                    }
                }
            },
            "responses": {
                "200": { "description": "Message response from upstream provider" },
                "401": { "description": "Invalid or missing master key" },
                "404": { "description": "Model not found in config" }
            }
        }
    })
}

fn messages_schema(models: &[Value], model_enum_desc: &str) -> Value {
    json!({
        "type": "object",
        "required": ["model", "messages", "max_tokens"],
        "properties": {
            "model": {
                "type": "string",
                "description": format!("Model alias from config. Available: {}", model_enum_desc),
                "example": models.first().and_then(|v| v.as_str()).unwrap_or("claude-sonnet")
            },
            "messages": {
                "type": "array",
                "items": {
                    "type": "object",
                    "required": ["role", "content"],
                    "properties": {
                        "role": { "type": "string", "enum": ["user", "assistant"] },
                        "content": { "type": "string" }
                    }
                },
                "example": [{ "role": "user", "content": "Hello!" }]
            },
            "max_tokens": { "type": "integer", "example": 1024 },
            "stream": { "type": "boolean", "example": false }
        }
    })
}

fn components() -> Value {
    json!({
        "securitySchemes": {
            "BearerAuth": {
                "type": "http",
                "scheme": "bearer",
                "description": "Your LITELLM_MASTER_KEY"
            }
        }
    })
}

pub async fn redirect_to_docs() -> Redirect {
    Redirect::permanent("/docs")
}
