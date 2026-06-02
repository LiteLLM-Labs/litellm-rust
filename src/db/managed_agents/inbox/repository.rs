use sqlx::PgPool;

use crate::{db::managed_agents::now_ms, errors::GatewayError};

use super::schema::InboxItemRow;

pub async fn list(pool: &PgPool, filter: &str) -> Result<Vec<InboxItemRow>, GatewayError> {
    let rows = match filter {
        "attention" => {
            sqlx::query_as::<_, InboxItemRow>(
                r#"
                SELECT *
                FROM "LiteLLM_ManagedAgentInboxItemsTable"
                WHERE status IN ('pending', 'open')
                ORDER BY created_at DESC
                "#,
            )
            .fetch_all(pool)
            .await
        }
        "completed" => {
            sqlx::query_as::<_, InboxItemRow>(
                r#"
                SELECT *
                FROM "LiteLLM_ManagedAgentInboxItemsTable"
                WHERE status IN ('accepted', 'rejected', 'resolved')
                ORDER BY created_at DESC
                "#,
            )
            .fetch_all(pool)
            .await
        }
        _ => {
            sqlx::query_as::<_, InboxItemRow>(
                r#"
                SELECT *
                FROM "LiteLLM_ManagedAgentInboxItemsTable"
                ORDER BY created_at DESC
                "#,
            )
            .fetch_all(pool)
            .await
        }
    }
    .map_err(GatewayError::Database)?;

    Ok(rows)
}

pub async fn pending_approvals(pool: &PgPool) -> Result<Vec<InboxItemRow>, GatewayError> {
    sqlx::query_as::<_, InboxItemRow>(
        r#"
        SELECT *
        FROM "LiteLLM_ManagedAgentInboxItemsTable"
        WHERE kind = 'approval' AND status = 'pending'
        ORDER BY created_at ASC
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(GatewayError::Database)
}

pub async fn resolve_issue(
    pool: &PgPool,
    item_id: &str,
    note: Option<String>,
) -> Result<bool, GatewayError> {
    let result = sqlx::query(
        r#"
        UPDATE "LiteLLM_ManagedAgentInboxItemsTable"
        SET status = 'resolved', feedback = COALESCE($2, feedback), resolved_at = $3
        WHERE id = $1 AND kind = 'issue' AND status = 'open'
        "#,
    )
    .bind(item_id)
    .bind(note)
    .bind(now_ms())
    .execute(pool)
    .await
    .map_err(GatewayError::Database)?;

    Ok(result.rows_affected() > 0)
}

pub async fn decide_approval(
    pool: &PgPool,
    item_id: &str,
    decision: &str,
    feedback: Option<String>,
    arguments: Option<serde_json::Value>,
) -> Result<bool, GatewayError> {
    let status = match decision {
        "accept" => "accepted",
        "reject" => "rejected",
        _ => {
            return Err(GatewayError::InvalidJsonMessage(
                "invalid decision".to_owned(),
            ))
        }
    };
    let args_json = arguments.map(|value| value.to_string());
    let result = sqlx::query(
        r#"
        UPDATE "LiteLLM_ManagedAgentInboxItemsTable"
        SET status = $2,
            feedback = COALESCE($3, feedback),
            args_json = COALESCE($4, args_json),
            resolved_at = $5
        WHERE id = $1 AND kind = 'approval' AND status = 'pending'
        "#,
    )
    .bind(item_id)
    .bind(status)
    .bind(feedback)
    .bind(args_json)
    .bind(now_ms())
    .execute(pool)
    .await
    .map_err(GatewayError::Database)?;

    Ok(result.rows_affected() > 0)
}
