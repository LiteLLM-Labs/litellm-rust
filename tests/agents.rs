#[path = "support/agents.rs"]
mod agents_support;

use agents_support::{
    app_for_e2b, create_agent_session, mock_e2b, mock_e2b_without_tool_stop,
    read_events_until_completed, send_session_prompt, session_messages, start_agent_run,
};

#[tokio::test]
async fn starts_agent_and_streams_e2b_output() {
    let e2b = mock_e2b().await;
    let app = app_for_e2b(e2b.uri());

    let (event_url, run_id) = start_agent_run(&app).await;
    let body = read_events_until_completed(app, event_url).await;

    assert_ui_events(&body);
    assert!(body.contains(&run_id));
}

#[tokio::test]
async fn creates_agent_session_and_streams_chat_events() {
    let e2b = mock_e2b().await;
    let app = app_for_e2b(e2b.uri());

    let session_id = create_agent_session(&app).await;
    send_session_prompt(&app, &session_id).await;
    let body = read_events_until_completed(app.clone(), "/event".to_owned()).await;

    assert!(body.contains(&session_id));
    assert_ui_events(&body);
    assert_session_delta(&body, &session_id);

    let messages = session_messages(&app, &session_id).await;
    assert!(messages.contains("please say hello"));
    assert!(messages.contains("hello from sandbox"));
    assert!(messages.contains("thinking trace"));
    assert!(messages.contains("\"tool\":\"bash\""));
}

#[tokio::test]
async fn completes_open_tool_parts_when_stream_omits_tool_stop() {
    let e2b = mock_e2b_without_tool_stop().await;
    let app = app_for_e2b(e2b.uri());

    let session_id = create_agent_session(&app).await;
    send_session_prompt(&app, &session_id).await;
    let body = read_events_until_completed(app.clone(), "/event".to_owned()).await;

    assert!(body.contains("\"type\":\"session.idle\""));
    assert!(body.contains("\"status\":\"completed\""));

    let messages = session_messages(&app, &session_id).await;
    assert!(messages.contains("\"tool\":\"bash\""));
    assert!(messages.contains("\"status\":\"completed\""));
    assert!(!messages.contains("\"status\":\"running\""));
}

fn assert_ui_events(body: &str) {
    assert!(body.contains("\"type\":\"session.status\""));
    assert!(body.contains("\"type\":\"message.updated\""));
    assert!(body.contains("\"type\":\"message.part.updated\""));
    assert!(body.contains("\"type\":\"message.part.delta\""));
    assert!(body.contains("\"delta\":\"hello \""));
    assert!(body.contains("\"delta\":\"from sandbox\\n\""));
    assert!(body.contains("thinking trace"));
    assert!(body.contains("\"type\":\"tool\""));
    assert!(body.contains("\"tool\":\"bash\""));
    assert!(body.contains("\"field\":\"text\""));
    assert!(body.contains("\"sessionID\""));
    assert!(!body.contains("npm notice"));
    assert!(!body.contains("\"stream\":\"stderr\""));
    assert!(!body.contains("\"event\":{\"start\""));
    assert!(!body.contains("\"event\":{\"end\""));
    assert!(body.contains("\"type\":\"session.idle\""));
}

fn assert_session_delta(body: &str, session_id: &str) {
    let has_delta_for_session = body.lines().any(|line| {
        line.contains("\"type\":\"message.part.delta\"")
            && line.contains(&format!("\"sessionID\":\"{session_id}\""))
    });
    assert!(has_delta_for_session);
}
