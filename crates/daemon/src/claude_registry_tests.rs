//! The registry's stale-file rules, against processes that are made up.

use std::collections::HashMap;
use std::path::PathBuf;

use super::*;

/// A process table: pid to (start, tty).
#[derive(Default, Clone)]
struct Table(HashMap<i32, (i64, Option<u64>)>);

impl Processes for Table {
    fn started(&self, pid: i32) -> Option<i64> {
        self.0.get(&pid).map(|p| p.0)
    }
    fn tty(&self, pid: i32) -> Option<u64> {
        self.0.get(&pid).and_then(|p| p.1)
    }
}

/// `Sun Oct  4 18:06:13 2026` UTC.
const STARTED: i64 = 1_791_137_173;

fn file(pid: i32, session: &str, started: &str) -> String {
    format!(
        r#"{{"pid":{pid},"sessionId":"{session}","cwd":"/tmp/fc-t/proj","startedAt":1,"procStart":"{started}","version":"2.1.290","kind":"interactive","entrypoint":"cli","pidDomain":"darwin","tmux":"farcooler:@1.%7","messagingSocketPath":"/tmp/cc-socks/{pid}.sock","status":"busy"}}"#
    )
}

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        let dir = std::env::temp_dir().join(format!("fc-registry-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sessions")).unwrap();
        Dir(dir)
    }
    fn write(&self, pid: i32, text: &str) {
        std::fs::write(self.0.join("sessions").join(format!("{pid}.json")), text).unwrap();
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_registry_file_reads_as_its_session_status_and_pane() {
    let e = parse(file(29434, "s-1", "Sun Oct  4 18:06:13 2026").as_bytes()).unwrap();
    assert_eq!(e.pid, 29434);
    assert_eq!(e.session_id, "s-1");
    assert_eq!(e.status, Some(Activity::Busy));
    assert_eq!(e.tmux, Some(TmuxPlace { session: "farcooler".into(), window: "@1".into(), pane: "%7".into() }));
    assert_eq!(e.started, Some(STARTED));
    assert_eq!(e.messaging_socket, Some(PathBuf::from("/tmp/cc-socks/29434.sock")));
    let shell = file(1, "s", "Sun Oct  4 18:06:13 2026").replace("\"busy\"", "\"shell\"");
    assert_eq!(parse(shell.as_bytes()).unwrap().status, Some(Activity::Shell), "seen live; not only busy and idle");
}

#[test]
fn proc_start_is_utc_and_strict() {
    assert_eq!(parse_proc_start("Sun Oct  4 18:06:13 2026"), Some(STARTED));
    assert_eq!(parse_proc_start("Thu Jan  1 00:00:00 1970"), Some(0));
    assert_eq!(parse_proc_start("Wed Feb 29 12:00:00 2028"), Some(1_835_438_400), "a leap day");
    for bad in ["", "Sun Oct 32 18:06:13 2026", "Sun Oct  4 24:06:13 2026", "Sun Okt  4 18:06:13 2026", "Sun Oct  4 18:06 2026", "Sun Oct  4 18:06:13 2026 UTC"] {
        assert_eq!(parse_proc_start(bad), None, "{bad:?}");
    }
}

#[test]
fn a_live_process_with_its_start_time_is_found_by_pid_and_by_session() {
    let dir = Dir::new("live");
    dir.write(29434, &file(29434, "s-1", "Sun Oct  4 18:06:13 2026"));
    let table = Table([(29434, (STARTED, None))].into());
    let registry = Registry::new(dir.0.clone(), Box::new(table));
    assert_eq!(registry.by_pid(29434).map(|e| e.session_id), Some("s-1".into()));
    assert_eq!(registry.by_session("s-1").map(|e| e.pid), Some(29434));
}

#[test]
fn a_stale_file_for_a_dead_pid_is_nobody() {
    let dir = Dir::new("dead");
    dir.write(29434, &file(29434, "s-1", "Sun Oct  4 18:06:13 2026"));
    let registry = Registry::new(dir.0.clone(), Box::new(Table::default()));
    assert_eq!(registry.by_pid(29434), None);
    assert_eq!(registry.by_session("s-1"), None);
}

#[test]
fn a_reused_pid_is_not_the_claude_that_wrote_the_file() {
    let dir = Dir::new("reused");
    dir.write(29434, &file(29434, "s-1", "Sun Oct  4 18:06:13 2026"));
    // Some other process has pid 29434 now, started an hour later.
    let table = Table([(29434, (STARTED + 3600, None))].into());
    let registry = Registry::new(dir.0.clone(), Box::new(table));
    assert_eq!(registry.by_pid(29434), None);
}

#[test]
fn a_start_within_a_second_is_the_same_process() {
    // procStart has whole seconds; the kernel's start may round the other way.
    let entry = parse(file(5, "s", "Sun Oct  4 18:06:13 2026").as_bytes()).unwrap();
    assert!(is_live(&entry, &Table([(5, (STARTED + 1, None))].into())));
    assert!(!is_live(&entry, &Table([(5, (STARTED + 2, None))].into())));
}

#[test]
fn an_unreadable_proc_start_is_refused_rather_than_trusted() {
    let entry = parse(file(5, "s", "sometime").as_bytes()).unwrap();
    assert!(!is_live(&entry, &Table([(5, (STARTED, None))].into())));
}

#[test]
fn a_missing_file_or_directory_is_nothing_and_a_file_written_later_is_found() {
    let dir = Dir::new("missing");
    std::fs::remove_dir_all(dir.0.join("sessions")).unwrap();
    let table = Table([(29434, (STARTED, None))].into());
    let registry = Registry::new(dir.0.clone(), Box::new(table));
    assert_eq!(registry.by_pid(29434), None, "no directory yet");
    std::fs::create_dir_all(dir.0.join("sessions")).unwrap();
    dir.write(29434, &file(29434, "s-2", "Sun Oct  4 18:06:13 2026"));
    assert_eq!(registry.by_pid(29434).map(|e| e.session_id), Some("s-2".into()), "found once written");
}

#[test]
fn a_rewritten_file_is_read_again_after_the_watch_fires() {
    let dir = Dir::new("rewrite");
    dir.write(29434, &file(29434, "s-1", "Sun Oct  4 18:06:13 2026"));
    let table = Table([(29434, (STARTED, None))].into());
    let registry = Registry::new(dir.0.clone(), Box::new(table));
    assert_eq!(registry.by_pid(29434).map(|e| e.session_id), Some("s-1".into()));
    // `/clear`: claude rewrites its file with the new session.
    dir.write(29434, &file(29434, "s-2", "Sun Oct  4 18:06:13 2026"));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut seen = None;
    while std::time::Instant::now() < deadline {
        seen = registry.by_pid(29434).map(|e| e.session_id);
        if seen.as_deref() == Some("s-2") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(seen.as_deref(), Some("s-2"));
}

#[test]
fn two_live_processes_claiming_one_session_answer_neither() {
    let dir = Dir::new("twice");
    dir.write(1, &file(1, "s-1", "Sun Oct  4 18:06:13 2026"));
    dir.write(2, &file(2, "s-1", "Sun Oct  4 18:06:13 2026"));
    let table = Table([(1, (STARTED, None)), (2, (STARTED, None))].into());
    let registry = Registry::new(dir.0.clone(), Box::new(table));
    assert_eq!(registry.by_session("s-1"), None);
}

#[test]
fn a_pane_is_matched_by_its_tty_device_not_its_tmux_name() {
    let dir = Dir::new("tty");
    dir.write(1, &file(1, "s-1", "Sun Oct  4 18:06:13 2026"));
    // /dev/null's device number stands in for a pane's tty.
    let null = device_of("/dev/null").unwrap();
    let registry = Registry::new(dir.0.clone(), Box::new(Table([(1, (STARTED, Some(null)))].into())));
    let entry = registry.by_pid(1).unwrap();
    assert!(registry.runs_on(&entry, "/dev/null"));
    assert!(!registry.runs_on(&entry, "/dev/zero"), "same tmux name, another terminal");
}

#[test]
fn the_kernel_knows_this_process_and_when_it_started() {
    let me = std::process::id() as i32;
    let started = Kernel.started(me).expect("this process");
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    assert!(started <= now && now - started < 24 * 3600, "{started} vs {now}");
    assert_eq!(Kernel.started(i32::MAX), None);
}

#[test]
fn the_transcript_is_the_session_file_under_the_slugged_cwd() {
    let dir = Dir::new("transcript");
    let registry = Registry::new(dir.0.clone(), Box::new(Table::default()));
    let entry = parse(file(1, "s-1", "Sun Oct  4 18:06:13 2026").as_bytes()).unwrap();
    assert_eq!(registry.transcript(&entry, "/elsewhere"), None, "not written yet");
    let project = dir.0.join("projects").join("-tmp-fc-t-proj");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("s-1.jsonl"), "").unwrap();
    assert_eq!(registry.transcript(&entry, "/elsewhere"), Some(project.join("s-1.jsonl")));
}

/// This machine's own registry, read and never written: every entry whose
/// process is alive should verify. Ignored; run by hand.
#[test]
#[ignore]
fn this_machines_registry_verifies_against_the_kernel() {
    let Some(config) = std::env::var_os("FARCOOLER_REGISTRY_PROBE").map(PathBuf::from) else { return };
    for entry in std::fs::read_dir(config.join("sessions")).unwrap().flatten() {
        let Some(e) = std::fs::read(entry.path()).ok().and_then(|b| parse(&b)) else { continue };
        let kernel = Kernel.started(e.pid);
        eprintln!("pid {} said {:?} kernel {:?} live {} tty {:?}", e.pid, e.started, kernel, is_live(&e, &Kernel), Kernel.tty(e.pid));
        if kernel.is_some() {
            assert!(is_live(&e, &Kernel), "{e:?}");
        }
    }
}
