//! A projector forgotten while a watch event for it is waiting lets the watch
//! go without spinning (ov-366 review 1, finding 1).
//!
//! The reviewer's scenario: the projector's lock is held (a hook, a page), a
//! line is appended so an event waits on that lock, and the terminal is
//! forgotten meanwhile. With a watcher per projector whose callback held the
//! projector, the callback dropped the last reference when the lock came
//! free, the watcher went on its own run-loop thread, and FSEvents' `stop()`
//! spun there for good: 1.96 s of CPU in every 2 s.
//!
//! Its own test binary, so the CPU it measures is this process's alone.

use std::time::{Duration, Instant};

use farcooler_daemon::session_projectors::SessionProjectors;

/// This process's CPU time, user and system.
fn cpu() -> Duration {
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: `usage` is a valid, writable rusage for getrusage to fill.
    assert_eq!(unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) }, 0);
    let time = |t: libc::timeval| Duration::from_secs(t.tv_sec as u64) + Duration::from_micros(t.tv_usec as u64);
    time(usage.ru_utime) + time(usage.ru_stime)
}

/// How long a step may take before it counts as waiting forever. Waits
/// return the moment the step is done; this bounds a machine under load, and
/// is not what the test measures (ov-547).
const FOREVER: Duration = Duration::from_secs(60);

#[test]
fn forgetting_a_projector_with_an_event_waiting_spins_nothing() {
    let dir = tempfile::Builder::new().prefix("fc-forget").tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    let line = r#"{"type":"user","promptId":"p1","promptSource":"typed","uuid":"u1","message":{"content":"hi"}}"#;
    std::fs::write(&path, format!("{line}\n")).unwrap();
    let projectors = std::sync::Arc::new(SessionProjectors::default());
    let terminal = uuid::Uuid::now_v7();
    projectors.open(terminal, path.clone());
    std::thread::sleep(Duration::from_millis(300));

    // The lock is held until `forget` has returned, not for a fixed time: a
    // forget that waits on the event waiting on this lock then never returns
    // while it is held, however slow or fast the machine. (It was a 700 ms
    // hold and a 300 ms bound on the forget, and a loaded machine took
    // longer than that over a forget that waited on nothing.)
    let (held, holding) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel::<()>();
    let holder = {
        let projectors = projectors.clone();
        std::thread::spawn(move || {
            projectors.with_session(terminal, |_| {
                held.send(()).unwrap();
                // Until told, or until the test ends and drops the sender.
                let _ = released.recv();
            })
        })
    };
    holding.recv().unwrap();
    // Another session's transcript in the same project directory: the
    // directory is watched whole, so its writes are events here too.
    std::fs::write(dir.path().join("other.jsonl"), format!("{line}\n")).unwrap();
    // Until the event has arrived and waits on the held lock: a forget
    // before it would have nothing to wait on, and pass whatever it does.
    let arriving = Instant::now();
    while projectors.events_waiting() == 0 {
        assert!(arriving.elapsed() < FOREVER, "the write never reached the projector");
        std::thread::sleep(Duration::from_millis(10));
    }
    let before = cpu();
    let forgot = within(FOREVER, {
        let projectors = projectors.clone();
        move || projectors.forget(terminal)
    });
    let _ = release.send(());
    holder.join().unwrap();
    assert!(forgot, "forget waited on the event, which waits on the held lock");
    // The watch is free for the next projector: nothing in it waits on the
    // event the forgotten one left behind. Taking a directory off the watch
    // restarts it, about half a second at rest, so this asks only that the
    // open comes back at all.
    let other = tempfile::Builder::new().prefix("fc-forget-next").tempdir().unwrap();
    let next = other.path().join("n.jsonl");
    std::fs::write(&next, format!("{line}\n")).unwrap();
    let reopened = within(FOREVER, {
        let projectors = projectors.clone();
        move || projectors.open(uuid::Uuid::now_v7(), next)
    });
    assert!(reopened, "the next open waited on the watch");
    // The bound here is the test's subject, and load cannot break it: an
    // idle process spends next to no CPU however busy the machine is, and
    // the spin this guards cost 1.96 s in every 2.
    let idle = Instant::now();
    std::thread::sleep(Duration::from_millis(2500));
    let spent = cpu() - before;
    eprintln!("{spent:?} of CPU from the forget to {:?} after the next open", idle.elapsed());
    assert!(spent < Duration::from_millis(500), "{spent:?} of CPU in 2.5 s idle");
    drop(projectors);
}

/// Run `step` on a thread of its own and say whether it finished within
/// `limit`. A step that does not is left running: the test is failing.
fn within(limit: Duration, step: impl FnOnce() + Send + 'static) -> bool {
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        step();
        let _ = done.send(());
    });
    finished.recv_timeout(limit).is_ok()
}
