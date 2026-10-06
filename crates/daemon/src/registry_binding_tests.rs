//! Binding panes from claude's registry, against a real store and made-up
//! processes.

use std::path::PathBuf;

use farcooler_core::inventory::{RuntimeSnapshot, TaggedPane};
use farcooler_store::models::PaneMode;
use uuid::Uuid;

use super::*;
use crate::claude_registry::{device_of, Processes, Registry};

/// `Sun Oct  4 18:06:13 2026` UTC.
const STARTED: i64 = 1_791_137_173;

/// One process, pid 4242, started at `STARTED` on the terminal `tty`, or no
/// process at all.
struct One {
    alive: bool,
    tty: &'static str,
}

impl Processes for One {
    fn started(&self, pid: i32) -> Option<i64> {
        (self.alive && pid == 4242).then_some(STARTED)
    }
    fn tty(&self, pid: i32) -> Option<u64> {
        (pid == 4242).then(|| device_of(self.tty)).flatten()
    }
}

struct Config(PathBuf);

impl Config {
    fn new(tag: &str, session: &str, cwd: &str) -> Config {
        let dir = std::env::temp_dir().join(format!("fc-binding-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sessions")).unwrap();
        std::fs::write(
            dir.join("sessions/4242.json"),
            format!(r#"{{"pid":4242,"sessionId":"{session}","cwd":"{cwd}","procStart":"Sun Oct  4 18:06:13 2026","status":"idle","tmux":"farcooler:@1.%7"}}"#),
        )
        .unwrap();
        Config(dir)
    }
    fn registry(&self, alive: bool, tty: &'static str) -> Registry {
        Registry::new(self.0.clone(), Box::new(One { alive, tty }))
    }
}

impl Drop for Config {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn pane(terminal: Uuid, pane_id: &str, tty: &str) -> TaggedPane {
    TaggedPane {
        daemon_id: Uuid::nil(),
        worktree_id: Uuid::nil(),
        terminal_id: terminal,
        schema_version: 1,
        pane_id: pane_id.into(),
        window_id: "@1".into(),
        columns: 80,
        rows: 24,
        left: 0,
        top: 0,
        window_active: true,
        pane_active: true,
        zoomed: false,
        tty: tty.into(),
        dead: false,
        dead_status: None,
        dead_signal: None,
        command: "claude".into(),
        title: String::new(),
    }
}

async fn terminal_with_session(session: Option<&str>) -> (crate::test_support::ScratchDir, std::sync::Arc<crate::service::Service>, Uuid) {
    let (dir, svc, repo) = crate::test_support::fixture().await;
    let rows = svc.store.list_worktrees_for_repository(repo).unwrap();
    let main = rows.iter().find(|w| w.is_main_checkout).unwrap();
    let workspace = svc.store.create_workspace(repo, "Billing", "bil").unwrap();
    let term = svc.store.create_terminal_for_test(main.id, workspace.id);
    if let Some(session) = session {
        let row = svc.store.get_terminal(term).unwrap();
        svc.store.set_pane_mode(term, row.resource_version, PaneMode::Terminal, Some(session.into()), false).unwrap();
    }
    (dir, svc, term)
}

#[tokio::test]
async fn after_clear_the_new_sessions_hook_finds_its_pane_and_the_row_follows() {
    let (_dir, svc, term) = terminal_with_session(Some("before-clear")).await;
    let config = Config::new("rebind", "after-clear", "/tmp");
    let registry = config.registry(true, "/dev/null");
    let snapshot = RuntimeSnapshot::healthy(vec![pane(term, "%7", "/dev/null")]);
    assert_eq!(bind_pane(&registry, &svc.store, &snapshot, "after-clear"), Some(term));
    assert_eq!(svc.store.get_terminal(term).unwrap().agent_session_id.as_deref(), Some("after-clear"));
    assert_eq!(svc.store.terminals_with_agent_session("after-clear").unwrap().len(), 1, "and its next hook routes by the row");
}

#[tokio::test]
async fn a_hand_started_claude_binds_by_the_registry_rather_than_by_guessing() {
    let (_dir, svc, term) = terminal_with_session(None).await;
    let config = Config::new("hand", "typed-by-hand", "/tmp");
    let registry = config.registry(true, "/dev/null");
    let snapshot = RuntimeSnapshot::healthy(vec![pane(term, "%7", "/dev/null")]);
    assert_eq!(bind_pane(&registry, &svc.store, &snapshot, "typed-by-hand"), Some(term));
}

#[tokio::test]
async fn the_same_pane_id_on_another_terminal_device_is_another_server() {
    let (_dir, svc, term) = terminal_with_session(Some("before")).await;
    let config = Config::new("tty", "after", "/tmp");
    let registry = config.registry(true, "/dev/null");
    let snapshot = RuntimeSnapshot::healthy(vec![pane(term, "%7", "/dev/zero")]);
    assert_eq!(bind_pane(&registry, &svc.store, &snapshot, "after"), None);
    assert_eq!(svc.store.get_terminal(term).unwrap().agent_session_id.as_deref(), Some("before"), "untouched");
}

#[tokio::test]
async fn a_stale_registry_file_binds_nothing() {
    let (_dir, svc, term) = terminal_with_session(Some("before")).await;
    let config = Config::new("stale", "after", "/tmp");
    let registry = config.registry(false, "/dev/null");
    let snapshot = RuntimeSnapshot::healthy(vec![pane(term, "%7", "/dev/null")]);
    assert_eq!(bind_pane(&registry, &svc.store, &snapshot, "after"), None);
    assert_eq!(svc.store.get_terminal(term).unwrap().agent_session_id.as_deref(), Some("before"));
}

#[tokio::test]
async fn a_chat_pane_is_never_bound() {
    let (_dir, svc, term) = terminal_with_session(Some("before")).await;
    let row = svc.store.get_terminal(term).unwrap();
    svc.store.set_pane_mode(term, row.resource_version, PaneMode::Agent, None, false).unwrap();
    let config = Config::new("chat", "after", "/tmp");
    let registry = config.registry(true, "/dev/null");
    let snapshot = RuntimeSnapshot::healthy(vec![pane(term, "%7", "/dev/null")]);
    assert_eq!(bind_pane(&registry, &svc.store, &snapshot, "after"), None);
}

#[test]
fn the_log_and_the_adoption_come_from_the_registry_when_it_answers() {
    let config = Config::new("log", "s-1", "/tmp/fc-t/proj");
    let registry = config.registry(true, "/dev/null");
    assert_eq!(registered_log(&registry, Some("claude"), Some(4242), "/x"), None, "no transcript yet");
    assert_eq!(session_to_adopt(&registry, 4242, "/x", &[]), None, "nothing to resume yet");
    let project = config.0.join("projects/-tmp-fc-t-proj");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("s-1.jsonl"), "").unwrap();
    assert_eq!(registered_log(&registry, Some("claude"), Some(4242), "/x"), Some(project.join("s-1.jsonl")));
    assert_eq!(registered_log(&registry, Some("codex"), Some(4242), "/x"), None, "claude's registry, claude's panes");
    assert_eq!(registered_log(&registry, Some("claude"), Some(1), "/x"), None, "another pid");
    assert_eq!(session_to_adopt(&registry, 4242, "/x", &[]), Some("s-1".into()));
    assert_eq!(session_to_adopt(&registry, 4242, "/x", &["s-1".into()]), None, "another terminal has it");
    let dead = config.registry(false, "/dev/null");
    assert_eq!(registered_log(&dead, Some("claude"), Some(4242), "/x"), None, "a stale file");
}

/// The call site: a hook from a session no row names reaches the pane the
/// registry places it in, through `HookIngress::terminal_for`.
#[tokio::test]
async fn a_hook_from_an_unnamed_session_routes_through_the_registry() {
    use farcooler_agent_hooks::{Agent, facts::Facts};
    let (_dir, svc, term) = terminal_with_session(Some("before-clear")).await;
    let config = Config::new("ingress", "after-clear", "/tmp");
    let registry: &'static Registry = Box::leak(Box::new(config.registry(true, "/dev/null")));
    let inventory = farcooler_core::inventory::FakeInventory { snapshot: RuntimeSnapshot::healthy(vec![pane(term, "%7", "/dev/null")]) };
    let ingress = crate::hook_ingress::HookIngress::new(svc.store.clone(), std::sync::Arc::new(inventory), Default::default())
        .with_registry(registry);
    let facts = Facts { session_id: Some("after-clear".into()), ..Facts::default() };
    assert_eq!(ingress.terminal_for(&facts, Agent::Claude), Some(term));
    let unknown = Facts { session_id: Some("nobody".into()), ..Facts::default() };
    assert_eq!(ingress.terminal_for(&unknown, Agent::Claude), None);
}
