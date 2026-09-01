//! Hermetic OpenAI-compatible mock provider for integration tests.
//!
//! A std-only TCP server speaking the chat-completions SSE protocol.
//! Scripted behaviors keyed off the last user message:
//! - contains "slow"  → streams word-by-word with 700 ms gaps (cancel tests)
//! - contains "tool"  → first turn returns a `get_scan_summary` tool call;
//!   after the tool result is fed back, answers with the computer name found
//!   in the tool result (proving real tool data reached the model)
//! - otherwise        → streams "`MOCK_REPLY`: you said …" (round-trip tests)

#![allow(unsafe_code)]

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc as std_mpsc;
use std::time::Duration;

pub const MOCK_PORT: u16 = 18093;
pub const MOCK_ENDPOINT: &str = "http://127.0.0.1:18093";
pub const MOCK_MODEL: &str = "mock-model";
pub const SCAN_SUMMARY_TOOL: &str = "get_scan_summary";

pub struct MockController {
    shutdown: std_mpsc::Sender<()>,
}

/// Bind the mock on 127.0.0.1 and spawn its accept loop. Returns a
/// controller whose drop stops the server.
///
/// # Panics
/// When the port cannot be bound.
#[must_use]
pub fn spawn() -> MockController {
    let listener =
        TcpListener::bind(("127.0.0.1", MOCK_PORT)).expect("mock provider port must bind");
    listener
        .set_nonblocking(true)
        .expect("mock provider listener must become nonblocking");
    let (shutdown_tx, shutdown_rx) = std_mpsc::channel::<()>();
    std::thread::Builder::new()
        .name("mock-provider".to_string())
        .spawn(move || {
            loop {
                if shutdown_rx.try_recv().is_ok() {
                    return;
                }
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = std::thread::Builder::new()
                            .name("mock-conn".to_string())
                            .spawn(move || handle_connection(stream));
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => return,
                }
            }
        })
        .expect("mock provider thread");
    MockController {
        shutdown: shutdown_tx,
    }
}

impl Drop for MockController {
    fn drop(&mut self) {
        let _ = self.shutdown.send(());
    }
}

fn handle_connection(mut stream: TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut request = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        match stream.read(&mut byte) {
            Ok(0) | Err(_) => return,
            Ok(_) => {
                request.push(byte[0]);
                if request.ends_with(b"\r\n\r\n") {
                    break;
                }
                if request.len() > 64 * 1024 {
                    return;
                }
            }
        }
    }
    let headers = String::from_utf8_lossy(&request).to_string();
    let content_length = headers
        .to_ascii_lowercase()
        .lines()
        .find_map(|line| {
            line.strip_prefix("content-length:")
                .and_then(|value| value.trim().parse::<usize>().ok())
        })
        .unwrap_or(0);
    let mut body = vec![0_u8; content_length];
    if content_length > 0 && stream.read_exact(&mut body).is_err() {
        return;
    }
    let body_text = String::from_utf8_lossy(&body).to_string();
    let path = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or_default()
        .to_string();

    if path.starts_with("/v1/models") {
        let body = "{\"data\":[]}";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes());
        return;
    }
    if !path.starts_with("/v1/chat/completions") {
        let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
        return;
    }

    // Script the response from the conversation content.
    let last_user = last_user_content(&body_text);
    let lowered = last_user.to_ascii_lowercase();
    let has_tool_result = body_text.contains("\"role\":\"tool\"");
    let has_tools = body_text.contains("\"tools\":[");

    if stream
        .write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
        )
        .is_err()
    {
        return;
    }

    if has_tools && !has_tool_result && lowered.contains("tool") {
        stream
            .write_all(tool_call_response(SCAN_SUMMARY_TOOL).as_bytes())
            .ok();
        return;
    }
    if has_tool_result {
        let computer = extract_computer_name(&body_text).unwrap_or_else(|| "UNKNOWN".to_string());
        stream_text_response(
            &mut stream,
            &format!("MOCK_TOOL_REPLY: this machine is {computer}"),
        );
        return;
    }
    if lowered.contains("slow") {
        stream_slow_response(&mut stream);
        return;
    }
    stream_text_response(
        &mut stream,
        &format!("MOCK_REPLY: you said {}", last_user.trim()),
    );
}

fn last_user_content(request_body: &str) -> String {
    let document: serde_json::Value = serde_json::from_str(request_body).unwrap_or_default();
    document
        .get("messages")
        .and_then(|messages| messages.as_array())
        .and_then(|messages| {
            messages
                .iter()
                .rev()
                .find(|message| message.get("role").and_then(|role| role.as_str()) == Some("user"))
                .and_then(|message| message.get("content"))
                .and_then(|content| content.as_str())
                .map(str::to_string)
        })
        .unwrap_or_default()
}

fn extract_computer_name(tool_result_text: &str) -> Option<String> {
    let marker = "Computer name: ";
    let start = tool_result_text.find(marker)? + marker.len();
    let rest = &tool_result_text[start..];
    let end = rest.find(['\n', '\r']).unwrap_or(rest.len());
    Some(rest[..end].trim().to_string())
}

fn sse_chunk(delta: &str, finish: Option<&str>) -> String {
    let delta_body = if delta.is_empty() {
        "{}".to_string()
    } else {
        format!("{{\"content\":{}}}", serde_json::to_string(delta).unwrap())
    };
    let finish_body = match finish {
        None => "null".to_string(),
        Some(reason) => serde_json::to_string(reason).unwrap(),
    };
    format!(
        "data: {{\"id\":\"mock\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"{MOCK_MODEL}\",\"choices\":[{{\"index\":0,\"delta\":{delta_body},\"finish_reason\":{finish_body}}}]}}\n\n"
    )
}

fn tool_call_response(tool_name: &str) -> String {
    let arguments = serde_json::to_string("{}").unwrap();
    format!(
        "data: {{\"id\":\"mock\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"{MOCK_MODEL}\",\"choices\":[{{\"index\":0,\"delta\":{{\"role\":\"assistant\",\"tool_calls\":[{{\"index\":0,\"id\":\"call_mock\",\"type\":\"function\",\"function\":{{\"name\":\"{tool_name}\",\"arguments\":{arguments}}}}}]}},\"finish_reason\":null}}]}}\n\ndata: {{\"id\":\"mock\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"{MOCK_MODEL}\",\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"tool_calls\"}}]}}\n\ndata: [DONE]\n\n"
    )
}

fn stream_text_response(stream: &mut TcpStream, text: &str) {
    let mut response = String::new();
    for word in text.split(' ') {
        response.push_str(&sse_chunk(&format!("{word} "), None));
    }
    response.push_str(&sse_chunk("", Some("stop")));
    response.push_str("data: [DONE]\n\n");
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn stream_slow_response(stream: &mut TcpStream) {
    for index in 0..30 {
        let _ = stream.write_all(sse_chunk(&format!("chunk{index} "), None).as_bytes());
        let _ = stream.flush();
        std::thread::sleep(Duration::from_millis(700));
    }
    let _ = stream.write_all(sse_chunk("", Some("stop")).as_bytes());
}
