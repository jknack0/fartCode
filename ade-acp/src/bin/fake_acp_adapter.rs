//! Fake ACP adapter binary — the E2-11-1 test fixture.
//!
//! Speaks enough of ACP v1 over stdio to exercise the client:
//! `initialize`, `session/new`, `session/load` (replays two updates),
//! `session/prompt` with three behaviors keyed off the prompt text:
//!
//! - contains "tool"    → agent chunk, tool_call, permission request
//!   (blocks until the client answers), tool_call_update completed,
//!   final chunk, `end_turn`
//! - contains "hang"    → streams nothing; waits until `session/cancel`,
//!   then responds `cancelled`
//! - contains "readfile" → issues `fs/read_text_file` against
//!   `<cwd>/fixture.txt` and echoes the content back
//! - anything else      → two agent message chunks, `end_turn`
//!
//! Malformed input lines are warned and skipped — never a crash.

use std::io::{BufRead, Write};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::{json, Value};

type Out = Arc<Mutex<std::io::Stdout>>;

fn send(out: &Out, frame: Value) {
    let mut stdout = out.lock();
    let mut line = serde_json::to_vec(&frame).unwrap();
    line.push(b'\n');
    stdout.write_all(&line).unwrap();
    stdout.flush().unwrap();
}

fn reply(out: &Out, request: &Value, result: Value) {
    send(
        out,
        json!({ "jsonrpc": "2.0", "id": request["id"], "result": result }),
    );
}

fn reply_error(out: &Out, request: &Value, code: i32, message: &str) {
    send(
        out,
        json!({ "jsonrpc": "2.0", "id": request["id"], "error": { "code": code, "message": message } }),
    );
}

fn session_update(out: &Out, session_id: &str, update: Value) {
    send(
        out,
        json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": { "sessionId": session_id, "update": update },
        }),
    );
}

fn agent_chunk(out: &Out, session_id: &str, text: &str) {
    session_update(
        out,
        session_id,
        json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": text },
        }),
    );
}

fn prompt_text(params: &Value) -> String {
    params["prompt"]
        .as_array()
        .and_then(|blocks| blocks.first())
        .and_then(|block| block["text"].as_str())
        .unwrap_or("")
        .to_string()
}

/// Blocks on the frames channel until a response with `id` arrives.
/// `session/cancel` notifications seen en route flip `cancelled`.
fn await_response(rx: &Receiver<Value>, id: i64, cancelled: &Mutex<bool>) -> Option<Value> {
    while let Ok(frame) = rx.recv() {
        if frame.get("method").and_then(|m| m.as_str()) == Some("session/cancel") {
            *cancelled.lock() = true;
            continue;
        }
        if frame["id"].as_i64() == Some(id) {
            return Some(frame);
        }
    }
    None
}

fn handle_prompt(
    rx: &Receiver<Value>,
    out: &Out,
    request: &Value,
    session_id: &str,
    cwd: &Mutex<String>,
    cancelled: &Mutex<bool>,
) {
    let text = prompt_text(&request["params"]);

    if text.contains("envcheck") {
        // Echo the value of ADE_TEST_ENV the adapter process received —
        // E2-11-2 asserts server-resolved env reaches the adapter.
        let value = std::env::var("ADE_TEST_ENV").unwrap_or_else(|_| "<unset>".into());
        agent_chunk(out, session_id, &format!("ENV: {value}"));
        reply(out, request, json!({ "stopReason": "end_turn" }));
        return;
    }

    if text.contains("rich") {
        // E2-11-4 acceptance: one interleaved sequence exercising every
        // reducer slice — thought, message, tool_call lifecycle, plan,
        // config options, usage, title.
        session_update(
            out,
            session_id,
            json!({
                "sessionUpdate": "agent_thought_chunk",
                "content": { "type": "text", "text": "Let me think. " },
            }),
        );
        agent_chunk(out, session_id, "Starting the work. ");
        session_update(
            out,
            session_id,
            json!({
                "sessionUpdate": "tool_call",
                "toolCallId": "call_rich_1",
                "title": "cargo test",
                "kind": "execute",
                "status": "in_progress",
            }),
        );
        session_update(
            out,
            session_id,
            json!({
                "sessionUpdate": "plan",
                "entries": [
                    { "content": "write code", "priority": "high", "status": "completed" },
                    { "content": "run tests", "priority": "high", "status": "in_progress" },
                ],
            }),
        );
        session_update(
            out,
            session_id,
            json!({
                "sessionUpdate": "config_option_update",
                "configOptions": [{
                    "id": "model",
                    "name": "Model",
                    "category": "model",
                    "type": "select",
                    "currentValue": "fake-1",
                    "options": [
                        { "value": "fake-1", "name": "Fake 1" },
                        { "value": "fake-2", "name": "Fake 2" },
                    ],
                }],
            }),
        );
        session_update(
            out,
            session_id,
            json!({
                "sessionUpdate": "usage_update",
                "used": 1234,
                "size": 200000,
            }),
        );
        session_update(
            out,
            session_id,
            json!({
                "sessionUpdate": "session_info_update",
                "title": "Rich turn",
            }),
        );
        session_update(
            out,
            session_id,
            json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": "call_rich_1",
                "status": "completed",
                "content": [{
                    "type": "content",
                    "content": { "type": "text", "text": "all tests passed" },
                }],
            }),
        );
        agent_chunk(out, session_id, "All done.");
        reply(out, request, json!({ "stopReason": "end_turn" }));
        return;
    }

    if text.contains("emitgarbage") {
        let mut stdout = out.lock();
        stdout.write_all(b"this is not json\n").unwrap();
        stdout.flush().unwrap();
        drop(stdout);
        agent_chunk(out, session_id, "recovered after garbage. ");
        reply(out, request, json!({ "stopReason": "end_turn" }));
        return;
    }

    if text.contains("hang") {
        loop {
            while let Ok(frame) = rx.try_recv() {
                if frame.get("method").and_then(|m| m.as_str()) == Some("session/cancel") {
                    *cancelled.lock() = true;
                }
            }
            if *cancelled.lock() {
                *cancelled.lock() = false;
                reply(out, request, json!({ "stopReason": "cancelled" }));
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    if text.contains("tool") {
        agent_chunk(out, session_id, "I will write a file. ");
        session_update(
            out,
            session_id,
            json!({
                "sessionUpdate": "tool_call",
                "toolCallId": "call_001",
                "title": "Write fixture file",
                "kind": "edit",
                "status": "pending",
            }),
        );
        // Permission request — the client must answer before we proceed.
        send(
            out,
            json!({
                "jsonrpc": "2.0",
                "id": 9001,
                "method": "session/request_permission",
                "params": {
                    "sessionId": session_id,
                    "toolCall": {
                        "toolCallId": "call_001",
                        "title": "Write fixture file",
                        "kind": "edit",
                        "status": "pending",
                    },
                    "options": [
                        { "optionId": "allow-once", "name": "Allow once", "kind": "allow_once" },
                        { "optionId": "reject-once", "name": "Reject", "kind": "reject_once" },
                    ],
                },
            }),
        );
        let outcome = await_response(rx, 9001, cancelled).map(|frame| {
            frame["result"]["outcome"]["optionId"]
                .as_str()
                .unwrap_or("")
                .to_string()
        });
        let allowed = outcome.as_deref() == Some("allow-once");
        if allowed {
            session_update(
                out,
                session_id,
                json!({
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": "call_001",
                    "status": "completed",
                }),
            );
            agent_chunk(out, session_id, "Done writing.");
        } else {
            session_update(
                out,
                session_id,
                json!({
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": "call_001",
                    "status": "failed",
                }),
            );
            agent_chunk(out, session_id, "Skipped: permission denied.");
        }
        reply(out, request, json!({ "stopReason": "end_turn" }));
        return;
    }

    if text.contains("readfile") {
        let path = format!("{}/fixture.txt", cwd.lock());
        send(
            out,
            json!({
                "jsonrpc": "2.0",
                "id": 9100,
                "method": "fs/read_text_file",
                "params": { "sessionId": session_id, "path": path },
            }),
        );
        let frame = await_response(rx, 9100, cancelled);
        let content = frame
            .as_ref()
            .and_then(|f| f["result"]["content"].as_str())
            .unwrap_or("<no file>");
        agent_chunk(out, session_id, &format!("File says: {content}"));
        reply(out, request, json!({ "stopReason": "end_turn" }));
        return;
    }

    agent_chunk(out, session_id, "Hello from the fake adapter. ");
    agent_chunk(out, session_id, "Turn complete.");
    reply(out, request, json!({ "stopReason": "end_turn" }));
}

fn main() {
    let stdin = std::io::stdin();
    let (tx, rx): (Sender<Value>, Receiver<Value>) = mpsc::channel();
    std::thread::spawn(move || {
        for line in stdin.lock().lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => break,
            };
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Value>(&line) {
                Ok(frame) => {
                    if tx.send(frame).is_err() {
                        break;
                    }
                }
                Err(e) => eprintln!("fake-adapter: malformed frame skipped: {e}"),
            }
        }
        // Client closed stdin — we are done.
        std::process::exit(0);
    });

    let out: Out = Arc::new(Mutex::new(std::io::stdout()));
    let cwd = Mutex::new(String::new());
    let cancelled = Mutex::new(false);
    let session_id = "fake-session-1";

    let protocol_version: u16 = std::env::var("FAKE_ACP_VERSION")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    while let Ok(frame) = rx.recv() {
        let method = frame.get("method").and_then(|m| m.as_str()).unwrap_or("");
        match method {
            "initialize" => reply(
                &out,
                &frame,
                json!({
                    "protocolVersion": protocol_version,
                    "agentCapabilities": { "loadSession": true },
                    "agentInfo": { "name": "fake-adapter", "version": "0.0.0" },
                }),
            ),
            "session/new" => {
                *cwd.lock() = frame["params"]["cwd"].as_str().unwrap_or("").to_string();
                reply(&out, &frame, json!({ "sessionId": session_id }));
            }
            "session/load" => {
                session_update(
                    &out,
                    session_id,
                    json!({
                        "sessionUpdate": "user_message_chunk",
                        "content": { "type": "text", "text": "earlier user message" },
                    }),
                );
                session_update(
                    &out,
                    session_id,
                    json!({
                        "sessionUpdate": "agent_message_chunk",
                        "content": { "type": "text", "text": "earlier agent reply" },
                    }),
                );
                reply(&out, &frame, json!({}));
            }
            "session/prompt" => {
                handle_prompt(&rx, &out, &frame, session_id, &cwd, &cancelled);
            }
            "session/cancel" => {
                *cancelled.lock() = true;
            }
            other => reply_error(&out, &frame, -32601, &format!("unknown method {other}")),
        }
    }
}
