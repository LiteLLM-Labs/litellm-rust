use serde_json::Value;
use sqlx::{FromRow, PgPool};

use crate::errors::GatewayError;

#[derive(Debug, Clone, FromRow)]
pub struct CredentialRow {
    pub credential_values: Value,
}

pub async fn get_by_name(
    pool: &PgPool,
    credential_name: &str,
) -> Result<Option<CredentialRow>, GatewayError> {
    sqlx::query_as::<_, CredentialRow>(
        r#"
        SELECT credential_values
        FROM "LiteLLM_CredentialsTable"
        WHERE credential_name = $1
        "#,
    )
    .bind(credential_name)
    .fetch_optional(pool)
    .await
    .map_err(GatewayError::Database)
}

pub async fn upsert(
    pool: &PgPool,
    credential_name: &str,
    credential_values: Value,
    credential_info: Value,
    actor: &str,
) -> Result<(), GatewayError> {
    sqlx::query(
        r#"
        INSERT INTO "LiteLLM_CredentialsTable" (
            credential_id,
            credential_name,
            credential_values,
            credential_info,
            created_by,
            updated_by
        )
        VALUES ($1, $2, $3, $4, $5, $5)
        ON CONFLICT (credential_name) DO UPDATE SET
            credential_values = EXCLUDED.credential_values,
            credential_info = EXCLUDED.credential_info,
            updated_at = CURRENT_TIMESTAMP,
            updated_by = EXCLUDED.updated_by
        "#,
    )
    .bind(format!("cred_{}", uuid::Uuid::new_v4().simple()))
    .bind(credential_name)
    .bind(credential_values)
    .bind(credential_info)
    .bind(actor)
    .execute(pool)
    .await
    .map_err(GatewayError::Database)?;
    Ok(())
}

pub async fn delete_by_name(pool: &PgPool, credential_name: &str) -> Result<bool, GatewayError> {
    let result = sqlx::query(
        r#"
        DELETE FROM "LiteLLM_CredentialsTable"
        WHERE credential_name = $1
        "#,
    )
    .bind(credential_name)
    .execute(pool)
    .await
    .map_err(GatewayError::Database)?;
    Ok(result.rows_affected() > 0)
}
