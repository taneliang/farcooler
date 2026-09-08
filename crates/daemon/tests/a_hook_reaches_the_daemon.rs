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
use farcooler_core::inventory::{FakeInventory, RuntimeSnapshot, TaggedPane};
use farcooler_daemon::hook_ingress::HookIngress;
use farcooler_protocol::v1::TerminalIntent;
use farcooler_store::{Store, TerminalUpdate};
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
    let with_intent: Vec<_> =
        panes.iter().map(|(p, s)| (*p, *s, TerminalIntent::Running)).collect();
    store_with_intents(worktree, &with_intent)
}

/// The same, for a test that needs a pane the user has stopped.
fn store_with_intents(
    worktree: &str,
    panes: &[(&str, Option<&str>, TerminalIntent)],
) -> (Arc<Store>, Vec<Uuid>) {
    let store = Store::open_in_memory().expect("an in-memory store");
    let host = Uuid::now_v7();
    let root = store.create_repository_root(host, "/repos/hooks", 1_000).unwrap();
    let repo = store.create_repository(host, root.id, "repo", "/repos/hooks/.git", "").unwrap();
    let ws = store.create_workspace(repo.id, "feature/hooks", worktree, false).unwrap();

    let mut ids = Vec::new();
    for (preset, session, intent) in panes {
        let term = store.create_terminal(ws.id, "pane", preset, *intent, 80, 24).unwrap();
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

/// An ingress whose tmux view is `snapshot`.
///
/// `FakeInventory` is the repo's own double for exactly this — it is how
/// `derive` is unit-tested with no tmux present — so a test can state the
/// runtime view it means rather than depending on whether the machine running
/// it happens to have a server up.
fn ingress_over(store: Arc<Store>, snapshot: RuntimeSnapshot) -> HookIngress {
    HookIngress::new(store, Arc::new(FakeInventory { snapshot }))
}

/// The default view: tmux is readable, and it claims a live pane for each of
/// `live`.
///
/// This is the fixture almost every test here should be standing on, and
/// getting it wrong once would be invisible. An ingress over an UNAVAILABLE
/// inventory derives every `Running` row as `unknown`, and `unknown` is
/// deliberately kept as a candidate — so under that view `still_a_pane` says
/// yes to everything and is not a filter at all. Every announce test sharing
/// that one default would agree just as happily with an implementation that
/// had no liveness rule in it, which is a fixture too uniform to tell two
/// implementations apart rather than a set of independent probes. The relay's
/// account-scoping tests were wrong in precisely this shape three times: each
/// held one account's rows, so a scoped query and an unscoped one were
/// literally the same query.
fn ingress_claiming(store: Arc<Store>, live: &[Uuid]) -> HookIngress {
    let panes = live.iter().copied().map(live_pane).collect();
    ingress_over(store, RuntimeSnapshot::healthy(panes))
}

/// Mark a terminal as one the daemon has seen alive at least once.
///
/// Load-bearing for `lost`, which is a FINDING: a row that was never confirmed
/// and has no pane derives `starting` — it may simply not have come up yet —
/// and only a row that WAS confirmed and is now claimed by nothing is dead.
fn confirm(store: &Store, id: Uuid) {
    let t = store.get_terminal(id).unwrap();
    store
        .update_terminal(
            id,
            t.resource_version,
            TerminalUpdate {
                title: t.title.clone(),
                command_preset: t.command_preset.clone(),
                intent: t.intent,
                runtime_confirmed: true,
                exit_code: t.exit_code,
                exit_signal: t.exit_signal,
                lease_generation: t.lease_generation,
                epoch: t.epoch,
                columns: t.columns,
                rows: t.rows,
            },
        )
        .unwrap();
}

/// A tmux pane claiming `terminal_id`, alive.
fn live_pane(terminal_id: Uuid) -> TaggedPane {
    TaggedPane {
        daemon_id: Uuid::now_v7(),
        workspace_id: Uuid::now_v7(),
        terminal_id,
        schema_version: 1,
        pane_id: "%1".into(),
        window_id: "@1".into(),
        columns: 80,
        rows: 24,
        left: 0,
        top: 0,
        window_active: true,
        pane_active: true,
        zoomed: false,
        tty: String::new(),
        dead: false,
        dead_status: None,
        command: "codex".into(),
        title: String::new(),
    }
}

/// A pane tmux is still showing whose command has exited.
///
/// `remain-on-exit` keeps it, which is the whole reason a clean exit is
/// distinguishable from a loss — so this is an OBSERVED death rather than an
/// absence, and it derives `exited` where the same row with no pane at all
/// would derive `lost`.
fn dead_pane(terminal_id: Uuid) -> TaggedPane {
    TaggedPane { dead: true, dead_status: Some(0), ..live_pane(terminal_id) }
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
async fn listening(
    store: Arc<Store>,
    live: &[Uuid],
    dir: &std::path::Path,
) -> (std::path::PathBuf, Seen) {
    let ingress = ingress_claiming(store, live);
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
    let (socket, seen) = listening(store, &[terminal], dir.path()).await;

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
    let (store, terminal) = store_with_terminal("/wt/unclaimed", "claude", Some("sess-1"));
    let (socket, seen) = listening(store, &[terminal], dir.path()).await;

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
    let (store, terminal) = store_with_terminal("/wt/truncated", "claude", Some("sess-1"));
    let (socket, seen) = listening(store, &[terminal], dir.path()).await;

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
    let (socket, seen) = listening(store, &[terminal], dir.path()).await;

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
    let ingress = ingress_claiming(store, &ids);
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
    let ingress = ingress_claiming(store, &ids);
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
    let ingress = ingress_claiming(store, &ids);
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
    let (store, ids) = store_with("/wt/handstarted", &[("claude", None)]);
    let ingress = ingress_claiming(store, &ids);
    let f = facts(Agent::Claude, &payload(Agent::Claude, "/wt/handstarted", "started-by-hand"));

    assert_eq!(ingress.terminal_for(&f, Agent::Claude), None);
}

/// Two panes of one agent in one worktree tell the announcement nothing.
#[tokio::test]
async fn two_codex_panes_in_one_worktree_leave_an_announcement_unattached() {
    let (store, ids) = store_with("/wt/twocodex", &[("codex", None), ("codex", None)]);
    let ingress = ingress_claiming(store, &ids);
    let f = facts(Agent::Codex, &payload(Agent::Codex, "/wt/twocodex", "brand-new"));

    assert_eq!(ingress.terminal_for(&f, Agent::Codex), None);
}

/// A pane already speaking for a conversation must not be handed a second one.
#[tokio::test]
async fn an_announcement_passes_over_a_codex_pane_that_already_names_a_session() {
    let (store, ids) = store_with("/wt/taken", &[("codex", Some("someone-elses"))]);
    let ingress = ingress_claiming(store, &ids);
    let f = facts(Agent::Codex, &payload(Agent::Codex, "/wt/taken", "brand-new"));

    assert_eq!(ingress.terminal_for(&f, Agent::Codex), None);
}

/// Two spellings of one directory, made deliberately rather than borrowed
/// from the platform.
///
/// The real case is macOS, where `/tmp` is a symlink to `/private/tmp` and
/// `$TMPDIR` sits under another one, so the `cwd` an agent reports and the
/// path git recorded differ by a symlink neither side chose. But
/// `tempfile::tempdir()` hands back a real `/tmp/...` path on Linux, where
/// canonicalization is the identity — so a test that leaned on the platform
/// for its symlink would prove nothing on the ubuntu leg of CI and bite only
/// on the macOS one. This builds the link itself.
#[tokio::test]
async fn a_worktree_reached_through_a_symlink_is_still_the_same_worktree() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("worktree");
    std::fs::create_dir(&real).unwrap();
    let link = dir.path().join("reached-through-here");
    std::os::unix::fs::symlink(&real, &link).unwrap();

    let resolved = std::fs::canonicalize(&real).expect("a real directory resolves");
    assert_ne!(
        link, resolved,
        "the fixture is only interesting because the two spellings differ before resolution"
    );

    let (store, ids) = store_with(&resolved.display().to_string(), &[("codex", None)]);
    let ingress = ingress_claiming(store, &ids);
    let f = facts(Agent::Codex, &payload(Agent::Codex, &link.display().to_string(), "new"));

    assert_eq!(
        ingress.terminal_for(&f, Agent::Codex),
        Some(ids[0]),
        "{} and {} are one worktree",
        link.display(),
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
    let ingress = ingress_claiming(Arc::new(store), &panes);

    for (worktree, expected) in [("/wt/left", panes[0]), ("/wt/right", panes[1])] {
        let f = facts(Agent::Codex, &payload(Agent::Codex, worktree, "brand-new"));
        assert_eq!(
            ingress.terminal_for(&f, Agent::Codex),
            Some(expected),
            "a session announcing itself in {worktree} belongs to the pane that is in it"
        );
    }
}

/// Hiding a workspace hides a card. It does not stop the panes inside it.
///
/// `hide_workspace` only sets a flag and never touches git or tmux, so an
/// agent in a hidden worktree keeps running and keeps firing hooks. Reading
/// the workspaces through `list_all_workspaces` — which filters `hidden = 0`
/// and says in its own doc that it is for summaries — would leave every codex
/// and cursor session in a hidden worktree permanently unattached while the
/// claude fast path, which never consults a workspace at all, kept working.
/// Nobody could guess that asymmetry from the symptom.
#[tokio::test]
async fn a_hidden_workspace_still_holds_panes_an_announcement_must_reach() {
    let store = Store::open_in_memory().expect("an in-memory store");
    let host = Uuid::now_v7();
    let root = store.create_repository_root(host, "/repos/hooks", 1_000).unwrap();
    let repo = store.create_repository(host, root.id, "repo", "/repos/hooks/.git", "").unwrap();
    let ws = store.create_workspace(repo.id, "branch", "/wt/hidden", false).unwrap();
    let term =
        store.create_terminal(ws.id, "pane", "codex", TerminalIntent::Running, 80, 24).unwrap();
    let hidden = store.set_workspace_flags(ws.id, ws.resource_version, true, false).unwrap();
    assert!(hidden.hidden, "the fixture is only interesting if the workspace really is hidden");

    let ingress = ingress_claiming(Arc::new(store), &[term.id]);
    let f = facts(Agent::Codex, &payload(Agent::Codex, "/wt/hidden", "brand-new"));

    assert_eq!(
        ingress.terminal_for(&f, Agent::Codex),
        Some(term.id),
        "a pane the user merely stopped looking at is still a pane the session is in"
    );
}

// ---- a row that outlived its pane ----

/// A pane the user stopped is not a candidate for anybody's announcement.
///
/// Nothing auto-reaps a terminal's row. A worktree that has held a codex pane
/// and then a second one accumulates two rows, and an announce path that
/// counted both would find two candidates and bind NEITHER — permanently,
/// silently, and in the direction nobody looks, because a refusal to bind is
/// also what an ordinary unmanaged pane looks like. The poisoning is worse
/// than the misbinding it was guarding against.
#[tokio::test]
async fn a_stopped_pane_does_not_poison_its_worktree_for_the_live_one() {
    let (store, ids) = store_with_intents(
        "/wt/stopped",
        &[
            ("codex", None, TerminalIntent::Stopped),
            ("codex", None, TerminalIntent::Running),
        ],
    );
    let ingress = ingress_claiming(store, &ids);
    let f = facts(Agent::Codex, &payload(Agent::Codex, "/wt/stopped", "brand-new"));

    assert_eq!(
        ingress.terminal_for(&f, Agent::Codex),
        Some(ids[1]),
        "one live candidate and one dead row is not an ambiguity"
    );
}

/// The same for a pane that simply went away.
///
/// Nothing writes an exit onto a row when a process ends on its own — the
/// daemon records intent, and `derive` reads tmux for the rest — so a codex
/// that quit still carries `intent = Running` and looks, to the store alone,
/// exactly like the live one beside it. Only the runtime view separates them.
#[tokio::test]
async fn a_lost_pane_does_not_poison_its_worktree_for_the_live_one() {
    let (store, ids) = store_with("/wt/lost", &[("codex", None), ("codex", None)]);
    // Both rows say `Running` and both have been seen alive; tmux claims only
    // the second. A healthy inventory is what makes the first a FINDING of
    // death rather than an absence of information.
    confirm(&store, ids[0]);
    confirm(&store, ids[1]);
    let ingress = ingress_over(store, RuntimeSnapshot::healthy(vec![live_pane(ids[1])]));
    let f = facts(Agent::Codex, &payload(Agent::Codex, "/wt/lost", "brand-new"));

    assert_eq!(
        ingress.terminal_for(&f, Agent::Codex),
        Some(ids[1]),
        "the row tmux still claims is the only candidate"
    );
}

/// An inventory that could not be read is not proof of death.
///
/// `derive_terminal`'s own note: reporting `lost` from a failed read "says
/// every terminal on the runner has died". tmux answers one request at a time
/// per server, so a single wedged pane turns every read unhealthy for a few
/// seconds — and if that silently stopped every announcement binding, the
/// feature would fail exactly when the runner is busiest.
#[tokio::test]
async fn an_unreadable_inventory_does_not_retire_a_pane() {
    let (store, ids) = store_with("/wt/unknown", &[("codex", None)]);
    let ingress = ingress_over(store, RuntimeSnapshot::unavailable());
    let f = facts(Agent::Codex, &payload(Agent::Codex, "/wt/unknown", "brand-new"));

    assert_eq!(
        ingress.terminal_for(&f, Agent::Codex),
        Some(ids[0]),
        "an unreadable inventory must not read as a dead pane"
    );
}

/// A frame the daemon cannot read must not cost it the rest of the batch.
///
/// The nearest wrong reader returns on a decode failure instead of
/// continuing, and `two_frames_on_one_connection_both_arrive` cannot notice
/// because both of its frames are valid.
#[tokio::test]
async fn a_frame_that_cannot_be_read_does_not_swallow_the_one_behind_it() {
    let dir = tempfile::tempdir().unwrap();
    let (store, terminal) = store_with_terminal("/wt/garbage", "claude", Some("sess-1"));
    let (socket, seen) = listening(store, &[terminal], dir.path()).await;

    let mut both = String::from("{\"this\":\"is not a HookLine\"}\n");
    both.push_str(
        &encode_line(&HookLine {
            agent: Agent::Claude,
            event: "UserPromptSubmit".to_string(),
            payload: serde_json::json!({ "session_id": "sess-1", "prompt": "after the garbage" }),
        })
        .unwrap(),
    );

    let mut stream = tokio::net::UnixStream::connect(&socket).await.expect("connect");
    stream.write_all(both.as_bytes()).await.expect("write");
    stream.shutdown().await.expect("shutdown");

    let got = eventually(|| seen.lock().unwrap().first().cloned())
        .await
        .expect("the frame behind the unreadable one still arrived");
    assert_eq!(got.0, terminal);
    assert_eq!(
        got.1,
        vec![AgentEvent::Message {
            role: Role::User,
            text: "after the garbage".to_string(),
            parent: None,
        }]
    );
}

// ---- what a person reading the logs can find out ----

/// Everything written while `f` runs, as a person would read it.
fn captured(f: impl FnOnce()) -> String {
    #[derive(Clone)]
    struct Shared(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Shared {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl tracing_subscriber::fmt::MakeWriter<'_> for Shared {
        type Writer = Self;
        fn make_writer(&self) -> Self {
            self.clone()
        }
    }

    let buffer = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_writer(Shared(buffer.clone()))
        // The daemon's own default (`main.rs`: `farcooler=info,warn`). A test
        // that turned on `debug` would pass against a module whose every
        // diagnostic is invisible in production, which is the whole defect.
        .with_max_level(tracing::Level::INFO)
        .finish();
    tracing::subscriber::with_default(subscriber, f);
    String::from_utf8(buffer.lock().unwrap().clone()).unwrap()
}

/// Which conversation ended up on which pane must have an answer afterwards.
///
/// It is the one fact with no other record. The hook exits 0 and prints
/// nothing whatever happens, and a binding that went to the wrong pane renders
/// as a perfectly ordinary transcript — so if this is not written down at a
/// level the daemon actually runs at, a runner where binding is broken has no
/// observable symptom anywhere. `set_pane_mode`'s adoption path learned the
/// same thing and logs at the same level: "the success path was the silent
/// one ... an ADOPTION recorded nothing at all, so 'which session did it pick'
/// had no answer after the fact."
#[tokio::test]
async fn the_first_binding_of_a_session_to_a_terminal_is_recorded() {
    let (store, terminal) = store_with_terminal("/wt/logged", "claude", Some("sess-1"));
    let ingress = ingress_claiming(store, &[terminal]);

    let log = captured(|| {
        ingress.accept(
            terminal,
            Agent::Claude,
            "UserPromptSubmit",
            &serde_json::json!({ "prompt": "hello" }),
            Some("sess-1"),
        );
    });

    assert!(log.contains("INFO"), "a debug line is invisible at the level the daemon runs: {log}");
    assert!(log.contains("sess-1"), "the log has to name the session it bound: {log}");
    assert!(log.contains(&terminal.to_string()), "and the terminal it bound it to: {log}");
}

/// The transition, not the flush.
///
/// Claude flushes roughly every two seconds for the length of an answer. A
/// line per flush would bury the one binding anybody wants to find, and would
/// make the log grow with the conversation rather than with the fleet.
#[tokio::test]
async fn the_binding_is_recorded_once_and_not_on_every_flush() {
    let (store, terminal) = store_with_terminal("/wt/logged-once", "claude", Some("sess-1"));
    let ingress = ingress_claiming(store, &[terminal]);

    let log = captured(|| {
        for _ in 0..5 {
            ingress.accept(
                terminal,
                Agent::Claude,
                "UserPromptSubmit",
                &serde_json::json!({ "prompt": "hello" }),
                Some("sess-1"),
            );
        }
    });

    assert_eq!(
        log.matches("a live agent session is bound").count(),
        1,
        "five hooks, one binding: {log}"
    );
}

/// An ordinary unbound session is not a problem, and must not be logged as
/// one.
///
/// The two arms have to be distinct in both directions. A `warn!` on every
/// session Far Cooler does not manage would fire constantly on any runner
/// where somebody runs an agent in a pane of their own — and a warning that
/// cries wolf is how the one that matters gets ignored. So the store-error
/// arm warns and this one does not.
#[tokio::test]
async fn a_session_nobody_claims_is_not_logged_as_a_fault() {
    let (store, ids) = store_with("/wt/quiet", &[("claude", Some("sess-1"))]);
    let ingress = ingress_claiming(store, &ids);
    let f = facts(Agent::Claude, &payload(Agent::Claude, "/wt/quiet", "started-by-hand"));

    let log = captured(|| {
        assert_eq!(ingress.terminal_for(&f, Agent::Claude), None);
    });

    assert!(
        log.is_empty(),
        "a pane Far Cooler does not manage is an ordinary thing, not a fault: {log}"
    );
}

/// A pane tmux still shows, whose command exited, is not a candidate either.
///
/// The third way into the exclusion, and a different one: `stopped` is intent,
/// `lost` is an absence, and this is a pane sitting right there in the layout
/// with a dead process in it. `remain-on-exit` keeps it deliberately, so it is
/// the case most likely to be looked at and mistaken for alive.
#[tokio::test]
async fn a_retained_dead_pane_does_not_poison_its_worktree_for_the_live_one() {
    let (store, ids) = store_with("/wt/retained", &[("codex", None), ("codex", None)]);
    let ingress =
        ingress_over(store, RuntimeSnapshot::healthy(vec![dead_pane(ids[0]), live_pane(ids[1])]));
    let f = facts(Agent::Codex, &payload(Agent::Codex, "/wt/retained", "brand-new"));

    assert_eq!(
        ingress.terminal_for(&f, Agent::Codex),
        Some(ids[1]),
        "a pane still on the screen with a dead process in it is not somewhere a session is"
    );
}

/// And the fourth: a pane that never started.
///
/// `intent = Failed` is what `create_terminal` records when the launch itself
/// did not work. It derives `error` with no reference to tmux at all, which is
/// why it needs its own test — the exclusion list names three states and each
/// is reached down a different path.
#[tokio::test]
async fn a_pane_that_failed_to_launch_does_not_poison_its_worktree() {
    let (store, ids) = store_with_intents(
        "/wt/failed",
        &[("codex", None, TerminalIntent::Failed), ("codex", None, TerminalIntent::Running)],
    );
    let ingress = ingress_claiming(store, &ids);
    let f = facts(Agent::Codex, &payload(Agent::Codex, "/wt/failed", "brand-new"));

    assert_eq!(
        ingress.terminal_for(&f, Agent::Codex),
        Some(ids[1]),
        "a pane whose launch failed is not a pane a session could be in"
    );
}

/// A peer that says nothing at all does not hold a descriptor forever.
///
/// This is the input to the exhaustion the accept loop now has to survive: a
/// hook process that leaked, or a client that connected and died, pins a
/// descriptor and a task for the life of the daemon. Nothing else reclaims
/// one — there is no other timeout anywhere on this path.
///
/// `start_paused` because the bound is deliberately long: tokio auto-advances
/// its clock whenever every task is parked, which is exactly what this
/// situation is, so the whole test runs in no wall time at all.
#[tokio::test(start_paused = true)]
async fn a_connection_that_says_nothing_is_eventually_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let (store, terminal) = store_with_terminal("/wt/idle", "claude", Some("sess-1"));
    let (socket, _seen) = listening(store, &[terminal], dir.path()).await;

    let mut stream = tokio::net::UnixStream::connect(&socket).await.expect("connect");

    // Not a write of nothing — no write at all, which is what a hook that was
    // killed between `connect` and `write_all` leaves behind.
    //
    // The outer hour is what makes a missing bound a FAILURE rather than a
    // hang: with no timer on the daemon's side there is nothing for the
    // paused clock to advance to, and this test would park forever instead of
    // saying anything. It is far past the daemon's own bound, so it never
    // fires while that one exists — and on a paused clock an hour costs
    // nothing.
    let mut buf = [0u8; 1];
    let read = tokio::time::timeout(
        Duration::from_secs(3600),
        tokio::io::AsyncReadExt::read(&mut stream, &mut buf),
    )
    .await
    .expect("the daemon never let go of a connection that said nothing")
    .expect("read");

    assert_eq!(read, 0, "the daemon let go of a connection that never said anything");
}
