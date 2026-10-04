//! A refused message for a pane's agent, in this CLI's words (ov-174). Its own
//! file to keep `tasks.rs` inside its size budget.

use farcooler_transport::ClientError;

use super::Refused;

/// A refused message for a pane's agent, in this CLI's words when its agent
/// has stopped, with the command that restarts it. The code is kept for
/// `--json`. Any other refusal is left as it was.
pub(crate) fn agent_refused(terminal: &str) -> impl FnOnce(ClientError) -> Box<dyn std::error::Error> + '_ {
    move |e| match &e {
        ClientError::Daemon { code, .. } if farcooler_core::error::word_for(*code) == "agent-stopped" => {
            let said = format!(
                "the agent in this pane stopped; restart it with `farcooler terminal set-pane-mode {terminal} agent`, then try again"
            );
            Box::new(Refused::new(said, Some(*code)))
        }
        _ => Box::new(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_protocol::v1 as pb;

    fn daemon(code: pb::ErrorCode) -> ClientError {
        ClientError::Daemon { code: code as i32, retryable: false, message: "runner's words".into(), what: String::new() }
    }

    /// A prompt to a stopped agent says how to restart it and keeps its word;
    /// the retryable "not connected yet" is left as the runner said it.
    #[test]
    fn a_stopped_agent_is_told_how_to_restart_it() {
        let stopped = agent_refused("ab12")(daemon(pb::ErrorCode::AgentStopped));
        assert!(stopped.to_string().contains("set-pane-mode ab12 agent"), "{stopped}");
        assert!(!stopped.to_string().contains(". r"), "nothing after a full stop goes uncapitalized: {stopped}");
        assert_eq!(stopped.downcast_ref::<Refused>().and_then(Refused::word), Some("agent-stopped"));
        assert_eq!(agent_refused("ab12")(daemon(pb::ErrorCode::AgentNotConnected)).to_string(), "runner's words");
    }

    /// An answer to a stopped agent names the pane it was for, not a
    /// placeholder (review finding 4).
    #[test]
    fn a_refused_answer_names_its_pane() {
        let said = crate::answer_refused(daemon(pb::ErrorCode::AgentStopped), "ab12").to_string();
        assert!(said.contains("set-pane-mode ab12 agent"), "{said}");
        assert!(!said.contains('<'), "{said}");
    }

    /// `set-pane-mode`'s help says agent mode restarts a stopped agent.
    #[test]
    fn set_pane_mode_says_it_restarts_a_stopped_agent() {
        use clap::CommandFactory;
        let mut cli = crate::Cli::command();
        let mode = cli
            .find_subcommand_mut("terminal")
            .and_then(|t| t.find_subcommand_mut("set-pane-mode"))
            .and_then(|s| s.get_arguments().find(|a| a.get_id() == "mode").cloned())
            .expect("set-pane-mode takes a mode");
        let help = mode.get_help().map(|h| h.to_string()).unwrap_or_default();
        assert!(help.contains("restarts"), "{help}");
    }
}
