#![allow(clippy::expect_used)]

use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, Command, ExitStatus, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

use nix::{
    sys::signal::{Signal, kill},
    unistd::Pid,
};
use serde_json::{Value, json};
use tempfile::TempDir;

const BRR: &str = env!("CARGO_BIN_EXE_brr");
const READ_TIMEOUT: Duration = Duration::from_secs(20);
const ENDED_TIMEOUT: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(100);
const PROTOCOL_VERSION: &str = "2025-06-18";
const XDG_VARS: [&str; 4] = [
    "XDG_RUNTIME_DIR",
    "XDG_DATA_HOME",
    "XDG_CONFIG_HOME",
    "XDG_STATE_HOME",
];

// A scratch daemon: its own XDG dirs, a free port, stopped on drop.
struct Scratch {
    dirs: Vec<TempDir>,
}

impl Scratch {
    fn start() -> Scratch {
        let scratch = Scratch {
            dirs: XDG_VARS
                .iter()
                .map(|_| tempfile::tempdir().expect("tempdir"))
                .collect(),
        };
        let started = scratch.brr(&["daemon", "start"]).output().expect("spawn");
        assert!(started.status.success(), "daemon start: {started:?}");

        scratch
    }

    fn brr(&self, args: &[&str]) -> Command {
        let mut command = Command::new(BRR);
        command
            .args(args)
            .env("BRR_SERVER__BIND", "127.0.0.1:0")
            .env("BRR_VAULT__KEY_SOURCE", "file");
        for (var, dir) in XDG_VARS.iter().zip(&self.dirs) {
            command.env(var, dir.path());
        }

        command
    }

    fn mcp(&self, args: &[&str]) -> Session {
        let mut child = self
            .brr(&[&["mcp"], args].concat())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn brr mcp");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = child.stdout.take().expect("stdout");

        let (lines, received) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if lines.send(line).is_err() {
                    return;
                }
            }
        });

        let mut session = Session {
            child,
            stdin: Some(stdin),
            lines: received,
            next_id: 0,
        };
        session.request(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "test", "version": "0"}
            }),
        );
        session.notify("notifications/initialized");

        session
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = self.brr(&["daemon", "stop"]).output();
    }
}

struct Session {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
    next_id: u64,
}

impl Session {
    fn send(&mut self, message: &Value) {
        let stdin = self.stdin.as_mut().expect("stdin open");
        writeln!(stdin, "{message}").expect("write");
        stdin.flush().expect("flush");
    }

    fn notify(&mut self, method: &str) {
        self.send(&json!({"jsonrpc": "2.0", "method": method}));
    }

    // Every stdout line must be JSON-RPC; returns the `result` of the matching response.
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));

        loop {
            let line = self.lines.recv_timeout(READ_TIMEOUT).expect("response");
            let message: Value = serde_json::from_str(&line).expect("stdout line is JSON");
            assert_eq!(message["jsonrpc"], "2.0", "{line}");

            if message["id"] == id {
                return message["result"].clone();
            }
        }
    }

    fn close(mut self) -> ExitStatus {
        drop(self.stdin.take());
        let status = self.child.wait().expect("wait");

        self.assert_remaining_lines_are_jsonrpc();

        status
    }

    fn terminate(mut self) -> ExitStatus {
        let pid = i32::try_from(self.child.id()).expect("pid");
        kill(Pid::from_raw(pid), Signal::SIGTERM).expect("signal");
        let status = self.child.wait().expect("wait");

        self.assert_remaining_lines_are_jsonrpc();

        status
    }

    // The reader thread ends at EOF, which the child's exit guarantees.
    fn assert_remaining_lines_are_jsonrpc(&self) {
        for line in self.lines.iter() {
            let message: Value = serde_json::from_str(&line).expect("stdout line is JSON");
            assert_eq!(message["jsonrpc"], "2.0", "{line}");
        }
    }
}

fn tool_names(tools: &Value) -> Vec<String> {
    let mut names: Vec<String> = tools["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .map(|tool| tool["name"].as_str().expect("name").to_string())
        .collect();
    names.sort();

    names
}

#[test]
fn tools_list_system() {
    let scratch = Scratch::start();
    let mut session = scratch.mcp(&["--toolsets", "system"]);

    let tools = session.request("tools/list", json!({}));

    assert_eq!(
        tool_names(&tools),
        [
            "events_list",
            "events_watch",
            "host_check",
            "host_create",
            "host_delete",
            "host_get",
            "host_list",
            "host_update",
            "settings_get",
            "settings_set",
            "system_doctor",
            "system_info"
        ]
    );
    for tool in tools["tools"].as_array().expect("tools") {
        assert!(tool["inputSchema"]["properties"].get("client_id").is_none());
    }
    assert!(session.close().success());
}

#[test]
fn call_forwarding() {
    let scratch = Scratch::start();
    let mut session = scratch.mcp(&["--toolsets", "system,profiles"]);

    let settings = session.request(
        "tools/call",
        json!({"name": "settings_get", "arguments": {}}),
    );
    assert!(settings["structuredContent"]["settings"].is_object());

    let missing = session.request(
        "tools/call",
        json!({"name": "profile_get", "arguments": {"profile": "nope"}}),
    );
    assert_eq!(missing["isError"], true);
    // The daemon puts the typed error in the text block (its second line), not structuredContent.
    let text = missing["content"][0]["text"].as_str().expect("text");
    let body: Value = serde_json::from_str(text.lines().nth(1).expect("body line")).expect("body");
    assert_eq!(body["code"], "NotFound");
    assert!(session.close().success());
}

#[test]
fn sigterm_exits_zero() {
    let scratch = Scratch::start();
    let session = scratch.mcp(&[]);

    assert!(session.terminate().success());
}

#[test]
fn exit_ends_client() {
    let scratch = Scratch::start();
    let session = scratch.mcp(&[]);

    assert!(session.close().success());

    let deadline = Instant::now() + ENDED_TIMEOUT;
    while Instant::now() < deadline {
        if client_disconnected(&scratch) {
            return;
        }
        thread::sleep(POLL);
    }
    panic!("no client.ended with reason disconnected for the mcp client");
}

// A client.ended(disconnected) whose client_id was created with kind mcp.
fn client_disconnected(scratch: &Scratch) -> bool {
    let events = scratch
        .brr(&["events", "--kinds", "client.created,client.ended", "--json"])
        .output()
        .expect("events");
    let output: Value = serde_json::from_slice(&events.stdout).unwrap_or_default();
    let events = output["events"].as_array().map_or(&[][..], Vec::as_slice);

    let mcp_clients: Vec<&Value> = events
        .iter()
        .filter(|event| event["kind"] == "client.created" && event["data"]["kind"] == "mcp")
        .map(|event| &event["data"]["client_id"])
        .collect();

    events.iter().any(|event| {
        event["kind"] == "client.ended"
            && event["data"]["reason"] == "disconnected"
            && mcp_clients.contains(&&event["data"]["client_id"])
    })
}
