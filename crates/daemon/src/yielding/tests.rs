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
    Target::Group { pid: child.id(), started: crate::effect_runner::Owner::started_at(child.id()).unwrap() }
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
    assert!(y.admit_gate().await.is_err(), "a gate cannot start without a durable pause");
    std::thread::sleep(Duration::from_millis(100));
    assert!(running(&child), "no record, no stop: {:?}", states(&child));
    assert!(!y.paused());
    read_only(dir.path(), false);
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

fn settle() { std::thread::sleep(Duration::from_millis(200)); }
/// Stopped by hand, as a native pause does, then recorded.
fn pause_by_hand(guard: &BuilderGuard, child: &std::process::Child) {
    guard.manual(true);
    unsafe { libc::killpg(child.id() as i32, libc::SIGSTOP); }
}

#[tokio::test]
async fn a_manually_paused_builder_stays_paused_when_a_gate_ends() {
    let y = Yielding::default();
    let child = builder();
    let guard = y.builder(group(&child));
    let gate = y.gate();
    pause_by_hand(&guard, &child);
    drop(gate);
    settle();
    assert!(stopped(&child), "the orchestrator still holds it: {:?}", states(&child));
    guard.manual(false);
    settle();
    assert!(running(&child), "no gate and no hand pause: it runs: {:?}", states(&child));
    end(child);
}

#[tokio::test]
async fn a_builder_paused_by_gate_and_hand_resumes_only_after_both_are_lifted_in_either_order() {
    let y = Yielding::default();
    let child = builder();
    let guard = y.builder(group(&child));
    // The gate ends first.
    let gate = y.gate();
    pause_by_hand(&guard, &child);
    drop(gate);
    settle();
    assert!(stopped(&child), "the hand still holds it: {:?}", states(&child));
    guard.manual(false);
    settle();
    assert!(running(&child), "both lifted: {:?}", states(&child));
    // The hand resume comes first.
    let gate = y.gate();
    pause_by_hand(&guard, &child);
    guard.manual(false);
    settle();
    assert!(stopped(&child), "the gate still holds it: {:?}", states(&child));
    drop(gate);
    settle();
    assert!(running(&child), "both lifted: {:?}", states(&child));
    end(child);
}

/// A gate's hold is counted per builder, so its step's limit and quiet periods can leave it out (ov-1301); time it
/// is also paused by hand is not counted twice, since a hand pause stops those clocks itself.
#[tokio::test]
async fn a_builder_counts_the_time_a_gate_held_it_and_not_its_hand_pause() {
    let y = Yielding::default();
    let child = builder();
    let guard = y.builder(group(&child));
    assert_eq!(guard.held(), (Duration::ZERO, false), "no gate yet");
    let gate = y.gate();
    std::thread::sleep(Duration::from_millis(300));
    let (held, holding) = guard.held();
    assert!(holding && held >= Duration::from_millis(300), "a gate holds it: {held:?}");
    pause_by_hand(&guard, &child);
    let (by_gate, holding) = guard.held();
    std::thread::sleep(Duration::from_millis(300));
    assert!(!holding && guard.held().0 == by_gate, "a hand pause's time is the hand's: {:?}", guard.held());
    guard.manual(false);
    drop(gate);
    let (held, holding) = guard.held();
    assert!(!holding && held >= Duration::from_millis(300) && held < Duration::from_millis(550), "{held:?}");
    assert_eq!(guard.held_clock().held(), held, "the clock's handle reads the same");
    end(child);
}

/// A tty number alone can be reused by another run. Both the shell's start identity and its presence on that tty
/// must agree before a gate signals anything, and an unrelated pane keeps working throughout a valid pause.
#[tokio::test]
async fn a_native_gate_leaves_other_panes_and_reused_ttys_alone() {
    let dir = farcooler_core::scratch::dir("native-gate-identity");
    let id = uuid::Uuid::now_v7();
    let tmux = farcooler_tmux::TmuxServer::in_dir(&format!("gate-identity-{id}"), id, &dir);
    struct Reap<'a>(&'a farcooler_tmux::TmuxServer);
    impl Drop for Reap<'_> { fn drop(&mut self) { farcooler_tmux::reap_server(self.0.socket()); } }
    let _reap = Reap(&tmux);
    let mut panes = Vec::new();
    for file in ["first", "other"] {
        let terminal = uuid::Uuid::now_v7();
        let command = format!("exec /bin/sh -c 'sh -c \"while :; do echo tick >> {file}; sleep 0.02; done\"; status=$?; exit \"$status\"'");
        let window = tmux.create_terminal_window(terminal, terminal, "gate identity", dir.to_str().unwrap(), &command).await.unwrap();
        panes.push(tmux.list_tagged_panes().await.unwrap().into_iter().find(|p| p.pane_id == window.pane_id).unwrap());
    }
    for _ in 0..500 {
        if dir.join("first").exists() && dir.join("other").exists() { break; }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let first = &panes[0];
    let target = Target::Native { tty: first.tty.clone(), supervisor: first.stamp.pid,
        started: crate::effect_runner::Owner::started_at(first.stamp.pid).unwrap() };
    let y = Yielding::at(&dir);
    let mut wrong_tty = target.clone();
    if let Target::Native { tty, .. } = &mut wrong_tty { *tty = panes[1].tty.clone(); }
    let mut recycled = target.clone();
    if let Target::Native { started, .. } = &mut recycled { *started = "old shell".into(); }
    let invalid = (y.builder(wrong_tty), y.builder(recycled));
    let gate = y.gate();
    let before = (std::fs::read(dir.join("first")).unwrap().len(), std::fs::read(dir.join("other")).unwrap().len());
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(std::fs::read(dir.join("first")).unwrap().len() > before.0);
    assert!(std::fs::read(dir.join("other")).unwrap().len() > before.1);
    drop((gate, invalid));
    let builder = y.builder(target.clone());
    let gate = y.gate();
    let recorded: Vec<Target> = serde_json::from_slice(&std::fs::read(dir.join("yielding.json")).unwrap()).unwrap();
    assert_eq!(recorded, vec![target], "the restart record retains the supervisor identity");
    tokio::time::sleep(Duration::from_millis(100)).await;
    let paused = std::fs::read(dir.join("first")).unwrap();
    let other = std::fs::read(dir.join("other")).unwrap();
    tokio::time::sleep(Duration::from_millis(700)).await; // also covers the gate's periodic sweep
    assert_eq!(std::fs::read(dir.join("first")).unwrap(), paused);
    assert!(std::fs::read(dir.join("other")).unwrap().len() > other.len());
    for pane in &panes { assert_eq!(tmux.pane_life(&pane.pane_id, pane.terminal_id).await.unwrap(), farcooler_tmux::PaneLife::Live); }
    builder.manual(true);
    drop(gate);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(std::fs::read(dir.join("first")).unwrap(), paused, "a hand pause survives the gate");
    builder.manual(false);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(std::fs::read(dir.join("first")).unwrap().len() > paused.len(), "the same pane resumes");
}

/// Fish's interactive command exits successfully if its foreground job is still stopped
/// when fish handles SIGCHLD. Widen the gap between signals to make the gate's race deterministic.
#[tokio::test]
async fn a_fish_builder_survives_a_gate_with_delayed_signals() {
    use crate::service::shell_quote;
    use farcooler_tmux::PaneLife;

    let Some(fish) = farcooler_core::programs::find("fish") else {
        eprintln!("fish is unavailable");
        return;
    };
    let dir = farcooler_core::scratch::dir("gate-fish");
    let id = uuid::Uuid::now_v7();
    let tmux = farcooler_tmux::TmuxServer::in_dir(&format!("gate-fish-{id}"), id, &dir);
    struct Reap<'a>(&'a farcooler_tmux::TmuxServer);
    impl Drop for Reap<'_> { fn drop(&mut self) { farcooler_tmux::reap_server(self.0.socket()); } }
    let _reap = Reap(&tmux);
    let child = "/bin/sh -c 'echo $$ > child; exec /bin/sleep 30'";
    let login = format!("/usr/bin/nice -n 10 {} --no-config -ilc {}; status=$?; exit \"$status\"",
        shell_quote(fish.to_str().unwrap()), shell_quote(child));
    let launch = format!("exec /bin/sh -c {}", shell_quote(&login));
    let window = tmux.create_terminal_window(id, id, "fish gate", dir.to_str().unwrap(), &launch).await.unwrap();
    let marker = dir.join("child");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let child: i32 = loop {
        if let Ok(pid) = std::fs::read_to_string(&marker) {
            if let Ok(pid) = pid.trim().parse() { break pid; }
        }
        assert!(tokio::time::Instant::now() < deadline, "fish never started its foreground job");
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let pane = tmux.list_tagged_panes().await.unwrap().into_iter().find(|p| p.pane_id == window.pane_id).unwrap();
    let supervisor = pane.stamp.pid as i32;
    let ppid = Command::new("/bin/ps").args(["-o", "ppid=", "-p", &child.to_string()]).output().unwrap();
    let shell: i32 = String::from_utf8(ppid.stdout).unwrap().trim().parse().unwrap();
    assert_ne!(shell, supervisor, "the login shell must be between the PTY supervisor and its job");
    let raw = crate::proc_table::snapshot().unwrap();
    let table_in_order = |first: i32, second: i32| {
        let mut rows: Vec<_> = raw.lines().collect();
        rows.sort_by_key(|row| match row.split_whitespace().next().and_then(|pid| pid.parse::<i32>().ok()) {
            Some(pid) if pid == first => 0,
            Some(pid) if pid == second => 1,
            _ => 2,
        });
        crate::foreground::parse(&rows.join("\n"))
    };
    let send = |pid, signal| {
        unsafe { libc::kill(pid, signal); }
        std::thread::sleep(Duration::from_millis(200));
    };
    // Begin with a quiet pause so the red run isolates the clean exit on gate release.
    send(shell, libc::SIGSTOP);
    send(child, libc::SIGSTOP);
    signal_native(&table_in_order(shell, child), &pane.tty, supervisor, libc::SIGCONT, send);
    let life = tmux.pane_life_with_exit_evidence(&pane.pane_id, id).await.unwrap();
    eprintln!("fish pane after delayed resume: {life:?}");
    assert_eq!(life, PaneLife::Live, "resuming fish before its stopped job loses the pane");
    // The process table can also list the job first while stopping. The login shell
    // must be stopped before that job even if this is the order of the snapshot.
    signal_native(&table_in_order(child, shell), &pane.tty, supervisor, libc::SIGSTOP, send);
    let life = tmux.pane_life_with_exit_evidence(&pane.pane_id, id).await.unwrap();
    eprintln!("fish pane after delayed stop: {life:?}");
    assert_eq!(life, PaneLife::Live, "stopping the job before fish loses the pane");
    signal_native(&table_in_order(shell, child), &pane.tty, supervisor, libc::SIGCONT, send);
}
