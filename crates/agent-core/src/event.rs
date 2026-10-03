//! What an agent did, in terms no vendor owns.

/// Position in a session's event stream. Monotonic, starts at 0.
pub type Seq = u64;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Role {
    User,
    Agent,
    /// Reasoning the agent showed its working for. Collapsed by default in UI.
    Thought,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ToolStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

/// An edit, as before-and-after rather than as a reconstruction.
///
/// This exists only because the client answers `fs/write_text_file`. Rebuilding
/// it from tool-call arguments would couple this crate to each agent's private
/// tool schemas, which is the coupling ACP was chosen to avoid.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Diff {
    pub path: String,
    /// `None` when the file did not exist before the write.
    pub old_text: Option<String>,
    pub new_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PlanEntry {
    pub content: String,
    pub priority: String,
    pub status: String,
}

/// A prompt written but not yet sent.
///
/// It exists because an agent takes one turn at a time: a message written while
/// a turn is running cannot be delivered until that turn ends. Far Cooler holds
/// it rather than handing it to the adapter to sit on, which is what makes it
/// possible to show the queue, edit an entry, or take one back.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct QueuedPrompt {
    pub id: String,
    pub text: String,
    /// Pictures attached to it, waiting with it.
    #[serde(default)]
    pub images: Vec<PromptImage>,
}

/// An image traveling WITH a prompt rather than as a path beside it.
///
/// Base64 because that is what an ACP image content block carries, and because
/// the alternative — a path — is only meaningful on the runner that produced
/// it. See `RunningSession::send_prompt`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PromptImage {
    pub mime: String,
    pub base64: String,
}

/// One selectable option an agent offers: a mode, or a model.
///
/// Carries the human `name` as well as the `id`. Only the id was captured at
/// first, so the picker listed `acceptEdits` and `bypassPermissions` — the
/// wire's identifiers — at a user who never chose those words.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AgentChoice {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
}

/// One thing a user can change about a session.
///
/// ACP's stabilised, generic form: the adapter advertises a list of these and
/// the client renders one control each, rather than the client knowing in
/// advance that "mode" and "model" exist. That genericness is the point —
/// this same list already carries a subagent selector nobody designed a field
/// for, and a `thought_level` will arrive the same way.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ConfigOption {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// `mode`, `model`, `model_config`, `thought_level`, or absent. A hint for
    /// ordering and icons only — never a reason to special-case one.
    #[serde(default)]
    pub category: String,
    /// `select` or `boolean`.
    #[serde(default)]
    pub kind: String,
    /// The current value: an option id for a select, `"true"`/`"false"` for a
    /// boolean. Stringly typed so one field covers both without the client
    /// branching before it can even render.
    #[serde(default)]
    pub current_value: String,
    /// Empty for a boolean.
    #[serde(default)]
    pub options: Vec<AgentChoice>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PermissionOption {
    pub id: String,
    pub name: String,
    pub kind: String,
}

/// How a turn ended.
///
/// On the wire this is TWO keys beside each other in `TurnEnded`, not one:
/// `"reason"` is still the bare word every client already decodes as a string
/// (`"EndTurn"`, …, and now `"Failed"`), and a failed end adds `"failure"`
/// next to it. `Failed { kind, detail }` serialized the derived way would have
/// turned `reason` into an object, and every app in the field decodes it as a
/// string — one failed turn would have failed the whole batch it rode in.
/// See `EndReasonWire`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(into = "EndReasonWire", from = "EndReasonWire")]
pub enum EndReason {
    EndTurn,
    Cancelled,
    Refusal,
    MaxTokens,
    /// The turn could not run, or stopped partway, because something between
    /// the agent and its model failed: a bad key, no credit, a 5xx.
    ///
    /// Distinct from `Refusal`, which is the MODEL declining. Before this
    /// existed a failure either ended the turn as if it had succeeded (Claude)
    /// or put the adapter's raw sentence in the transcript as the agent's own
    /// words (ACP, codex).
    Failed {
        /// What kind of failure, as one of a few stable words a client can
        /// branch on.
        kind: FailureKind,
        /// The backend's own description, for logs. Never shown as the
        /// agent's words, and not guaranteed to be a sentence.
        detail: String,
    },
}

/// Why a turn failed, in a small vocabulary no backend owns.
///
/// Deliberately coarse: each kind is one thing a person does about it —
/// sign in, add credit, wait a minute, wait longer, check the connection. A
/// finer split would be one nobody could act on differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    /// Not signed in, or the key was refused.
    Auth,
    /// Out of credit, or over a usage or budget limit.
    Quota,
    /// Too many requests; retrying shortly will work.
    RateLimited,
    /// The provider is down or busy: an overloaded or 5xx answer.
    Overloaded,
    /// The provider could not be reached at all.
    Network,
    /// Anything else, including a kind a newer daemon names that this build
    /// does not know.
    #[serde(other)]
    Other,
}

/// `EndReason` as JSON: the bare word, and the failure beside it.
///
/// Flattened into `TurnEnded`, so a clean end is byte-identical to what it
/// always was — `{"reason":"EndTurn"}` — and every transcript in SQLite still
/// decodes.
#[derive(serde::Serialize, serde::Deserialize)]
struct EndReasonWire {
    reason: EndWord,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    failure: Option<FailureWire>,
}

#[derive(serde::Serialize, serde::Deserialize)]
enum EndWord {
    EndTurn,
    Cancelled,
    Refusal,
    MaxTokens,
    Failed,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct FailureWire {
    kind: FailureKind,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    detail: String,
}

impl From<EndReason> for EndReasonWire {
    fn from(reason: EndReason) -> Self {
        let (reason, failure) = match reason {
            EndReason::EndTurn => (EndWord::EndTurn, None),
            EndReason::Cancelled => (EndWord::Cancelled, None),
            EndReason::Refusal => (EndWord::Refusal, None),
            EndReason::MaxTokens => (EndWord::MaxTokens, None),
            EndReason::Failed { kind, detail } => {
                (EndWord::Failed, Some(FailureWire { kind, detail }))
            }
        };
        Self { reason, failure }
    }
}

impl From<EndReasonWire> for EndReason {
    fn from(wire: EndReasonWire) -> Self {
        match wire.reason {
            EndWord::EndTurn => EndReason::EndTurn,
            EndWord::Cancelled => EndReason::Cancelled,
            EndWord::Refusal => EndReason::Refusal,
            EndWord::MaxTokens => EndReason::MaxTokens,
            // A `Failed` with its detail lost is still a failure.
            EndWord::Failed => {
                let failure = wire.failure.unwrap_or(FailureWire {
                    kind: FailureKind::Other,
                    detail: String::new(),
                });
                EndReason::Failed { kind: failure.kind, detail: failure.detail }
            }
        }
    }
}

/// Map one backend failure onto a `FailureKind`.
///
/// The ONE place the mapping lives, so Claude, codex, ACP and a dead adapter
/// cannot drift into calling the same failure different things. Each backend
/// passes what its own shape carries, most specific first:
///
/// - `code`: the backend's machine word, if it has one. Claude's assistant
///   `error` (`authentication_failed`, `billing_error`, `rate_limit`, …) or
///   result `subtype` (`error_max_budget_usd`); codex's `codexErrorInfo`
///   (`unauthorized`, `usageLimitExceeded`, `serverOverloaded`,
///   `httpConnectionFailed`, …); ACP's `auth_required` for JSON-RPC -32000.
/// - `http_status`: the provider's status, when the backend forwards it
///   (Claude's `api_error_status`, codex's `httpStatusCode`).
/// - `message`: the human sentence, read for a few well-known phrases only
///   when nothing better said what happened. A LAST resort, and a loose one:
///   the phrases are wide substrings ("network", "quota", "/login"), so a
///   sentence that merely mentions one of them can land on the wrong kind.
///   It is reached for every ACP error but -32000, a refused codex
///   `turn/start`, and a Claude result with no `error` word or status.
///
/// Unrecognized is `Other`, never a guess.
pub fn classify_error(code: Option<&str>, http_status: Option<u16>, message: &str) -> FailureKind {
    // A connection word with a status is a connection that DID get an
    // answer, and the answer is the more specific of the two: codex sends
    // `{"httpConnectionFailed":{"httpStatusCode":429}}` for a rate limit.
    if code.and_then(kind_from_code) == Some(FailureKind::Network)
        && let Some(kind) = http_status.and_then(kind_from_status)
    {
        return kind;
    }
    if let Some(kind) = code.and_then(kind_from_code) {
        return kind;
    }
    if let Some(kind) = http_status.and_then(kind_from_status) {
        return kind;
    }
    kind_from_message(message).unwrap_or(FailureKind::Other)
}

fn kind_from_code(code: &str) -> Option<FailureKind> {
    // Compared without case or separators, so `rate_limit`, `rateLimit` and
    // `RATE-LIMIT` are one word.
    let word: String =
        code.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
    Some(match word.as_str() {
        "authenticationfailed" | "authenticationerror" | "unauthorized" | "authrequired"
        | "permissionerror" | "invalidapikey" | "oauthorgnotallowed" => FailureKind::Auth,
        "billingerror" | "usagelimitexceeded" | "sessionbudgetexceeded" | "insufficientquota"
        | "errormaxbudgetusd" | "creditbalancetoolow" => FailureKind::Quota,
        "ratelimit" | "ratelimiterror" | "ratelimited" | "toomanyrequests" => {
            FailureKind::RateLimited
        }
        "serveroverloaded" | "overloaded" | "overloadederror" | "servererror"
        | "internalservererror" | "apierror" => FailureKind::Overloaded,
        "httpconnectionfailed" | "responsestreamconnectionfailed"
        | "responsestreamdisconnected" | "connectionerror" | "networkerror" => {
            FailureKind::Network
        }
        _ => return None,
    })
}

fn kind_from_status(status: u16) -> Option<FailureKind> {
    Some(match status {
        401 | 403 => FailureKind::Auth,
        402 => FailureKind::Quota,
        429 => FailureKind::RateLimited,
        500..=599 => FailureKind::Overloaded,
        _ => return None,
    })
}

fn kind_from_message(message: &str) -> Option<FailureKind> {
    let m = message.to_ascii_lowercase();
    let any = |needles: &[&str]| needles.iter().any(|n| m.contains(n));
    if any(&["invalid api key", "/login", "codex login", "not authenticated", "unauthorized",
        "authentication", "not logged in", "auth required"])
    {
        Some(FailureKind::Auth)
    } else if any(&["credit balance", "quota", "billing", "usage limit", "budget"]) {
        Some(FailureKind::Quota)
    } else if any(&["rate limit", "rate_limit", "too many requests"]) {
        Some(FailureKind::RateLimited)
    } else if any(&["overloaded", "internal server error", "service unavailable", "bad gateway"]) {
        Some(FailureKind::Overloaded)
    } else if any(&["connection refused", "connection error", "network", "timed out",
        "could not resolve", "econnrefused", "econnreset", "enotfound", "stream disconnected"])
    {
        Some(FailureKind::Network)
    } else {
        None
    }
}

/// Why history is missing. Named so a client can explain itself to a user.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AgentGapReason {
    /// The ring dropped events the subscriber had not read.
    RingTrimmed,
    /// Reconnected to an agent that declared, at `initialize`, that it does
    /// not implement `session/load` at all — `session/load` was never even
    /// attempted. Distinct from `LoadFailed`: nothing here says the id was
    /// wrong, only that this agent cannot replay history for any id.
    LoadUnsupported,
    /// Reconnected to a session id with nothing recorded for it yet.
    ///
    /// The common case, not an exotic one: Far Cooler hands every claude and
    /// codex terminal a `--session-id` at launch, and a terminal switched to
    /// chat before anyone typed into it has no transcript to load. Nothing
    /// was lost — there was nothing to lose yet — which is why this is kept
    /// distinct from `LoadFailed` rather than folded into it: one is news,
    /// the other is a problem.
    LoadEmpty,
    /// `session/load` was attempted and the agent refused or errored for a
    /// reason other than "nothing recorded yet".
    ///
    /// Carries the adapter's own message. The pane the raw error would
    /// otherwise reach — the `println!` in `session.rs` — is exactly the
    /// surface chat mode replaces, so without this the detail never gets to
    /// whoever is actually looking at the conversation.
    LoadFailed { detail: String },
    /// An update arrived that this adapter could not interpret.
    Unparsed,
}

/// What a finished subagent reports about itself, as clients render it.
///
/// A normalized shape rather than the adapter's own: the wire's field set is
/// the adapter's to change, and nothing downstream should have to track that.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SubagentSummary {
    pub agent_type: String,
    pub model: String,
    pub tokens: u64,
    pub tool_uses: u64,
    pub duration_ms: u64,
    pub status: String,
}

/// `skip_serializing_if` takes a predicate by path, and `bool` has no method
/// with the right shape.
fn is_false(b: &bool) -> bool {
    !*b
}

/// What a `SessionStarted` with no `backend` recorded was: ACP was the only
/// backend that existed when those transcripts were written.
fn acp_backend() -> String {
    crate::backend::BackendKind::Acp.as_str().to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AgentEvent {
    SessionStarted {
        session_id: String,
        agent_mode: Option<String>,
        available_modes: Vec<AgentChoice>,
        /// The model in use, and what else this agent offers.
        ///
        /// Both live in the `session/new` result beside the modes. Not
        /// capturing them meant the UI had no model picker to build at all.
        model: Option<String>,
        available_models: Vec<AgentChoice>,
        /// Every selector the agent offers, generically.
        ///
        /// `available_modes` and `available_models` above are derived from
        /// this for the surfaces that still ask for them by name. New
        /// selectors — a subagent picker, a thought level — arrive here and
        /// need no protocol change to be rendered.
        config_options: Vec<ConfigOption>,
        /// `AgentChoice` rather than a bare name: a command has a description
        /// and the picker needs it. Reused rather than a second struct of the
        /// same three fields.
        available_commands: Vec<AgentChoice>,
        /// Which protocol is carrying this conversation: `acp`, `claude`, or
        /// `codex`.
        ///
        /// Here rather than left for a user to infer from `ps`, because the
        /// two paths behave differently in ways that show — a native backend
        /// has no adapter to go stale and steers into a running turn, an ACP
        /// one does neither — and a chat that cannot say which one it is makes
        /// every such difference look like a bug.
        ///
        /// Defaulted, because every transcript already in SQLite was written
        /// before this field existed and must still decode.
        #[serde(default = "acp_backend")]
        backend: String,
    },
    Message {
        role: Role,
        text: String,
        /// The `ToolCall` id of the dispatch this belongs to, if a subagent
        /// produced it. `None` is the ordinary case: the agent itself spoke.
        ///
        /// Skipped when absent so an ordinary event's JSON stays byte-identical
        /// to what it was before subagents were modelled — every transcript
        /// already in SQLite was written by that older code.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent: Option<String>,
    },
    ToolCall {
        id: String,
        title: String,
        kind: String,
        status: ToolStatus,
        locations: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent: Option<String>,
        /// This call IS a subagent dispatch, so it owns a block rather than
        /// being a row inside one.
        #[serde(default, skip_serializing_if = "is_false")]
        subagent: bool,
    },
    ToolUpdate {
        id: String,
        status: ToolStatus,
        /// A revised name for the call. `Terminal` becomes the command it ran.
        title: Option<String>,
        content: Option<String>,
        diff: Option<Diff>,
        locations: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent: Option<String>,
        /// Present once, on a dispatch's final update.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subagent: Option<SubagentSummary>,
    },
    Plan {
        entries: Vec<PlanEntry>,
    },
    Permission {
        id: String,
        tool_call: String,
        options: Vec<PermissionOption>,
    },
    Resolved {
        id: String,
        chosen: String,
    },
    ModeSet {
        agent_mode: String,
    },
    /// What this conversation is called.
    ///
    /// Session metadata, so NOT a `Gap` — nothing was lost. It arrived
    /// unmodelled and therefore became one, which drew a "history missing"
    /// break at the end of every turn for a title nobody had asked for.
    SessionInfo {
        title: String,
    },
    /// How much of the context window this session has consumed.
    ///
    /// Resent as a turn burns through it. Worth showing: the honest answer to
    /// "why has it started forgetting things" is a number, and a chat that
    /// hides it makes the user guess.
    Usage {
        used: u64,
        size: u64,
    },
    /// A selector changed, whether the user or the agent changed it.
    ConfigSet {
        id: String,
        value: String,
    },
    /// The slash-command menu, which an agent resends once per turn.
    ///
    /// Its own event rather than a second `SessionStarted`, because a consumer
    /// is entitled to assume a session starts exactly once — a repeat would
    /// read as a restart and reset everything built from the first one. And
    /// not a `Gap`, because nothing was lost: that would draw a "history
    /// missing" break on every turn for a menu nobody asked for.
    CommandsAvailable {
        commands: Vec<AgentChoice>,
    },
    TurnEnded {
        /// Flattened: `reason` stays a bare string on the wire, and a failure
        /// rides beside it as `failure`. See `EndReason`.
        #[serde(flatten)]
        reason: EndReason,
    },
    /// What the turn that just ended spent, for the runner's store.
    ///
    /// Bookkeeping that never reaches a transcript: the daemon takes it out of
    /// the stream before numbering (`AgentSupervisor::record`), so a client
    /// never meets a variant it has no case for. See `crate::usage`.
    TurnUsage {
        usage: crate::usage::TurnUsage,
    },
    /// Everything waiting to be sent, in order, sent whole on every change.
    ///
    /// Wholesale like `Plan`, and for the same reason: the list is short, it is
    /// replaced rather than appended to, and a client that had to reconstruct
    /// it from adds and removes could drift from what will actually be sent.
    PromptQueue { items: Vec<QueuedPrompt> },

    Gap {
        reason: AgentGapReason,
    },
}

/// An event and where it sits in the stream.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Sequenced {
    pub seq: Seq,
    pub event: AgentEvent,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gap_is_a_first_class_event_not_an_absence() {
        // The whole reason a derived transcript is allowed to exist in this
        // product: it can say where it is incomplete. A missing range must be
        // representable, or callers will express it as a shorter list and the
        // UI will render a lie.
        let e = AgentEvent::Gap { reason: AgentGapReason::RingTrimmed };
        assert!(matches!(e, AgentEvent::Gap { .. }));
    }

    #[test]
    fn a_sequenced_event_carries_its_own_position() {
        // Clients subscribe from a cursor, so an event that does not know its
        // own seq cannot be replayed into the right place.
        let s = Sequenced { seq: 7, event: AgentEvent::TurnEnded { reason: EndReason::EndTurn } };
        assert_eq!(s.seq, 7);
    }

    #[test]
    fn a_clean_end_is_the_same_bytes_it_always_was() {
        // Every transcript in SQLite and every app in the field reads this
        // shape. A failed end must not have changed it for the others.
        let json = serde_json::to_string(&AgentEvent::TurnEnded { reason: EndReason::EndTurn })
            .unwrap();
        assert_eq!(json, r#"{"TurnEnded":{"reason":"EndTurn"}}"#);
        let back: AgentEvent = serde_json::from_str(r#"{"TurnEnded":{"reason":"Cancelled"}}"#)
            .unwrap();
        assert_eq!(back, AgentEvent::TurnEnded { reason: EndReason::Cancelled });
    }

    #[test]
    fn a_failed_end_keeps_reason_a_string_and_puts_the_failure_beside_it() {
        // `reason` is decoded as a STRING by the iOS, Mac and Android apps
        // (`TurnEndedPayload`, `body.string("reason")`). An object there would
        // fail the whole batch on every client already shipped.
        let event = AgentEvent::TurnEnded {
            reason: EndReason::Failed { kind: FailureKind::Auth, detail: "Invalid API key".into() },
        };
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["TurnEnded"]["reason"], "Failed");
        assert_eq!(value["TurnEnded"]["failure"]["kind"], "auth");
        assert_eq!(value["TurnEnded"]["failure"]["detail"], "Invalid API key");
        assert_eq!(serde_json::from_value::<AgentEvent>(value).unwrap(), event);
        // A kind a newer daemon invents is still a failure, not an error.
        let newer: AgentEvent = serde_json::from_str(
            r#"{"TurnEnded":{"reason":"Failed","failure":{"kind":"solar_flare"}}}"#,
        )
        .unwrap();
        assert_eq!(
            newer,
            AgentEvent::TurnEnded {
                reason: EndReason::Failed { kind: FailureKind::Other, detail: String::new() }
            }
        );
    }

    #[test]
    fn each_backends_failure_word_lands_on_one_kind() {
        let k = |code, status, message| classify_error(code, status, message);
        // Claude's assistant `error`, and its status.
        assert_eq!(k(Some("authentication_failed"), None, ""), FailureKind::Auth);
        assert_eq!(k(Some("billing_error"), None, ""), FailureKind::Quota);
        assert_eq!(k(Some("rate_limit"), None, ""), FailureKind::RateLimited);
        assert_eq!(k(None, Some(401), ""), FailureKind::Auth);
        assert_eq!(k(None, Some(529), ""), FailureKind::Overloaded);
        // Codex's `codexErrorInfo`.
        assert_eq!(k(Some("unauthorized"), None, ""), FailureKind::Auth);
        assert_eq!(k(Some("usageLimitExceeded"), None, ""), FailureKind::Quota);
        assert_eq!(k(Some("serverOverloaded"), None, ""), FailureKind::Overloaded);
        assert_eq!(k(Some("httpConnectionFailed"), None, ""), FailureKind::Network);
        assert_eq!(k(Some("httpConnectionFailed"), Some(429), ""), FailureKind::RateLimited);
        assert_eq!(k(Some("oauth_org_not_allowed"), None, ""), FailureKind::Auth);
        // The code wins over a message that says something else.
        assert_eq!(k(Some("rate_limit"), Some(401), "Invalid API key"), FailureKind::RateLimited);
        // Only words, when nothing better was said.
        assert_eq!(k(None, None, "Invalid API key · Please run /login"), FailureKind::Auth);
        assert_eq!(k(None, None, "Credit balance is too low"), FailureKind::Quota);
        assert_eq!(k(Some("contextWindowExceeded"), Some(400), "huh"), FailureKind::Other);
    }
}
