//! One `claude` process in stream-json mode, and the conversation with it.
//!
//! Line-delimited JSON both ways. Unlike ACP and codex this is NOT JSON-RPC:
//! ordinary traffic is typed frames with no ids at all, and only the control
//! channel correlates by `request_id`. So there is no generic `request` here —
//! a caller either sends a frame or sends a control request and waits for the
//! matching `control_response`.

use std::path::PathBuf;
use std::process::Stdio;

use farcooler_agent_core::event::PromptImage;
use farcooler_agent_core::stderr::{GRACE, Head, reason_in};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::sync::{mpsc, oneshot};

#[derive(Debug, thiserror::Error)]
pub enum ClaudeError {
    #[error("could not start claude")]
    Spawn,
    #[error("claude closed its connection")]
    Closed,
    /// Closed during startup, with the reason it left on stderr (ov-414).
    ///
    /// Its own variant so `Closed` keeps meaning what `adapter_is_gone` takes
    /// it to mean; it reaches a caller as a refusal, in the agent's words.
    #[error("claude closed its connection: {0}")]
    Died(String),
    #[error("claude refused: {0}")]
    Refused(String),
}

/// A frame from the CLI.
#[derive(Debug)]
pub enum Incoming {
    /// An ordinary typed frame: assistant, user, result, system…
    Frame(serde_json::Value),
    /// The CLI is asking us something and is blocked until we answer —
    /// `can_use_tool` above all. Dropping these hangs the turn.
    Control { request_id: String, request: serde_json::Value },
}

fn classify(value: serde_json::Value) -> Option<Incoming> {
    match value["type"].as_str() {
        Some("control_request") => Some(Incoming::Control {
            request_id: value["request_id"].as_str().unwrap_or_default().to_string(),
            request: value["request"].clone(),
        }),
        Some(_) => Some(Incoming::Frame(value)),
        None => None,
    }
}

/// A prompt's `content` array: the question, then the pictures.
///
/// The image block is the Anthropic SDK's — `SDKUserMessage.message` is typed
/// `MessageParam`, which `vendor/claude-sdk.d.ts` only IMPORTS from
/// `@anthropic-ai/sdk`, so the pinned declarations cannot be read for this
/// shape and an earlier version dropped every picture on those grounds. The
/// shape below was measured against a live claude 2.1.226 instead: a 64x64 PNG
/// sent this way beside "What color is this image?" came back "Red".
///
/// Worth sending even where it cannot be checked, because the two failures are
/// not equally bad. A malformed block fails LOUDLY — 2.1.226 answers "an image
/// in the conversation could not be processed and was removed", which is a
/// thing a person can see and report. A dropped block fails silently, and the
/// user is left arguing with an agent about a picture it was never shown.
///
/// The text goes first and unconditionally, so a rejected image can only ever
/// cost the picture. Losing the picture is bad; losing the question with it
/// would be worse.
fn user_content(text: &str, images: &[PromptImage]) -> serde_json::Value {
    let mut content = vec![serde_json::json!({ "type": "text", "text": text })];
    for image in images {
        content.push(serde_json::json!({
            "type": "image",
            "source": {
                "type": "base64",
                // `media_type` is required and cannot be guessed from base64,
                // so an image that arrived without one is sent as PNG — the
                // format Far Cooler's own screenshot path produces. A wrong
                // guess costs the picture, which is what omitting it costs too.
                "media_type": if image.mime.is_empty() { "image/png" } else { &image.mime },
                "data": image.base64,
            },
        }));
    }
    serde_json::Value::Array(content)
}

/// The `PermissionResult` that answers a `can_use_tool`.
///
/// `behavior`, not `decision`, and an `allow` MUST carry `updatedInput` or the
/// CLI rejects it — a documented contract the SDK's own changelog records
/// getting wrong.
fn permission_response(allow: bool, input: serde_json::Value) -> serde_json::Value {
    if allow {
        serde_json::json!({ "behavior": "allow", "updatedInput": input })
    } else {
        serde_json::json!({ "behavior": "deny", "message": "Denied by the user" })
    }
}

/// The write half, plus the process handle.
pub struct ClaudeWriter {
    stdin: ChildStdin,
    next_control_id: u64,
    pub worktree: PathBuf,
    _child: Child,
}

impl ClaudeWriter {
    async fn write(&mut self, value: serde_json::Value) -> Result<(), ClaudeError> {
        self.stdin
            .write_all(format!("{value}\n").as_bytes())
            .await
            .map_err(|_| ClaudeError::Closed)?;
        self.stdin.flush().await.map_err(|_| ClaudeError::Closed)
    }

    /// Send a prompt as a user message, with any pictures attached to it.
    pub async fn send_user(
        &mut self,
        text: &str,
        images: &[PromptImage],
    ) -> Result<(), ClaudeError> {
        self.write(serde_json::json!({
            "type": "user",
            "message": { "role": "user", "content": user_content(text, images) },
            "parent_tool_use_id": null,
        }))
        .await
    }

    /// Answer a control request this client cannot do, saying so.
    ///
    /// A `subtype: "error"` response is how the CLI hears "no": a bare success
    /// to an `elicitation` or `hook_callback` told it the user answered the
    /// dialog and the hook ran, when neither happened.
    pub async fn respond_error(&mut self, request_id: &str, message: &str) -> Result<(), ClaudeError> {
        self.write(serde_json::json!({
            "type": "control_response",
            "response": { "subtype": "error", "request_id": request_id, "error": message },
        }))
        .await
    }

    /// Send a control request without waiting for its answer.
    ///
    /// Not waited on for the same reason `turn/start` is not: the turn cannot
    /// finish while nobody is answering the CLI's own requests, so blocking
    /// here would deadlock.
    ///
    /// `fields` are merged into the request beside its subtype, because the
    /// setters carry their value there — `set_model` wants `{ model }`, not a
    /// subtype with the value glued onto it. An earlier version built
    /// `"set_model:opus"` as the subtype, which the CLI does not recognize and
    /// ignores in silence: the picker moved and the model did not.
    pub async fn control(
        &mut self,
        subtype: &str,
        fields: serde_json::Value,
    ) -> Result<String, ClaudeError> {
        let id = format!("fc-{}", self.next_control_id);
        self.next_control_id += 1;
        let mut request = serde_json::json!({ "subtype": subtype });
        if let (Some(request), Some(fields)) = (request.as_object_mut(), fields.as_object()) {
            for (key, value) in fields {
                request.insert(key.clone(), value.clone());
            }
        }
        self.write(serde_json::json!({
            "type": "control_request",
            "request_id": id,
            "request": request,
        }))
        .await?;
        Ok(id)
    }

    /// Answer a `can_use_tool` request.
    ///
    /// `input` is the tool input the request itself carried, echoed back — see
    /// `permission_response` for why an allow cannot be sent without it.
    pub async fn answer_permission(
        &mut self,
        request_id: &str,
        allow: bool,
        input: serde_json::Value,
    ) -> Result<(), ClaudeError> {
        let response = permission_response(allow, input);
        self.write(serde_json::json!({
            "type": "control_response",
            "response": { "subtype": "success", "request_id": request_id, "response": response },
        }))
        .await
    }
}

/// A connection during startup, before the read half is split off.
pub struct ClaudeConnection {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    child: Child,
    /// What the process said on stderr, once it ends. Read only to explain a
    /// death during startup; after `split` nothing reads it, and the task
    /// that fills it ends with the process.
    stderr: Option<oneshot::Receiver<String>>,
    pub worktree: PathBuf,
}

/// Read a child's stderr to its end on a task of its own, keeping the head.
fn keep_stderr(mut stderr: ChildStderr) -> oneshot::Receiver<String> {
    let (tx, rx) = oneshot::channel();
    tokio::spawn(async move {
        let mut head = Head::default();
        let mut chunk = [0u8; 4096];
        while let Ok(n) = stderr.read(&mut chunk).await {
            if n == 0 {
                break;
            }
            head.push(&chunk[..n]);
        }
        let _ = tx.send(head.text());
    });
    rx
}

impl ClaudeConnection {
    /// The error for a pipe that has closed during startup: claude's own
    /// stderr reason when it left one, waited for up to `GRACE`.
    async fn closed(&mut self) -> ClaudeError {
        let Some(stderr) = self.stderr.take() else { return ClaudeError::Closed };
        match tokio::time::timeout(GRACE, stderr).await {
            Ok(Ok(text)) => reason_in(&text).map(ClaudeError::Died).unwrap_or(ClaudeError::Closed),
            _ => ClaudeError::Closed,
        }
    }

    pub async fn spawn(
        program: &std::path::Path,
        args: &[String],
        env: &std::collections::BTreeMap<String, String>,
        worktree: PathBuf,
    ) -> Result<Self, ClaudeError> {
        let mut command = Command::new(program);
        command
            .args(args)
            .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .current_dir(&worktree)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Piped, not discarded: a process that dies during startup says why
            // on stderr and nowhere else (ov-410, ov-414).
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        // Nested inside another Claude Code session the CLI neither answers nor
        // exits — the documented cause of `Status::AdapterSilent`.
        for var in crate::handshake::NESTING_VARS {
            command.env_remove(var);
        }

        let mut child = command.spawn().map_err(|_| ClaudeError::Spawn)?;
        let stdin = child.stdin.take().ok_or(ClaudeError::Spawn)?;
        let stdout = child.stdout.take().ok_or(ClaudeError::Spawn)?;
        let stderr = child.stderr.take().map(keep_stderr);
        Ok(ClaudeConnection { stdin, stdout: BufReader::new(stdout), child, stderr, worktree })
    }

    async fn write(&mut self, value: serde_json::Value) -> Result<(), ClaudeError> {
        let line = format!("{value}\n");
        let sent = match self.stdin.write_all(line.as_bytes()).await {
            Ok(()) => self.stdin.flush().await,
            Err(e) => Err(e),
        };
        match sent {
            Ok(()) => Ok(()),
            // A broken pipe is a death, and the reason is on stderr.
            Err(_) => Err(self.closed().await),
        }
    }

    /// Initialize, and read until the CLI answers.
    ///
    /// Waits for the `control_response` to our own request and NOTHING else.
    ///
    /// It waited for the `system: init` frame first, which deadlocks: measured
    /// against 2.1.226, `init` does not arrive until a prompt is sent, so a
    /// session that waits for it before allowing a prompt waits forever. In 25
    /// seconds with no prompt the CLI sends the user's hook frames and the
    /// control response, and no `init` at all.
    ///
    /// The control response carries what a session needs anyway — commands,
    /// agents, models, account — and the session id comes from `--session-id`,
    /// which the caller passes because Far Cooler assigned it at launch.
    ///
    /// Frames seen on the way are returned: the user's own hooks fire first,
    /// and dropping them unread would be a silent choice rather than a
    /// deliberate one.
    pub async fn initialize(
        &mut self,
    ) -> Result<(serde_json::Value, Vec<serde_json::Value>), ClaudeError> {
        self.write(serde_json::json!({
            "type": "control_request",
            "request_id": "fc-init",
            "request": { "subtype": "initialize" },
        }))
        .await?;

        let mut seen = Vec::new();
        loop {
            let mut line = String::new();
            match self.stdout.read_line(&mut line).await {
                Ok(n) if n > 0 => {}
                _ => return Err(self.closed().await),
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
                continue;
            };
            if value["type"] == "control_response" {
                // An error here is the CLI refusing to start at all, worth
                // reporting in its own words rather than as a timeout later.
                if value["response"]["subtype"] == "error" {
                    let message = value["response"]["error"]
                        .as_str()
                        .unwrap_or("claude refused to initialize");
                    return Err(ClaudeError::Refused(message.to_string()));
                }
                return Ok((value["response"]["response"].clone(), seen));
            }
            seen.push(value);
        }
    }

    /// Split into a writer and a receiver, with reading moved into a task.
    pub fn split(self) -> (ClaudeWriter, mpsc::UnboundedReceiver<Incoming>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut stdout = self.stdout;
        tokio::spawn(async move {
            loop {
                let mut line = String::new();
                match stdout.read_line(&mut line).await {
                    Ok(0) | Err(_) => return,
                    Ok(_) => {}
                }
                let Ok(value) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
                    continue;
                };
                if let Some(incoming) = classify(value)
                    && tx.send(incoming).is_err()
                {
                    return;
                }
            }
        });
        (
            ClaudeWriter {
                stdin: self.stdin,
                next_control_id: 1,
                worktree: self.worktree,
                _child: self.child,
            },
            rx,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_control_request_is_told_apart_from_an_ordinary_frame() {
        // Dropping a control request hangs the turn: the CLI is blocked until
        // it is answered.
        let control = serde_json::json!({
            "type": "control_request", "request_id": "7",
            "request": { "subtype": "can_use_tool" }
        });
        assert!(matches!(
            classify(control),
            Some(Incoming::Control { request_id, .. }) if request_id == "7"
        ));

        let assistant = serde_json::json!({ "type": "assistant" });
        assert!(matches!(classify(assistant), Some(Incoming::Frame(_))));
    }

    #[test]
    fn a_frame_with_no_type_is_dropped_rather_than_guessed_at() {
        assert!(classify(serde_json::json!({ "hello": 1 })).is_none());
    }

    #[test]
    fn a_prompt_carries_its_picture_as_well_as_its_question() {
        // Both blocks in the one `content` array, which is what a live 2.1.226
        // answered "Red" to. Images used to be dropped here on the grounds that
        // the source shape was unverified.
        let content = user_content(
            "what color is this",
            &[PromptImage { mime: "image/png".into(), base64: "QUJD".into() }],
        );
        assert_eq!(content.as_array().map(Vec::len), Some(2), "{content}");
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "what color is this");
        assert_eq!(content[1]["type"], "image");
        assert_eq!(content[1]["source"]["type"], "base64");
        assert_eq!(content[1]["source"]["media_type"], "image/png");
        assert_eq!(content[1]["source"]["data"], "QUJD");
    }

    #[test]
    fn a_picture_can_never_cost_the_question_it_came_with() {
        // The text goes first and unconditionally, so a block the API refuses
        // takes the picture and nothing else with it.
        let content = user_content("still ask me", &[]);
        assert_eq!(content.as_array().map(Vec::len), Some(1));
        assert_eq!(content[0]["text"], "still ask me");
    }

    #[test]
    fn an_allow_echoes_the_tools_input_because_the_cli_rejects_one_without() {
        let response = permission_response(true, serde_json::json!({ "command": "ls" }));
        assert_eq!(response["behavior"], "allow");
        assert_eq!(response["updatedInput"]["command"], "ls");

        // A deny carries a reason instead, and must NOT carry an input.
        let denied = permission_response(false, serde_json::json!({ "command": "ls" }));
        assert_eq!(denied["behavior"], "deny");
        assert!(denied["updatedInput"].is_null(), "{denied}");
    }

    /// A stand-in claude that does what npm's launcher did on Oct 7 (ov-410):
    /// takes the first request, prints node's report on stderr, and exits.
    async fn spawn_a_dying_claude() -> (ClaudeConnection, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir()
            .join(format!("farcooler-claude-conn-{}-dying", std::process::id()));
        std::fs::write(
            &path,
            concat!(
                "#!/bin/sh\nread request\n",
                "echo 'file:///x/claude.js:107' >&2\n",
                "echo 'Error: Missing optional dependency @x/claude-linux-x64' >&2\n",
                "echo '    at find (file:///x/claude.js:107:9)' >&2\n",
                "exit 1\n",
            ),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let conn = ClaudeConnection::spawn(&path, &[], &Default::default(), std::env::temp_dir())
            .await
            .unwrap();
        (conn, path)
    }

    #[tokio::test]
    async fn a_claude_that_dies_during_startup_is_reported_in_its_own_words() {
        let (mut conn, path) = spawn_a_dying_claude().await;
        let result = conn.initialize().await;
        let _ = std::fs::remove_file(&path);
        let Err(ClaudeError::Died(reason)) = result else {
            panic!("a death during startup must carry its reason, got {result:?}")
        };
        assert_eq!(reason, "Error: Missing optional dependency @x/claude-linux-x64");
    }

    #[tokio::test]
    async fn a_claude_that_dies_silently_is_just_closed() {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir()
            .join(format!("farcooler-claude-conn-{}-mute", std::process::id()));
        std::fs::write(&path, "#!/bin/sh\nread request\nexit 1\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut conn =
            ClaudeConnection::spawn(&path, &[], &Default::default(), std::env::temp_dir()).await.unwrap();
        let result = conn.initialize().await;
        let _ = std::fs::remove_file(&path);
        assert!(matches!(result, Err(ClaudeError::Closed)), "{result:?}");
    }
}
