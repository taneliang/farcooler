//! The two halves of the hook path live in different crates, which is exactly
//! how `ShimMessage::Failed` came to be declared, handled, and constructed
//! nowhere. So this drives the real frame the hook binary writes against the
//! real listener over a real socket, rather than calling `serve` in a way no
//! hook ever will.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use farcooler_agent::event::{AgentEvent, Role};
use farcooler_agent_hooks::Agent;
use farcooler_agent_hooks::facts::facts;
use farcooler_agent_hooks::wire::{HookLine, encode_line};
use farcooler_daemon::hook_ingress::HookIngress;
use farcooler_protocol::v1::TerminalIntent;
use farcooler_store::Store;
use farcooler_store::models::PaneMode;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

/// Every `(terminal, events)` the listener handed out, in order.
type Seen = Arc<Mutex<Vec<(Uuid, Vec<AgentEvent>)>>>;

/// Polled rather than slept on, for `common::listening_daemon`'s reason: the
/// bind happens on a spawned task and a fixed wait is flaky in exactly the
/// direction that wastes an hour.
async fn eventually<T>(mut f: impl FnMut() -> Option<T>) -> Option<T> {
    for _ in 0..200 {
        if let Some(v) = f() {
            return Some(v);
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    None
}

/// A store holding one workspace whose worktree is `worktree`, and one pane
/// per `(command_preset, agent_session_id)` given.
///
/// Store rows rather than a live `Service`: this file is about the socket and
/// the routing, and standing up git and tmux to reach them would make it a
/// slower test of something else.
fn store_with(worktree: &str, panes: &[(&str, Option<&str>)]) -> (Arc<Store>, Vec<Uuid>) {
    let store = Store::open_in_memory().expect("an in-memory store");
    let host = Uuid::now_v7();
    let root = store.create_repository_root(host, "/repos/hooks", 1_000).unwrap();
    let repo = store.create_repository(host, root.id, "repo", "/repos/hooks/.git", "").unwrap();
    let ws = store.create_workspace(repo.id, "feature/hooks", worktree, false).unwrap();

    let mut ids = Vec::new();
    for (preset, session) in panes {
        let term = store
            .create_terminal(ws.id, "pane", preset, TerminalIntent::Running, 80, 24)
            .unwrap();
        if let Some(session) = session {
            store
                .set_pane_mode(
                    term.id,
                    term.resource_version,
                    PaneMode::Terminal,
                    Some(session.to_string()),
                )
                .unwrap();
        }
        ids.push(term.id);
    }
    (Arc::new(store), ids)
}

fn store_with_terminal(worktree: &str, preset: &str, session: Option<&str>) -> (Arc<Store>, Uuid) {
    let (store, ids) = store_with(worktree, &[(preset, session)]);
    (store, ids[0])
}

/// What a hook of `agent` in `worktree` naming `session` reports about itself.
fn payload(agent: Agent, worktree: &str, session: &str) -> serde_json::Value {
    match agent {
        // Cursor sends no `cwd` at all; its worktree is `workspace_roots[0]`.
        Agent::Cursor => serde_json::json!({
            "session_id": session,
            "workspace_roots": [worktree],
        }),
        Agent::Claude | Agent::Codex => serde_json::json!({
            "session_id": session,
            "cwd": worktree,
        }),
    }
}

/// A bound listener, and the socket a hook would dial.
///
/// Returns once the socket exists, so a caller never races the bind.
async fn listening(store: Arc<Store>, dir: &std::path::Path) -> (std::path::PathBuf, Seen) {
    let ingress = HookIngress::new(store);
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    {
        let seen = seen.clone();
        let dir = dir.to_path_buf();
        tokio::spawn(async move {
            let _ = ingress
                .listen(&dir, move |id, events| seen.lock().unwrap().push((id, events)))
                .await;
        });
    }
    let socket = HookIngress::socket_path(dir);
    eventually(|| socket.exists().then_some(())).await.expect("the daemon bound its socket");
    (socket, seen)
}

/// Exactly what `hook.rs`'s `converse` writes: one encoded frame, and nothing
/// else.
async fn send(socket: &std::path::Path, line: &HookLine) {
    let mut stream = tokio::net::UnixStream::connect(socket).await.expect("connect");
    stream.write_all(encode_line(line).unwrap().as_bytes()).await.expect("write");
    stream.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn a_claude_session_in_a_known_worktree_binds_to_its_terminal() {
    let dir = tempfile::tempdir().unwrap();
    let (store, terminal) = store_with_terminal("/wt/bound", "claude", Some("sess-1"));
    let (socket, seen) = listening(store, dir.path()).await;

    send(
        &socket,
        &HookLine {
            agent: Agent::Claude,
            event: "UserPromptSubmit".to_string(),
            payload: serde_json::json!({ "session_id": "sess-1", "prompt": "hello" }),
        },
    )
    .await;

    let got = eventually(|| seen.lock().unwrap().first().cloned())
        .await
        .expect("the daemon routed the hook to a terminal");

    assert_eq!(got.0, terminal, "a session id binds to the terminal that declared it");
    assert_eq!(
        got.1,
        vec![AgentEvent::Message {
            role: Role::User,
            text: "hello".to_string(),
            parent: None,
        }],
        "and the prompt arrives as the event every client already renders"
    );
}

/// Never guess. An unbound session is a legitimate state and the wrong
/// terminal is not.
#[tokio::test]
async fn a_session_nothing_claims_is_dropped_rather_than_attached() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _terminal) = store_with_terminal("/wt/unclaimed", "claude", Some("sess-1"));
    let (socket, seen) = listening(store, dir.path()).await;

    send(
        &socket,
        &HookLine {
            agent: Agent::Claude,
            event: "UserPromptSubmit".to_string(),
            payload: serde_json::json!({
                "session_id": "a-session-nobody-declared",
                "prompt": "hi",
            }),
        },
    )
    .await;

    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(seen.lock().unwrap().is_empty(), "an unmatched session reaches no terminal at all");
}

/// The property `hook.rs`'s `converse` doc asserts on this side's behalf.
///
/// Its deadline can cut the hook off mid-write, and it says so: "the daemon
/// reads whole lines, and half a frame with no newline is discarded at EOF
/// rather than acted on". A truncated frame is still often VALID JSON — a
/// `PreToolUse` cut short mid-payload is not, but a frame cut at exactly the
/// closing brace is — so a reader that acts on whatever `read_line` returned
/// at EOF acts on a fragment, and does it silently. The newline is the only
/// thing that distinguishes a complete frame from a fragment that happens to
/// parse.
#[tokio::test]
async fn half_a_frame_with_no_newline_is_discarded_rather_than_acted_on() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _terminal) = store_with_terminal("/wt/truncated", "claude", Some("sess-1"));
    let (socket, seen) = listening(store, dir.path()).await;

    let line = HookLine {
        agent: Agent::Claude,
        event: "UserPromptSubmit".to_string(),
        payload: serde_json::json!({ "session_id": "sess-1", "prompt": "hello" }),
    };
    let whole = encode_line(&line).unwrap();
    let truncated = whole.trim_end_matches('\n');
    assert!(
        serde_json::from_str::<HookLine>(truncated).is_ok(),
        "the fixture is only interesting because the fragment still parses"
    );

    let mut stream = tokio::net::UnixStream::connect(&socket).await.expect("connect");
    stream.write_all(truncated.as_bytes()).await.expect("write");
    stream.shutdown().await.expect("shutdown");

    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        seen.lock().unwrap().is_empty(),
        "a frame that never ended is not a frame the daemon may act on"
    );
}

/// One connection, one hook process — but the reader must not stop at the
/// first line and leave the rest of a batch unread.
#[tokio::test]
async fn two_frames_on_one_connection_both_arrive() {
    let dir = tempfile::tempdir().unwrap();
    let (store, terminal) = store_with_terminal("/wt/two", "claude", Some("sess-1"));
    let (socket, seen) = listening(store, dir.path()).await;

    let mut both = String::new();
    for prompt in ["one", "two"] {
        both.push_str(
            &encode_line(&HookLine {
                agent: Agent::Claude,
                event: "UserPromptSubmit".to_string(),
                payload: serde_json::json!({ "session_id": "sess-1", "prompt": prompt }),
            })
            .unwrap(),
        );
    }
    let mut stream = tokio::net::UnixStream::connect(&socket).await.expect("connect");
    stream.write_all(both.as_bytes()).await.expect("write");
    stream.shutdown().await.expect("shutdown");

    let got = eventually(|| {
        let seen = seen.lock().unwrap();
        (seen.len() == 2).then(|| seen.clone())
    })
    .await
    .expect("both frames reached the terminal");
    assert!(got.iter().all(|(id, _)| *id == terminal), "both belong to the same terminal");
}

// ---- binding a session to a terminal ----

/// The refusal, at the join key.
///
/// Nothing constrains `agent_session_id` to be unique: a split pane copies it
/// and an adoption can write one somebody else already holds. Taking the first
/// row a query returns would pick by rowid — invisibly, and differently on two
/// runners — and half the time it would draw one person's conversation into
/// another pane's transcript, which reads as a working transcript.
#[tokio::test]
async fn a_session_two_terminals_both_claim_binds_to_neither() {
    let (store, ids) = store_with("/wt/ambiguous", &[("claude", Some("s")), ("claude", Some("s"))]);
    let ingress = HookIngress::new(store);
    let f = facts(Agent::Claude, &payload(Agent::Claude, "/wt/ambiguous", "s"));

    let bound = ingress.terminal_for(&f, Agent::Claude);
    assert!(
        bound.is_none(),
        "two claimants is not an answer, and {bound:?} is one of {ids:?} chosen at random"
    );
}

/// Codex announces, so a worktree match is the only binding it has.
#[tokio::test]
async fn a_codex_session_binds_to_the_codex_pane_in_its_worktree() {
    let (store, ids) = store_with(
        "/wt/announce",
        &[("shell", None), ("claude", Some("claude-s")), ("codex:gpt-5.6-terra", None)],
    );
    let ingress = HookIngress::new(store);
    let f = facts(Agent::Codex, &payload(Agent::Codex, "/wt/announce", "brand-new"));

    assert_eq!(
        ingress.terminal_for(&f, Agent::Codex),
        Some(ids[2]),
        "the pane in that worktree running that agent, and not its neighbors"
    );
}

/// Cursor's worktree is `workspace_roots[0]`, which is the asymmetry that
/// would otherwise be found in production: a fallback written against `cwd`
/// finds nothing and leaves every cursor session unattached, silently.
#[tokio::test]
async fn a_cursor_session_binds_through_workspace_roots() {
    let (store, ids) = store_with("/wt/cursor", &[("cursor", None)]);
    let ingress = HookIngress::new(store);
    let f = facts(Agent::Cursor, &payload(Agent::Cursor, "/wt/cursor", "brand-new"));

    assert_eq!(ingress.terminal_for(&f, Agent::Cursor), Some(ids[0]));
}

/// Claude gets no worktree fallback, because it never needs one.
///
/// `preset_command` passes `--session-id` and `create_terminal` mints it, so a
/// claude session no row claims is one somebody started by hand — in a pane
/// this daemon may know nothing about. Falling back would hand that
/// conversation to whichever claude pane happened to share the directory.
#[tokio::test]
async fn a_claude_session_no_row_claims_is_never_guessed_from_the_worktree() {
    let (store, _ids) = store_with("/wt/handstarted", &[("claude", None)]);
    let ingress = HookIngress::new(store);
    let f = facts(Agent::Claude, &payload(Agent::Claude, "/wt/handstarted", "started-by-hand"));

    assert_eq!(ingress.terminal_for(&f, Agent::Claude), None);
}

/// Two panes of one agent in one worktree tell the announcement nothing.
#[tokio::test]
async fn two_codex_panes_in_one_worktree_leave_an_announcement_unattached() {
    let (store, _ids) = store_with("/wt/twocodex", &[("codex", None), ("codex", None)]);
    let ingress = HookIngress::new(store);
    let f = facts(Agent::Codex, &payload(Agent::Codex, "/wt/twocodex", "brand-new"));

    assert_eq!(ingress.terminal_for(&f, Agent::Codex), None);
}

/// A pane already speaking for a conversation must not be handed a second one.
#[tokio::test]
async fn an_announcement_passes_over_a_codex_pane_that_already_names_a_session() {
    let (store, _ids) = store_with("/wt/taken", &[("codex", Some("someone-elses"))]);
    let ingress = HookIngress::new(store);
    let f = facts(Agent::Codex, &payload(Agent::Codex, "/wt/taken", "brand-new"));

    assert_eq!(ingress.terminal_for(&f, Agent::Codex), None);
}

/// The macOS trap: `/var/folders/...` and `/private/var/folders/...` are one
/// directory reached through a symlink, and which spelling arrives depends on
/// who resolved the path. A string comparison leaves every session in such a
/// worktree unattached and says nothing about it.
#[tokio::test]
async fn a_worktree_reached_through_a_symlink_is_still_the_same_worktree() {
    let dir = tempfile::tempdir().unwrap();
    let as_given = dir.path().to_path_buf();
    let resolved = std::fs::canonicalize(&as_given).expect("a real directory resolves");

    let (store, ids) = store_with(&resolved.display().to_string(), &[("codex", None)]);
    let ingress = HookIngress::new(store);
    let f = facts(Agent::Codex, &payload(Agent::Codex, &as_given.display().to_string(), "new"));

    assert_eq!(
        ingress.terminal_for(&f, Agent::Codex),
        Some(ids[0]),
        "{} and {} are one worktree",
        as_given.display(),
        resolved.display()
    );
}

/// The worktree is what the announcement is FOR.
///
/// Every other test in this file has one workspace in the store, and with one
/// workspace a fallback that never looked at the worktree at all still binds
/// correctly — it would go wrong only on a runner with a second one, which is
/// every real runner. Two codex panes, one in each worktree, and only the
/// worktree tells them apart.
#[tokio::test]
async fn an_announcement_binds_inside_its_own_worktree_and_not_the_one_next_door() {
    let store = Store::open_in_memory().expect("an in-memory store");
    let host = Uuid::now_v7();
    let root = store.create_repository_root(host, "/repos/hooks", 1_000).unwrap();
    let repo = store.create_repository(host, root.id, "repo", "/repos/hooks/.git", "").unwrap();

    let mut panes = Vec::new();
    for worktree in ["/wt/left", "/wt/right"] {
        let ws = store.create_workspace(repo.id, "branch", worktree, false).unwrap();
        let term = store
            .create_terminal(ws.id, "pane", "codex", TerminalIntent::Running, 80, 24)
            .unwrap();
        panes.push(term.id);
    }
    let ingress = HookIngress::new(Arc::new(store));

    for (worktree, expected) in [("/wt/left", panes[0]), ("/wt/right", panes[1])] {
        let f = facts(Agent::Codex, &payload(Agent::Codex, worktree, "brand-new"));
        assert_eq!(
            ingress.terminal_for(&f, Agent::Codex),
            Some(expected),
            "a session announcing itself in {worktree} belongs to the pane that is in it"
        );
    }
}
