//! Which agent runs in a pane, by its processes alone (ov-443).
//!
//! What a pane has drawn is not evidence: claude does not use the alternate
//! screen, so after it quits its banner and footer stay in view, and a `cat` of
//! a transcript draws them too. A client that offers the conversation view on
//! that would be offering it in a shell. So this reads the pane's foreground
//! process and the command lines of the foreground group (a typed `claude` is
//! there even when tmux reports its version number as the command), and
//! nothing the pane drew.

use farcooler_core::activity::Registry;

/// The preset of the agent in the pane's foreground, or `""` where none is.
///
/// Empty rather than absent: `Terminal.running_agent` absent means "this runner
/// does not say", and a runner that has looked and found a shell must be
/// believed over the preset the pane was launched as.
pub fn of(registry: &Registry, command: &str, foreground: &[String]) -> String {
    std::iter::once(command)
        .chain(foreground.iter().map(String::as_str))
        .find_map(|c| registry.rules_for_command(c))
        .map(|rules| rules.preset.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claude_banner() -> &'static str {
        "Claude Code v2.1.220\n? for shortcuts\nesc to interrupt\n"
    }

    /// Goes red when the screen decides: `identify` finds claude in this text.
    #[test]
    fn a_shell_showing_claudes_banner_runs_no_agent() {
        let registry = Registry::built_in();
        assert!(registry.identify("zsh", claude_banner()).is_some(), "the screen alone would say claude");
        assert_eq!(of(&registry, "zsh", &["-zsh".to_string()]), "");
        assert_eq!(of(&registry, "cat", &["cat claude-transcript.txt".to_string()]), "");
    }

    #[test]
    fn a_claude_in_the_foreground_is_found_by_its_process() {
        let registry = Registry::built_in();
        assert_eq!(of(&registry, "claude", &[]), "claude");
        // tmux reports the version it renamed itself to; the argv has the name.
        assert_eq!(of(&registry, "2.1.220", &["/Users/x/.local/bin/claude --model opus".to_string()]), "claude");
        assert_eq!(of(&registry, "codex-aarch64-a", &[]), "codex");
    }
}
