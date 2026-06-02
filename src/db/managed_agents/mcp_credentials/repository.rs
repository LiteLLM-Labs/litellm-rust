use sqlx::PgPool;

use crate::{db::managed_agents::now_ms, errors::GatewayError};

use super::{
    crypto,
    schema::{McpUserCredentialStatus, ResolvedCredential},
};

const TYPE_STATIC: &str = "static";
const TYPE_OAUTH: &str = "oauth";

/// Row shape for credential resolution: `(credential_type, credential_enc, access_token_enc)`.
type SecretRow = (String, Option<Vec<u8>>, Option<Vec<u8>>);

/// Store (or replace) a pasted static/BYOK token for a (user, server).
pub async fn upsert_static(
    pool: &PgPool,
    enc_key: &[u8; 32],
    user_id: &str,
    server_id: &str,
    credential: &str,
) -> Result<(), GatewayError> {
    let credential_enc = crypto::encrypt(enc_key, credential)?;
    let now = now_ms();
    sqlx::query(
        r#"
        INSERT INTO "LiteLLM_MCPUserCredentialTable"
            (user_id, server_id, credential_type, credential_enc,
             access_token_enc, refresh_token_enc, expires_at, scopes, created_at, updated_at)
        VALUES ($1, $2, $3, $4, NULL, NULL, NULL, NULL, $5, $5)
        ON CONFLICT (user_id, server_id) DO UPDATE SET
            credential_type   = EXCLUDED.credential_type,
            credential_enc    = EXCLUDED.credential_enc,
            access_token_enc  = NULL,
            refresh_token_enc = NULL,
            expires_at        = NULL,
            scopes            = NULL,
            updated_at        = EXCLUDED.updated_at
        "#,
    )
    .bind(user_id)
    .bind(server_id)
    .bind(TYPE_STATIC)
    .bind(credential_enc)
    .bind(now)
    .execute(pool)
    .await
    .map_err(GatewayError::Database)?;
    Ok(())
}

/// Store (or replace) an OAuth2 token set for a (user, server).
#[allow(clippy::too_many_arguments)]
pub async fn upsert_oauth(
    pool: &PgPool,
    enc_key: &[u8; 32],
    user_id: &str,
    server_id: &str,
    access_token: &str,
    refresh_token: Option<&str>,
    expires_at: Option<i64>,
    scopes: Option<&str>,
) -> Result<(), GatewayError> {
    let access_enc = crypto::encrypt(enc_key, access_token)?;
    let refresh_enc = match refresh_token {
        Some(value) => Some(crypto::encrypt(enc_key, value)?),
        None => None,
    };
    let now = now_ms();
    sqlx::query(
        r#"
        INSERT INTO "LiteLLM_MCPUserCredentialTable"
            (user_id, server_id, credential_type, credential_enc,
             access_token_enc, refresh_token_enc, expires_at, scopes, created_at, updated_at)
        VALUES ($1, $2, $3, NULL, $4, $5, $6, $7, $8, $8)
        ON CONFLICT (user_id, server_id) DO UPDATE SET
            credential_type   = EXCLUDED.credential_type,
            credential_enc    = NULL,
            access_token_enc  = EXCLUDED.access_token_enc,
            refresh_token_enc = EXCLUDED.refresh_token_enc,
            expires_at        = EXCLUDED.expires_at,
            scopes            = EXCLUDED.scopes,
            updated_at        = EXCLUDED.updated_at
        "#,
    )
    .bind(user_id)
    .bind(server_id)
    .bind(TYPE_OAUTH)
    .bind(access_enc)
    .bind(refresh_enc)
    .bind(expires_at)
    .bind(scopes)
    .bind(now)
    .execute(pool)
    .await
    .map_err(GatewayError::Database)?;
    Ok(())
}

/// Resolve and decrypt the token to inject for a (user, server), if stored.
pub async fn resolve(
    pool: &PgPool,
    enc_key: &[u8; 32],
    user_id: &str,
    server_id: &str,
) -> Result<Option<ResolvedCredential>, GatewayError> {
    let row: Option<SecretRow> = sqlx::query_as(
        r#"SELECT credential_type, credential_enc, access_token_enc
           FROM "LiteLLM_MCPUserCredentialTable"
           WHERE user_id = $1 AND server_id = $2"#,
    )
    .bind(user_id)
    .bind(server_id)
    .fetch_optional(pool)
    .await
    .map_err(GatewayError::Database)?;

    let Some((credential_type, credential_enc, access_token_enc)) = row else {
        return Ok(None);
    };

    let blob = if credential_type == TYPE_OAUTH {
        access_token_enc
    } else {
        credential_enc
    };
    match blob {
        Some(bytes) => Ok(Some(ResolvedCredential {
            value: crypto::decrypt(enc_key, &bytes)?,
        })),
        None => Ok(None),
    }
}

/// Status for one (user, server), without decrypting the secret.
pub async fn status(
    pool: &PgPool,
    user_id: &str,
    server_id: &str,
) -> Result<Option<McpUserCredentialStatus>, GatewayError> {
    let row: Option<(String, Option<i64>)> = sqlx::query_as(
        r#"SELECT credential_type, expires_at
           FROM "LiteLLM_MCPUserCredentialTable"
           WHERE user_id = $1 AND server_id = $2"#,
    )
    .bind(user_id)
    .bind(server_id)
    .fetch_optional(pool)
    .await
    .map_err(GatewayError::Database)?;

    Ok(
        row.map(|(credential_type, expires_at)| McpUserCredentialStatus {
            server_id: server_id.to_owned(),
            credential_type,
            has_credential: true,
            expires_at,
        }),
    )
}

/// All stored credentials for a user, across servers.
pub async fn list_for_user(
    pool: &PgPool,
    user_id: &str,
) -> Result<Vec<McpUserCredentialStatus>, GatewayError> {
    let rows: Vec<(String, String, Option<i64>)> = sqlx::query_as(
        r#"SELECT server_id, credential_type, expires_at
           FROM "LiteLLM_MCPUserCredentialTable"
           WHERE user_id = $1
           ORDER BY server_id"#,
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(GatewayError::Database)?;

    Ok(rows
        .into_iter()
        .map(
            |(server_id, credential_type, expires_at)| McpUserCredentialStatus {
                server_id,
                credential_type,
                has_credential: true,
                expires_at,
            },
        )
        .collect())
}

/// Delete a stored credential. Returns true if a row was removed.
pub async fn delete(pool: &PgPool, user_id: &str, server_id: &str) -> Result<bool, GatewayError> {
    let result = sqlx::query(
        r#"DELETE FROM "LiteLLM_MCPUserCredentialTable" WHERE user_id = $1 AND server_id = $2"#,
    )
    .bind(user_id)
    .bind(server_id)
    .execute(pool)
    .await
    .map_err(GatewayError::Database)?;
    Ok(result.rows_affected() > 0)
}
