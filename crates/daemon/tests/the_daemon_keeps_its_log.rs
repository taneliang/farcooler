//! A daemon started with its stderr thrown away still keeps what it says
//! (ov-798).
//!
//! `farcooler daemon ensure` starts it with `/dev/null` for stderr and so does
//! launchd, which is how a reported failure used to leave nothing to read. This
//! starts it the same way and reads the log it kept.

use std::process::{Command, Stdio};

/// Start a daemon the way `daemon_link::daemon_command` does, with stderr to
/// nowhere, run until it has said something, and return what the log holds.
fn run_and_read_log(extra_env: &[(&str, &str)]) -> (std::path::PathBuf, String) {
    let home = tempfile::tempdir().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_farcoolerd"));
    command
        .env("FARCOOLER_HOME", home.path())
        .env("FARCOOLER_CONFIG", home.path().join("config.toml"))
        .env("FARCOOLER_TEST_STUB_AGENTS", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for (k, v) in extra_env {
        command.env(k, v);
    }
    let mut child = command.spawn().expect("spawn farcoolerd");

    let log = home.path().join("logs/farcoolerd.log");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let mut text = String::new();
    while std::time::Instant::now() < deadline {
        text = std::fs::read_to_string(&log).unwrap_or_default();
        if !text.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
    if let Ok(install) = std::fs::read_to_string(home.path().join("install-id")) {
        farcooler_tmux::reap_server(&format!("farcooler-{}", install.trim()));
    }
    (log, text)
}

#[test]
fn what_the_daemon_logs_lands_in_a_file_though_stderr_was_discarded() {
    let (log, text) = run_and_read_log(&[]);
    assert!(
        !text.is_empty(),
        "{} stayed empty: the daemon's stderr was discarded and nothing kept it",
        log.display()
    );
}
