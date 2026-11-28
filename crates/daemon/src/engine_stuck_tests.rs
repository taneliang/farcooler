//! A pane is read the way the engine's builds are: through a real tmux, with claude's own captured screens.
use std::sync::atomic::AtomicBool;

use uuid::Uuid;

use super::*;

const TRUST: &str = include_str!("../../core/captures/claude-trust-gate.txt");
const PERMISSION: &str = include_str!("../../core/captures/claude-blocked.txt");
const IDLE: &str = include_str!("../../core/captures/claude-idle-fresh.txt");

#[test]
fn a_trust_or_permission_screen_has_a_question_and_an_idle_one_has_none() {
    assert!(asking("claude", TRUST).is_some_and(|q| q.contains("Trust this folder")), "{:?}", asking("claude", TRUST));
    assert!(asking("claude", PERMISSION).is_some());
    assert_eq!(asking("claude", IDLE), None);
}

struct Pane {
    tmux: TmuxServer,
    id: String,
    dir: std::path::PathBuf,
}
impl Drop for Pane {
    fn drop(&mut self) {
        farcooler_tmux::reap_server(self.tmux.socket());
    }
}
impl Pane {
    /// A pane that prints `screen` as it is, then waits.
    async fn showing(screen: &str) -> Self {
        assert!(farcooler_core::programs::find("tmux").is_some(), "these tests need tmux");
        let dir = farcooler_core::scratch::dir("engine-stuck");
        let file = dir.join("screen.txt");
        std::fs::write(&file, screen).unwrap();
        let tmux = TmuxServer::in_dir(&format!("test-{}", Uuid::now_v7().simple()), Uuid::now_v7(), &dir);
        let window = tmux
            .create_terminal_window(Uuid::now_v7(), Uuid::now_v7(), "t", "/tmp", &format!("sh -c \"cat '{}'; sleep 600\"", file.display()))
            .await
            .expect("a window");
        Self { tmux, id: window.pane_id, dir }
    }
}

/// Collect what the watch reports for `ms`.
async fn run(pane: &Pane, item: &str, heard: &AtomicBool, ms: u64) -> Vec<Stuck> {
    let mut seen = Vec::new();
    let mut watching = std::pin::pin!(watch(Startup::new("ws-test", item, "claude"), &pane.tmux, &pane.id, heard, None, None));
    let end = tokio::time::sleep(Duration::from_millis(ms));
    tokio::pin!(end);
    let mut look = tokio::time::interval(Duration::from_millis(40));
    loop {
        tokio::select! {
            never = &mut watching => match never {},
            _ = &mut end => break,
            _ = look.tick() => seen.extend(list().into_iter().filter(|s| s.item == item)),
        }
    }
    seen
}

#[tokio::test]
async fn a_build_on_the_trust_question_is_listed_with_it_before_any_hook() {
    let pane = Pane::showing(TRUST).await;
    let heard = AtomicBool::new(false);
    // Well inside `START_WITHIN`: the question alone puts it in the list.
    let seen = run(&pane, "ov-t1", &heard, 200).await;
    assert!(seen.iter().any(|s| s.question.contains("Trust this folder")), "{seen:?}");
    assert!(list().iter().all(|s| s.item != "ov-t1"), "ending the watch unlists it");
    let _ = &pane.dir;
}

#[tokio::test]
async fn a_build_that_becomes_stuck_tells_clients_to_reread_needs_you() {
    use farcooler_protocol::v1::event::Payload;
    let (_dir, svc, _repo) = crate::test_support::fixture().await;
    let watcher = crate::watch::Watcher::new(svc);
    let mut events = watcher.subscribe();
    let pane = Pane::showing(TRUST).await;
    let heard = AtomicBool::new(false);
    run(&pane, "ov-t4", &heard, 1000).await;
    let said = tokio::time::timeout(Duration::from_secs(3), async {
        while let Ok(event) = events.recv().await {
            if matches!(event.payload, Some(Payload::NeedsYouChanged(_))) { return true; }
        }
        false
    }).await.unwrap_or(false);
    assert!(said, "no NeedsYouChanged was sent when the build became stuck");
}

fn secs(s: u64) -> Duration { Duration::from_secs(s) }
fn cpu(pairs: &[(i32, u64)]) -> Option<Cpu> { Some(pairs.iter().map(|(pid, ms)| (*pid, Duration::from_millis(*ms))).collect()) }

#[test]
fn an_agent_that_prints_nothing_and_uses_no_cpu_is_quiet_only_after_the_whole_time() {
    let mut quiet = Quiet::default();
    assert!(!quiet.look(secs(0), 100, cpu(&[(1, 5_000)]), secs(600)));
    // An idle terminal program still ticks over a little.
    assert!(!quiet.look(secs(300), 100, cpu(&[(1, 5_600)]), secs(600)));
    assert!(quiet.look(secs(600), 100, cpu(&[(1, 6_200)]), secs(600)));
    // A word from it ends the quiet.
    assert!(!quiet.look(secs(630), 101, cpu(&[(1, 6_200)]), secs(600)));
}

#[test]
fn output_or_cpu_alone_keeps_an_agent_from_being_stalled() {
    let mut printing = Quiet::default();
    let mut building = Quiet::default();
    let mut unread = Quiet::default();
    for minute in 0..30 {
        assert!(!printing.look(secs(minute * 60), 100 + minute, cpu(&[(1, 0)]), secs(600)));
        // A silent compile: a new child each minute, busy while it lives.
        assert!(!building.look(secs(minute * 60), 100, cpu(&[(1, 0), (100 + minute as i32, 40_000)]), secs(600)));
        // CPU that can't be read is not "no CPU".
        assert!(!unread.look(secs(minute * 60), 100, None, secs(600)));
    }
}

#[test]
fn process_times_are_read_for_the_agent_and_everything_under_it() {
    let table = "  10     1   0:01.50\n  11    10  12:00.00\n  12    11   0:00.25\n  20     1  99:00.00\n  30    1 1-00:00:01\n";
    assert_eq!(cpu_under(table, 10), cpu(&[(10, 1_500), (11, 720_000), (12, 250)]));
    assert_eq!(cpu_under(table, 30), cpu(&[(30, 86_401_000)]));
    assert_eq!(cpu_under(table, 999), None);
}

#[tokio::test]
async fn a_silent_idle_pane_is_listed_as_stalled_with_its_last_lines() {
    let pane = Pane::showing("compiling things\nwaiting on nothing at all\n").await;
    let log = pane.dir.join("agent.output");
    std::fs::write(&log, "said once").unwrap();
    // The pane's own process stands in for the agent: asleep, so it uses no CPU.
    let leader = pane.tmux.run(&["display-message", "-p", "-t", &pane.id, "#{pane_pid}"]).await.unwrap().stdout.trim().parse().unwrap();
    let pulse = Pulse { log, agent: Agent::Process(leader), after: Duration::from_millis(600), every: Duration::from_millis(100) };
    let heard = AtomicBool::new(true);
    let mut watching = std::pin::pin!(watch(Startup::new("ws-test", "ov-stalled", "codex"), &pane.tmux, &pane.id, &heard, None, Some(pulse)));
    let found = async {
        loop {
            if let Some(stuck) = list().into_iter().find(|s| s.item == "ov-stalled") { break stuck; }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    };
    let stuck = tokio::select! {
        never = &mut watching => match never {},
        stuck = tokio::time::timeout(Duration::from_secs(10), found) => stuck.expect("an idle agent is never listed"),
    };
    assert!(stuck.question.starts_with("codex has printed nothing and used no CPU for "), "{}", stuck.question);
    assert!(stuck.question.contains("compiling things\nwaiting on nothing at all"), "{}", stuck.question);
}
