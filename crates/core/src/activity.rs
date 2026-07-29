//! What the agent is doing, derived from its screen.
//!
//! This is a different question from `derive.rs`, which asks whether a
//! terminal's process is alive. A Claude Code that is sitting at a permission
//! prompt and one that is halfway through editing a file are both `running`;
//! the difference between them is the entire reason to look at a fleet at 3am.
//!
//! ```text
//!   derive.rs   process liveness   starting | running | exited | error | lost
//!   activity.rs what it is doing   none | idle | working | blocked | done
//! ```
//!
//! **`Done` is not a state of the agent.** It is `Idle` that nobody has looked
//! at yet. That is what makes it the thing worth sending a notification about,
//! and what makes it clear itself the moment someone opens the terminal. An
//! agent that finished an hour ago and was seen is just idle.
//!
//! **Detection is the daemon's job, never a client's.** A phone has no screen
//! to inspect, and two clients each deciding for themselves would disagree
//! about the same terminal — the failure the derivation model exists to
//! prevent. The daemon reads the screen and everyone renders what it decided.
//!
//! The rules are a table rather than a match, because "Claude Code changed its
//! prompt" should be a data change, not a code change.

use overnight_protocol::v1::AgentActivity;

/// One agent's screen signatures.
///
/// Order matters within a screen: `blocked` is checked before `working`,
/// because an agent asking permission usually still shows its working
/// furniture. Getting that backwards means never noticing that something needs
/// you, which is the one failure that makes the whole feature pointless.
#[derive(Debug, Clone)]
pub struct AgentRules {
    /// What to call this agent.
    pub preset: &'static str,

    /// Process-name PREFIXES, as `pane_current_command` reports them.
    ///
    /// Prefixes, not exact names, because tmux truncates the field: codex
    /// arrives as `codex-aarch64-a`. And necessary but never sufficient —
    /// Claude Code renames itself to its version (`2.1.220`) and cursor-agent
    /// runs as plain `node`, which is far too generic to claim. Screen identity
    /// is what actually carries this.
    pub commands: &'static [&'static str],

    /// Screen text that means "this IS this agent".
    ///
    /// Furniture the agent always draws, never a phrase a user could type, so a
    /// shell echoing "do you want to proceed?" is not promoted to an agent.
    pub identity: &'static [&'static str],

    /// Waiting on the user.
    ///
    /// The list that has to be right. A missed blocked state is a notification
    /// that never arrives, which is the one failure that makes the whole
    /// feature pointless — so these are deliberately generous.
    pub blocked: &'static [&'static str],

    /// Actively doing something.
    pub working: &'static [&'static str],
}

/// The built-in rules.
///
/// Every signature below was read off a running agent rather than guessed. The
/// first version of this file was guesswork and it matched no real screen.
///
/// There is deliberately no `idle` list. An agent that is identified, not
/// blocked and not working IS idle, and requiring positive idle furniture meant
/// a version bump renaming a footer left an agent stuck on `unknown` — never
/// reaching `done`, never notifying.
pub const RULES: &[AgentRules] = &[
    AgentRules {
        preset: "claude",
        commands: &["claude"],
        identity: &["? for shortcuts", "Claude Code", "auto-accept edits", "esc to interrupt"],
        blocked: &[
            "Do you want to",
            "Do you want me to",
            "❯ 1. Yes",
            "1. Yes, and don't ask again",
            // The footer under every approval prompt.
            "Esc to cancel · Tab to amend",
            "[y/n]",
            "(y/N)",
        ],
        working: &["esc to interrupt", "Thinking…"],
    },
    AgentRules {
        preset: "codex",
        // Truncated by tmux to `codex-aarch64-a`, hence the prefix.
        commands: &["codex"],
        identity: &["OpenAI Codex", "/model to change"],
        blocked: &[
            // Codex draws every choice as a numbered list under a `›` marker.
            "\u{203a} 1.",
            "Press enter to continue",
            "Allow command",
            "Do you want to",
            "[y/n]",
            "(y/N)",
        ],
        working: &["esc to interrupt", "Working ("],
    },
    AgentRules {
        preset: "cursor",
        // cursor-agent runs as `node`, which cannot be claimed — matching it
        // would label every node process a coding agent. Kept for installs that
        // expose a real name; screen identity is what finds it here.
        commands: &["cursor-agent"],
        identity: &["Cursor Agent", "cursor-agent", "Press any key to sign in"],
        // UNVERIFIED. This install could not get past its sign-in screen, so
        // unlike the two above these were not read off a running agent. They
        // follow the same shapes and should be checked against a signed-in
        // cursor-agent before being trusted.
        blocked: &["Do you want to", "Allow?", "[y/n]", "(y/N)", "\u{203a} 1."],
        working: &["esc to interrupt", "Generating"],
    },
];

/// Which agent, if any, is running in a pane.
///
/// Matched on the foreground process. That is the only thing that answers the
/// question honestly: a terminal launched as a shell in which someone typed
/// `claude` IS a Claude Code terminal, and one launched as an agent that has
/// since exited to a prompt is not.
pub fn rules_for_command(command: &str) -> Option<&'static AgentRules> {
    let name = command.rsplit('/').next().unwrap_or(command).trim();
    if name.is_empty() {
        return None;
    }
    // Prefix, because tmux truncates: `codex-aarch64-a` must match `codex`.
    RULES.iter().find(|r| r.commands.iter().any(|c| name.starts_with(c)))
}

/// Look up by agent name, for callers that already know which one.
pub fn rules_for(preset: &str) -> Option<&'static AgentRules> {
    RULES.iter().find(|r| r.preset == preset)
}

/// Which agent is in this pane: by process, or failing that, by what it drew.
///
/// Both are needed. Process matching is exact when it works, and it does not
/// always work — Claude Code renames itself to its version number, so tmux
/// reports `2.1.220` and no name matching will ever find it. Screen matching
/// catches that, and is why the identity markers are agent furniture rather
/// than anything a user could type.
pub fn identify(command: &str, screen: &str) -> Option<&'static AgentRules> {
    if let Some(rules) = rules_for_command(command) {
        return Some(rules);
    }
    let text = plain_text(screen);
    RULES.iter().find(|r| r.identity.iter().any(|needle| text.contains(needle)))
}

/// What to call whatever is running here.
///
/// The agent's name when one is recognised, otherwise the process itself. A row
/// then reads `claude` or `zsh` rather than the preset a terminal was created
/// with, which after the first command is usually a lie.
pub fn describe(command: &str, screen: &str) -> String {
    if let Some(rules) = identify(command, screen) {
        return rules.preset.to_string();
    }
    let name = command.rsplit('/').next().unwrap_or(command).trim();
    // A process whose name is a version number tells a user nothing. Better to
    // say "shell" than to label a row `2.1.220`.
    let meaningless = name.is_empty()
        || name.chars().all(|c| c.is_ascii_digit() || c == '.');
    if meaningless { "shell".to_string() } else { name.to_string() }
}

/// Strip escape sequences and collapse whitespace.
///
/// This is not defensive tidying; without it nothing matches. Claude Code
/// colours every WORD separately, so a line that reads
///
/// ```text
/// ❯ 1. Yes, I trust this folder
/// ```
///
/// arrives as `ESC[38;5;153m❯ESC[39m ESC[38;5;246m1.ESC[39m ESC[38;5;153mYes,…`
/// and the substring "1. Yes" appears nowhere in it. tmux is asked for a plain
/// capture, so in practice the input is already clean — but a rule table whose
/// correctness depends on a flag at a distant call site is one that breaks
/// silently, so the matching does its own stripping too.
///
/// Whitespace is collapsed because a terminal pads with spaces to the column
/// width, and a signature spanning a wrap would otherwise never match.
pub fn plain_text(screen: &str) -> String {
    let mut out = String::with_capacity(screen.len());
    let mut chars = screen.chars().peekable();

    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.next() {
            // CSI: parameters, then a final byte in @-~.
            Some('[') => {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            // OSC: variable length, ended by BEL or ST (ESC \). A fixed skip
            // here is what leaks a hyperlink's URL into the text.
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\u{7}' {
                        break;
                    }
                    if c == '\u{1b}' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            // Anything else is a two-character escape, already consumed.
            _ => {}
        }
    }

    // One pass, so a signature written with single spaces matches a screen
    // padded with many.
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Classify a rendered screen.
///
/// `screen` is the visible pane, with or without escape sequences.
pub fn classify(command: &str, screen: &str) -> AgentActivity {
    let Some(rules) = identify(command, screen) else {
        return AgentActivity::None;
    };
    let screen = &plain_text(screen);

    // Blocked first. An agent asking permission still shows its working
    // furniture, so checking working first would hide every question.
    if rules.blocked.iter().any(|needle| screen.contains(needle)) {
        return AgentActivity::Blocked;
    }
    if rules.working.iter().any(|needle| screen.contains(needle)) {
        return AgentActivity::Working;
    }
    // Identified, not asking, not busy: it is sitting there. Requiring positive
    // idle furniture left an agent whose footer changed between versions stuck
    // on `unknown` forever — never reaching `done`, never notifying.
    AgentActivity::Idle
}

/// Fold a fresh classification against what was last reported.
///
/// This is where `Done` is created and destroyed, and it is the only place that
/// happens. Two rules:
///
/// - Finishing means `Working` (or `Blocked`) becoming `Idle`. That transition
///   produces `Done`, which is what a notification fires on.
/// - `Done` survives further idle observations. An agent that finished and is
///   still sitting there has not become less finished because we looked again.
///   It stops being `Done` when someone SEES it, which is `seen()`, not here.
pub fn advance(previous: AgentActivity, observed: AgentActivity) -> AgentActivity {
    use AgentActivity::*;
    match (previous, observed) {
        // The transition worth telling someone about.
        (Working | Blocked, Idle) => Done,
        // Already announced; stay announced until acknowledged.
        (Done, Idle | Unknown) => Done,
        // Anything else is just the new observation. In particular Done ->
        // Working is a real restart of work and must not linger as Done.
        (_, observed) => observed,
    }
}

/// The activity after a user has looked at the terminal.
///
/// Only `Done` is affected: it is defined as unseen, so seeing it ends it.
/// Everything else describes the agent rather than the user's attention and is
/// unchanged by looking.
pub fn seen(current: AgentActivity) -> AgentActivity {
    match current {
        AgentActivity::Done => AgentActivity::Idle,
        other => other,
    }
}

/// Does this activity want the user's attention?
///
/// The single definition, so a Mac badge, a phone notification and a Live
/// Activity cannot disagree about what is worth interrupting someone for.
pub fn wants_attention(activity: AgentActivity) -> bool {
    matches!(activity, AgentActivity::Blocked | AgentActivity::Done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use AgentActivity::*;

    #[test]
    fn a_shell_is_not_an_agent() {
        // Reporting a shell as idle would put it in the same visual language as
        // an agent waiting for work, in the list a user scans for something
        // that needs them.
        assert_eq!(classify("zsh", "e-liang@Mac project % "), None);
        assert_eq!(classify("bash", "anything at all"), None);
    }

    #[test]
    fn an_agent_is_found_by_what_is_running_not_by_how_it_was_launched() {
        // The whole point: terminals are created as plain shells and the user
        // types `claude` into them. Keying on the launch preset would report a
        // live agent as a shell forever.
        assert!(rules_for_command("claude").is_some());
        assert!(rules_for_command("/opt/homebrew/bin/claude").is_some());
        assert!(rules_for_command("cursor-agent").is_some());
        assert!(rules_for_command("zsh").is_none());
        assert!(rules_for_command("").is_none());
    }

    #[test]
    fn a_row_is_labelled_by_what_is_actually_running() {
        assert_eq!(describe("claude", ""), "claude");
        assert_eq!(describe("cursor-agent", ""), "cursor");
        // Not an agent: say what it is rather than inventing a category.
        assert_eq!(describe("zsh", ""), "zsh");
        assert_eq!(describe("cargo", ""), "cargo");
        assert_eq!(describe("", ""), "shell");
    }

    #[test]
    fn an_agent_that_renamed_itself_is_still_found() {
        // The real case that broke process matching: Claude Code sets its
        // process name to its version, so tmux reports `2.1.220`.
        let screen = "❯ hello?\n  ⏸ manual mode on · ? for shortcuts";
        assert!(identify("2.1.220", screen).is_some());
        assert_eq!(describe("2.1.220", screen), "claude");
        assert_eq!(classify("2.1.220", screen), Idle);
    }

    #[test]
    fn a_version_number_is_never_shown_as_a_name() {
        // Whatever it is, `2.1.220` is not a useful label for a row.
        assert_eq!(describe("2.1.220", "nothing recognisable"), "shell");
        assert_eq!(describe("1.2", ""), "shell");
    }

    #[test]
    fn a_shell_showing_agent_like_text_is_not_promoted_to_an_agent() {
        // Identity markers are agent furniture, not phrases a user might type.
        assert!(identify("zsh", "$ echo 'do you want to proceed?'").is_none());
        assert_eq!(classify("zsh", "$ echo '[y/n]'"), None);
    }

    #[test]
    fn claude_working_is_recognised() {
        let screen = "\
✻ Cooked for 6s
  Running 2 shell commands · 4s
  esc to interrupt";
        assert_eq!(classify("claude", screen), Working);
    }

    #[test]
    fn claude_idle_is_recognised() {
        let screen = "\
❯ hello?
────────────────────────
  ⏸ manual mode on · ? for shortcuts";
        assert_eq!(classify("claude", screen), Idle);
    }

    #[test]
    fn codex_is_recognised_through_a_truncated_process_name() {
        // tmux caps the field, so a real codex arrives as `codex-aarch64-a`.
        // Exact matching found nothing and every codex reported as a shell.
        assert!(rules_for_command("codex-aarch64-a").is_some());
        assert_eq!(describe("codex-aarch64-a", ""), "codex");
    }

    #[test]
    fn codex_states_come_from_its_real_screen() {
        let idle = "› Explain this codebase\n  gpt-5.6-sol high · ~/project\n  >_ OpenAI Codex (v0.145.0)";
        assert_eq!(classify("codex-aarch64-a", idle), Idle);

        let working = "• Working (6s • esc to interrupt) · 1 background terminal running";
        assert_eq!(classify("codex-aarch64-a", working), Working);

        let blocked = "› 1. Update now\n  2. Skip\n  Press enter to continue";
        assert_eq!(classify("codex-aarch64-a", blocked), Blocked);
    }

    #[test]
    fn cursor_cannot_be_claimed_by_its_process() {
        // cursor-agent runs as plain `node`. Matching that would label every
        // node process a coding agent, which is worse than missing it.
        assert!(rules_for_command("node").is_none());
        assert_eq!(describe("node", ""), "node");
        // It is found by what it draws instead.
        assert!(identify("node", "Press any key to sign in...").is_some());
    }

    #[test]
    fn an_identified_agent_is_never_stuck_on_unknown() {
        // The failure this replaced: an agent whose footer changed between
        // versions matched no idle signature and reported `unknown` forever,
        // so it never reached `done` and never notified.
        let unfamiliar = "OpenAI Codex\nsome screen nobody wrote a rule for";
        assert_eq!(classify("codex", unfamiliar), Idle);
    }

    #[test]
    fn a_question_beats_the_working_furniture_around_it() {
        // The case that decides whether this feature is worth having. An agent
        // asking permission still shows its spinner chrome, so checking
        // "working" first would mean never noticing that something needs you.
        let screen = "\
Do you want to allow this command?
❯ 1. Yes
  2. No
  esc to interrupt";
        assert_eq!(classify("claude", screen), Blocked);
    }

    #[test]
    fn something_that_is_not_an_agent_stays_not_an_agent() {
        // The important half of the honesty: a screen we cannot identify is not
        // promoted to an agent, so a shell never appears in the list of things
        // that might need you.
        assert_eq!(classify("zsh", "some output nobody wrote a rule for"), None);
    }

    #[test]
    fn finishing_produces_done() {
        assert_eq!(advance(Working, Idle), Done);
        // Answering a question and then finishing counts too.
        assert_eq!(advance(Blocked, Idle), Done);
    }

    #[test]
    fn done_survives_further_looks_at_the_same_idle_screen() {
        // The detector runs on a timer. An agent that finished has not become
        // less finished because we sampled it again.
        assert_eq!(advance(Done, Idle), Done);
        assert_eq!(advance(Done, Unknown), Done);
    }

    #[test]
    fn done_ends_when_work_restarts() {
        // Otherwise a terminal that was told to do something else would still
        // be advertising a completion from ten minutes ago.
        assert_eq!(advance(Done, Working), Working);
        assert_eq!(advance(Done, Blocked), Blocked);
    }

    #[test]
    fn seeing_it_is_what_ends_done() {
        assert_eq!(seen(Done), Idle);
        // Everything else describes the agent, not the user's attention.
        for other in [Working, Blocked, Idle, None, Unknown] {
            assert_eq!(seen(other), other);
        }
    }

    #[test]
    fn only_blocked_and_done_interrupt_someone() {
        // Working is not worth a notification: it is the normal case, and a
        // product that buzzes for it is one people turn off.
        assert!(wants_attention(Blocked));
        assert!(wants_attention(Done));
        assert!(!wants_attention(Working));
        assert!(!wants_attention(Idle));
        assert!(!wants_attention(None));
    }

    #[test]
    fn starting_from_nothing_reports_the_first_observation() {
        assert_eq!(advance(Unspecified, Working), Working);
        assert_eq!(advance(Unspecified, Idle), Idle);
    }

    #[test]
    fn every_rule_set_has_signatures_for_all_three_states() {
        // A preset with an empty list can never report that state, which would
        // be a silent hole rather than a visible bug.
        for rules in RULES {
            assert!(!rules.blocked.is_empty(), "{} has no blocked rules", rules.preset);
            assert!(!rules.working.is_empty(), "{} has no working rules", rules.preset);
        }
    }

    #[test]
    fn no_signature_appears_in_two_states_of_the_same_agent() {
        // An overlapping signature makes the result depend on check order,
        // which is exactly the kind of thing that works until it does not.
        for rules in RULES {
            for needle in rules.blocked {
                assert!(!rules.working.contains(needle), "{needle:?} is both blocked and working");
            }
        }
    }

    #[test]
    fn every_agent_can_be_recognised_without_its_process_name() {
        // Process names are unreliable, so identity markers are what actually
        // has to hold. An agent with none is invisible the moment it renames
        // itself.
        for rules in RULES {
            assert!(!rules.identity.is_empty(), "{} has no identity markers", rules.preset);
        }
    }

    #[test]
    fn every_agent_declares_a_process_to_match() {
        // A rule set with no command can never be selected, which is a silent
        // hole rather than a visible bug.
        for rules in RULES {
            assert!(!rules.commands.is_empty(), "{} matches no process", rules.preset);
        }
    }
}
