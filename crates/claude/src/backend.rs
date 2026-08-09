//! `AgentBackend`, as the Claude CLI performs it.

use farcooler_agent_core::backend::{
    AgentBackend, BackendError, BackendKind, Capabilities, Launch,
};
use farcooler_agent_core::event::{AgentChoice, AgentEvent, ConfigOption, PromptImage};

use crate::conn::{ClaudeConnection, ClaudeError, ClaudeWriter, Incoming};
use crate::normalize::{frame_to_events, init_from, permission_event};

impl From<ClaudeError> for BackendError {
    fn from(e: ClaudeError) -> Self {
        match e {
            ClaudeError::Spawn => BackendError::Spawn,
            ClaudeError::Closed => BackendError::Closed,
            ClaudeError::Refused(message) => BackendError::Refused(message),
        }
    }
}

/// A live Claude session.
pub struct ClaudeBackend {
    writer: ClaudeWriter,
    incoming: tokio::sync::mpsc::UnboundedReceiver<Incoming>,
    pub session_id: String,
    /// The input of each tool call we have seen, by id.
    ///
    /// Kept because answering a permission with `allow` requires echoing the
    /// tool's input back as `updatedInput` — the CLI rejects an allow without
    /// it. The request itself carries the input, so this is belt and braces
    /// for the case where it does not.
    tool_inputs: std::collections::HashMap<String, serde_json::Value>,
}

impl ClaudeBackend {
    /// Start the CLI, initialize, and take the session it announces.
    pub async fn start(
        launch: &Launch,
        worktree: std::path::PathBuf,
        resume: Option<String>,
    ) -> Result<(Self, Vec<AgentEvent>), BackendError> {
        // Resume is a LAUNCH FLAG here, not a request — so a session that
        // cannot be restored fails before any transcript exists, rather than
        // mid-conversation the way ACP's `session/load` does.
        //
        // `--session-id` when there is nothing to resume, so the id Far Cooler
        // handed the terminal is the one the CLI uses and a later reconnect
        // finds it.
        let mut args = crate::handshake::launch_args(&launch.args);
        // `--session-id` rather than waiting to be told: the CLI does not
        // announce a session until a prompt is sent, and a pane needs an id
        // before then so a reconnect can find it. Far Cooler assigned this one
        // at launch, which is the same id the terminal used.
        let session_id = resume.clone().unwrap_or_else(new_session_id);
        if resume.is_some() {
            args.push("--resume".to_string());
        } else {
            args.push("--session-id".to_string());
        }
        args.push(session_id.clone());

        let mut conn =
            ClaudeConnection::spawn(&launch.program, &args, &launch.env, worktree).await?;
        let (init_frame, seen) = conn.initialize().await?;
        let mut init = init_from(&init_frame);
        if init.session_id.is_empty() {
            init.session_id = session_id.clone();
        }

        let mut prelude = vec![AgentEvent::SessionStarted {
            session_id: init.session_id.clone(),
            agent_mode: init.permission_mode.clone(),
            available_modes: permission_modes(),
            model: init.model.clone(),
            available_models: init
                .models
                .iter()
                .map(|m| AgentChoice {
                    id: m.value.clone(),
                    name: m.name.clone(),
                    description: m.description.clone(),
                })
                .collect(),
            config_options: config_options(&init),
            available_commands: init.commands.clone(),
            backend: BackendKind::Claude.as_str().to_string(),
        }];
        // Anything the CLI said before announcing itself — the user's own hooks
        // fire first. Mostly nothing, but dropping them unread would be a
        // silent choice rather than a deliberate one.
        for frame in seen {
            prelude.extend(frame_to_events(&frame));
        }

        let session_id = init.session_id;
        let (writer, incoming) = conn.split();
        Ok((
            ClaudeBackend {
                writer,
                incoming,
                session_id,
                tool_inputs: std::collections::HashMap::new(),
            },
            prelude,
        ))
    }

    /// Wait for one frame, and do nothing else. Safe in a `select!`.
    pub async fn recv_frame(&mut self) -> Result<Incoming, BackendError> {
        self.incoming.recv().await.ok_or(BackendError::Closed)
    }

    /// Act on one frame.
    pub async fn handle(&mut self, incoming: Incoming) -> Result<Vec<AgentEvent>, BackendError> {
        match incoming {
            Incoming::Frame(frame) => {
                // Remember tool inputs on the way past, so a permission answer
                // can echo one back.
                if let Some(blocks) = frame["message"]["content"].as_array() {
                    for block in blocks {
                        if block["type"] == "tool_use"
                            && let Some(id) = block["id"].as_str()
                        {
                            self.tool_inputs.insert(id.to_string(), block["input"].clone());
                        }
                    }
                }
                Ok(frame_to_events(&frame))
            }
            Incoming::Control { request_id, request } => {
                match request["subtype"].as_str().unwrap_or_default() {
                    "can_use_tool" => Ok(vec![permission_event(&request_id, &request)]),
                    // Anything else the CLI asks is answered with a bare
                    // success rather than left hanging: an unanswered control
                    // request blocks the turn, which is worse than a poor
                    // answer to a question we do not understand.
                    _ => {
                        let _ = self
                            .writer
                            .answer_permission(&request_id, true, serde_json::json!({}))
                            .await;
                        Ok(Vec::new())
                    }
                }
            }
        }
    }
}

/// A session id for a conversation that does not have one yet.
///
/// A UUID assembled from the clock and the process, rather than a `uuid`
/// dependency: this crate has four, and one identifier is not worth a fifth.
/// The CLI only requires the shape.
fn new_session_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let pid = std::process::id() as u128;
    let mix = nanos ^ (pid << 64);
    let hex = format!("{mix:032x}");
    format!(
        "{}-{}-4{}-8{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[13..16],
        &hex[17..20],
        &hex[20..32]
    )
}

/// The permission modes the CLI accepts, as a picker.
///
/// A fixed list because `PermissionMode` in `sdk.d.ts` is a closed union — but
/// the list has to MATCH that union, and the first version of this did not. It
/// was written from memory with four entries and dropped `acceptEdits` and
/// `auto`, so the two modes people reach for most were simply missing while
/// the comment claimed the list came from the declarations.
/// `every_permission_mode_the_cli_accepts_is_offered` now checks it against
/// `vendor/claude-sdk.d.ts` rather than against my recollection.
///
/// Ordered by how much they let the agent do on its own, which is the axis a
/// person is actually choosing along — not the order the union happens to
/// declare them in. Descriptions are the CLI's own words.
fn permission_modes() -> Vec<AgentChoice> {
    [
        ("plan", "Plan", "Plan only — no tools run"),
        ("default", "Ask", "Prompts before anything dangerous"),
        ("acceptEdits", "Accept Edits", "File edits go through without asking"),
        ("auto", "Auto", "A model decides which prompts to approve"),
        ("dontAsk", "Don't Ask", "Never prompts — denies whatever is not pre-approved"),
        ("bypassPermissions", "Bypass", "Skips every permission check"),
    ]
    .iter()
    .map(|(id, name, description)| AgentChoice {
        id: (*id).to_string(),
        name: (*name).to_string(),
        description: (*description).to_string(),
    })
    .collect()
}

/// The selectors this session offers.
///
/// Four, and every one of them settable — mode and model through their own
/// control requests, effort and output style through `apply_flag_settings`.
/// The session also reports its subagents, and they are deliberately NOT
/// offered: nothing on this channel can select one, and a picker that cannot
/// change anything is worse than no picker.
fn config_options(init: &crate::normalize::Init) -> Vec<ConfigOption> {
    let mut out = Vec::new();

    if let Some(mode) = &init.permission_mode {
        out.push(ConfigOption {
            id: "mode".into(),
            name: "Mode".into(),
            description: String::new(),
            category: "mode".into(),
            kind: "select".into(),
            current_value: mode.clone(),
            options: permission_modes(),
        });
    }

    if let Some(current) = &init.model {
        let options: Vec<AgentChoice> = init
            .models
            .iter()
            .map(|m| AgentChoice {
                id: m.value.clone(),
                name: m.name.clone(),
                description: m.description.clone(),
            })
            .collect();
        out.push(ConfigOption {
            id: "model".into(),
            name: "Model".into(),
            description: String::new(),
            category: "model".into(),
            kind: "select".into(),
            current_value: current.clone(),
            // Falls back to the current value alone rather than an empty menu.
            options: if options.is_empty() {
                vec![AgentChoice {
                    id: current.clone(),
                    name: current.clone(),
                    description: String::new(),
                }]
            } else {
                options
            },
        });

        // The current model's own levels, for the reason codex needed the same:
        // they differ per model, and offering one a model would refuse is a
        // control that produces an error instead of an effect.
        let efforts = init
            .models
            .iter()
            .find(|m| &m.value == current)
            .map(|m| m.efforts.clone())
            .unwrap_or_default();
        if !efforts.is_empty() {
            out.push(ConfigOption {
                id: "effort".into(),
                name: "Reasoning".into(),
                description: String::new(),
                category: "thought_level".into(),
                kind: "select".into(),
                // The CLI reports the levels but not which one is in use, and
                // `medium` is its documented default.
                current_value: if efforts.iter().any(|e| e == "medium") {
                    "medium".to_string()
                } else {
                    efforts[0].clone()
                },
                options: efforts
                    .iter()
                    .map(|e| AgentChoice {
                        id: e.clone(),
                        name: e.clone(),
                        description: String::new(),
                    })
                    .collect(),
            });
        }
    }

    if let Some(style) = &init.output_style
        && !init.output_styles.is_empty()
    {
        out.push(ConfigOption {
            id: "output_style".into(),
            name: "Style".into(),
            description: String::new(),
            category: "output_style".into(),
            kind: "select".into(),
            current_value: style.clone(),
            options: init
                .output_styles
                .iter()
                .map(|s| AgentChoice {
                    id: s.clone(),
                    name: s.clone(),
                    description: String::new(),
                })
                .collect(),
        });
    }

    out
}

impl AgentBackend for ClaudeBackend {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            backend: BackendKind::Claude,
            // The CLI accepts a user message mid-turn and picks it up between
            // tool calls — the `streamInput` the SDK exposes.
            native_steer: true,
            // `--resume` is a launch flag, so replay is decided before a
            // session exists rather than advertised during one.
            replay: true,
            // The CLI does its own file IO and never asks the client to.
            client_side_fs: false,
        }
    }

    async fn prompt(&mut self, text: &str, _images: &[PromptImage]) -> Result<(), BackendError> {
        // Images are dropped and the text kept. The wire takes content blocks
        // that could carry them, but Far Cooler holds bytes and the block wants
        // a media source shape not yet verified against a live CLI — sending a
        // guess would fail the whole prompt. Losing the picture is bad; losing
        // the question with it is worse.
        Ok(self.writer.send_user(text).await?)
    }

    async fn steer(&mut self, text: &str, images: &[PromptImage]) -> Result<(), BackendError> {
        // Identical on the wire: a user message sent during a turn IS the
        // steer. The distinction lives in `ChatSession`, which decides whether
        // to hold it.
        self.prompt(text, images).await
    }

    async fn answer(&mut self, request_id: &str, option_id: &str) -> Result<(), BackendError> {
        let input = self
            .tool_inputs
            .get(request_id)
            .cloned()
            .unwrap_or(serde_json::json!({}));
        Ok(self.writer.answer_permission(request_id, option_id == "allow", input).await?)
    }

    async fn set_config_option(&mut self, id: &str, value: &str) -> Result<(), BackendError> {
        // Each setter carries its value in its own named field. Mode and model
        // have dedicated subtypes; effort and output style do not, and ride
        // `apply_flag_settings`, which takes an arbitrary settings object.
        let (subtype, fields) = match id {
            "mode" => ("set_permission_mode", serde_json::json!({ "mode": value })),
            "model" => ("set_model", serde_json::json!({ "model": value })),
            "effort" | "thought_level" => (
                "apply_flag_settings",
                serde_json::json!({ "settings": { "effort": value } }),
            ),
            "output_style" => (
                "apply_flag_settings",
                serde_json::json!({ "settings": { "outputStyle": value } }),
            ),
            // Unknown selectors are ignored rather than guessed at: sending a
            // subtype the CLI does not know is silently dropped, which would
            // move a control and change nothing.
            _ => return Ok(()),
        };
        self.writer.control(subtype, fields).await.map(|_| ()).map_err(Into::into)
    }

    async fn cancel(&mut self) -> Result<(), BackendError> {
        self.writer
            .control("interrupt", serde_json::json!({}))
            .await
            .map(|_| ())
            .map_err(Into::into)
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
    fn every_permission_mode_the_cli_accepts_is_offered() {
        // Read out of the pinned declarations rather than restated here,
        // because restating it is exactly how `acceptEdits` and `auto` went
        // missing — the two modes people reach for most — while the comment
        // above the list claimed it came from this union.
        let declarations = include_str!("../../../vendor/claude-sdk.d.ts");
        let union = declarations
            .lines()
            .find(|l| l.contains("declare type PermissionMode = "))
            .expect("sdk.d.ts must declare PermissionMode");
        let declared: std::collections::BTreeSet<String> = union
            .split('=')
            .nth(1)
            .expect("a union body")
            .split('|')
            .map(|part| part.trim().trim_end_matches(';').trim_matches('\'').to_string())
            .collect();
        assert!(declared.len() >= 5, "the union parsed to something implausible: {declared:?}");

        let offered: std::collections::BTreeSet<String> =
            permission_modes().iter().map(|m| m.id.clone()).collect();

        assert_eq!(offered, declared, "the picker and the CLI must agree on the whole set");
    }

    #[test]
    fn a_mode_is_named_the_way_a_person_would_choose_it() {
        // Not the way the wire spells it — the mistake `AgentChoice` was given
        // a `name` to fix, and one a picker offering `bypassPermissions` makes.
        let modes = permission_modes();
        let auto = modes.iter().find(|m| m.id == "auto").expect("auto is offered");
        assert_eq!(auto.name, "Auto");
        assert!(auto.description.contains("model"), "and says what it does: {auto:?}");
        assert!(modes.iter().any(|m| m.name == "Accept Edits"));
    }

    #[test]
    fn the_modes_run_from_least_autonomous_to_most() {
        // The axis a person is actually choosing along, rather than the order
        // the union happens to declare them in.
        let ids: Vec<_> = permission_modes().iter().map(|m| m.id.clone()).collect();
        assert_eq!(
            ids,
            ["plan", "default", "acceptEdits", "auto", "dontAsk", "bypassPermissions"]
        );
    }

    /// The exact shape `initialize` answered with on 2.1.226, trimmed.
    fn real_init() -> crate::normalize::Init {
        crate::normalize::init_from(&serde_json::json!({
            "current_permission_mode": "default",
            "output_style": "default",
            "available_output_styles": ["default", "Proactive"],
            "commands": [{ "name": "review", "description": "Review a PR" }],
            "models": [
                { "value": "default", "displayName": "Default (recommended)",
                  "description": "Opus 5 with 1M context",
                  "supportedEffortLevels": ["low", "medium", "high", "xhigh", "max"] },
                { "value": "haiku", "displayName": "Haiku 4.5", "description": "Fast",
                  "supportedEffortLevels": ["low", "medium"] }
            ]
        }))
    }

    #[test]
    fn a_session_offers_every_selector_it_can_actually_set() {
        // Four, not one. The session also reports its subagents and they are
        // deliberately absent: nothing on this channel selects one, and a
        // picker that cannot change anything is worse than no picker.
        let ids: Vec<_> = config_options(&real_init()).iter().map(|o| o.id.clone()).collect();
        assert_eq!(ids, ["mode", "model", "effort", "output_style"]);
    }

    #[test]
    fn the_model_menu_carries_the_names_a_person_reads() {
        let options = config_options(&real_init());
        let model = options.iter().find(|o| o.id == "model").expect("a model selector");
        assert_eq!(model.options.len(), 2);
        assert_eq!(model.options[0].name, "Default (recommended)");
        assert_eq!(model.options[0].id, "default", "and the value set_model wants");
    }

    #[test]
    fn the_effort_menu_is_the_current_models_own() {
        // Per model, as codex turned out to need too: offering a level the
        // model refuses is a control that produces an error, not an effect.
        let options = config_options(&real_init());
        let effort = options.iter().find(|o| o.id == "effort").expect("an effort selector");
        let ids: Vec<_> = effort.options.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(ids, ["low", "medium", "high", "xhigh", "max"]);
        assert_eq!(effort.current_value, "medium", "the CLI's documented default");
    }

    #[test]
    fn the_command_menu_keeps_the_descriptions_the_cli_sent() {
        // The initialize response sends objects; `system: init` sends bare
        // names. A list of names is a list you have to already know.
        let init = real_init();
        assert_eq!(init.commands[0].name, "review");
        assert_eq!(init.commands[0].description, "Review a PR");
    }

    #[test]
    fn a_session_that_reports_nothing_offers_no_empty_pickers() {
        assert!(config_options(&crate::normalize::Init::default()).is_empty());
    }
}
