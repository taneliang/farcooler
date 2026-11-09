//! The launch gate (ov-886): a frame's check that its launch is current, and
//! the effects that check permits, can't be split by a relaunch.

use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use farcooler_agent_hooks::Agent;
use farcooler_core::inventory::{FakeInventory, RuntimeInventory};
use farcooler_protocol::v1::TerminalIntent;
use farcooler_store::Store;
use uuid::Uuid;

use super::HookIngress;

fn launched() -> (HookIngress, Arc<Store>, Uuid, Uuid) {
    let store = Arc::new(Store::open_in_memory().expect("store"));
    let host = Uuid::now_v7();
    let root = store.create_repository_root(host, "/wt/gate", 1_000).unwrap();
    let repo = store.create_repository(host, root.id, "repo", "/wt/gate/.git", "").unwrap();
    let ws = store.create_worktree(repo.id, "main", "/wt/gate", false).unwrap();
    let t = store.create_terminal(ws.id, "pane", "codex", TerminalIntent::Running, 80, 24).unwrap();
    let (_, launch) = store.begin_launch(t.id, t.resource_version).unwrap();
    let inventory: Arc<dyn RuntimeInventory> = Arc::new(FakeInventory::default());
    (HookIngress::new(store.clone(), inventory, Arc::default()), store, t.id, launch.id)
}

/// Relaunch the pane, as `Service` does: the new launch committed, then what
/// the old one left cancelled.
fn relaunch(ingress: &HookIngress, store: &Store, terminal: Uuid) {
    let t = store.get_terminal(terminal).unwrap();
    store.begin_launch(terminal, t.resource_version).unwrap();
    ingress.relaunched(terminal);
}

fn prompt() -> serde_json::Value {
    serde_json::json!({ "session_id": "s", "prompt": "a turn the old process began" })
}

/// A frame admitted before the relaunch committed finishes its effects first,
/// and the relaunch then cancels what they left. It can't be the other way
/// round: effects landing on a pane the relaunch had already cleared.
#[test]
fn a_relaunch_waits_for_an_admitted_frame_then_clears_what_it_left() {
    let (ingress, store, terminal, old) = launched();
    let held = ingress.held_for(terminal, Some(old)).expect("the frame's launch is current");

    let (done, finished) = mpsc::channel();
    let relauncher = {
        let ingress = ingress.clone();
        let store = store.clone();
        std::thread::spawn(move || {
            relaunch(&ingress, &store, terminal);
            done.send(()).unwrap();
        })
    };
    assert!(
        finished.recv_timeout(Duration::from_millis(300)).is_err(),
        "the relaunch cancelled the pane while an admitted frame was still acting on it"
    );
    // The admitted frame's effect, under the gate.
    ingress.accept(terminal, Agent::Codex, "UserPromptSubmit", &prompt(), Some("s"));
    assert!(ingress.is_tracking(terminal));
    drop(held);

    finished.recv_timeout(Duration::from_secs(10)).expect("the relaunch finished once the frame had");
    relauncher.join().unwrap();
    assert!(!ingress.is_tracking(terminal), "the relaunch cleared what the old launch's frame left");
}

/// A frame whose check comes after the relaunch committed is refused at the
/// gate, even though it was routed before.
#[test]
fn a_frame_checked_after_a_relaunch_acts_on_nothing() {
    let (ingress, store, terminal, old) = launched();
    relaunch(&ingress, &store, terminal);
    assert!(ingress.held_for(terminal, Some(old)).is_none(), "the old launch may no longer act on the pane");
    let current = store.current_launch(terminal).unwrap().expect("a launch");
    assert!(ingress.held_for(terminal, Some(current)).is_some(), "the new one may");
    assert!(ingress.held_for(terminal, None).is_some(), "and a frame that names no launch is judged as before");
}
