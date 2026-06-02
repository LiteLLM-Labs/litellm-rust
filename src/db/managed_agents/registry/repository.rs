use serde_json::json;
use sqlx::PgPool;

use crate::{
    db::managed_agents::{id, now_ms},
    errors::GatewayError,
};

use super::schema::{CreateManagedAgent, ManagedAgentRow, UpdateManagedAgent};

pub async fn create(
    pool: &PgPool,
    input: CreateManagedAgent,
) -> Result<ManagedAgentRow, GatewayError> {
    if input.name.trim().is_empty() || input.owner_id.trim().is_empty() {
        return Err(GatewayError::InvalidJsonMessage(
            "name and owner_id required".to_owned(),
        ));
    }

    let now = now_ms();
    let agent_id = id("agent");
    let session_id = id("ses");
    let title = format!("agent-builder-{}", input.name);
    let model = input
        .model
        .unwrap_or_else(|| "claude-sonnet-4-6".to_owned());
    let system = input
        .system
        .or_else(|| input.prompt.clone())
        .unwrap_or_default();
    let cron = input
        .schedule
        .as_ref()
        .map(|schedule| schedule.cron.clone());
    let timezone = input
        .schedule
        .as_ref()
        .and_then(|schedule| schedule.timezone.clone())
        .unwrap_or_else(|| "UTC".to_owned());

    let mut tx = pool.begin().await.map_err(GatewayError::Database)?;
    sqlx::query(
        r#"
        INSERT INTO "LiteLLM_ManagedAgentSessionsTable"
          (id, harness, agent_id, title, created_at)
        VALUES ($1, $2, $3, $4, $5)
        "#,
    )
    .bind(&session_id)
    .bind("cc")
    .bind(&agent_id)
    .bind(title)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(GatewayError::Database)?;

    let row = sqlx::query_as::<_, ManagedAgentRow>(
        r#"
        INSERT INTO "LiteLLM_ManagedAgentsTable" (
          id, name, model, system, tools, cadence, interval_seconds, session_id,
          loop_id, created_at, prompt, cron, timezone, vault_keys, setup_commands,
          max_runtime_minutes, on_failure, config, owner_id, status, description,
          harness, skill_ids
        )
        VALUES (
          $1, $2, $3, $4, $5, $6, NULL, $7,
          NULL, $8, $9, $10, $11, $12, $13,
          $14, $15, $16, $17, 'paused', $18,
          $19, $20
        )
        RETURNING *
        "#,
    )
    .bind(&agent_id)
    .bind(input.name)
    .bind(model)
    .bind(system)
    .bind(json!([]))
    .bind(cron.clone())
    .bind(&session_id)
    .bind(now)
    .bind(input.prompt)
    .bind(cron)
    .bind(timezone)
    .bind(input.vault_keys.unwrap_or_else(|| json!([])))
    .bind(input.setup_commands.unwrap_or_else(|| json!([])))
    .bind(input.max_runtime_minutes.unwrap_or(30))
    .bind(
        input
            .on_failure
            .unwrap_or_else(|| "pause_and_notify".to_owned()),
    )
    .bind(input.config.unwrap_or_else(|| json!({})))
    .bind(input.owner_id)
    .bind(input.description)
    .bind(input.harness.unwrap_or_else(|| "claude-code".to_owned()))
    .bind(input.skill_ids.unwrap_or_else(|| json!([])))
    .fetch_one(&mut *tx)
    .await
    .map_err(GatewayError::Database)?;

    tx.commit().await.map_err(GatewayError::Database)?;
    Ok(row)
}

pub async fn list(
    pool: &PgPool,
    owner_id: Option<&str>,
) -> Result<Vec<ManagedAgentRow>, GatewayError> {
    let rows = if let Some(owner_id) = owner_id {
        sqlx::query_as::<_, ManagedAgentRow>(
            r#"
            SELECT * FROM "LiteLLM_ManagedAgentsTable"
            WHERE owner_id = $1
            ORDER BY created_at ASC
            "#,
        )
        .bind(owner_id)
        .fetch_all(pool)
        .await
    } else {
        sqlx::query_as::<_, ManagedAgentRow>(
            r#"
            SELECT * FROM "LiteLLM_ManagedAgentsTable"
            ORDER BY created_at ASC
            "#,
        )
        .fetch_all(pool)
        .await
    }
    .map_err(GatewayError::Database)?;

    Ok(rows)
}

pub async fn get(pool: &PgPool, agent_id: &str) -> Result<Option<ManagedAgentRow>, GatewayError> {
    sqlx::query_as::<_, ManagedAgentRow>(
        r#"SELECT * FROM "LiteLLM_ManagedAgentsTable" WHERE id = $1"#,
    )
    .bind(agent_id)
    .fetch_optional(pool)
    .await
    .map_err(GatewayError::Database)
}

pub async fn update(
    pool: &PgPool,
    agent_id: &str,
    input: UpdateManagedAgent,
) -> Result<Option<ManagedAgentRow>, GatewayError> {
    sqlx::query_as::<_, ManagedAgentRow>(
        r#"
        UPDATE "LiteLLM_ManagedAgentsTable"
        SET
          name = COALESCE($2, name),
          model = COALESCE($3, model),
          system = COALESCE($4, system),
          prompt = COALESCE($5, prompt),
          cron = COALESCE($6, cron),
          timezone = COALESCE($7, timezone),
          vault_keys = COALESCE($8, vault_keys),
          setup_commands = COALESCE($9, setup_commands),
          max_runtime_minutes = COALESCE($10, max_runtime_minutes),
          on_failure = COALESCE($11, on_failure),
          config = COALESCE($12, config),
          owner_id = COALESCE($13, owner_id),
          status = COALESCE($14, status),
          description = COALESCE($15, description),
          harness = COALESCE($16, harness),
          skill_ids = COALESCE($17, skill_ids)
        WHERE id = $1
        RETURNING *
        "#,
    )
    .bind(agent_id)
    .bind(input.name)
    .bind(input.model)
    .bind(input.system)
    .bind(input.prompt)
    .bind(input.cron)
    .bind(input.timezone)
    .bind(input.vault_keys)
    .bind(input.setup_commands)
    .bind(input.max_runtime_minutes)
    .bind(input.on_failure)
    .bind(input.config)
    .bind(input.owner_id)
    .bind(input.status)
    .bind(input.description)
    .bind(input.harness)
    .bind(input.skill_ids)
    .fetch_optional(pool)
    .await
    .map_err(GatewayError::Database)
}

pub async fn set_status(
    pool: &PgPool,
    agent_id: &str,
    status: &str,
) -> Result<Option<ManagedAgentRow>, GatewayError> {
    sqlx::query_as::<_, ManagedAgentRow>(
        r#"
        UPDATE "LiteLLM_ManagedAgentsTable"
        SET status = $2
        WHERE id = $1
        RETURNING *
        "#,
    )
    .bind(agent_id)
    .bind(status)
    .fetch_optional(pool)
    .await
    .map_err(GatewayError::Database)
}

pub async fn delete(pool: &PgPool, agent_id: &str) -> Result<bool, GatewayError> {
    let result = sqlx::query(r#"DELETE FROM "LiteLLM_ManagedAgentsTable" WHERE id = $1"#)
        .bind(agent_id)
        .execute(pool)
        .await
        .map_err(GatewayError::Database)?;

    Ok(result.rows_affected() > 0)
}
