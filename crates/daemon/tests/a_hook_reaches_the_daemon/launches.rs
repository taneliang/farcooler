//! A restarted pane's old process can't reach it (ov-886).
//!
//! Every launch of an agent into a pane gets a fresh launch id, and `farcooler
//! hook` copies it into each frame (`HookLine::launch`). These drive real
//! recorded payloads from each harness, with a launch added the way the hook
//! binary adds it, against the real listener.

use super::*;

/// The spike's pane, launched twice: a first launch, and the restart that
/// retired it. Returns `(retired, current)`.
fn restarted(s: &Spike) -> (Uuid, Uuid) {
    let t = s.store.get_terminal(s.terminal).unwrap();
    let (t, first) = s.store.begin_launch(t.id, t.resource_version).unwrap();
    let (_, second) = s.store.begin_launch(t.id, t.resource_version).unwrap();
    (first.id, second.id)
}

/// `line` as a hook in a pane launched as `launch` sends it.
fn from(launch: Uuid, mut line: HookLine) -> HookLine {
    line.launch = Some(launch.to_string());
    line
}

/// `line` with `prompt` as its prompt.
fn saying(mut line: HookLine, prompt: &str) -> HookLine {
    line.payload["prompt"] = serde_json::json!(prompt);
    line
}

/// Every user message that reached `terminal`.
fn prompts_on(seen: &Seen, terminal: Uuid) -> Vec<String> {
    seen.lock()
        .unwrap()
        .iter()
        .filter(|(id, _)| *id == terminal)
        .flat_map(|(_, events)| events)
        .filter_map(|e| match e {
            AgentEvent::Message { role: Role::User, text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// Send `lines` on one connection, then wait until `sentinel` has reached
/// `terminal`: served in order, so whatever came before it has landed too.
async fn served(seen: &Seen, socket: &std::path::Path, terminal: Uuid, lines: &[HookLine]) -> Vec<String> {
    send_all(socket, lines).await;
    let arrived = eventually(|| prompts_on(seen, terminal).iter().any(|p| p == SENTINEL).then_some(())).await;
    assert!(arrived.is_some(), "the current launch's sentinel never arrived: {:?}", seen.lock().unwrap());
    prompts_on(seen, terminal)
}

/// The fixture and the session a restart of each harness keeps. Claude's row
/// names the session it resumes; Codex's and Cursor's rows name none.
const HARNESSES: [(&str, &str, Option<&str>); 3] = [
    ("claude", "claude-before-move", Some("00000000-0000-4000-8000-000000000001")),
    ("codex", "codex-before-move", None),
    ("cursor", "cursor-before-move", None),
];

/// The card's first case. A restarted Claude or Codex resumes the SAME
/// session, so nothing in the payload tells the killed process's last hook
/// from the new one's; only the launch does.
#[tokio::test]
async fn a_hook_from_the_launch_a_restart_retired_reaches_nothing() {
    let mut reached = Vec::new();
    for (preset, fixture, session) in HARNESSES {
        let s = spike(preset, session);
        let (retired, current) = restarted(&s);
        let sock = tempfile::tempdir().unwrap();
        let (socket, seen) = listening(s.store.clone(), &[s.terminal], sock.path()).await;
        let line = recorded(fixture, &s.root);
        let prompts = served(
            &seen,
            &socket,
            s.terminal,
            &[from(retired, saying(line.clone(), "from the killed process")), from(current, saying(line, SENTINEL))],
        )
        .await;
        if prompts != vec![SENTINEL.to_string()] {
            reached.push((preset, prompts));
        }
    }
    assert!(reached.is_empty(), "the retired launch's hook reached the pane: {reached:?}");
}

/// A retired launch's permission ask is answered "no decision" at once, and
/// nothing is held on the pane for a phone to be offered.
#[tokio::test]
async fn a_retired_launchs_ask_is_never_held() {
    let s = spike("claude", Some("sess-1"));
    let (retired, _) = restarted(&s);
    let sock = tempfile::tempdir().unwrap();
    let ingress = ingress_claiming(s.store.clone(), &[s.terminal]);
    let asks = ingress.asks().clone();
    let (socket, _seen) = listening_on(ingress, sock.path()).await;

    let mut ask = a_permission_request(Agent::Claude, "sess-1");
    ask.payload["cwd"] = serde_json::json!(s.root.join("repo"));
    let mut stream = tokio::net::UnixStream::connect(&socket).await.expect("connect");
    stream.write_all(encode_line(&from(retired, ask)).unwrap().as_bytes()).await.expect("write");
    let mut reader = tokio::io::BufReader::new(stream);
    let mut reply = String::new();
    tokio::time::timeout(A_LINE, tokio::io::AsyncBufReadExt::read_line(&mut reader, &mut reply))
        .await
        .expect("an answer inside the hook's wait")
        .expect("read");
    assert_eq!(reply.trim(), "{}", "no decision, and no hold");
    assert!(asks.open_on(s.terminal).is_none(), "nothing is held on the pane");
}

/// Two Codex panes in one worktree: by worktree alone neither can be told
/// from the other, so neither ever heard a hook. By launch, each does.
#[tokio::test]
async fn a_hook_from_the_current_launch_reaches_its_pane_beside_a_twin() {
    let s = spike("codex", None);
    let checkout = s.store.get_terminal(s.terminal).unwrap().worktree_id;
    let twin = s.store.create_terminal(checkout, "twin", "codex", TerminalIntent::Running, 80, 24).unwrap();
    let (_, current) = restarted(&s);
    let sock = tempfile::tempdir().unwrap();
    let (socket, seen) = listening(s.store.clone(), &[s.terminal, twin.id], sock.path()).await;
    let line = recorded("codex-before-move", &s.root);
    let prompts = served(&seen, &socket, s.terminal, &[from(current, saying(line, SENTINEL))]).await;
    assert_eq!(prompts, vec![SENTINEL.to_string()]);
    assert!(prompts_on(&seen, twin.id).is_empty(), "the twin heard nothing");
}

/// A launch is inherited by everything the agent starts. A `claude` someone
/// runs by hand inside a Claude pane carries the pane's launch, under a
/// session the row doesn't name and Claude's registry doesn't place there: it
/// isn't pulled into the pane.
#[tokio::test]
async fn a_claude_started_inside_a_pane_is_not_pulled_into_it() {
    let s = spike("claude", Some("00000000-0000-4000-8000-000000000001"));
    let (_, current) = restarted(&s);
    let sock = tempfile::tempdir().unwrap();
    let (socket, seen) = listening(s.store.clone(), &[s.terminal], sock.path()).await;
    let line = recorded("claude-before-move", &s.root);
    let mut nested = saying(line.clone(), "from a claude started by hand");
    nested.payload["session_id"] = serde_json::json!("00000000-0000-4000-8000-0000000000ff");
    let prompts = served(&seen, &socket, s.terminal, &[from(current, nested), from(current, saying(line, SENTINEL))]).await;
    assert_eq!(prompts, vec![SENTINEL.to_string()]);
    assert_eq!(
        s.store.get_terminal(s.terminal).unwrap().agent_session_id.as_deref(),
        Some("00000000-0000-4000-8000-000000000001"),
        "the pane still names its own session"
    );
}

/// A Codex launched in another worktree that somehow carries this pane's
/// launch (a launch id is only an environment variable) isn't this pane's.
#[tokio::test]
async fn a_launch_carried_out_of_its_worktree_reaches_nothing() {
    let s = spike("codex", None);
    let (_, current) = restarted(&s);
    let sock = tempfile::tempdir().unwrap();
    let (socket, seen) = listening(s.store.clone(), &[s.terminal], sock.path()).await;
    let line = recorded("codex-before-move", &s.root);
    let mut elsewhere = saying(line.clone(), "from another worktree");
    elsewhere.payload["cwd"] = serde_json::json!(s.root.join("repo").join(".worktrees").join("nested"));
    let prompts = served(&seen, &socket, s.terminal, &[from(current, elsewhere), from(current, saying(line, SENTINEL))]).await;
    assert_eq!(prompts, vec![SENTINEL.to_string()]);
}
