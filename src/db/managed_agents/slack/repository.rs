use sqlx::PgPool;

use crate::errors::GatewayError;

use super::schema::SlackThreadSessionRow;

pub async fn list(
    pool: &PgPool,
    agent_id: &str,
) -> Result<Vec<SlackThreadSessionRow>, GatewayError> {
    sqlx::query_as::<_, SlackThreadSessionRow>(
        r#"
        SELECT *
        FROM "LiteLLM_ManagedAgentSlackThreadSessionsTable"
        WHERE agent_id = $1
        ORDER BY updated_at DESC
        "#,
    )
    .bind(agent_id)
    .fetch_all(pool)
    .await
    .map_err(GatewayError::Database)
}
