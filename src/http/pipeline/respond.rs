//! Small response/error helpers shared by the dispatch paths.

use axum::{http::HeaderMap, response::Response};
use serde_json::{json, Value};

use crate::{
    errors::GatewayError,
    http::llm,
    sdk::{
        codec::{
            ir::{ChatResponse, StopReason, Usage},
            ProtocolCodec, RequestCtx,
        },
        router::Deployment,
    },
};

/// Rewrite the body's `model` to the upstream name on the same-protocol path.
/// Gemini carries the model in the URL, so its body has none to rewrite.
pub(super) fn rewrite_model(body: &mut Value, deployment: &Deployment) {
    use crate::sdk::codec::WireFormat;
    if deployment.wire != WireFormat::Gemini
        && body.get("model").and_then(Value::as_str) != Some(deployment.upstream_model.as_str())
    {
        body["model"] = json!(deployment.upstream_model);
    }
}

/// True when a Responses JSON body reports `status: "failed"` despite HTTP 2xx.
pub(super) fn is_failed_responses(bytes: &[u8]) -> bool {
    serde_json::from_slice::<Value>(bytes)
        .ok()
        .as_ref()
        .and_then(|v| v.get("status"))
        .and_then(Value::as_str)
        == Some("failed")
}

/// True when a 2xx JSON body carries a top-level non-null `error` object
/// (OpenAI/Anthropic report a failure this way without a non-2xx status).
/// Successful Responses objects carry `error: null`, which must not match.
pub(super) fn has_error_object(bytes: &[u8]) -> bool {
    matches!(
        serde_json::from_slice::<Value>(bytes),
        Ok(Value::Object(ref o)) if o.get("error").is_some_and(|e| !e.is_null())
    )
}

/// Render a translated upstream error in the inbound protocol's error shape.
pub(super) fn translated_error(
    in_codec: &dyn ProtocolCodec,
    ctx: &RequestCtx,
    status: reqwest::StatusCode,
    resp_headers: HeaderMap,
    bytes: &[u8],
) -> Result<Response, GatewayError> {
    let ir = ChatResponse {
        id: String::new(),
        model: ctx.model.clone(),
        content: Vec::new(),
        stop_reason: Some(StopReason::Other(format!(
            "error: {}",
            error_message(bytes)
        ))),
        usage: Usage::default(),
    };
    let client = in_codec.render_response(&ir, ctx)?;
    Ok(llm::build_bytes_response(
        status,
        resp_headers,
        serde_json::to_vec(&client)?,
    ))
}

/// Message from a top-level `error` (object `message` or bare string).
fn error_message(bytes: &[u8]) -> String {
    let value: Value = serde_json::from_slice(bytes).unwrap_or(Value::Null);
    let err = value.get("error");
    err.and_then(|e| e.get("message"))
        .and_then(Value::as_str)
        .or_else(|| err.and_then(Value::as_str))
        .unwrap_or("upstream error")
        .to_owned()
}
