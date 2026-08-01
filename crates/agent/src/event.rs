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
        available_modes: Vec<String>,
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
        content: Option<String>,
        diff: Option<Diff>,
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
    TurnEnded {
        reason: EndReason,
    },
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
