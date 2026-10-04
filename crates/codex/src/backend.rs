//! `AgentBackend`, as codex app-server performs it.

use farcooler_agent_core::backend::{
    AgentBackend, BackendError, BackendKind, Capabilities, Launch, ReturnedSteer,
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
    ///
    /// That last sentence was a claim, not a fact, until `Incoming::Failure`
    /// existed: a `turn/start` that failed outright was answered with a
    /// JSON-RPC error, `classify` returned `None` for it, and the frame never
    /// reached `handle` at all. The field was set and nothing ever cleared it.
    pending_turn: Option<u64>,
    /// The id of the turn that is running, as codex names it.
    ///
    /// `turn/interrupt` requires it and `turn/steer` requires it as
    /// `expectedTurnId` (`TurnInterruptParams` and `TurnSteerParams` in the
    /// vendored schema). Neither was sent, so Stop went out as a notification
    /// codex had no method for and Send Now went out missing a required field.
    /// Learned from the `turn/start` reply or `turn/started`, whichever lands
    /// first, and forgotten on that turn's `turn/completed`.
    turn_id: Option<String>,
    /// Stop was pressed after `turn/start` went out but before codex named
    /// the turn. The interrupt goes out the moment the id arrives, rather than
    /// being dropped for want of one.
    interrupt_wanted: bool,
    /// Requests sent without waiting whose refusal someone has to hear about.
    ///
    /// `turn/start` has `pending_turn`; these are the others. Before this, an
    /// error answering a steer was matched against `pending_turn`, found not
    /// to be it, and dropped, so a Send Now codex refused looked delivered.
    awaiting: std::collections::HashMap<u64, Awaiting>,
    /// Send Now pressed after `turn/start` went out but before codex named
    /// the turn. Each goes out with `expectedTurnId` the moment the id
    /// arrives, the same way `interrupt_wanted` does.
    held_steers: Vec<ReturnedSteer>,
    /// Steers codex turned down, waiting for `ChatSession` to requeue them.
    /// See `AgentBackend::take_returned_steers`.
    returned: Vec<ReturnedSteer>,
    /// Chosen per turn rather than held by the server: `turn/start` takes
    /// `model` and `effort` overrides, so a selector change applies to the next
    /// turn instead of needing a new thread.
    model: Option<String>,
    effort: Option<String>,
    /// The thread's running token total, which each turn's spend is read
    /// off. See `crate::usage`.
    tokens: crate::usage::TokenLedger,
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
        let mut resume_failure: Option<String> = None;
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
                    Err(e) => {
                        resume_failure = Some(e.to_string());
                        (conn.request("thread/start", serde_json::json!({ "cwd": cwd })).await?, false)
                    }
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
        // A pane attached mid-turn heard no `turn/started`: it went out
        // before this connection existed. `thread/resume` reports the
        // thread's turns, and one still in progress is the one Stop stops.
        let mut turn_id = running_turn(&result["thread"]);
        for (method, params) in conn.take_pending() {
            track_turn(&mut turn_id, &method, &params);
            prelude.extend(frame_to_events(&method, &params, Origin::Replay));
        }

        // A resume that failed started a NEW thread, and the pane must say so
        // rather than present an empty conversation as the one asked for. The
        // common "nothing recorded yet" refusal is the empty case, not news.
        if let Some(detail) = resume_failure {
            let lower = detail.to_lowercase();
            let reason = if lower.contains("no rollout found") || lower.contains("not found") {
                farcooler_agent_core::event::AgentGapReason::LoadEmpty
            } else {
                farcooler_agent_core::event::AgentGapReason::LoadFailed { detail }
            };
            prelude.push(AgentEvent::Gap { reason });
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
                    if turn_id.is_none() {
                        turn_id = running_turn(&result["thread"]);
                    }
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
                turn_id,
                interrupt_wanted: false,
                awaiting: std::collections::HashMap::new(),
                held_steers: Vec::new(),
                returned: Vec::new(),
                model,
                effort,
                tokens: Default::default(),
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
                track_turn(&mut self.turn_id, &method, &params);
                // A turn that ended before codex named it to us needs no Stop.
                if method == "turn/completed" && self.pending_turn.is_none() {
                    self.interrupt_wanted = false;
                }
                self.send_held().await?;
                let mut events = frame_to_events(&method, &params, Origin::Live);
                let spent = self.tokens.observe(&method, &params, self.model.as_deref());
                events.extend(spent.map(|usage| AgentEvent::TurnUsage { usage }));
                Ok(events)
            }
            Incoming::Request { id, method, params } => {
                // Every approval the server can ask becomes one event. The id
                // travels as JSON text so the answer is routable — inventing a
                // new one here would make it unanswerable.
                let request_id = serde_json::to_string(&id).unwrap_or_default();
                Ok(vec![approval_event(&request_id, &method, &params)])
            }
            Incoming::Response { id, result } => {
                if id.as_u64() == self.pending_turn {
                    self.pending_turn = None;
                    // `TurnStartResponse.turn.id`. Usually ahead of
                    // `turn/started`, and Stop pressed in between is waiting
                    // on exactly this.
                    if self.turn_id.is_none() {
                        self.turn_id = result["turn"]["id"].as_str().map(str::to_string);
                    }
                    self.send_held().await?;
                } else if let Some(id) = id.as_u64() {
                    self.awaiting.remove(&id);
                }
                Ok(Vec::new())
            }
            Incoming::Failure { id, message } => {
                // A `turn/start` the server refused outright. No
                // `turn/completed` follows one of these, so this frame is the
                // only announcement the turn is over — and it was dropped in
                // `classify` until recently, which left the pane reporting an
                // agent that had stopped as still working.
                if id.as_u64() != self.pending_turn {
                    let Some(sent) = id.as_u64().and_then(|id| self.awaiting.remove(&id)) else {
                        // An id nothing is waiting on.
                        return Ok(Vec::new());
                    };
                    return Ok(self.refused(sent, &message).into_iter().collect());
                }
                self.pending_turn = None;
                self.interrupt_wanted = false;
                // Steers waiting on this turn's id have no turn to join. Back
                // to the queue, which sends them as the next turn.
                self.returned.append(&mut self.held_steers);
                // A failed end, not the server's sentence drawn as the agent
                // speaking — which is what this was, under `Refusal`, a word
                // that means the MODEL declined. The sentence ("unauthorized:
                // run `codex login`") is kept as the failure's detail, and its
                // kind is what a client can act on.
                let kind = farcooler_agent_core::event::classify_error(None, None, &message);
                Ok(vec![AgentEvent::TurnEnded {
                    reason: farcooler_agent_core::event::EndReason::Failed { kind, detail: message },
                }])
            }
        }
    }

    /// Send what was waiting on the turn's id, once there is one.
    async fn send_held(&mut self) -> Result<(), BackendError> {
        let Some(turn_id) = self.turn_id.clone() else { return Ok(()) };
        for steer in std::mem::take(&mut self.held_steers) {
            self.send_steer(&turn_id, steer).await?;
        }
        if self.interrupt_wanted {
            self.interrupt_wanted = false;
            let id = self
                .writer
                .request_no_wait("turn/interrupt", interrupt_params(&self.thread_id, &turn_id))
                .await?;
            self.awaiting.insert(id, Awaiting::Interrupt { turn: turn_id });
        }
        Ok(())
    }

    async fn send_steer(&mut self, turn_id: &str, steer: ReturnedSteer) -> Result<(), BackendError> {
        let id = self
            .writer
            .request_no_wait(
                "turn/steer",
                serde_json::json!({
                    "threadId": self.thread_id,
                    "expectedTurnId": turn_id,
                    "input": input_for(&steer.text, &steer.images),
                }),
            )
            .await?;
        self.awaiting.insert(id, Awaiting::Steer { turn: turn_id.to_string(), steer });
        Ok(())
    }

    /// What a refused Stop or Send Now means for the person looking.
    ///
    /// A refused steer always goes back to the queue: the message is the
    /// user's, and a refusal is never a reason to lose it. Whether anything is
    /// SAID depends on why:
    ///
    /// - The turn it was aimed at is over. The commonest case by far, and
    ///   benign: Stop has nothing left to stop, and the message simply starts
    ///   the next turn. Nothing is said.
    /// - The turn is still running and codex will not take it (a review or a
    ///   compaction cannot be steered). The message waits for the turn to end,
    ///   and that is said.
    ///
    /// The server's own sentence is logged, not shown: it carries turn ids and
    /// backticks, which are for whoever reads the log.
    fn refused(&mut self, sent: Awaiting, message: &str) -> Option<AgentEvent> {
        let (turn, what) = match &sent {
            Awaiting::Interrupt { turn } => (turn.as_str(), "turn/interrupt"),
            Awaiting::Steer { turn, .. } => (turn.as_str(), "turn/steer"),
        };
        let stale = self.turn_id.as_deref() != Some(turn) || stale_turn_refusal(message);
        tracing::info!(request = what, stale, detail = %message, "codex refused a request");
        let said = match sent {
            Awaiting::Interrupt { .. } if stale => return None,
            Awaiting::Interrupt { .. } => "Codex couldn’t stop this turn. Try again in a moment.",
            Awaiting::Steer { steer, .. } => {
                self.returned.push(steer);
                if stale {
                    return None;
                }
                if message.starts_with("cannot steer") {
                    "Codex can’t take new messages during this kind of turn, so yours will \
                     send when it ends."
                } else {
                    "Codex couldn’t add your message to this turn, so it will send when the \
                     turn ends."
                }
            }
        };
        Some(AgentEvent::Message {
            role: farcooler_agent_core::event::Role::Agent,
            text: said.to_string(),
            parent: None,
        })
    }
}

/// Whether codex refused because the turn it was aimed at is no longer
/// running.
///
/// The pinned schema says only that a steer "fails when it does not match the
/// currently active turn"; it defines no JSON-RPC error codes, and no refusal
/// was ever recorded. These sentences are codex's own, read from
/// `app-server/src/request_processors/turn_processor.rs` as it is compiled
/// into the installed codex-cli 0.153.4 binary (`strings`, not a run):
/// "no active turn to steer", "no active turn to interrupt", and "expected
/// active turn id `…` but found `…`". The id comparison in `refused` catches
/// the same race without relying on any of them.
fn stale_turn_refusal(message: &str) -> bool {
    message.contains("no active turn to") || message.contains("expected active turn id")
}

/// What a request sent without waiting was, and which turn it was aimed at,
/// so its refusal can be told apart from the turn simply having ended.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Awaiting {
    Steer { turn: String, steer: ReturnedSteer },
    Interrupt { turn: String },
}

/// The turn a thread reports as still running, if any.
///
/// `Thread.turns` is filled on `thread/resume` and `thread/read`, and a
/// `TurnStatus` of `inProgress` is a turn nobody has seen start.
fn running_turn(thread: &serde_json::Value) -> Option<String> {
    thread["turns"]
        .as_array()?
        .iter()
        .rev()
        .find(|t| t["status"] == "inProgress")
        .and_then(|t| t["id"].as_str())
        .map(str::to_string)
}

/// Follow which turn is running from the notifications that say so.
///
/// `turn/started` names it and `turn/completed` ends it. A `turn/completed`
/// for some other turn leaves the current one alone.
fn track_turn(turn_id: &mut Option<String>, method: &str, params: &serde_json::Value) {
    let named = params["turn"]["id"].as_str();
    match (method, named) {
        ("turn/started", Some(id)) => *turn_id = Some(id.to_string()),
        ("turn/completed", None) => *turn_id = None,
        ("turn/completed", named) if named == turn_id.as_deref() => *turn_id = None,
        _ => {}
    }
}

/// `TurnInterruptParams`: both fields are required.
fn interrupt_params(thread_id: &str, turn_id: &str) -> serde_json::Value {
    serde_json::json!({ "threadId": thread_id, "turnId": turn_id })
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
///
/// That last sentence now stands in deliberate contrast to `permission_modes`
/// in the claude backend, which does the opposite: it reproduces the agent's
/// own names and descriptions exactly, because claude publishes them. Codex
/// sends `approvalPolicy` as a bare string — `"untrusted"`, `"on-request"`,
/// `"never"` — with no name and no description anywhere on that wire, so there
/// is nothing to borrow and these three sentences are ours. Read the difference
/// as a decision, not as drift somebody missed.
///
/// Being ours, they follow Far Cooler's copy rules, curly apostrophe included
/// ("isn’t"). `"Don't Ask"` is the exception, and the harder call: it is our
/// rendering of `never` that happens to land on claude's word for a different
/// id. Straight, so that the two pickers agree when they sit side by side —
/// a curly one here would show a person `Don’t Ask` on codex and `Don't Ask` on
/// claude in the same list of panes.
fn approval_policies() -> Vec<AgentChoice> {
    [
        ("untrusted", "Ask Always", "Prompts before anything not already trusted"),
        ("on-request", "Ask When Needed", "Prompts only when the sandbox isn’t enough"),
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
/// only meaningful on the runner that codex runs on.
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
        //
        // `expectedTurnId` is required, and is how codex refuses a steer aimed
        // at a turn that has since ended — `refused` hands that one back.
        let steer = ReturnedSteer { text: text.to_string(), images: images.to_vec() };
        match (self.turn_id.clone(), self.pending_turn) {
            (Some(turn_id), _) => self.send_steer(&turn_id, steer).await,
            // `turn/start` is out and codex has not named the turn yet.
            (None, Some(_)) => {
                self.held_steers.push(steer);
                Ok(())
            }
            // No turn at all: the one the chat thinks is running ended and
            // the news has not reached it. Refused, so the message stays in
            // the queue, which that turn's end sends.
            (None, None) => Err(BackendError::Refused("no turn is running".into())),
        }
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
            // Refused, not ignored: `Ok` told the picker the control had moved
            // when nothing had.
            None => return Err(BackendError::Refused(format!("there is no `{id}` setting"))),
        }
        Ok(())
    }

    async fn cancel(&mut self) -> Result<(), BackendError> {
        // A request, not a notification: `turn/interrupt` is in the schema's
        // `ClientRequest` union with `id`, `threadId` and `turnId` required.
        // Sent as a notification without a turn id, Stop did nothing.
        //
        // With `turn/start` out and no id back yet, the interrupt waits for
        // the id. With no turn at all there is nothing to stop.
        if self.turn_id.is_none() && self.pending_turn.is_none() {
            return Ok(());
        }
        self.interrupt_wanted = true;
        self.send_held().await
    }

    fn take_returned_steers(&mut self) -> Vec<ReturnedSteer> {
        std::mem::take(&mut self.returned)
    }

    async fn next_events(&mut self) -> Result<Vec<AgentEvent>, BackendError> {
        let frame = self.recv_frame().await?;
        self.handle(frame).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod refusals;

    fn on(conn: CodexConnection, thread_id: &str) -> CodexBackend {
        let (writer, incoming) = conn.split();
        CodexBackend {
            writer,
            incoming,
            thread_id: thread_id.to_string(),
            pending_turn: None,
            turn_id: None,
            interrupt_wanted: false,
            awaiting: std::collections::HashMap::new(),
            held_steers: Vec::new(),
            returned: Vec::new(),
            model: None,
            effort: None,
            tokens: Default::default(),
            approval: None,
        }
    }

    /// The thread and turn of the recorded session in `turn_basic.jsonl`.
    const THREAD: &str = "019fe879-59c8-71c2-bdff-6399e868d62f";
    const TURN: &str = "019fe879-657e-7b90-a8e8-007dfeec7a4a";

    // SYNTHETIC refusals. No codex refusal of `turn/interrupt` or `turn/steer`
    // has been recorded, and the schema defines no JSON-RPC error codes. The
    // MESSAGES are codex's own, read with `strings` from the installed
    // codex-cli 0.153.4 binary (app-server turn_processor.rs); the code
    // -32600 and the framing around them are invented. Replace with recorded
    // frames when one is captured: that takes a real codex driven by
    // `tests/fixtures/capture_turn.py` through a Stop and a Send Now at a
    // turn's end, which is a run on the owner's machine, not a test.
    const SYNTHETIC_NO_TURN_TO_INTERRUPT: &str =
        r#"{"id":2,"error":{"code":-32600,"message":"no active turn to interrupt"}}"#;
    const SYNTHETIC_NO_TURN_TO_STEER: &str =
        r#"{"id":2,"error":{"code":-32600,"message":"no active turn to steer"}}"#;
    const SYNTHETIC_REVIEW_NOT_STEERABLE: &str =
        r#"{"id":2,"error":{"code":-32600,"message":"cannot steer a review turn"}}"#;
    const SYNTHETIC_INTERRUPT_FAILED: &str =
        r#"{"id":2,"error":{"code":-32603,"message":"failed to interrupt turn: channel closed"}}"#;

    /// A fake app-server: `/bin/sh` running `script`, with every line it reads
    /// copied to a capture file so a test can hold the exact frames sent.
    ///
    /// `$FIX` is the recorded session from a real codex, so the frames this
    /// server answers with are codex's own: `fix 9 1` replays line 9 (the
    /// `turn/start` reply) renumbered to answer request 1, `fix 11` replays
    /// `turn/started`, `fix 29` replays `turn/completed`. `take` reads one
    /// frame and captures it; `$REFUSAL` is the test's one error frame.
    async fn fake_app_server(script: &str, refusal: &str) -> (CodexBackend, Capture) {
        let (launch, capture) = fake_launch(script, refusal);
        let conn = CodexConnection::spawn(
            &launch.program,
            &crate::handshake::launch_args(&launch.args),
            &launch.env,
            capture.0.clone(),
        )
        .await
        .expect("spawn a fake app-server");
        (on(conn, THREAD), capture)
    }

    /// The same fake, as a `Launch` that `CodexBackend::start` can run.
    ///
    /// `/bin/sh app-server`, run in the fake's own directory: `start` puts
    /// `app-server` first on the command line, so sh reads the script by
    /// that name. Nothing freshly written is executed, so the spawn can't
    /// fail with ETXTBSY (a new executable exec'd while another test thread
    /// forks with it still open for writing), as it did on CI's Linux.
    /// Spawn it with the scratch directory as the working directory.
    fn fake_launch(script: &str, refusal: &str) -> (Launch, Capture) {
        // One directory per call, named by a process-wide counter: the tests
        // run in parallel in one process, and a clock-derived name collided
        // (two tests wrote to, and one deleted, the same file).
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "codex-fake-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        let capture = Capture(dir);
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/turn_basic.jsonl");
        // `printf '%s\n'`, not `echo`: dash's echo rewrites backslashes, and
        // JSON is full of them.
        let prelude = r#"take() { IFS= read -r line; printf '%s\n' "$line" >> "$CAP"; }
fix() { if [ -n "$2" ]; then sed -n "$1p" "$FIX" | sed "s/^{\"id\":[0-9]*,/{\"id\":$2,/"; else sed -n "$1p" "$FIX"; fi; }
refuse() { printf '%s\n' "$REFUSAL"; }
"#;
        std::fs::write(capture.0.join("app-server"), format!("{prelude}{script}\n"))
            .expect("write the fake server");
        let program = std::path::PathBuf::from("/bin/sh");
        let env = std::collections::BTreeMap::from([
            ("CAP".to_string(), capture.file().display().to_string()),
            ("FIX".to_string(), fixture.display().to_string()),
            ("REFUSAL".to_string(), refusal.to_string()),
            ("TURN".to_string(), TURN.to_string()),
        ]);
        (Launch { program, args: Vec::new(), env }, capture)
    }

    /// The fake server's scratch directory, removed when the test ends.
    struct Capture(std::path::PathBuf);

    impl Capture {
        fn file(&self) -> std::path::PathBuf {
            self.0.join("sent.jsonl")
        }

        /// Every frame the fake server read, in order.
        fn frames(&self) -> Vec<serde_json::Value> {
            std::fs::read_to_string(self.file())
                .unwrap_or_default()
                .lines()
                .map(|l| serde_json::from_str(l).expect("a frame is JSON"))
                .collect()
        }
    }

    impl Drop for Capture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    async fn next(backend: &mut CodexBackend) -> Vec<AgentEvent> {
        tokio::time::timeout(std::time::Duration::from_secs(5), backend.next_events())
            .await
            .expect("the fake server answered")
            .expect("a frame, not a closed connection")
    }

    /// A frame is a `ClientRequest` the pinned codex accepts: the method's
    /// variant in the vendored union, with `id` and every required param.
    fn assert_schema_accepts(frame: &serde_json::Value) {
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../../../vendor/codex-app-server.schema.json"))
                .expect("the vendored schema is JSON");
        let method = frame["method"].as_str().expect("a method");
        let variant = schema["definitions"]["ClientRequest"]["oneOf"]
            .as_array()
            .expect("ClientRequest is a union")
            .iter()
            .find(|v| v["properties"]["method"]["enum"][0] == method)
            .unwrap_or_else(|| panic!("{method} is not a request codex takes"));
        for key in variant["required"].as_array().expect("required members") {
            let key = key.as_str().expect("a name");
            assert!(frame.get(key).is_some(), "{method} needs `{key}`: {frame}");
        }
        let params = variant["properties"]["params"]["$ref"]
            .as_str()
            .and_then(|r| r.strip_prefix("#/definitions/"))
            .expect("params by reference");
        for key in schema["definitions"][params]["required"].as_array().expect("required params") {
            let key = key.as_str().expect("a name");
            assert!(frame["params"].get(key).is_some(), "{params} needs `{key}`: {frame}");
        }
    }

    fn texts(steers: &[ReturnedSteer]) -> Vec<&str> {
        steers.iter().map(|s| s.text.as_str()).collect()
    }

    #[tokio::test]
    async fn stop_sends_turn_interrupt_as_a_request_naming_the_running_turn() {
        // It went out as a notification carrying only `threadId`, which the
        // pinned schema has no notification for and whose request form
        // requires `turnId`. Stop did nothing on a codex pane.
        let (mut backend, capture) = fake_app_server(
            r#"take; fix 9 1; fix 11; take; printf '{"id":2,"result":{}}\n'; read -r done"#,
            "",
        )
        .await;
        backend.prompt("hello", &[]).await.expect("the turn goes out");
        next(&mut backend).await; // the `turn/start` reply
        next(&mut backend).await; // `turn/started`
        backend.cancel().await.expect("Stop goes out");
        let events = next(&mut backend).await; // the interrupt's reply
        assert!(events.is_empty(), "an accepted Stop says nothing; turn/completed ends the turn");

        let frames = capture.frames();
        assert_eq!(frames.len(), 2, "{frames:?}");
        assert_eq!(
            frames[1],
            serde_json::json!({
                "id": 2,
                "method": "turn/interrupt",
                "params": { "threadId": THREAD, "turnId": TURN },
            })
        );
        assert_schema_accepts(&frames[1]);
        assert!(backend.awaiting.is_empty(), "its reply was matched to it");
    }

    #[tokio::test]
    async fn stop_pressed_before_codex_names_the_turn_goes_out_once_it_does() {
        // The gap between sending `turn/start` and hearing its id back. Stop
        // there has no turn to name; it waits for one rather than vanishing.
        let (mut backend, capture) = fake_app_server(
            r#"take; fix 9 1; take; printf '{"id":2,"result":{}}\n'; read -r done"#,
            "",
        )
        .await;
        backend.prompt("hello", &[]).await.expect("the turn goes out");
        backend.cancel().await.expect("Stop is held, not refused");
        next(&mut backend).await; // the `turn/start` reply, which names the turn
        next(&mut backend).await; // the interrupt's reply

        let frames = capture.frames();
        assert_eq!(frames.len(), 2, "exactly one interrupt: {frames:?}");
        assert_eq!(frames[1]["method"], "turn/interrupt");
        assert_eq!(frames[1]["params"]["turnId"], TURN);
        assert_schema_accepts(&frames[1]);
    }

    #[tokio::test]
    async fn stop_on_a_pane_attached_mid_turn_names_the_turn_the_thread_reports() {
        // `turn/started` went out before this connection existed, so only the
        // thread's own report says a turn is running. Stop was a silent no-op.
        //
        // The handshake replies are the recorded ones; the `thread/resume`
        // reply is SYNTHETIC (no mid-turn resume was recorded), trimmed to
        // the members `start` reads, with the turn shaped as `Thread.turns`
        // declares it.
        let (launch, capture) = fake_launch(
            r#"take; fix 1 1; take
take; printf '{"id":2,"result":{"thread":{"id":"%s","turns":[{"id":"earlier","status":"completed","items":[]},{"id":"%s","status":"inProgress","items":[]}]},"model":"gpt-5.6-luna","approvalPolicy":"on-request"}}\n' "$THREAD" "$TURN"
take; printf '{"id":3,"error":{"code":-32601,"message":"synthetic"}}\n'
take; printf '{"id":4,"result":{"thread":{"id":"%s","turns":[]}}}\n' "$THREAD"
take; printf '{"id":5,"result":{}}\n'; read -r done"#,
            "",
        );
        let mut launch = launch;
        launch.env.insert("THREAD".into(), THREAD.into());
        let (mut backend, _prelude) =
            CodexBackend::start(&launch, capture.0.clone(), Some(THREAD.into()))
                .await
                .expect("the fake handshake completes");
        backend.cancel().await.expect("Stop goes out");
        next(&mut backend).await; // the interrupt's reply

        let frames = capture.frames();
        assert_eq!(frames[2]["method"], "thread/resume", "{frames:?}");
        let interrupt = frames.last().expect("frames");
        assert_eq!(interrupt["method"], "turn/interrupt", "{frames:?}");
        assert_eq!(interrupt["params"]["turnId"], TURN);
        assert_schema_accepts(interrupt);

        // And the recorded `thread/read` of a finished thread names none.
        let recorded: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/thread_read.json"))
                .expect("the recorded thread/read");
        assert_eq!(running_turn(&recorded["thread"]), None);
    }

    #[tokio::test]
    async fn a_stop_refused_for_a_live_turn_is_said_in_plain_words() {
        let (mut backend, _capture) = fake_app_server(
            r#"take; fix 9 1; fix 11; take; refuse; read -r done"#,
            SYNTHETIC_INTERRUPT_FAILED,
        )
        .await;
        backend.prompt("hello", &[]).await.expect("the turn goes out");
        next(&mut backend).await;
        next(&mut backend).await;
        backend.cancel().await.expect("Stop goes out");
        let events = next(&mut backend).await;

        assert_eq!(
            events,
            vec![AgentEvent::Message {
                role: farcooler_agent_core::event::Role::Agent,
                text: "Codex couldn’t stop this turn. Try again in a moment.".into(),
                parent: None,
            }],
            "said, without the server's text, and without ending the turn itself"
        );
    }

    #[tokio::test]
    async fn a_stop_that_lands_after_the_turn_ended_says_nothing() {
        // Stop pressed as the turn finished: codex completes the turn, then
        // refuses an interrupt with nothing left to interrupt. Nothing went
        // wrong, so nothing is said.
        let (mut backend, _capture) = fake_app_server(
            r#"take; fix 9 1; fix 11; take; fix 29; refuse; read -r done"#,
            SYNTHETIC_NO_TURN_TO_INTERRUPT,
        )
        .await;
        backend.prompt("hello", &[]).await.expect("the turn goes out");
        next(&mut backend).await;
        next(&mut backend).await;
        backend.cancel().await.expect("Stop goes out");
        let ended = next(&mut backend).await; // the recorded `turn/completed`
        assert!(ended.iter().any(|e| matches!(e, AgentEvent::TurnEnded { .. })), "{ended:?}");
        let events = next(&mut backend).await;
        assert!(events.is_empty(), "a stale Stop is not an error: {events:?}");
    }

    #[tokio::test]
    async fn a_live_turns_stop_refusal_is_said_even_when_the_turn_ends_right_after() {
        // Refused BEFORE `turn/completed`, for a reason that is not the turn
        // being gone: the turn was still running when Stop failed, so the
        // person is told, and the completion that follows does not unsay it.
        let (mut backend, _capture) = fake_app_server(
            r#"take; fix 9 1; fix 11; take; refuse; fix 29; read -r done"#,
            SYNTHETIC_INTERRUPT_FAILED,
        )
        .await;
        backend.prompt("hello", &[]).await.expect("the turn goes out");
        next(&mut backend).await;
        next(&mut backend).await;
        backend.cancel().await.expect("Stop goes out");
        let refused = next(&mut backend).await;
        assert_eq!(
            refused,
            vec![AgentEvent::Message {
                role: farcooler_agent_core::event::Role::Agent,
                text: "Codex couldn’t stop this turn. Try again in a moment.".into(),
                parent: None,
            }],
            "not silent"
        );
        let ended = next(&mut backend).await;
        assert!(ended.iter().any(|e| matches!(e, AgentEvent::TurnEnded { .. })), "{ended:?}");
    }

    #[tokio::test]
    async fn codexs_own_no_turn_answer_is_benign_even_before_turn_completed_arrives() {
        // The case the text match exists for: codex finished the turn and
        // answered the Stop and the Send Now before its `turn/completed`
        // reached us, so the id we track still names the turn. Only codex's
        // words say it is over. Exact 0.153.4 text (SYNTHETIC framing).
        for (refusal, steer) in [
            (SYNTHETIC_NO_TURN_TO_INTERRUPT, false),
            (SYNTHETIC_NO_TURN_TO_STEER, true),
        ] {
            let (mut backend, _capture) = fake_app_server(
                r#"take; fix 9 1; fix 11; take; refuse; fix 29; read -r done"#,
                refusal,
            )
            .await;
            backend.prompt("hello", &[]).await.expect("the turn goes out");
            next(&mut backend).await;
            next(&mut backend).await;
            if steer {
                backend.steer("also this", &[]).await.expect("the steer goes out");
            } else {
                backend.cancel().await.expect("Stop goes out");
            }
            assert_eq!(backend.turn_id.as_deref(), Some(TURN), "still tracked as running");
            let events = next(&mut backend).await;
            assert!(events.is_empty(), "{refusal}: nothing went wrong: {events:?}");
            let returned = backend.take_returned_steers();
            if steer {
                assert_eq!(texts(&returned), ["also this"], "never lost");
            }
        }
    }

    #[test]
    fn the_stale_turn_texts_are_codexs_own_exactly() {
        // Read with `strings` from the installed codex-cli 0.153.4 binary,
        // beside `app-server/src/request_processors/turn_processor.rs`. A
        // codex that rewords these makes a stale Stop say "Try again", which
        // is loud rather than silent; this table is what notices first.
        for stale in [
            "no active turn to interrupt",
            "no active turn to steer",
            "expected active turn id `019fe879-657e` but found `019fe879-9999`",
            "expected active turn id  but found ",
        ] {
            assert!(stale_turn_refusal(stale), "{stale:?} means the turn is over");
        }
        for live in [
            "cannot steer a review turn",
            "cannot steer a compact turn",
            "failed to interrupt turn: channel closed",
            "failed to steer turn: channel closed",
            "completed rollout item has no active turn",
            "expectedTurnId must not be empty",
        ] {
            assert!(!stale_turn_refusal(live), "{live:?} is not the turn being over");
        }
    }

    #[tokio::test]
    async fn send_now_steers_the_named_turn() {
        // `turn/steer` went out without `expectedTurnId`, which the schema
        // requires.
        let (mut backend, capture) = fake_app_server(
            r#"take; fix 9 1; fix 11; take; printf '{"id":2,"result":{"turnId":"%s"}}\n' "$TURN"; read -r done"#,
            "",
        )
        .await;
        backend.prompt("hello", &[]).await.expect("the turn goes out");
        next(&mut backend).await;
        next(&mut backend).await;
        backend.steer("also this", &[]).await.expect("the steer goes out");
        next(&mut backend).await;

        let frames = capture.frames();
        assert_eq!(
            frames[1],
            serde_json::json!({
                "id": 2,
                "method": "turn/steer",
                "params": {
                    "threadId": THREAD,
                    "expectedTurnId": TURN,
                    "input": [{ "type": "text", "text": "also this" }],
                },
            })
        );
        assert_schema_accepts(&frames[1]);
        assert!(backend.take_returned_steers().is_empty(), "accepted, so nothing comes back");
    }

    #[tokio::test]
    async fn send_now_that_races_the_turns_end_comes_back_and_says_nothing() {
        // The commonest way a steer fails: the turn finished while the
        // message was on the wire. It was dropped, after the pane had already
        // shown it as sent. Now it comes back to be sent as the next turn.
        let (mut backend, _capture) = fake_app_server(
            r#"take; fix 9 1; fix 11; take; fix 29; refuse; read -r done"#,
            SYNTHETIC_NO_TURN_TO_STEER,
        )
        .await;
        backend.prompt("hello", &[]).await.expect("the turn goes out");
        next(&mut backend).await;
        next(&mut backend).await;
        backend.steer("also this", &[]).await.expect("the steer goes out");
        next(&mut backend).await; // `turn/completed`
        let events = next(&mut backend).await;
        assert!(events.is_empty(), "the turn simply ended; nothing to apologize for: {events:?}");
        assert_eq!(texts(&backend.take_returned_steers()), ["also this"], "never lost");
    }

    #[tokio::test]
    async fn send_now_into_a_turn_that_cannot_be_steered_waits_and_says_so() {
        let (mut backend, _capture) = fake_app_server(
            r#"take; fix 9 1; fix 11; take; refuse; read -r done"#,
            SYNTHETIC_REVIEW_NOT_STEERABLE,
        )
        .await;
        backend.prompt("hello", &[]).await.expect("the turn goes out");
        next(&mut backend).await;
        next(&mut backend).await;
        backend.steer("also this", &[]).await.expect("the steer goes out");
        let events = next(&mut backend).await;
        assert_eq!(
            events,
            vec![AgentEvent::Message {
                role: farcooler_agent_core::event::Role::Agent,
                text: "Codex can’t take new messages during this kind of turn, so yours will \
                       send when it ends."
                    .into(),
                parent: None,
            }]
        );
        assert_eq!(texts(&backend.take_returned_steers()), ["also this"], "never lost");
    }

    #[tokio::test]
    async fn send_now_before_codex_names_the_turn_goes_out_once_it_does() {
        // Nothing to aim `expectedTurnId` at yet. Held, like Stop, rather
        // than refused into a log line nobody reads.
        let (mut backend, capture) = fake_app_server(
            r#"take; fix 9 1; take; printf '{"id":2,"result":{"turnId":"%s"}}\n' "$TURN"; read -r done"#,
            "",
        )
        .await;
        backend.prompt("hello", &[]).await.expect("the turn goes out");
        backend.steer("also this", &[]).await.expect("held, not refused");
        next(&mut backend).await; // the `turn/start` reply names the turn
        next(&mut backend).await; // the steer's reply

        let frames = capture.frames();
        assert_eq!(frames.len(), 2, "{frames:?}");
        assert_eq!(frames[1]["method"], "turn/steer");
        assert_eq!(frames[1]["params"]["expectedTurnId"], TURN);
        assert_schema_accepts(&frames[1]);
    }

    #[test]
    fn the_running_turn_is_followed_through_the_recorded_session() {
        // Replayed from a real codex: named by `turn/started`, forgotten by
        // its own `turn/completed`.
        let mut turn = None;
        for line in include_str!("../tests/fixtures/turn_basic.jsonl").lines() {
            let frame: serde_json::Value = serde_json::from_str(line).expect("JSON");
            if let Some(method) = frame["method"].as_str() {
                track_turn(&mut turn, method, &frame["params"]);
                if method == "turn/started" {
                    assert_eq!(turn.as_deref(), Some(TURN));
                }
            }
        }
        assert_eq!(turn, None, "the recorded turn completed");
    }

    #[tokio::test]
    async fn a_turn_the_server_refuses_ends_instead_of_working_forever() {
        // The native twin of the ACP hang. `turn/start` reports its end with
        // the `turn/completed` notification, and a `turn/start` that FAILS
        // never sends one — the error reply on its own id is the only word
        // that the turn is over. `classify` dropped it, so `pending_turn`
        // stayed set and the pane said Working for a server that had already
        // refused. "unauthorized" is in the schema's own `codexErrorInfo`
        // enum, so an agent asking a human to sign in lands here for real.
        //
        // Driven from the wire, because the drop happened in `classify`: a
        // test that starts after classification cannot see it.
        let args = vec![
            "-c".to_string(),
            // Refuses the turn and then blocks on stdin, so stdout stays open.
            // A server that EXITED would end the turn by the `Closed` path
            // instead, and this would pass without the fix.
            r#"read line; printf '{"id":1,"error":{"code":-32000,"message":"unauthorized","data":{"details":"run `codex login`"}}}\n'; read done"#
                .to_string(),
        ];
        let conn = crate::conn::CodexConnection::spawn(
            std::path::Path::new("/bin/sh"),
            &args,
            &Default::default(),
            std::env::temp_dir(),
        )
        .await
        .expect("spawn a fake app-server");
        let mut backend = on(conn, "t");
        backend.prompt("hello", &[]).await.expect("the turn goes out");
        assert_eq!(backend.pending_turn, Some(1));

        let events =
            tokio::time::timeout(std::time::Duration::from_secs(5), backend.next_events())
                .await
                .expect("a refused turn must not hang the pane")
                .expect("a refusal is handled, not a connection error");

        assert!(
            matches!(events.last(), Some(AgentEvent::TurnEnded { .. })),
            "the turn has to end, or activity stays Working: {events:?}"
        );
        assert_eq!(backend.pending_turn, None, "no turn is in flight anymore");
        // Failed, as auth, with the server's own words — including the half
        // it buried in `data` — kept as the detail rather than drawn as the
        // agent speaking.
        let [AgentEvent::TurnEnded {
            reason: farcooler_agent_core::event::EndReason::Failed { kind, detail },
        }] = events.as_slice()
        else {
            panic!("one failed end and no words: {events:?}");
        };
        assert_eq!(*kind, farcooler_agent_core::event::FailureKind::Auth);
        assert!(detail.contains("codex login"), "{detail}");
    }

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
