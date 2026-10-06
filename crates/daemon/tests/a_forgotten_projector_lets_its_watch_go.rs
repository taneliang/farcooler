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

    let (held, holding) = std::sync::mpsc::channel();
    let holder = {
        let projectors = projectors.clone();
        std::thread::spawn(move || {
            projectors.with_session(terminal, |_| {
                held.send(()).unwrap();
                std::thread::sleep(Duration::from_millis(700));
            })
        })
    };
    holding.recv().unwrap();
    // Another session's transcript in the same project directory: the
    // directory is watched whole, so its writes are events here too.
    std::fs::write(dir.path().join("other.jsonl"), format!("{line}\n")).unwrap();
    // Long enough for the event to arrive and wait on the held lock.
    std::thread::sleep(Duration::from_millis(250));
    let before = cpu();
    let started = Instant::now();
    projectors.forget(terminal);
    let forgetting = started.elapsed();
    holder.join().unwrap();
    // The watch is free for the next projector at once: nothing in it waits
    // on the event the forgotten one left behind.
    let other = tempfile::Builder::new().prefix("fc-forget-next").tempdir().unwrap();
    let next = other.path().join("n.jsonl");
    std::fs::write(&next, format!("{line}\n")).unwrap();
    let started = Instant::now();
    projectors.open(uuid::Uuid::now_v7(), next);
    let reopening = started.elapsed();
    std::thread::sleep(Duration::from_millis(2500));
    let spent = cpu() - before;
    eprintln!("forget took {forgetting:?}; the next open {reopening:?}; {spent:?} of CPU in the 2.5 s after");
    assert!(forgetting < Duration::from_millis(300), "forget waited on the event: {forgetting:?}");
    assert!(reopening < Duration::from_millis(1500), "the next open waited on the watch: {reopening:?}");
    assert!(spent < Duration::from_millis(500), "{spent:?} of CPU in 2.5 s idle");
    drop(projectors);
}
