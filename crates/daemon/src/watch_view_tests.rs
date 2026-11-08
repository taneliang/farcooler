use super::*;

fn payload(json: &str) -> farcooler_protocol::v1::WorkflowCommand {
    farcooler_protocol::v1::WorkflowCommand { schema: 1, json: json.into() }
}

#[test]
fn the_item_is_all_a_watch_can_name() {
    assert_eq!(item_of(&payload(r#"{"item":"ov-9"}"#)).as_deref(), Some("ov-9"));
    // Nothing else rides along: no keys, no size, no terminal id.
    assert_eq!(item_of(&payload(r#"{"item":"ov-9","keys":"rm -rf /\n"}"#)), None);
    assert_eq!(item_of(&payload(r#"{"item":"ov-9","columns":10}"#)), None);
}

#[test]
fn this_module_has_no_way_to_type_or_resize() {
    // The door in hands out a Runtime; what it is used for here is `attach`, which only reads.
    // If anyone wires an input verb into the watch path, this goes red.
    let source = include_str!("watch_view.rs");
    let rpc = include_str!("rpc.rs");
    let start = rpc.find("\"workflow.watch\" =>").unwrap();
    let end = rpc[start..].find("\"workflow.cutover\" =>").unwrap();
    let handler = &rpc[start..start + end];
    let code = source.split("#[cfg(test)]").next().unwrap();
    let strip = |text: &str| text.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
    let (code, handler) = (strip(code), strip(handler));
    for verb in ["send_keys", "send_literal", "send_bytes_hex", "send_bytes", "resize", "set_pane_size", "input_from", "mark_input", "paste"] {
        assert!(!code.contains(verb) && !handler.contains(verb), "the watch path mentions {verb}");
    }
    assert!(handler.contains(".attach("));
}

// ---- a real pane: a watcher sees it and cannot touch it ----

use std::sync::{Arc, Mutex};
use std::time::Duration;
use uuid::Uuid;

/// Collects what `attach` would send a phone or the Mac.
struct Collect(Arc<Mutex<Vec<u8>>>);
impl crate::resync::AttachSink for Collect {
    fn open(&mut self, picture: Vec<u8>) -> bool { self.0.lock().unwrap().extend(picture); true }
    fn output(&mut self, bytes: Vec<u8>) -> crate::resync::Taken { self.0.lock().unwrap().extend(bytes); crate::resync::Taken::Sent }
    async fn caught_up(&self) -> bool { true }
    fn resync(&mut self, picture: Vec<u8>) -> bool { self.0.lock().unwrap().extend(picture); true }
}

async fn until(what: &str, mut ok: impl FnMut() -> bool) {
    for _ in 0..600 {
        if ok() { return; }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("never saw {what}");
}

/// The pane's program records every line it is typed and its size, so a test can tell exactly what reached it.
const PROGRAM: &str = "stty size > size.txt; echo ready; while read -r line; do echo \"$line\" >> typed.txt; echo \"got $line\"; done";

#[tokio::test]
async fn a_watcher_sees_the_agent_and_nothing_it_does_reaches_the_pane() {
    if crate::runtime::fanout_binary().is_none() {
        assert!(std::env::var_os("CI").is_none(), "no farcoolerd beside the test binary");
        eprintln!("SKIP: no farcoolerd beside the test binary (run `cargo test -p farcooler-daemon`)");
        return;
    }
    let dir = farcooler_core::scratch::dir("watch-view");
    let server = farcooler_tmux::TmuxServer::in_dir(&format!("test-{}", Uuid::now_v7().simple()), Uuid::now_v7(), &dir);
    let terminal = Uuid::now_v7();
    let window = server.create_terminal_window(Uuid::now_v7(), terminal, "t", dir.to_str().unwrap(), &format!("sh -c '{PROGRAM}'")).await.unwrap();
    let pane = server.list_tagged_panes().await.unwrap().into_iter().find(|p| p.pane_id == window.pane_id).unwrap();
    let (columns, rows) = (pane.columns, pane.rows);
    let _listed = publish("ws", "ov-1", server.clone(), pane);

    // Watch it, the way `workflow.watch` does.
    let (runtime, pane) = lookup("ws", "ov-1").expect("listed while it runs");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let task = tokio::spawn({ let seen = seen.clone(); async move { let _ = runtime.attach(pane, Collect(seen)).await; } });
    let text = |seen: &Arc<Mutex<Vec<u8>>>| String::from_utf8_lossy(&seen.lock().unwrap()).into_owned();
    until("the opening picture", || text(&seen).contains("ready")).await;

    // The control: the engine's own input reaches the pane and the watcher sees the result, so the
    // checks below cannot pass merely because nothing moves.
    // Sent until the live pipe is up: the opening picture comes first and the pipe a moment later.
    for _ in 0..200 {
        server.send_keys(&window.pane_id, "engine\n").await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        if text(&seen).contains("got engine") { break; }
    }
    assert!(text(&seen).contains("got engine"), "the live stream never carried the engine's line: {:?}", text(&seen));

    // What the daemon's own terminal verbs can do with this terminal's id: it is in no inventory they read.
    let daemons = crate::runtime::Runtime::open_in(&dir.join("daemon")).await;
    if let Ok(daemons) = daemons {
        assert!(daemons.send_input(terminal, "typed by a viewer\n").await.is_err());
        assert!(daemons.resolve_terminal(&terminal.simple().to_string()).is_err());
    }
    // And a watcher has no verb of its own: the only thing the handle is used for is `attach`.
    tokio::time::sleep(Duration::from_millis(500)).await;

    let typed = std::fs::read_to_string(dir.join("typed.txt")).unwrap();
    assert!(typed.lines().all(|l| l == "engine"), "something besides the engine reached the pane: {typed:?}");
    let (now_columns, now_rows) = server.window_size(&window.window_id).await.unwrap();
    assert_eq!((now_columns, now_rows), (columns, rows), "watching resized the pane");
    assert!(std::fs::read_to_string(dir.join("size.txt")).unwrap().trim().ends_with(&columns.to_string()));

    // Gone with the run: unlisted, and the stream ends.
    drop(_listed);
    assert!(lookup("ws", "ov-1").is_none());
    task.abort();
    farcooler_tmux::reap_server(server.socket());
}
