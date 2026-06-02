//! Proxy-server concerns: config, request auth, shared state. The translation
//! layer (`providers/`) must not depend on anything here so it can ship as an
//! SDK on its own.

pub mod auth;
pub mod config;
mod mcp_config;
pub mod state;
