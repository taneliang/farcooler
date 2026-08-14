//! One ACP session: the conversation, the capability answers, and the events.
//!
//! Every function here speaks ACP — `initialize`, `session/load`, `session/new`,
//! and the mode and model parsers that read their results. That is why the whole
//! file lives in this crate rather than only the part that turned into
//! `AcpBackend`: none of it was ever neutral.

use std::path::{Path, PathBuf};

use tokio::sync::mpsc;

use crate::conn::{AcpConnection, AcpError, AcpWriter, Incoming};
use crate::normalize::{relativize, update_to_events};
use crate::wire::Rpc;
use farcooler_agent_core::event::{
    AgentChoice, AgentEvent, AgentGapReason, ConfigOption, Diff, EndReason, PermissionOption,
    PromptImage, ToolStatus,
};
use farcooler_agent_core::backend::BackendKind;
use farcooler_agent_core::fs_guard::confine;

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error(transparent)]
    Acp(#[from] AcpError),
    #[error("refused: the path is outside the workspace worktree")]
    Refused,
    #[error("the agent did not accept the session")]
    Rejected,
}

/// Perform a confined write and describe it as a diff.
pub fn handle_fs_write(
    worktree: &Path,
    requested: &str,
    contents: &str,
) -> Result<AgentEvent, SessionError> {
    let path = confine(worktree, Path::new(requested)).map_err(|_| SessionError::Refused)?;
    let old_text = std::fs::read_to_string(&path).ok();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&path, contents).map_err(|_| SessionError::Refused)?;
    Ok(AgentEvent::ToolUpdate {
        id: path.display().to_string(),
        status: ToolStatus::Completed,
        title: None,
        locations: vec![path.display().to_string()],
        content: None,
        diff: Some(Diff {
            path: path.display().to_string(),
            old_text,
            new_text: contents.to_string(),
        }),
        // An `fs/write_text_file` request carries no attribution, so a write
        // made by a subagent is indistinguishable from one made by the agent
        // itself. Claiming a parent we were not told about would be a guess.
        parent: None,
        subagent: None,
    })
}

/// Read a confined file for the agent.
pub fn handle_fs_read(worktree: &Path, requested: &str) -> Result<String, SessionError> {
    let path = confine(worktree, Path::new(requested)).map_err(|_| SessionError::Refused)?;
    std::fs::read_to_string(&path).map_err(|_| SessionError::Refused)
}

/// A `session/request_permission` as the event that blocks a fleet row.
pub fn permission_event(request_id: &str, params: &serde_json::Value) -> AgentEvent {
    let options = params["options"]
        .as_array()
        .map(|opts| {
            opts.iter()
                .map(|o| PermissionOption {
                    id: o["optionId"].as_str().unwrap_or_default().to_string(),
                    name: o["name"].as_str().unwrap_or_default().to_string(),
                    kind: o["kind"].as_str().unwrap_or_default().to_string(),
                })
                .collect()
        })
        .unwrap_or_default();
    AgentEvent::Permission {
        id: request_id.to_string(),
        tool_call: params["toolCall"]["toolCallId"].as_str().unwrap_or_default().to_string(),
        options,
    }
}

/// What a reconnect emits when the agent never even offered `session/load`.
///
/// Only reachable when `initialize` did not set `agentCapabilities.loadSession`
/// — the one case the name is still literally true for. A failed ATTEMPT at
/// `session/load` is a different thing and goes through `load_failed_event`.
pub fn load_unsupported_event() -> AgentEvent {
    AgentEvent::Gap { reason: AgentGapReason::LoadUnsupported }
}

/// What a reconnect emits when `session/load` was attempted and refused.
///
/// Splits `detail` into the benign case and the genuine one, because the two
/// read completely differently to a person: "there is nothing to restore" is
/// not a failure, and folding it into "this broke, here is why" would make the
/// ordinary, expected path — a claude or codex terminal switched to chat
/// before its first turn — look like something went wrong when nothing did.
pub fn load_failed_event(detail: &str) -> AgentEvent {
    if is_missing_transcript(detail) {
        AgentEvent::Gap { reason: AgentGapReason::LoadEmpty }
    } else {
        AgentEvent::Gap { reason: AgentGapReason::LoadFailed { detail: detail.to_string() } }
    }
}

/// Whether a `session/load` refusal means "nothing recorded for this id",
/// rather than a genuine failure.
///
/// Matched by phrase, not by anything structural, because there is nothing
/// structural to use: JSON-RPC gives every `session/load` refusal the same
/// generic error code regardless of cause (`-32603`, "Internal error" — seen
/// from BOTH adapters below, for this case and, presumably, for others), so
/// the adapter's own wording is the only signal that exists. `code` was
/// considered and rejected as a discriminator for exactly that reason: it
/// does not vary with what went wrong, only `message`/`data.details` do.
///
/// Two adapters have been read directly, not guessed at:
/// - `claude-agent-acp` answers exactly `"Session not found"` — pinned in
///   `acp::conn`'s `an_error_reply_is_returned_rather_than_waited_on_forever`.
/// - `@agentclientprotocol/codex-acp`, probed on 2026-08-03 by sending
///   `session/load` for an unknown id with stdin held open, answered
///   `{"code":-32603,"message":"Internal error","data":{"details":"no
///   rollout found for thread id 00000000-0000-7000-8000-000000000000"}}` —
///   pinned in `acp::conn`'s `an_error_with_data_details_folds_the_details_in`.
///   `AcpConnection::request` folds `data.details` into the message it
///   returns, so `detail` here reads `"Internal error: no rollout found for
///   thread id …"`.
///
/// Matched by phrase rather than by adapter identity: an id this build does
/// not recognize still falls through to `LoadFailed` and shows its raw
/// message, which is the right default on both sides of the split — erring
/// toward showing a message the user did not need beats erring toward hiding
/// one they did.
fn is_missing_transcript(detail: &str) -> bool {
    let lower = detail.to_lowercase();
    lower.contains("not found") || lower.contains("no rollout found")
}

/// The modes an agent offers, from a `session/new` or `session/load` result.
///
/// Id AND name. An id is what the protocol needs; a name is what a person
/// reads, and a picker offering `acceptEdits` is offering wire vocabulary.
pub fn modes_from(session_result: &serde_json::Value) -> Vec<AgentChoice> {
    choices(&session_result["modes"]["availableModes"], "id")
}

/// The models an agent offers. Same shape, different key for the identifier —
/// the adapter names it `modelId` here and `id` for modes.
pub fn models_from(session_result: &serde_json::Value) -> Vec<AgentChoice> {
    choices(&session_result["models"]["availableModels"], "modelId")
}

/// Every selector the agent advertises.
///
/// The stabilised generic form. Older adapters send only `modes`/`models`, so
/// `config_options_from` synthesises equivalents from those — a client that
/// renders this list alone therefore works against both without knowing which
/// it is talking to.
pub fn config_options_from(session_result: &serde_json::Value) -> Vec<ConfigOption> {
    if let Some(list) = session_result["configOptions"].as_array() {
        return list
            .iter()
            .filter_map(|o| {
                Some(ConfigOption {
                    id: o["id"].as_str()?.to_string(),
                    name: o["name"].as_str().unwrap_or_default().to_string(),
                    description: o["description"].as_str().unwrap_or_default().to_string(),
                    category: o["category"].as_str().unwrap_or_default().to_string(),
                    kind: o["type"].as_str().unwrap_or("select").to_string(),
                    current_value: match &o["currentValue"] {
                        serde_json::Value::String(s) => s.clone(),
                        serde_json::Value::Bool(b) => b.to_string(),
                        other => other.to_string(),
                    },
                    options: choices(&o["options"], "value"),
                })
            })
            .collect();
    }

    // Fallback for an adapter that predates config options.
    let mut out = Vec::new();
    let modes = modes_from(session_result);
    if !modes.is_empty() {
        out.push(ConfigOption {
            id: "mode".into(),
            name: "Mode".into(),
            description: String::new(),
            category: "mode".into(),
            kind: "select".into(),
            current_value: session_result["modes"]["currentModeId"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            options: modes,
        });
    }
    let models = models_from(session_result);
    if !models.is_empty() {
        out.push(ConfigOption {
            id: "model".into(),
            name: "Model".into(),
            description: String::new(),
            category: "model".into(),
            kind: "select".into(),
            current_value: session_result["models"]["currentModelId"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            options: models,
        });
    }
    out
}

fn choices(list: &serde_json::Value, id_key: &str) -> Vec<AgentChoice> {
    list.as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|v| {
                    let id = v[id_key].as_str()?.to_string();
                    // Falls back to the id rather than showing an empty row:
                    // an unnamed option a user cannot see is worse than a
                    // technical one they can.
                    let name = v["name"].as_str().unwrap_or(&id).to_string();
                    let description = v["description"].as_str().unwrap_or_default().to_string();
                    Some(AgentChoice { id, name, description })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A turn's `stopReason`, as the reason it ended.
///
/// An unrecognized reason is `EndTurn` rather than an error: the turn IS over
/// whatever the adapter chose to call it, and refusing to admit that would
/// leave the row stuck on `Working` forever.
pub fn end_reason(stop_reason: &str) -> EndReason {
    match stop_reason {
        "cancelled" => EndReason::Cancelled,
        "refusal" => EndReason::Refusal,
        "max_tokens" | "max_tokens_reached" => EndReason::MaxTokens,
        _ => EndReason::EndTurn,
    }
}

pub struct AgentSession {
    conn: AcpConnection,
    pub session_id: String,
    /// Whether this adapter declared `agentCapabilities.loadSession` at
    /// `initialize`. Kept rather than left a local, because it is what
    /// `Capabilities::replay` reports and a backend cannot answer that
    /// honestly without it.
    pub can_load: bool,
    pub available_modes: Vec<String>,
    pub available_commands: Vec<String>,
    /// The id of the `session/prompt` we are waiting to see answered.
    ///
    /// Without this a response cannot be told apart from any other, and the
    /// one frame that reports a turn's end goes unrecognized.
    pending_prompt: Option<u64>,
}

impl AgentSession {
    /// Initialize, then either load an existing session or create a new one.
    ///
    /// `resume` carries the session id from SQLite. Its absence means this is a
    /// terminal that has never been in agent pane mode.
    pub async fn start(
        mut conn: AcpConnection,
        resume: Option<String>,
    ) -> Result<(Self, Vec<AgentEvent>), SessionError> {
        let init = conn
            .request(
                "initialize",
                serde_json::json!({
                    "protocolVersion": 1,
                    "clientCapabilities": {
                        "fs": { "readTextFile": true, "writeTextFile": true },
                        // Opt in to nested subagent transcripts. Without it the
                        // adapter flattens a subagent's work into the parent's
                        // stream and the structure is simply gone.
                        "_meta": { "subagent-transcript": true }
                    }
                }),
            )
            .await?;
        let can_load = init["agentCapabilities"]["loadSession"].as_bool().unwrap_or(false);

        let mut prelude = Vec::new();
        let cwd = conn.worktree.display().to_string();

        // The session result, not the initialize result, is where the modes
        // are. `initialize` advertises `loadSession` and the prompt
        // capabilities and nothing about modes at all — reading them from there
        // yielded an empty list forever, so the mode switcher had nothing to
        // offer. Measured in the Gate 1 spike.
        let (session_id, session_result) = match resume {
            Some(id) if can_load => {
                let loaded = conn
                    .request(
                        "session/load",
                        serde_json::json!({ "sessionId": id, "cwd": cwd, "mcpServers": [] }),
                    )
                    .await;
                match loaded {
                    Ok(result) => (id, result),
                    // A declared session id whose transcript does not exist yet
                    // is the COMMON case, not an exotic one: Far Cooler hands
                    // every claude terminal a `--session-id` at launch, and a
                    // terminal switched to agent mode before anyone typed into
                    // it has no transcript to load. Failing here killed agent
                    // mode outright for exactly the path the product steers
                    // people down.
                    //
                    // Starting fresh is right, and the gap is the honest part:
                    // whatever the old id referred to is not being shown. But
                    // `load_failed_event` is what decides whether that gap
                    // reads as "nothing to restore" or "this broke" — this
                    // arm is NOT the agent refusing to implement `session/load`
                    // (that is the `can_load == false` arm below), so it must
                    // not claim to be. Blaming that arm's reason on this one is
                    // the exact bug this event exists to fix.
                    Err(e) => {
                        // The pane is this process's log surface — see
                        // `agent_host`'s module doc. A `tracing` warning here
                        // goes nowhere, and this is the failure that silently
                        // costs a user their conversation.
                        println!("farcooler: could not load session {id}: {e}");
                        // `AcpError::Refused` carries the adapter's own words;
                        // anything else (a closed pipe, a malformed frame) is
                        // this connection's own description of what happened,
                        // which is the next best thing to the adapter's.
                        let detail = match &e {
                            AcpError::Refused(message) => message.clone(),
                            other => other.to_string(),
                        };
                        prelude.push(load_failed_event(&detail));
                        let result = conn
                            .request(
                                "session/new",
                                serde_json::json!({ "cwd": cwd, "mcpServers": [] }),
                            )
                            .await?;
                        let new_id = result["sessionId"].as_str().unwrap_or(&id).to_string();
                        (new_id, result)
                    }
                }
            }
            Some(id) => {
                // `can_load` was false: this agent never claimed `session/load`
                // at `initialize`, so nothing was even attempted. Honest rather
                // than convenient: the conversation continues, but the history
                // before this point cannot be shown.
                prelude.push(load_unsupported_event());
                let result = conn
                    .request("session/new", serde_json::json!({ "cwd": cwd, "mcpServers": [] }))
                    .await?;
                let new_id = result["sessionId"].as_str().unwrap_or(&id).to_string();
                (new_id, result)
            }
            None => {
                let result = conn
                    .request("session/new", serde_json::json!({ "cwd": cwd, "mcpServers": [] }))
                    .await?;
                let new_id =
                    result["sessionId"].as_str().ok_or(SessionError::Rejected)?.to_string();
                (new_id, result)
            }
        };

        let available_modes = modes_from(&session_result);
        let agent_mode = session_result["modes"]["currentModeId"].as_str().map(String::from);
        let config_options = config_options_from(&session_result);
        // Derived from the generic list so both shapes feed one code path.
        let by_category = |c: &str| config_options.iter().find(|o| o.category == c);
        let available_models = by_category("model").map(|o| o.options.clone()).unwrap_or_default();
        let model = by_category("model").map(|o| o.current_value.clone());
        let available_modes = if available_modes.is_empty() {
            by_category("mode").map(|o| o.options.clone()).unwrap_or_default()
        } else {
            available_modes
        };

        prelude.insert(
            0,
            AgentEvent::SessionStarted {
                session_id: session_id.clone(),
                agent_mode,
                available_modes: available_modes.clone(),
                model,
                available_models,
                config_options,
                available_commands: Vec::new(),
                backend: BackendKind::Acp.as_str().to_string(),
            },
        );

        // The replayed history, as one batch that ends where history ends.
        //
        // It is queued in the connection right now — `session/load` streams the
        // conversation as notifications while its own request is still in
        // flight. Left there, it trickles out AFTER startup with nothing to say
        // it was history, so a pane reopened onto an idle agent showed
        // "Working…" forever: the last thing anyone had seen was the agent
        // talking, and nothing ever said it had stopped.
        let mut replayed = conn.take_pending_updates();
        if !replayed.is_empty() {
            relativize(&mut replayed, &conn.worktree);
            prelude.extend(replayed);
            prelude.push(AgentEvent::TurnEnded { reason: end_reason("end_turn") });
        }

        Ok((
            Self {
                conn,
                session_id,
                can_load,
                // Ids only here: this field feeds the proto's
                // `available_agent_modes`, which is a repeated string. The
                // human names ride on the SessionStarted event, which is what
                // the pickers actually read.
                available_modes: available_modes.iter().map(|m| m.id.clone()).collect(),
                available_commands: Vec::new(),
                pending_prompt: None,
            },
            prelude,
        ))
    }

    /// Start a turn.
    ///
    /// Sent as a REQUEST, but not waited on. Its response is the only place a
    /// turn's end is reported (`stopReason`), so sending it as a notification
    /// meant `TurnEnded` never fired: activity stayed `Working` forever, `Done`
    /// never happened, and nothing ever notified anyone — the failure that
    /// makes the whole feature pointless. Waiting on it instead would deadlock,
    /// because the turn cannot finish while nobody is answering the agent's
    /// `fs/*` and permission requests.
    pub async fn prompt(&mut self, text: &str) -> Result<(), SessionError> {
        let id = self
            .conn
            .request_no_wait(
                "session/prompt",
                serde_json::json!({
                    "sessionId": self.session_id,
                    "prompt": [{ "type": "text", "text": text }]
                }),
            )
            .await?;
        self.pending_prompt = Some(id);
        Ok(())
    }

    pub async fn answer(
        &mut self,
        request_id: serde_json::Value,
        option_id: &str,
    ) -> Result<(), SessionError> {
        self.conn
            .respond(
                request_id,
                serde_json::json!({ "outcome": { "outcome": "selected", "optionId": option_id } }),
            )
            .await?;
        Ok(())
    }

    pub async fn set_mode(&mut self, agent_mode: &str) -> Result<(), SessionError> {
        self.conn
            .notify(
                "session/set_mode",
                serde_json::json!({ "sessionId": self.session_id, "modeId": agent_mode }),
            )
            .await?;
        Ok(())
    }

    pub async fn cancel(&mut self) -> Result<(), SessionError> {
        self.conn
            .notify("session/cancel", serde_json::json!({ "sessionId": self.session_id }))
            .await?;
        Ok(())
    }

    /// Hand off to the running form used once startup is over.
    ///
    /// `start` above still calls `AcpConnection::request` sequentially and
    /// that is fine — nothing races it. Once the daemon link and the agent's
    /// own `fs/*`/permission requests both need to be answered concurrently,
    /// the connection has to be split so the read side can live in a task
    /// that is never cancelled. See `AcpConnection::split` for why.
    pub fn into_running(self) -> RunningSession {
        let worktree = self.conn.worktree.clone();
        let (writer, incoming) = self.conn.split();
        RunningSession {
            writer,
            incoming,
            session_id: self.session_id,
            pending_prompt: self.pending_prompt,
            worktree,
        }
    }
}

/// An `AgentSession` after startup, with its connection split.
///
/// Every method here either awaits `self.incoming.recv()` (an mpsc receiver,
/// cancellation safe) or writes to stdin without reading anything back — no
/// method on this type contains a `read_line`. That is what makes it sound to
/// put `next_events` in a `tokio::select!` with commands arriving from a
/// daemon link that can connect and disconnect at any time.
///
/// What is left here after the prompt queue moved to
/// `farcooler_agent::chat::ChatSession` is entirely ACP: a writer, a frame
/// receiver, and the request id whose response ends a turn.
pub struct RunningSession {
    writer: AcpWriter,
    incoming: mpsc::UnboundedReceiver<Incoming>,
    pub session_id: String,
    /// See the field of the same name on `AgentSession`.
    pending_prompt: Option<u64>,
    worktree: PathBuf,
}

impl RunningSession {
    /// Start a turn.
    ///
    /// The queue that used to wrap this is `ChatSession`'s now. Holding a
    /// message so it can still be shown, rewritten, or taken back is Far
    /// Cooler's own behavior rather than anything ACP offers, and it works the
    /// same against a backend that is not ACP at all. What is left here is the
    /// one thing the protocol actually does, which is send.
    pub async fn prompt(&mut self, text: &str, images: &[PromptImage]) -> Result<(), SessionError> {
        self.send_prompt(text, images).await
    }

    /// Send into the turn already running.
    ///
    /// The adapter advertises `promptQueueing` and `steering`, meaning it will
    /// accept a prompt while a turn is running and Claude picks it up between
    /// tool calls rather than after everything finishes. That is the better
    /// behavior when what you are sending is a correction — "stop, do it this
    /// way" is worth nothing once the wrong thing is done.
    ///
    /// `pending_prompt` is deliberately restored afterwards. It names the
    /// request whose response ends the current turn, and a steering prompt
    /// joins that turn rather than starting its own — letting `send_prompt`
    /// overwrite it would leave the original turn with nothing to report its
    /// end, and the pane would say Working forever.
    ///
    /// Note that `Capabilities::acp()` reports `native_steer: false`, so
    /// `ChatSession` sends an ordinary `prompt` here instead and this is
    /// currently unreachable through it. It is kept because the ACP capability
    /// is real and the moment a client opts into it, this is the correct
    /// implementation rather than one to be written under time pressure.
    pub async fn steer(&mut self, text: &str, images: &[PromptImage]) -> Result<(), SessionError> {
        let pending = self.pending_prompt;
        self.send_prompt(text, images).await?;
        if pending.is_some() {
            self.pending_prompt = pending;
        }
        Ok(())
    }

    async fn send_prompt(&mut self, text: &str, images: &[PromptImage]) -> Result<(), SessionError> {
        // Images as CONTENT BLOCKS, not as paths.
        //
        // The apps used to attach a picture by putting `@/Users/you/x.png` in
        // the message. That works on the machine you are sitting at and nowhere
        // else: the agent runs on the HOST, and a path from a phone or from a
        // Mac driving a remote box refers to a file that does not exist there.
        // The adapter advertises `promptCapabilities.image`, so the bytes
        // travel with the prompt and the question never arises.
        let mut blocks: Vec<serde_json::Value> = images
            .iter()
            .map(|image| {
                serde_json::json!({
                    "type": "image",
                    "mimeType": image.mime,
                    "data": image.base64,
                })
            })
            .collect();
        if !text.is_empty() || blocks.is_empty() {
            blocks.push(serde_json::json!({ "type": "text", "text": text }));
        }

        let id = self
            .writer
            .request_no_wait(
                "session/prompt",
                serde_json::json!({ "sessionId": self.session_id, "prompt": blocks }),
            )
            .await?;
        self.pending_prompt = Some(id);
        Ok(())
    }

    pub async fn answer(
        &mut self,
        request_id: serde_json::Value,
        option_id: &str,
    ) -> Result<(), SessionError> {
        self.writer
            .respond(
                request_id,
                serde_json::json!({ "outcome": { "outcome": "selected", "optionId": option_id } }),
            )
            .await?;
        Ok(())
    }

    pub async fn set_mode(&mut self, agent_mode: &str) -> Result<(), SessionError> {
        self.writer
            .notify(
                "session/set_mode",
                serde_json::json!({ "sessionId": self.session_id, "modeId": agent_mode }),
            )
            .await?;
        Ok(())
    }

    /// Change one of the agent's advertised selectors.
    ///
    /// `configId`, not `optionId` — the adapter rejects the latter with
    /// "expected string, received undefined", which is the kind of thing only
    /// trying it tells you.
    pub async fn set_config_option(&mut self, id: &str, value: &str) -> Result<(), SessionError> {
        self.writer
            .notify(
                "session/set_config_option",
                serde_json::json!({ "sessionId": self.session_id, "configId": id, "value": value }),
            )
            .await?;
        Ok(())
    }

    /// Switch the model this session uses.
    ///
    /// `session/set_model`, verified against a live adapter rather than assumed:
    /// ACP has since stabilised `session/set_config_option` as the general form
    /// (categories `mode`, `model`, `model_config`, `thought_level`) and says
    /// the dedicated methods will eventually go, but the adapter in use answers
    /// `Method not found` to the new one and `OK` to this. When an agent starts
    /// advertising `configOptions`, that is the branch to add — not a
    /// replacement, since both will be in the wild at once.
    pub async fn set_model(&mut self, model: &str) -> Result<(), SessionError> {
        self.writer
            .notify(
                "session/set_model",
                serde_json::json!({ "sessionId": self.session_id, "modelId": model }),
            )
            .await?;
        Ok(())
    }

    pub async fn cancel(&mut self) -> Result<(), SessionError> {
        self.writer
            .notify("session/cancel", serde_json::json!({ "sessionId": self.session_id }))
            .await?;
        Ok(())
    }

    /// Wait for the next frame and turn it into events, answering
    /// capabilities inline.
    ///
    /// Cancellation-safe: the only await that can be pending for an
    /// unbounded time — the adapter decides when the next frame arrives — is
    /// `self.incoming.recv()`, and `recv` is documented cancellation safe:
    /// dropping it before it resolves leaves the message in the channel for
    /// the next call. That is the same property `read_line` lacked, which is
    /// why `read_line` was moved into `AcpConnection::split`'s reader task
    /// instead of living here.
    ///
    /// Everything from here on is bounded, local work this process controls
    /// the pace of: `handle_fs_write` is synchronous, and the `respond` that
    /// follows it writes a few dozen bytes to a pipe with a much larger OS
    /// buffer, so it resolves in the same scheduler turn rather than
    /// genuinely suspending. That is what keeps the old failure mode — the
    /// file lands but the answer never does — closed in practice: nothing
    /// unrelated to this exchange (in particular, the high-frequency daemon
    /// socket read that used to race `pump()` directly) can preempt it
    /// mid-write anymore. What CAN still preempt this call is a command
    /// arriving on the daemon link's own `mpsc` channel, and that channel is
    /// low-frequency by nature — one message per human action, not one per
    /// socket byte.
    /// Wait for one frame, and do nothing else.
    ///
    /// This is the ONLY thing that may appear as a `select!` branch. It awaits
    /// an mpsc receive and returns — it performs no file writes and sends no
    /// responses, so losing the race costs nothing.
    ///
    /// `next_events` must never be used in a `select!`: it awaits `respond`
    /// after `handle_fs_write` has already touched the disk, so being cancelled
    /// between them leaves the file written and the agent waiting forever on an
    /// answer that is never coming. Splitting the receive from the handling is
    /// what makes that impossible rather than merely rare.
    pub async fn recv_frame(&mut self) -> Result<Incoming, SessionError> {
        self.incoming.recv().await.ok_or({
            // The reader task spawned by `split` only exits by dropping its
            // sender, and it only does that when the adapter's stdout closed
            // or sent something unparseable — either way the adapter is gone.
            // Reporting "nothing happened yet" instead would spin a caller
            // forever on a channel that can never produce again.
            SessionError::Acp(AcpError::Closed)
        })
    }

    /// Receive one frame and handle it.
    ///
    /// Convenience for callers that are not racing anything. A `select!` must
    /// use `recv_frame` and then `handle` the result outside the select — see
    /// `recv_frame`.
    pub async fn next_events(&mut self) -> Result<Vec<AgentEvent>, SessionError> {
        let incoming = self.recv_frame().await?;
        self.handle(incoming).await
    }

    /// Act on one frame: answer capabilities, and report what happened.
    ///
    /// Awaits, and must therefore run to completion outside any `select!`.
    pub async fn handle(
        &mut self,
        incoming: Incoming,
    ) -> Result<Vec<AgentEvent>, SessionError> {
        let (method, id, params) = match incoming {
            Incoming::Request { id, method, params } => (method, Some(id), params),
            Incoming::Notification { method, params } => (method, None, params),
            Incoming::Response { id, result } => {
                // The turn's end, and the only place it is reported.
                if id.as_u64() == self.pending_prompt {
                    self.pending_prompt = None;
                    let reason = end_reason(result["stopReason"].as_str().unwrap_or_default());
                    // Reporting the end is all this does now. Draining the
                    // queue is `ChatSession`'s job, and it triggers on exactly
                    // this event — which is what lets a backend that reports a
                    // turn's end some other way get the same behavior for free.
                    return Ok(vec![AgentEvent::TurnEnded { reason }]);
                }
                return Ok(Vec::new());
            }
        };

        let worktree = self.worktree.clone();
        match (method.as_str(), id) {
            ("fs/read_text_file", Some(id)) => {
                let path = params["path"].as_str().unwrap_or_default();
                match handle_fs_read(&worktree, path) {
                    Ok(content) => {
                        self.writer.respond(id, serde_json::json!({ "content": content })).await?
                    }
                    // Answered rather than left hanging: an unanswered request
                    // stalls the agent forever, and a refusal it can see is
                    // better than a turn that never ends.
                    Err(_) => self.writer.respond(id, serde_json::json!({ "content": "" })).await?,
                }
                Ok(Vec::new())
            }
            ("fs/write_text_file", Some(id)) => {
                let path = params["path"].as_str().unwrap_or_default();
                let content = params["content"].as_str().unwrap_or_default();
                // The write and its paired response used to straddle the
                // `select!` that raced `pump()` against every line on the
                // daemon socket — a socket that could produce a new line far
                // more often than an adapter answers a turn. Cancellation
                // there left the file written with the agent still blocked
                // on an answer that would never come. That racing partner is
                // gone now: this call is only reachable from `next_events`,
                // which nothing but a daemon command can preempt, and that is
                // orders of magnitude rarer than "the daemon sent a byte".
                let outcome = handle_fs_write(&worktree, path, content);
                self.writer.respond(id, serde_json::json!({})).await?;
                match outcome {
                    Ok(event) => Ok(vec![event]),
                    Err(_) => Ok(vec![AgentEvent::Gap { reason: AgentGapReason::Unparsed }]),
                }
            }
            ("session/request_permission", Some(id)) => {
                // The id is kept as JSON text so the client can hand back
                // exactly what the adapter sent; inventing a new id here would
                // make the answer unroutable.
                let request_id = serde_json::to_string(&id).unwrap_or_default();
                Ok(vec![permission_event(&request_id, &params)])
            }
            ("session/update", _) => {
                let raw = params.clone();
                let rpc = Rpc { method: Some(method), params: Some(params), id: None, result: None, error: None };
                match rpc.session_notification() {
                    Some(n) => {
                        let mut events = update_to_events(&n.update);
                        // Paths shown to a person, not to a machine: everything
                        // here is understood to be in the worktree already.
                        relativize(&mut events, &worktree);
                        // Any gap from an update names the frame that caused
                        // it. `SessionUpdate::Unknown` throws the raw JSON
                        // away by design, so without this the one thing needed
                        // to fix the gap is the one thing not recorded.
                        if events.iter().any(|e| matches!(e, AgentEvent::Gap { .. })) {
                            println!(
                                "farcooler: unmodelled session/update: {}",
                                serde_json::to_string(&raw)
                                    .unwrap_or_default()
                                    .chars()
                                    .take(300)
                                    .collect::<String>()
                            );
                        }
                        Ok(events)
                    }
                    None => {
                        // The one Gap path that used to say nothing. A
                        // `session/update` whose params do not deserialize
                        // produced a break in the transcript with no way to
                        // find out which frame caused it.
                        println!(
                            "farcooler: could not read a session/update: {}",
                            serde_json::to_string(&raw).unwrap_or_default().chars().take(300).collect::<String>()
                        );
                        Ok(vec![AgentEvent::Gap { reason: AgentGapReason::Unparsed }])
                    }
                }
            }
            (other, id) => {
                // Named, because a silent gap is untraceable. An adapter that
                // asks us something we do not implement is blocked until it is
                // answered, so an unanswered REQUEST is worse than a gap: it
                // hangs the turn.
                // Printed, not traced. The pane IS this process's log surface
                // — see the module doc on `agent_host` — and a `tracing`
                // subscriber is never installed on this path, so a warning
                // there goes nowhere at all.
                println!("farcooler: unhandled ACP method `{other}`");
                if let Some(id) = id {
                    // Answer anyway. An empty result is a poor answer, but a
                    // turn that continues beats one that waits forever.
                    let _ = self.writer.respond(id, serde_json::json!({})).await;
                }
                Ok(vec![AgentEvent::Gap { reason: AgentGapReason::Unparsed }])
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_agent_core::event::{AgentEvent, AgentGapReason};

    #[tokio::test]
    async fn next_events_on_a_closed_connection_reports_closure_not_a_hang() {
        // The bug this guards against: `pump()`'s `read_line` was not
        // cancellation safe, so an `mpsc` receiver replaced it. If closure
        // showed up as `Ok(vec![])` instead of an error, a caller `select!`ing
        // on this would treat it as "nothing happened yet" and loop forever
        // on a channel that can never produce again — a hang indistinguishable
        // from the agent being merely slow.
        let launch = farcooler_agent_core::backend::Launch {
            program: "/bin/sh".into(),
            args: vec!["-c".to_string(), "exit 0".to_string()],
            env: Default::default(),
        };
        let conn = AcpConnection::spawn(&launch, std::env::temp_dir())
            .await
            .expect("spawn a fake adapter that exits immediately");
        let worktree = conn.worktree.clone();
        let (writer, incoming) = conn.split();
        let mut session = RunningSession {
            writer,
            incoming,
            session_id: "s".to_string(),
            pending_prompt: None,
            worktree,
        };

        let outcome = tokio::time::timeout(std::time::Duration::from_secs(5), session.next_events())
            .await
            .expect("next_events must not hang waiting on an adapter that already exited");
        assert!(outcome.is_err(), "closure must be a reported error, not a silently empty batch");
    }

    #[test]
    fn an_fs_write_becomes_a_diff_carrying_what_was_there_before() {
        // Tier 2's whole justification: the diff is a protocol fact, not a
        // reconstruction from a vendor's private tool schema.
        let dir = std::env::temp_dir().join(format!("farcooler-sess-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        std::fs::write(&file, "old\n").unwrap();

        let event = handle_fs_write(&dir, file.to_str().unwrap(), "new\n").expect("allowed");
        let AgentEvent::ToolUpdate { diff: Some(d), .. } = event else { panic!("expected a diff") };
        assert_eq!(d.old_text.as_deref(), Some("old\n"));
        assert_eq!(d.new_text, "new\n");
    }

    #[test]
    fn a_write_outside_the_worktree_is_refused_and_says_so() {
        let dir = std::env::temp_dir().join(format!("farcooler-sess2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(handle_fs_write(&dir, "/etc/passwd", "x").is_err());
    }

    #[test]
    fn a_permission_request_becomes_a_blocking_event() {
        let params = serde_json::json!({
            "toolCall": { "toolCallId": "t1", "title": "Run ls" },
            "options": [
                { "optionId": "allow", "name": "Yes", "kind": "allow_once" },
                { "optionId": "reject", "name": "No", "kind": "reject_once" }
            ]
        });
        let event = permission_event("req-1", &params);
        let AgentEvent::Permission { id, options, .. } = event else { panic!("expected permission") };
        assert_eq!(id, "req-1");
        assert_eq!(options.len(), 2);
        assert_eq!(options[0].id, "allow");
    }

    #[test]
    fn a_reconnect_to_an_agent_that_never_offered_load_produces_a_visible_gap() {
        assert_eq!(
            load_unsupported_event(),
            AgentEvent::Gap { reason: AgentGapReason::LoadUnsupported }
        );
    }

    #[test]
    fn a_session_load_refused_as_not_found_reads_as_empty_not_broken() {
        // The bug this pins: `claude-agent-acp` answers exactly "Session not
        // found" for a `--session-id` nobody has typed into yet — the ordinary
        // path a fresh chat-mode pane takes — and that used to render as
        // "This session could not be loaded from where it left off", which
        // blames the adapter for something it did not do.
        assert_eq!(
            load_failed_event("Session not found"),
            AgentEvent::Gap { reason: AgentGapReason::LoadEmpty }
        );
        // Case-insensitively, and regardless of what surrounds the phrase —
        // no adapter has promised this exact sentence, only the word choice.
        assert_eq!(
            load_failed_event("session NOT FOUND: 019fc8af"),
            AgentEvent::Gap { reason: AgentGapReason::LoadEmpty }
        );
    }

    #[test]
    fn a_codex_session_load_refused_as_no_rollout_found_also_reads_as_empty() {
        // Codex's real wording, read directly off `@agentclientprotocol/codex-acp`
        // on 2026-08-03 by probing `session/load` for an unknown id: `message`
        // is the useless "Internal error", and `AcpConnection::request` folds
        // `data.details` in, producing exactly this string. It does NOT
        // contain "not found" — "no rollout found" reads "found", negated by
        // the leading "no" — so it needs its own phrase in the sniff, not a
        // looser match on the first one.
        assert_eq!(
            load_failed_event(
                "Internal error: no rollout found for thread id \
                 00000000-0000-7000-8000-000000000000"
            ),
            AgentEvent::Gap { reason: AgentGapReason::LoadEmpty }
        );
    }

    #[test]
    fn a_session_load_refused_for_any_other_reason_carries_the_adapters_detail() {
        // The other defect this fixes: the adapter's own message used to reach
        // only a `println!` on the pane's log surface, which is exactly the
        // surface chat mode replaces — so the user saw a dead end and nothing
        // else. It has to survive into the event to reach them at all.
        assert_eq!(
            load_failed_event("permission denied: /var/db/is/root-owned"),
            AgentEvent::Gap {
                reason: AgentGapReason::LoadFailed {
                    detail: "permission denied: /var/db/is/root-owned".to_string()
                }
            }
        );
    }

    #[test]
    fn modes_come_from_the_session_result_not_from_initialize() {
        // Measured in the Gate 1 spike against a real adapter: `initialize`
        // advertises `loadSession` and prompt capabilities and says nothing
        // about modes. Reading them there gave an empty list forever, so the
        // mode switcher had nothing to offer and silently did nothing.
        let session_result = serde_json::json!({
            "sessionId": "s",
            "modes": {
                "currentModeId": "default",
                "availableModes": [
                    { "id": "default", "name": "Default" },
                    { "id": "acceptEdits", "name": "Accept Edits" },
                    { "id": "plan", "name": "Plan Mode" }
                ]
            }
        });
        let modes = modes_from(&session_result);
        assert_eq!(
            modes.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec!["default", "acceptEdits", "plan"]
        );
        // The names are the point: a picker offering `acceptEdits` is offering
        // the wire's vocabulary to someone who never chose those words.
        assert_eq!(
            modes.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(),
            vec!["Default", "Accept Edits", "Plan Mode"]
        );

        // And the shape the bug assumed yields nothing, which is what made the
        // failure invisible rather than loud.
        let initialize_result = serde_json::json!({
            "agentCapabilities": { "loadSession": true }
        });
        assert!(modes_from(&initialize_result).is_empty());
    }

    #[test]
    fn a_stop_reason_ends_the_turn_and_an_unfamiliar_one_still_ends_it() {
        // The turn is over whatever the adapter chose to call it. Refusing to
        // admit that would leave the row on `Working` forever, which is exactly
        // the state that never becomes `Done` and never notifies.
        assert_eq!(end_reason("end_turn"), EndReason::EndTurn);
        assert_eq!(end_reason("cancelled"), EndReason::Cancelled);
        assert_eq!(end_reason("refusal"), EndReason::Refusal);
        assert_eq!(end_reason("max_tokens"), EndReason::MaxTokens);
        assert_eq!(end_reason("something_a_later_adapter_invented"), EndReason::EndTurn);
        assert_eq!(end_reason(""), EndReason::EndTurn);
    }

    #[test]
    fn config_options_come_through_generically() {
        // The real shape, copied from a live 0.64 adapter. The `agent` selector
        // is the reason this is generic: nobody designed a field for a subagent
        // picker, and it arrives here for free alongside mode and model.
        let result = serde_json::json!({
            "sessionId": "s",
            "configOptions": [
                { "id": "mode", "name": "Mode", "category": "mode", "type": "select",
                  "currentValue": "default",
                  "options": [{ "value": "default", "name": "Manual", "description": "Prompts" }] },
                { "id": "model", "name": "Model", "category": "model", "type": "select",
                  "currentValue": "haiku",
                  "options": [
                    { "value": "opus", "name": "Opus", "description": "Opus 5" },
                    { "value": "haiku", "name": "Haiku", "description": "Haiku 4.5" }] },
                { "id": "agent", "name": "Agent", "type": "select", "currentValue": "default",
                  "options": [{ "value": "default", "name": "Default" }] }
            ]
        });
        let options = config_options_from(&result);
        assert_eq!(
            options.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(),
            vec!["mode", "model", "agent"]
        );
        let model = options.iter().find(|o| o.category == "model").expect("a model selector");
        assert_eq!(model.current_value, "haiku");
        assert_eq!(model.options[0].name, "Opus");
    }

    #[test]
    fn an_adapter_without_config_options_still_offers_its_modes_and_models() {
        // The older adapter sends `modes`/`models` and no `configOptions`, and
        // both are in the wild at once. Synthesising here means the client
        // renders one list and never learns which shape it is talking to.
        let legacy = serde_json::json!({
            "sessionId": "s",
            "modes": { "currentModeId": "plan",
                       "availableModes": [{ "id": "plan", "name": "Plan Mode" }] },
            "models": { "currentModelId": "sonnet",
                        "availableModels": [{ "modelId": "sonnet", "name": "Sonnet" }] }
        });
        let options = config_options_from(&legacy);
        assert_eq!(options.len(), 2);
        assert_eq!(options[0].current_value, "plan");
        assert_eq!(options[1].options[0].name, "Sonnet");
    }
}

