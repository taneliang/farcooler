use super::{Answer, BLIND, FLAG, Held, Leader, Lineage, ask_blocking, signal_group};
#[cfg(target_os = "macos")]
use super::responsibility;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A hold run as `farcoolerd` runs it in a pane, here without a terminal.
struct Hold { child: std::process::Child, socket: PathBuf, dir: tempfile::TempDir }
fn hold(script: &str) -> Hold { hold_with(script, &[]) }
fn hold_with(script: &str, env: &[(&str, &str)]) -> Hold {
    let dir = tempfile::Builder::new().prefix("h").tempdir_in("/tmp").unwrap();
    let socket = dir.path().join("t.hold.sock");
    let mut child = std::process::Command::new(crate::test_binary::daemon_binary().expect("build farcoolerd before this test"))
        .args([FLAG, socket.to_str().unwrap(), "--", "/bin/sh", "-c", script])
        .envs(env.iter().copied()).current_dir(dir.path()).stdin(std::process::Stdio::null()).spawn().unwrap();
    for _ in 0..500 {
        if ask_blocking(&socket, "ready", Duration::from_secs(1)) == Answer::Said("ready".into()) { return Hold { child, socket, dir }; }
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = child.kill();
    let _ = child.wait();
    panic!("the hold never answered");
}
fn alive(pid: i32) -> bool { unsafe { libc::kill(pid, 0) == 0 } }

#[cfg(target_os = "linux")]
fn job_exited(pid: i32) -> bool {
    // kill(pid, 0) still succeeds for an unreaped zombie.
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => {
            let state = stat.rsplit_once(')').and_then(|(_, fields)| fields.split_whitespace().next())
                .expect("job stat contains a process state");
            matches!(state, "Z" | "X" | "x")
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound || error.raw_os_error() == Some(libc::ESRCH) => true,
        Err(error) => panic!("could not read process state for job {pid}: {error}"),
    }
}

#[cfg(not(target_os = "linux"))]
fn job_exited(pid: i32) -> bool { !alive(pid) }

fn pid_in(dir: &Path, name: &str) -> i32 {
    for _ in 0..500 {
        if let Ok(text) = std::fs::read_to_string(dir.join(name)) && let Ok(pid) = text.trim().parse() { return pid; }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("no {name}");
}
fn ticks(dir: &Path) -> usize { std::fs::read(dir.join("ticks")).map_or(0, |t| t.len()) }

#[test]
fn a_hold_stops_continues_and_ends_its_agents_group() {
    let mut h = hold("/bin/sh -c 'while :; do echo t >> ticks; sleep 0.02; done' & echo $! > job; wait");
    let job = pid_in(h.dir.path(), "job");
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(ask_blocking(&h.socket, "stop", Duration::from_secs(5)), Answer::Said("stopped".into()));
    let at = ticks(h.dir.path());
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(ticks(h.dir.path()), at, "the whole group stopped");
    assert_eq!(ask_blocking(&h.socket, "cont", Duration::from_secs(5)), Answer::Said("continued".into()));
    std::thread::sleep(Duration::from_millis(200));
    assert!(ticks(h.dir.path()) > at, "and continued");
    assert_eq!(ask_blocking(&h.socket, "end", Duration::from_secs(30)), Answer::Said("ended".into()));
    assert!(!h.socket.exists(), "the socket goes once the group is empty");
    assert!(job_exited(job), "the job ended with the group");
    assert_eq!(h.child.wait().unwrap().code(), Some(128 + libc::SIGTERM), "the agent's status, as a shell reports it");
    assert_eq!(ask_blocking(&h.socket, "stop", Duration::from_secs(1)), Answer::Gone, "an ended hold stops nothing");
}

/// The agent exiting by itself ends what it left in its group, and the hold exits with its status, so the pane dies
/// as it did before the hold existed.
#[test]
fn an_agent_that_exits_takes_its_group_with_it() {
    let mut h = hold("/bin/sleep 30 & echo $! > job; while [ ! -f go ]; do sleep 0.01; done; exit 7");
    let job = pid_in(h.dir.path(), "job");
    let was_alive = !job_exited(job);
    std::fs::write(h.dir.path().join("go"), "").unwrap();
    assert_eq!(h.child.wait().unwrap().code(), Some(7));
    assert!(was_alive, "the job was alive before the agent exited");
    assert!(job_exited(job), "the job it left in its group ended");
    assert!(!h.socket.exists());
}

#[test]
fn a_hangup_ends_the_agent_and_exits_the_hold() {
    let mut h = hold("echo $$ > agent; exec /bin/sleep 30");
    let agent = pid_in(h.dir.path(), "agent");
    assert!(alive(agent));
    // SAFETY: the hold (the relay on macOS) is our own unreaped child.
    assert_eq!(unsafe { libc::kill(h.child.id() as i32, libc::SIGHUP) }, 0);
    for _ in 0..500 {
        if let Some(status) = h.child.try_wait().unwrap() {
            assert_eq!(status.code(), Some(128 + libc::SIGTERM));
            assert!(!alive(agent), "the hold reaped its agent");
            assert!(!h.socket.exists(), "the hold removed its socket");
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = ask_blocking(&h.socket, "end", Duration::from_secs(5));
    h.child.wait().unwrap();
    panic!("the hold did not exit on SIGHUP");
}

/// One reply line from a raw client.
fn reply(stream: &UnixStream) -> String {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    line.trim().to_owned()
}

/// ov-1330 review: a landing gate tells the hold `stop` and then `cont` without waiting for either answer. If the
/// stop's words arrive after the continue's, the stop must still be done first, or the builder stays stopped with no
/// gate left to continue it.
#[test]
fn a_stop_asked_first_is_done_first_even_when_its_words_arrive_last() {
    let mut h = hold("/bin/sh -c 'while :; do echo t >> ticks; sleep 0.02; done' & wait");
    std::thread::sleep(Duration::from_millis(100));
    let mut stop = UnixStream::connect(&h.socket).unwrap();
    let mut cont = UnixStream::connect(&h.socket).unwrap();
    cont.write_all(b"cont\n").unwrap();
    // The continue has arrived; the stop, asked first, has not.
    std::thread::sleep(Duration::from_millis(300));
    stop.write_all(b"stop\n").unwrap();
    assert_eq!(reply(&stop), "stopped");
    assert_eq!(reply(&cont), "continued");
    let at = ticks(h.dir.path());
    std::thread::sleep(Duration::from_millis(300));
    assert!(ticks(h.dir.path()) > at, "the later continue was done last, so the group runs");
    assert_eq!(ask_blocking(&h.socket, "end", Duration::from_secs(30)), Answer::Said("ended".into()));
    h.child.wait().unwrap();
}

/// Disconnect after the stop reply but before confirming it, both with and without an earlier gate.
#[test]
fn an_unconfirmed_pause_rolls_back_only_its_own_stop() {
    let mut h = hold("while :; do sleep 0.02; done");
    let wait = Duration::from_secs(5);
    for gate in [false, true] {
        if gate {
            assert_eq!(ask_blocking(&h.socket, "stop", wait), Answer::Said("stopped".into()));
        }
        let mut manual = UnixStream::connect(&h.socket).unwrap();
        manual.write_all(b"pause\n").unwrap();
        assert_eq!(reply(&manual), "stopped");
        drop(manual);
        let expected = Answer::Said(if gate { "stopped" } else { "unsettled" }.into());
        let mut state = ask_blocking(&h.socket, "stopped", wait);
        for _ in 0..20 {
            if state == expected { break; }
            std::thread::sleep(Duration::from_millis(10));
            state = ask_blocking(&h.socket, "stopped", wait);
        }
        assert_eq!(state, expected);
    }
    assert_eq!(ask_blocking(&h.socket, "end", wait), Answer::Said("ended".into()));
    h.child.wait().unwrap();
}

/// A command that leaves the agent's group (a new session) and ignores the agent's termination, appending to `work`
/// until it is killed.
const STRAY: &str = "perl -MPOSIX -e 'setsid; $SIG{TERM} = q(IGNORE); $SIG{HUP} = q(IGNORE); \
    while (1) { open(F, q(>>work)); print F 1; close F; select(undef, undef, undef, 0.02) }'";

/// The test's own stray, killed when the test ends however it ends. Only the test signals it: the hold never does.
struct Stray(i32);
impl Drop for Stray { fn drop(&mut self) { unsafe { libc::kill(self.0, libc::SIGKILL); } } }
fn work(dir: &Path) -> usize { std::fs::read(dir.join("work")).map_or(0, |w| w.len()) }
fn working(dir: &Path) -> bool {
    let at = work(dir);
    std::thread::sleep(Duration::from_millis(200));
    work(dir) > at
}

/// What the hold must say about a stray that is still working: `unsettled`, its socket kept, and `ended` only once the
/// stray is gone.
fn refuses_until_the_stray_is_gone(mut h: Hold) {
    let stray = Stray(pid_in(h.dir.path(), "stray"));
    for _ in 0..100 { if work(h.dir.path()) > 0 { break; } std::thread::sleep(Duration::from_millis(10)); }
    assert_eq!(ask_blocking(&h.socket, "end", Duration::from_secs(30)), Answer::Said("unsettled".into()));
    assert!(h.socket.exists(), "a hold that cannot prove its work ended keeps its socket");
    assert!(working(h.dir.path()), "the stray is still working, and nothing said the agent had ended");
    assert!(h.child.try_wait().unwrap().is_none(), "the hold stays to account for it");
    assert!(alive(stray.0), "the hold never signals what left its group");
    drop(stray);
    assert_eq!(ask_blocking(&h.socket, "end", Duration::from_secs(30)), Answer::Said("ended".into()));
    assert!(!h.socket.exists());
    h.child.wait().unwrap();
}

/// ov-1330 review: the group being empty is not quiescence while something the agent started outside it still works.
#[test]
fn a_process_that_left_the_group_keeps_end_from_succeeding() {
    refuses_until_the_stray_is_gone(hold(&format!("{STRAY} & echo $! > stray; while :; do sleep 1; done")));
}

/// The same when the stray's parent exited long before `end`: on Linux it was reparented to the hold, on macOS to
/// launchd, where the kernel still names the parent it was forked by.
#[test]
fn a_stray_whose_parent_exited_first_is_still_accounted_for() {
    let h = hold(&format!("( {STRAY} & echo $! > stray; sleep 0.5 ); echo > orphaned; while :; do sleep 1; done"));
    pid_in(h.dir.path(), "stray");
    for _ in 0..500 { if h.dir.path().join("orphaned").exists() { break; } std::thread::sleep(Duration::from_millis(10)); }
    refuses_until_the_stray_is_gone(h);
}

/// ov-1330 review (fix round 2): the stray's parent exits at once, so the hold may never have seen it, and here it
/// certainly has not: the hold is started blind (it learns no lineage; debug builds only). On macOS only
/// responsibility can then find the stray, on Linux the subreaper; either way `end` must not say `ended` while it
/// writes.
#[test]
fn a_stray_whose_parent_the_hold_never_saw_keeps_end_from_succeeding() {
    refuses_until_the_stray_is_gone(hold_with(&format!("( {STRAY} & echo $! > stray ); while :; do sleep 1; done"), &[(BLIND, "1")]));
}

/// macOS: the pane's process is a relay; the hold it starts is responsible for itself, and so for everything the
/// agent starts, while the relay is not.
#[cfg(target_os = "macos")]
#[test]
fn the_hold_is_responsible_for_its_agent_and_the_relay_is_not() {
    let mut h = hold("echo $PPID > hold; echo $$ > agent; while :; do sleep 1; done");
    let (relay, held, agent) = (h.child.id() as i32, pid_in(h.dir.path(), "hold"), pid_in(h.dir.path(), "agent"));
    assert_ne!(relay, held, "tmux's process relays to a hold of its own");
    assert_eq!(responsibility::of(held), Some(held));
    assert_eq!(responsibility::of(agent), Some(held));
    assert_ne!(responsibility::of(relay), Some(held));
    assert_eq!(ask_blocking(&h.socket, "end", Duration::from_secs(30)), Answer::Said("ended".into()));
    assert_eq!(h.child.wait().unwrap().code(), Some(128 + libc::SIGTERM), "the relay exits with the hold's status");
}

/// An agent that ends what it started outside its group on SIGTERM (as Codex does) ends cleanly.
#[test]
fn an_agent_that_ends_its_own_stray_ends() {
    let mut h = hold(&format!("{STRAY} & S=$!; echo $S > stray; trap 'kill -9 $S; exit 0' TERM; while :; do sleep 0.05; done"));
    let stray = Stray(pid_in(h.dir.path(), "stray"));
    for _ in 0..100 { if work(h.dir.path()) > 0 { break; } std::thread::sleep(Duration::from_millis(10)); }
    assert_eq!(ask_blocking(&h.socket, "end", Duration::from_secs(30)), Answer::Said("ended".into()));
    assert_eq!(h.child.wait().unwrap().code(), Some(0));
    assert!(!working(h.dir.path()), "the stray is gone");
    drop(stray);
}

/// A pid stops being the agent's the moment the hold reaps it, so the hold signals nothing after that, even when the
/// number is (as here) a live process.
#[test]
fn a_reaped_leader_is_never_signalled() {
    let mut other = std::process::Command::new("/bin/sleep").arg("30").spawn().unwrap();
    let pid = other.id() as i32;
    let held = Held { leader: Leader(pid), reaped: Some(0), lineage: Lineage::new().unwrap(), gate_stop: false };
    assert!(!signal_group(&held, libc::SIGKILL));
    std::thread::sleep(Duration::from_millis(50));
    assert!(alive(pid));
    other.kill().unwrap();
    other.wait().unwrap();
}

/// Where a native agent is signalled, and what each file may do.
const NATIVE: [(&str, Rule); 6] = [
    ("src/workflow_runtime/coding_native.rs", Rule::NoSignals),
    ("src/workflow_runtime/coding_native_quiesce.rs", Rule::NoSignals),
    ("src/workflow_runtime/native_control.rs", Rule::NoSignals),
    ("src/native_hold.rs", Rule::OnlySignalGroup),
    ("src/yielding.rs", Rule::NoProcessListing),
    ("src/yielding/heavy_queue.rs", Rule::NoSignals),
];
#[derive(Clone, Copy)]
enum Rule { NoSignals, OnlySignalGroup, NoProcessListing }
const SIGNALS: [&str; 5] = ["libc::kill(", "killpg(", "libc::raise(", "sigqueue(", "Command::new(\"kill\")"];
const LISTINGS: [&str; 3] = ["under_tty(", "proc_table::", "foreground::parse("];

/// What `rule` finds wrong with a file's text. Test modules are left out: they assert with `kill(pid, 0)`.
fn violations(text: &str, rule: Rule) -> Vec<String> {
    let shipped = text.find("\n#[cfg(test)]\nmod ").or_else(|| text.find("\n#[cfg(test)]\npub(super) mod ")).map_or(text, |at| &text[..at]);
    let found = |needles: &[&str]| -> Vec<String> {
        shipped.lines().filter(|l| !l.trim_start().starts_with("//"))
            .filter(|l| needles.iter().any(|n| l.contains(n))).map(str::to_owned).collect()
    };
    match rule {
        Rule::NoSignals => found(&SIGNALS),
        Rule::NoProcessListing => found(&LISTINGS),
        Rule::OnlySignalGroup => {
            // The group signal inside `signal_group`, and on macOS the relay passing a hangup to the hold it spawned
            // and has not reaped; nothing else.
            let calls = found(&SIGNALS);
            let body = |name: &str| shipped.split(name).nth(1).and_then(|rest| rest.split("\n}\n").next()).unwrap_or("");
            let only = |name: &str| shipped.matches(name).count() == 1;
            let group = body("fn signal_group(").lines().filter(|l| l.contains("libc::killpg(held.leader.0,")).count();
            let relay = body("fn relay(");
            let relayed = relay.lines().filter(|l| l.contains("libc::kill(hold,")).count();
            let spawned = relay.matches("libc::posix_spawn(&mut hold,").count() == 1 && relay.matches("hold =").count() == 0;
            let fine = only("fn signal_group(") && group == 1 && (calls.len() == 1
                || (calls.len() == 2 && only("fn relay(") && relayed == 1 && spawned));
            if fine { Vec::new() } else { calls }
        }
    }
}

/// ov-1330: no native signal targets a pid found by looking. Only the hold signals, only through `signal_group`,
/// whose only target is the leader it forked and has not reaped.
#[test]
fn the_native_engine_signals_only_its_own_group() {
    for (file, rule) in NATIVE {
        let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(file)).unwrap();
        let bad = violations(&text, rule);
        assert!(bad.is_empty(), "{file} signals outside the hold's own group (ov-1330): {bad:#?}");
    }
}

/// The check above fails on each thing it forbids.
#[test]
fn the_signal_check_catches_what_it_forbids() {
    let pane = "fn quiesce() {\n    for pid in table.under_tty(tty) { unsafe { libc::kill(pid, libc::SIGKILL); } }\n}\n";
    assert!(!violations(pane, Rule::NoSignals).is_empty());
    assert!(!violations(pane, Rule::NoProcessListing).is_empty());
    assert!(!violations("fn f() { unsafe { libc::killpg(g, 9); } }\n", Rule::NoSignals).is_empty());
    let hold = "fn signal_group(held: &Held, signal: i32) -> bool {\n    unsafe { libc::killpg(held.leader.0, signal) == 0 }\n}\n";
    assert!(violations(hold, Rule::OnlySignalGroup).is_empty());
    let second = format!("{hold}fn other(pid: i32) {{ unsafe {{ libc::killpg(pid, 9); }} }}\n");
    assert!(!violations(&second, Rule::OnlySignalGroup).is_empty(), "a second signal call");
    let elsewhere = "fn signal_group(held: &Held, signal: i32) -> bool {\n    unsafe { libc::killpg(found_pid, signal) == 0 }\n}\n";
    assert!(!violations(elsewhere, Rule::OnlySignalGroup).is_empty(), "a target other than the forked leader");
    let relay = "fn relay() -> i32 {\n    let mut hold: i32 = 0;\n    libc::posix_spawn(&mut hold, p, a, e);\n    unsafe { libc::kill(hold, libc::SIGHUP); }\n}\n";
    assert!(violations(&format!("{hold}{relay}"), Rule::OnlySignalGroup).is_empty(), "the relay's hangup to its own child");
    let found = relay.replace("let mut hold: i32 = 0;", "let mut hold: i32 = 0;\n    hold = found_pid;");
    assert!(!violations(&format!("{hold}{found}"), Rule::OnlySignalGroup).is_empty(), "the relay signalling a pid it did not spawn");
    let unspawned = relay.replace("libc::posix_spawn(&mut hold, p, a, e);", "");
    assert!(!violations(&format!("{hold}{unspawned}"), Rule::OnlySignalGroup).is_empty(), "a relay that spawned nothing");
    // Test modules may assert with `kill(pid, 0)`.
    assert!(violations("fn f() {}\n#[cfg(test)]\nmod tests { fn g() { unsafe { libc::kill(1, 0); } } }\n", Rule::NoSignals).is_empty());
}
