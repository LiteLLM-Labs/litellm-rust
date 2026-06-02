use sqlx::PgPool;

use crate::errors::GatewayError;

use super::schema::SessionMessageRow;

pub async fn list(pool: &PgPool, session_id: &str) -> Result<Vec<SessionMessageRow>, GatewayError> {
    sqlx::query_as::<_, SessionMessageRow>(
        r#"
        SELECT *
        FROM "LiteLLM_ManagedAgentSessionMessagesTable"
        WHERE session_id = $1
        ORDER BY seq ASC
        "#,
    )
    .bind(session_id)
    .fetch_all(pool)
    .await
    .map_err(GatewayError::Database)
}
