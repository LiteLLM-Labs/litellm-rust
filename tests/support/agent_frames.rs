use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine};
use serde_json::json;

pub(super) fn process_frames() -> Vec<Vec<u8>> {
    vec![
        br#"{"event":{"start":{"pid":1470}}}"#.to_vec(),
        stdout_frame(
            json!({"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}}),
        ),
        stdout_frame(
            json!({"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello "}}}),
        ),
        stdout_frame(
            json!({"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"from sandbox\n"}}}),
        ),
        stdout_frame(
            json!({"type":"stream_event","event":{"type":"content_block_start","index":1,"content_block":{"type":"thinking","thinking":""}}}),
        ),
        stdout_frame(
            json!({"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"thinking_delta","thinking":"thinking trace"}}}),
        ),
        stdout_frame(
            json!({"type":"stream_event","event":{"type":"content_block_start","index":2,"content_block":{"type":"tool_use","name":"bash","input":{}}}}),
        ),
        stdout_frame(
            json!({"type":"stream_event","event":{"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"command\":\"pwd\"}"}}}),
        ),
        stdout_frame(
            json!({"type":"stream_event","event":{"type":"content_block_stop","index":2}}),
        ),
        stderr_frame(json!({"type":"text_delta","text":"npm notice\n"})),
        br#"{"event":{"end":{"exited":true,"status":"exit status 0"}}}"#.to_vec(),
    ]
}

pub(super) fn process_frames_without_tool_stop() -> Vec<Vec<u8>> {
    let mut frames = process_frames();
    frames.retain(|frame| {
        std::str::from_utf8(frame)
            .map(|frame| !frame.contains(r#""content_block_stop","index":2"#))
            .unwrap_or(true)
    });
    frames
}

fn stdout_frame(value: serde_json::Value) -> Vec<u8> {
    output_frame("stdout", value)
}

fn stderr_frame(value: serde_json::Value) -> Vec<u8> {
    output_frame("stderr", value)
}

fn output_frame(stream: &str, value: serde_json::Value) -> Vec<u8> {
    json!({ stream: BASE64_STANDARD.encode(format!("{value}\n")) })
        .to_string()
        .into_bytes()
}

pub(super) fn connect_json_frames(payloads: Vec<Vec<u8>>) -> Vec<u8> {
    let mut frames = Vec::new();
    for payload in payloads.iter() {
        frames.push(0);
        frames.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        frames.extend_from_slice(payload);
    }
    frames
}
