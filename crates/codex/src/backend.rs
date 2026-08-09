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

        // Asked for rather than inferred: `thread/start` reports the model in
        // use but not what else is on offer, so without this the picker holds
        // exactly one entry — a control that cannot change anything. A failure
        // here costs the menu, not the session.
        let catalog = match conn.request("model/list", serde_json::json!({})).await {
            Ok(result) => models_from(&result),
            Err(_) => Vec::new(),
        };

        let mut prelude = vec![AgentEvent::SessionStarted {
            session_id: thread_id.clone(),
            agent_mode: None,
            available_modes: Vec::new(),
            model: model.clone(),
            config_options: config_options(&model, &effort, &catalog),
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

/// The selectors this thread offers, as the generic list clients render.
///
/// Model and reasoning effort, because those are what `turn/start` accepts as
/// per-turn overrides.
///
/// The effort menu is the CURRENT model's, which is a real limitation worth
/// naming: switching model cannot re-send the menu, because a consumer is
/// entitled to assume a session starts exactly once and `SessionStarted` is
/// where the options ride. Picking an effort the new model does not support is
/// refused by codex rather than silently misapplied.
fn config_options(
    model: &Option<String>,
    effort: &Option<String>,
    catalog: &[ModelInfo],
) -> Vec<ConfigOption> {
    let mut out = Vec::new();
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
/// Images are dropped with the text kept rather than the whole prompt refused:
/// `UserInput` has an image variant that takes a URL, and Far Cooler carries
/// image bytes inline, so there is nothing honest to put in it yet. Losing the
/// picture is bad; losing the question with it would be worse.
fn input_for(text: &str, _images: &[PromptImage]) -> serde_json::Value {
    serde_json::json!([{ "type": "text", "text": text }])
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
        let mut params = serde_json::json!({
            "threadId": self.thread_id,
            "input": input_for(text, images),
        });
        if let Some(model) = &self.model {
            params["model"] = serde_json::json!(model);
        }
        if let Some(effort) = &self.effort {
            params["effort"] = serde_json::json!(effort);
        }
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
        // supports: both are per-turn overrides rather than thread state, so
        // there is no server call to make until there is a turn to make it on.
        match id {
            "model" => self.model = Some(value.to_string()),
            "effort" | "thought_level" => self.effort = Some(value.to_string()),
            _ => {}
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
    fn an_image_is_dropped_but_never_takes_the_question_with_it() {
        // UserInput's image variant takes a URL and Far Cooler carries bytes,
        // so there is nothing honest to send yet. Losing the picture is bad;
        // losing the question too would be worse.
        let input = input_for(
            "what is this",
            &[PromptImage { mime: "image/png".into(), base64: "AAAA".into() }],
        );
        assert_eq!(input.as_array().map(|a| a.len()), Some(1));
        assert_eq!(input[0]["text"], "what is this");
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
        let options = config_options(&Some("gpt-5.6-luna".into()), &Some("high".into()), &catalog);
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

        let luna = config_options(&Some("gpt-5.6-luna".into()), &Some("high".into()), &catalog);
        let efforts: Vec<_> = luna
            .iter()
            .find(|o| o.id == "effort")
            .expect("an effort selector")
            .options
            .iter()
            .map(|o| o.id.as_str())
            .collect();
        assert_eq!(efforts, ["low", "high", "max"]);

        let older = config_options(&Some("gpt-5.4".into()), &Some("low".into()), &catalog);
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
        let options = config_options(&Some("gpt-5.6-luna".into()), &Some("high".into()), &[]);
        let model = options.iter().find(|o| o.id == "model").expect("a model selector");
        assert_eq!(model.options.len(), 1);
        assert_eq!(model.options[0].id, "gpt-5.6-luna");
    }

    #[test]
    fn a_thread_that_reports_nothing_offers_no_empty_pickers() {
        // A picker with nothing in it is worse than no picker.
        assert!(config_options(&None, &None, &[]).is_empty());
    }
}
