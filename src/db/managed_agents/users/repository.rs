use sqlx::{PgConnection, PgPool};

use crate::{
    db::managed_agents::{id, now_ms},
    errors::GatewayError,
};

use super::schema::{GenerateKeyResponse, NewUserRequest, NewUserResponse};

/// Create a user, optionally minting an API key in the same transaction.
pub async fn create_user(
    pool: &PgPool,
    input: NewUserRequest,
) -> Result<NewUserResponse, GatewayError> {
    let user_id = input
        .user_id
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| id("user"));

    let mut tx = pool.begin().await.map_err(GatewayError::Database)?;
    insert_user(tx.as_mut(), &user_id, input.user_alias.as_deref()).await?;

    let key = if input.auto_create_key {
        Some(insert_key(tx.as_mut(), &user_id, input.key_alias.as_deref()).await?)
    } else {
        None
    };

    tx.commit().await.map_err(GatewayError::Database)?;
    Ok(NewUserResponse { user_id, key })
}

/// Mint a new API key for an existing (or newly created) user.
pub async fn generate_key(
    pool: &PgPool,
    user_id: Option<String>,
    key_alias: Option<String>,
) -> Result<GenerateKeyResponse, GatewayError> {
    let user_id = user_id
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| id("user"));

    let mut tx = pool.begin().await.map_err(GatewayError::Database)?;
    // Idempotent: create the user row if the caller passed an unknown id.
    insert_user_if_absent(tx.as_mut(), &user_id).await?;
    let key = insert_key(tx.as_mut(), &user_id, key_alias.as_deref()).await?;
    tx.commit().await.map_err(GatewayError::Database)?;

    Ok(GenerateKeyResponse { user_id, key })
}

/// Resolve a presented API key to its owning `user_id`, if any.
pub async fn resolve_user(pool: &PgPool, token: &str) -> Result<Option<String>, GatewayError> {
    if token.is_empty() {
        return Ok(None);
    }
    let row: Option<(String,)> =
        sqlx::query_as(r#"SELECT user_id FROM "LiteLLM_VerificationTokenTable" WHERE token = $1"#)
            .bind(token)
            .fetch_optional(pool)
            .await
            .map_err(GatewayError::Database)?;
    Ok(row.map(|(user_id,)| user_id))
}

async fn insert_user(
    conn: &mut PgConnection,
    user_id: &str,
    user_alias: Option<&str>,
) -> Result<(), GatewayError> {
    sqlx::query(
        r#"INSERT INTO "LiteLLM_UserTable" (user_id, user_alias, created_at) VALUES ($1, $2, $3)"#,
    )
    .bind(user_id)
    .bind(user_alias)
    .bind(now_ms())
    .execute(conn)
    .await
    .map_err(GatewayError::Database)?;
    Ok(())
}

async fn insert_user_if_absent(conn: &mut PgConnection, user_id: &str) -> Result<(), GatewayError> {
    sqlx::query(
        r#"INSERT INTO "LiteLLM_UserTable" (user_id, created_at)
           VALUES ($1, $2) ON CONFLICT (user_id) DO NOTHING"#,
    )
    .bind(user_id)
    .bind(now_ms())
    .execute(conn)
    .await
    .map_err(GatewayError::Database)?;
    Ok(())
}

async fn insert_key(
    conn: &mut PgConnection,
    user_id: &str,
    key_alias: Option<&str>,
) -> Result<String, GatewayError> {
    let token = format!("sk-{}", uuid::Uuid::new_v4().simple());
    sqlx::query(
        r#"INSERT INTO "LiteLLM_VerificationTokenTable" (token, user_id, key_alias, created_at)
           VALUES ($1, $2, $3, $4)"#,
    )
    .bind(&token)
    .bind(user_id)
    .bind(key_alias)
    .bind(now_ms())
    .execute(conn)
    .await
    .map_err(GatewayError::Database)?;
    Ok(token)
}
