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

use farcooler_protocol::v1::AgentActivity;

/// How to launch an agent's ACP adapter.
///
/// Three real shapes have to fit: an npm package run through `npx`, a native
/// subcommand on an installed binary, and a script with flags. The previous
/// representation — a bare program with no arguments — could express none of
/// them, so a user-supplied adapter silently lost everything after the program
/// name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AdapterSpec {
    pub program: String,
    pub args: Vec<String>,
    /// Set in the adapter's environment before it starts. For secrets and
    /// endpoints an agent needs and Far Cooler has no opinion about.
    ///
    /// A map, not a list of pairs: TOML writes this as `env = { KEY = "v" }`,
    /// which is a table and will not deserialize into tuples. `BTreeMap` rather
    /// than `HashMap` so the order an adapter's environment is built in is
    /// stable, and a test can assert on it.
    pub env: std::collections::BTreeMap<String, String>,
}

/// One agent's screen signatures.
///
/// Order matters within a screen: `blocked` is checked before `working`,
/// because an agent asking permission usually still shows its working
/// furniture. Getting that backwards means never noticing that something needs
/// you, which is the one failure that makes the whole feature pointless.
#[derive(Debug, Clone)]
pub struct AgentRules {
    /// What to call this agent.
    pub preset: String,

    /// Process-name PREFIXES, as `pane_current_command` reports them.
    ///
    /// Prefixes, not exact names, because tmux truncates the field: codex
    /// arrives as `codex-aarch64-a`. And necessary but never sufficient —
    /// Claude Code renames itself to its version (`2.1.220`) and cursor-agent
    /// runs as plain `node`, which is far too generic to claim. Screen identity
    /// is what actually carries this.
    pub commands: Vec<String>,

    /// Screen text that means "this IS this agent".
    ///
    /// Furniture the agent always draws, never a phrase a user could type, so a
    /// shell echoing "do you want to proceed?" is not promoted to an agent.
    pub identity: Vec<String>,

    /// Waiting on the user.
    ///
    /// The list that has to be right. A missed blocked state is a notification
    /// that never arrives, which is the one failure that makes the whole
    /// feature pointless — so these are deliberately generous.
    pub blocked: Vec<String>,

    /// Actively doing something.
    pub working: Vec<String>,

    /// How to host this agent as a native chat, when Far Cooler can.
    ///
    /// `None` means recognized-but-terminal-only, which is a real and honest
    /// state rather than a gap: an agent with no adapter renders as the TUI it
    /// is. This field replaces the separate `CHAT_CAPABLE` list in the daemon,
    /// which was maintained by hand beside these rules and disagreed with them.
    pub adapter: Option<AdapterSpec>,
}

/// A short-hand so the built-in table reads as it did when it was a `const`.
fn s(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn npx(package: &str) -> Option<AdapterSpec> {
    Some(AdapterSpec {
        program: "npx".to_string(),
        args: vec!["-y".to_string(), package.to_string()],
        env: Default::default(),
    })
}

/// Every agent Far Cooler knows, and how to host the ones it can.
///
/// One table rather than two. Recognition and hostability were separate lists
/// maintained by hand, and they drifted: codex was recognized in a pane and
/// absent from the chat list, so `⌃B a` on a codex pane did nothing and said
/// nothing. Hostability is now a field on the entry, so there is no second list
/// to disagree with.
#[derive(Debug, Clone)]
pub struct Registry {
    rules: Vec<AgentRules>,
}

impl Registry {
    /// The built-in rules.
    ///
    /// Every signature below was read off a running agent rather than guessed.
    /// The first version of this file was guesswork and it matched no real
    /// screen.
    ///
    /// There is deliberately no `idle` list. An agent that is identified, not
    /// blocked and not working IS idle, and requiring positive idle furniture
    /// meant a version bump renaming a footer left an agent stuck on `unknown`
    /// — never reaching `done`, never notifying.
    pub fn built_in() -> Self {
        Registry {
            rules: vec![
                AgentRules {
                    preset: "claude".to_string(),
                    commands: s(&["claude"]),
                    identity: s(&[
                        "? for shortcuts",
                        "Claude Code",
                        "auto-accept edits",
                        "esc to interrupt",
                    ]),
                    blocked: s(&[
                        "Do you want to",
                        "Do you want me to",
                        "❯ 1. Yes",
                        "1. Yes, and don't ask again",
                        // The footer under every approval prompt.
                        "Esc to cancel · Tab to amend",
                        "[y/n]",
                        "(y/N)",
                    ]),
                    working: s(&["esc to interrupt", "Thinking…"]),
                    adapter: npx("@agentclientprotocol/claude-agent-acp"),
                },
                AgentRules {
                    preset: "codex".to_string(),
                    // Truncated by tmux to `codex-aarch64-a`, hence the prefix.
                    commands: s(&["codex"]),
                    identity: s(&["OpenAI Codex", "/model to change"]),
                    blocked: s(&[
                        // Codex draws every choice as a numbered list under a `›` marker.
                        "\u{203a} 1.",
                        "Press enter to continue",
                        "Allow command",
                        "Do you want to",
                        "[y/n]",
                        "(y/N)",
                    ]),
                    working: s(&["esc to interrupt", "Working ("]),
                    // NOT `@zed-industries/codex-acp`: npm reports it deprecated
                    // and replaced by this one, and it stalled at 0.16.0 against
                    // this package's 1.1.9. The same rename that caught the
                    // Claude adapter.
                    adapter: npx("@agentclientprotocol/codex-acp"),
                },
                AgentRules {
                    preset: "opencode".to_string(),
                    // Confirmed by `tmux display-message -p '#{pane_current_command}'`
                    // against a running opencode: it does not rename itself, unlike
                    // claude and codex. This is the reliable path; see the identity
                    // comment below for what breaks if that ever stops being true.
                    commands: s(&["opencode"]),
                    // Two markers, for two different failure modes of the other one.
                    //
                    // "ctrl+p commands" is the command-palette hint bar: present on
                    // the first blank prompt and after every completed turn
                    // (captures/opencode-idle.txt, captures/opencode-idle2.txt,
                    // captures/opencode-working2.txt,
                    // captures/opencode-fresh-dir-risky.txt). But it shares the
                    // bottom line with variable-length status text, and long status
                    // crowds it off entirely: it is ABSENT from
                    // captures/opencode-blocked.txt, captures/opencode-after-retries.txt
                    // and captures/opencode-working.txt, where a long provider error
                    // pushed it past the terminal's right edge. Any sufficiently long
                    // footer does this, not just that provider's error text, and it
                    // gets worse below the 120-column width these captures were taken
                    // at.
                    //
                    // "Build ·" is the current-agent-mode label opencode draws on its
                    // own status line, one line above the crowded footer and always
                    // first on that line, so it is never pushed off by trailing model
                    // or status text. It is present in every capture taken this
                    // session, including all three where "ctrl+p commands" was
                    // crowded out (captures/opencode-blocked.txt,
                    // captures/opencode-after-retries.txt,
                    // captures/opencode-working.txt) — confirmed with
                    // `grep -c 'Build ·'`. Caveat: "Build" is the default mode; the
                    // idle screen's "tab agents" hint implies opencode has other
                    // agent modes (a "Plan"-style one, going by the palette's
                    // abbreviated "Ask"/"Buil[d]" row), and no capture this session
                    // ever showed one of those, so this string is only confirmed for
                    // the default mode.
                    //
                    // Between the two: if opencode ever renamed its process (it has
                    // not, per captures/opencode-process-name.txt, unlike claude and
                    // codex), a screen in a non-Build mode with a crowded footer would
                    // go unidentified. Real, but narrow enough not to invent a third
                    // marker for on top of two things that were never observed.
                    identity: s(&["ctrl+p commands", "Build ·"]),
                    // Deliberately empty. Real attempts to trigger a permission
                    // prompt — a file write, a file delete, a network fetch (`curl`),
                    // and `sudo -n` — all ran with no approval screen at all in this
                    // environment, including repeated in a directory opencode had
                    // never touched before to rule out a remembered per-directory
                    // trust grant (captures/opencode-autoapproved-write.txt,
                    // captures/opencode-autoapproved-delete-attempt.txt,
                    // captures/opencode-autoapproved-delete-done.txt,
                    // captures/opencode-autoapproved-curl.txt,
                    // captures/opencode-sudo-attempt.txt,
                    // captures/opencode-fresh-dir-risky.txt,
                    // captures/opencode-fresh-dir-risky2.txt). A billing error from
                    // the account's paid provider (captures/opencode-blocked.txt,
                    // captures/opencode-after-retries.txt) did produce a screen a
                    // human needs to act on, but that text is the provider's own
                    // error message passed through opencode's footer, not opencode's
                    // furniture — a user on a different provider, or with balance,
                    // would never see it, so it does not belong here. This list is
                    // empty by observation, not omission: opencode's own
                    // permission-prompt screen has never actually been seen.
                    blocked: Vec::new(),
                    // The escape-to-cancel hint, drawn only while a turn is in flight
                    // and gone the moment it finishes (captures/opencode-working2.txt
                    // has it, captures/opencode-idle2.txt right after does not).
                    working: s(&["esc interrupt"]),
                    // Native ACP subcommand, so no npm package to be renamed
                    // or deprecated out from under it — unlike every other
                    // adapter in this table.
                    adapter: Some(AdapterSpec {
                        program: "opencode".to_string(),
                        args: vec!["acp".to_string()],
                        env: Default::default(),
                    }),
                },
                AgentRules {
                    preset: "cursor".to_string(),
                    // cursor-agent runs as `node`, which cannot be claimed —
                    // matching it would label every node process a coding
                    // agent. Kept for installs that expose a real name; screen
                    // identity is what finds it here.
                    commands: s(&["cursor-agent"]),
                    // "Press any key to sign in" is now confirmed: it is the exact
                    // screen this install draws (captures/cursor-idle.txt). Pressing
                    // that key starts an OAuth sign-in flow, which this task was told
                    // not to attempt, so this install still could not get past it.
                    // "Cursor Agent" and "cursor-agent" remain UNVERIFIED — that
                    // sign-in screen renders its banner as block-graphic ASCII art,
                    // not literal text, so neither string actually appears in the
                    // one real cursor-agent screen reachable this session. They are
                    // left in place rather than removed on no evidence either way,
                    // but should be checked against a signed-in cursor-agent.
                    identity: s(&["Cursor Agent", "cursor-agent", "Press any key to sign in"]),
                    // UNVERIFIED. This install still could not get past its
                    // sign-in screen (see above), so unlike opencode's and the two
                    // above claude/codex sets, these were not read off a running
                    // agent. They follow the same shapes and should be checked
                    // against a signed-in cursor-agent before being trusted.
                    blocked: s(&["Do you want to", "Allow?", "[y/n]", "(y/N)", "\u{203a} 1."]),
                    working: s(&["esc to interrupt", "Generating"]),
                    // Best-effort, and knowingly so. `cursor-agent-acp` is
                    // third-party, sits at 0.1.1 and was last published in
                    // September 2025; there is no first-party alternative and
                    // no `@agentclientprotocol/cursor-acp`. It is shipped
                    // because failing to start is already handled honestly —
                    // `AdapterMissing` keeps the pane alive and says terminal
                    // mode is unaffected — and a toggle that might work beats
                    // one that certainly does not.
                    adapter: npx("cursor-agent-acp"),
                },
            ],
        }
    }

    /// Every entry, for callers that need the whole table.
    pub fn all(&self) -> &[AgentRules] {
        &self.rules
    }

    /// How to host this preset as a chat, if Far Cooler can.
    pub fn adapter(&self, preset: &str) -> Option<&AdapterSpec> {
        self.rules
            .iter()
            .find(|r| r.preset == preset)
            .and_then(|r| r.adapter.as_ref())
    }

    /// Whether Far Cooler can render this agent as a chat.
    ///
    /// Derived, never listed. This being a second hand-maintained list is what
    /// let it disagree with the rules it was supposed to match.
    pub fn chat_capable(&self, preset: &str) -> bool {
        self.adapter(preset).is_some()
    }

    /// Which agent, if any, is running in a pane.
    ///
    /// Matched on the foreground process. That is the only thing that answers
    /// the question honestly: a terminal launched as a shell in which someone
    /// typed `claude` IS a Claude Code terminal, and one launched as an agent
    /// that has since exited to a prompt is not.
    pub fn rules_for_command(&self, command: &str) -> Option<&AgentRules> {
        // The program, not the whole command line: identity comes from what was
        // run, and `claude --model opus` is still claude.
        let first = command.split_whitespace().next().unwrap_or(command);
        let name = first.rsplit('/').next().unwrap_or(first).trim();
        if name.is_empty() {
            return None;
        }
        // Prefix, because tmux truncates: `codex-aarch64-a` must match `codex`.
        self.rules
            .iter()
            .find(|r| r.commands.iter().any(|c| name.starts_with(c.as_str())))
    }

    /// Which agent is in this pane: by process, or failing that, by what it drew.
    ///
    /// Both are needed. Process matching is exact when it works, and it does not
    /// always work — Claude Code renames itself to its version number, so tmux
    /// reports `2.1.220` and no name matching will ever find it. Screen matching
    /// catches that, and is why the identity markers are agent furniture rather
    /// than anything a user could type.
    pub fn identify(&self, command: &str, screen: &str) -> Option<&AgentRules> {
        if let Some(rules) = self.rules_for_command(command) {
            return Some(rules);
        }
        let text = plain_text(screen);
        self.rules.iter().find(|r| {
            r.identity
                .iter()
                .any(|needle| text.contains(needle.as_str()))
        })
    }

    /// What to call whatever is running here.
    ///
    /// The agent's name when one is recognized, otherwise the process itself. A row
    /// then reads `claude` or `zsh` rather than the preset a terminal was created
    /// with, which after the first command is usually a lie.
    pub fn describe(&self, command: &str, screen: &str) -> String {
        if let Some(rules) = self.identify(command, screen) {
            return rules.preset.clone();
        }
        // A command line with arguments is already the label: `pnpm dev` says more
        // than `pnpm`, and taking a basename of it would cut it back to one word.
        if command.contains(' ') {
            return command.trim().to_string();
        }
        // A command line with arguments is already the label: `pnpm dev` says more
        // than `pnpm`, and taking a basename would cut it back to one word.
        if command.trim().contains(' ') {
            return command.trim().to_string();
        }
        let name = command.rsplit('/').next().unwrap_or(command).trim();
        // A process whose name is a version number tells a user nothing. Better to
        // say "shell" than to label a row `2.1.220`.
        let meaningless = name.is_empty() || name.chars().all(|c| c.is_ascii_digit() || c == '.');
        if meaningless {
            "shell".to_string()
        } else {
            name.to_string()
        }
    }

    /// Overlay configured adapters onto the built-in table.
    ///
    /// By preset name: a name that already exists replaces that entry's
    /// adapter and any detection field the user supplied; a new name appends a
    /// new entry. Appended rather than prepended so a user cannot accidentally
    /// shadow a built-in's detection by adding a broad `identity` string.
    pub fn merge(&mut self, entries: Vec<(String, crate::config::ConfigAdapter)>) {
        for (preset, cfg) in entries {
            // An adapter with no program cannot start, and offering the toggle
            // for one would produce exactly the silent failure this work exists
            // to remove.
            if cfg.program.trim().is_empty() {
                tracing::warn!(%preset, "ignoring a configured adapter with no program");
                continue;
            }
            let spec = AdapterSpec {
                program: cfg.program,
                args: cfg.args,
                env: cfg.env,
            };
            match self.rules.iter_mut().find(|r| r.preset == preset) {
                Some(existing) => {
                    existing.adapter = Some(spec);
                    // Only what was actually supplied. An empty list in TOML is
                    // indistinguishable from an absent one, and wiping a
                    // built-in's identity strings would make it undetectable.
                    if !cfg.commands.is_empty() {
                        existing.commands = cfg.commands;
                    }
                    if !cfg.identity.is_empty() {
                        existing.identity = cfg.identity;
                    }
                    if !cfg.blocked.is_empty() {
                        existing.blocked = cfg.blocked;
                    }
                    if !cfg.working.is_empty() {
                        existing.working = cfg.working;
                    }
                }
                None => self.rules.push(AgentRules {
                    preset,
                    commands: cfg.commands,
                    identity: cfg.identity,
                    blocked: cfg.blocked,
                    working: cfg.working,
                    adapter: Some(spec),
                }),
            }
        }
    }
}

/// Strip escape sequences and collapse whitespace.
///
/// This is not defensive tidying; without it nothing matches. Claude Code
/// colors every WORD separately, so a line that reads
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

impl Registry {
    /// Classify a rendered screen.
    ///
    /// `screen` is the visible pane, with or without escape sequences.
    pub fn classify(&self, command: &str, screen: &str) -> AgentActivity {
        let Some(rules) = self.identify(command, screen) else {
            return AgentActivity::None;
        };
        let screen = &plain_text(screen);

        // Blocked first. An agent asking permission still shows its working
        // furniture, so checking working first would hide every question.
        if rules
            .blocked
            .iter()
            .any(|needle| screen.contains(needle.as_str()))
        {
            return AgentActivity::Blocked;
        }
        if rules
            .working
            .iter()
            .any(|needle| screen.contains(needle.as_str()))
        {
            return AgentActivity::Working;
        }
        // Identified, not asking, not busy: it is sitting there. Requiring positive
        // idle furniture left an agent whose footer changed between versions stuck
        // on `unknown` forever — never reaching `done`, never notifying.
        AgentActivity::Idle
    }
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

// ---------------------------------------------------------------------------
// Proving an adapter works
// ---------------------------------------------------------------------------

/// How long to wait for `initialize` to answer before treating the adapter as
/// wedged, when the caller has no reason to pick a different bound.
///
/// Matches `crates/cli/src/agent_host.rs`'s `AgentSession::start` timeout, which
/// reasons about the identical state under the identical name
/// (`Status::AdapterSilent`): "an adapter that starts but never answers
/// `initialize` is a real state and it looks like nothing at all: the process is
/// alive, the pane is `running`, and the screen stays blank forever." 90s is
/// generous enough that a cold `npx` fetching a package on first use is not
/// killed mid-download.
pub const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);

/// What an adapter said when asked to identify itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handshake {
    /// The agent and version it reported, when it answered.
    pub reported: String,
}

/// Spawn an adapter, send an ACP `initialize`, and read until it answers.
///
/// This is what makes an adapter form worth having: without it, a wrong launch
/// command is discovered by opening a pane, pressing the chat toggle and getting
/// a blank screen — a failure that lands nowhere near the form that caused it.
///
/// Lives here rather than in `crates/core/tests/adapters.rs`, where it was
/// written, so the test that checks every built-in and the button a user presses
/// are one implementation rather than two that agree today.
///
/// Three properties worth keeping, each for an observed reason:
///
/// - **A silent adapter fails rather than hanging.** `BufReader::lines().next()`
///   has no timeout of its own, and a process that starts and writes nothing is
///   a real failure mode (`Status::AdapterSilent`). The read happens on its own
///   thread so the bound can be enforced from outside it; killing the child
///   closes the pipe that thread is blocked on and turns the block into an EOF,
///   so it exits on its own.
/// - **Lines that are not the answer are skipped.** Adapters log before they
///   answer, and treating the first line as the response fails on the ones that
///   are chattiest about starting up.
/// - **It runs in a temp directory.** An `initialize` is not scoped to a project
///   and should not be able to touch one.
///
/// What it does NOT prove, and callers must not imply otherwise: that the
/// adapter will be RECOGNIZED. `commands`, `identity`, `blocked` and `working`
/// are matched against agent output, and nothing here exercises them.
pub fn handshake(
    spec: &AdapterSpec,
    timeout: std::time::Duration,
) -> std::result::Result<Handshake, String> {
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command, Stdio};

    if spec.program.trim().is_empty() {
        return Err("no program to run".to_string());
    }

    let mut child = Command::new(&spec.program)
        .args(&spec.args)
        .envs(spec.env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("could not start `{}`: {e}", spec.program))?;

    let request = serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "protocolVersion": 1, "clientCapabilities": {} }
    });
    let mut stdin = child.stdin.take().expect("piped");
    let sent = writeln!(stdin, "{request}").and_then(|()| stdin.flush());
    if let Err(e) = sent {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("could not talk to `{}`: {e}", spec.program));
    }

    let stdout = child.stdout.take().expect("piped");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut lines = BufReader::new(stdout).lines();
        let result = loop {
            match lines.next() {
                Some(Ok(line)) => {
                    let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
                        continue;
                    };
                    if value.get("id") == Some(&serde_json::json!(1)) {
                        break Ok(value);
                    }
                }
                Some(Err(e)) => break Err(e.to_string()),
                None => break Err("the adapter closed without answering".to_string()),
            }
        };
        // If `recv_timeout` already gave up, the receiver is gone and this send
        // fails. There is nothing left to report to, and the thread's only
        // remaining job is to exit — which it now does.
        let _ = tx.send(result);
    });

    let answer = rx.recv_timeout(timeout).unwrap_or_else(|_| {
        Err("the adapter started and then went silent".to_string())
    });
    let _ = child.kill();
    let _ = child.wait();

    let value = answer?;
    // An `error` member is a well-formed refusal, not a success. Reported as the
    // adapter's own words rather than as "handshake failed", because the message
    // is the only clue about which parameter it disliked.
    if let Some(error) = value.get("error") {
        let message = error
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("the adapter refused to initialize");
        return Err(message.to_string());
    }
    let result = value
        .get("result")
        .ok_or_else(|| "the adapter answered without a result".to_string())?;

    Ok(Handshake { reported: describe(result) })
}

/// A one-line "who are you" from an `initialize` result.
///
/// ACP puts the agent's name and version under `agentInfo`, but not every
/// adapter fills it in, and one that answered correctly should not be reported
/// as anonymous. So: the name and version when they are there, the protocol
/// version when they are not, and a bare acknowledgement when neither is.
fn describe(result: &serde_json::Value) -> String {
    let info = result.get("agentInfo");
    let name = info.and_then(|i| i.get("name")).and_then(|n| n.as_str());
    let version = info.and_then(|i| i.get("version")).and_then(|v| v.as_str());
    match (name, version) {
        (Some(n), Some(v)) => format!("{n} {v}"),
        (Some(n), None) => n.to_string(),
        _ => match result.get("protocolVersion") {
            Some(p) => format!("answered, ACP protocol {p}"),
            None => "answered".to_string(),
        },
    }
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
        let r = Registry::built_in();
        assert_eq!(r.classify("zsh", "e-liang@Mac project % "), None);
        assert_eq!(r.classify("bash", "anything at all"), None);
    }

    #[test]
    fn an_agent_is_found_by_what_is_running_not_by_how_it_was_launched() {
        // The whole point: terminals are created as plain shells and the user
        // types `claude` into them. Keying on the launch preset would report a
        // live agent as a shell forever.
        let r = Registry::built_in();
        assert!(r.rules_for_command("claude").is_some());
        assert!(r.rules_for_command("/opt/homebrew/bin/claude").is_some());
        assert!(r.rules_for_command("cursor-agent").is_some());
        assert!(r.rules_for_command("zsh").is_none());
        assert!(r.rules_for_command("").is_none());
    }

    #[test]
    fn a_row_is_labeled_by_what_is_actually_running() {
        let r = Registry::built_in();
        assert_eq!(r.describe("claude", ""), "claude");
        assert_eq!(r.describe("cursor-agent", ""), "cursor");
        // Not an agent: say what it is rather than inventing a category.
        assert_eq!(r.describe("zsh", ""), "zsh");
        assert_eq!(r.describe("cargo", ""), "cargo");
        assert_eq!(r.describe("", ""), "shell");
    }

    #[test]
    fn an_agent_that_renamed_itself_is_still_found() {
        // The real case that broke process matching: Claude Code sets its
        // process name to its version, so tmux reports `2.1.220`.
        let r = Registry::built_in();
        let screen = "❯ hello?\n  ⏸ manual mode on · ? for shortcuts";
        assert!(r.identify("2.1.220", screen).is_some());
        assert_eq!(r.describe("2.1.220", screen), "claude");
        assert_eq!(r.classify("2.1.220", screen), Idle);
    }

    #[test]
    fn a_version_number_is_never_shown_as_a_name() {
        // Whatever it is, `2.1.220` is not a useful label for a row.
        let r = Registry::built_in();
        assert_eq!(r.describe("2.1.220", "nothing recognizable"), "shell");
        assert_eq!(r.describe("1.2", ""), "shell");
    }

    #[test]
    fn a_shell_showing_agent_like_text_is_not_promoted_to_an_agent() {
        // Identity markers are agent furniture, not phrases a user might type.
        let r = Registry::built_in();
        assert!(
            r.identify("zsh", "$ echo 'do you want to proceed?'")
                .is_none()
        );
        assert_eq!(r.classify("zsh", "$ echo '[y/n]'"), None);
    }

    #[test]
    fn claude_working_is_recognized() {
        let r = Registry::built_in();
        let screen = "\
✻ Cooked for 6s
  Running 2 shell commands · 4s
  esc to interrupt";
        assert_eq!(r.classify("claude", screen), Working);
    }

    #[test]
    fn claude_idle_is_recognized() {
        let r = Registry::built_in();
        let screen = "\
❯ hello?
────────────────────────
  ⏸ manual mode on · ? for shortcuts";
        assert_eq!(r.classify("claude", screen), Idle);
    }

    #[test]
    fn codex_is_recognized_through_a_truncated_process_name() {
        // tmux caps the field, so a real codex arrives as `codex-aarch64-a`.
        // Exact matching found nothing and every codex reported as a shell.
        let r = Registry::built_in();
        assert!(r.rules_for_command("codex-aarch64-a").is_some());
        assert_eq!(r.describe("codex-aarch64-a", ""), "codex");
    }

    #[test]
    fn codex_states_come_from_its_real_screen() {
        let r = Registry::built_in();
        let idle =
            "› Explain this codebase\n  gpt-5.6-sol high · ~/project\n  >_ OpenAI Codex (v0.145.0)";
        assert_eq!(r.classify("codex-aarch64-a", idle), Idle);

        let working = "• Working (6s • esc to interrupt) · 1 background terminal running";
        assert_eq!(r.classify("codex-aarch64-a", working), Working);

        let blocked = "› 1. Update now\n  2. Skip\n  Press enter to continue";
        assert_eq!(r.classify("codex-aarch64-a", blocked), Blocked);
    }

    #[test]
    fn cursor_cannot_be_claimed_by_its_process() {
        // cursor-agent runs as plain `node`. Matching that would label every
        // node process a coding agent, which is worse than missing it.
        let r = Registry::built_in();
        assert!(r.rules_for_command("node").is_none());
        assert_eq!(r.describe("node", ""), "node");
        // It is found by what it draws instead.
        assert!(r.identify("node", "Press any key to sign in...").is_some());
    }

    #[test]
    fn an_identified_agent_is_never_stuck_on_unknown() {
        // The failure this replaced: an agent whose footer changed between
        // versions matched no idle signature and reported `unknown` forever,
        // so it never reached `done` and never notified.
        let r = Registry::built_in();
        let unfamiliar = "OpenAI Codex\nsome screen nobody wrote a rule for";
        assert_eq!(r.classify("codex", unfamiliar), Idle);
    }

    #[test]
    fn a_question_beats_the_working_furniture_around_it() {
        // The case that decides whether this feature is worth having. An agent
        // asking permission still shows its spinner chrome, so checking
        // "working" first would mean never noticing that something needs you.
        let r = Registry::built_in();
        let screen = "\
Do you want to allow this command?
❯ 1. Yes
  2. No
  esc to interrupt";
        assert_eq!(r.classify("claude", screen), Blocked);
    }

    #[test]
    fn something_that_is_not_an_agent_stays_not_an_agent() {
        // The important half of the honesty: a screen we cannot identify is not
        // promoted to an agent, so a shell never appears in the list of things
        // that might need you.
        let r = Registry::built_in();
        assert_eq!(
            r.classify("zsh", "some output nobody wrote a rule for"),
            None
        );
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
        // be a silent hole rather than a visible bug — EXCEPT opencode's
        // `blocked`, which is empty on purpose. Every real attempt to trigger
        // its permission prompt (write, delete, curl, `sudo -n`, repeated in a
        // directory it had never touched) ran with no approval screen at all;
        // the one "needs a human" screen this session ever produced was a
        // provider's own billing-error text, not opencode's furniture, so it
        // was left out rather than shipped as a rule that looks like coverage
        // but fires for almost nobody. See the field's own comment in
        // `built_in()` for the full account. A silent hole is a rule nobody
        // wrote; this is a rule nobody has seen yet — worth distinguishing.
        for rules in Registry::built_in().all() {
            if rules.preset != "opencode" {
                assert!(
                    !rules.blocked.is_empty(),
                    "{} has no blocked rules",
                    rules.preset
                );
            }
            assert!(
                !rules.working.is_empty(),
                "{} has no working rules",
                rules.preset
            );
        }
    }

    #[test]
    fn no_signature_appears_in_two_states_of_the_same_agent() {
        // An overlapping signature makes the result depend on check order,
        // which is exactly the kind of thing that works until it does not.
        for rules in Registry::built_in().all() {
            for needle in &rules.blocked {
                assert!(
                    !rules.working.contains(needle),
                    "{needle:?} is both blocked and working"
                );
            }
        }
    }

    #[test]
    fn every_agent_can_be_recognized_without_its_process_name() {
        // Process names are unreliable, so identity markers are what actually
        // has to hold. An agent with none is invisible the moment it renames
        // itself.
        for rules in Registry::built_in().all() {
            assert!(
                !rules.identity.is_empty(),
                "{} has no identity markers",
                rules.preset
            );
        }
    }

    #[test]
    fn every_agent_declares_a_process_to_match() {
        // A rule set with no command can never be selected, which is a silent
        // hole rather than a visible bug.
        for rules in Registry::built_in().all() {
            assert!(
                !rules.commands.is_empty(),
                "{} matches no process",
                rules.preset
            );
        }
    }

    #[test]
    fn an_agent_is_chat_capable_exactly_when_it_has_an_adapter() {
        // The bug this replaces: `CHAT_CAPABLE` was a second list maintained by
        // hand beside the rules, and the two disagreed. Derived, they cannot.
        //
        // `opencode` is back in this list as of Task 5: it now has a real,
        // screen-verified entry in `built_in()`. See task-1-report.md for why
        // it was dropped from here in Task 1.
        let r = Registry::built_in();
        for preset in ["claude", "codex", "cursor", "opencode"] {
            assert!(r.chat_capable(preset), "{preset} ships an adapter");
            assert!(r.adapter(preset).is_some());
        }
        assert!(!r.chat_capable("zsh"));
        assert!(r.adapter("zsh").is_none());
    }

    #[test]
    fn opencode_needs_no_npm_package() {
        // Its ACP mode is a native subcommand, which is the whole reason it is
        // preferred over an npm-packaged ACP adapter. Restored from Task 1,
        // which had to omit it because `opencode` did not exist in the
        // registry yet.
        let spec = Registry::built_in()
            .adapter("opencode")
            .expect("opencode ships an adapter")
            .clone();
        assert_eq!(spec.program, "opencode");
        assert_eq!(spec.args, vec!["acp".to_string()]);
    }

    #[test]
    fn opencode_is_recognized_by_process_name() {
        // Confirmed with `tmux display-message -p '#{pane_current_command}'`
        // against a running opencode (captures/opencode-process-name.txt): it
        // reports itself as plain `opencode`, unlike claude and codex.
        let r = Registry::built_in();
        assert_eq!(r.describe("opencode", ""), "opencode");
    }

    #[test]
    fn opencode_states_come_from_its_real_screen() {
        // Substrings pasted verbatim from captures/opencode-idle.txt and
        // captures/opencode-working2.txt — real screens from a running
        // opencode, not invented ones. No Blocked case here: opencode's
        // `blocked` list is deliberately empty (see the comment on it in
        // `built_in()`), so there is no real screen to assert one against.
        let r = Registry::built_in();

        let idle = "\
                       ┃  Ask anything... \"Fix a TODO in the codebase\"
                       ┃
                       ┃  Build · GLM-5.2 Z.AI Coding Plan
                       tab agents  ctrl+p commands";
        assert_eq!(r.classify("opencode", idle), Idle);

        let working = "\
  ┃  Build · Laguna S 2.1 Free OpenCode Zen
   ⬝⬝⬝⬝⬝⬝⬝⬝  esc interrupt                                                                 17.7K (7%)  ctrl+p commands";
        assert_eq!(r.classify("opencode", working), Working);

        // The screen a billing-crippled account actually produced. It IS a
        // real screen a human needs to act on, but the text is the provider's
        // own error message, not opencode's — so it classifies as Working
        // (its "esc interrupt" hint is still present) rather than Blocked,
        // and that is the honest result of leaving `blocked` empty rather
        // than a bug to paper over.
        let billing_error = "\
   ⬝⬝⬝⬝⬝⬝⬝⬝ Insufficient balance or no resource package. Please recharge. [retrying in 11s attempt #4]   esc interrupt";
        assert_eq!(r.classify("opencode", billing_error), Working);
    }

    #[test]
    fn every_adapter_belongs_to_an_agent_that_can_be_detected() {
        // An adapter on a preset nothing can identify is dead config: `⌃B a`
        // chooses the adapter from the DETECTED preset, so an entry no screen and
        // no process name can reach could never be selected.
        //
        // Non-empty `commands`/`identity` is not enough to prove that, though —
        // a preset shadowed by an EARLIER entry's command prefix (`identify`
        // returns the first match, in table order) would carry a non-empty list
        // and still never be reachable. So this drives `identify` itself, by
        // both paths, and asserts it resolves to THIS preset rather than merely
        // to something.
        let r = Registry::built_in();
        for rules in r.all() {
            if rules.adapter.is_none() {
                continue;
            }
            let Some(command) = rules.commands.first() else {
                assert!(
                    !rules.identity.is_empty(),
                    "{} has an adapter but no commands and no identity to detect it",
                    rules.preset
                );
                continue;
            };
            // Screen left blank: `identify` checks the process first and never
            // looks at the screen when that hits, so a blank screen isolates
            // this to the command path.
            let by_command = r.identify(command, "");
            assert_eq!(
                by_command.map(|found| found.preset.as_str()),
                Some(rules.preset.as_str()),
                "{}'s command `{command}` resolves to {:?}, not {} — shadowed by an \
                 earlier entry's command prefix",
                rules.preset,
                by_command.map(|found| found.preset.as_str()),
                rules.preset
            );

            let Some(identity) = rules.identity.first() else {
                continue;
            };
            // Command left as something no built-in claims, so this run
            // isolates the screen path the same way the one above isolated
            // the command path.
            let by_identity = r.identify("nothing-recognizable", identity);
            assert_eq!(
                by_identity.map(|found| found.preset.as_str()),
                Some(rules.preset.as_str()),
                "{}'s identity string `{identity}` resolves to {:?}, not {} — shadowed \
                 by an earlier entry's identity string",
                rules.preset,
                by_identity.map(|found| found.preset.as_str()),
                rules.preset
            );
        }
    }

    #[test]
    fn no_built_in_adapter_uses_a_deprecated_zed_package() {
        // Every `@zed-industries/*` ACP package has moved to
        // `@agentclientprotocol/*`. The old codex one is deprecated and stalled at
        // 0.16.0 while the live package is at 1.1.9 — the identical trap already
        // recorded for the Claude adapter.
        for rules in Registry::built_in().all() {
            let Some(spec) = &rules.adapter else { continue };
            for arg in &spec.args {
                assert!(
                    !arg.contains("@zed-industries/"),
                    "{} uses a deprecated package: {arg}",
                    rules.preset
                );
            }
        }
    }

    // ---- the ACP handshake ----

    /// An adapter that answers `initialize` the way a real one does.
    fn answering(body: &str) -> AdapterSpec {
        AdapterSpec {
            program: "sh".into(),
            args: vec!["-c".into(), format!("read line; printf '%s\\n' '{body}'")],
            env: Default::default(),
        }
    }

    #[test]
    fn a_handshake_reports_the_agent_it_was_told_about() {
        let spec = answering(
            r#"{"jsonrpc":"2.0","id":1,"result":{"agentInfo":{"name":"My Agent","version":"1.2.3"}}}"#,
        );
        let shake = handshake(&spec, std::time::Duration::from_secs(10)).expect("answered");
        assert_eq!(shake.reported, "My Agent 1.2.3");
    }

    #[test]
    fn an_adapter_that_answers_without_naming_itself_still_succeeds() {
        // Not every adapter fills in agentInfo, and one that spoke ACP
        // correctly must not be reported as a failure over a missing field.
        let spec = answering(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        let shake = handshake(&spec, std::time::Duration::from_secs(10)).expect("answered");
        assert!(shake.reported.contains("protocol 1"), "{}", shake.reported);
    }

    #[test]
    fn chatter_before_the_answer_is_skipped() {
        // Adapters log while starting up. Treating the first line as the
        // response fails on exactly the ones that say the most about booting.
        let spec = AdapterSpec {
            program: "sh".into(),
            args: vec![
                "-c".into(),
                concat!(
                    "read line; ",
                    "echo 'starting up'; ",
                    r#"echo '{"jsonrpc":"2.0","method":"log","params":{}}'; "#,
                    r#"echo '{"jsonrpc":"2.0","id":1,"result":{"agentInfo":{"name":"A","version":"9"}}}'"#,
                ).into(),
            ],
            env: Default::default(),
        };
        assert_eq!(
            handshake(&spec, std::time::Duration::from_secs(10)).expect("answered").reported,
            "A 9"
        );
    }

    #[test]
    fn an_error_answer_is_reported_in_the_adapters_own_words() {
        // The message is the only clue about which parameter it disliked, so
        // "handshake failed" would throw away the useful half.
        let spec = answering(
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"unsupported protocolVersion"}}"#,
        );
        let failure = handshake(&spec, std::time::Duration::from_secs(10)).expect_err("refused");
        assert_eq!(failure, "unsupported protocolVersion");
    }

    #[test]
    fn a_silent_adapter_fails_fast_instead_of_hanging() {
        // `sh -c "sleep 1000"` is a live process with a stdout that never
        // produces a line, which is exactly what an adapter that starts and
        // goes silent looks like from here. A short explicit bound, so the
        // suite does not pay the production timeout to prove the mechanism.
        let spec = AdapterSpec {
            program: "sh".into(),
            args: vec!["-c".into(), "sleep 1000".into()],
            env: Default::default(),
        };
        let failure =
            handshake(&spec, std::time::Duration::from_millis(500)).expect_err("must not hang");
        assert!(failure.contains("silent"), "{failure}");
    }

    #[test]
    fn a_program_that_does_not_exist_says_so() {
        let spec = AdapterSpec {
            program: "farcooler-no-such-program".into(),
            args: vec![],
            env: Default::default(),
        };
        let failure = handshake(&spec, std::time::Duration::from_secs(5)).expect_err("no program");
        assert!(failure.contains("could not start"), "{failure}");
    }

    #[test]
    fn an_adapter_with_no_program_is_refused_without_spawning_anything() {
        let spec = AdapterSpec { program: "  ".into(), args: vec![], env: Default::default() };
        assert_eq!(
            handshake(&spec, std::time::Duration::from_secs(5)).expect_err("refused"),
            "no program to run"
        );
    }
}
