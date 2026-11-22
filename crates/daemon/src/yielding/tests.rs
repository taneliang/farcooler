use super::*;
use std::process::{Command, Stdio};
use std::os::unix::process::CommandExt;
use tokio::io::{AsyncBufReadExt, BufReader};

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
async fn a_landing_gate_runs_after_a_builder_releases_its_heavy_ticket() {
    landing_after_heavy_builder(HeavyBuilder::Current).await;
}

#[tokio::test]
async fn a_builder_holding_a_legacy_tmpdir_ticket_stays_runnable_during_landing() {
    landing_after_heavy_builder(HeavyBuilder::Legacy).await;
}

#[tokio::test]
async fn a_legacy_wrapper_launched_after_the_pause_snapshot_does_not_block_landing() {
    landing_after_heavy_builder(HeavyBuilder::LateLegacy).await;
}

enum HeavyBuilder { Current, Legacy, LateLegacy }

async fn landing_after_heavy_builder(builder: HeavyBuilder) {
    const HANG_GUARD: Duration = Duration::from_secs(30);

    let legacy = !matches!(builder, HeavyBuilder::Current);
    let late = matches!(builder, HeavyBuilder::LateLegacy);
    let dir = tempfile::tempdir().unwrap();
    let scripts = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts");
    let old_tmp = dir.path().join("old temp");
    std::fs::create_dir(&old_tmp).unwrap();
    let lock = dir.path().join("lane-gate.lock");
    let driver = r#"
import pathlib, runpy, sys
sys.path.insert(0, sys.argv.pop(1))
import heavy_lock as hl
lock = pathlib.Path(sys.argv.pop(1))
legacy = sys.argv.pop(1) == 'legacy'
if legacy:
    machine_lock = hl.machine_lock
    hl.machine_lock = lambda *a, **kw: machine_lock(*a, path=lock, **kw)
else:
    hl.MACHINE_LOCK_PATH = lock
    discover = hl.legacy_queues
    hl.legacy_queues = lambda queue: [q for q in discover(queue) if lock.parent.resolve() in q.parents]
runpy.run_path(sys.argv.pop(1), run_name='__main__')
"#;
    let command = |old, delayed| {
        let mut cmd = if delayed {
            let mut shell = Command::new("/bin/sh");
            shell.args(["-c", "read -r line && { TMPDIR=\"$1\"; shift; TMPDIR=\"$TMPDIR\" \"$@\"; exit $?; }", "sh"])
                .arg(&old_tmp).arg("python3");
            shell.stdin(Stdio::piped());
            shell
        } else { Command::new("python3") };
        cmd.args(["-c", driver]).arg(&scripts).arg(if old { old_tmp.join("lane-gate.lock") } else { lock.clone() })
            .arg(if old { "legacy" } else { "current" }).arg(scripts.join("heavy"));
        cmd.env_remove("CI").env_remove("HEAVY_LOCK_HELD").env_remove("FARCOOLER_OPERATION_BINDING")
            .env("TMPDIR", if old && !delayed { &old_tmp } else { dir.path() }).env("HEAVY_LOAD_LIMIT", "100000").env("HEAVY_POLL", "0.02")
            .current_dir(dir.path()).process_group(0);
        cmd
    };
    let mut holder = command(legacy, late).args(["--", "/bin/sh", "-c", "touch ready; while [ ! -f release ]; do sleep 0.02; done"])
        .spawn().unwrap();
    let wait_ready = || {
        let deadline = std::time::Instant::now() + HANG_GUARD;
        while !dir.path().join("ready").exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        dir.path().join("ready").exists()
    };
    if !late && !wait_ready() { end(holder); panic!("the builder did not acquire the lock"); }
    let y = Yielding::new(None, dir.path().join(heavy_queue::NAME));
    let _builder = y.builder(group(&holder));
    if late {
        let mut state = y.0.state.lock().unwrap();
        let mut ready = false;
        stop_builders_with_snapshot(&mut state, &y.0.queue, || {
            let raw = crate::proc_table::snapshot()?;
            // Release the shell only after discovery has its process list.
            std::io::Write::write_all(&mut holder.stdin.take().unwrap(), b"start\n").unwrap();
            ready = wait_ready();
            Some(raw)
        });
        let held = state.builders.values().any(|builder| builder.held_since.is_some());
        drop(state);
        if !ready || held {
            end(holder);
            assert!(ready, "the late wrapper did not publish its ticket");
            panic!("the pause sweep left the late ticket holder paused");
        }
    }
    // The forced sweep can resume the late holder while leaving the queue
    // latch held. Normal gate admission settles the pause and releases it.
    let gate = match y.admit_gate().await {
        Ok(gate) => gate,
        Err(error) => {
            end(holder);
            panic!("could not admit the landing gate: {error}");
        }
    };
    let mut landing_command = command(false, false);
    landing_command.env("FARCOOLER_OPERATION_BINDING", "landing")
        .stderr(Stdio::piped()).args(["--", "touch", "landed"]);
    let mut landing = tokio::process::Command::from(landing_command).spawn().unwrap();
    let mut stderr = BufReader::new(landing.stderr.take().unwrap()).lines();
    let mut notes = Vec::new();
    let mut observed_wait = None;
    let landed = dir.path().join("landed");
    let result = tokio::time::timeout(HANG_GUARD, async {
        while let Some(line) = stderr.next_line().await? {
            let waiting_on_holder = line.contains("held by") && line.contains("touch ready");
            notes.push(line);
            // The wrapper flushes this message from inside the lock wait.
            if waiting_on_holder && observed_wait.is_none() {
                let was_waiting = !landed.exists()
                    && !crate::proc_table::stopped_or_gone(holder.id() as i32);
                observed_wait = Some((was_waiting, states(&holder)));
                std::fs::write(dir.path().join("release"), "")?;
            }
        }
        landing.wait().await
    }).await;
    if let Some(pid) = landing.id() {
        let _ = crate::effect_runner::Group::new(pid).kill();
        let _ = landing.wait().await;
    }
    let (was_waiting, holder_state) = observed_wait.unwrap_or_else(|| (false, states(&holder)));
    drop(gate);
    end(holder);
    assert!(was_waiting && matches!(result, Ok(Ok(status)) if status.success()) && landed.exists(),
        "landing must wait on a runnable builder and finish after release.\nBuilder states {holder_state:?}\nLanding result {result:?}\n{}",
        notes.join("\n"));
}

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

/// Confirm that gate admission waits for the hold and that its signals only affect the builder.
#[tokio::test]
async fn a_native_gate_pauses_only_the_builder_and_preserves_a_manual_pause() {
    let dir = farcooler_core::scratch::dir("native-gate-identity");
    let id = uuid::Uuid::now_v7();
    let tmux = farcooler_tmux::TmuxServer::in_dir(&format!("gate-identity-{id}"), id, &dir);
    struct Reap<'a>(&'a farcooler_tmux::TmuxServer);
    impl Drop for Reap<'_> { fn drop(&mut self) { farcooler_tmux::reap_server(self.0.socket()); } }
    let _reap = Reap(&tmux);
    let mut panes = Vec::new();
    let binary = crate::native_hold::test_binary::daemon_binary().unwrap();
    let socket = dir.join("first.sock");
    for file in ["first", "other"] {
        let terminal = uuid::Uuid::now_v7();
        let command = crate::native_hold::pane_command(&binary, &dir.join(format!("{file}.sock")),
            &format!("while :; do echo tick >> {file}; sleep 0.02; done"));
        let window = tmux.create_terminal_window(terminal, terminal, "gate identity", dir.to_str().unwrap(), &command).await.unwrap();
        panes.push((window.pane_id, terminal));
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        while !dir.join("first").exists() || !dir.join("other").exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap();
    let y = Yielding::at(&dir);
    let group = crate::native_hold::group(&socket).unwrap().unwrap();
    std::fs::create_dir_all(&y.0.queue).unwrap();
    let ticket = y.0.queue.join(format!("1-{group}.ticket"));
    std::fs::write(&ticket, "").unwrap();
    let target = Target::Hold(socket.clone());
    let builder = y.builder(target.clone());
    assert!(manual_pause_latch(Some(&builder), &socket).is_err());
    let gate = y.admit_gate().await.unwrap();
    assert!(matches!(crate::proc_table::process_state(group), crate::proc_table::ProcessState::Running),
        "the native builder must be able to release its heavy ticket");
    drop(gate);
    std::fs::remove_file(ticket).unwrap();

    let blocker = std::os::unix::net::UnixStream::connect(&socket).unwrap();
    let admission = y.admit_gate();
    tokio::pin!(admission);
    tokio::select! {
        biased;
        result = &mut admission => panic!("gate decided before the hold could answer: {}", result.is_ok()),
        _ = tokio::time::sleep(Duration::from_millis(100)) => {}
    }
    drop(blocker);
    let gate = admission.await.unwrap();
    let recorded: Vec<Target> = serde_json::from_slice(&std::fs::read(dir.join("yielding.json")).unwrap()).unwrap();
    assert_eq!(recorded, vec![target]);
    let paused = std::fs::read(dir.join("first")).unwrap();
    let other = std::fs::read(dir.join("other")).unwrap();
    tokio::time::sleep(Duration::from_millis(700)).await; // also covers the gate's periodic sweep
    assert_eq!(std::fs::read(dir.join("first")).unwrap(), paused);
    assert!(std::fs::read(dir.join("other")).unwrap().len() > other.len());
    for (pane, terminal) in &panes {
        assert_eq!(tmux.pane_life(pane, *terminal).await.unwrap(), farcooler_tmux::PaneLife::Live);
    }
    builder.manual(true);
    drop(gate);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(std::fs::read(dir.join("first")).unwrap(), paused, "a hand pause survives the gate");
    builder.manual(false);
    tokio::time::timeout(Duration::from_secs(5), async {
        while std::fs::read(dir.join("first")).unwrap() == paused {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap();
}
