pub mod files;
pub mod inbox;
pub mod memory;
pub mod registry;
pub mod routes;
pub mod runs;
pub mod skills;

use axum::http::HeaderMap;
use sqlx::PgPool;

use crate::{
    errors::GatewayError,
    proxy::{auth::master_key::require_master_key, state::AppState},
};

pub fn db<'a>(state: &'a AppState, headers: &HeaderMap) -> Result<&'a PgPool, GatewayError> {
    require_master_key(headers, state.config.general_settings.master_key.as_deref())?;

    state.db.as_ref().ok_or(GatewayError::MissingDatabase)
}
