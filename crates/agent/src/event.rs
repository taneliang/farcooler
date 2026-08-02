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
/// a turn is running cannot be delivered until that turn ends. Overnight holds
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

/// An image travelling WITH a prompt rather than as a path beside it.
///
/// Base64 because that is what an ACP image content block carries, and because
/// the alternative — a path — is only meaningful on the machine that produced
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

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum EndReason {
    EndTurn,
    Cancelled,
    Refusal,
    MaxTokens,
}

/// Why history is missing. Named so a client can explain itself to a user.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AgentGapReason {
    /// The ring dropped events the subscriber had not read.
    RingTrimmed,
    /// Reconnected, but the agent does not implement `session/load`.
    LoadUnsupported,
    /// An update arrived that this adapter could not interpret.
    Unparsed,
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
        available_commands: Vec<String>,
    },
    Message {
        role: Role,
        text: String,
    },
    ToolCall {
        id: String,
        title: String,
        kind: String,
        status: ToolStatus,
        locations: Vec<String>,
    },
    ToolUpdate {
        id: String,
        status: ToolStatus,
        /// A revised name for the call. `Terminal` becomes the command it ran.
        title: Option<String>,
        content: Option<String>,
        diff: Option<Diff>,
        locations: Vec<String>,
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
        commands: Vec<String>,
    },
    TurnEnded {
        reason: EndReason,
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
}
