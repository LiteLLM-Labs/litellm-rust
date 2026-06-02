use axum::http::HeaderMap;

use crate::errors::GatewayError;

pub fn require_master_key(
    headers: &HeaderMap,
    configured: Option<&str>,
) -> Result<(), GatewayError> {
    let Some(master_key) = configured else {
        return Ok(());
    };

    let expected = format!("Bearer {master_key}");
    let actual = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());

    if actual == Some(expected.as_str()) {
        Ok(())
    } else {
        Err(GatewayError::Unauthorized)
    }
}
