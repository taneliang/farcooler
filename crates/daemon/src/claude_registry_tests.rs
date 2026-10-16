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
        let dir = farcooler_core::scratch::dir(&format!("registry-{tag}"));
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
    let waiting = file(1, "s", "Sun Oct  4 18:06:13 2026").replace("\"busy\"", "\"waiting\"");
    assert_eq!(parse(waiting.as_bytes()).unwrap().status, Some(Activity::Waiting), "under a dialog (ov-368)");
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

#[test]
fn a_session_the_cache_has_not_seen_yet_is_read_for_once_more() {
    // `/clear` rewrites the file just before the new session's first hook;
    // the watch's event can come after it. A registry that never watches
    // stands for that moment.
    let dir = Dir::new("miss");
    fake::write(&dir.0, 29434, "before", "/tmp");
    let registry = Registry::unwatched(dir.0.clone(), Box::new(fake::Alive(vec![(29434, None)])));
    assert_eq!(registry.by_pid(29434).map(|e| e.session_id), Some("before".into()));
    fake::write(&dir.0, 29434, "after", "/tmp");
    assert_eq!(registry.by_pid(29434).map(|e| e.session_id), Some("before".into()), "the cache is stale");
    assert_eq!(registry.by_session("after").map(|e| e.pid), Some(29434), "a miss reads again");
}

#[test]
fn a_missing_directory_is_not_watched_again_on_every_lookup() {
    let dir = Dir::new("nowatch");
    std::fs::remove_dir_all(dir.0.join("sessions")).unwrap();
    let registry = Registry::new(dir.0.clone(), Box::new(fake::Alive(vec![])));
    for _ in 0..50 {
        registry.by_pid(1);
    }
    assert!(registry.watch_attempts() <= 1, "{} tries", registry.watch_attempts());
}

#[test]
fn a_deleted_directory_made_again_is_read_again() {
    let dir = Dir::new("deleted");
    fake::write(&dir.0, 29434, "first", "/tmp");
    let registry = Registry::new(dir.0.clone(), Box::new(fake::Alive(vec![(29434, None)])));
    assert_eq!(registry.by_pid(29434).map(|e| e.session_id), Some("first".into()));
    std::fs::remove_dir_all(dir.0.join("sessions")).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while registry.by_pid(29434).is_some() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(registry.by_pid(29434), None, "the deletion is seen");
    fake::write(&dir.0, 29434, "second", "/tmp");
    assert_eq!(registry.by_pid(29434).map(|e| e.session_id), Some("second".into()), "and a new directory is read");
}

/// A member's claude registers under their own config directory (ov-583),
/// and the daemon's registry finds it there: by pid, by session, and its
/// transcript under that directory's projects, never the runner's.
#[test]
fn a_members_session_is_found_in_their_own_directory() {
    let runtime = Dir::new("members-runtime");
    let runner = Dir::new("members-runner");
    let alice = crate::claude_logins::login_dir(&runtime.0, &uuid::Uuid::now_v7().simple().to_string()).unwrap();
    std::fs::create_dir_all(alice.join("sessions")).unwrap();
    std::fs::write(alice.join("sessions").join("29434.json"), file(29434, "s-alice", "Sun Oct  4 18:06:13 2026")).unwrap();
    let mut table = Table::default();
    table.0.insert(29434, (fake::STARTED, None));
    let registry = Registry::new(runner.0.clone(), Box::new(table)).with_members(runtime.0.clone());
    let entry = registry.by_pid(29434).expect("found in Alice's directory");
    assert_eq!(entry.session_id, "s-alice");
    assert_eq!(registry.by_session("s-alice").map(|e| e.pid), Some(29434));
    let project = alice.join("projects").join("-tmp-fc-t-proj");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("s-alice.jsonl"), "").unwrap();
    std::fs::create_dir_all(runner.0.join("projects").join("-tmp-fc-t-proj")).unwrap();
    std::fs::write(runner.0.join("projects").join("-tmp-fc-t-proj").join("s-alice.jsonl"), "").unwrap();
    assert_eq!(registry.transcript(&entry, "/elsewhere"), Some(project.join("s-alice.jsonl")), "Alice's, not the runner's");
    let elsewhere = alice.join("projects").join("-nonexistent-ov583-proj");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::write(elsewhere.join("s-alice.jsonl"), "").unwrap();
    assert_eq!(registry.transcript_in(&entry, "/nonexistent-ov583/proj"), Some(elsewhere.join("s-alice.jsonl")));
}

/// The log watch joins members from this registry's own list (ov-593): each
/// real member directory, the runner's left out, none when members aren't
/// read, and a member made after the first listing once `MEMBERS_EVERY` has
/// passed.
#[test]
fn the_member_directories_are_the_registrys_own_list() {
    let runtime = Dir::new("members-list-runtime");
    let runner = Dir::new("members-list-runner");
    let alice = crate::claude_logins::login_dir(&runtime.0, &uuid::Uuid::now_v7().simple().to_string()).unwrap();
    std::fs::create_dir_all(&alice).unwrap();
    let plain = Registry::new(runner.0.clone(), Box::new(Table::default()));
    assert!(plain.member_config_dirs().is_empty(), "members not read");
    let registry = Registry::new(runner.0.clone(), Box::new(Table::default())).with_members(runtime.0.clone());
    assert_eq!(registry.member_config_dirs(), vec![alice.clone()]);
    let bob = crate::claude_logins::login_dir(&runtime.0, &uuid::Uuid::now_v7().simple().to_string()).unwrap();
    std::fs::create_dir_all(&bob).unwrap();
    assert_eq!(registry.member_config_dirs(), vec![alice.clone()], "listed again only after MEMBERS_EVERY");
    let aged = Registry::new(runner.0.clone(), Box::new(Table::default())).with_members(runtime.0.clone());
    let mut want = vec![alice, bob];
    want.sort();
    assert_eq!(aged.member_config_dirs(), want, "a member made since is listed");
}
