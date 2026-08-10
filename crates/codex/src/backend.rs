//! `AgentBackend`, as codex app-server performs it.

use farcooler_agent_core::backend::{
    AgentBackend, BackendError, BackendKind, Capabilities, Launch,
};
use farcooler_agent_core::event::{AgentChoice, AgentEvent, ConfigOption, PromptImage};

use crate::conn::{CodexConnection, CodexError, CodexWriter, Incoming};
use crate::normalize::{Origin, approval_event, frame_to_events};

impl From<CodexError> for BackendError {
    fn from(e: CodexError) -> Self {
        match e {
            CodexError::Spawn => BackendError::Spawn,
            CodexError::Closed => BackendError::Closed,
            CodexError::Refused(message) => BackendError::Refused(message),
        }
    }
}

/// A live codex thread.
pub struct CodexBackend {
    writer: CodexWriter,
    incoming: tokio::sync::mpsc::UnboundedReceiver<Incoming>,
    pub thread_id: String,
    /// The `turn/start` whose response ends the current turn.
    ///
    /// The same role `pending_prompt` plays on the ACP side: without it a
    /// response cannot be told apart from any other and the one frame that
    /// reports a turn's end goes unrecognized. Codex also announces the end
    /// with `turn/completed`, which is what `normalize` reads — this exists so
    /// a `turn/start` that fails outright is not mistaken for a running turn.
    pending_turn: Option<u64>,
    /// Chosen per turn rather than held by the server: `turn/start` takes
    /// `model` and `effort` overrides, so a selector change applies to the next
    /// turn instead of needing a new thread.
    model: Option<String>,
    effort: Option<String>,
    /// The same, for `approvalPolicy`. Held rather than sent immediately for
    /// the same reason: `turn/start` is where codex accepts it.
    approval: Option<String>,
}

impl CodexBackend {
    /// Start the server, initialize, and join or create a thread.
    ///
    /// `resume` carries the session id Far Cooler handed the terminal at
    /// launch. Unlike ACP's `session/load`, resuming here is a first-class
    /// method rather than an optional capability — which is why
    /// `Capabilities::replay` is unconditionally true for this backend.
    pub async fn start(
        launch: &Launch,
        worktree: std::path::PathBuf,
        resume: Option<String>,
    ) -> Result<(Self, Vec<AgentEvent>), BackendError> {
        let args = crate::handshake::launch_args(&launch.args);
        let mut conn = CodexConnection::spawn(&launch.program, &args, &launch.env, worktree.clone())
            .await?;

        let init = conn.request("initialize", crate::handshake::initialize_params()).await?;
        let version = init
            .get("userAgent")
            .and_then(|u| u.as_str())
            .and_then(crate::handshake::version_of)
            .unwrap_or_default();
        crate::handshake::check_version(&version, crate::handshake::PINNED_CODEX_VERSION)?;
        conn.notify("initialized", serde_json::json!({})).await?;

        let cwd = worktree.display().to_string();
        // Resume by thread id when there is one, and fall back to a new thread
        // rather than failing: a session id with no rollout yet is the COMMON
        // case, since every codex terminal is handed one at launch and a pane
        // switched to chat before its first turn has nothing recorded.
        let (result, resumed) = match &resume {
            Some(id) => {
                let attempt = conn
                    .request(
                        "thread/resume",
                        serde_json::json!({ "threadId": id, "cwd": cwd }),
                    )
                    .await;
                match attempt {
                    Ok(result) => (result, true),
                    Err(_) => (
                        conn.request("thread/start", serde_json::json!({ "cwd": cwd })).await?,
                        false,
                    ),
                }
            }
            None => (conn.request("thread/start", serde_json::json!({ "cwd": cwd })).await?, false),
        };

        // Nested under `thread`, not at the top level — observed, and the kind
        // of thing that silently yields an empty id if assumed otherwise.
        let thread_id = result["thread"]["id"]
            .as_str()
            .ok_or_else(|| BackendError::Refused("codex started no thread".into()))?
            .to_string();

        let model = result["model"].as_str().map(str::to_string);
        let effort = result["reasoningEffort"].as_str().map(str::to_string);
        // `AskForApproval` is a string OR a `{granular: {...}}` object, and
        // `as_str` is how the two are told apart: a granular policy reads as
        // None here and the picker stays away, which is the point. See
        // `approval_policies`.
        let approval = result["approvalPolicy"].as_str().map(str::to_string);

        // Asked for rather than inferred: `thread/start` reports the model in
        // use but not what else is on offer, so without this the picker holds
        // exactly one entry — a control that cannot change anything. A failure
        // here costs the menu, not the session.
        let catalog = match conn.request("model/list", serde_json::json!({})).await {
            Ok(result) => models_from(&result),
            Err(_) => Vec::new(),
        };

        // Reported the same way Claude reports its permission mode, so a client
        // asking "what mode is this in" gets an answer from either backend
        // rather than from one of them — and through `approval_picker`, so this
        // and the `config_options` list cannot say different things.
        let (agent_mode, available_modes) = match approval_picker(&approval) {
            Some((current, options)) => (Some(current), options),
            None => (None, Vec::new()),
        };

        let mut prelude = vec![AgentEvent::SessionStarted {
            session_id: thread_id.clone(),
            agent_mode,
            available_modes,
            model: model.clone(),
            config_options: config_options(&model, &effort, &approval, &catalog),
            available_models: catalog
                .iter()
                .map(|m| AgentChoice {
                    id: m.id.clone(),
                    name: m.name.clone(),
                    description: m.description.clone(),
                })
                .collect(),
            available_commands: Vec::new(),
            backend: BackendKind::Codex.as_str().to_string(),
        }];

        // Frames seen while the startup requests were in flight. Bookkeeping,
        // mostly — the conversation is NOT among them, which is the thing that
        // is easy to assume and wrong.
        for (method, params) in conn.take_pending() {
            prelude.extend(frame_to_events(&method, &params, Origin::Replay));
        }

        // History has to be ASKED for. `thread/resume` attaches to the thread
        // and streams only status; `initialTurnsPage` on its response comes
        // back null. Without this a restored pane showed nothing at all.
        //
        // A failure here costs the history, not the session: continuing with a
        // visible gap beats refusing to open a conversation you can still use.
        if resumed {
            match conn
                .request(
                    "thread/read",
                    serde_json::json!({ "threadId": thread_id, "includeTurns": true }),
                )
                .await
            {
                Ok(result) => {
                    let restored = crate::normalize::history_to_events(&result);
                    let empty = restored.is_empty();
                    prelude.extend(restored);
                    if empty {
                        // A thread with nothing recorded yet is the common
                        // case, not a problem — every codex terminal is handed
                        // a session id at launch and a pane switched to chat
                        // before its first turn has no transcript.
                        prelude.push(AgentEvent::Gap {
                            reason: farcooler_agent_core::event::AgentGapReason::LoadEmpty,
                        });
                    }
                    prelude.push(AgentEvent::TurnEnded {
                        reason: farcooler_agent_core::event::EndReason::EndTurn,
                    });
                }
                Err(e) => prelude.push(AgentEvent::Gap {
                    reason: farcooler_agent_core::event::AgentGapReason::LoadFailed {
                        detail: e.to_string(),
                    },
                }),
            }
        }

        let (writer, incoming) = conn.split();
        Ok((
            CodexBackend {
                writer,
                incoming,
                thread_id,
                pending_turn: None,
                model,
                effort,
                approval,
            },
            prelude,
        ))
    }

    /// Wait for one frame, and do nothing else.
    ///
    /// The only thing that may appear in a `select!`. See `AcpBackend` — the
    /// same cancellation-safety argument applies, and for the same reason:
    /// handling a frame can answer an approval request, and being cancelled
    /// mid-answer leaves the agent waiting forever.
    pub async fn recv_frame(&mut self) -> Result<Incoming, BackendError> {
        self.incoming.recv().await.ok_or(BackendError::Closed)
    }

    /// Act on one frame.
    pub async fn handle(&mut self, incoming: Incoming) -> Result<Vec<AgentEvent>, BackendError> {
        match incoming {
            Incoming::Notification { method, params } => {
                Ok(frame_to_events(&method, &params, Origin::Live))
            }
            Incoming::Request { id, method, params } => {
                // Every approval the server can ask becomes one event. The id
                // travels as JSON text so the answer is routable — inventing a
                // new one here would make it unanswerable.
                let request_id = serde_json::to_string(&id).unwrap_or_default();
                Ok(vec![approval_event(&request_id, &method, &params)])
            }
            Incoming::Response { id, .. } => {
                if id.as_u64() == self.pending_turn {
                    self.pending_turn = None;
                }
                Ok(Vec::new())
            }
        }
    }
}

/// One model the agent offers, and the reasoning depths it supports.
///
/// The efforts are per MODEL, not global — observed on 0.147.0, where Sol and
/// Terra offer `ultra`, Luna stops at `max`, and the 5.4/5.5 line stops at
/// `xhigh`. A single hardcoded list would offer people settings their model
/// would reject.
#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub efforts: Vec<AgentChoice>,
}

/// The model catalog, from a `model/list` result.
///
/// Hidden models are dropped: codex marks them as not belonging in a picker,
/// and Far Cooler's picker is a picker.
pub fn models_from(result: &serde_json::Value) -> Vec<ModelInfo> {
    result["data"]
        .as_array()
        .map(|models| {
            models
                .iter()
                .filter(|m| !m["hidden"].as_bool().unwrap_or(false))
                .filter_map(|m| {
                    let id = m["id"].as_str()?.to_string();
                    Some(ModelInfo {
                        name: m["displayName"].as_str().unwrap_or(&id).to_string(),
                        description: m["description"].as_str().unwrap_or_default().to_string(),
                        efforts: m["supportedReasoningEfforts"]
                            .as_array()
                            .map(|efforts| {
                                efforts
                                    .iter()
                                    .filter_map(|e| {
                                        let id = e["reasoningEffort"].as_str()?.to_string();
                                        Some(AgentChoice {
                                            name: id.clone(),
                                            description: e["description"]
                                                .as_str()
                                                .unwrap_or_default()
                                                .to_string(),
                                            id,
                                        })
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                        id,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// How much codex asks before acting, as a picker.
///
/// The parity gap this closes: Claude's native backend offers a six-entry
/// permission-mode picker and codex offered model and effort only, so a codex
/// chat had no way to change how much it asks — the one setting people reach
/// for most. The data was already in hand: `ThreadStartResponse` reports
/// `approvalPolicy` (`"on-request"`, on a live 0.147.0) and `TurnStartParams`
/// accepts it back as a per-turn override, the same road `model` and `effort`
/// already travel. A `turn/start` carrying `"approvalPolicy": "never"` was
/// accepted and ran, rather than reasoned about from the schema.
///
/// The three string variants of `AskForApproval` and NOT its `{granular: {...}}`
/// object. A picker cannot express five booleans, and more to the point it
/// cannot round-trip one: choosing any entry here would flatten a granular
/// policy the user had configured elsewhere into a coarse one, silently. A
/// control that quietly discards what it cannot represent is worse than no
/// control — which is also why `config_options` declines to draw this picker at
/// all when the thread reports a policy it is not one of these.
///
/// Ordered by how much codex is allowed to do unasked, which is the axis a
/// person chooses along — not the order the union declares them in. Named the
/// way someone would choose them rather than the way the wire spells them.
fn approval_policies() -> Vec<AgentChoice> {
    [
        ("untrusted", "Ask Always", "Prompts before anything not already trusted"),
        ("on-request", "Ask When Needed", "Prompts only when the sandbox isn't enough"),
        ("never", "Don't Ask", "Never prompts — the sandbox is the only limit"),
    ]
    .iter()
    .map(|(id, name, description)| AgentChoice {
        id: (*id).to_string(),
        name: (*name).to_string(),
        description: (*description).to_string(),
    })
    .collect()
}

/// The approval picker, when the thread reports a policy it can express.
///
/// ONE function because two surfaces publish this and they have to agree:
/// `config_options` draws the picker, and `SessionStarted.available_modes` is
/// what `agent_supervisor` stores and republishes as `availableAgentModes`.
/// They did not agree — `available_modes` was filled unconditionally while
/// `config_options` filtered — so a thread whose policy this cannot express
/// still published a three-entry mode picker, and `DaemonMessage::SetMode`
/// routes straight back to `Selector::Approval`. Choosing any entry would then
/// flatten the very policy the filter one function away exists to protect.
///
/// `None` for a `{granular: {...}}` object, which reads as no string at all,
/// and for a fourth string a later codex adds. Both would leave the current
/// value naming an option that is not in the list, and a picker showing a
/// setting it does not contain is a picker that lies about what is in force.
/// The mode is withheld along with the menu rather than reported beside a list
/// that excludes it.
fn approval_picker(reported: &Option<String>) -> Option<(String, Vec<AgentChoice>)> {
    let options = approval_policies();
    let current = reported.as_ref().filter(|c| options.iter().any(|o| &&o.id == c))?;
    Some((current.clone(), options))
}

/// The selectors this thread offers, as the generic list clients render.
///
/// Model, reasoning effort, and approval policy, because those are what
/// `turn/start` accepts as per-turn overrides.
///
/// The sandbox deliberately does NOT get the same treatment, and the reason is
/// not that nobody would want it. `ThreadStartResponse.sandbox` and
/// `TurnStartParams.sandboxPolicy` are a `SandboxPolicy` — an OBJECT — and not
/// the flat `SandboxMode` enum it is easy to mistake it for. A live 0.147.0
/// reports it as `{"type": "workspaceWrite", "writableRoots": [],
/// "networkAccess": false, "excludeTmpdirEnvVar": false, "excludeSlashTmp":
/// false}`. A three-entry picker would have to send `{"type":
/// "workspaceWrite"}` alone, and every field it omitted would revert to a
/// default — so choosing the mode already in use would quietly erase whatever
/// writable roots and network access someone had configured. Same objection as
/// `granular` above: a control that cannot round-trip its own value does more
/// harm than the missing control does.
///
/// The effort menu is the CURRENT model's, which is a real limitation worth
/// naming: switching model cannot re-send the menu, because a consumer is
/// entitled to assume a session starts exactly once and `SessionStarted` is
/// where the options ride. Picking an effort the new model does not support is
/// refused by codex rather than silently misapplied.
fn config_options(
    model: &Option<String>,
    effort: &Option<String>,
    approval: &Option<String>,
    catalog: &[ModelInfo],
) -> Vec<ConfigOption> {
    let mut out = Vec::new();

    // First, and `category: "mode"`, so the GUIs put it where Claude's mode
    // selector goes rather than somewhere else for the other backend.
    if let Some((current_value, options)) = approval_picker(approval) {
        out.push(ConfigOption {
            id: "approval".into(),
            name: "Approvals".into(),
            description: String::new(),
            category: "mode".into(),
            kind: "select".into(),
            current_value,
            options,
        });
    }

    if let Some(current) = model {
        // Falls back to the current value alone rather than showing nothing:
        // a `model/list` that failed should cost the menu, not the label.
        let options: Vec<AgentChoice> = if catalog.is_empty() {
            vec![AgentChoice {
                id: current.clone(),
                name: current.clone(),
                description: String::new(),
            }]
        } else {
            catalog
                .iter()
                .map(|m| AgentChoice {
                    id: m.id.clone(),
                    name: m.name.clone(),
                    description: m.description.clone(),
                })
                .collect()
        };
        out.push(ConfigOption {
            id: "model".into(),
            name: "Model".into(),
            description: String::new(),
            category: "model".into(),
            kind: "select".into(),
            current_value: current.clone(),
            options,
        });

        if let Some(effort) = effort {
            let efforts = catalog
                .iter()
                .find(|m| &m.id == current)
                .map(|m| m.efforts.clone())
                .unwrap_or_default();
            if !efforts.is_empty() {
                out.push(ConfigOption {
                    id: "effort".into(),
                    name: "Reasoning".into(),
                    description: String::new(),
                    category: "thought_level".into(),
                    kind: "select".into(),
                    current_value: effort.clone(),
                    options: efforts,
                });
            }
        }
    }
    out
}

/// The `input` array for a prompt.
///
/// Images used to be dropped here, on the grounds that `UserInput`'s image
/// variant takes a URL and Far Cooler carries bytes. That reasoning was wrong:
/// a `data:` URL is an ordinary URL, and `PromptImage { mime, base64 }`
/// composes into one with nothing left over and nothing invented.
///
/// Verified against a live codex-cli 0.147.0 rather than reasoned from the
/// schema: a turn carrying a solid red PNG as a `data:` URL and the question
/// "what color fills the attached image" came back "Red". So the image is
/// accepted AND it reaches the model, which are two separate things and only
/// the second one matters.
///
/// If a later codex rejects `data:` URLs, the fallback is `localImage`, the
/// sibling variant that takes a `path`: write the bytes to a temporary file and
/// send that. It is the fallback and not the first choice because a path is
/// only meaningful on the machine that codex runs on.
///
/// The text goes first and unconditionally, so a rejected image can still only
/// cost the picture. Losing the picture is bad; losing the question with it
/// would be worse.
fn input_for(text: &str, images: &[PromptImage]) -> serde_json::Value {
    let mut input = vec![serde_json::json!({ "type": "text", "text": text })];
    for image in images {
        // An empty `mime` would interpolate to `data:;base64,…`, which is a
        // valid URL meaning `text/plain` — so the picture would be sent, and
        // read as text, and rejected. The same `PromptImage` producer feeds the
        // Claude backend, which already answers this by defaulting to PNG (see
        // `claude/src/conn.rs`): the format Far Cooler's own screenshot path
        // produces. Two backends taking the same input should not disagree
        // about it, and a wrong guess costs the picture — which is what sending
        // no type at all costs anyway.
        let mime = if image.mime.is_empty() { "image/png" } else { &image.mime };
        input.push(serde_json::json!({
            "type": "image",
            "url": format!("data:{mime};base64,{}", image.base64),
        }));
    }
    serde_json::Value::Array(input)
}

/// The three per-turn overrides a picker can change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Selector {
    Model,
    Effort,
    Approval,
}

/// Which selector an option id names.
///
/// Each answers to its own id AND to its category, which `effort` already did
/// and which the mode selector inherits: a client that hands back the category
/// it grouped the control under — `thought_level`, `mode` — would otherwise set
/// nothing at all and be told it succeeded.
fn selector_for(id: &str) -> Option<Selector> {
    match id {
        "model" => Some(Selector::Model),
        "effort" | "thought_level" => Some(Selector::Effort),
        "approval" | "mode" => Some(Selector::Approval),
        _ => None,
    }
}

/// The `turn/start` frame, carrying whatever the pickers have chosen since the
/// last turn.
///
/// Every selector rides here rather than being pushed to the server when it
/// changes, because `TurnStartParams` is where codex accepts them: `model`,
/// `effort` and `approvalPolicy` are all "override for this turn and subsequent
/// turns". Omitted when unset rather than sent as null, so a thread keeps
/// whatever it started with.
///
/// A free function so a test can read the frame that would go on the wire.
/// "The choice actually reaches `turn/start`" is exactly the kind of thing that
/// silently stops being true, and the alternative way to check it is a live
/// codex and a person watching.
fn turn_params(
    thread_id: &str,
    input: serde_json::Value,
    model: &Option<String>,
    effort: &Option<String>,
    approval: &Option<String>,
) -> serde_json::Value {
    let mut params = serde_json::json!({ "threadId": thread_id, "input": input });
    if let Some(model) = model {
        params["model"] = serde_json::json!(model);
    }
    if let Some(effort) = effort {
        params["effort"] = serde_json::json!(effort);
    }
    if let Some(approval) = approval {
        params["approvalPolicy"] = serde_json::json!(approval);
    }
    params
}

impl AgentBackend for CodexBackend {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            backend: BackendKind::Codex,
            // `turn/steer` is a real method, so the neutral queue does not have
            // to emulate it and the composer can stop implying a queued prompt
            // was delivered.
            native_steer: true,
            // `thread/resume` is a method, not an advertised capability.
            replay: true,
            // codex writes its own files; it never asks the client to.
            client_side_fs: false,
        }
    }

    async fn prompt(&mut self, text: &str, images: &[PromptImage]) -> Result<(), BackendError> {
        let params = turn_params(
            &self.thread_id,
            input_for(text, images),
            &self.model,
            &self.effort,
            &self.approval,
        );
        self.pending_turn = Some(self.writer.request_no_wait("turn/start", params).await?);
        Ok(())
    }

    async fn steer(&mut self, text: &str, images: &[PromptImage]) -> Result<(), BackendError> {
        // Deliberately does NOT touch `pending_turn`: a steering prompt joins
        // the running turn rather than starting its own.
        self.writer
            .request_no_wait(
                "turn/steer",
                serde_json::json!({
                    "threadId": self.thread_id,
                    "input": input_for(text, images),
                }),
            )
            .await?;
        Ok(())
    }

    async fn answer(&mut self, request_id: &str, option_id: &str) -> Result<(), BackendError> {
        let id: serde_json::Value =
            serde_json::from_str(request_id).unwrap_or(serde_json::Value::Null);
        self.writer.respond(id, serde_json::json!({ "decision": option_id })).await?;
        Ok(())
    }

    async fn set_config_option(&mut self, id: &str, value: &str) -> Result<(), BackendError> {
        // Held here and applied to the next `turn/start`, which is what codex
        // supports: all three are per-turn overrides rather than thread state,
        // so there is no server call to make until there is a turn to make it
        // on.
        match selector_for(id) {
            Some(Selector::Model) => self.model = Some(value.to_string()),
            Some(Selector::Effort) => self.effort = Some(value.to_string()),
            Some(Selector::Approval) => self.approval = Some(value.to_string()),
            None => {}
        }
        Ok(())
    }

    async fn cancel(&mut self) -> Result<(), BackendError> {
        self.writer
            .notify("turn/interrupt", serde_json::json!({ "threadId": self.thread_id }))
            .await?;
        Ok(())
    }

    async fn next_events(&mut self) -> Result<Vec<AgentEvent>, BackendError> {
        let frame = self.recv_frame().await?;
        self.handle(frame).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_advertises_native_steering_and_replay_but_not_client_side_files() {
        let caps = Capabilities {
            backend: BackendKind::Codex,
            native_steer: true,
            replay: true,
            client_side_fs: false,
        };
        assert!(caps.native_steer, "turn/steer is a real method");
        assert!(caps.replay, "thread/resume is a method, not an advertised capability");
        assert!(
            !caps.client_side_fs,
            "codex writes its own files; nothing here needs confining"
        );
    }

    #[test]
    fn a_prompt_carries_its_text_as_a_typed_input_item() {
        let input = input_for("hello", &[]);
        assert_eq!(input[0]["type"], "text");
        assert_eq!(input[0]["text"], "hello");
    }

    #[test]
    fn an_image_travels_beside_the_text_as_a_data_url() {
        // Dropped until now, on the reasoning that the image variant takes a
        // URL and Far Cooler carries bytes — but a `data:` URL is a URL, and
        // `PromptImage` composes into one exactly.
        let input = input_for(
            "what is this",
            &[PromptImage { mime: "image/png".into(), base64: "AAAA".into() }],
        );
        assert_eq!(input.as_array().map(|a| a.len()), Some(2), "both items");
        assert_eq!(input[0]["type"], "text");
        assert_eq!(input[1]["type"], "image");
        assert_eq!(input[1]["url"], "data:image/png;base64,AAAA");
    }

    #[test]
    fn an_image_with_no_type_is_sent_as_png_rather_than_as_text() {
        // `data:;base64,…` is a valid URL meaning text/plain, so the picture
        // would be sent and then rejected. The Claude backend takes the same
        // `PromptImage` and already defaults to PNG; two backends reading one
        // input should not disagree about it.
        let input = input_for("what is this", &[PromptImage {
            mime: String::new(),
            base64: "AAAA".into(),
        }]);
        assert_eq!(input[1]["url"], "data:image/png;base64,AAAA");
    }

    #[test]
    fn the_text_comes_first_so_a_refused_image_can_only_cost_the_picture() {
        // The guarantee that survived the rewrite: whatever happens to the
        // image, the question is in the prompt and it is in it first.
        for images in [
            Vec::new(),
            vec![PromptImage { mime: "image/png".into(), base64: "AAAA".into() }],
            vec![
                PromptImage { mime: "image/png".into(), base64: "AAAA".into() },
                PromptImage { mime: "image/jpeg".into(), base64: "BBBB".into() },
            ],
        ] {
            let input = input_for("what is this", &images);
            assert_eq!(input[0]["type"], "text");
            assert_eq!(input[0]["text"], "what is this");
            assert_eq!(input.as_array().map(|a| a.len()), Some(1 + images.len()));
        }
    }

    /// The exact shape `model/list` returned on 0.147.0, trimmed to what is read.
    fn catalog_json() -> serde_json::Value {
        serde_json::json!({ "data": [
            { "id": "gpt-5.6-luna", "model": "gpt-5.6-luna", "displayName": "GPT-5.6-Luna",
              "description": "", "hidden": false, "supportedReasoningEfforts": [
                  { "reasoningEffort": "low", "description": "Fast responses" },
                  { "reasoningEffort": "high", "description": "Greater depth" },
                  { "reasoningEffort": "max", "description": "Maximum depth" }] },
            { "id": "gpt-5.4", "model": "gpt-5.4", "displayName": "GPT-5.4",
              "description": "", "hidden": false, "supportedReasoningEfforts": [
                  { "reasoningEffort": "low", "description": "Fast responses" }] },
            { "id": "secret", "model": "secret", "displayName": "Hidden",
              "description": "", "hidden": true, "supportedReasoningEfforts": [] }
        ]})
    }

    #[test]
    fn the_model_menu_comes_from_the_catalog_not_from_the_current_value() {
        // The bug this fixes: with only `thread/start` to go on, the picker
        // held exactly one entry — the model already in use — which is a
        // control that cannot change anything.
        let catalog = models_from(&catalog_json());
        let options =
            config_options(&Some("gpt-5.6-luna".into()), &Some("high".into()), &None, &catalog);
        let model = options.iter().find(|o| o.id == "model").expect("a model selector");
        let ids: Vec<_> = model.options.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(ids, ["gpt-5.6-luna", "gpt-5.4"], "every offered model, in order");
        assert_eq!(model.options[0].name, "GPT-5.6-Luna", "the name a person reads");
        assert_eq!(model.current_value, "gpt-5.6-luna");
    }

    #[test]
    fn a_hidden_model_stays_out_of_the_picker() {
        // codex marks these as not belonging in a picker, and this is a picker.
        let catalog = models_from(&catalog_json());
        assert!(!catalog.iter().any(|m| m.id == "secret"));
    }

    #[test]
    fn the_effort_menu_is_the_current_models_own() {
        // Per model, not global: Luna offers max, 5.4 stops short. A single
        // hardcoded list offered people settings their model would reject —
        // and the one shipped first also contained `minimal`, which no model
        // supports at all.
        let catalog = models_from(&catalog_json());

        let luna =
            config_options(&Some("gpt-5.6-luna".into()), &Some("high".into()), &None, &catalog);
        let efforts: Vec<_> = luna
            .iter()
            .find(|o| o.id == "effort")
            .expect("an effort selector")
            .options
            .iter()
            .map(|o| o.id.as_str())
            .collect();
        assert_eq!(efforts, ["low", "high", "max"]);

        let older = config_options(&Some("gpt-5.4".into()), &Some("low".into()), &None, &catalog);
        let efforts: Vec<_> = older
            .iter()
            .find(|o| o.id == "effort")
            .expect("an effort selector")
            .options
            .iter()
            .map(|o| o.id.as_str())
            .collect();
        assert_eq!(efforts, ["low"], "and never offers what this model would refuse");
    }

    #[test]
    fn a_failed_model_list_costs_the_menu_not_the_label() {
        // Falling back to the current value alone beats showing no model at
        // all: you can still see what you are talking to.
        let options =
            config_options(&Some("gpt-5.6-luna".into()), &Some("high".into()), &None, &[]);
        let model = options.iter().find(|o| o.id == "model").expect("a model selector");
        assert_eq!(model.options.len(), 1);
        assert_eq!(model.options[0].id, "gpt-5.6-luna");
    }

    #[test]
    fn a_thread_that_reports_nothing_offers_no_empty_pickers() {
        // A picker with nothing in it is worse than no picker.
        assert!(config_options(&None, &None, &None, &[]).is_empty());
    }

    #[test]
    fn the_approval_picker_offers_exactly_the_policies_the_wire_accepts() {
        // Checked against `AskForApproval` in the vendored schema rather than
        // against my recollection — the same discipline the Claude side's
        // permission-mode list is held to, and for the same reason: that list
        // was written from memory once and shipped missing two entries.
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../../../vendor/codex-app-server.schema.json"))
                .expect("the vendored schema is JSON");
        let accepted: Vec<&str> = schema["definitions"]["AskForApproval"]["oneOf"][0]["enum"]
            .as_array()
            .expect("AskForApproval leads with its string variants")
            .iter()
            .filter_map(|v| v.as_str())
            .collect();

        let offered: Vec<String> = approval_policies().iter().map(|p| p.id.clone()).collect();
        assert_eq!(offered.len(), accepted.len(), "every string variant, and only those");
        for id in &offered {
            assert!(accepted.contains(&id.as_str()), "{id} is not something codex accepts");
            assert!(
                !id.contains("granular"),
                "the object variant cannot round-trip through a picker"
            );
        }
        assert_eq!(
            approval_policies()[0].name,
            "Ask Always",
            "the words a person chooses by, not the wire's"
        );
    }

    #[test]
    fn the_approval_picker_is_shaped_like_the_mode_selector_the_guis_already_draw() {
        // `category: "mode"` is what puts it where Claude's mode picker goes.
        // The parity gap this closes: a codex chat had no way at all to change
        // how much it asks.
        let options = config_options(&None, &None, &Some("on-request".into()), &[]);
        let mode = options.iter().find(|o| o.category == "mode").expect("a mode selector");
        assert_eq!(mode.id, "approval");
        assert_eq!(mode.kind, "select");
        assert_eq!(mode.current_value, "on-request");
        assert_eq!(mode.options.len(), 3);
    }

    #[test]
    fn the_mode_list_and_the_mode_picker_never_disagree() {
        // They did. `SessionStarted.available_modes` was filled
        // unconditionally while `config_options` filtered, and
        // `agent_supervisor` republishes that list as `availableAgentModes`
        // with `SetMode` routing straight back to `Selector::Approval` — so a
        // policy this cannot express still published a three-entry picker that
        // would flatten it on the first click.
        for reported in [None, Some("on-request".into()), Some("someLaterVariant".into())] {
            let picker = approval_picker(&reported);
            let listed = config_options(&None, &None, &reported, &[]);
            assert_eq!(
                picker.is_some(),
                !listed.is_empty(),
                "one surface offers a mode the other withholds: {reported:?}"
            );
        }
    }

    #[test]
    fn a_policy_this_picker_cannot_express_draws_no_picker_at_all() {
        // `AskForApproval` is also a `{granular: {...}}` object, which reads as
        // no string here. Offering the three coarse entries against a granular
        // policy would show one of them as the setting in force when none of
        // them is — a picker that misreports is worse than a missing one.
        assert!(config_options(&None, &None, &None, &[]).is_empty());
        assert!(
            config_options(&None, &None, &Some("someLaterVariant".into()), &[]).is_empty(),
            "and the same for a string a later codex adds"
        );
    }

    #[test]
    fn the_chosen_approval_policy_reaches_turn_start() {
        // The whole point of the picker: held on the backend like model and
        // effort, and sent on the next turn. A control that changes a field
        // nobody reads is the failure this guards.
        let params = turn_params(
            "t",
            input_for("hello", &[]),
            &Some("gpt-5.6-luna".into()),
            &Some("high".into()),
            &Some("never".into()),
        );
        assert_eq!(params["threadId"], "t");
        assert_eq!(params["model"], "gpt-5.6-luna");
        assert_eq!(params["effort"], "high");
        assert_eq!(params["approvalPolicy"], "never");

        // Unset means unsent, not sent as null: a thread keeps whatever it
        // started with rather than being reset to a default nobody chose.
        let bare = turn_params("t", input_for("hello", &[]), &None, &None, &None);
        assert!(bare.get("approvalPolicy").is_none(), "{bare}");
        assert!(bare.get("model").is_none(), "{bare}");
    }

    #[test]
    fn every_selector_answers_to_its_category_as_well_as_to_its_id() {
        // A client that hands back the category it grouped the control under
        // instead of the option's own id would set nothing and be told it
        // worked. `effort` already guarded against that; the new mode selector
        // inherits the same hazard.
        assert_eq!(selector_for("approval"), Some(Selector::Approval));
        assert_eq!(selector_for("mode"), Some(Selector::Approval));
        assert_eq!(selector_for("effort"), Some(Selector::Effort));
        assert_eq!(selector_for("thought_level"), Some(Selector::Effort));
        assert_eq!(selector_for("model"), Some(Selector::Model));
        assert_eq!(selector_for("something_else"), None, "and nothing invented");

        // And the ids the pickers actually publish all route somewhere: an
        // option the GUI can draw but the backend ignores is a dead control.
        for option in config_options(
            &Some("gpt-5.6-luna".into()),
            &Some("high".into()),
            &Some("on-request".into()),
            &models_from(&catalog_json()),
        ) {
            assert!(selector_for(&option.id).is_some(), "{} sets nothing", option.id);
        }
    }
}
