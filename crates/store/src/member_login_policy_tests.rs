use super::*;
use crate::claude_logins::TerminalLogin;

fn store(tag: &str) -> (std::path::PathBuf, Store) {
    let dir = farcooler_core::scratch::dir(tag);
    std::fs::create_dir_all(&dir).unwrap();
    let store = Store::open(dir.join("state.db")).unwrap();
    (dir, store)
}

/// Off for every harness until an admin sets one; each is set on its own,
/// kept across a reopen, with who set it and when.
#[test]
fn off_by_default_and_set_per_harness() {
    let (dir, s) = store("member-login-policy");
    assert_eq!(s.member_login_policy().unwrap(), MemberLoginPolicy::default(), "off by default");
    assert_eq!(s.member_login_policy_set("codex").unwrap(), None);
    assert!(s.set_member_login_policy("codex", true, "phone-1", 10).unwrap(), "a change");
    assert!(!s.set_member_login_policy("codex", true, "phone-2", 20).unwrap(), "the same again is no change");
    assert_eq!(s.member_login_policy().unwrap(), MemberLoginPolicy { codex: true, ..Default::default() }, "only codex");
    assert_eq!(s.member_login_policy_set("codex").unwrap(), Some(("phone-2".to_string(), 20)), "the last to set it");
    assert!(!s.set_member_login_policy("claude", false, "-", 30).unwrap(), "off is the default, so no change");
    assert!(matches!(s.set_member_login_policy("aider", true, "-", 40), Err(DomainError::InvalidArgument { what: "harness" })));
    drop(s);
    let s = Store::open(dir.join("state.db")).unwrap();
    assert_eq!(s.member_login_policy().unwrap().members_own("codex"), Some(true), "kept");
    assert!(s.set_member_login_policy("codex", false, "-", 50).unwrap());
    assert_eq!(s.member_login_policy().unwrap(), MemberLoginPolicy::default());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A pane's shared flag is written with its login, read back, and changed
/// only for the generation it records.
#[test]
fn a_panes_shared_flag_changes_only_for_its_generation() {
    let (dir, s) = store("shared-terminal-login");
    let host = uuid::Uuid::now_v7();
    let root = s.create_repository_root(host, "/wt/shared", 1_000).unwrap();
    let repo = s.create_repository(host, root.id, "name", "/gitdir", "origin").unwrap();
    let ws = s.create_worktree(repo.id, "feature/x", "/wt/shared", false).unwrap();
    let pane = s.create_terminal(ws.id, "Agent", "claude", farcooler_protocol::v1::TerminalIntent::Running, 120, 40).unwrap().id;
    let shared = TerminalLogin { handle: "bob".into(), login_id: "id-1".into(), config_dir: "/r/claude/id-1".into(), shared: true };
    s.set_terminal_login(pane, &shared).unwrap();
    assert_eq!(s.terminal_login(pane).unwrap(), Some(shared.clone()));
    assert!(!s.set_terminal_login_shared(pane, "id-2", false).unwrap(), "another generation's");
    assert!(s.set_terminal_login_shared(pane, "id-1", false).unwrap());
    assert_eq!(s.terminal_login(pane).unwrap(), Some(TerminalLogin { shared: false, ..shared.clone() }));
    assert!(!s.set_terminal_login_shared(pane, "id-1", false).unwrap(), "no change");
    assert!(s.set_terminal_login_shared(pane, "id-1", true).unwrap(), "and back");
    assert_eq!(s.terminal_login(pane).unwrap(), Some(shared));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The migration is the 48th (0057) and `Welcome`: a build from before it
/// never names the table or the column.
#[test]
fn the_migration_is_welcome() {
    let it = &crate::migrate::MIGRATIONS[47];
    assert!(std::ptr::fn_addr_eq(it.0, migration_0057_member_login_policy as fn(&rusqlite::Transaction) -> rusqlite::Result<()>));
    assert_eq!(it.1, crate::compat::Older::Welcome);
}
