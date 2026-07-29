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
    /// A name for this agent, and the process names it runs under.
    ///
    /// Keyed on the RUNNING PROCESS, not on what the terminal was launched as.
    /// A terminal is created as a plain shell and the user types `claude` into
    /// it — so its launch preset says nothing about what is in it now, and a
    /// terminal that was started as an agent and exited back to a shell is not
    /// an agent any more.
    pub preset: &'static str,
    /// Process names, as `pane_current_command` reports them.
    ///
    /// Necessary but not sufficient. Claude Code rewrites its own process name
    /// to its version string — tmux reports `2.1.220` — so a running agent is
    /// invisible to process matching alone.
    pub commands: &'static [&'static str],
    /// Screen text that means "this IS this agent", whatever the process is
    /// called. Chosen to be furniture the agent always draws, not something a
    /// user might type.
    pub identity: &'static [&'static str],
    /// Any of these on screen means the agent is waiting for the user.
    pub blocked: &'static [&'static str],
    /// Any of these means it is working.
    pub working: &'static [&'static str],
    /// Any of these means it is present and ready for input.
    pub idle: &'static [&'static str],
}

/// The built-in rules.
///
/// Deliberately conservative: a signature that only ever appears in one state
/// is worth having, and a clever one that is usually right is not. An agent
/// whose screen matches nothing reports `Unknown`, which is honest, rather than
/// `Idle`, which would silence a notification the user wanted.
pub const RULES: &[AgentRules] = &[
    AgentRules {
        preset: "claude",
        commands: &["claude", "claude-code"],
        identity: &["? for shortcuts", "esc to interrupt", "Claude Code", "auto-accept edits"],
        blocked: &[
            "Do you want to",
            "Do you want me to",
            "❯ 1. Yes",
            "1. Yes, and don't ask again",
            "Allow this tool",
            "waiting for your input",
            "[y/n]",
            "(y/N)",
        ],
        // "esc to interrupt" is on screen for the whole time Claude Code is
        // thinking or running a tool, and never when it is not.
        working: &["esc to interrupt", "Thinking…", "ctrl+b to run in background"],
        idle: &["? for shortcuts", "auto mode on", "manual mode on", "Try \""],
    },
    AgentRules {
        preset: "codex",
        commands: &["codex"],
        identity: &["Codex", "/help for"],
        blocked: &["Allow command?", "approve this", "[y/n]", "(y/N)", "Do you want to"],
        working: &["Esc to interrupt", "Working…", "Running command"],
        idle: &["send a message", "/help for", "▌"],
    },
    AgentRules {
        preset: "cursor",
        commands: &["cursor-agent", "cursor"],
        identity: &["cursor-agent", "Ask anything"],
        blocked: &["Allow?", "[y/n]", "(y/N)", "Do you want to", "Approve"],
        working: &["esc to interrupt", "Generating", "Running"],
        idle: &["Ask anything", "/ for commands"],
    },
];

/// Which agent, if any, is running in a pane.
///
/// Matched on the foreground process. That is the only thing that answers the
/// question honestly: a terminal launched as a shell in which someone typed
/// `claude` IS a Claude Code terminal, and one launched as an agent that has
/// since exited to a prompt is not.
pub fn rules_for_command(command: &str) -> Option<&'static AgentRules> {
    // tmux reports the basename, but a wrapper may add a path or a suffix.
    let name = command.rsplit('/').next().unwrap_or(command).trim();
    RULES.iter().find(|r| r.commands.iter().any(|c| *c == name))
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
    if rules.idle.iter().any(|needle| screen.contains(needle)) {
        return AgentActivity::Idle;
    }
    // Present but unrecognised. Saying so beats guessing `Idle` and swallowing
    // the notification the user was waiting for.
    AgentActivity::Unknown
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
  ⏸ manual mode on";
        assert_eq!(classify("claude", screen), Idle);
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
    fn an_unrecognised_screen_says_so_rather_than_guessing_idle() {
        // Guessing idle would swallow the notification the user was waiting
        // for. Unknown is visible and fixable; a wrong idle is neither.
        assert_eq!(classify("claude", "some output nobody wrote a rule for"), Unknown);
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
        assert!(!wants_attention(Unknown));
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
            assert!(!rules.idle.is_empty(), "{} has no idle rules", rules.preset);
        }
    }

    #[test]
    fn no_signature_appears_in_two_states_of_the_same_agent() {
        // An overlapping signature makes the result depend on check order,
        // which is exactly the kind of thing that works until it does not.
        for rules in RULES {
            for needle in rules.blocked {
                assert!(!rules.working.contains(needle), "{needle:?} is both blocked and working");
                assert!(!rules.idle.contains(needle), "{needle:?} is both blocked and idle");
            }
            for needle in rules.working {
                assert!(!rules.idle.contains(needle), "{needle:?} is both working and idle");
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
