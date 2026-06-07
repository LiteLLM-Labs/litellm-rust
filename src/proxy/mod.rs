//! Proxy-server concerns: config, request auth, shared state. The translation
//! layer (`providers/`) must not depend on anything here so it can ship as an
//! SDK on its own.

pub mod auth;
pub mod cache;
pub mod config;
pub mod credential_crypto;
mod mcp_config;
pub mod provider_credentials;
pub mod state;
