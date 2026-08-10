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
    /// What each outstanding `can_use_tool` is asking about, keyed by the
    /// CONTROL REQUEST id — the id its answer will be sent under.
    ///
    /// Answering with `allow` requires echoing the tool's input back as
    /// `updatedInput`, which the CLI rejects an allow without. This used to be
    /// keyed by the TOOL_USE id instead, scraped off assistant blocks on their
    /// way past, while `answer` looked it up by the control request id that
    /// `permission_event` puts in `AgentEvent::Permission.id`. Those are two
    /// different identifiers — `toolu_01BJow…` against `46878485-200f-…` on a
    /// real 2.1.226 ask — so the lookup missed every single time and every
    /// Allow sent `{"behavior":"allow","updatedInput":{}}`. The control request
    /// carries the input itself, so remembering it under the id the answer
    /// arrives with is both simpler and correct.
    permission_inputs: std::collections::HashMap<String, serde_json::Value>,
    /// What each slash command does, by name, from whichever source last said.
    ///
    /// Needed because the two sources are not equally rich and the poorer one
    /// arrives second. The `initialize` response sends objects with
    /// descriptions; `system: init` sends bare names. Pushing the bare list
    /// through unchanged refreshes the menu at the cost of emptying every
    /// description in it — a picker that had said "Review a PR" a moment
    /// earlier and now says nothing. `commands_changed` sends objects again, so
    /// this fills the gap rather than papering over it permanently.
    command_help: std::collections::HashMap<String, String>,
    /// The `get_context_usage` request whose answer has not come back yet.
    ///
    /// The context meter needs a number the CLI only volunteers when asked, so
    /// a turn ending asks. Kept as an id because `control_response` correlates
    /// by one and nothing else does.
    usage_request: Option<String>,
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

        // Kept because the transcript path is derived from it, and `spawn`
        // takes ownership.
        let worktree_for_history = worktree.clone();
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

        // History has to be READ, not waited for. `--resume` attaches the
        // session so the agent keeps its context and replays nothing at all —
        // measured against 2.1.226, a resume sends four hook frames and the
        // control response and no conversation. A pane that waited for one
        // showed an empty chat, which is exactly what it did.
        if resume.is_some() {
            match transcript_for(&worktree_for_history, &init.session_id) {
                Some(path) => match std::fs::read_to_string(&path) {
                    Ok(raw) => {
                        let restored = crate::normalize::history_to_events(&raw);
                        if restored.is_empty() {
                            prelude.push(AgentEvent::Gap {
                                reason: farcooler_agent_core::event::AgentGapReason::LoadEmpty,
                            });
                        } else {
                            prelude.extend(restored);
                        }
                        prelude.push(AgentEvent::TurnEnded {
                            reason: farcooler_agent_core::event::EndReason::EndTurn,
                        });
                    }
                    Err(e) => prelude.push(AgentEvent::Gap {
                        reason: farcooler_agent_core::event::AgentGapReason::LoadFailed {
                            detail: format!("could not read {}: {e}", path.display()),
                        },
                    }),
                },
                // No file is the COMMON case rather than a failure: Claude Code
                // writes a transcript only once a turn has happened, and every
                // pane is handed a session id at launch. A chat opened and
                // never typed into has nothing to restore, and nothing was lost.
                None => prelude.push(AgentEvent::Gap {
                    reason: farcooler_agent_core::event::AgentGapReason::LoadEmpty,
                }),
            }
        }

        let session_id = init.session_id;
        // Kept before `init` is dropped: `system: init` will push the same
        // commands back as bare names, and these are the only descriptions
        // this session will ever be told.
        let command_help = init
            .commands
            .iter()
            .filter(|c| !c.description.is_empty())
            .map(|c| (c.name.clone(), c.description.clone()))
            .collect();
        let (writer, incoming) = conn.split();
        Ok((
            ClaudeBackend {
                writer,
                incoming,
                session_id,
                command_help,
                permission_inputs: std::collections::HashMap::new(),
                usage_request: None,
            },
            prelude,
        ))
    }

    /// A command list with the descriptions this session has already been
    /// told, so a refresh cannot cost the menu its words.
    ///
    /// Remembers as well as fills: whichever push carried descriptions is the
    /// one a later bare list is measured against.
    fn described(&mut self, commands: Vec<AgentChoice>) -> Vec<AgentChoice> {
        commands
            .into_iter()
            .map(|mut command| {
                if command.description.is_empty() {
                    command.description =
                        self.command_help.get(&command.name).cloned().unwrap_or_default();
                } else {
                    self.command_help.insert(command.name.clone(), command.description.clone());
                }
                command
            })
            .collect()
    }

    /// Wait for one frame, and do nothing else. Safe in a `select!`.
    pub async fn recv_frame(&mut self) -> Result<Incoming, BackendError> {
        self.incoming.recv().await.ok_or(BackendError::Closed)
    }

    /// Act on one frame.
    pub async fn handle(&mut self, incoming: Incoming) -> Result<Vec<AgentEvent>, BackendError> {
        match incoming {
            Incoming::Frame(frame) => {
                // A `control_response` carries no `message`, so the normalizer
                // is right to be silent about it — which is exactly why the
                // correlation has to happen here, where the id we asked under
                // is remembered.
                if self.usage_request.is_some()
                    && crate::handshake::control_response_id(&frame) == self.usage_request
                {
                    self.usage_request = None;
                    return Ok(usage_events(&frame["response"]["response"]));
                }

                // `system` frames are otherwise silent, and the menu rode in on
                // one. `init_from`'s comment claimed a session was refined by
                // `system: init`, but its only caller was `start`, which never
                // sees one — and `commands_changed` exists precisely to push a
                // new list mid-session (skills discovered in a subdirectory,
                // say) and was being dropped on the floor. So the picker showed
                // whatever `initialize` happened to say at launch, forever.
                //
                // `CommandsAvailable`, never a second `SessionStarted`: a
                // consumer is entitled to assume a session starts exactly once.
                if frame["type"] == "system" {
                    let subtype = frame["subtype"].as_str().unwrap_or_default();
                    if subtype == "init" || subtype == "commands_changed" {
                        let commands = self.described(init_from(&frame).commands);
                        if !commands.is_empty() {
                            return Ok(vec![AgentEvent::CommandsAvailable { commands }]);
                        }
                    }
                }

                let events = frame_to_events(&frame);
                // Ask for the context number when the turn stops, because the
                // CLI never volunteers it and the meter is dark without it. A
                // failed send is dropped rather than reported: a missing meter
                // is not worth failing a turn that has already succeeded.
                if events.iter().any(|e| matches!(e, AgentEvent::TurnEnded { .. })) {
                    self.usage_request =
                        self.writer.control("get_context_usage", serde_json::json!({})).await.ok();
                }
                Ok(events)
            }
            Incoming::Control { request_id, request } => {
                match request["subtype"].as_str().unwrap_or_default() {
                    "can_use_tool" => {
                        // The request carries the very input an allow has to
                        // echo back, under the very id the answer will use.
                        self.permission_inputs.insert(request_id.clone(), request["input"].clone());
                        Ok(vec![permission_event(&request_id, &request)])
                    }
                    // Anything else the CLI asks is answered rather than left
                    // hanging — an unanswered control request blocks the turn —
                    // but answered with a bare success, not a permission
                    // verdict. `SDKControlRequestInner` lets the CLI send
                    // `hook_callback`, `elicitation`, `request_user_dialog` and
                    // `mcp_message` on this channel, and each has its own
                    // response shape; this used to reply to all four with
                    // `{"behavior":"allow","updatedInput":{}}`, which is an
                    // answer to a question none of them asked. Answering each
                    // one properly is future work.
                    _ => {
                        let _ = self.writer.respond_success(&request_id).await;
                        Ok(Vec::new())
                    }
                }
            }
        }
    }
}

/// A `get_context_usage` answer, as the number the context meter draws.
///
/// `SDKControlGetContextUsageResponse` reports a dozen breakdowns; `Usage`
/// wants two, and `totalTokens`/`maxTokens` are them. Measured on 2.1.226 a
/// fresh session answered `{"totalTokens":25450,"maxTokens":1000000,…}`.
///
/// A zero `maxTokens` produces nothing at all — the same guard codex needs on
/// `contextWindow`, for the same reason: a number with nothing to measure it
/// against is not worth drawing, and dividing by it is worse.
fn usage_events(response: &serde_json::Value) -> Vec<AgentEvent> {
    match (response["totalTokens"].as_u64(), response["maxTokens"].as_u64()) {
        (Some(used), Some(size)) if size > 0 => vec![AgentEvent::Usage { used, size }],
        _ => Vec::new(),
    }
}

/// Where Claude Code keeps this session's transcript, if it wrote one.
///
/// `~/.claude/projects/<worktree, every non-alphanumeric turned into a
/// dash>/<session id>.jsonl`. The daemon already derives the same directory
/// name in `session_discovery::project_dir_name` to decide whether a terminal
/// can be resumed at all; this is the same rule, reimplemented rather than
/// shared because a backend crate must not depend on the daemon.
///
/// `None` when there is no file, which is a normal state and not an error.
pub fn transcript_for(worktree: &std::path::Path, session_id: &str) -> Option<std::path::PathBuf> {
    // The REALPATH, not the path as written. Claude Code munges the RESOLVED
    // cwd, so a worktree under `/tmp` lands in `-private-tmp-…` on macOS and
    // looking under `-tmp-…` finds an empty directory — a session that exists
    // reported as no history at all. `session_discovery::discover_claude_session`
    // already had to learn this; its comment records it as measured rather than
    // guessed, and this had it wrong.
    //
    // A path that cannot be resolved is used as written, for the same reason
    // that function gives: failing here would turn a missing directory into a
    // confusing error about a different one.
    let resolved = std::fs::canonicalize(worktree).unwrap_or_else(|_| worktree.to_path_buf());
    let dir: String = resolved
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let home = std::env::var("HOME").ok()?;
    let path = std::path::Path::new(&home)
        .join(".claude/projects")
        .join(dir)
        .join(format!("{session_id}.jsonl"));
    path.exists().then_some(path)
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

    async fn prompt(&mut self, text: &str, images: &[PromptImage]) -> Result<(), BackendError> {
        // Pictures included. They used to be dropped here on the grounds that
        // the block's media source shape was unverified; it is verified now,
        // against a live 2.1.226 — see `user_content`.
        Ok(self.writer.send_user(text, images).await?)
    }

    async fn steer(&mut self, text: &str, images: &[PromptImage]) -> Result<(), BackendError> {
        // Identical on the wire: a user message sent during a turn IS the
        // steer. The distinction lives in `ChatSession`, which decides whether
        // to hold it.
        self.prompt(text, images).await
    }

    async fn answer(&mut self, request_id: &str, option_id: &str) -> Result<(), BackendError> {
        // `remove`, not `get`: a request is answered once, and a map that only
        // ever grew would hold every tool input of the whole session.
        let input = self.permission_inputs.remove(request_id).unwrap_or(serde_json::json!({}));
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

    /// A backend whose CLI is `cat`, so everything it SENDS comes back as a
    /// frame and can be asserted on.
    ///
    /// The wire is line-delimited JSON in both directions and `cat` is a
    /// faithful echo of it, which makes it possible to test what an answer
    /// actually puts on stdin — the half of these bugs that no amount of
    /// normalizer testing could have caught, since the empty `updatedInput`
    /// was a correct-looking call with the wrong key.
    async fn echoing_backend() -> ClaudeBackend {
        let conn = ClaudeConnection::spawn(
            std::path::Path::new("/bin/cat"),
            &[],
            &Default::default(),
            std::env::temp_dir(),
        )
        .await
        .expect("cat must run");
        let (writer, incoming) = conn.split();
        ClaudeBackend {
            writer,
            incoming,
            session_id: "test".into(),
            command_help: std::collections::HashMap::new(),
            permission_inputs: std::collections::HashMap::new(),
            usage_request: None,
        }
    }

    /// The exact `can_use_tool` a live 2.1.226 sent, trimmed to what is read.
    ///
    /// The two ids are the point: `request_id` is a UUID the CLI minted for the
    /// ASK, `tool_use_id` names the assistant block. Keying the input by the
    /// second and looking it up by the first is the bug below.
    fn real_ask() -> (String, serde_json::Value) {
        (
            "46878485-200f-44e1-b27b-a70821eec0ff".to_string(),
            serde_json::json!({
                "subtype": "can_use_tool",
                "tool_name": "Bash",
                "input": { "command": "curl -s https://example.com/", "description": "Fetch it" },
                "tool_use_id": "toolu_01A5aKxtkbRTQKMLULnQRAWe",
            }),
        )
    }

    #[tokio::test]
    async fn an_allow_carries_the_tools_real_input_and_not_an_empty_object() {
        // The input was remembered under the TOOL_USE id and looked up under
        // the CONTROL REQUEST id, so it missed every time and every approval
        // sent `{"behavior":"allow","updatedInput":{}}` — which the CLI
        // rejects, meaning no Allow this pane offered had ever worked.
        let mut backend = echoing_backend().await;
        let (request_id, request) = real_ask();

        let events = backend
            .handle(Incoming::Control { request_id: request_id.clone(), request })
            .await
            .expect("the ask has to reach the pane");
        assert!(matches!(
            events.as_slice(),
            [AgentEvent::Permission { id, tool_call, .. }]
                if id == &request_id && tool_call == "toolu_01A5aKxtkbRTQKMLULnQRAWe"
        ));

        backend.answer(&request_id, "allow").await.expect("the answer has to be sent");

        let Incoming::Frame(sent) = backend.recv_frame().await.expect("cat echoes it") else {
            panic!("an answer is a control_response, not a request");
        };
        assert_eq!(sent["response"]["request_id"], request_id);
        assert_eq!(sent["response"]["response"]["behavior"], "allow");
        assert_eq!(
            sent["response"]["response"]["updatedInput"]["command"],
            "curl -s https://example.com/",
            "the CLI rejects an allow whose updatedInput is not the tool's own: {sent}"
        );
    }

    #[tokio::test]
    async fn a_question_that_is_not_a_permission_is_not_answered_with_a_verdict() {
        // `hook_callback`, `elicitation`, `request_user_dialog` and
        // `mcp_message` all arrive on this channel with their own response
        // shapes. Every one of them used to be answered
        // `{"behavior":"allow","updatedInput":{}}` — a permission verdict for a
        // question nobody asked.
        let mut backend = echoing_backend().await;
        let events = backend
            .handle(Incoming::Control {
                request_id: "fc-hook".into(),
                request: serde_json::json!({ "subtype": "hook_callback", "callback_id": "h1" }),
            })
            .await
            .expect("it still has to be answered or the turn hangs");
        assert!(events.is_empty(), "a hook callback is not conversation: {events:?}");

        let Incoming::Frame(sent) = backend.recv_frame().await.expect("cat echoes it") else {
            panic!("a response, not a request");
        };
        assert_eq!(sent["response"]["subtype"], "success");
        assert_eq!(sent["response"]["request_id"], "fc-hook");
        assert_eq!(sent["response"]["response"], serde_json::json!({}));
        assert!(
            sent["response"]["response"]["behavior"].is_null(),
            "a hook response is not a permission verdict: {sent}"
        );
    }

    #[tokio::test]
    async fn a_turn_ending_asks_what_the_context_window_costs() {
        // The CLI never volunteers the number, so nobody asking meant the
        // meter stayed dark while codex and ACP both drew one.
        let mut backend = echoing_backend().await;
        let ended = backend
            .handle(Incoming::Frame(serde_json::json!({
                "type": "result", "stop_reason": "end_turn"
            })))
            .await
            .expect("a turn has to be able to end");
        assert!(matches!(ended.as_slice(), [AgentEvent::TurnEnded { .. }]));

        let Incoming::Control { request_id, request } =
            backend.recv_frame().await.expect("cat echoes what we asked")
        else {
            panic!("we sent a control_request");
        };
        assert_eq!(request["subtype"], "get_context_usage");

        // And the answer, correlated by the id we asked under, becomes the
        // number. `control_response` carries no message, so the normalizer is
        // silent about it and this correlation cannot live there.
        let usage = backend
            .handle(Incoming::Frame(serde_json::json!({
                "type": "control_response",
                "response": { "subtype": "success", "request_id": request_id,
                              "response": { "totalTokens": 25450, "maxTokens": 1000000 } },
            })))
            .await
            .expect("the answer has to land");
        assert!(
            matches!(usage.as_slice(), [AgentEvent::Usage { used: 25450, size: 1000000 }]),
            "{usage:?}"
        );
    }

    #[test]
    fn a_context_number_with_nothing_to_measure_it_against_is_not_drawn() {
        // The guard codex needs on `contextWindow`, for the same reason: a
        // meter with a zero denominator is worse than no meter.
        assert!(usage_events(&serde_json::json!({ "totalTokens": 10, "maxTokens": 0 })).is_empty());
        assert!(usage_events(&serde_json::json!({ "totalTokens": 10 })).is_empty());
    }

    #[tokio::test]
    async fn the_command_menu_is_refreshed_when_the_cli_pushes_a_new_one() {
        // `init_from`'s comment claimed a session was refined by `system: init`
        // and nothing ever called it with one, so the picker showed whatever
        // `initialize` said at launch and never moved — including through
        // `commands_changed`, which exists precisely to push a new list.
        let mut backend = echoing_backend().await;

        // `system: init` sends bare names.
        let events = backend
            .handle(Incoming::Frame(serde_json::json!({
                "type": "system", "subtype": "init",
                "slash_commands": ["review", "ship"],
            })))
            .await
            .expect("a system frame is not an error");
        assert!(
            matches!(events.as_slice(), [AgentEvent::CommandsAvailable { commands }]
                if commands.len() == 2 && commands[0].name == "review"),
            "{events:?}"
        );

        // `commands_changed` sends objects, and keeps the descriptions.
        let events = backend
            .handle(Incoming::Frame(serde_json::json!({
                "type": "system", "subtype": "commands_changed",
                "commands": [{ "name": "review", "description": "Review a PR" }],
            })))
            .await
            .expect("a system frame is not an error");
        assert!(
            matches!(events.as_slice(), [AgentEvent::CommandsAvailable { commands }]
                if commands[0].description == "Review a PR"),
            "{events:?}"
        );

        // A session still starts exactly once: never a second SessionStarted,
        // which a consumer would read as a restart.
        assert!(!events.iter().any(|e| matches!(e, AgentEvent::SessionStarted { .. })));
    }

    #[tokio::test]
    async fn refreshing_the_menu_does_not_cost_it_the_words_it_already_had() {
        // The two sources are not equally rich and the poorer one arrives
        // second: `initialize` and `commands_changed` send descriptions,
        // `system: init` sends bare names. Passing the bare list straight
        // through refreshes the menu by emptying it — a picker that said
        // "Review a PR" a moment ago and now says nothing at all.
        let mut backend = echoing_backend().await;
        backend.command_help.insert("review".into(), "Review a PR".into());

        let events = backend
            .handle(Incoming::Frame(serde_json::json!({
                "type": "system", "subtype": "init", "slash_commands": ["review", "brand-new"],
            })))
            .await
            .expect("a system frame is not an error");
        let [AgentEvent::CommandsAvailable { commands }] = events.as_slice() else {
            panic!("{events:?}");
        };
        assert_eq!(commands[0].description, "Review a PR");
        // A name nobody has described yet stays blank rather than borrowing
        // someone else's words. A picker you have to already know still beats
        // no picker.
        assert_eq!(commands[1].name, "brand-new");
        assert_eq!(commands[1].description, "");
    }

    #[tokio::test]
    async fn an_ordinary_system_frame_stays_silent() {
        // 2.1.226 sends six hook frames before it says anything else, plus
        // `status` and `thinking_tokens` during a turn. None of them is a menu.
        let mut backend = echoing_backend().await;
        for subtype in ["hook_started", "hook_response", "status", "permission_denied"] {
            let events = backend
                .handle(Incoming::Frame(serde_json::json!({
                    "type": "system", "subtype": subtype
                })))
                .await
                .expect("still not an error");
            assert!(events.is_empty(), "{subtype} should be silent: {events:?}");
        }
    }
}
