use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, MutexGuard,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use serde_json::{json, Value};

use crate::agents::events;

#[derive(Debug, Clone, Serialize)]
pub struct AgentSession {
    pub id: String,
    pub title: Option<String>,
    pub agent: String,
    pub agent_id: String,
    pub harness: String,
    pub time: SessionTime,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionTime {
    pub created: u128,
    pub updated: u128,
}

#[derive(Debug)]
struct StoredSession {
    session: AgentSession,
    messages: Vec<Value>,
}

#[derive(Debug, Default)]
pub struct AgentSessionStore {
    sessions: Mutex<HashMap<String, StoredSession>>,
    counter: AtomicU64,
}

impl AgentSessionStore {
    pub fn create(&self, title: Option<String>, agent_id: String, harness: String) -> AgentSession {
        let id = format!(
            "session_{}_{:x}",
            now_millis(),
            self.counter.fetch_add(1, Ordering::Relaxed)
        );
        let session = AgentSession {
            id: id.clone(),
            title,
            agent: agent_id.clone(),
            agent_id,
            harness,
            time: SessionTime {
                created: now_millis(),
                updated: now_millis(),
            },
        };
        self.sessions().insert(
            id,
            StoredSession {
                session: session.clone(),
                messages: Vec::new(),
            },
        );
        session
    }

    pub fn list(&self) -> Vec<AgentSession> {
        let mut sessions: Vec<_> = self
            .sessions()
            .values()
            .map(|stored| stored.session.clone())
            .collect();
        sessions.sort_by_key(|session| std::cmp::Reverse(session.time.created));
        sessions
    }

    pub fn get(&self, session_id: &str) -> Option<AgentSession> {
        self.sessions()
            .get(session_id)
            .map(|stored| stored.session.clone())
    }

    pub fn delete(&self, session_id: &str) -> bool {
        self.sessions().remove(session_id).is_some()
    }

    pub fn messages(&self, session_id: &str) -> Option<Vec<Value>> {
        self.sessions()
            .get(session_id)
            .map(|stored| stored.messages.clone())
    }

    pub fn push_user_message(&self, session_id: &str, text: &str) {
        let mut sessions = self.sessions();
        let Some(stored) = sessions.get_mut(session_id) else {
            return;
        };
        let message_id = format!("{}_user_{}", session_id, stored.messages.len());
        stored.messages.push(json!({
            "info": {
                "id": message_id,
                "role": "user",
                "sessionID": session_id,
                "time": { "created": now_millis() },
            },
            "parts": [{
                "id": format!("{message_id}_text"),
                "messageID": message_id,
                "sessionID": session_id,
                "type": "text",
                "text": text,
            }],
        }));
        stored.session.time.updated = now_millis();
        if stored.session.title.as_deref().unwrap_or("").is_empty() {
            stored.session.title = Some(short_title(text));
        }
    }

    pub fn apply_event(&self, session_id: &str, event: &str, data: &Value) {
        match event {
            events::MESSAGE_UPDATED => self.apply_message_updated(session_id, data),
            events::MESSAGE_PART_UPDATED => self.apply_part_updated(session_id, data),
            events::MESSAGE_PART_DELTA => self.apply_part_delta(session_id, data),
            _ => {}
        }
    }

    fn apply_message_updated(&self, session_id: &str, data: &Value) {
        let Some(info) = data.get("info").cloned() else {
            return;
        };
        let Some(message_id) = info.get("id").and_then(Value::as_str) else {
            return;
        };
        let mut sessions = self.sessions();
        let Some(stored) = sessions.get_mut(session_id) else {
            return;
        };
        let message = find_or_insert_message(&mut stored.messages, message_id, info.clone());
        if let Some(current) = message.get_mut("info").and_then(Value::as_object_mut) {
            merge_object(current, info);
        }
        stored.session.time.updated = now_millis();
    }

    fn apply_part_updated(&self, session_id: &str, data: &Value) {
        let Some(part) = data.get("part").cloned() else {
            return;
        };
        let Some(message_id) = part.get("messageID").and_then(Value::as_str) else {
            return;
        };
        let Some(part_id) = part.get("id").and_then(Value::as_str).map(str::to_owned) else {
            return;
        };
        let mut sessions = self.sessions();
        let Some(stored) = sessions.get_mut(session_id) else {
            return;
        };
        let message = find_or_insert_message(
            &mut stored.messages,
            message_id,
            default_info(message_id, session_id),
        );
        upsert_part(message, &part_id, part);
        stored.session.time.updated = now_millis();
    }

    fn apply_part_delta(&self, session_id: &str, data: &Value) {
        let Some(message_id) = data.get("messageID").and_then(Value::as_str) else {
            return;
        };
        let Some(part_id) = data.get("partID").and_then(Value::as_str) else {
            return;
        };
        let delta = data.get("delta").and_then(Value::as_str).unwrap_or("");
        let mut sessions = self.sessions();
        let Some(stored) = sessions.get_mut(session_id) else {
            return;
        };
        let message = find_or_insert_message(
            &mut stored.messages,
            message_id,
            default_info(message_id, session_id),
        );
        append_part_text(message, part_id, delta);
        stored.session.time.updated = now_millis();
    }

    fn sessions(&self) -> MutexGuard<'_, HashMap<String, StoredSession>> {
        self.sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn find_or_insert_message<'a>(
    messages: &'a mut Vec<Value>,
    id: &str,
    info: Value,
) -> &'a mut Value {
    if let Some(index) = messages
        .iter()
        .position(|message| message["info"]["id"].as_str() == Some(id))
    {
        return &mut messages[index];
    }
    let index = messages.len();
    messages.push(json!({ "info": info, "parts": [] }));
    &mut messages[index]
}

fn upsert_part(message: &mut Value, part_id: &str, part: Value) {
    let Some(parts) = message.get_mut("parts").and_then(Value::as_array_mut) else {
        return;
    };
    if let Some(index) = parts
        .iter()
        .position(|part| part.get("id").and_then(Value::as_str) == Some(part_id))
    {
        parts[index] = part;
    } else {
        parts.push(part);
    }
}

fn append_part_text(message: &mut Value, part_id: &str, delta: &str) {
    let Some(parts) = message.get_mut("parts").and_then(Value::as_array_mut) else {
        return;
    };
    let Some(part) = parts
        .iter_mut()
        .find(|part| part.get("id").and_then(Value::as_str) == Some(part_id))
    else {
        return;
    };
    let current = part.get("text").and_then(Value::as_str).unwrap_or("");
    part["text"] = format!("{current}{delta}").into();
}

fn merge_object(target: &mut serde_json::Map<String, Value>, source: Value) {
    let Some(source) = source.as_object() else {
        return;
    };
    for (key, value) in source {
        target.insert(key.clone(), value.clone());
    }
}

fn default_info(message_id: &str, session_id: &str) -> Value {
    json!({
        "id": message_id,
        "role": "assistant",
        "sessionID": session_id,
        "time": { "created": now_millis() },
    })
}

fn short_title(text: &str) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let title = compact.chars().take(64).collect::<String>();
    if title.len() < compact.len() {
        format!("{}...", title.trim_end())
    } else {
        compact
    }
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
