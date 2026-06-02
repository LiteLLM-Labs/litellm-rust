use serde::Serialize;
use sqlx::FromRow;

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct SlackThreadSessionRow {
    pub agent_id: String,
    pub channel_id: String,
    pub thread_ts: String,
    pub session_id: String,
    pub created_at: i64,
    pub updated_at: i64,
}
