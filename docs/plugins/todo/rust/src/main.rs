// SPDX-License-Identifier: MPL-2.0
//! Standalone epoch-2 todo showcase for Linux/macOS.
use serde_json::{Value, json};
use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

const VERSION: &str = "runyte-experimental-2";
const LIMIT: usize = 1024 * 1024;
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn wait(fd: i32, events: i16, deadline: Option<Instant>) -> Result {
    loop {
        if deadline.is_some_and(|end| Instant::now() >= end) {
            return Err("Host timed out".into());
        }
        let timeout = deadline.map_or(-1, |end| {
            end.saturating_duration_since(Instant::now())
                .as_millis()
                .min(i32::MAX as u128) as i32
        });
        let mut descriptor = libc::pollfd {
            fd,
            events,
            revents: 0,
        };
        // SAFETY: descriptor points to one initialized pollfd for this call.
        let result = unsafe { libc::poll(&mut descriptor, 1, timeout) };
        if result > 0 {
            return Ok(());
        }
        if result == 0 {
            return Err("Host timed out".into());
        }
        if io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            return Err(io::Error::last_os_error().into());
        }
    }
}

fn send(message: Value) -> Result {
    let mut data = serde_json::to_vec(&message)?;
    data.push(b'\n');
    if data.len() > LIMIT {
        return Err("Output limit".into());
    }
    let deadline = Some(Instant::now() + Duration::from_secs(2));
    let mut remaining = data.as_slice();
    while !remaining.is_empty() {
        wait(1, libc::POLLOUT, deadline)?;
        match io::stdout().write(remaining) {
            Ok(0) => return Err("Output closed".into()),
            Ok(count) => remaining = &remaining[count..],
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn finish(id: &str, code: Option<&str>) -> Result {
    send(match code {
        Some(code) => {
            json!({"type":"response", "id":id, "error": {"code":code, "message": match code {
                "stale" => "Todo list changed; invoke the action again",
                "busy" => "A todo update is pending",
                "unavailable" => "Restart this plugin after an uncertain update",
                "limit_exceeded" => "Todo limit reached",
                _ => "Todo request refused",
            }}})
        }
        None => json!({"type":"response", "id":id, "result":{"job":null}}),
    })
}

#[derive(Clone)]
struct Task {
    id: String,
    title: String,
    done: bool,
}
struct Pending {
    command: String,
    id: String,
    stage: &'static str,
    closed: bool,
    tasks: Vec<Task>,
    filtered: bool,
    added: bool,
}
struct Todo {
    phase: u8,
    tasks: Vec<Task>,
    next_task: u64,
    filtered: bool,
    view: Option<String>,
    revision: Option<String>,
    pending: Option<Pending>,
    serial: u64,
    deadline: Option<Instant>,
    uncertain: bool,
}
impl Todo {
    fn new() -> Self {
        Self {
            phase: 0,
            tasks: vec![Task {
                id: "task-1".into(),
                title: "Read the plugin guide".into(),
                done: false,
            }],
            next_task: 2,
            filtered: false,
            view: None,
            revision: None,
            pending: None,
            serial: 0,
            deadline: Some(Instant::now() + Duration::from_secs(8)),
            uncertain: false,
        }
    }
    fn model(tasks: &[Task], filtered: bool) -> Value {
        json!({"title": if filtered {"Todo · Rust · unfinished"} else {"Todo · Rust"}, "purpose":"list",
            "rows": tasks.iter().filter(|task| !filtered || !task.done).map(|task| json!({
                "id":task.id, "text":format!("{} {}", if task.done {"[x]"} else {"[ ]"}, task.title),
                "role":if task.done {"muted"} else {"ordinary"}
            })).collect::<Vec<_>>()})
    }
    fn request(&mut self, stage: &'static str, params: Value) -> Result {
        self.serial = self.serial.checked_add(1).ok_or("Request ID overflow")?;
        let pending = self.pending.as_mut().ok_or("No pending command")?;
        pending.id = format!("p:{}", self.serial);
        pending.stage = stage;
        send(json!({"type":"request", "id":pending.id, "method":stage, "params":params}))
    }
    fn invoke(&mut self, message: &Value) -> Result {
        let id = message["id"].as_str().ok_or("Missing command ID")?;
        let context = &message["params"];
        if self.pending.as_ref().is_some_and(|p| p.command == id) {
            return Err("Duplicate command".into());
        }
        if message["method"] != "command.invoke" {
            return finish(id, Some("unsupported"));
        }
        if self.pending.is_some() {
            return finish(id, Some("busy"));
        }
        if self.uncertain {
            return finish(id, Some("unavailable"));
        }
        let command = context["command"].as_str().ok_or("Missing command")?;
        let mut candidate = self.tasks.clone();
        let mut filtered = self.filtered;
        if command == "open" {
            if context["context"] != "workspace" {
                return finish(id, Some("invalid_argument"));
            }
        } else {
            if !["add", "toggle", "remove", "filter"].contains(&command) {
                return finish(id, Some("not_found"));
            }
            if self.view.is_none()
                || context["context"] != "view"
                || context["view"].as_str() != self.view.as_deref()
                || context["model_revision"].as_str() != self.revision.as_deref()
            {
                return finish(id, Some("stale"));
            }
            match command {
                "add" => {
                    let title = context["arguments"]["title"].as_str().unwrap_or("");
                    if title.trim_matches(' ').is_empty()
                        || title.len() > 256
                        || title
                            .chars()
                            .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
                    {
                        return finish(id, Some("invalid_argument"));
                    }
                    if candidate.len() >= 100 || self.next_task >= 1 << 53 {
                        return finish(id, Some("limit_exceeded"));
                    }
                    candidate.push(Task {
                        id: format!("task-{}", self.next_task),
                        title: title.into(),
                        done: false,
                    });
                }
                "filter" => filtered = !filtered,
                _ => {
                    let Some(rows) = context["rows"].as_array() else {
                        return finish(id, Some("invalid_argument"));
                    };
                    if rows.is_empty()
                        || rows.len() > 100
                        || rows.iter().enumerate().any(|(index, row)| {
                            rows[..index].contains(row)
                                || !candidate
                                    .iter()
                                    .any(|t| row.as_str() == Some(&t.id) && (!filtered || !t.done))
                        })
                    {
                        return finish(id, Some("invalid_argument"));
                    }
                    if command == "remove" {
                        candidate.retain(|t| !rows.iter().any(|r| r.as_str() == Some(&t.id)));
                    } else {
                        for task in &mut candidate {
                            if rows.iter().any(|r| r.as_str() == Some(&task.id)) {
                                task.done = !task.done;
                            }
                        }
                    }
                }
            }
        }
        let model = Self::model(&candidate, filtered);
        self.pending = Some(Pending {
            command: id.into(),
            id: String::new(),
            stage: "",
            closed: false,
            tasks: candidate,
            filtered,
            added: command == "add",
        });
        self.deadline = Some(Instant::now() + Duration::from_secs(8));
        if command == "open" {
            if self.view.is_none() {
                self.request("view.create", json!({"model":model}))
            } else {
                self.request("pane.show", json!({"invocation":id, "view":self.view}))
            }
        } else {
            self.request(
                "view.publish",
                json!({"view":self.view, "expected_revision":self.revision, "model":model}),
            )
        }
    }
    fn receive(&mut self, message: Value) -> Result {
        if !message.is_object() {
            return Err("Expected object".into());
        }
        if self.phase < 2 {
            if message["type"]
                != if self.phase == 0 {
                    "hello"
                } else {
                    "registered"
                }
                || !message["capabilities"]
                    .as_array()
                    .is_some_and(|c| c.iter().any(|v| v == "views"))
            {
                return Err("Invalid handshake".into());
            }
            if self.phase == 0 {
                if message["version"] != VERSION {
                    return Err("Wrong epoch".into());
                }
                send(
                    json!({"type":"register", "version":VERSION, "name":"Todo · Rust",
                    "required_capabilities":["views"], "optional_capabilities":[], "commands":[
                        {"name":"open", "description":"Open todo list", "context":"workspace"},
                        {"name":"add", "description":"Add a task", "context":"view", "arguments":[{"name":"title", "type":"string"}]},
                        {"name":"toggle", "description":"Toggle selected tasks", "context":"view", "primary":true},
                        {"name":"remove", "description":"Remove selected tasks", "context":"view"},
                        {"name":"filter", "description":"Toggle unfinished-only filter", "context":"view"}
                    ]}),
                )?;
            } else {
                self.deadline = None;
            }
            self.phase += 1;
            return Ok(());
        }
        match message["type"].as_str() {
            Some("request") => self.invoke(&message),
            Some("event") => {
                if message["event"] == "view.closed"
                    && self.view.is_some()
                    && message["data"]["view"].as_str() == self.view.as_deref()
                {
                    self.view = None;
                    self.revision = None;
                    if let Some(pending) = &mut self.pending {
                        pending.closed = true;
                    }
                }
                Ok(())
            }
            Some("response") => {
                let pending = self.pending.as_ref().ok_or("Unmatched response")?;
                if message["id"] != pending.id
                    || message.get("result").is_some() == message.get("error").is_some()
                {
                    return Err("Invalid response".into());
                }
                let mut code = None;
                if let Some(error) = message.get("error") {
                    let supplied = error["code"].as_str().unwrap_or("unavailable");
                    code = Some(
                        if [
                            "stale",
                            "closed",
                            "busy",
                            "context_changed",
                            "no_frontend",
                            "unavailable",
                            "limit_exceeded",
                            "timeout",
                            "outcome_unknown",
                        ]
                        .contains(&supplied)
                        {
                            supplied
                        } else {
                            "unavailable"
                        },
                    );
                    if ["timeout", "outcome_unknown"].contains(&supplied)
                        && pending.stage != "pane.show"
                    {
                        self.uncertain = true;
                    }
                } else if pending.closed {
                    code = Some("stale");
                } else if pending.stage != "pane.show" {
                    let result = &message["result"];
                    let view = result["view"].as_str().unwrap_or("");
                    let revision = result["revision"].as_str().unwrap_or("");
                    if view.is_empty()
                        || view.len() > 256
                        || revision.is_empty()
                        || revision.len() > 256
                        || (pending.stage == "view.publish"
                            && (Some(view) != self.view.as_deref()
                                || Some(revision) == self.revision.as_deref()))
                    {
                        self.uncertain = true;
                        code = Some("outcome_unknown");
                    } else if pending.stage == "view.create" {
                        self.view = Some(view.into());
                        self.revision = Some(revision.into());
                        let params = json!({"invocation":pending.command, "view":view});
                        return self.request("pane.show", params);
                    } else {
                        self.tasks = pending.tasks.clone();
                        self.filtered = pending.filtered;
                        self.next_task += u64::from(pending.added);
                        self.revision = Some(revision.into());
                    }
                }
                finish(
                    &self.pending.as_ref().ok_or("Missing pending")?.command,
                    code,
                )?;
                self.pending = None;
                self.deadline = None;
                Ok(())
            }
            _ => Err("Unexpected message".into()),
        }
    }
}

fn run() -> Result {
    // SAFETY: fcntl operates on our stdout descriptor, with integer flag arguments.
    unsafe {
        let flags = libc::fcntl(1, libc::F_GETFL);
        if flags < 0 || libc::fcntl(1, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            return Err(io::Error::last_os_error().into());
        }
    }
    let mut app = Todo::new();
    let mut buffer = Vec::new();
    let mut chunk = [0; 8192];
    loop {
        wait(0, libc::POLLIN, app.deadline)?;
        let count = io::stdin().read(&mut chunk)?;
        if count == 0 {
            return if buffer.is_empty() {
                Ok(())
            } else {
                Err("Truncated frame".into())
            };
        }
        for byte in &chunk[..count] {
            buffer.push(*byte);
            if buffer.len() > LIMIT {
                return Err("Frame limit".into());
            }
            if *byte == b'\n' {
                app.receive(serde_json::from_slice(&buffer)?)?;
                buffer.clear();
            } else if buffer.len() == LIMIT {
                return Err("Frame limit".into());
            }
        }
    }
}
fn main() {
    if run().is_err() {
        std::process::exit(1);
    }
}
