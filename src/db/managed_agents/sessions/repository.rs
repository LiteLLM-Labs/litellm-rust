use sqlx::PgPool;

use crate::errors::GatewayError;

use super::schema::SessionRow;

pub async fn get(pool: &PgPool, session_id: &str) -> Result<Option<SessionRow>, GatewayError> {
    sqlx::query_as::<_, SessionRow>(
        r#"SELECT * FROM "LiteLLM_ManagedAgentSessionsTable" WHERE id = $1"#,
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await
    .map_err(GatewayError::Database)
}
