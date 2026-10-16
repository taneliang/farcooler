//! Whose Claude login each pane launches on (ov-583), on the fixture's real
//! tmux server with the test stub in place of claude.

use std::sync::Arc;

use farcooler_protocol::v1::{self as pb, Request, Scope, request, response};
use farcooler_transport::Peer;

use super::*;
use crate::test_support::ScratchDir;
use crate::wire::id_bytes;

/// A runner with Alice and Bob as members, Main's workspace, and the
/// repository's main checkout.
async fn runner() -> (ScratchDir, Arc<Service>, Uuid, Uuid) {
    let (dir, svc, repo) = crate::test_support::fixture().await;
    crate::reconcile::repository(&svc, repo).await.unwrap();
    let main = svc.store.ensure_main_workspace(repo).unwrap().id;
    let checkout = svc.store.list_worktrees_for_repository(repo).unwrap().into_iter().find(|w| w.is_main_checkout).unwrap().id;
    svc.store.set_member("alice", "Alice Chen", &["phone-a".to_string()]).unwrap();
    svc.store.set_member("bob", "Bob", &["phone-b".to_string()]).unwrap();
    (dir, svc, main, checkout)
}

/// What tmux was told to run in `terminal`'s pane.
async fn launched(svc: &Service, terminal: Uuid) -> String {
    let snapshot = svc.inventory.refresh().await;
    let pane = snapshot.claimants(terminal).into_iter().next().expect("a pane").clone();
    let out = svc.tmux.run(&["display-message", "-p", "-t", &pane.pane_id, "#{pane_start_command}"]).await.unwrap();
    out.stdout.trim().to_string()
}

/// The pid of `terminal`'s pane process.
async fn pid_of(svc: &Service, terminal: Uuid) -> i32 {
    let snapshot = svc.inventory.refresh().await;
    snapshot.claimants(terminal).into_iter().next().expect("a pane").stamp.pid as i32
}

/// `handle`'s directory, as a launch names it: canonical, every link above
/// it resolved (the fixture's root is under `/tmp` on macOS, a link to
/// `/private/tmp`), no trailing slash.
fn dir_of(svc: &Service, handle: &str) -> String {
    let dir = std::fs::canonicalize(svc.member_claude_dir(handle).expect("a login")).unwrap();
    dir.into_os_string().into_string().unwrap()
}

fn factory(svc: &Arc<Service>, client_id: Option<&str>, pid: Option<i32>) -> crate::rpc::RpcFactory {
    let peer = Peer { client_id: client_id.map(str::to_string), scope: Scope::Control };
    crate::rpc::RpcFactory::new(svc.clone(), crate::watch::Watcher::new(svc.clone()), Arc::new(tokio::sync::Notify::new()), peer)
        .with_caller_pid(pid)
}

/// `terminal.create` of `preset` in `worktree`, as `factory`'s caller: the
/// new pane's id, or the refusal as `<code>:<what>`.
async fn create(factory: &crate::rpc::RpcFactory, worktree: Uuid, preset: &str) -> std::result::Result<Uuid, String> {
    use farcooler_transport::Handler as _;
    let req = Request {
        method: "terminal.create".into(),
        target_resource_id: Some(id_bytes(worktree)),
        payload: Some(request::Payload::TerminalCreate(pb::TerminalCreate {
            title: "Agent".into(),
            command_preset: preset.into(),
            ..Default::default()
        })),
        ..Default::default()
    };
    match factory.handle(req).await.outcome {
        Some(response::Outcome::Result(r)) => match r.value {
            Some(pb::result::Value::Terminal(t)) => Ok(Uuid::from_slice(&t.id).unwrap()),
            other => panic!("not a terminal: {other:?}"),
        },
        Some(response::Outcome::Error(e)) => Err(format!("{}:{}", farcooler_core::error::word_for(e.code), e.what)),
        other => panic!("no outcome: {other:?}"),
    }
}

/// Whether `command` sets `CLAUDE_CONFIG_DIR` to exactly `dir`, through
/// whatever quoting its layer of the launch needs.
fn sets(command: &str, dir: &str) -> bool {
    command.match_indices("CLAUDE_CONFIG_DIR=").any(|(at, word)| {
        let value = command[at + word.len()..].trim_start_matches(['\'', '\\']);
        value.strip_prefix(dir).is_some_and(|rest| rest.starts_with(['\'', '\\', ' ']))
    })
}

fn assert_on(command: &str, dir: &str) {
    assert!(sets(command, dir), "{dir} in {command}");
}

fn assert_runners(command: &str) {
    assert!(!command.contains("CLAUDE_CONFIG_DIR"), "the runner's own login sets no variable: {command}");
}

/// A rep and a pane a member opens from their phone launch on that member's
/// own directory, named by their login id and made 0700; the owner's panes,
/// and an orchestrator the owner leads, set no `CLAUDE_CONFIG_DIR` at all
/// (ov-582: setting it to the default directory signs the owner out); an
/// orchestrator a member leads runs on its lead's.
#[tokio::test]
async fn each_pane_launches_on_the_login_of_whoever_it_is() {
    let (_dir, svc, main, checkout) = runner().await;
    let rep = svc.start_rep(main, "alice", "").await.expect("Alice's rep").terminal;
    let alice = dir_of(&svc, "alice");
    assert_on(&launched(&svc, rep.id).await, &alice);
    let recorded = svc.store.terminal_login(rep.id).unwrap().expect("recorded");
    assert_eq!((recorded.handle.as_str(), recorded.config_dir.as_str()), ("alice", alice.as_str()));
    assert!(alice.ends_with(&format!("/claude/{}", recorded.login_id)), "named by the login id, not the handle: {alice}");
    use std::os::unix::fs::PermissionsExt as _;
    for dir in [member_root(svc.root_dir()), PathBuf::from(&alice)] {
        assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700, "{}", dir.display());
    }

    let owners = create(&factory(&svc, None, None), checkout, "claude").await.expect("the owner's pane");
    assert_runners(&launched(&svc, owners).await);
    assert_eq!(svc.store.terminal_login(owners).unwrap(), None);

    let bobs = create(&factory(&svc, Some("phone-b"), None), checkout, "claude").await.expect("Bob's pane");
    assert_on(&launched(&svc, bobs).await, &dir_of(&svc, "bob"));
    // A member's shell too, so a claude typed in it runs on their login.
    let shell = create(&factory(&svc, Some("phone-b"), None), checkout, "shell").await.expect("Bob's shell");
    let shell = launched(&svc, shell).await;
    assert!(shell.contains("env CLAUDE_CONFIG_DIR=") && sets(&shell, &dir_of(&svc, "bob")), "{shell}");

    let orchestrator = svc.start_orchestrator(main, "claude", false, None).await.expect("the owner leads");
    assert_runners(&launched(&svc, orchestrator.id).await);
    svc.store.set_workspace_lead(main, Some("bob")).unwrap();
    let led = svc.start_orchestrator(main, "claude", true, None).await.expect("Bob leads");
    assert_on(&launched(&svc, led.id).await, &dir_of(&svc, "bob"));
}

/// A pane opened from inside another, through the local socket, takes its
/// login from the pane it was opened from, by process ancestry: an
/// orchestrator's lanes run on its lead's, a member's pane's on that
/// member's, the owner's on the runner's.
#[tokio::test]
async fn a_pane_opened_from_a_pane_takes_its_login_from_that_pane() {
    let (_dir, svc, main, checkout) = runner().await;
    svc.store.set_workspace_lead(main, Some("bob")).unwrap();
    let orchestrator = svc.start_orchestrator(main, "claude", false, None).await.expect("Bob leads");
    let in_orchestrator = factory(&svc, None, Some(pid_of(&svc, orchestrator.id).await));
    let lane = create(&in_orchestrator, checkout, "claude").await.expect("a lane");
    assert_on(&launched(&svc, lane).await, &dir_of(&svc, "bob"));

    let alices = create(&factory(&svc, Some("phone-a"), None), checkout, "claude").await.expect("Alice's pane");
    let in_alices = factory(&svc, None, Some(pid_of(&svc, alices).await));
    let helper = create(&in_alices, checkout, "claude").await.expect("a helper Alice's agent opens");
    assert_on(&launched(&svc, helper).await, &dir_of(&svc, "alice"));

    let owners = create(&factory(&svc, None, None), checkout, "shell").await.expect("the owner's shell");
    let from_owners = create(&factory(&svc, None, Some(pid_of(&svc, owners).await)), checkout, "claude").await.unwrap();
    assert_runners(&launched(&svc, from_owners).await);
}

/// A process in a pane that names a member's device in its session preamble
/// (device ids aren't secret) still gets its own pane's login, never the
/// device's member's (review 1, H1): from the owner's pane, the runner's;
/// from Alice's rep, which may open nothing, a refusal before any login.
#[tokio::test]
async fn a_pane_claiming_a_members_device_gets_its_own_login() {
    let (_dir, svc, main, checkout) = runner().await;
    let owners = create(&factory(&svc, None, None), checkout, "shell").await.expect("the owner's shell");
    let spoofed = factory(&svc, Some("phone-b"), Some(pid_of(&svc, owners).await));
    let pane = create(&spoofed, checkout, "claude").await.expect("opened");
    assert_runners(&launched(&svc, pane).await);
    assert_eq!(svc.store.terminal_login(pane).unwrap(), None, "not Bob's");

    let bobs_other = create(&factory(&svc, Some("phone-b"), None), checkout, "claude").await.expect("Bob's own pane");
    let in_bobs = factory(&svc, Some("phone-a"), Some(pid_of(&svc, bobs_other).await));
    let helper = create(&in_bobs, checkout, "claude").await.expect("opened from Bob's pane");
    assert_on(&launched(&svc, helper).await, &dir_of(&svc, "bob"));

    let rep = svc.start_rep(main, "alice", "").await.expect("Alice's rep").terminal;
    let from_rep = create(&factory(&svc, Some("phone-b"), Some(pid_of(&svc, rep.id).await)), checkout, "claude").await;
    assert_eq!(from_rep, Err("resource-conflict:rep_messenger".to_string()), "a rep opens nothing");
}

/// A restart comes back on the exact directory string the first launch
/// passed, which is the login's Keychain key (ov-582). One swapped for a
/// link, to the owner's or to another member's, is refused, never followed
/// and never replaced by the runner's.
#[tokio::test]
async fn a_restart_reuses_the_recorded_dir_and_refuses_a_swapped_one() {
    let (_dir, svc, _main, checkout) = runner().await;
    let alices = create(&factory(&svc, Some("phone-a"), None), checkout, "claude").await.expect("Alice's pane");
    let pane = create(&factory(&svc, Some("phone-b"), None), checkout, "claude").await.expect("Bob's pane");
    let recorded = svc.store.terminal_login(pane).unwrap().expect("recorded").config_dir;
    assert!(Path::new(&recorded).is_absolute() && !recorded.ends_with('/'), "{recorded}");
    assert_eq!(recorded, dir_of(&svc, "bob"), "canonical: links above it resolved");
    svc.stop_terminal(pane).await.unwrap();
    svc.restart_terminal(pane).await.expect("restarted");
    assert_on(&launched(&svc, pane).await, &recorded);

    // Opened up since: a relaunch sets it back to 0700.
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(&recorded, std::fs::Permissions::from_mode(0o755)).unwrap();
    svc.stop_terminal(pane).await.unwrap();
    svc.restart_terminal(pane).await.expect("restarted");
    assert_eq!(std::fs::metadata(&recorded).unwrap().permissions().mode() & 0o777, 0o700);

    let bob = svc.member_claude_dir("bob").unwrap();
    for target in [svc.agent_home().unwrap().to_path_buf(), PathBuf::from(dir_of(&svc, "alice"))] {
        let _ = std::fs::remove_dir_all(&bob);
        let _ = std::fs::remove_file(&bob);
        std::os::unix::fs::symlink(&target, &bob).unwrap();
        svc.stop_terminal(pane).await.unwrap();
        let refused = svc.restart_terminal(pane).await;
        assert!(matches!(refused, Err(DomainError::OperationFailed)), "a link to {}: {refused:?}", target.display());
        assert!(!svc.is_running(&svc.store.get_terminal(pane).unwrap()), "not started on anything");
    }
    let _ = alices;
}

/// A member's first launch onto a directory that's a link to another
/// member's is refused, before any pane is made.
#[tokio::test]
async fn a_first_launch_onto_a_link_to_another_members_dir_is_refused() {
    let (_dir, svc, _main, checkout) = runner().await;
    create(&factory(&svc, Some("phone-a"), None), checkout, "claude").await.expect("Alice's pane");
    let bobs = login_dir(svc.root_dir(), &svc.store.ensure_member_login("bob").unwrap()).unwrap();
    std::os::unix::fs::symlink(dir_of(&svc, "alice"), &bobs).unwrap();
    let before = svc.store.list_terminals_for_worktree(checkout).unwrap().len();
    assert_eq!(create(&factory(&svc, Some("phone-b"), None), checkout, "claude").await, Err("operation-failed:".to_string()));
    assert_eq!(svc.store.list_terminals_for_worktree(checkout).unwrap().len(), before, "nothing made");
}

/// A removed member's pane isn't restarted on the runner's login instead,
/// nor on the login of someone given their handle since (review 1, M2):
/// `login_gone`. A released phone opens nothing (`not_yours`).
#[tokio::test]
async fn a_removed_members_pane_stays_down_and_a_reused_handle_starts_fresh() {
    let (_dir, svc, _main, checkout) = runner().await;
    let pane = create(&factory(&svc, Some("phone-b"), None), checkout, "claude").await.expect("Bob's pane");
    let old = dir_of(&svc, "bob");
    svc.stop_terminal(pane).await.unwrap();
    svc.store.remove_member("bob").unwrap();
    let refused = svc.restart_terminal(pane).await;
    assert!(matches!(refused, Err(DomainError::Conflict { what: "login_gone" })), "{refused:?}");
    let released = create(&factory(&svc, Some("phone-b"), None), checkout, "claude").await;
    assert_eq!(released, Err("not-yours:not_yours".to_string()));

    svc.store.set_member("bob", "Another Bob", &["phone-b2".to_string()]).unwrap();
    let refused = svc.restart_terminal(pane).await;
    assert!(matches!(refused, Err(DomainError::Conflict { what: "login_gone" })), "the old Bob's pane: {refused:?}");
    let fresh = create(&factory(&svc, Some("phone-b2"), None), checkout, "claude").await.expect("the new Bob's pane");
    let new = dir_of(&svc, "bob");
    assert_ne!(new, old, "a new person, a new directory");
    assert_on(&launched(&svc, fresh).await, &new);
}

/// An orchestrator stays on its lead's login (review 1, H2): once the lead is
/// handed on, a restart on the old one is refused `lead_changed`, and a pane
/// on another login isn't made the orchestrator. A new start runs on the new
/// lead's.
#[tokio::test]
async fn an_orchestrator_isnt_relaunched_on_an_old_leads_login() {
    let (_dir, svc, main, checkout) = runner().await;
    let first = svc.start_orchestrator(main, "claude", false, None).await.expect("the owner leads");
    svc.store.set_workspace_lead(main, Some("bob")).unwrap();
    svc.stop_terminal(first.id).await.unwrap();
    let refused = svc.restart_terminal(first.id).await;
    assert!(matches!(refused, Err(DomainError::Conflict { what: "lead_changed" })), "{refused:?}");
    let started = svc.start_orchestrator(main, "claude", true, None).await.expect("a new start");
    assert_on(&launched(&svc, started.id).await, &dir_of(&svc, "bob"));
    svc.stop_terminal(started.id).await.unwrap();
    svc.restart_terminal(started.id).await.expect("on the lead's own login, it restarts");
    assert_on(&launched(&svc, started.id).await, &dir_of(&svc, "bob"));

    svc.store.set_workspace_lead(main, Some("alice")).unwrap();
    svc.stop_terminal(started.id).await.unwrap();
    let refused = svc.restart_terminal(started.id).await;
    assert!(matches!(refused, Err(DomainError::Conflict { what: "lead_changed" })), "Bob's, not Alice's: {refused:?}");

    let owners = create(&factory(&svc, None, None), checkout, "claude").await.expect("the owner's pane");
    svc.store.set_terminal_workspace(owners, main).unwrap();
    let promoted = svc.set_terminal_role(owners, models::TerminalRole::Orchestrator).await;
    assert!(matches!(promoted, Err(DomainError::Conflict { what: "lead_changed" })), "{promoted:?}");
}

/// A rep from before logins were recorded runs on its member's login, and
/// its first relaunch records it (review 1, L2).
#[tokio::test]
async fn an_old_reps_login_is_recorded_on_its_first_relaunch() {
    let (_dir, svc, main, _checkout) = runner().await;
    let rep = svc.start_rep(main, "alice", "").await.expect("Alice's rep").terminal;
    svc.stop_terminal(rep.id).await.unwrap();
    farcooler_store::testing::forget_terminal_logins(&svc.store);
    assert_eq!(svc.store.terminal_login(rep.id).unwrap(), None, "as a rep from before");
    svc.restart_terminal(rep.id).await.expect("restarted");
    assert_on(&launched(&svc, rep.id).await, &dir_of(&svc, "alice"));
    assert_eq!(svc.store.terminal_login(rep.id).unwrap().map(|l| l.config_dir), Some(dir_of(&svc, "alice")));
}

/// Below the minimum claude, a member's claude pane is refused before
/// anything is made; one whose version can't be read is refused too. A
/// member's shell on a runner with no claude to read opens, and the owner's
/// panes are never checked.
#[tokio::test]
async fn a_member_launch_needs_a_claude_that_keys_logins_by_dir() {
    let (_dir, svc, _main, checkout) = runner().await;
    let set = |v: Option<Version>| *svc.claude_logins.claimed.lock().unwrap() = Some(v);
    let bob = factory(&svc, Some("phone-b"), None);
    set(Some(Version(1, 0, 7)));
    let before = svc.store.list_terminals_for_worktree(checkout).unwrap().len();
    assert_eq!(create(&bob, checkout, "claude").await, Err("resource-conflict:claude_too_old".to_string()));
    assert_eq!(svc.store.list_terminals_for_worktree(checkout).unwrap().len(), before, "nothing made");
    assert_eq!(svc.store.member_login("bob").unwrap(), None, "no login made");
    assert!(create(&factory(&svc, None, None), checkout, "claude").await.is_ok(), "the owner's isn't checked");
    set(None);
    assert_eq!(create(&bob, checkout, "claude").await, Err("resource-conflict:claude_too_old".to_string()));
    assert!(create(&bob, checkout, "shell").await.is_ok(), "no claude to read: a shell opens");
    set(Some(MIN_MEMBER_CLAUDE));
    assert!(create(&bob, checkout, "claude").await.is_ok());
}

/// Only a line that is exactly claude's version counts (review 1, M1): an
/// rc file's banner with a version in it, a pre-release, or nothing at all
/// doesn't.
#[test]
fn a_version_is_read_only_from_claudes_own_line() {
    assert_eq!(version_line("2.1.300 (Claude Code)\n"), Some(Version(2, 1, 300)));
    assert_eq!(version_line("node v22.11.0 via mise 2024.1.0\nWelcome back!\n1.0.8 (Claude Code)\n"), Some(Version(1, 0, 8)));
    assert_eq!(version_line("mise 2024.12.3 activated\n"), None);
    assert_eq!(version_line("1.0.8-beta.1 (Claude Code)"), None);
    assert_eq!(version_line("v1.0.8 (Claude Code)"), None);
    assert_eq!(version_line("zsh: command not found: claude"), None);
    assert!(Version(1, 0, 7) < MIN_MEMBER_CLAUDE && Version(1, 0, 10) > MIN_MEMBER_CLAUDE);
}

/// The probe itself, through a stand-in login shell: it runs in the pane's
/// directory, needs exit 0, and reads only claude's own line.
#[tokio::test]
async fn the_version_probe_runs_in_the_panes_directory_and_fails_closed() {
    let dir = farcooler_core::scratch::dir("version-probe");
    let here = dir.join("repo");
    std::fs::create_dir_all(&here).unwrap();
    let here = std::fs::canonicalize(&here).unwrap();
    let shell = |name: &str, body: &str| {
        use std::os::unix::fs::PermissionsExt as _;
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.to_string_lossy().into_owned()
    };
    let banner = shell("banner", "echo 'mise 2025.1.0 (Claude Code)x'; echo 'Welcome 9.9.9'; echo '1.0.9 (Claude Code)'");
    assert_eq!(ask_claude_version(&banner, &here).await, Some(Version(1, 0, 9)));
    let failing = shell("failing", "echo '9.9.9 (Claude Code)'; exit 1");
    assert_eq!(ask_claude_version(&failing, &here).await, None, "a non-zero exit is no version");
    let by_dir = shell("by-dir", &format!("if [ \"$(pwd -P)\" = '{}' ]; then echo '1.0.8 (Claude Code)'; else echo '1.0.7 (Claude Code)'; fi", here.display()));
    assert_eq!(ask_claude_version(&by_dir, &here).await, Some(Version(1, 0, 8)), "run in the pane's directory");
    assert_eq!(ask_claude_version(&by_dir, &dir).await, Some(Version(1, 0, 7)));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A member's directory handed through both layers of a launch arrives as
/// exactly itself, quote and space included.
#[test]
fn a_dir_with_a_quote_and_a_space_reaches_claude_whole() {
    let dir = "/tmp/fc t/it's/claude/0193a1b2c3d4e5f60718293a4b5c6d7e";
    let extras = crate::service::LaunchExtras { claude_dir: Some(dir.to_string()), ..crate::service::LaunchExtras::NONE };
    let shell = crate::service::claude_dir_env(&extras);
    let out = std::process::Command::new("/bin/sh").args(["-c", &format!("{shell}printenv CLAUDE_CONFIG_DIR")]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim_end_matches('\n'), dir, "the shell's env layer");
    let inner = crate::service::claude_dir_assignment(&extras);
    let payload = crate::service::shell_quote(&format!("{inner}printenv CLAUDE_CONFIG_DIR"));
    let out = std::process::Command::new("/bin/sh").args(["-c", &format!("/bin/sh -c {payload}")]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim_end_matches('\n'), dir, "the -ilc payload layer");
}

/// Only real directories named by a login id count as a member's: never a
/// link, never a stray file or a handle.
#[test]
fn member_dirs_are_real_and_named_by_a_login_id() {
    let dir = farcooler_core::scratch::dir("member-dirs");
    std::fs::create_dir_all(&dir).unwrap();
    let root = member_root(&dir);
    let id = Uuid::now_v7().simple().to_string();
    let other = Uuid::now_v7().simple().to_string();
    std::fs::create_dir_all(root.join(&id)).unwrap();
    std::fs::create_dir_all(root.join("alice")).unwrap();
    std::fs::write(root.join(".claude.json"), "{}").unwrap();
    std::os::unix::fs::symlink(root.join(&id), root.join(&other)).unwrap();
    assert_eq!(member_dirs(&dir), vec![root.join(&id)]);
    assert!(ensure_login_dir(&dir, &other).is_err(), "a link is refused, not followed");
    assert!(ensure_login_dir(&dir, "../x").is_err());
    assert!(ensure_login_dir(&dir, "alice").is_err(), "a handle is not a login");
    assert!(ensure_login_dir(&dir, &id).is_ok());
    assert_eq!(runner_dir(Some("".into()), Some(Path::new("/h"))), Some(PathBuf::from("/h/.claude")));
    assert_eq!(runner_dir(Some("/c".into()), Some(Path::new("/h"))), Some(PathBuf::from("/c")));
    let _ = std::fs::remove_dir_all(&dir);
}

/// An orchestrator still on an old lead's login opens nothing on the new
/// lead's (review 2, M1): a lane it opens is refused, `lead_changed` after a
/// hand-on, and `login_gone` after its member lead is removed (when the lead
/// falls back to the owner).
#[tokio::test]
async fn an_orchestrator_on_an_old_leads_login_opens_no_lanes() {
    let (_dir, svc, main, checkout) = runner().await;
    svc.store.set_workspace_lead(main, Some("bob")).unwrap();
    let orchestrator = svc.start_orchestrator(main, "claude", false, None).await.expect("Bob leads");
    let in_orchestrator = factory(&svc, None, Some(pid_of(&svc, orchestrator.id).await));
    assert!(create(&in_orchestrator, checkout, "claude").await.is_ok(), "on its lead's own login");
    svc.store.set_workspace_lead(main, Some("alice")).unwrap();
    assert_eq!(create(&in_orchestrator, checkout, "claude").await, Err("resource-conflict:lead_changed".to_string()));
    svc.store.set_workspace_lead(main, Some("bob")).unwrap();
    svc.store.remove_member("bob").unwrap();
    assert_eq!(svc.store.workspace_lead(main).unwrap(), None, "the owner leads again");
    // Its own login is a removed member's now: refused before the lead is read.
    assert_eq!(create(&in_orchestrator, checkout, "claude").await, Err("resource-conflict:login_gone".to_string()));
    let n = svc.store.list_terminals_for_worktree(checkout).unwrap();
    assert!(n.iter().all(|t| svc.store.terminal_login(t.id).unwrap().is_none_or(|l| l.handle == "bob")), "nothing on the owner's or Alice's");
}

/// A removed member's still-running pane opens nothing on whoever has their
/// handle now (review 2, M2): `login_gone`.
#[tokio::test]
async fn a_removed_members_pane_opens_nothing_on_a_new_holder_of_the_handle() {
    let (_dir, svc, _main, checkout) = runner().await;
    let old = create(&factory(&svc, Some("phone-b"), None), checkout, "claude").await.expect("Bob's pane");
    let in_old = factory(&svc, None, Some(pid_of(&svc, old).await));
    assert!(create(&in_old, checkout, "claude").await.is_ok(), "while Bob is Bob");
    svc.store.remove_member("bob").unwrap();
    svc.store.set_member("bob", "Another Bob", &["phone-b2".to_string()]).unwrap();
    assert_eq!(create(&in_old, checkout, "claude").await, Err("resource-conflict:login_gone".to_string()));
}

/// The version is asked in the pane's own worktree, on a first launch and a
/// split alike, never the daemon's directory (review 2).
#[tokio::test]
async fn the_version_is_asked_in_the_panes_worktree() {
    let (_dir, svc, _main, checkout) = runner().await;
    let worktree = PathBuf::from(svc.store.get_worktree(checkout).unwrap().worktree_path);
    let asked = |svc: &Service| svc.claude_logins.asked_in.lock().unwrap().last().cloned();
    let first = create(&factory(&svc, Some("phone-b"), None), checkout, "claude").await.expect("Bob's pane");
    assert_eq!(asked(&svc), Some(worktree.clone()), "first launch");
    svc.claude_logins.asked_in.lock().unwrap().clear();
    let peer = Peer { client_id: Some("phone-b".into()), scope: Scope::Control };
    launching(peer, None, svc.split_terminal(checkout, first, pb::SplitSide::Right, "Two", "claude")).await.expect("a split");
    assert_eq!(asked(&svc), Some(worktree), "split");
}
