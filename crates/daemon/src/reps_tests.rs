//! Starting a member's rep (ov-494), on the fixture's real tmux server with
//! the test stub in place of claude.

use std::sync::Arc;

use farcooler_protocol::v1::{self as pb, Request, Scope, request, response};
use farcooler_transport::Peer;

use super::*;
use crate::test_support::ScratchDir;
use crate::wire::id_bytes;

async fn runner() -> (ScratchDir, Arc<Service>, Uuid) {
    let (dir, svc, repo) = crate::test_support::fixture().await;
    crate::reconcile::repository(&svc, repo).await.unwrap();
    let main = svc.store.ensure_main_workspace(repo).unwrap().id;
    svc.store.set_member("alice", "Alice Chen", &["phone-a".to_string()]).unwrap();
    svc.store.set_member("bob", "Bob", &["phone-b".to_string()]).unwrap();
    // The account, where a test's service reads it: under its own root,
    // never the suite runner's own Claude config (ov-508).
    let config = svc.root_dir().join("claude");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(config.join(".claude.json"), r#"{"oauthAccount":{"emailAddress":"owner@example.com"}}"#).unwrap();
    (dir, svc, main)
}

/// What tmux was told to run in `terminal`'s pane.
async fn launched(svc: &Service, terminal: Uuid) -> String {
    let snapshot = svc.inventory.refresh().await;
    let pane = snapshot.claimants(terminal).into_iter().next().expect("a pane").clone();
    let out = svc.tmux.run(&["display-message", "-p", "-t", &pane.pane_id, "#{pane_start_command}"]).await.unwrap();
    out.stdout.trim().to_string()
}

/// The first message names the skill, the person, the workspace and the
/// account everyone shares, or says it's the runner's when it can't be read.
#[test]
fn a_reps_first_message_says_who_and_whose_account() {
    assert_eq!(
        first_prompt("Alice Chen", "alice", "Main", Some("team@example.com")),
        "/farcooler:rep You represent Alice Chen (alice) in the Main workspace. Work here runs on the Claude \
         account team@example.com, which everyone on this runner shares."
    );
    assert!(first_prompt("Bob", "bob", "Main", None).ends_with("the Claude account signed in on this runner, which everyone on it shares."));
}

/// A rep is a claude pane in the main checkout, seated in its workspace and
/// bound to its member before it launches, started on the rep skill, with a
/// desk card labeled `rep` that the next rep reuses. A running one is
/// refused; a stopped one is resumed (`a_stopped_rep_resumes_its_conversation`).
#[tokio::test]
async fn a_rep_is_bound_seated_and_given_a_desk() {
    let (_dir, svc, main) = runner().await;
    let started = svc.start_rep(main, "alice", "").await.expect("a rep");
    let pane = svc.store.get_terminal(started.terminal.id).unwrap();
    assert_eq!((pane.workspace_id, pane.role, pane.title.as_str()), (Some(main), models::TerminalRole::Agent, "Alice Chen's rep"));
    assert!(svc.store.get_worktree(pane.worktree_id).unwrap().is_main_checkout);
    let rep = svc.store.rep_of_terminal(pane.id).unwrap().expect("bound");
    assert_eq!((rep.workspace, rep.handle.as_str()), (main, "alice"));
    let command = launched(&svc, pane.id).await;
    assert!(command.contains("/farcooler:rep You represent Alice Chen (alice) in the Main workspace."), "{command}");
    assert!(command.contains(&format!("FARCOOLER_WORKSPACE={main}")), "{command}");
    assert!(command.contains("FARCOOLER_REP=alice"), "{command}");
    assert_eq!(started.desk.title, "Alice Chen's desk");
    assert_eq!(started.desk.labels, [DESK_LABEL]);
    assert_eq!(svc.store.rep_desk(main, "alice").unwrap(), Some(started.desk.id));

    let again = svc.start_rep(main, "alice", "claude").await;
    assert!(matches!(again, Err(DomainError::Conflict { what: "rep_running" })), "{again:?}");
    svc.stop_terminal(pane.id).await.unwrap();
    let next = svc.start_rep(main, "alice", "claude:sonnet").await.expect("a stopped rep comes back");
    assert_eq!(next.terminal.id, pane.id, "the same rep, not a new one");
    assert_eq!(next.desk.id, started.desk.id, "the desk is the member's, not the pane's");

    // In another workspace, the main checkout is still Main's: the rep is
    // seated in its own before it launches, so it files there.
    let repo = svc.store.get_workspace(main).unwrap().repository_id;
    let billing = svc.store.create_workspace(repo, "Billing", "bil").unwrap().id;
    let bobs = svc.start_rep(billing, "bob", "claude").await.expect("Bob's rep in Billing");
    assert_eq!(svc.store.get_terminal(bobs.terminal.id).unwrap().workspace_id, Some(billing));
    assert!(launched(&svc, bobs.terminal.id).await.contains(&format!("FARCOOLER_WORKSPACE={billing}")));
    assert_eq!(bobs.desk.workspace_id, billing);

    let codex = svc.start_rep(main, "alice", "codex").await;
    assert!(matches!(codex, Err(DomainError::InvalidArgument { what: "command_preset" })), "{codex:?}");
    assert!(matches!(svc.start_rep(main, "carol", "claude").await, Err(DomainError::NotFound)), "nobody by that handle");
}

/// Where Claude Code keeps `pane`'s conversation, under the service's own
/// home (`Service::agent_home`): written, it's a conversation to resume.
fn transcript(svc: &Service, pane: &models::Terminal) -> std::path::PathBuf {
    let dir = svc.store.get_worktree(pane.worktree_id).unwrap().worktree_path;
    let dir = std::fs::canonicalize(&dir).unwrap();
    let sid = pane.agent_session_id.as_deref().expect("a claude pane has its session id");
    let projects = svc.agent_home().expect("a test's home").join(".claude/projects");
    projects.join(crate::session_discovery::project_dir_name(&dir)).join(format!("{sid}.jsonl"))
}

/// A rep's Claude Code launches with its permission profile and its gate
/// (R-66, ov-527): nothing asked, no settings source but the runner's own
/// file, no file or web tool, and a settings file whose `PreToolUse` runs the
/// rep gate on every tool, failing closed. An agent's pane carries none of
/// it.
#[tokio::test]
async fn a_rep_launches_with_its_permission_profile_and_gate() {
    let (_dir, svc, main) = runner().await;
    let rep = svc.start_rep(main, "alice", "").await.expect("a rep").terminal;
    let command = launched(&svc, rep.id).await;
    for flag in [
        "--permission-mode dontAsk",
        "--setting-sources=",
        "--disallowedTools=Edit,Write,MultiEdit,NotebookEdit,WebFetch",
        "--allowedTools=Bash(",
        " message orchestrator:*),",
        " task note:*),",
        &format!("rep-{}.json", rep.id),
    ] {
        assert!(command.contains(flag), "{flag} in {command}");
    }
    // No settings source at all: an empty list, the exact word (measured on
    // 2.1.299: a user hook then doesn't run, and a `--settings` one does).
    let args = permission_args("'/x/farcooler'");
    assert!(args.iter().any(|a| a == "--setting-sources="), "{args:?}");
    assert!(!args.iter().any(|a| a.starts_with("--setting-sources") && a != "--setting-sources="), "{args:?}");
    // No MCP server starts at launch, the checkout's or the user's.
    assert!(args.iter().any(|a| a == "--strict-mcp-config") && command.contains("--strict-mcp-config"), "{command}");
    assert!(!command.contains("--mcp-config"), "{command}");
    for gone in ["--setting-sources user", "Bash(git", "Bash(gh", "Bash(rg", "Bash(grep", "Bash(*", "skip-permissions", "claude-hooks.json"] {
        assert!(!command.contains(gone), "{gone} in {command}");
    }

    // The gate, on every tool, after the hooks every pane reports with.
    let settings: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(svc.root_dir().join(format!("rep-{}.json", rep.id))).unwrap()).unwrap();
    let pre = settings["hooks"]["PreToolUse"].as_array().unwrap();
    let gate = pre.iter().find(|e| e["hooks"][0]["command"].as_str().is_some_and(|c| c.contains(" rep-gate "))).expect("the gate");
    assert_eq!(gate["matcher"], "*");
    let gate = gate["hooks"][0]["command"].as_str().unwrap();
    let checkout = svc.store.get_worktree(svc.store.get_terminal(rep.id).unwrap().worktree_id).unwrap().worktree_path;
    assert!(gate.ends_with(" || exit 2"), "fails closed: {gate}");
    let entry = pre.iter().find(|e| e["hooks"][0]["command"].as_str().is_some_and(|c| c.contains(" rep-gate "))).unwrap();
    assert_eq!(entry["hooks"][0]["timeout"], GATE_TIMEOUT_S, "a gate that stalls is a gate that said nothing");
    let cli = svc.rep_cli().unwrap().display().to_string();
    assert!(cli.starts_with('/') && gate.starts_with(&format!("'{cli}' rep-gate")) && gate.contains(&format!("--cli '{cli}'")), "{gate}");
    use std::os::unix::fs::PermissionsExt as _;
    let mode = std::fs::metadata(svc.root_dir().join(format!("rep-{}.json", rep.id))).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600, "the gate's file is the runner's alone");
    let leftovers: Vec<_> = std::fs::read_dir(svc.root_dir()).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".tmp")).collect();
    assert!(leftovers.is_empty(), "written through a temporary file, renamed into place: {leftovers:?}");
    assert!(gate.contains(&format!("--checkout '{checkout}'")), "{gate}");
    assert!(settings["hooks"]["Stop"].is_array(), "it still reports like any pane");

    let main_checkout = svc.store.get_terminal(rep.id).unwrap().worktree_id;
    let agent = svc.create_terminal(main_checkout, "Agent 2", "claude").await.expect("an agent");
    let theirs = launched(&svc, agent.id).await;
    assert!(!theirs.contains("dontAsk") && !theirs.contains("--allowedTools") && !theirs.contains("rep-"), "{theirs}");
}

/// A gate that answers nothing never lets a call through: the profile has
/// no allow rule for a read tool, and none for a CLI found through `PATH`.
/// Only the gate's own allow passes a read; Claude Code's `dontAsk` refuses
/// the rest (checked on canary with a real turn).
#[test]
fn a_silent_gate_allows_nothing_by_rule() {
    let args = permission_args("/opt/fc/farcooler");
    let allowed = args.iter().find_map(|a| a.strip_prefix("--allowedTools=")).unwrap();
    for rule in allowed.split(',') {
        assert!(rule.starts_with("Bash('/opt/fc/farcooler' ") || rule.starts_with("Bash(/opt/fc/farcooler "), "{rule}");
    }
    assert!(!allowed.contains("Read") && !allowed.contains("Grep") && !allowed.contains("Glob"), "{allowed}");
    assert_eq!(absolute_cli(Some(std::path::Path::new("farcooler"))), None, "a name found through PATH");
    assert_eq!(absolute_cli(None), None);
    assert_eq!(absolute_cli(Some(std::path::Path::new("/opt/fc/farcooler"))).as_deref(), Some("/opt/fc/farcooler"));
}

/// Settings with no `PreToolUse` list are refused, never written without
/// the gate.
#[test]
fn settings_without_a_pre_tool_use_list_are_refused() {
    assert!(with_gate(serde_json::json!({ "hooks": {} }), "gate").is_err());
    assert!(with_gate(serde_json::json!({}), "gate").is_err());
    let made = with_gate(serde_json::json!({ "hooks": { "PreToolUse": [] } }), "gate").unwrap();
    assert_eq!(made["hooks"]["PreToolUse"][0]["hooks"][0]["command"], "gate");
}

/// The real hook command, run by `sh` as Claude Code runs it, with a CLI
/// that isn't there, exits 2: Claude Code's blocking exit, never a pass.
#[test]
fn a_gate_that_cannot_run_blocks() {
    let command = rep_gate_command("/nonexistent/fc-t/farcooler", std::path::Path::new("/tmp"));
    let status = std::process::Command::new("/bin/sh")
        .args(["-c", &command])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(2), "{command}");
}

/// A rep's lock goes with its pane, and a client's restart takes it like a
/// wake does.
#[tokio::test]
async fn a_reps_lock_is_held_by_every_launch_and_goes_with_its_pane() {
    let (_dir, svc, main) = runner().await;
    let pane = svc.start_rep(main, "alice", "").await.expect("a rep").terminal;
    let lock = svc.rep_resumes.lock_for(pane.id);
    let held = lock.lock().await;
    let restart = svc.restart_terminal(pane.id);
    tokio::pin!(restart);
    let waited = tokio::time::timeout(std::time::Duration::from_millis(300), &mut restart).await;
    assert!(waited.is_err(), "a client's restart waits for the rep's lock");
    drop(held);
    restart.await.expect("then restarts");
    assert!(svc.rep_resumes.holds(pane.id));
    svc.stop_terminal(pane.id).await.unwrap();
    svc.remove_terminal(pane.id).await.unwrap();
    assert!(!svc.rep_resumes.holds(pane.id), "forgotten with its pane");
    assert!(!svc.root_dir().join(format!("rep-{}.json", pane.id)).exists(), "and its gate's file");
}

/// Starting a stopped rep again resumes its conversation (R-66): the same
/// pane, on the session its record holds, with its profile, and no first
/// message sent twice. Its person's thread is never replaced.
#[tokio::test]
async fn a_stopped_rep_resumes_its_conversation() {
    let (_dir, svc, main) = runner().await;
    let first = svc.start_rep(main, "alice", "").await.expect("a rep");
    let pane = svc.store.get_terminal(first.terminal.id).unwrap();
    let file = transcript(&svc, &pane);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "{}\n").unwrap();
    svc.stop_terminal(pane.id).await.unwrap();

    let again = svc.start_rep(main, "alice", "claude").await.expect("resumed");
    assert_eq!(again.terminal.id, pane.id, "the same pane");
    assert_eq!(again.desk.id, first.desk.id);
    let now = svc.store.get_terminal(pane.id).unwrap();
    assert_eq!(now.agent_session_id, pane.agent_session_id, "the same conversation");
    assert!(svc.is_running(&now), "running again");
    let command = launched(&svc, pane.id).await;
    let sid = pane.agent_session_id.as_deref().unwrap();
    assert!(command.contains(&format!("--resume {sid}")), "{command}");
    assert!(command.contains("--permission-mode dontAsk"), "a resumed rep keeps its profile: {command}");
    assert!(!command.contains("/farcooler:rep"), "no first message again: {command}");
}

/// A stopped rep with no conversation on disk (it never finished a turn)
/// starts clean on its first message again, so it still loads its skill,
/// with its profile and gate.
#[tokio::test]
async fn a_stopped_rep_with_nothing_to_resume_starts_on_its_skill() {
    let (_dir, svc, main) = runner().await;
    let first = svc.start_rep(main, "alice", "").await.expect("a rep");
    svc.stop_terminal(first.terminal.id).await.unwrap();
    let again = svc.start_rep(main, "alice", "").await.expect("restarted");
    assert_eq!(again.terminal.id, first.terminal.id);
    let command = launched(&svc, again.terminal.id).await;
    assert!(!command.contains("--resume"), "{command}");
    assert!(command.contains("/farcooler:rep You represent Alice Chen (alice) in the Main workspace."), "{command}");
    assert!(command.contains("--permission-mode dontAsk") && command.contains(&format!("rep-{}.json", first.terminal.id)), "{command}");
}

/// Two resumes of one stopped rep at once (two messages, or a message and a
/// `rep.start`) restart it once: the second waits for the first and finds it
/// running. One restart is one new epoch.
#[tokio::test]
async fn resuming_a_rep_is_single_flight() {
    let (_dir, svc, main) = runner().await;
    let pane = svc.start_rep(main, "alice", "").await.expect("a rep").terminal;
    svc.stop_terminal(pane.id).await.unwrap();
    let before = svc.store.get_terminal(pane.id).unwrap().epoch;
    let (one, two) = tokio::join!(svc.resume_rep(pane.id), svc.resume_rep(pane.id));
    let (one, two) = (one.expect("the first resumes"), two.expect("the second finds it resumed"));
    assert_eq!((one.id, two.id), (pane.id, pane.id));
    let now = svc.store.get_terminal(pane.id).unwrap();
    assert_eq!(now.epoch, before + 1, "restarted once");
    assert!(svc.is_running(&now));
}

fn factory(svc: &Arc<Service>, client_id: Option<&str>, scope: Scope) -> crate::rpc::RpcFactory {
    let peer = Peer { client_id: client_id.map(str::to_string), scope };
    crate::rpc::RpcFactory::new(svc.clone(), crate::watch::Watcher::new(svc.clone()), Arc::new(tokio::sync::Notify::new()), peer)
}

async fn call(factory: &crate::rpc::RpcFactory, req: Request) -> std::result::Result<pb::result::Value, String> {
    use farcooler_transport::Handler as _;
    match factory.handle(req).await.outcome {
        Some(response::Outcome::Result(r)) => Ok(r.value.expect("a value")),
        Some(response::Outcome::Error(e)) => Err(format!("{}:{}", farcooler_core::error::word_for(e.code), e.what)),
        other => panic!("no outcome: {other:?}"),
    }
}

/// `rep.start` is a runner admin's, or the member's own; `member.remove`
/// stops their rep; `member.list` says who's asking and who has a rep.
#[tokio::test]
async fn members_start_their_own_rep_and_lose_it_when_removed() {
    let (_dir, svc, main) = runner().await;
    let start = |handle: &str| Request {
        method: "rep.start".into(),
        payload: Some(request::Payload::RepStart(pb::RepStart {
            workspace_id: id_bytes(main),
            handle: handle.into(),
            harness: String::new(),
        })),
        ..Default::default()
    };
    let bob = factory(&svc, Some("phone-b"), Scope::Control);
    assert_eq!(call(&bob, start("alice")).await.err().as_deref(), Some("scope-denied:"), "not Bob's to start");
    let Ok(pb::result::Value::RepStarted(started)) = call(&bob, start("bob")).await else { panic!("Bob starts his own") };
    let desk = svc.store.rep_desk(main, "bob").unwrap().unwrap();
    assert_eq!(started.desk_task_key, svc.store.get_task(desk).unwrap().key);
    let admin = factory(&svc, None, Scope::HostAdmin);
    assert!(call(&admin, start("alice")).await.is_ok(), "an admin starts anyone's");
    let owners_phone = factory(&svc, Some("phone-o"), Scope::Control);
    svc.store.set_member("carol", "Carol", &[]).unwrap();
    assert!(call(&owners_phone, start("carol")).await.is_ok(), "the owner starts anyone's, at Control too");

    let list = Request { method: "member.list".into(), ..Default::default() };
    let Ok(pb::result::Value::MemberList(listed)) = call(&bob, list.clone()).await else { panic!("a list") };
    assert_eq!(listed.you, "bob");
    assert_eq!(listed.claude_account, "owner@example.com", "a member at Control is told whose account");
    let bobs = listed.members.iter().find(|m| m.handle == "bob").unwrap();
    assert_eq!((bobs.client_ids.as_slice(), bobs.rep_terminal_ids.as_slice()), (["phone-b".to_string()].as_slice(), [started.terminal_id.clone()].as_slice()));
    // Where each rep works, beside its pane, so an app can name the board (ov-508).
    assert_eq!(bobs.rep_workspace_ids.as_slice(), [id_bytes(main)].as_slice());

    let looker = factory(&svc, Some("phone-r"), Scope::Read);
    let Ok(pb::result::Value::MemberList(read)) = call(&looker, list.clone()).await else { panic!("a list") };
    assert_eq!(read.claude_account, "", "the owner's email isn't for a device enrolled only to look");
    assert_eq!(read.members.len(), listed.members.len(), "the list itself is for every scope");

    let remove = Request {
        method: "member.remove".into(),
        payload: Some(request::Payload::MemberRemove(pb::MemberRemove { handle: "bob".into() })),
        ..Default::default()
    };
    assert_eq!(call(&bob, remove.clone()).await.err().as_deref(), Some("scope-denied:"));
    let after = match call(&admin, remove).await {
        Ok(pb::result::Value::MemberList(after)) => after,
        other => panic!("removed: {other:?}"),
    };
    assert_eq!(after.you, "", "the local socket is the owner");
    assert!(after.members.iter().all(|m| m.handle != "bob"));
    let pane = Uuid::from_slice(&started.terminal_id).unwrap();
    assert!(svc.store.get_terminal(pane).is_err(), "Bob's rep went with him");
}


/// `member.set` binds only devices this runner enrolled, so a typo can't
/// leave a teammate's real phone the owner's. Its own service, with an
/// `authorized_keys` this test can see.
#[tokio::test]
async fn a_member_is_bound_only_to_enrolled_devices() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let ssh = dir.path().join("home").join(".ssh");
    std::fs::create_dir_all(&ssh).unwrap();
    std::fs::set_permissions(&ssh, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
    let svc = Arc::new(Service::open_in(state).await.unwrap().enrolling_into(ssh.join("authorized_keys")));
    crate::enrollment::enroll(
        &svc,
        &pb::ClientEnroll {
            public_key: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBERERERERERERERERERERERERERERERERERERERERER phone".into(),
            label: "Bob's phone".into(),
            client_id: "phone-b".into(),
            scope: Scope::Control as i32,
            shell_access: false,
            node_key: String::new(),
        },
    )
    .await
    .expect("the phone enrolls");
    let admin = factory(&svc, None, Scope::HostAdmin);
    let set = |device: &str| Request {
        method: "member.set".into(),
        payload: Some(request::Payload::MemberSet(pb::MemberSet {
            handle: "bob".into(),
            name: "Bob".into(),
            client_ids: vec![device.into()],
        })),
        ..Default::default()
    };
    assert_eq!(call(&admin, set("phne-b")).await.err().as_deref(), Some("invalid-argument:device"), "a typo");
    assert!(svc.store.member("bob").is_err(), "nothing was written");
    assert!(call(&admin, set("phone-b")).await.is_ok());
    assert_eq!(svc.store.member_of_device("phone-b").unwrap().as_deref(), Some("bob"));

    // Removing Bob revokes his phone: released is not enough, it keeps its scope.
    let remove = Request {
        method: "member.remove".into(),
        payload: Some(request::Payload::MemberRemove(pb::MemberRemove { handle: "bob".into() })),
        ..Default::default()
    };
    assert!(call(&admin, remove).await.is_ok());
    let left = crate::enrollment::list(&svc).await.unwrap();
    assert!(left.items.iter().all(|e| e.client_id != "phone-b"), "Bob's phone is still enrolled: {left:?}");
}

/// A service opened on a scratch root enrolls into a file under that root,
/// never the `~/.ssh` of whoever runs the suite (ov-506): CI's macOS runner
/// has one the fence refuses to read, and a test must not rewrite a
/// developer's.
#[tokio::test]
async fn a_test_service_keeps_its_authorized_keys_in_its_own_root() {
    let (_dir, svc, _main) = runner().await;
    assert!(svc.authorized_keys().starts_with(svc.root_dir()), "{:?}", svc.authorized_keys());
}

/// Removing a member with no devices never reads `authorized_keys`, so a file
/// the fence refuses doesn't keep them on the runner; one with devices is
/// refused there instead, and stays, rather than leave a device logging in.
#[tokio::test]
async fn a_member_with_no_devices_is_removed_whatever_authorized_keys_says() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    // Group-writable, which the fence refuses as sshd's StrictModes would.
    let ssh = dir.path().join("home").join(".ssh");
    std::fs::create_dir_all(&ssh).unwrap();
    std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o775)).unwrap();
    let svc = Arc::new(Service::open_in(state).await.unwrap().enrolling_into(ssh.join("authorized_keys")));
    svc.store.set_member("carol", "Carol", &[]).unwrap();
    svc.store.set_member("bob", "Bob", &["phone-b".to_string()]).unwrap();
    let admin = factory(&svc, None, Scope::HostAdmin);
    let remove = |handle: &str| Request {
        method: "member.remove".into(),
        payload: Some(request::Payload::MemberRemove(pb::MemberRemove { handle: handle.into() })),
        ..Default::default()
    };
    let carol = call(&admin, remove("carol")).await;
    assert!(carol.is_ok(), "{carol:?}");
    assert!(svc.store.member("carol").is_err(), "Carol is gone");
    assert!(call(&admin, remove("bob")).await.is_err(), "Bob's phone can't be shown revoked");
    assert!(svc.store.member("bob").is_ok(), "so Bob stays");
}
