//! The one switch for tests that start a real coding agent.
//!
//! Shared by `#[path]` rather than through a crate, so no library carries a
//! test-only function: every test that can launch `claude`, `codex`,
//! `cursor-agent` or an `npx` ACP adapter includes this file and asks it first.
//!
//! Two locks, both needed. Each such test is `#[ignore]`, so a plain
//! `cargo test` never reaches it; and it returns at once unless
//! `FARCOOLER_LIVE_AGENTS=1`, so `cargo test -- --ignored` (or
//! `--include-ignored`) does not reach a real agent either. A lane that runs
//! the workspace's tests, ignored or not, starts nothing it pays for.
//!
//! Each gated test's `#[ignore = "…"]` names the agent and the switch, so the
//! default run's `ignored, …` line says how to run it. (An attribute takes only
//! a literal, so that sentence cannot live here.)
//!
//! With the switch on, a missing agent is still a FAILURE where the test says
//! so: the switch decides whether to try, never whether failing to try counts.
//!
//! ```text
//! FARCOOLER_LIVE_AGENTS=1 cargo test -p farcooler-claude --test live_turn -- --ignored
//! ```

#![allow(dead_code)]

use std::io::Write;
use std::path::PathBuf;

/// The variable that has to be exactly `1`.
pub const SWITCH: &str = "FARCOOLER_LIVE_AGENTS";

/// Whether `test` may start a real agent. When it may not, says so and why.
///
/// The message goes straight to the process's stderr rather than through
/// `eprintln!`, which libtest captures and throws away for a passing test: a
/// skip nobody can see is the silent pass this switch must not become.
pub fn enabled(test: &str) -> bool {
    if std::env::var_os(SWITCH).is_some_and(|v| v == "1") {
        return true;
    }
    let _ = writeln!(
        std::io::stderr(),
        "SKIP {test}: it starts a real agent, and {SWITCH} is not 1. \
         Set {SWITCH}=1 to run it."
    );
    false
}

/// The installed `name`, found the way the daemon finds it: this process's
/// own `PATH` first (`farcooler_core::programs::find`), then a login shell's.
///
/// `PATH` first matters beyond matching the daemon. A login `sh` runs macOS's
/// `path_helper`, which puts `/etc/paths.d` — `/opt/homebrew/bin` — AHEAD of
/// anything inherited, so asking it first would pick a real agent over a
/// `PATH` a caller set up to keep the real one out.
///
/// Panics when nothing has it: a missing agent is a failure, not a skip.
pub fn installed(name: &str) -> PathBuf {
    let on_path = std::env::var_os("PATH")
        .and_then(|raw| std::env::split_paths(&raw).map(|d| d.join(name)).find(|p| p.is_file()));
    if let Some(path) = on_path {
        return path;
    }
    let out = std::process::Command::new("sh")
        .args(["-lc", &format!("command -v {name}")])
        .output()
        .expect("sh must run");
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert!(!path.is_empty(), "{name} must be installed for this test to mean anything");
    PathBuf::from(path)
}
