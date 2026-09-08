//! The hook side of a live agent session.
//!
//! Pure on purpose: everything here is a function from a captured JSON payload
//! to a value, so the whole of it is testable against real payloads with no
//! socket, no tmux and no agent. The I/O lives in `farcooler-cli`'s `hook`
//! subcommand and in the daemon's `hook_ingress`.

pub mod assemble;
pub mod facts;
pub mod wire;

/// Which agent a hook fired from.
///
/// Named rather than inferred from the event name, because the three agents
/// disagree about spelling: claude and codex send `Stop`, cursor sends `stop`,
/// and cursor's tool gate is `beforeShellExecution` where the others send
/// `PreToolUse`. The hook binary is registered per agent and therefore always
/// knows; nothing downstream should have to guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Agent {
    Claude,
    Codex,
    Cursor,
}

impl Agent {
    pub fn as_str(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Codex => "codex",
            Agent::Cursor => "cursor",
        }
    }
}

impl std::str::FromStr for Agent {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "claude" => Ok(Agent::Claude),
            "codex" => Ok(Agent::Codex),
            "cursor" => Ok(Agent::Cursor),
            _ => Err(()),
        }
    }
}
