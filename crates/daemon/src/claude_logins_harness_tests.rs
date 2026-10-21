//! A member's Codex and Cursor are refused until they have a login of their
//! own (ov-652): their arms set nothing per member, so a pane would run on the
//! runner owner's ChatGPT or Cursor account.

use super::tests::{create, factory, runner};
use super::*;

fn conflict(r: std::result::Result<impl std::fmt::Debug, DomainError>) -> &'static str {
    match r {
        Err(DomainError::Conflict { what }) => what,
        other => panic!("not a conflict: {other:?}"),
    }
}

fn terminals(svc: &Service) -> usize {
    svc.store.list_worktrees_in_order().unwrap().iter().map(|w| svc.store.list_terminals_for_worktree(w.id).unwrap().len()).sum()
}

/// A member's codex pane and cursor pane, opened from their phone, are
/// refused with the harness's own word and nothing opens; the owner's opens.
#[tokio::test]
async fn a_members_codex_and_cursor_panes_are_refused_and_the_owners_open() {
    let (_dir, svc, _main, checkout) = runner().await;
    let before = terminals(&svc);
    let bob = factory(&svc, Some("phone-b"), None);
    assert_eq!(create(&bob, checkout, "codex").await, Err("resource-conflict:codex_sign_in_needed".to_string()));
    assert_eq!(create(&bob, checkout, "cursor").await, Err("resource-conflict:cursor_sign_in_needed".to_string()));
    assert_eq!(create(&bob, checkout, "codex:gpt-5").await, Err("resource-conflict:codex_sign_in_needed".to_string()), "a model on the preset too");
    assert_eq!(terminals(&svc), before, "nothing opened");

    let owner = factory(&svc, None, None);
    let owners = create(&owner, checkout, "codex").await.expect("the owner's codex opens");
    assert_eq!(svc.store.terminal_login(owners).unwrap(), None);
    create(&owner, checkout, "cursor").await.expect("the owner's cursor opens");
    create(&bob, checkout, "claude").await.expect("a member's claude is untouched");
    create(&bob, checkout, "shell").await.expect("and so is their shell");
}

/// A member lead's codex or cursor orchestrator is refused and leaves none;
/// the owner's starts.
#[tokio::test]
async fn a_member_leads_codex_orchestrator_is_refused() {
    let (_dir, svc, main, _checkout) = runner().await;
    svc.store.set_workspace_lead(main, Some("bob")).unwrap();
    let before = terminals(&svc);
    assert_eq!(conflict(svc.start_orchestrator(main, "codex", false, None).await), "codex_sign_in_needed");
    assert_eq!(conflict(svc.start_orchestrator(main, "cursor", false, None).await), "cursor_sign_in_needed");
    assert_eq!(terminals(&svc), before, "no orchestrator opened");

    svc.store.set_workspace_lead(main, None).unwrap();
    svc.start_orchestrator(main, "codex", false, None).await.expect("the owner's codex orchestrator");
}

/// A codex pane recorded on a member's login (one from before this refusal)
/// is not relaunched, whether through `relaunch_dir` or a restart.
#[tokio::test]
async fn a_relaunch_of_a_members_codex_pane_is_refused() {
    let (_dir, svc, _main, checkout) = runner().await;
    let pane = create(&factory(&svc, None, None), checkout, "codex").await.expect("a codex pane");
    let login_id = svc.store.ensure_member_login("bob").unwrap();
    let config_dir = ensure_login_dir(svc.root_dir(), &login_id).unwrap();
    svc.store.set_terminal_login(pane, &TerminalLogin { handle: "bob".into(), login_id, config_dir, shared: false }).unwrap();
    svc.stop_terminal(pane).await.unwrap();

    let term = svc.store.get_terminal(pane).unwrap();
    let path = PathBuf::from(svc.store.get_worktree(checkout).unwrap().worktree_path);
    assert_eq!(conflict(svc.relaunch_dir(&term, &path).await), "codex_sign_in_needed");
    assert_eq!(conflict(svc.restart_terminal(pane).await), "codex_sign_in_needed");
    assert!(!svc.is_running(&svc.store.get_terminal(pane).unwrap()), "still stopped");
}

/// The refusal raises the ask for that member alone, once, and a Claude
/// refusal or sign-in neither raises nor clears it.
#[tokio::test]
async fn the_ask_is_raised_for_that_member_only() {
    let (_dir, svc, _main, checkout) = runner().await;
    assert!(svc.login_needs().is_empty());
    create(&factory(&svc, Some("phone-b"), None), checkout, "codex").await.unwrap_err();
    create(&factory(&svc, Some("phone-b"), None), checkout, "cursor").await.unwrap_err();
    create(&factory(&svc, None, None), checkout, "codex").await.expect("the owner's");
    let needs: Vec<_> = svc.login_needs().into_iter().map(|n| (n.handle, n.why)).collect();
    assert_eq!(
        needs,
        vec![("bob".to_string(), crate::claude_login_renewal::Why::CodexAsked), ("bob".to_string(), crate::claude_login_renewal::Why::CursorAsked)]
    );
    let first = svc.store.login_flags_of("bob").unwrap().unwrap().codex_asked_at;
    create(&factory(&svc, Some("phone-b"), None), checkout, "codex").await.unwrap_err();
    assert_eq!(svc.store.login_flags_of("bob").unwrap().unwrap().codex_asked_at, first, "noted once");
    svc.store.record_sign_in("bob", 5).unwrap();
    assert_eq!(svc.login_needs().len(), 2, "signing in to Claude doesn't connect Codex");
}

/// A pane recorded on a removed member's login is `login_gone` on restart,
/// and the new member who reuses the handle is asked nothing.
#[tokio::test]
async fn a_restart_on_a_removed_members_login_asks_the_new_holder_nothing() {
    let (_dir, svc, _main, checkout) = runner().await;
    let pane = create(&factory(&svc, None, None), checkout, "codex").await.expect("a codex pane");
    let login_id = svc.store.ensure_member_login("bob").unwrap();
    let config_dir = ensure_login_dir(svc.root_dir(), &login_id).unwrap();
    svc.store.set_terminal_login(pane, &TerminalLogin { handle: "bob".into(), login_id, config_dir, shared: false }).unwrap();
    svc.stop_terminal(pane).await.unwrap();
    svc.store.remove_member("bob").unwrap();
    svc.store.set_member("bob", "Another Bob", &["phone-b2".to_string()]).unwrap();
    svc.store.ensure_member_login("bob").unwrap();

    assert_eq!(conflict(svc.restart_terminal(pane).await), "login_gone");
    assert!(svc.login_needs().is_empty(), "the new Bob is asked nothing");
}

/// The launch builder runs any identifier as a program, so a member gets only
/// presets Far Cooler names: `agent` (Cursor's alias) is unsupported, and
/// `Codex` or `Cursor-agent` are the harnesses refused whatever their case.
#[tokio::test]
async fn a_member_gets_only_presets_far_cooler_knows() {
    let (_dir, svc, _main, checkout) = runner().await;
    let bob = factory(&svc, Some("phone-b"), None);
    let before = terminals(&svc);
    assert_eq!(create(&bob, checkout, "agent").await, Err("resource-conflict:unsupported".to_string()));
    assert_eq!(create(&bob, checkout, "aider").await, Err("resource-conflict:unsupported".to_string()));
    assert_eq!(create(&bob, checkout, "Codex").await, Err("resource-conflict:codex_sign_in_needed".to_string()));
    assert_eq!(create(&bob, checkout, "Cursor-agent").await, Err("resource-conflict:cursor_sign_in_needed".to_string()));
    assert_eq!(terminals(&svc), before);
    create(&bob, checkout, "claude:opus").await.expect("claude with a model");
    create(&factory(&svc, None, None), checkout, "agent").await.expect("the owner's is unchanged");
}

/// A Codex or Cursor ask lasts a day, then goes from Needs You; a refusal
/// after that raises it again.
#[tokio::test]
async fn a_harness_ask_expires_after_a_day_and_is_raised_again() {
    let (_dir, svc, _main, _checkout) = runner().await;
    let day = farcooler_store::claude_logins::HARNESS_ASK_TTL_MS;
    svc.store.ensure_member_login("bob").unwrap();
    assert!(svc.store.mark_harness_asked("bob", "codex", 1_000).unwrap());
    assert_eq!(svc.login_needs_at(1_000 + day).len(), 1, "still there at a day");
    assert!(svc.login_needs_at(1_001 + day).is_empty(), "gone just after");
    assert!(!svc.store.mark_harness_asked("bob", "codex", 2_000).unwrap(), "not news while it lasts");
    assert!(svc.store.mark_harness_asked("bob", "codex", 1_001 + day).unwrap(), "news once expired");
    assert_eq!(svc.login_needs_at(1_002 + day).len(), 1);
}
