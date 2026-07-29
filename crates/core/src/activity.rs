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
    /// The command preset this applies to.
    pub preset: &'static str,
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
        blocked: &["Allow command?", "approve this", "[y/n]", "(y/N)", "Do you want to"],
        working: &["Esc to interrupt", "Working…", "Running command"],
        idle: &["send a message", "/help for", "▌"],
    },
    AgentRules {
        preset: "cursor",
        blocked: &["Allow?", "[y/n]", "(y/N)", "Do you want to", "Approve"],
        working: &["esc to interrupt", "Generating", "Running"],
        idle: &["Ask anything", "/ for commands"],
    },
];

/// Is this preset an agent at all?
///
/// A plain shell is not one, and reporting it `Idle` would put it in the same
/// visual language as an agent waiting for work — which is noise in exactly the
/// list a user scans for something that needs them.
pub fn rules_for(preset: &str) -> Option<&'static AgentRules> {
    RULES.iter().find(|r| r.preset == preset)
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
pub fn classify(preset: &str, screen: &str) -> AgentActivity {
    let Some(rules) = rules_for(preset) else {
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
        assert_eq!(classify("shell", "e-liang@Mac project % "), None);
        assert_eq!(classify("bash", "anything at all"), None);
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
}
