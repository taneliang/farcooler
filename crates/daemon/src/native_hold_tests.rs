use super::*;

/// A hold run as `farcoolerd` runs it in a pane, here without a terminal.
struct Hold { child: std::process::Child, socket: PathBuf, dir: tempfile::TempDir }
fn hold(script: &str) -> Hold {
    let dir = tempfile::Builder::new().prefix("h").tempdir_in("/tmp").unwrap();
    let socket = dir.path().join("t.hold.sock");
    let mut child = std::process::Command::new(crate::runtime::fanout_binary().expect("build farcoolerd before this test"))
        .args([FLAG, socket.to_str().unwrap(), "--", "/bin/sh", "-c", script])
        .current_dir(dir.path()).stdin(std::process::Stdio::null()).spawn().unwrap();
    for _ in 0..500 {
        if ask_blocking(&socket, "ready", Duration::from_secs(1)) == Answer::Said("ready".into()) { return Hold { child, socket, dir }; }
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = child.kill();
    let _ = child.wait();
    panic!("the hold never answered");
}
fn alive(pid: i32) -> bool { unsafe { libc::kill(pid, 0) == 0 } }
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
    assert!(!alive(job));
    assert_eq!(h.child.wait().unwrap().code(), Some(128 + libc::SIGTERM), "the agent's status, as a shell reports it");
    assert_eq!(ask_blocking(&h.socket, "stop", Duration::from_secs(1)), Answer::Gone, "an ended hold stops nothing");
}

/// The agent exiting by itself ends what it left in its group, and the hold exits with its status, so the pane dies
/// as it did before the hold existed.
#[test]
fn an_agent_that_exits_takes_its_group_with_it() {
    let mut h = hold("/bin/sleep 30 & echo $! > job; while [ ! -f go ]; do sleep 0.01; done; exit 7");
    let job = pid_in(h.dir.path(), "job");
    std::fs::write(h.dir.path().join("go"), "").unwrap();
    assert_eq!(h.child.wait().unwrap().code(), Some(7));
    assert!(!alive(job), "the job it left in its group ended");
    assert!(!h.socket.exists());
}

/// A pid stops being the agent's the moment the hold reaps it, so the hold signals nothing after that, even when the
/// number is (as here) a live process.
#[test]
fn a_reaped_leader_is_never_signalled() {
    let mut other = std::process::Command::new("/bin/sleep").arg("30").spawn().unwrap();
    let pid = other.id() as i32;
    let held = Held { leader: Leader(pid), reaped: Some(0) };
    assert!(!signal_group(&held, libc::SIGKILL));
    std::thread::sleep(Duration::from_millis(50));
    assert!(alive(pid));
    other.kill().unwrap();
    other.wait().unwrap();
}

/// Where a native agent is signalled, and what each file may do.
const NATIVE: [(&str, Rule); 4] = [
    ("src/workflow_runtime/coding_native.rs", Rule::NoSignals),
    ("src/workflow_runtime/native_control.rs", Rule::NoSignals),
    ("src/native_hold.rs", Rule::OnlySignalGroup),
    ("src/yielding.rs", Rule::NoProcessListing),
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
            // Exactly one signal call, and it is the one inside `signal_group`.
            let calls = found(&SIGNALS);
            let body = shipped.split("fn signal_group(").nth(1).and_then(|rest| rest.split("\n}\n").next()).unwrap_or("");
            let inside = body.lines().filter(|l| l.contains("libc::killpg(held.leader.0,")).count();
            if calls.len() == 1 && inside == 1 && shipped.matches("fn signal_group(").count() == 1 { Vec::new() } else { calls }
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
    // Test modules may assert with `kill(pid, 0)`.
    assert!(violations("fn f() {}\n#[cfg(test)]\nmod tests { fn g() { unsafe { libc::kill(1, 0); } } }\n", Rule::NoSignals).is_empty());
}
