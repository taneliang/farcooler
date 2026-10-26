//! The log owns file descriptor 2 (ov-798): a raw write to stderr lands in it,
//! it follows a rotation, a flood nobody routed through the writer is still
//! bounded, and the modes that are not a daemon make no log at all.
//!
//! Redirecting fd 2 would take over the test runner's own, so each case runs in
//! a child: this very test binary, re-run on `child`, told what to do by env.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use farcooler_daemon::daemon_log::{RotatingLog, FILE_NAME};

fn raw_stderr(text: &str) {
    let mut err = std::io::stderr();
    err.write_all(text.as_bytes()).unwrap();
}

/// The child half. Does nothing unless the parent set `LOG_CASE`.
#[test]
fn child() {
    let Ok(case) = std::env::var("LOG_CASE") else { return };
    let dir = std::env::var("LOG_DIR").unwrap();
    let sweep = (case == "flood").then(|| std::time::Duration::from_millis(50));
    let log = RotatingLog::open(Path::new(&dir), 200, 3, true, sweep).unwrap();
    match case.as_str() {
        "raw" => raw_stderr("RAW-LINE\n"),
        "rotation" => {
            for n in 0..20 {
                writeln!(&log, "padding line {n:03} padding padding padding").unwrap();
            }
            raw_stderr("AFTER-ROTATION\n");
        }
        "flood" => {
            for _ in 0..50 {
                raw_stderr(&"x".repeat(99));
                raw_stderr("\n");
            }
            std::thread::sleep(std::time::Duration::from_millis(600));
        }
        other => panic!("unknown case {other}"),
    }
    drop(log);
}

fn run_case(case: &str) -> std::path::PathBuf {
    let dir = tempfile::tempdir().unwrap().keep().join("logs");
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["child", "--exact", "--nocapture", "--test-threads=1"])
        .env("LOG_CASE", case)
        .env("LOG_DIR", &dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    dir
}

#[test]
fn a_raw_write_to_stderr_lands_in_the_log() {
    let dir = run_case("raw");
    let text = std::fs::read_to_string(dir.join(FILE_NAME)).unwrap();
    assert!(text.contains("RAW-LINE"), "{text:?}");
}

#[test]
fn after_a_rotation_stderr_points_at_the_new_file() {
    let dir = run_case("rotation");
    assert!(dir.join("farcoolerd.log.1").exists(), "nothing rotated");
    let current = std::fs::read_to_string(dir.join(FILE_NAME)).unwrap();
    assert!(current.contains("AFTER-ROTATION"), "{current:?}");
    let older = std::fs::read_to_string(dir.join("farcoolerd.log.1")).unwrap();
    assert!(!older.contains("AFTER-ROTATION"));
}

#[test]
fn a_flood_to_stderr_alone_is_rotated_by_the_timer() {
    let dir = run_case("flood");
    assert!(dir.join("farcoolerd.log.1").exists(), "the flood grew one file unseen");
    let len = std::fs::metadata(dir.join(FILE_NAME)).unwrap().len();
    assert!(len <= 300, "the current file is {len} bytes");
}

#[test]
fn the_modes_that_are_not_a_daemon_make_no_log_folder() {
    for args in [vec!["--version"], vec!["--stdio"], vec!["--stream", "t"], vec!["--fanout", "%1"]] {
        let home = tempfile::tempdir().unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_farcoolerd"))
            .args(&args)
            .env("FARCOOLER_HOME", home.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let _ = child.kill();
        let _ = child.wait();
        assert!(!home.path().join("logs").exists(), "{args:?} made a log folder");
    }
}
