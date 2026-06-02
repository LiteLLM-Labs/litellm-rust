use serde_json::json;
use sqlx::PgPool;

use crate::{
    db::managed_agents::{id, now_ms},
    errors::GatewayError,
};

use super::schema::{AgentRunRow, CreateRun};

pub async fn create(
    pool: &PgPool,
    agent_id: &str,
    session_id: String,
    input: CreateRun,
) -> Result<AgentRunRow, GatewayError> {
    sqlx::query_as::<_, AgentRunRow>(
        r#"
        INSERT INTO "LiteLLM_ManagedAgentRunsTable"
          (id, agent_id, session_id, status, started_at, config_overrides)
        VALUES ($1, $2, $3, 'starting', $4, $5)
        RETURNING *
        "#,
    )
    .bind(id("run"))
    .bind(agent_id)
    .bind(input.session_id.unwrap_or(session_id))
    .bind(now_ms())
    .bind(input.config_overrides.unwrap_or_else(|| json!({})))
    .fetch_one(pool)
    .await
    .map_err(GatewayError::Database)
}

pub async fn list(
    pool: &PgPool,
    agent_id: &str,
    limit: i64,
) -> Result<Vec<AgentRunRow>, GatewayError> {
    sqlx::query_as::<_, AgentRunRow>(
        r#"
        SELECT *
        FROM "LiteLLM_ManagedAgentRunsTable"
        WHERE agent_id = $1
        ORDER BY started_at DESC
        LIMIT $2
        "#,
    )
    .bind(agent_id)
    .bind(limit.clamp(1, 100))
    .fetch_all(pool)
    .await
    .map_err(GatewayError::Database)
}

pub async fn get(
    pool: &PgPool,
    agent_id: &str,
    run_id: &str,
) -> Result<Option<AgentRunRow>, GatewayError> {
    sqlx::query_as::<_, AgentRunRow>(
        r#"
        SELECT *
        FROM "LiteLLM_ManagedAgentRunsTable"
        WHERE agent_id = $1 AND id = $2
        "#,
    )
    .bind(agent_id)
    .bind(run_id)
    .fetch_optional(pool)
    .await
    .map_err(GatewayError::Database)
}
