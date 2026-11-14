use super::*;
use std::process::{Command, Stdio};
use std::os::unix::process::CommandExt;

/// A builder: a shell with a compiler-like child, in a group of its own.
fn builder() -> std::process::Child {
    let child = Command::new("/bin/sh").args(["-c", "sleep 60 & sleep 60"]).stdout(Stdio::null()).process_group(0).spawn().unwrap();
    // Until the shell has forked both sleeps, a stop could miss them.
    while states(&child).len() < 3 { std::thread::sleep(Duration::from_millis(10)); }
    child
}
fn group(child: &std::process::Child) -> Target {
    Target::Group { pid: child.id(), started: crate::effect_runner::Owner::started_at(child.id()) }
}
/// The state letters of every process in the group.
fn states(child: &std::process::Child) -> Vec<String> {
    let out = Command::new("/bin/ps").args(["-axo", "pgid=,stat="]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| {
        let (g, s) = l.trim().split_once(char::is_whitespace)?;
        (g.trim() == child.id().to_string()).then(|| s.trim().to_string())
    }).collect()
}
fn stopped(child: &std::process::Child) -> bool { let s = states(child); s.len() >= 3 && s.iter().all(|s| s.starts_with('T')) }
fn running(child: &std::process::Child) -> bool { let s = states(child); s.len() >= 3 && s.iter().all(|s| !s.starts_with('T')) }
fn end(mut child: std::process::Child) { unsafe { libc::killpg(child.id() as i32, libc::SIGKILL); } let _ = child.wait(); }

#[tokio::test]
async fn a_running_gate_stops_a_builder_and_its_children_and_the_end_of_it_resumes_them() {
    let y = Yielding::default();
    let child = builder();
    let _guard = y.builder(group(&child));
    std::thread::sleep(Duration::from_millis(200));
    assert!(running(&child), "no gate: the builder runs: {:?}", states(&child));
    let gate = y.gate();
    assert!(y.paused() && stopped(&child), "a gate runs: builder and children stop: {:?}", states(&child));
    drop(gate);
    std::thread::sleep(Duration::from_millis(200));
    assert!(!y.paused() && running(&child), "the gate ended: they resume: {:?}", states(&child));
    end(child);
}

#[tokio::test]
async fn builders_resume_only_when_the_last_of_several_gates_ends() {
    let y = Yielding::default();
    let child = builder();
    let _guard = y.builder(group(&child));
    let (first, second) = (y.gate(), y.gate());
    drop(first);
    assert!(stopped(&child), "one gate still runs");
    drop(second);
    std::thread::sleep(Duration::from_millis(200));
    assert!(running(&child), "no gate runs");
    end(child);
}

#[tokio::test]
async fn a_builder_that_starts_during_a_gate_is_stopped_at_once() {
    let y = Yielding::default();
    let gate = y.gate();
    let child = builder();
    let _guard = y.builder(group(&child));
    assert!(stopped(&child), "{:?}", states(&child));
    drop(gate);
    std::thread::sleep(Duration::from_millis(200));
    assert!(running(&child));
    end(child);
}

#[tokio::test]
async fn a_builder_that_ends_is_continued_and_never_signalled_again() {
    let y = Yielding::default();
    let child = builder();
    let gate = y.gate();
    let guard = y.builder(group(&child));
    assert!(stopped(&child));
    drop(guard); // as when a cancelled build is killed: continued, then forgotten
    std::thread::sleep(Duration::from_millis(200));
    assert!(running(&child), "{:?}", states(&child));
    drop(gate);
    end(child);
}

#[tokio::test]
async fn a_daemon_that_died_during_a_gate_continues_what_it_stopped_when_it_starts_again() {
    let dir = tempfile::tempdir().unwrap();
    let child = builder();
    let y = Yielding::at(dir.path());
    let guard = y.builder(group(&child));
    let gate = y.gate();
    assert!(stopped(&child));
    assert!(dir.path().join("yielding.json").exists(), "the pause is written down");
    // The daemon dies: nothing runs its drops.
    std::mem::forget(guard); std::mem::forget(gate);
    let _again = Yielding::at(dir.path());
    std::thread::sleep(Duration::from_millis(200));
    assert!(running(&child), "{:?}", states(&child));
    assert!(!dir.path().join("yielding.json").exists());
    end(child);
}

#[tokio::test]
async fn a_pid_that_started_at_another_time_is_not_stopped() {
    let y = Yielding::default();
    let child = builder();
    let _guard = y.builder(Target::Group { pid: child.id(), started: "Mon Jan  1 00:00:00 2001".into() });
    let _gate = y.gate();
    std::thread::sleep(Duration::from_millis(200));
    assert!(running(&child), "someone else's process is left alone");
    end(child);
}

fn read_only(dir: &Path, on: bool) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(if on { 0o555 } else { 0o755 })).unwrap();
}

#[tokio::test]
async fn a_first_pause_that_cannot_be_recorded_stops_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let y = Yielding::at(dir.path());
    let child = builder();
    let _guard = y.builder(group(&child));
    read_only(dir.path(), true);
    let gate = y.gate();
    std::thread::sleep(Duration::from_millis(100));
    assert!(running(&child), "no record, no stop: {:?}", states(&child));
    assert!(!y.paused());
    read_only(dir.path(), false);
    drop(gate);
    end(child);
}

#[tokio::test]
async fn an_update_that_cannot_be_recorded_continues_what_was_stopped_and_keeps_the_old_record() {
    let dir = tempfile::tempdir().unwrap();
    let y = Yielding::at(dir.path());
    let (first, second) = (builder(), builder());
    let g1 = y.builder(group(&first));
    let gate = y.gate();
    assert!(stopped(&first));
    let old = std::fs::read(dir.path().join("yielding.json")).unwrap();
    read_only(dir.path(), true);
    let g2 = y.builder(group(&second)); // the update cannot be written
    std::thread::sleep(Duration::from_millis(100));
    assert!(running(&first) && running(&second), "{:?} {:?}", states(&first), states(&second));
    assert_eq!(std::fs::read(dir.path().join("yielding.json")).unwrap(), old, "the valid record is retained");
    read_only(dir.path(), false);
    drop((g1, g2, gate));
    end(first); end(second);
}

#[tokio::test]
async fn a_restart_after_an_update_resumes_every_stopped_builder() {
    let dir = tempfile::tempdir().unwrap();
    let y = Yielding::at(dir.path());
    let (first, second) = (builder(), builder());
    let g1 = y.builder(group(&first));
    let gate = y.gate();
    let g2 = y.builder(group(&second));
    assert!(stopped(&first) && stopped(&second));
    // A half-written update that died before its rename is left beside the record.
    std::fs::write(dir.path().join("yielding.json.tmp"), b"[{\"Gro").unwrap();
    std::mem::forget((g1, g2, gate));
    let _again = Yielding::at(dir.path());
    std::thread::sleep(Duration::from_millis(200));
    assert!(running(&first) && running(&second), "{:?} {:?}", states(&first), states(&second));
    end(first); end(second);
}

#[tokio::test]
async fn an_unreadable_record_is_kept_not_deleted() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("yielding.json"), b"[{\"Gro").unwrap();
    let _y = Yielding::at(dir.path());
    assert!(dir.path().join("yielding.json").exists());
}
