# Agent Activity and Pane Names — Stage 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop a finished agent reading as `Working` forever, anchor its timer to the turn rather than to the classifier changing its mind, and make every pane — agent or not — say what it is and what it is doing.

**Architecture:** `Registry::classify` currently greps the whole visible pane, so an agent's own transcript triggers its signatures. Scope the match to the last 8 non-blank lines, where every real signature lives (measured: depth ≤ 6 for all three agents; nearest false positive at 18). Then add the signals already on offer and unused — the OSC pane title, the blocked question text, listening ports — and split the single `changed_at` clock into a turn clock and a state clock.

**Tech Stack:** Rust 2021 workspace, `cargo` at `~/.cargo/bin/cargo` (not on `PATH`), prost/protobuf wire, tmux control mode, SwiftUI on the Mac.

## Global Constraints

- **US English throughout**, in code and copy. Never "authorise", "colour", "centre".
- **Never run `cargo fmt`.** The Rust tree is hand-formatted and CI skips `fmt --check` on purpose. Match surrounding style by hand.
- **`cargo` is not on `PATH`.** Use `~/.cargo/bin/cargo` in every command.
- **Do not set `FARCOOLER_HOME`** when running `farcooler-daemon` tests. It breaks `paths::tests`, which read the real home. Baseline without it is green.
- **Apple copy conventions** in user-visible strings: title-case buttons, contractions, "machine" not "host", never a raw Rust error in the UI.
- **A live Far Cooler app is running on this machine.** Never `pkill` by broad pattern. Kill scratch tmux servers by their exact `-L` socket name.
- **Comments explain why, not what.** This codebase's comments carry the reasoning and the evidence; match that. A comment that restates the code is worse than none.
- **Baseline:** `farcooler-core` 145 tests, `farcooler-daemon` 195 tests, all passing.

## The corpus

`crates/core/captures/` holds 13 real screens from the publicly shipped binaries — claude 2.1.233, Homebrew codex 0.147.0, cursor-agent 2026.08.11 — with `README.md` recording provenance. Tests load them with `include_str!`.

The one that matters most is `claude-idle-transcript-says-esc-to-interrupt.txt`: a **finished, idle** Claude pane whose footer reads `? for shortcuts` (depth 1) and whose transcript, at depths 24 and 38, explains what `esc to interrupt` means. It classifies as `Working` today and must classify as `Idle`.

## File structure

| file | responsibility |
| --- | --- |
| `crates/core/src/activity.rs` (modify) | footer slicing, classification, rules table, question extraction |
| `crates/core/src/redact.rs` (create) | strip credential-shaped text before anything leaves the machine |
| `crates/core/src/title.rs` (create) | parse an OSC pane title into a status hint and a name candidate |
| `crates/core/src/ports.rs` (create) | listening ports for a process group |
| `crates/daemon/src/foreground.rs` (modify) | argv summary that keeps the informative token |
| `crates/daemon/src/watch.rs` (modify) | two clocks, hysteresis, feed the new fields |
| `crates/tmux/src/windows.rs` (modify) | ask tmux for `#{pane_title}` |
| `crates/core/src/inventory.rs` (modify) | carry `title` on `TaggedPane` |
| `proto/farcooler.proto` (modify) | `turn_started_at`, `blocked_question` |
| `crates/daemon/src/wire.rs`, `rpc.rs` (modify) | plumb the new fields |
| `apps/macos/Sources/FarCooler/Model.swift` (modify) | render two clocks, failed exits |

---

### Task 1: Scope classification to the footer

The core fix. Everything else is additive; this one changes an answer that is wrong today.

**Files:**
- Modify: `crates/core/src/activity.rs` (`plain_text` at ~line 530, `AgentRules` at ~line 69, `classify` at ~line 575, `Registry::built_in` at ~line 206)
- Test: `crates/core/src/activity.rs` (the existing `mod tests`)

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `pub fn strip_ansi(screen: &str) -> String` — escape sequences removed, newlines and spacing preserved.
  - `pub fn footer_text(screen: &str, lines: usize) -> String` — the last `lines` non-blank-trailing lines, whitespace collapsed, joined with a single space. For matching.
  - `pub fn footer_lines(screen: &str, lines: usize) -> Vec<String>` — the same slice, one entry per line, each collapsed. For extraction.
  - `AgentRules.footer_lines: usize` — new field, `8` for every built-in.
  - `pub const DEFAULT_FOOTER_LINES: usize = 8;`

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `crates/core/src/activity.rs`:

```rust
/// The bug this whole change exists for, frozen.
///
/// A finished Claude pane whose transcript explains what `esc to interrupt`
/// means. Matching the whole screen calls it Working forever: `Done` never
/// fires, no notification arrives, and the timer counts from whenever that
/// text first appeared. Measured depths: the live footer at 1, the transcript
/// at 24 and 38.
#[test]
fn a_finished_agent_is_not_working_because_it_said_the_words() {
    let screen = include_str!("../captures/claude-idle-transcript-says-esc-to-interrupt.txt");
    let r = Registry::built_in();
    assert_eq!(r.classify("claude", screen), AgentActivity::Idle);
}

#[test]
fn the_footer_is_the_last_lines_with_trailing_blanks_dropped() {
    let screen = "alpha\nbravo\ncharlie\ndelta\n\n\n";
    assert_eq!(footer_lines(screen, 2), vec!["charlie".to_string(), "delta".to_string()]);
    assert_eq!(footer_text(screen, 2), "charlie delta");
    // Asking for more lines than exist is the whole screen, not a panic.
    assert_eq!(footer_lines(screen, 99).len(), 4);
}

#[test]
fn the_footer_strips_escapes_the_way_the_whole_screen_does() {
    let screen = "\u{1b}[38;5;153mfirst\u{1b}[39m\n\u{1b}[1mesc to interrupt\u{1b}[0m\n";
    assert_eq!(footer_text(screen, 1), "esc to interrupt");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `~/.cargo/bin/cargo test -p farcooler-core activity:: 2>&1 | tail -20`

Expected: FAIL. The two footer tests fail to compile (`footer_lines` not found); once that is fixed, `a_finished_agent_is_not_working_because_it_said_the_words` fails with `Working != Idle`.

- [ ] **Step 3: Factor the escape stripping out of `plain_text`**

`plain_text` today strips escapes and collapses all whitespace in one pass, which destroys line structure. Split it, keeping `plain_text`'s behavior byte-for-byte identical so nothing that calls it changes:

```rust
/// Strip escape sequences, keeping every other byte — newlines included.
///
/// Split out of `plain_text` because the footer needs line structure and
/// `plain_text` deliberately destroys it. The escape handling is the same
/// and lives in one place, so a future OSC quirk is fixed once.
pub fn strip_ansi(screen: &str) -> String {
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
    out
}
```

Then rewrite `plain_text`'s body to reuse it, leaving its doc comment as it is:

```rust
pub fn plain_text(screen: &str) -> String {
    strip_ansi(screen).split_whitespace().collect::<Vec<_>>().join(" ")
}
```

- [ ] **Step 4: Add the footer helpers**

Add below `plain_text`:

```rust
/// How much of a pane's bottom is furniture rather than conversation.
///
/// Measured, not chosen. Across claude, codex and cursor every signature that
/// decides a state sits at depth 6 or less, and the nearest transcript false
/// positive is at 18 — see `captures/` and the design document. Eight leaves
/// ten lines of margin without reaching into anything anyone said.
pub const DEFAULT_FOOTER_LINES: usize = 8;

/// The bottom `lines` lines of a pane, as separate lines.
///
/// Trailing blank lines are dropped first: a pane is padded to its full height,
/// so counting from the literal end would spend the whole window on empty rows.
/// Interior blanks are kept, because they are part of the block being read.
pub fn footer_lines(screen: &str, lines: usize) -> Vec<String> {
    let stripped = strip_ansi(screen);
    let mut all: Vec<&str> = stripped.lines().collect();
    while all.last().is_some_and(|l| l.trim().is_empty()) {
        all.pop();
    }
    let start = all.len().saturating_sub(lines);
    all[start..]
        .iter()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect()
}

/// The bottom `lines` lines as one string, for substring matching.
///
/// Joined with a space rather than a newline so a signature that wrapped across
/// two rows still matches — the same tolerance `plain_text` has always had, and
/// the reason the rules can be written as plain phrases.
pub fn footer_text(screen: &str, lines: usize) -> String {
    footer_lines(screen, lines).join(" ")
}
```

- [ ] **Step 5: Add the field to `AgentRules`**

In the struct at ~line 69, after `working`:

```rust
    /// How many lines from the bottom `blocked` and `working` may look at.
    ///
    /// The fix for an agent that read `Working` for thirty-three hours because
    /// it had been asked to explain the phrase `esc to interrupt`. Signatures
    /// are furniture, furniture is drawn at the bottom, and the conversation
    /// scrolls above it — so the conversation is simply not in scope.
    ///
    /// A field rather than a constant so an agent added by config can widen it.
    /// Every built-in takes `DEFAULT_FOOTER_LINES`.
    pub footer_lines: usize,
```

- [ ] **Step 6: Set it on every built-in and every constructor**

Add `footer_lines: DEFAULT_FOOTER_LINES,` to all four `AgentRules` literals in `Registry::built_in` (claude, codex, opencode, cursor), and to the `None => self.rules.push(AgentRules { … })` arm in `apply_config` at ~line 500.

- [ ] **Step 7: Scope `classify` to the footer**

Replace the body of `classify` (~line 575):

```rust
    pub fn classify(&self, command: &str, screen: &str) -> AgentActivity {
        let Some(rules) = self.identify(command, screen) else {
            return AgentActivity::None;
        };
        // Identity is asked of the WHOLE screen and state only of the footer.
        //
        // Not an inconsistency. Identity strings are furniture an agent always
        // draws, and one appearing in a transcript promotes nothing on its own
        // — the banner is at the top, which is exactly where the footer window
        // cannot see. State is the opposite: `esc to interrupt` is a phrase a
        // person can type and an agent can explain, so it only means anything
        // where the agent draws its own status.
        let footer = footer_text(screen, rules.footer_lines);

        // Blocked first. An agent asking permission still shows its working
        // furniture, so checking working first would hide every question.
        if rules.blocked.iter().any(|needle| footer.contains(needle.as_str())) {
            return AgentActivity::Blocked;
        }
        if rules.working.iter().any(|needle| footer.contains(needle.as_str())) {
            return AgentActivity::Working;
        }
        // Identified, not asking, not busy: it is sitting there. Requiring positive
        // idle furniture left an agent whose footer changed between versions stuck
        // on `unknown` forever — never reaching `done`, never notifying.
        AgentActivity::Idle
    }
```

- [ ] **Step 8: Run the tests**

Run: `~/.cargo/bin/cargo test -p farcooler-core 2>&1 | tail -25`

Expected: PASS, and the count is above the 145 baseline. If an existing test fails, read it before changing it — several construct screens by hand and may put a signature above the window; those are fixtures to correct, not behavior to revert.

- [ ] **Step 9: Add the property test**

This is what stops the bug coming back by a different route:

```rust
/// No classification may depend on text above the footer window.
///
/// The bug was born by adding a signature to a whole-screen match. This asserts
/// the shape of the fix rather than one instance of it: bury every signature in
/// the table under a thousand lines of transcript and nothing may move.
#[test]
fn nothing_above_the_footer_can_change_a_verdict() {
    let r = Registry::built_in();
    let every_signature: String = r
        .all()
        .iter()
        .flat_map(|rules| rules.blocked.iter().chain(rules.working.iter()))
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");

    for (command, capture) in [
        ("claude", include_str!("../captures/claude-working.txt")),
        ("claude", include_str!("../captures/claude-blocked.txt")),
        ("claude", include_str!("../captures/claude-idle-fresh.txt")),
        ("codex", include_str!("../captures/codex-working.txt")),
        ("codex", include_str!("../captures/codex-blocked.txt")),
    ] {
        let clean = r.classify(command, capture);
        let noise = "filler line\n".repeat(1000);
        let poisoned = format!("{noise}{every_signature}\n{capture}");
        assert_eq!(
            r.classify(command, &poisoned),
            clean,
            "{command}: a transcript above the footer changed the verdict"
        );
    }
}
```

- [ ] **Step 10: Run it**

Run: `~/.cargo/bin/cargo test -p farcooler-core activity:: 2>&1 | tail -20`

Expected: PASS.

- [ ] **Step 11: Commit**

```bash
git add crates/core/src/activity.rs
git commit -m "fix: an agent that explained 'esc to interrupt' was not doing it

classify matched its signature table against the whole visible pane, and
the visible pane holds the transcript. A finished Claude pane that had been
asked what the phrase means read Working for thirty-three hours: Done never
fired, so no notification arrived and the timer counted from whenever that
text was first drawn.

Signatures are furniture and furniture is drawn at the bottom. Blocked and
working now match the last eight lines only, which is measured rather than
picked -- every deciding signature across the three agents sits at depth 6
or less, and the nearest transcript hit is at 18. Identity still reads the
whole screen, because a banner is at the top and an identity string in a
transcript promotes nothing on its own.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_011A27xRp5hrevabCQpUYbZB"
```

---

### Task 2: Correct the rules against the captures

Cursor's entry is guesswork the code itself flags as unverified, and it is wrong. Codex has one string that matches nothing.

**Files:**
- Modify: `crates/core/src/activity.rs` (`Registry::built_in`, codex at ~line 231 and cursor at ~line 331)
- Test: `crates/core/src/activity.rs`

**Interfaces:**
- Consumes: `footer_text`, `AgentRules.footer_lines` from Task 1.
- Produces: no new API. Behavior only.

- [ ] **Step 1: Write the failing tests**

```rust
/// Cursor's real furniture, read off the shipped binary.
///
/// The table shipped with `Do you want to`, `Allow?` and `> 1.`, none of which
/// cursor-agent has ever drawn — the entry was written against a sign-in screen
/// nobody could get past, and its own comments said so.
#[test]
fn cursor_is_classified_from_its_real_screens() {
    let r = Registry::built_in();
    let working = include_str!("../captures/cursor-working.txt");
    let blocked = include_str!("../captures/cursor-blocked.txt");
    let idle = include_str!("../captures/cursor-idle.txt");
    let trust = include_str!("../captures/cursor-trust-gate.txt");

    assert_eq!(r.classify("cursor-agent", working), AgentActivity::Working);
    assert_eq!(r.classify("cursor-agent", blocked), AgentActivity::Blocked);
    assert_eq!(r.classify("cursor-agent", idle), AgentActivity::Idle);
    // The first screen a new workspace shows, and it blocks everything behind
    // it, so a pane sitting on it is precisely a pane that needs you.
    assert_eq!(r.classify("cursor-agent", trust), AgentActivity::Blocked);
}

/// cursor-agent runs as plain `node`, so identity has to carry it.
#[test]
fn cursor_is_found_without_a_usable_process_name() {
    let r = Registry::built_in();
    let working = include_str!("../captures/cursor-working.txt");
    assert_eq!(r.identify("node", working).map(|x| x.preset.as_str()), Some("cursor"));
}

#[test]
fn codex_is_classified_from_its_real_screens() {
    let r = Registry::built_in();
    assert_eq!(
        r.classify("codex", include_str!("../captures/codex-working.txt")),
        AgentActivity::Working
    );
    assert_eq!(
        r.classify("codex", include_str!("../captures/codex-blocked.txt")),
        AgentActivity::Blocked
    );
    assert_eq!(
        r.classify("codex", include_str!("../captures/codex-idle-after-turn.txt")),
        AgentActivity::Idle
    );
    assert_eq!(
        r.classify("codex", include_str!("../captures/codex-trust-gate.txt")),
        AgentActivity::Blocked
    );
}

/// The approval prompt says "confirm", not "continue".
///
/// `Press enter to continue` was in the table and matched nothing on it; only
/// `> 1.` was ever firing. The string it was written for is on the trust gate,
/// which is a different screen.
#[test]
fn codex_approval_and_trust_are_spelled_as_codex_spells_them() {
    let r = Registry::built_in();
    let codex = r.all().iter().find(|x| x.preset == "codex").expect("codex is built in");
    assert!(codex.blocked.iter().any(|s| s == "Press enter to confirm"));
    assert!(codex.blocked.iter().any(|s| s == "Press enter to continue"));
}

#[test]
fn claude_is_classified_from_its_real_screens() {
    let r = Registry::built_in();
    assert_eq!(
        r.classify("claude", include_str!("../captures/claude-working.txt")),
        AgentActivity::Working
    );
    assert_eq!(
        r.classify("claude", include_str!("../captures/claude-blocked.txt")),
        AgentActivity::Blocked
    );
    assert_eq!(
        r.classify("claude", include_str!("../captures/claude-idle-fresh.txt")),
        AgentActivity::Idle
    );
    assert_eq!(
        r.classify("claude", include_str!("../captures/claude-trust-gate.txt")),
        AgentActivity::Blocked
    );
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `~/.cargo/bin/cargo test -p farcooler-core activity::tests::cursor 2>&1 | tail -20`

Expected: FAIL — cursor's working screen classifies as `Idle` (no signature matches) and its blocked screen likewise.

- [ ] **Step 3: Replace cursor's rules**

In `Registry::built_in`, replace the cursor entry's `identity`, `blocked` and `working` (keep `preset`, `commands` and `adapter`):

```rust
                    // Read off cursor-agent 2026.08.11, not guessed. The entry
                    // this replaces was written against a sign-in screen that
                    // could never be got past, and matched nothing real.
                    identity: s(&["Cursor Agent", "→ Add a follow-up", "cursor-agent"]),
                    blocked: s(&[
                        "Run this command?",
                        "Not in allowlist:",
                        "Skip & tell the agent what to do instead",
                        "Run Everything (shift+tab)",
                        // The trust gate. First screen of a new workspace, and
                        // everything is behind it.
                        "Trust this workspace",
                        "Do you trust the contents of this directory?",
                    ]),
                    // `ctrl+c to stop` and nothing else.
                    //
                    // The spinner line `⠞ Working` was drawn in one observed run
                    // and absent from another that was working just as hard, so
                    // it is drawn for some turns and not others. A signature
                    // that is sometimes there is worse than none: it makes the
                    // absence of the string mean nothing at all.
                    working: s(&["ctrl+c to stop"]),
```

- [ ] **Step 4: Correct codex's blocked list**

In the codex entry, replace `blocked`:

```rust
                    blocked: s(&[
                        // Codex draws every choice as a numbered list under a `›` marker.
                        "\u{203a} 1.",
                        // The approval prompt, spelled as codex spells it. The
                        // table had `Press enter to continue`, which is the
                        // TRUST gate's wording — so on an approval prompt only
                        // the `›` marker was ever matching.
                        "Press enter to confirm",
                        "Would you like to run the following command?",
                        // The trust gate, which really does say "continue".
                        "Press enter to continue",
                        "Do you trust the contents of this directory?",
                        "Allow command",
                        "Do you want to",
                        "[y/n]",
                        "(y/N)",
                    ]),
```

- [ ] **Step 5: Run the tests**

Run: `~/.cargo/bin/cargo test -p farcooler-core 2>&1 | tail -20`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/activity.rs
git commit -m "fix: cursor's rules were guesses, and they were wrong

The table's own comments admitted the cursor entry was written against a
sign-in screen nobody could get past. Checked against the shipped binary:
'Do you want to', 'Allow?', 'Generating' and '> 1.' appear nowhere. What
cursor actually draws is 'ctrl+c to stop' while working and 'Run this
command?' with 'Not in allowlist:' when it needs you.

The spinner line 'Working' is deliberately left out. It appeared in one run
and was absent from another that was working just as hard, so its absence
would mean nothing.

Codex had one of the same kind: 'Press enter to continue' is the trust
gate's wording, not the approval prompt's, which says 'confirm' -- so only
the numbered-list marker was ever matching a permission request. Both
spellings are now present, and both trust gates classify as blocked, since
a pane sitting on one is exactly a pane that needs you.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_011A27xRp5hrevabCQpUYbZB"
```

---

### Task 3: Redact credential-shaped text

Built before the question extraction that needs it, because the question goes on the wire the moment it exists.

**Files:**
- Create: `crates/core/src/redact.rs`
- Modify: `crates/core/src/lib.rs` (add `pub mod redact;`)
- Test: in `crates/core/src/redact.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `pub fn redact(text: &str) -> String`.

- [ ] **Step 1: Write the failing test**

Create `crates/core/src/redact.rs`:

```rust
//! Strip anything credential-shaped before it leaves the machine.
//!
//! A pane's feed and its blocked question are put on the wire and into push
//! payloads, so they reach a phone through the relay. The strings involved are
//! short, but one of them is a shell command, and a shell command is exactly
//! where a token lives: `curl -H "Authorization: Bearer …"`, an inline
//! `AWS_SECRET_ACCESS_KEY=`, a URL with a key in the query.
//!
//! Conservative on purpose. A redaction that fires on something harmless costs
//! a few characters of context; one that misses puts a live credential on a
//! lock screen.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bearer_token_does_not_travel() {
        let out = redact("curl -H \"Authorization: Bearer sk-abc123DEF456ghi789\" https://api.example.com");
        assert!(!out.contains("sk-abc123DEF456ghi789"), "{out}");
        assert!(out.contains("Bearer"), "the shape stays, so the line still reads: {out}");
    }

    #[test]
    fn a_secret_environment_assignment_does_not_travel() {
        for raw in [
            "AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMIK7MDENGbPxRfiCYEXAMPLEKEY make deploy",
            "GITHUB_TOKEN=ghp_16C7e42F292c6912E7710c838347Ae178B4a cargo publish",
            "export DATABASE_PASSWORD=hunter2correcthorse",
            "MY_API_KEY=abcdef123456 npm start",
        ] {
            let out = redact(raw);
            assert!(!out.contains("wJalrXUtnFEMIK7MDENGbPxRfiCYEXAMPLEKEY"), "{out}");
            assert!(!out.contains("ghp_16C7e42F292c6912E7710c838347Ae178B4a"), "{out}");
            assert!(!out.contains("hunter2correcthorse"), "{out}");
            assert!(!out.contains("abcdef123456"), "{out}");
        }
    }

    #[test]
    fn a_key_in_a_url_query_does_not_travel() {
        let out = redact("curl 'https://api.example.com/v1/thing?api_key=live_9f8e7d6c5b4a3210&page=2'");
        assert!(!out.contains("live_9f8e7d6c5b4a3210"), "{out}");
        assert!(out.contains("page=2"), "the harmless parameter survives: {out}");
    }

    #[test]
    fn a_password_flag_does_not_travel() {
        let out = redact("mysql -u root --password supersecretvalue mydb");
        assert!(!out.contains("supersecretvalue"), "{out}");
    }

    #[test]
    fn an_ordinary_command_is_left_alone() {
        // The cost of a false positive is a line that stops making sense, so
        // the common cases are asserted to pass through untouched.
        for raw in [
            "cargo test -p farcooler-core",
            "git commit -m \"fix the parser\"",
            "pnpm dev",
            "Would you like to run the following command?",
            "vim src/some/deeply/nested/module.rs",
        ] {
            assert_eq!(redact(raw), raw, "a harmless line was redacted");
        }
    }

    #[test]
    fn nothing_in_means_nothing_out() {
        assert_eq!(redact(""), "");
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `~/.cargo/bin/cargo test -p farcooler-core redact 2>&1 | tail -20`

Expected: FAIL to compile — `redact` is not defined, and the module is not declared.

- [ ] **Step 3: Declare the module**

In `crates/core/src/lib.rs`, add `pub mod redact;` beside the other module declarations, in alphabetical position.

- [ ] **Step 4: Implement it**

Add above `mod tests` in `crates/core/src/redact.rs`:

```rust
/// Words that make the thing after them a secret.
const SECRET_WORDS: &[&str] = &[
    "SECRET", "TOKEN", "PASSWORD", "PASSWD", "APIKEY", "API_KEY", "ACCESS_KEY",
    "PRIVATE_KEY", "CREDENTIAL", "AUTH",
];

/// `text` with anything credential-shaped replaced by an ellipsis.
///
/// Hand-written rather than regex-driven: this crate has no regex dependency,
/// the shapes are few, and a hand-rolled scan is easier to read than the
/// alternation that would replace it.
pub fn redact(text: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut redact_next = false;

    for token in text.split_whitespace() {
        if redact_next {
            redact_next = false;
            // A flag with no operand is followed by another flag, not a value.
            if !token.starts_with('-') {
                out.push("…".to_string());
                continue;
            }
        }

        if introduces_a_secret(token) {
            out.push(token.to_string());
            redact_next = true;
            continue;
        }

        out.push(redact_token(token));
    }

    // `split_whitespace` collapses runs of spaces, so a line that needed no
    // redaction would still come back subtly different. Returning the original
    // keeps `redact` a no-op on the overwhelmingly common case.
    let rebuilt = out.join(" ");
    if rebuilt == text.split_whitespace().collect::<Vec<_>>().join(" ") {
        return text.to_string();
    }
    rebuilt
}

/// Whether the token AFTER this one is the secret.
///
/// Two shapes, one rule: `--password hunter2`, and the `Bearer sk-…` that an
/// `Authorization:` header splits into two words.
fn introduces_a_secret(token: &str) -> bool {
    if token.eq_ignore_ascii_case("bearer") {
        return true;
    }
    let flag = token.trim_start_matches('-').replace('-', "_").to_ascii_uppercase();
    SECRET_WORDS.iter().any(|w| flag == *w)
}

/// One whitespace-free token, redacted if it carries a secret inside it.
fn redact_token(token: &str) -> String {
    // A URL query is handled first and per-parameter, so a key in the query
    // does not take the rest of the URL down with it — `?api_key=…&page=2`
    // should still say which page.
    if token.contains('?') && token.contains('=') {
        return redact_query(token);
    }

    // NAME=value, where NAME says it is a secret.
    if let Some((name, value)) = token.split_once('=') {
        if !value.is_empty() {
            let upper = name.trim_start_matches('-').replace('-', "_").to_ascii_uppercase();
            if SECRET_WORDS.iter().any(|w| upper.contains(*w)) {
                return format!("{name}=…");
            }
        }
    }

    token.to_string()
}

/// A URL with its secret query parameters replaced, the rest intact.
fn redact_query(token: &str) -> String {
    let (head, query) = match token.split_once('?') {
        Some(parts) => parts,
        None => ("", token),
    };
    let cleaned: Vec<String> = query
        .split('&')
        .map(|pair| match pair.split_once('=') {
            Some((k, v)) if !v.is_empty() => {
                let upper = k.to_ascii_uppercase().replace('-', "_");
                if SECRET_WORDS.iter().any(|w| upper.contains(*w)) {
                    format!("{k}=…")
                } else {
                    pair.to_string()
                }
            }
            _ => pair.to_string(),
        })
        .collect();
    if head.is_empty() {
        cleaned.join("&")
    } else {
        format!("{head}?{}", cleaned.join("&"))
    }
}

```

- [ ] **Step 5: Run the tests**

Run: `~/.cargo/bin/cargo test -p farcooler-core redact 2>&1 | tail -25`

Expected: PASS, all six. If `an_ordinary_command_is_left_alone` fails, the rule that fired is too broad — narrow it rather than weakening the test.

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/redact.rs crates/core/src/lib.rs
git commit -m "feat: strip credentials before anything leaves the machine

A pane's blocked question and its feed go on the wire and into push
payloads, so they reach a phone through the relay. They are short strings,
but one of them is a shell command, and a shell command is exactly where a
token lives.

Conservative in the direction that matters: a redaction firing on something
harmless costs a few characters of context, and one that misses puts a live
credential on a lock screen. Ordinary commands are asserted to pass through
untouched, because a rule broad enough to mangle 'cargo test' would get
turned off.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_011A27xRp5hrevabCQpUYbZB"
```

---

### Task 4: Extract the blocked question

**Files:**
- Modify: `crates/core/src/activity.rs`
- Test: `crates/core/src/activity.rs`

**Interfaces:**
- Consumes: `footer_lines` (Task 1), `redact` (Task 3), `AgentRules` (Task 1).
- Produces:
  - `pub const QUESTION_LINES: usize = 12;`
  - `pub fn blocked_question(&self, command: &str, screen: &str) -> Option<String>` on `Registry`.

- [ ] **Step 1: Write the failing test**

```rust
/// What the agent is asking, so a row can say more than "Needs you".
///
/// Extraction reads a WIDER window than classification: codex puts its
/// question at depth 11, where every state signature sits at 6 or less. That
/// is safe only because extraction runs after the state is already Blocked and
/// never feeds back into deciding it.
#[test]
fn the_question_is_read_from_the_blocked_screen() {
    let r = Registry::built_in();

    let claude = r
        .blocked_question("claude", include_str!("../captures/claude-blocked.txt"))
        .expect("claude asks a question on its blocked screen");
    assert!(claude.contains("Do you want to create haiku.txt?"), "{claude}");

    let codex = r
        .blocked_question("codex", include_str!("../captures/codex-blocked.txt"))
        .expect("codex asks a question on its blocked screen");
    assert!(codex.contains("Would you like to run"), "{codex}");
    // The subject matters as much as the question: "run WHAT".
    assert!(codex.contains("echo banana"), "{codex}");

    let cursor = r
        .blocked_question("cursor-agent", include_str!("../captures/cursor-blocked.txt"))
        .expect("cursor asks a question on its blocked screen");
    assert!(cursor.contains("Run this command?"), "{cursor}");
}

#[test]
fn a_pane_that_is_not_blocked_has_no_question() {
    let r = Registry::built_in();
    assert_eq!(r.blocked_question("claude", include_str!("../captures/claude-working.txt")), None);
    assert_eq!(r.blocked_question("claude", include_str!("../captures/claude-idle-fresh.txt")), None);
    // Not an agent at all.
    assert_eq!(r.blocked_question("zsh", "$ "), None);
}

/// A command in a question is still a command, and a command is where the
/// tokens are. Redaction runs before the string can reach a wire.
#[test]
fn a_question_carrying_a_secret_is_redacted() {
    let r = Registry::built_in();
    let screen = "Cursor Agent\n\
                  Run this command?\n\
                  $ curl -H \"Authorization: Bearer sk-live-abc123def456\" https://api.example.com\n\
                  → Run (once) (y)\n\
                  Skip & tell the agent what to do instead\n";
    let q = r.blocked_question("cursor-agent", screen).expect("blocked");
    assert!(!q.contains("sk-live-abc123def456"), "{q}");
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `~/.cargo/bin/cargo test -p farcooler-core activity::tests::the_question 2>&1 | tail -20`

Expected: FAIL to compile — `blocked_question` is not a method on `Registry`.

- [ ] **Step 3: Implement it**

Add to the `impl Registry` block that holds `classify`:

```rust
/// How far up an agent's own prompt block can reach.
///
/// Wider than the decision window because codex puts its question at depth 11
/// while every state signature sits at 6 or less. Safe only in this direction:
/// extraction runs when the state is ALREADY Blocked, and what it returns never
/// feeds back into deciding that.
pub const QUESTION_LINES: usize = 12;

impl Registry {
    /// What this agent is asking, when it is asking something.
    ///
    /// A row that says "Needs you" makes a person go and look. A row that says
    /// what is being asked lets them decide from where they are — which on a
    /// lock screen is most of the value.
    ///
    /// All three agents draw the same shape: a line ending in `?`, and often a
    /// `$`-prefixed command naming the subject. So the question is the last
    /// line ending in `?`, and the subject is the nearest `$` line above it.
    ///
    /// Always optional. A wording that changes leaves this `None` and the row
    /// reads "Needs you" exactly as it does today. Nothing regresses when an
    /// agent is redecorated; it just stops elaborating.
    pub fn blocked_question(&self, command: &str, screen: &str) -> Option<String> {
        if self.classify(command, screen) != AgentActivity::Blocked {
            return None;
        }
        let lines = footer_lines(screen, QUESTION_LINES);
        let asked = lines.iter().rposition(|l| l.trim_end().ends_with('?'))?;
        let question = lines[asked].trim().to_string();

        // "Run this command?" is not worth showing without the command. The
        // subject may sit above the question (codex, cursor) or below it, so
        // both sides of the prompt block are searched, nearest first.
        let subject = lines[..asked]
            .iter()
            .rev()
            .chain(lines[asked + 1..].iter())
            .find(|l| l.trim_start().starts_with("$ "))
            .map(|l| l.trim().trim_start_matches("$ ").trim().to_string());

        let joined = match subject {
            Some(s) if !s.is_empty() => format!("{question} {s}"),
            _ => question,
        };
        Some(crate::redact::redact(&joined))
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `~/.cargo/bin/cargo test -p farcooler-core activity:: 2>&1 | tail -20`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/activity.rs
git commit -m "feat: say what the agent is asking, not just that it is asking

'Needs you' makes a person walk to the machine to find out what for. All
three agents draw the same shape -- a line ending in '?', usually with a
'\$'-prefixed command naming the subject -- so the row can say 'Run this
command? cargo test' instead.

Extraction reads twelve lines where classification reads eight, because
codex puts its question at depth 11. That is only safe in this direction:
it runs when the state is already Blocked and never feeds back into
deciding it, so a wider window cannot reintroduce the bug it was narrowed
to fix.

Always optional. An agent that rewords its prompt leaves this empty and the
row reads as it does today.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_011A27xRp5hrevabCQpUYbZB"
```

---

### Task 5: Two clocks and hysteresis

**Files:**
- Modify: `proto/farcooler.proto` (`Terminal`, after field 22)
- Modify: `crates/daemon/src/watch.rs` (`Observed` at ~line 160, the sample loop at ~line 619-681, `activity()` at ~line 248, `mark_seen` at ~line 278)
- Modify: `crates/daemon/src/wire.rs` (`terminal` at ~line 144), `crates/daemon/src/rpc.rs` (`with_activity` at ~line 1482)
- Test: `crates/daemon/src/watch.rs`

**Interfaces:**
- Consumes: `activity::advance`, `activity::seen` (unchanged).
- Produces:
  - proto `optional google.protobuf.Timestamp turn_started_at = 23;`
  - proto `optional string blocked_question = 24;`
  - `Observed { activity, turn_started_at: Option<i64>, state_since: i64, state, command, chat_capable, blocked_question: Option<String>, pending: Option<(AgentActivity, u8)> }`
  - `Watcher::activity(&self, Uuid) -> (AgentActivity, Option<i64>, Option<i64>)` — activity, `state_since`, `turn_started_at`.

- [ ] **Step 1: Add the proto fields**

In `proto/farcooler.proto`, inside `message Terminal` after `bool chat_capable = 22;`:

```proto
  // When the USER'S REQUEST started, as distinct from when the label last
  // changed.
  //
  // `activity_changed_at` restarts whenever the classifier changes its mind, so
  // approving a permission prompt walks Working -> Blocked -> Working and turns
  // "working 12m" into "working 2s". This one is set when an idle agent starts
  // working and is HELD across Blocked, because saying yes to a tool call does
  // not begin a new turn. Cleared when the turn ends.
  //
  // Both are sent because they answer different questions: how long this task
  // has been running, and how long it has been stuck.
  optional google.protobuf.Timestamp turn_started_at = 23;

  // What the agent is asking, when it is blocked and legible.
  //
  // Credential-shaped text is stripped before this is set: it is put in front
  // of people on lock screens, and a shell command is where tokens live.
  optional string blocked_question = 24;
```

- [ ] **Step 2: Write the failing test**

Add to `mod tests` in `crates/daemon/src/watch.rs`:

```rust
/// The clock a person actually reads.
///
/// Approving a permission prompt walks Working -> Blocked -> Working. The turn
/// clock must not notice: the task has been running the whole time, and
/// restarting it at every approval is what made "working 12m" read as
/// "working 2s" for a job that had been going for a quarter of an hour.
#[test]
fn the_turn_clock_survives_a_permission_prompt() {
    let mut o = Observed::begin(AgentActivity::Working, 1_000);
    assert_eq!(o.turn_started_at, Some(1_000));

    o = o.advance_to(AgentActivity::Blocked, 5_000);
    assert_eq!(o.turn_started_at, Some(1_000), "the turn did not restart");
    assert_eq!(o.state_since, 5_000, "but the state clock did");

    o = o.advance_to(AgentActivity::Working, 9_000);
    assert_eq!(o.turn_started_at, Some(1_000), "still the same turn");
    assert_eq!(o.state_since, 9_000);
}

#[test]
fn the_turn_clock_clears_when_the_work_ends() {
    let o = Observed::begin(AgentActivity::Working, 1_000).advance_to(AgentActivity::Done, 7_000);
    assert_eq!(o.turn_started_at, None, "a finished turn has no running clock");
    assert_eq!(o.state_since, 7_000);
}

#[test]
fn a_new_turn_starts_a_new_clock() {
    let o = Observed::begin(AgentActivity::Working, 1_000)
        .advance_to(AgentActivity::Done, 7_000)
        .advance_to(AgentActivity::Working, 9_000);
    assert_eq!(o.turn_started_at, Some(9_000));
}

/// One bad sample between two good ones publishes nothing.
///
/// A capture taken mid-redraw can miss a footer that is being rewritten. Before
/// hysteresis that was enough to fire a spurious Done, with the notification
/// that goes with it.
#[test]
fn a_single_odd_sample_does_not_move_the_state() {
    let mut o = Observed::begin(AgentActivity::Working, 1_000);
    let first = o.observe(AgentActivity::Idle, 2_000);
    assert!(first.is_none(), "one sighting is not a state change");
    assert_eq!(o.activity, AgentActivity::Working);

    let second = o.observe(AgentActivity::Working, 3_000);
    assert!(second.is_none(), "and the anomaly is forgotten");
    assert_eq!(o.activity, AgentActivity::Working);
}

#[test]
fn two_agreeing_samples_do_move_it() {
    let mut o = Observed::begin(AgentActivity::Working, 1_000);
    assert!(o.observe(AgentActivity::Idle, 2_000).is_none());
    let moved = o.observe(AgentActivity::Idle, 3_000).expect("two agreeing samples publish");
    assert_eq!(moved, AgentActivity::Done, "Working -> Idle is what Done is made of");
}

/// Blocked is exempt, and must be.
///
/// A question that arrives a second late is fine. A question that never
/// arrives is the failure that makes the whole feature pointless, so a first
/// sighting of Blocked publishes immediately.
#[test]
fn a_question_is_never_made_to_wait() {
    let mut o = Observed::begin(AgentActivity::Working, 1_000);
    let moved = o.observe(AgentActivity::Blocked, 2_000).expect("blocked publishes at once");
    assert_eq!(moved, AgentActivity::Blocked);
}
```

- [ ] **Step 3: Run it to verify it fails**

Run: `~/.cargo/bin/cargo test -p farcooler-daemon watch:: 2>&1 | tail -20`

Expected: FAIL to compile — `Observed::begin`, `advance_to` and `observe` do not exist.

- [ ] **Step 4: Rewrite `Observed`**

Replace the struct at ~line 160 and add the impl:

```rust
#[derive(Debug, Clone)]
struct Observed {
    activity: AgentActivity,
    /// When the CURRENT state began. Reset on every state change.
    ///
    /// This is `changed_at` under its real name. It answers "how long has this
    /// been stuck", which is the question that matters while Blocked.
    state_since: i64,
    /// When the user's request started, or `None` between turns.
    ///
    /// Held across Blocked: approving a tool call does not begin a new turn,
    /// and a clock that restarted on every approval reported a quarter-hour job
    /// as two seconds old.
    turn_started_at: Option<i64>,
    /// The terminal's process state when last announced.
    ///
    /// Watched as well as activity, and that omission was a real bug: a
    /// terminal you ended with Ctrl-D went from `running` to `exited`, which is
    /// not an activity change, so nothing was broadcast. Clients no longer poll,
    /// so the dead terminal simply stayed in the list forever.
    state: TerminalState,
    /// What is running in the pane, so a shell someone typed `claude` into
    /// announces itself the moment it becomes an agent.
    command: String,
    /// Whether this pane can be rendered as a chat.
    chat_capable: bool,
    /// What the agent is asking, while it is asking.
    blocked_question: Option<String>,
    /// A candidate state and how many times running it has been seen.
    ///
    /// Hysteresis. A `capture-pane` taken while the footer is being rewritten
    /// can miss it, and one such sample used to be enough to fire a Done and
    /// the notification behind it.
    pending: Option<(AgentActivity, u8)>,
}

/// Agreeing samples needed before a state change is published.
///
/// Two, which costs one sampling interval of latency on every transition except
/// the one that must never be delayed. See `observe`.
const CONFIRMATIONS: u8 = 2;

impl Observed {
    /// A terminal seen for the first time.
    fn begin(activity: AgentActivity, now: i64) -> Self {
        Observed {
            activity,
            state_since: now,
            turn_started_at: (activity == AgentActivity::Working).then_some(now),
            state: TerminalState::Running,
            command: String::new(),
            chat_capable: false,
            blocked_question: None,
            pending: None,
        }
    }

    /// Move to `next`, keeping whichever clocks should survive it.
    fn advance_to(mut self, next: AgentActivity, now: i64) -> Self {
        use AgentActivity::*;
        self.turn_started_at = match (self.activity, next) {
            // A turn in progress. Blocked is part of it, not the end of it.
            (_, Working | Blocked) => self.turn_started_at.or(Some(now)),
            // Anything else has ended the turn, so there is no clock to run.
            _ => None,
        };
        self.activity = next;
        self.state_since = now;
        self.pending = None;
        self
    }

    /// Fold one sample in, returning the new activity if it should be published.
    ///
    /// `None` means nothing changed, or something changed and has not been seen
    /// enough times to be believed yet.
    fn observe(&mut self, sample: AgentActivity, now: i64) -> Option<AgentActivity> {
        let next = activity::advance(self.activity, sample);
        if next == self.activity {
            self.pending = None;
            return None;
        }

        // Blocked publishes on its first sighting, always.
        //
        // The asymmetry is deliberate and is the whole shape of the trade: a
        // question shown a second late costs nothing, and a question never
        // shown is the failure this feature exists to prevent. Every other
        // transition can afford to be sure.
        if next != AgentActivity::Blocked {
            let seen = match self.pending {
                Some((p, n)) if p == next => n + 1,
                _ => 1,
            };
            if seen < CONFIRMATIONS {
                self.pending = Some((next, seen));
                return None;
            }
        }

        *self = self.clone().advance_to(next, now);
        Some(next)
    }
}
```

- [ ] **Step 5: Run the unit tests**

Run: `~/.cargo/bin/cargo test -p farcooler-daemon watch::tests 2>&1 | tail -25`

Expected: PASS for the six new tests. Compilation of the rest of `watch.rs` will still fail — the sample loop uses the old fields. Fix that next.

- [ ] **Step 6: Rewire the sample loop**

In the `for (id, command, terminal_state, pane_mode, preset)` loop (~line 550-681), replace the block from `let mut state = self.state.lock().await;` down to `self.announce(id, record).await;`:

```rust
            let mut state = self.state.lock().await;
            let now = now_millis();
            let entry = state
                .entry(id)
                .or_insert_with(|| Observed::begin(observed, now));

            let activity_moved = entry.observe(observed, now);
            let changed = activity_moved.is_some()
                || entry.state != terminal_state
                || entry.command != command;
            if !changed {
                continue;
            }

            let previous_activity = entry.activity;
            entry.state = terminal_state;
            entry.command = command.clone();
            entry.chat_capable = chat_capable;
            entry.blocked_question = blocked_question;
            let record = entry.clone();
            drop(state);

            // Worth telling the owner about, and only on the transition.
            //
            // Only when the activity actually moved: a terminal whose command
            // changed has not become newsworthy, and pushing on that would
            // buzz for every `cd`.
            if let Some(next) = activity_moved {
                if previous_activity != next {
                    self.push_if_paired(id, next, &command);
                }
            }

            tracing::info!(
                terminal = %id,
                state = ?terminal_state,
                activity = ?record.activity,
                command = %command,
                "terminal changed"
            );
            self.announce(id, record).await;
```

Immediately above, where `observed` is computed, also compute the question. In the `else` arm that reads the screen (~line 596), change the `Ok` case to capture it:

```rust
                match runtime.screen(id).await {
                    Ok((screen, _, _)) => {
                        let activity = registry.classify(&command, &screen);
                        // Only while blocked, and derived here rather than on a
                        // client for the same reason activity is: a phone has
                        // no screen to read.
                        question = registry.blocked_question(&command, &screen);
                        (
                            registry.describe(&command, &screen),
                            activity,
                            registry
                                .identify(&command, &screen)
                                .is_some_and(|rules| registry.chat_capable(&rules.preset)),
                        )
                    }
                    Err(_) => (
                        registry.describe(&command, ""),
                        AgentActivity::Unspecified,
                        false,
                    ),
                }
```

Declare `let mut question: Option<String> = None;` just before the `let (label, observed, chat_capable) = …` binding, and `let blocked_question = question;` just after it.

- [ ] **Step 7: Update the accessors and the announce path**

`Watcher::activity` (~line 248) gains a third element:

```rust
    /// What the watcher last decided, with both clocks.
    pub async fn activity(&self, terminal: Uuid) -> (AgentActivity, Option<i64>, Option<i64>) {
        match self.state.lock().await.get(&terminal) {
            Some(o) => (o.activity, Some(o.state_since), o.turn_started_at),
            None => (AgentActivity::Unspecified, None, None),
        }
    }

    /// What the agent is asking, if it is.
    pub async fn blocked_question(&self, terminal: Uuid) -> Option<String> {
        self.state.lock().await.get(&terminal).and_then(|o| o.blocked_question.clone())
    }
```

In `mark_seen` (~line 278) replace `observed.changed_at = now_millis();` with `observed.state_since = now_millis();`.

In `announce` (~line 714), after `message.activity = observed.activity as i32;`:

```rust
            message.activity_changed_at = Some(wire::timestamp(observed.state_since));
            message.turn_started_at = observed.turn_started_at.map(wire::timestamp);
            message.blocked_question = observed.blocked_question.clone();
```

In `rpc.rs::with_activity` (~line 1482):

```rust
        let (activity, state_since, turn_started_at) = self.watcher.activity(view.terminal.id).await;
        message.activity = activity as i32;
        message.activity_changed_at = state_since.map(wire::timestamp);
        message.turn_started_at = turn_started_at.map(wire::timestamp);
        message.blocked_question = self.watcher.blocked_question(view.terminal.id).await;
```

In `wire.rs::terminal` (~line 170), beside the existing `activity_changed_at: None,`:

```rust
        turn_started_at: None,
        blocked_question: None,
```

- [ ] **Step 8: Build and run the whole daemon suite**

Run: `~/.cargo/bin/cargo test -p farcooler-daemon 2>&1 | tail -25`

Expected: PASS, at or above the 195 baseline. Do **not** set `FARCOOLER_HOME`.

- [ ] **Step 9: Commit**

```bash
git add proto/farcooler.proto crates/daemon/src/watch.rs crates/daemon/src/wire.rs crates/daemon/src/rpc.rs
git commit -m "feat: a clock for the turn, and one for the state

changed_at recorded when the classifier last changed its mind, which is not
a question anyone asks. Approving a permission prompt walks Working ->
Blocked -> Working, so a job running for a quarter of an hour reported two
seconds. The turn clock is set when an idle agent starts working and held
across Blocked; the state clock still resets, because 'stuck for 12m' is
worth knowing too. Both go on the wire so a phone and a Mac cannot derive
them differently.

Hysteresis with one exception: two agreeing samples to move a state, except
Blocked, which publishes on first sight. A capture taken mid-redraw could
miss a footer and fire a spurious Done with the notification behind it. A
question shown a second late costs nothing; a question never shown is the
failure the feature exists to prevent.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_011A27xRp5hrevabCQpUYbZB"
```

---

### Task 6: Read the pane title

**Files:**
- Create: `crates/core/src/title.rs`
- Modify: `crates/core/src/lib.rs`, `crates/core/src/inventory.rs` (`TaggedPane`), `crates/tmux/src/windows.rs` (`list_tagged_panes` format, `parse_pane_line`)
- Test: `crates/core/src/title.rs`, `crates/tmux/src/windows.rs`

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `pub enum TitleStatus { Working, NotWorking, NoOpinion }`
  - `pub struct PaneTitle { pub status: TitleStatus, pub name: Option<String> }`
  - `pub fn parse(title: &str, program: &str, hostname: &str) -> PaneTitle`
  - `TaggedPane.title: String`

- [ ] **Step 1: Write the failing test**

Create `crates/core/src/title.rs`:

```rust
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

    #[test]
    fn nothing_in_means_nothing_out() {
        let t = parse("", "zsh", "Mac.attlocal.net");
        assert_eq!(t.status, TitleStatus::NoOpinion);
        assert_eq!(t.name, None);
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `~/.cargo/bin/cargo test -p farcooler-core title 2>&1 | tail -20`

Expected: FAIL to compile — module not declared, `parse` and `PaneTitle` not defined.

- [ ] **Step 3: Declare the module**

Add `pub mod title;` to `crates/core/src/lib.rs`.

- [ ] **Step 4: Implement it**

Add above `mod tests` in `crates/core/src/title.rs`:

```rust
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
    let trimmed = title.trim();
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
```

- [ ] **Step 5: Run the tests**

Run: `~/.cargo/bin/cargo test -p farcooler-core title 2>&1 | tail -20`

Expected: PASS, all eight.

- [ ] **Step 6: Carry the title on the pane**

In `crates/core/src/inventory.rs`, after `pub command: String,` in `TaggedPane`:

```rust
    /// The pane's OSC title, as its program set it.
    ///
    /// Empty for most programs. The three coding agents all set one, and it
    /// carries both what they are doing and what they are doing it to — see
    /// `crate::title`.
    pub title: String,
```

Then fix every `TaggedPane { … }` literal the compiler points at by adding `title: String::new(),` — including the fixtures in `crates/daemon/src/test_support.rs` if present.

- [ ] **Step 7: Ask tmux for it**

In `crates/tmux/src/windows.rs::list_tagged_panes`, append `\t#{{pane_title}}` to the end of the format string (after `#{{pane_tty}}`). Appending keeps every existing field index unchanged, which matters: a title containing a tab would then add fields rather than shift them.

In `parse_pane_line`, after the `tty` line:

```rust
        // Appended last on purpose. A title is user-controlled text and may
        // contain a tab; putting it at the end means such a title costs its own
        // value and not every field after it.
        title: f.get(17).map(|v| v.trim().to_string()).unwrap_or_default(),
```

- [ ] **Step 8: Test the parse**

Add to `mod tests` in `crates/tmux/src/windows.rs`:

```rust
#[test]
fn a_pane_line_carries_the_title() {
    let d = uuid::Uuid::nil();
    let line = format!(
        "%1\t@0\t80\t24\t{d}\t{d}\t{d}\t1\t\t\tclaude\t0\t0\t1\t1\t0\t/dev/ttys001\t◐ Write a haiku"
    );
    let p = parse_pane_line(&line).expect("a well-formed line parses");
    assert_eq!(p.title, "◐ Write a haiku");
    assert_eq!(p.tty, "/dev/ttys001");
}

/// A build that predates the title field must still parse.
#[test]
fn a_line_without_a_title_still_parses() {
    let d = uuid::Uuid::nil();
    let line = format!(
        "%1\t@0\t80\t24\t{d}\t{d}\t{d}\t1\t\t\tclaude\t0\t0\t1\t1\t0\t/dev/ttys001"
    );
    let p = parse_pane_line(&line).expect("a short line parses");
    assert_eq!(p.title, "");
}
```

- [ ] **Step 9: Run the tmux and core suites**

Run: `~/.cargo/bin/cargo test -p farcooler-tmux -p farcooler-core 2>&1 | tail -20`

Expected: PASS.

- [ ] **Step 10: Commit**

```bash
git add crates/core/src/title.rs crates/core/src/lib.rs crates/core/src/inventory.rs crates/tmux/src/windows.rs
git commit -m "feat: read the title all three agents were already setting

claude writes a spinner and an AI-written summary of the task, codex a
spinner or '[ ! ] Action Required' and the directory, cursor a summary and
no status. The daemon's list-panes format never asked for pane_title, so
none of it was read.

Confirmed to be the agents' own behavior and not an installed hook: the
claude binary carries useTerminalTitle, codex carries terminal_title as a
config key, and codex advances ten spinner frames in five seconds. Nothing
fires a hook at three hertz.

That codex exposes it as a config key means it can be turned off, so every
answer here may be 'no opinion' and the screen takes over. A title is
rejected as a name when it is the hostname, a path, the program's own name,
a bare harness name, or a single word -- which is how a shell's cwd and
codex's directory stay out of the sidebar.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_011A27xRp5hrevabCQpUYbZB"
```

---

### Task 7: Name a non-agent pane properly

**Files:**
- Modify: `crates/daemon/src/foreground.rs` (`summarize` at ~line 57)
- Create: `crates/core/src/ports.rs`
- Modify: `crates/core/src/lib.rs`
- Test: `crates/daemon/src/foreground.rs`, `crates/core/src/ports.rs`

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `summarize` keeps its signature: `fn summarize(args: &str) -> String`.
  - `pub fn listening_ports() -> std::collections::HashMap<i32, Vec<u16>>` — pid to ports.
  - `pub fn purpose(ports: &[u16]) -> Option<String>` — `web :8099`.

- [ ] **Step 1: Write the failing test**

Replace the existing `mod tests` in `crates/daemon/src/foreground.rs` with:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_subcommand_survives_because_it_is_the_distinguishing_part() {
        assert_eq!(summarize("pnpm dev"), "pnpm dev");
        assert_eq!(summarize("cargo build --release"), "cargo build");
        assert_eq!(summarize("/opt/homebrew/bin/rg pattern"), "rg pattern");
    }

    /// The rule this replaces threw away the only informative part.
    ///
    /// `python3 -m http.server 8099` labelled as `Python`, because the first
    /// argument was a flag and everything after it was dropped. That is exactly
    /// backwards for every modern runner.
    #[test]
    fn a_flag_does_not_hide_the_thing_being_run() {
        assert_eq!(summarize("python3 -m http.server 8099"), "python http.server");
        assert_eq!(summarize("node --inspect server.js"), "node server.js");
        assert_eq!(summarize("npm --silent run dev"), "npm run dev");
        assert_eq!(summarize("cargo run -p api"), "cargo run api");
    }

    /// A flag with no operand behind it still says nothing.
    #[test]
    fn a_flag_that_leads_nowhere_leaves_the_program_alone() {
        assert_eq!(summarize("tail -f"), "tail");
        assert_eq!(summarize("top -l 0"), "top");
        assert_eq!(summarize("node"), "node");
    }

    #[test]
    fn a_long_argument_is_dropped_rather_than_truncated() {
        // Half a path is worse than none: it looks like a name and is not one.
        assert_eq!(summarize("vim src/some/deeply/nested/module.rs"), "vim module.rs");
        assert_eq!(summarize("python a_very_long_script_name_indeed_here.py"), "python");
    }

    /// The interpreter's real path is not the point.
    #[test]
    fn an_interpreter_reads_as_itself() {
        let real = "/opt/homebrew/Cellar/python@3.14/3.14.6/Frameworks/Python.framework/Versions/3.14/Resources/Python.app/Contents/MacOS/Python -m http.server 8099";
        assert_eq!(summarize(real), "python http.server");
    }

    #[test]
    fn nothing_in_means_nothing_out() {
        assert_eq!(summarize(""), "");
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `~/.cargo/bin/cargo test -p farcooler-daemon foreground 2>&1 | tail -20`

Expected: FAIL — `python3 -m http.server 8099` yields `Python`, `npm --silent run dev` yields `npm`.

- [ ] **Step 3: Rewrite `summarize`**

```rust
/// A command line short enough to be a label.
///
/// The program plus the first argument that says something. The rule this
/// replaces kept an argument only when it was not a flag, on the reasoning that
/// a subcommand is the distinguishing part — which is true, and is exactly why
/// dropping everything after a flag was wrong. `python3 -m http.server 8099`
/// labelled as `Python`, `npm --silent run dev` as `npm`, and the informative
/// half of every modern runner invocation went in the bin.
///
/// So flags are skipped rather than treated as terminal, and a flag that takes
/// a value has its value skipped with it — `-p api` contributes `api`, not `-p`.
fn summarize(args: &str) -> String {
    let mut parts = args.split_whitespace();
    let Some(program) = parts.next() else { return String::new() };
    let program = program.rsplit('/').next().unwrap_or(program);
    // `python3`, and the framework build that calls itself `Python`, are both
    // just python to a person reading a row.
    let program = normalize(program);

    let mut chosen: Option<String> = None;
    while let Some(arg) = parts.next() {
        if arg.starts_with('-') {
            // A short flag that takes a value swallows the next token, or
            // `cargo run -p api` would read as `cargo run -p`.
            if takes_a_value(arg) {
                parts.next();
            }
            continue;
        }
        chosen = Some(arg.rsplit('/').next().unwrap_or(arg).to_string());
        break;
    }

    // `cargo run -p api` wants both words, so a runner keeps looking past its
    // subcommand for the thing being run.
    let mut label = match chosen {
        Some(arg) => format!("{program} {arg}"),
        None => return program.to_string(),
    };
    if RUNNERS.contains(&program) {
        if let Some(next) = parts.find(|a| !a.starts_with('-')) {
            let next = next.rsplit('/').next().unwrap_or(next);
            let wider = format!("{label} {next}");
            if wider.chars().count() <= 24 {
                label = wider;
            }
        }
    }

    if label.chars().count() <= 24 { label } else { program.to_string() }
}

/// Programs whose first argument is a verb, so the word after it is the noun.
const RUNNERS: &[&str] = &["cargo", "npm", "pnpm", "yarn", "bun", "deno", "go", "uv", "poetry"];

/// Whether a flag consumes the token after it.
///
/// `-m` is deliberately absent. `python -m http.server` is the exact case this
/// rewrite exists for, and treating `-m` as swallowing its operand would drop
/// the only informative word in the line — the same failure under a new rule.
/// `top -l 0` is why `-l` is present: without it the label reads `top 0`.
///
/// Short forms only. `--flag=value` carries its own value and needs none of
/// this, and a long flag taking a separate value is rare enough that guessing
/// wrong costs one word.
fn takes_a_value(flag: &str) -> bool {
    matches!(flag, "-l" | "-p" | "-c" | "-o" | "-f" | "-e" | "-u" | "-t")
}

/// What a person calls this program.
///
/// `python3` and the framework build that reports itself as `Python` are both
/// just python in a row someone is scanning.
fn normalize(program: &str) -> &str {
    let stem = program.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    if stem.eq_ignore_ascii_case("python") {
        return "python";
    }
    if stem.eq_ignore_ascii_case("node") {
        return "node";
    }
    program
}
```

- [ ] **Step 4: Run the tests**

Run: `~/.cargo/bin/cargo test -p farcooler-daemon foreground 2>&1 | tail -20`

Expected: PASS, all six.

- [ ] **Step 5: Write the ports test**

Create `crates/core/src/ports.rs`:

```rust
//! What a pane is serving, read from the kernel rather than from its output.
//!
//! A pane running `python -m http.server 8099` either holds a listening socket
//! on 8099 or it does not; that is a fact about the machine, available without
//! interpreting a single character of what the program printed. Pattern
//! matching prose is the mistake the rest of this work exists to correct, and
//! it would be perverse to reintroduce it here.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_served_port_reads_as_a_purpose() {
        assert_eq!(purpose(&[8099]), Some("web :8099".to_string()));
    }

    /// Several ports is a fact about the process, not a label. The lowest is
    /// almost always the one a person typed.
    #[test]
    fn many_ports_report_the_lowest() {
        assert_eq!(purpose(&[9229, 5173]), Some("web :5173".to_string()));
    }

    #[test]
    fn nothing_listening_is_no_purpose() {
        assert_eq!(purpose(&[]), None);
    }
}
```

- [ ] **Step 6: Run it to verify it fails**

Run: `~/.cargo/bin/cargo test -p farcooler-core ports 2>&1 | tail -20`

Expected: FAIL to compile.

- [ ] **Step 7: Implement it**

Add `pub mod ports;` to `crates/core/src/lib.rs`, and above `mod tests`:

```rust
use std::collections::HashMap;

/// What a pane serving these ports is for.
///
/// The lowest, because a dev server that also opens a debugger port (node's
/// 9229, for one) should read as the server a person started, not the debugger
/// they did not.
pub fn purpose(ports: &[u16]) -> Option<String> {
    ports.iter().min().map(|p| format!("web :{p}"))
}

/// Every listening TCP port on this machine, by owning process.
///
/// One call for the whole host, on the sampling loop's cadence, for the same
/// reason `ps` is: a fleet of thirty panes must not mean thirty processes a
/// second.
///
/// Failure is silently empty. A machine without `lsof`, or one where it is
/// refused, loses a decoration — it must not lose the row.
pub fn listening_ports() -> HashMap<i32, Vec<u16>> {
    let out = std::process::Command::new("lsof")
        .args(["-nP", "-iTCP", "-sTCP:LISTEN", "-Fpn"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output();
    let Ok(out) = out else { return HashMap::new() };

    // `-F` is a field-per-line format: `p<pid>` opens a process block, and each
    // `n<name>` under it is one of its sockets.
    let mut found: HashMap<i32, Vec<u16>> = HashMap::new();
    let mut pid: Option<i32> = None;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let (tag, value) = line.split_at(1.min(line.len()));
        match tag {
            "p" => pid = value.trim().parse().ok(),
            "n" => {
                let Some(pid) = pid else { continue };
                // `*:8099`, `127.0.0.1:8099`, `[::1]:8099`.
                let Some(port) = value.rsplit(':').next().and_then(|p| p.trim().parse::<u16>().ok())
                else {
                    continue;
                };
                let ports = found.entry(pid).or_default();
                if !ports.contains(&port) {
                    ports.push(port);
                }
            }
            _ => {}
        }
    }
    found
}
```

- [ ] **Step 8: Run the tests**

Run: `~/.cargo/bin/cargo test -p farcooler-core ports 2>&1 | tail -20`

Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add crates/daemon/src/foreground.rs crates/core/src/ports.rs crates/core/src/lib.rs
git commit -m "fix: a pane running a server said 'Python'

summarize kept the first argument only when it was not a flag, so
'python3 -m http.server 8099' labelled as 'Python' and 'npm --silent run
dev' as 'npm'. The reasoning was that a subcommand is the distinguishing
part, which is true and is exactly why dropping everything after a flag was
wrong.

Flags are now skipped rather than treated as terminal, and a runner keeps
looking past its verb for the noun, so 'cargo run -p api' reads as 'cargo
run api'.

Ports come from the kernel, not from the pane's output: a process either
holds a listening socket or it does not, so 'web :8099' is derived rather
than pattern-matched out of scrollback. One lsof for the whole machine on
the sampling cadence, and a machine without lsof loses a decoration rather
than a row.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_011A27xRp5hrevabCQpUYbZB"
```

---

### Task 8: Put the name together

Tasks 6 and 7 produce a title parser and a port reader that nothing yet calls. This is where they become a pane's name, in the precedence the spec sets.

**Files:**
- Modify: `crates/core/src/activity.rs` (`Registry::describe` at ~line 435)
- Modify: `crates/daemon/src/watch.rs` (the label branch of the sample loop, ~line 555-617)
- Test: `crates/core/src/activity.rs`

**Interfaces:**
- Consumes: `title::parse`, `TitleStatus`, `PaneTitle` (Task 6); `ports::purpose`, `ports::listening_ports` (Task 7); `Registry::describe` (existing).
- Produces: `pub fn describe_pane(&self, command: &str, screen: &str, title: &str, purpose: Option<&str>, hostname: &str) -> String`.

- [ ] **Step 1: Write the failing test**

```rust
/// The precedence, in one place.
///
/// An agent's own summary of its work beats anything Far Cooler can derive,
/// and a bare command beats nothing at all.
#[test]
fn a_pane_is_named_by_the_best_source_that_has_an_answer() {
    let r = Registry::built_in();
    let working = include_str!("../captures/claude-working.txt");

    // The agent's own summary wins.
    assert_eq!(
        r.describe_pane("claude", working, "◐ Write tmux haiku and explain escape key behavior", None, "Mac"),
        "Write tmux haiku and explain escape key behavior"
    );

    // No usable title: fall back to what the agent is.
    assert_eq!(r.describe_pane("claude", working, "✳ Claude Code", None, "Mac"), "claude");

    // Not an agent: the command, and what it serves.
    assert_eq!(
        r.describe_pane("python http.server", "", "Mac", Some("web :8099"), "Mac"),
        "python http.server · web :8099"
    );
    // A command with nothing to serve is just the command.
    assert_eq!(r.describe_pane("vim module.rs", "", "Mac", None, "Mac"), "vim module.rs");
}

/// Codex's title is the directory, which the workspace row already says.
#[test]
fn codex_is_not_named_after_its_directory() {
    let r = Registry::built_in();
    let working = include_str!("../captures/codex-working.txt");
    assert_eq!(r.describe_pane("codex", working, "⠋ bare", None, "Mac"), "codex");
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `~/.cargo/bin/cargo test -p farcooler-core activity::tests::a_pane_is_named 2>&1 | tail -20`

Expected: FAIL to compile — `describe_pane` does not exist.

- [ ] **Step 3: Implement it**

Add to the `impl Registry` block holding `describe`:

```rust
    /// What to call this pane, from the best source that has an answer.
    ///
    /// `describe` answers "what is running here", which is a different and
    /// smaller question — it is still used wherever that is what is wanted.
    /// This one answers "what is this pane FOR", and the sources disagree about
    /// how well they can:
    ///
    /// 1. the agent's own summary of the work, from its OSC title
    /// 2. what the agent is, when its title says nothing useful
    /// 3. the command, and what it is serving
    ///
    /// A name the USER typed outranks all of these and never reaches here —
    /// `Service::remember_agent_title` refuses to overwrite one.
    pub fn describe_pane(
        &self,
        command: &str,
        screen: &str,
        title: &str,
        purpose: Option<&str>,
        hostname: &str,
    ) -> String {
        let parsed = crate::title::parse(title, command, hostname);
        if let Some(name) = parsed.name {
            return name;
        }
        let described = self.describe(command, screen);
        match purpose {
            // Only for a pane that is not an agent. An agent serving a port is
            // serving it incidentally, and the row is about the work.
            Some(p) if self.identify(command, screen).is_none() => format!("{described} · {p}"),
            _ => described,
        }
    }
```

- [ ] **Step 4: Run it**

Run: `~/.cargo/bin/cargo test -p farcooler-core activity:: 2>&1 | tail -20`

Expected: PASS.

- [ ] **Step 5: Feed it from the watcher**

In `crates/daemon/src/watch.rs`, read the ports once per tick beside the `ps` call (~line 464):

```rust
        let foreground = crate::foreground::read().await;
        // One `lsof` for the whole machine, on the same cadence and for the
        // same reason as the one `ps`.
        let ports = farcooler_core::ports::listening_ports();
        let hostname = crate::hostname();
```

If no `hostname()` helper exists in the daemon, add one to `crates/daemon/src/lib.rs`:

```rust
/// This machine's name, for rejecting it as a pane title.
///
/// A plain shell leaves the hostname in the title, and a row called
/// `Mac.attlocal.net` says nothing about the pane.
pub fn hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}
```

In the `live.push(...)` loop (~line 511), carry the pane's title and its ports alongside the command:

```rust
                let pane = panes.panes.iter().find(|p| p.terminal_id == id);
                let command = pane
                    .and_then(|p| foreground.get(p.tty.trim_start_matches("/dev/")).cloned())
                    .or_else(|| pane.map(|p| p.command.clone()))
                    .unwrap_or_default();
                let title = pane.map(|p| p.title.clone()).unwrap_or_default();
                // A pane's ports are its foreground process's, found through the
                // tty they share. `ps` already gave us that pid on this tick, so
                // this is a map lookup rather than a second walk of the process
                // table.
                let purpose = pane
                    .and_then(|p| foreground.get(p.tty.trim_start_matches("/dev/")))
                    .and_then(|(pid, _)| ports.get(pid))
                    .and_then(|open| farcooler_core::ports::purpose(open));
                live.push((id, command, title, purpose, terminal.state(), terminal.terminal.pane_mode, terminal.terminal.command_preset.clone()));
```

That lookup needs the pid, which `foreground::read` already sees and currently discards. Widen its type in `crates/daemon/src/foreground.rs`:

```rust
/// Foreground command lines and their pids, keyed by tty name (`ttys162`).
///
/// The pid rides along because the same `ps` walk already has it, and the ports
/// lookup needs a process to ask about. Finding it again would mean a second
/// walk of the whole process table on every tick.
pub type Foreground = HashMap<String, (i32, String)>;
```

In `read`, parse `pid` from a widened `ps` invocation — `["-axo", "pid=,tty=,stat=,args="]` — and store `(pid, summarize(args))`. The existing `command` lookup in `watch.rs` becomes `foreground.get(…).map(|(_, cmd)| cmd.clone())`. `summarize` itself is unchanged.

Then in the classification branch (~line 596), replace every `registry.describe(&command, …)` with `registry.describe_pane(&command, …, &title, purpose.as_deref(), &hostname)`, and use `parsed.status` as a working hint where the screen said nothing:

```rust
                match runtime.screen(id).await {
                    Ok((screen, _, _)) => {
                        let mut activity = registry.classify(&command, &screen);
                        // The title is the agent's own account of itself, so it
                        // outranks the screen where the screen had no opinion.
                        // Not where it did: a footer that says `esc to
                        // interrupt` is the agent drawing its status, and a
                        // title glyph that disagrees is a stale frame.
                        if activity == AgentActivity::Idle {
                            if let farcooler_core::title::TitleStatus::Working =
                                farcooler_core::title::parse(&title, &command, &hostname).status
                            {
                                activity = AgentActivity::Working;
                            }
                        }
                        question = registry.blocked_question(&command, &screen);
                        (
                            registry.describe_pane(&command, &screen, &title, purpose.as_deref(), &hostname),
                            activity,
                            registry
                                .identify(&command, &screen)
                                .is_some_and(|rules| registry.chat_capable(&rules.preset)),
                        )
                    }
                    Err(_) => (
                        registry.describe_pane(&command, "", &title, purpose.as_deref(), &hostname),
                        AgentActivity::Unspecified,
                        false,
                    ),
                }
```

- [ ] **Step 6: Build and test**

Run: `~/.cargo/bin/cargo test -p farcooler-core -p farcooler-daemon 2>&1 | tail -25`

Expected: PASS at or above baseline.

- [ ] **Step 7: Commit**

```bash
git add crates/core/src/activity.rs crates/daemon/src/watch.rs crates/daemon/src/foreground.rs crates/daemon/src/lib.rs
git commit -m "feat: name a pane after the work, not after the binary

describe answered 'what is running here', so every agent pane was called
'claude' and every shell 'shell'. describe_pane answers 'what is this pane
for' and takes the best source that has an answer: the agent's own summary
from its OSC title, then what the agent is, then the command and what it
serves.

A name the user typed still outranks all of it and never reaches here.
Codex's title is only its directory, which the workspace row already says,
so it keeps its harness name until stage 2 can read its session log.

The title also resolves a screen that had no opinion: where classification
came out Idle and the title glyph says the agent is spinning, the title
wins. Not the other way round -- a footer drawing 'esc to interrupt' is the
agent stating its status, and a title frame that disagrees is stale.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_011A27xRp5hrevabCQpUYbZB"
```

---

### Task 9: A failed command wants attention

**Files:**
- Modify: `crates/core/src/activity.rs` (`wants_attention` at ~line 643)
- Modify: `apps/macos/Sources/FarCooler/Model.swift` (`Status` at ~line 375, `status` at ~line 327, `statusDuration` at ~line 358)
- Test: `crates/core/src/activity.rs`

**Interfaces:**
- Consumes: `AgentActivity`.
- Produces: `pub fn exit_wants_attention(exit_code: Option<i32>, exit_signal: Option<i32>) -> bool`.

- [ ] **Step 1: Write the failing test**

```rust
/// A build that failed while you were asleep should say so.
///
/// `wants_attention` covered agents only, so a `cargo build` that exited 101 at
/// 3am was silent and you found out in the morning. For a design whose whole
/// point is overseeing work away from the machine, that is a hole in the one
/// direction that costs real time.
#[test]
fn a_command_that_failed_wants_you() {
    assert!(exit_wants_attention(Some(101), None));
    assert!(exit_wants_attention(Some(1), None));
    // Killed by a signal is a failure too.
    assert!(exit_wants_attention(None, Some(9)));
}

/// A clean exit is not news.
///
/// A shell you closed, or a dev server you stopped on purpose, must stay quiet
/// — something that buzzes for the ordinary case gets turned off, and then it
/// cannot tell you the thing that mattered.
#[test]
fn a_command_that_succeeded_says_nothing() {
    assert!(!exit_wants_attention(Some(0), None));
    assert!(!exit_wants_attention(None, None));
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `~/.cargo/bin/cargo test -p farcooler-core activity::tests::a_command 2>&1 | tail -20`

Expected: FAIL to compile — `exit_wants_attention` is not defined.

- [ ] **Step 3: Implement it**

Beside `wants_attention` in `crates/core/src/activity.rs`:

```rust
/// Whether how a command ENDED is worth interrupting someone for.
///
/// The companion to `wants_attention`, which asks the same question about an
/// agent. A fleet is not only agents: a `cargo build` that exits 101 overnight
/// is exactly as worth knowing about as an agent waiting on a question, and it
/// was silent because nothing asked.
///
/// Deliberately not a new `TerminalState`. The exit code is already stored and
/// already on the wire, so this is a reading of what is there rather than
/// another state for every client to learn.
pub fn exit_wants_attention(exit_code: Option<i32>, exit_signal: Option<i32>) -> bool {
    exit_signal.is_some() || matches!(exit_code, Some(code) if code != 0)
}
```

- [ ] **Step 4: Run it**

Run: `~/.cargo/bin/cargo test -p farcooler-core activity:: 2>&1 | tail -20`

Expected: PASS.

- [ ] **Step 5: Send the exit status to the client at all**

The Mac cannot distinguish a failure today because nothing tells it: the JSON
bridge never emits the exit code, and the Swift model has no field for it. The
wire has carried `exit_status` since long before this change; only the last hop
is missing.

In `crates/client/src/session.rs`, in the terminal `json!` (~line 291), beside
`"activitySince"`:

```rust
                            // How it ENDED, which is the difference between a
                            // shell you closed and a build that broke.
                            "exitCode": t.exit_status.as_ref().and_then(|e| e.code),
                            "exitSignal": t.exit_status.as_ref().and_then(|e| e.signal),
```

In `apps/macos/Sources/FarCooler/Model.swift`, beside `var activitySince: Double?`:

```swift
    /// How the command ended. Absent while it is still running, and absent from
    /// older daemons — which is why a missing value is never read as a failure.
    var exitCode: Int?
    var exitSignal: Int?
```

- [ ] **Step 6: Show it on the Mac**

In `apps/macos/Sources/FarCooler/Model.swift`, add a case to `Status` after `case exited`:

```swift
    /// The command ended badly.
    ///
    /// Distinct from `exited`, which is a shell you closed or a server you
    /// stopped and is not news. A build that failed overnight is the reason to
    /// look at the fleet at all.
    case failedRun
```

Give it a label in the `label` switch:

```swift
        case .failedRun: return "Failed"
```

In the `status` computed property, replace `case .exited: return .exited` with:

```swift
        case .exited:
            // A non-zero code or a signal is a failure worth seeing; a clean
            // exit is not. An ABSENT code is not a failure either — an older
            // daemon sends none, and reading that as broken would mark every
            // finished terminal on the machine.
            if exitSignal != nil || (exitCode.map { $0 != 0 } ?? false) {
                return .failedRun
            }
            return .exited
```

Then add it to `Status.wantsAttention` (`Model.swift:410`), which today reads:

```swift
    var wantsAttention: Bool {
        self == .blocked || self == .done || self == .lost || self == .failed
    }
```

It becomes:

```swift
    var wantsAttention: Bool {
        self == .blocked || self == .done || self == .lost || self == .failed
            || self == .failedRun
    }
```

`.failed` and `.failedRun` are different findings and both belong here: the
first is a terminal that never started, the second a command that started, ran
and came back non-zero.

- [ ] **Step 7: Render both clocks**

Replace `statusDuration` (~line 358):

```swift
    /// How long the current status has held, if that is worth knowing.
    ///
    /// Only for the two where duration changes what you do: an agent blocked
    /// for twenty minutes is a different situation from one blocked for ten
    /// seconds. "Idle for three days" is noise.
    var statusDuration: String? {
        guard status == .blocked || status == .working, let since = activitySince else {
            return nil
        }
        return Self.brief(secondsSince: since)
    }

    /// How long the whole turn has been running.
    ///
    /// Distinct from `statusDuration`, and the one a person means by "how long
    /// has this been going". It does not restart when a permission prompt is
    /// approved, because saying yes to a tool call does not begin a new turn.
    var turnDuration: String? {
        guard let since = turnStartedAt else { return nil }
        return Self.brief(secondsSince: since)
    }

    private static func brief(secondsSince millis: Double) -> String? {
        let seconds = Date().timeIntervalSince1970 - millis / 1000
        guard seconds >= 5 else { return nil }
        if seconds < 60 { return "\(Int(seconds))s" }
        if seconds < 3600 { return "\(Int(seconds / 60))m" }
        return "\(Int(seconds / 3600))h"
    }
```

Add the backing properties beside `activitySince` (~line 193):

```swift
    /// Unix milliseconds when the current TURN started, from the daemon.
    var turnStartedAt: Double?
    /// What the agent is asking, when it is blocked.
    var blockedQuestion: String?
```

- [ ] **Step 8: Decode the turn clock and the question**

In `crates/client/src/session.rs` (~line 291), beside `"activitySince": activity_since(t),`:

```rust
                                    "turnStartedAt": turn_started_at(t),
                                    "blockedQuestion": t.blocked_question.clone(),
```

And beside `activity_since` (~line 1300):

```rust
fn turn_started_at(t: &farcooler_protocol::v1::Terminal) -> Option<i64> {
    t.turn_started_at.as_ref().map(|ts| ts.seconds * 1000 + (ts.nanos as i64) / 1_000_000)
}
```

- [ ] **Step 9: Build everything**

Run: `~/.cargo/bin/cargo build --workspace 2>&1 | tail -20`

Expected: no errors.

Run: `~/.cargo/bin/cargo test --workspace 2>&1 | tail -30`

Expected: PASS, at or above the 145 + 195 baseline. Do not set `FARCOOLER_HOME`.

- [ ] **Step 10: Commit**

```bash
git add crates/core/src/activity.rs crates/client/src/session.rs apps/macos/Sources/FarCooler/Model.swift
git commit -m "feat: a build that failed overnight now says so

wants_attention covered agents only, so a cargo build that exited 101 at 3am
was silent and you found out in the morning. For a design whose whole point
is overseeing work away from the machine, that was a hole in the one
direction that costs real time.

Read from the exit code rather than added as a state: the code is already
stored and already on the wire, so this is a reading of what is there and
not another state for every client to learn. A clean exit stays quiet -- a
shell you closed is not news, and something that buzzes for the ordinary
case gets turned off.

The Mac now shows both clocks: how long the turn has run, and how long the
current state has held.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_011A27xRp5hrevabCQpUYbZB"
```

---

## Verification

- [ ] **Full suite green**

Run: `~/.cargo/bin/cargo test --workspace 2>&1 | grep -E "^test result"`

Expected: every line `ok`, totals at or above 145 (core) and 195 (daemon).

- [ ] **The original bug, end to end**

Run: `~/.cargo/bin/cargo test -p farcooler-core a_finished_agent_is_not_working -- --nocapture`

Expected: PASS. This is the thirty-three hour bug.

- [ ] **Against a live agent**

Start a scratch tmux on its own socket, run `claude`, give it a task, and watch the row: it must reach `Done` when the turn ends rather than staying `Working`. Kill the server by its exact socket name — never by pattern, since a live Far Cooler app is running.

```bash
tmux -L fcverify new-session -d -s v -x 120 -y 40 -c /tmp 'claude'
# … drive it, watch the daemon log …
tmux -L fcverify kill-server
```

## Notes for stage 2

Stage 2 reads the agents' session logs and is a separate plan. Two facts from this stage carry into it:

- `crates/core/src/title.rs` rejects a bare single word as a name, which is what keeps codex's cwd out of the sidebar. Stage 2 fills codex's name from the session log's first `user_message` instead.
- `redact` is already on the question path. The feed must use it too, and the push path must be tested separately rather than trusted to share the event path.
