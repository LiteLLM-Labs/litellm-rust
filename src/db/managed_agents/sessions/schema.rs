use serde::Serialize;
use sqlx::FromRow;

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct SessionRow {
    pub id: String,
    pub harness: String,
    pub agent_id: Option<String>,
    pub title: String,
    pub created_at: i64,
    pub updated_at: Option<i64>,
    pub sdk_session_id: Option<String>,
    pub tz: Option<String>,
}
