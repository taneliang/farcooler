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
//!
//! **Text in the box.** claude drops the mode line's `· ↓ to manage` once
//! its box holds anything. Agents still show as their `◯` rows; shells and
//! monitors have no rows, only the mode line's count, `· 7 shells` or
//! `· 8 shells, 1 monitor` (claude 2.1.308 and 2.1.309,
//! `captures/claude-2.1.309-shells-*-59x41-e.txt`). Read by the needles
//! alone, a pane with background shells and a message pasted into it read
//! no background work, and the daemon left the message in the box as
//! `paste_left` (ov-795). So the count is read too, on the mode line row
//! alone (`composer::claude_mode_line`): `N shells` anywhere in the footer
//! could be words typed into the box, and the row under the box's closing
//! rule can't be.

use super::{Registry, footer_text};

impl Registry {
    /// Whether the agent's footer shows background work still running.
    pub fn background_running(&self, command: &str, screen: &str) -> bool {
        let Some(rules) = self.identify(command, screen) else { return false };
        let footer = footer_text(screen, rules.footer_lines);
        rules.background.iter().any(|needle| footer.contains(needle.as_str()))
            || (rules.preset == "claude" && crate::composer::claude_mode_line(screen).is_some_and(|line| counts_background(&line)))
    }
}

/// Whether claude's mode line `line` counts background shells or monitors:
/// a ` · `-separated part that is only `N shell(s)` and `N monitor(s)`,
/// joined by `, `. A count cut short (`8 she…`) isn't one.
fn counts_background(line: &str) -> bool {
    line.split(" · ").any(|part| {
        let part = part.trim();
        !part.is_empty()
            && part.split(", ").all(|item| {
                let Some((n, what)) = item.split_once(' ') else { return false };
                !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) && matches!(what, "shell" | "shells" | "monitor" | "monitors")
            })
    })
}
