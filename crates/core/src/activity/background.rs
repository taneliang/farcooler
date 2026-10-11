//! Background work: agents and shells claude runs beside its main loop
//! (`AgentRules::background`, ov-468).
//!
//! A claude whose turn has ended while background agents still run is two
//! things at once. To a fleet row it's working, which is the bug
//! `captures/claude-background-agents-main-idle.txt` froze: a pane three
//! subagents deep read `idle`. To its box it's between turns: a message
//! typed there runs at once as a turn of its own, measured on claude 2.1.296
//! (`captures/claude-2.1.296-background-agents-main-idle-120x40-e.txt`), its
//! transcript a `user` record and no `enqueue`. So `classify` says Working,
//! and `classify_main`, which the daemon types by, says Idle.

use super::{Registry, footer_text};

impl Registry {
    /// Whether the agent's footer shows background work still running.
    pub fn background_running(&self, command: &str, screen: &str) -> bool {
        let Some(rules) = self.identify(command, screen) else { return false };
        let footer = footer_text(screen, rules.footer_lines);
        rules.background.iter().any(|needle| footer.contains(needle.as_str()))
    }
}
