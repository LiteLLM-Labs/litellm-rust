pub mod files;
pub mod inbox;
pub mod loops;
pub mod mcp_credentials;
pub mod memory;
pub mod messages;
pub mod pool;
pub mod registry;
pub mod runs;
pub mod saved;
pub mod sessions;
pub mod skills;
pub mod slack;
pub mod users;

pub fn id(prefix: &str) -> String {
    format!("{prefix}_{}", uuid::Uuid::new_v4().simple())
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}
