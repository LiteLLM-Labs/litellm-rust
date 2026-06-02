use serde::Serialize;

use crate::db::managed_agents::runs::schema::AgentRunRow;

#[derive(Debug, serde::Deserialize)]
pub struct ListRunsQuery {
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct RunCreateResponse {
    pub run_id: String,
    pub agent_id: String,
    pub session_id: String,
    pub status: String,
    pub event_url: &'static str,
}

#[derive(Debug, Serialize)]
pub struct RunsResponse {
    pub runs: Vec<AgentRunRow>,
}
