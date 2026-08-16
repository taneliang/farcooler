//! The only tests that run the real coding agents.
//!
//! # Why this exists
//!
//! Far Cooler decides what an agent is doing by matching furniture in somebody
//! else's TUI. That furniture changes with no changelog, and nothing else in
//! this repository notices. Two real instances, both found by hand:
//!
//! - the `cursor` rules were written against a sign-in screen nobody could get
//!   past, and every field was wrong — `Do you want to`, `Allow?`, `› 1.` and
//!   `Generating` appear nowhere in a running cursor-agent;
//! - the `codex` rules carried `Press enter to continue`, which is the wording
//!   of the TRUST GATE, not the approval prompt (`Press enter to confirm`), so
//!   for an unknown period only a numbered-list marker caught a permission
//!   request at all.
//!
//! A unit test cannot find either, because its fixture is a string somebody
//! typed. Only a real binary drawing a real screen can.
//!
//! # Running it
//!
//! Every test here is `#[ignore]`, so `cargo test` never touches it. Run it
//! deliberately — after changing anything in `activity.rs` or `title.rs`, and
//! periodically to catch a third-party release:
//!
//! ```text
//! cargo test -p farcooler-core --test live_agents -- --ignored --nocapture
//! ```
//!
//! One agent at a time:
//!
//! ```text
//! cargo test -p farcooler-core --test live_agents claude -- --ignored --nocapture
//! ```
//!
//! # What it costs, and what it needs
//!
//! A full run is about four minutes and a few cents: the cheapest model per
//! agent and a one-line prompt. It needs the CLIs already signed in — auth does
//! not survive pointing an agent at a scratch config directory, which is why
//! this runs on a developer's machine rather than in CI.
//!
//! A missing or unauthenticated binary SKIPS, loudly, and does not fail. An
//! agent running this suite on a machine without `cursor-agent` should learn
//! that it was not checked, not chase a red herring.
//!
//! # The rule that keeps it honest
//!
//! Assert on FURNITURE, never on model output. `esc to interrupt` is furniture.
//! Anything the model wrote is nondeterministic and will eventually differ.
//!
//! When a check fails it writes the captured screen to `target/live-agents/`
//! and prints the path. That capture is both the bug report and the fix: it
//! drops straight into `crates/core/captures/` once the rules are corrected.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use farcooler_core::activity::Registry;
use farcooler_protocol::v1::AgentActivity;

/// How long to wait for an agent to reach a state before giving up.
///
/// Generous: a cold model call over a slow link is not a regression, and a
/// flaky suite is one that gets switched off.
const REACH: Duration = Duration::from_secs(90);

/// How often to look. Fast enough to catch codex, whose turns have finished
/// inside a two-second window — which is how its footer depths came to be
/// inferred from an idle screen rather than measured from a working one.
const SAMPLE: Duration = Duration::from_millis(350);

// ---------------------------------------------------------------- the agents

struct AgentSpec {
    /// The preset name, matching `Registry::built_in`.
    preset: &'static str,
    /// The binary to look for on `PATH`.
    binary: &'static str,
    /// Arguments, including the cheapest model this agent offers.
    ///
    /// Cursor takes `auto` rather than a named model: a free plan REJECTS an
    /// explicit one with "free plans can only use Auto", and the turn then
    /// fails in a way that reads exactly like a classification bug.
    args: &'static [&'static str],
    /// What dismisses the trust gate. Each of the three differs.
    trust_key: &'static str,
    /// What `pane_current_command` reports, for `classify`. Claude renames
    /// itself to its version and cursor-agent runs as plain `node`, so this is
    /// deliberately not always the binary's name.
    command_hint: &'static str,
}

const CLAUDE: AgentSpec = AgentSpec {
    preset: "claude",
    binary: "claude",
    args: &["--model", "haiku"],
    trust_key: "Enter",
    command_hint: "claude",
};

const CODEX: AgentSpec = AgentSpec {
    preset: "codex",
    binary: "codex",
    // `untrusted` forces the approval prompt this suite needs to observe;
    // without it a trusted directory runs the command and the Blocked state
    // never happens.
    args: &["-m", "gpt-5.6-luna", "-a", "untrusted", "-s", "read-only"],
    trust_key: "Enter",
    command_hint: "codex",
};

const CURSOR: AgentSpec = AgentSpec {
    preset: "cursor",
    binary: "cursor-agent",
    args: &["--model", "auto"],
    trust_key: "a",
    command_hint: "node",
};

/// A prompt that must produce a permission prompt on every one of the three.
const PROMPT: &str = "Run the shell command: echo banana > fruit.txt";

// --------------------------------------------------------------- the harness

/// The real binary, skipping wrappers that shadow it.
///
/// A development machine can have `codex` and `cursor-agent` on `PATH` as
/// wrapper scripts that add flags and spawn side processes. Testing those tests
/// one developer's setup rather than what a user runs — and it is not cosmetic:
/// under a wrapper `pane_current_command` reads `bash`, where the bare binary
/// reads `codex`.
fn bare_binary(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .filter(|dir| !dir.to_string_lossy().contains("superset"))
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

/// A scratch tmux server that cleans itself up.
///
/// Its own socket, and its own minimal config: without `-f`, a developer's
/// `~/.tmux.conf` can spawn extra panes and make `list-panes` unreadable. It is
/// killed by that exact socket name and never by pattern — a live Far Cooler
/// app may be sharing this machine.
struct Pane {
    socket: String,
    dir: PathBuf,
}

impl Pane {
    fn start(spec: &AgentSpec, exe: &Path) -> Option<Pane> {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_millis();
        let socket = format!("fclive-{}-{}", spec.preset, stamp);
        let dir = std::env::temp_dir().join(format!("farcooler-live-{}-{}", spec.preset, stamp));
        std::fs::create_dir_all(&dir).ok()?;
        let conf = dir.join("tmux.conf");
        std::fs::write(&conf, "set -g default-terminal screen-256color\n").ok()?;

        let mut command = vec![exe.display().to_string()];
        command.extend(spec.args.iter().map(|a| a.to_string()));

        let ok = Command::new("tmux")
            .args(["-L", &socket, "-f"])
            .arg(&conf)
            .args(["new-session", "-d", "-s", "p", "-x", "120", "-y", "40", "-c"])
            .arg(&dir)
            .arg(command.join(" "))
            .status()
            .ok()?
            .success();
        if !ok {
            return None;
        }
        Some(Pane { socket, dir })
    }

    fn tmux(&self, args: &[&str]) -> String {
        let out = Command::new("tmux")
            .args(["-L", &self.socket])
            .args(args)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();
        out
    }

    fn screen(&self) -> String {
        self.tmux(&["capture-pane", "-p", "-t", "p"])
    }

    fn title(&self) -> String {
        self.tmux(&["display-message", "-p", "-t", "p", "#{pane_title}"]).trim().to_string()
    }

    /// A key by name (`Enter`), or a literal character.
    fn key(&self, key: &str) {
        self.tmux(&["send-keys", "-t", "p", key]);
    }

    /// Text, sent literally so a leading dash or a quote cannot be read as an
    /// option or a key name.
    fn type_text(&self, text: &str) {
        self.tmux(&["send-keys", "-t", "p", "-l", text]);
    }
}

impl Drop for Pane {
    fn drop(&mut self) {
        // By exact socket, never by pattern.
        let _ = Command::new("tmux").args(["-L", &self.socket, "kill-server"]).status();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Wait until the classifier reports `want`, or give up and say what it saw.
///
/// Returns the screen at the moment it matched, so a caller can assert more
/// about it without capturing twice.
fn wait_for(
    pane: &Pane,
    registry: &Registry,
    spec: &AgentSpec,
    want: AgentActivity,
) -> Result<String, String> {
    let start = Instant::now();
    let mut last = AgentActivity::Unspecified;
    while start.elapsed() < REACH {
        let screen = pane.screen();
        let seen = registry.classify(spec.command_hint, &screen);
        if seen == want {
            return Ok(screen);
        }
        last = seen;
        std::thread::sleep(SAMPLE);
    }
    let screen = pane.screen();
    let path = dump(spec, want, &screen);
    Err(format!(
        "{}: waited {}s for {:?} and never saw it (last {:?}). Screen written to {}",
        spec.preset,
        REACH.as_secs(),
        want,
        last,
        path.display()
    ))
}

/// Write a screen that failed a check, so the evidence outlives the run.
///
/// This file IS the fix: once the rules are corrected it belongs in
/// `crates/core/captures/` under a name saying which state it is.
fn dump(spec: &AgentSpec, want: AgentActivity, screen: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/live-agents");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("{}-expected-{:?}.txt", spec.preset, want).to_lowercase());
    let _ = std::fs::write(&path, screen);
    path
}

/// Drive one agent through working, blocked and idle.
///
/// The sequence is the contract, and each step is a separate claim: an agent
/// that reaches Blocked but never returns to Idle is the thirty-three hour bug,
/// and one that reaches Idle without ever having been Blocked means the
/// approval prompt was missed entirely.
fn drive(spec: &AgentSpec) {
    let Some(exe) = bare_binary(spec.binary) else {
        eprintln!("SKIP {}: no bare `{}` on PATH", spec.preset, spec.binary);
        return;
    };
    eprintln!("== {} ({})", spec.preset, exe.display());

    let Some(pane) = Pane::start(spec, &exe) else {
        eprintln!("SKIP {}: could not start a scratch tmux session", spec.preset);
        return;
    };
    let registry = Registry::built_in();

    // The trust gate. First screen of a new directory, and everything is behind
    // it — so it is also the first thing that must classify as Blocked.
    std::thread::sleep(Duration::from_secs(6));
    let gate = pane.screen();
    assert_eq!(
        registry.classify(spec.command_hint, &gate),
        AgentActivity::Blocked,
        "{}: a trust gate is a pane that needs you. Screen at {}",
        spec.preset,
        dump(spec, AgentActivity::Blocked, &gate).display()
    );
    pane.key(spec.trust_key);
    std::thread::sleep(Duration::from_secs(5));

    pane.type_text(PROMPT);
    std::thread::sleep(Duration::from_secs(1));
    pane.key("Enter");

    // Working, then blocked on the command it wants to run.
    wait_for(&pane, &registry, spec, AgentActivity::Working).unwrap_or_else(|e| panic!("{e}"));
    let blocked =
        wait_for(&pane, &registry, spec, AgentActivity::Blocked).unwrap_or_else(|e| panic!("{e}"));

    // And it must be able to say WHAT it is asking. A row that says "Needs you"
    // and nothing else is what this feature exists to improve on.
    let question = registry.blocked_question(spec.command_hint, &blocked);
    assert!(
        question.is_some(),
        "{}: blocked but no question could be read. Screen at {}",
        spec.preset,
        dump(spec, AgentActivity::Blocked, &blocked).display()
    );
    eprintln!("   question: {}", question.unwrap_or_default());

    // Approve, and the turn must END. This is the regression that started all
    // of it: a finished agent that never leaves Working never reaches Done, so
    // nobody is ever told.
    pane.key("Enter");
    wait_for(&pane, &registry, spec, AgentActivity::Idle).unwrap_or_else(|e| panic!("{e}"));
    eprintln!("   working -> blocked -> idle, title {:?}", pane.title());
}

// ----------------------------------------------------------------- the tests

#[test]
#[ignore = "runs the real claude binary; costs money"]
fn claude_still_reports_its_states() {
    drive(&CLAUDE);
}

#[test]
#[ignore = "runs the real codex binary; costs money"]
fn codex_still_reports_its_states() {
    drive(&CODEX);
}

#[test]
#[ignore = "runs the real cursor-agent binary; costs money"]
fn cursor_still_reports_its_states() {
    drive(&CURSOR);
}

// ------------------------------------------------- the session-log contracts
//
// Stage 2 reads these instead of scraping a screen. They are private formats
// with no compatibility promise, so a field vanishing from one breaks as much
// as a footer changing and far more quietly — which is the whole reason they
// are checked here beside the screens.

/// The newest `.jsonl` under `root`, if there is one.
fn newest_jsonl(root: &Path) -> Option<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "jsonl") {
                out.push(path);
            }
        }
    }
    let mut found = Vec::new();
    walk(root, &mut found);
    found.sort_by_key(|p| p.metadata().and_then(|m| m.modified()).ok());
    found.pop()
}

/// Every key appearing anywhere in a JSONL file, at any depth.
fn keys_in(path: &Path) -> std::collections::HashSet<String> {
    fn collect(value: &serde_json::Value, out: &mut std::collections::HashSet<String>) {
        match value {
            serde_json::Value::Object(map) => {
                for (k, v) in map {
                    out.insert(k.clone());
                    collect(v, out);
                }
            }
            serde_json::Value::Array(items) => items.iter().for_each(|v| collect(v, out)),
            _ => {}
        }
    }
    let mut keys = std::collections::HashSet::new();
    let Ok(text) = std::fs::read_to_string(path) else { return keys };
    for line in text.lines() {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
            collect(&value, &mut keys);
        }
    }
    keys
}

fn assert_log_has(agent: &str, root: PathBuf, wanted: &[&str]) {
    if !root.exists() {
        eprintln!("SKIP {agent} session log: {} does not exist", root.display());
        return;
    }
    let Some(log) = newest_jsonl(&root) else {
        eprintln!("SKIP {agent} session log: no .jsonl under {}", root.display());
        return;
    };
    let keys = keys_in(&log);
    let missing: Vec<&&str> = wanted.iter().filter(|k| !keys.contains(**k)).collect();
    assert!(
        missing.is_empty(),
        "{agent}: {} no longer carries {:?}. Stage 2 reads these; a field that \
         disappears here breaks as quietly as a footer that moves.",
        log.display(),
        missing
    );
    eprintln!("== {agent} session log OK ({})", log.display());
}

#[test]
#[ignore = "reads the agent's own session logs on this machine"]
fn claude_session_log_still_carries_what_stage_two_reads() {
    let root = dirs_home().join(".claude/projects");
    assert_log_has("claude", root, &["aiTitle", "isSidechain", "stop_reason", "usage", "cwd"]);
}

#[test]
#[ignore = "reads the agent's own session logs on this machine"]
fn codex_session_log_still_carries_what_stage_two_reads() {
    let root = dirs_home().join(".codex/sessions");
    assert_log_has("codex", root, &["payload", "timestamp", "type"]);
}

#[test]
#[ignore = "reads the agent's own session logs on this machine"]
fn cursor_session_log_still_carries_what_stage_two_reads() {
    let root = dirs_home().join(".cursor/projects");
    assert_log_has("cursor", root, &["role", "message", "type"]);
}

fn dirs_home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").expect("a home directory"))
}
