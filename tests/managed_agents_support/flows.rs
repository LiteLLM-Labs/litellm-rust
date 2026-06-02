use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;

use super::{request_json, request_raw, AppFixture};

pub async fn create_agent(fixture: &AppFixture) -> String {
    let created = request_json(
        fixture.app.clone(),
        "POST",
        "/api/agents",
        Some(json!({
            "name": "ops-agent",
            "owner_id": "user-1",
            "prompt": "watch deploys"
        })),
    )
    .await;
    created["id"].as_str().unwrap().to_owned()
}

pub async fn exercise_agent_lifecycle(fixture: &AppFixture, agent_id: &str) {
    let listed = request_json(
        fixture.app.clone(),
        "GET",
        "/api/agents?owner_id=user-1",
        None,
    )
    .await;
    assert_eq!(listed["agents"].as_array().unwrap().len(), 1);

    let paused = request_json(
        fixture.app.clone(),
        "POST",
        &format!("/api/agents/{agent_id}/pause"),
        None,
    )
    .await;
    assert_eq!(paused["status"], "paused");

    let resumed = request_json(
        fixture.app.clone(),
        "POST",
        &format!("/api/agents/{agent_id}/resume"),
        None,
    )
    .await;
    assert_eq!(resumed["status"], "active");
}

pub async fn exercise_memory(fixture: &AppFixture, agent_id: &str) {
    let memory = request_json(
        fixture.app.clone(),
        "POST",
        &format!("/api/agents/{agent_id}/memory"),
        Some(json!({"key": "deploys", "value": "watch prod", "always_on": true})),
    )
    .await;
    assert_eq!(memory["key"], "deploys");

    let memories = request_json(
        fixture.app.clone(),
        "GET",
        &format!("/api/agents/{agent_id}/memory"),
        None,
    )
    .await;
    assert_eq!(memories["memories"].as_array().unwrap().len(), 1);

    request_json(
        fixture.app.clone(),
        "DELETE",
        &format!("/api/agents/{agent_id}/memory/deploys"),
        None,
    )
    .await;
}

pub async fn exercise_files(fixture: &AppFixture, agent_id: &str) {
    let file_path = format!("/api/agents/{agent_id}/files/notes.txt");
    request_raw(
        fixture.app.clone(),
        "PUT",
        &file_path,
        Some("hello".to_owned()),
        "text/plain",
        StatusCode::OK,
    )
    .await;

    let files = request_json(
        fixture.app.clone(),
        "GET",
        &format!("/api/agents/{agent_id}/files"),
        None,
    )
    .await;
    assert_eq!(files["files"].as_array().unwrap().len(), 1);

    let file = request_raw(
        fixture.app.clone(),
        "GET",
        &file_path,
        None,
        "application/json",
        StatusCode::OK,
    )
    .await;
    assert_eq!(file, "hello");

    request_json(fixture.app.clone(), "DELETE", &file_path, None).await;
}

pub async fn exercise_runs(fixture: &AppFixture, agent_id: &str) {
    let run = request_json(
        fixture.app.clone(),
        "POST",
        &format!("/api/agents/{agent_id}/run"),
        Some(json!({})),
    )
    .await;
    assert!(run["run_id"].as_str().is_some());
    assert_eq!(run["event_url"], "/event");

    let runs = request_json(
        fixture.app.clone(),
        "GET",
        &format!("/api/agents/{agent_id}/runs"),
        None,
    )
    .await;
    assert_eq!(runs["runs"].as_array().unwrap().len(), 1);
}

pub async fn exercise_skills(fixture: &AppFixture) {
    let skill = request_json(
        fixture.app.clone(),
        "POST",
        "/api/skills",
        Some(json!({"name": "triage", "content": "do triage", "owner_id": "user-1"})),
    )
    .await;
    let skill_id = skill["id"].as_str().unwrap();
    let skill = request_json(
        fixture.app.clone(),
        "PATCH",
        &format!("/api/skills/{skill_id}"),
        Some(json!({"description": "daily"})),
    )
    .await;
    assert_eq!(skill["description"], "daily");
}

pub async fn exercise_inbox(fixture: &AppFixture) {
    seed_inbox(&fixture.pool).await;
    let inbox = request_json(
        fixture.app.clone(),
        "GET",
        "/api/inbox?filter=attention",
        None,
    )
    .await;
    assert_eq!(inbox["items"].as_array().unwrap().len(), 2);

    request_json(
        fixture.app.clone(),
        "POST",
        "/api/approvals/appr_1/accept",
        Some(json!({"arguments": {"ok": true}})),
    )
    .await;
    request_json(
        fixture.app.clone(),
        "POST",
        "/api/inbox/iss_1/resolve",
        Some(json!({"note": "done"})),
    )
    .await;
}

async fn seed_inbox(pool: &PgPool) {
    sqlx::query(
        r#"
        INSERT INTO "LiteLLM_ManagedAgentInboxItemsTable"
          (id, kind, title, status, created_at)
        VALUES
          ('appr_1', 'approval', 'approve deploy', 'pending', 1),
          ('iss_1', 'issue', 'deployment issue', 'open', 2)
        "#,
    )
    .execute(pool)
    .await
    .unwrap();
}
