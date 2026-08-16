//! What a pane's OSC title says about it.
//!
//! All three first-class agents set one, and each is useful differently. Claude
//! writes a spinner glyph and an AI-written summary of the task; codex writes a
//! spinner, or `[ ! ] Action Required` when it is waiting, and only the
//! directory name; cursor writes a summary and no status at all.
//!
//! Verified to be the agents' own behavior rather than an installed hook: the
//! claude binary carries `useTerminalTitle`, the codex binary carries
//! `terminal_title` as a config key, and the codex spinner advances ten frames
//! in five seconds — no hook fires at three hertz.
//!
//! That last point cuts the other way too. `terminal_title` being a config key
//! means a user can turn it off, so every answer here is allowed to be "no
//! opinion", and the layer below takes over.

/// What the title's leading glyph claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleStatus {
    Working,
    NotWorking,
    /// No glyph, or one we do not recognize. Falls through to the screen.
    NoOpinion,
}

/// A pane title, read for the two things it can answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneTitle {
    pub status: TitleStatus,
    /// A name worth showing, or `None` when the title is furniture.
    pub name: Option<String>,
}

/// Frames the agents animate while they work.
const SPINNERS: &[char] = &[
    // Braille, which codex cycles.
    '⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏', '⠞', '⠝', '⠛', '⠟',
    // Quadrant circles, which claude cycles.
    '◐', '◑', '◒', '◓',
];

/// Glyphs that mean the agent has stopped and is waiting on a person.
const RESTING: &[char] = &['✳', '✻', '✽', '✢', '·'];

/// Harness names an agent shows before it has a task to describe.
const PLACEHOLDERS: &[&str] = &["Claude Code", "Cursor Agent", "Codex", "OpenAI Codex"];

/// Read a pane title.
///
/// `program` is the pane's foreground process and `hostname` this machine's
/// name; both are needed only to reject a title that is one of them.
pub fn parse(title: &str, program: &str, hostname: &str) -> PaneTitle {
    let sanitized = sanitize(title);
    let trimmed = sanitized.trim();
    if trimmed.is_empty() {
        return PaneTitle { status: TitleStatus::NoOpinion, name: None };
    }

    // Codex says this in words rather than a glyph, and blinks between two
    // frames while it does. It is the only title of the three that admits to
    // being blocked, which is why it is worth matching by name.
    if trimmed.contains("Action Required") {
        return PaneTitle { status: TitleStatus::NotWorking, name: None };
    }

    let mut chars = trimmed.chars();
    let first = chars.next().unwrap_or(' ');
    let (status, rest) = if SPINNERS.contains(&first) {
        (TitleStatus::Working, chars.as_str())
    } else if RESTING.contains(&first) {
        (TitleStatus::NotWorking, chars.as_str())
    } else {
        (TitleStatus::NoOpinion, trimmed)
    };

    PaneTitle { status, name: name_from(rest.trim(), program, hostname) }
}

/// A title reduced to text that is safe to put in a row.
///
/// A title is whatever the program in the pane wrote with an OSC sequence, and
/// nothing between there and here inspects it — tmux stores the bytes and hands
/// them back. This is the only choke point before a title becomes a pane's name
/// and travels to every client, so it is where the string stops being arbitrary.
///
/// Five things go, each because it would survive into a row and misrender it —
/// or worse, be read: escape sequences, which a client that draws into a
/// terminal would execute; control characters, which break a single-line row (a
/// newline turns one row into two, a NUL truncates it in most C-backed
/// renderers); the bidi and zero-width format characters, which can reorder the
/// visible text of a row around a name the user never typed; anything
/// credential-shaped; and any length past a sensible bound, since a title has
/// none and this string is broadcast on every tick.
///
/// The redaction is not defensive tidiness. A shell routinely sets the terminal
/// title to the command line it is about to run — zsh's `preexec`, bash's
/// `PROMPT_COMMAND`, fish's `fish_title` all do it by default or by one-line
/// convention — and a command line is precisely what `crate::redact`'s own doc
/// calls "where a token lives". Whatever this returns becomes a pane's name,
/// rides `current_command` to every client, and is the TITLE of the push
/// notification that lands on a locked phone. Until this ran, a pane titled
/// `psql postgres://admin:hunter2@prod-db/app` put that password on a lock
/// screen verbatim.
///
/// Redaction runs BEFORE the length bound, never after: truncating first would
/// cut a secret in half and ship the half that fits, and a token cut in half is
/// a token the scan no longer recognizes.
fn sanitize(title: &str) -> String {
    // Escapes first, so a control character INSIDE a sequence goes with the
    // sequence rather than being turned into a space that survives it.
    let stripped = crate::activity::strip_ansi(title);
    let cleaned: String = stripped
        .chars()
        .map(|c| if c.is_control() || is_invisible_format(c) { ' ' } else { c })
        .collect();
    // Collapsed rather than trimmed: replacing a control character with a space
    // otherwise leaves a gap in the middle of the name where it used to be.
    // Also what the redactor wants: it scans whitespace-separated tokens, and a
    // secret split by a stray control character would not be one to it.
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let collapsed = crate::redact::redact(&collapsed);
    if collapsed.chars().count() <= MAX_TITLE_CHARS {
        return collapsed;
    }
    // One short of the bound, because the ellipsis is a character too and the
    // bound is on what a row has to render.
    let mut short: String = collapsed.chars().take(MAX_TITLE_CHARS - 1).collect();
    // Back to a word boundary, which costs a byte search and reads better than a
    // word cut in half. Only when a word survives the cut: a title of one very
    // long token has no boundary to find and is truncated where it falls.
    if let Some(space) = short.rfind(' ') {
        if space >= short.len() / 2 {
            short.truncate(space);
        }
    }
    short.push('…');
    short
}

/// Long enough for any summary an agent writes, short enough that a row cannot
/// be used as a payload. Claude's longest observed titles run under a hundred.
const MAX_TITLE_CHARS: usize = 120;

/// Whether one word of a title is a filesystem path.
fn is_a_path(token: &str) -> bool {
    let token = token.trim_start_matches(['(', '[', '<', '{', '"', '\'']);
    token.starts_with("~/") || (token.starts_with('/') && token.len() > 1)
}

/// Characters that occupy no width but change how the text around them reads.
///
/// The bidi overrides are the reason this exists: `RLO` makes the rest of a row
/// render right to left, so a title can display as words in an order it does
/// not contain. The zero-width family is here for the same reason at smaller
/// scale — a name that compares unequal to what a person sees.
fn is_invisible_format(c: char) -> bool {
    matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
}

/// Whatever is left of a title once the glyph is gone, if it is a name at all.
fn name_from(rest: &str, program: &str, hostname: &str) -> Option<String> {
    if rest.is_empty() {
        return None;
    }
    // The machine's name, which a plain shell leaves behind.
    if rest.eq_ignore_ascii_case(hostname) {
        return None;
    }
    // A path, which fish leaves behind — abbreviated (`/p/t/c/probe`) or not.
    if rest.starts_with('/') || rest.starts_with('~') {
        return None;
    }
    // bash's default title is `\u@\h: \w`, which is the default on most Linux
    // distributions and therefore on most machines anyone SSHes into. It is a
    // prompt, not a name: `e-liang@Mac: ~/Dev/overnight` in a sidebar row says
    // where the pane is, which the workspace row already said, and hides what it
    // is running.
    //
    // The user-at-host token has to end with a colon to count. `e-liang@prod-db`
    // on its own is a single word and is already rejected below, and a summary
    // that happens to mention an address mid-sentence keeps its name.
    let first = rest.split_whitespace().next().unwrap_or_default();
    if first.contains('@') && first.ends_with(':') {
        return None;
    }
    // A path where FURNITURE puts one, which is at the front.
    //
    // The distinction is the whole rule: a shell or an editor leads with the
    // location, and a sentence mentions one in passing. `Fix the bug in
    // /src/main.rs` and `Investigate why ~/Dev/overnight fails to build` are
    // exactly the summaries this feature exists to put in the sidebar, and an
    // any-token rule threw them away to reject furniture that the leading-token
    // rule already catches.
    //
    // The trade this accepts is vim's `module.rs (~/Dev) - VIM`, which is now
    // kept. That is the better side of it: the title does say which file is
    // being edited, and it is a far rarer shape than a summary naming a path.
    // Bracketing is trimmed so a leading `(~/Dev)` still counts. A lone `/` is
    // not a path here, though a title that begins with one is already rejected
    // by the whole-string check above — this only has to be right about a
    // bracketed leading token.
    if is_a_path(first) {
        return None;
    }
    // The program itself, which says nothing the command did not.
    let base = program.rsplit('/').next().unwrap_or(program);
    if rest.eq_ignore_ascii_case(base) {
        return None;
    }
    // The harness before it has anything to say.
    if PLACEHOLDERS.iter().any(|p| rest.eq_ignore_ascii_case(p)) {
        return None;
    }
    // A single bare word is a directory or a process, not a summary of work.
    //
    // This is what excludes codex, whose title is only the cwd basename — which
    // the workspace row already says, so repeating it costs a row and adds
    // nothing.
    if !rest.contains(' ') {
        return None;
    }
    Some(rest.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spinner_means_working() {
        // Codex cycles braille; claude cycles the quadrant circles.
        for raw in ["⠋ bare", "⠙ bare", "⠧ probe2", "◐ Write a haiku", "◑ Write a haiku"] {
            assert_eq!(parse(raw, "codex", "Mac.attlocal.net").status, TitleStatus::Working, "{raw}");
        }
    }

    #[test]
    fn claude_says_it_is_not_working_with_an_asterisk() {
        assert_eq!(
            parse("✳ Write a haiku", "claude", "Mac.attlocal.net").status,
            TitleStatus::NotWorking
        );
    }

    /// Codex is the only one whose title distinguishes blocked, and it blinks
    /// between two frames while it does.
    #[test]
    fn codex_action_required_is_not_working() {
        for raw in ["[ . ] Action Required | bare", "[ ! ] Action Required | bare"] {
            assert_eq!(parse(raw, "codex", "Mac.attlocal.net").status, TitleStatus::NotWorking, "{raw}");
        }
    }

    #[test]
    fn a_title_with_no_glyph_has_no_opinion() {
        assert_eq!(parse("Echo Banana", "node", "Mac.attlocal.net").status, TitleStatus::NoOpinion);
    }

    #[test]
    fn a_summary_is_a_name() {
        let t = parse("◐ Write tmux haiku and explain escape key behavior", "claude", "Mac.attlocal.net");
        assert_eq!(t.name.as_deref(), Some("Write tmux haiku and explain escape key behavior"));
    }

    /// Four ways a title is not a name.
    #[test]
    fn shell_furniture_is_not_a_name() {
        // The hostname, which is what a plain shell leaves behind.
        assert_eq!(parse("Mac.attlocal.net", "zsh", "Mac.attlocal.net").name, None);
        // A path, which is what fish leaves behind.
        assert_eq!(parse("/p/t/c/-/9/s/probe", "fish", "Mac.attlocal.net").name, None);
        assert_eq!(parse("~/Dev/overnight", "fish", "Mac.attlocal.net").name, None);
        // The program's own name, which says nothing the command did not.
        assert_eq!(parse("node", "node", "Mac.attlocal.net").name, None);
        // The bare harness name, shown before there is a task to describe.
        assert_eq!(parse("✳ Claude Code", "claude", "Mac.attlocal.net").name, None);
        assert_eq!(parse("Cursor Agent", "cursor-agent", "Mac.attlocal.net").name, None);
    }

    /// Codex names the directory, which the workspace row already says.
    #[test]
    fn a_directory_name_is_not_worth_repeating() {
        assert_eq!(parse("⠋ bare", "codex", "Mac.attlocal.net").name, None);
        assert_eq!(parse("[ ! ] Action Required | bare", "codex", "Mac.attlocal.net").name, None);
    }

    /// A title is written by the program in the pane and arrives unexamined.
    #[test]
    fn a_title_is_untrusted_text() {
        // An escape sequence goes whole, and the control characters with it.
        let t = parse("◐ Write \u{1b}[31ma haiku\u{1b}[0m\u{7}\nand\u{0} explain it", "claude", "Mac");
        assert_eq!(t.status, TitleStatus::Working);
        assert_eq!(t.name.as_deref(), Some("Write a haiku and explain it"));

        // A right-to-left override would render a row in an order it does not
        // contain.
        let t = parse("◐ deploy \u{202e}gnitset ot", "claude", "Mac");
        assert_eq!(t.name.as_deref(), Some("deploy gnitset ot"));

        // An escape sequence is not a name, however long it is.
        assert_eq!(parse("\u{1b}]0;\u{7}", "zsh", "Mac").name, Option::None);
    }

    /// A title has no length bound; a row does.
    #[test]
    fn an_endless_title_is_cut_to_a_row() {
        // Measured on `sanitize`, and with no glyph in front of it, because a
        // glyph is a character the name never carries and would hide an
        // off-by-one: the ellipsis has to fit INSIDE the bound, not beside it.
        // One unbroken word, so there is no word boundary to cut back to and
        // the result is exactly the bound.
        let one_long_word = "verbose".repeat(200);
        let cut = sanitize(&one_long_word);
        assert_eq!(cut.chars().count(), MAX_TITLE_CHARS, "{cut}");
        assert!(cut.ends_with('…'), "{cut}");

        // With words in it, the cut lands on a boundary rather than mid-word.
        let many_words = format!("◐ {}", "verbose ".repeat(200));
        let name = parse(&many_words, "claude", "Mac").name.expect("still a summary");
        assert!(name.chars().count() <= MAX_TITLE_CHARS, "{name}");
        assert!(name.ends_with("verbose…"), "{name}");
    }

    /// bash's default title is a prompt, and a prompt is not a name.
    ///
    /// `\u@\h: \w` is the default on most distributions, so this is what a row
    /// says for any pane on a machine nobody has customized.
    #[test]
    fn a_shell_prompt_is_not_a_name() {
        // The prompt: rejected on its `user@host:` token, before the path in it
        // is ever considered.
        assert_eq!(parse("e-liang@Mac: ~/Dev/overnight", "zsh", "Mac").name, Option::None);
        assert_eq!(parse("e-liang@Mac: ~/Dev", "node server.js", "Mac").name, Option::None);
        // A path at the front, which is where furniture puts one.
        assert_eq!(parse("/p/t/c/-/9/s/probe", "fish", "Mac").name, Option::None);
        assert_eq!(parse("~/Dev/overnight", "fish", "Mac").name, Option::None);
        assert_eq!(parse("(~/Dev) module.rs", "vim", "Mac").name, Option::None);

        // Unchanged: still rejected, and still for their original reasons.
        assert_eq!(parse("Mac.attlocal.net", "zsh", "Mac.attlocal.net").name, Option::None);
        assert_eq!(parse("e-liang@prod-db:~", "ssh", "Mac").name, Option::None);
        assert_eq!(parse("claude", "claude", "Mac").name, Option::None);
        assert_eq!(parse("node", "node", "Mac").name, Option::None);
        assert_eq!(parse("✳ Claude Code", "claude", "Mac").name, Option::None);
        assert_eq!(parse("Cursor Agent", "cursor-agent", "Mac").name, Option::None);
        assert_eq!(parse("", "zsh", "Mac").name, Option::None);
    }

    /// A summary that names a file is still a summary.
    ///
    /// This is the regression an any-token path rule caused, and it threw away
    /// exactly the titles this feature exists to show: an agent describing its
    /// work almost always names the thing it is working on.
    #[test]
    fn a_summary_that_mentions_a_path_keeps_its_name() {
        for summary in [
            "Fix the bug in /src/main.rs",
            "Deploy the build to /var/www safely",
            "Rename crates/core/src/ports.rs and update the callers",
            "Investigate why ~/Dev/overnight fails to build",
        ] {
            assert_eq!(
                parse(&format!("◐ {summary}"), "claude", "Mac").name.as_deref(),
                Some(summary),
                "{summary}"
            );
        }
    }

    /// The prompt rules are narrow on purpose.
    #[test]
    fn a_summary_that_mentions_an_address_keeps_its_name() {
        // An `@` mid-sentence is not a user-at-host token.
        let t = parse("◐ Route the @mentions digest to on-call", "claude", "Mac");
        assert_eq!(t.name.as_deref(), Some("Route the @mentions digest to on-call"));
        // Nor is one that ends a word without a colon.
        let t = parse("Email support@example.com about the outage", "claude", "Mac");
        assert_eq!(t.name.as_deref(), Some("Email support@example.com about the outage"));
        // A slash mid-sentence is not a path. A title that BEGINS with one is
        // still rejected, by the older whole-string rule above.
        let t = parse("◐ Add / remove buttons on the row", "claude", "Mac");
        assert_eq!(t.name.as_deref(), Some("Add / remove buttons on the row"));
        assert_eq!(parse("/ split the row", "claude", "Mac").name, Option::None);
    }

    /// A title is a command line often enough that it has to be redacted.
    ///
    /// zsh's `preexec`, bash's `PROMPT_COMMAND` and fish's `fish_title` all set
    /// the terminal title to the running command, so the shapes `crate::redact`
    /// exists for arrive here routinely — and this is the last place before the
    /// string becomes a pane's name, rides the wire, and titles a push
    /// notification on a locked phone.
    #[test]
    fn a_credential_in_a_title_does_not_become_a_name() {
        for (title, secret) in [
            ("psql postgres://admin:hunter2@prod-db:5432/app", "hunter2"),
            (
                "curl -H \"Authorization: Bearer sk-live-9f8a7b6c5d4e3f\" https://api.x/v1",
                "sk-live-9f8a7b6c5d4e3f",
            ),
            ("AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMIKEY deploy.sh", "wJalrXUtnFEMIKEY"),
        ] {
            let name = parse(title, "zsh", "Mac").name.unwrap_or_default();
            assert!(!name.contains(secret), "{title} -> {name}");
            // The shape survives, so the row still says what the pane is doing.
            assert!(!name.is_empty(), "{title} -> the whole name was thrown away");
        }
    }

    /// The cost of that redaction on the titles this feature exists to show.
    #[test]
    fn an_agent_summary_is_not_redacted() {
        for summary in [
            "Write tmux haiku and explain escape key behavior",
            "Fix the bug in /src/main.rs",
        ] {
            assert_eq!(
                parse(&format!("◐ {summary}"), "claude", "Mac").name.as_deref(),
                Some(summary),
                "{summary}"
            );
        }
    }

    /// A secret must not be cut in half and shipped.
    ///
    /// Redaction runs before the length bound. Truncating first would leave the
    /// leading characters of a token in the row — and a token cut in half is
    /// one the scan no longer recognizes, so it would never be redacted at all.
    #[test]
    fn a_credential_past_the_length_bound_still_goes() {
        // Sized so the assignment BEGINS inside the bound and ENDS outside it:
        // that is the only arrangement where the two orderings disagree.
        let padding = "deploy ".repeat(11);
        let title = format!("◐ {padding}AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMIKEYwJalrXUtnFEMIKEY");
        assert!(title.chars().count() > MAX_TITLE_CHARS, "the case needs to exceed the bound");
        let name = parse(&title, "claude", "Mac").name.expect("still a summary");
        assert!(!name.contains("wJalrXUtnFEMIKEY"), "{name}");
        assert!(name.chars().count() <= MAX_TITLE_CHARS, "{name}");
        // Redacted whole, not cut: truncating first would have ended this in
        // `…FEMIK…`, the first fourteen characters of a live key.
        assert!(name.ends_with("AWS_SECRET_ACCESS_KEY=…"), "{name}");
    }

    #[test]
    fn nothing_in_means_nothing_out() {
        let t = parse("", "zsh", "Mac.attlocal.net");
        assert_eq!(t.status, TitleStatus::NoOpinion);
        assert_eq!(t.name, None);
    }
}
