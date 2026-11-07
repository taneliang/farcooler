//! Private fixtures only: never run old normal startup against installed paths.
use super::*;

fn private_root() -> (tempfile::TempDir, PathBuf) {
    // Short Unix socket paths without changing the machine-wide FIFO's TMPDIR.
    let dir = tempfile::Builder::new().prefix("ov937-").tempdir_in("/tmp").unwrap();
    let root = dir.path().canonicalize().unwrap();
    (dir, root)
}

#[tokio::test]
async fn runtime_selection_normal_explicit_start_keeps_private_paths() {
    let (_dir, root) = private_root();
    let selected = paths::RuntimeSelection::from_override(farcooler_protocol::CHANNEL, Some(root.clone().into_os_string())).unwrap();
    let service = Service::open_selected(&selected).await.unwrap();
    assert!(service.private_runtime);
    assert_eq!(service.authorized_keys(), scratch_authorized_keys(&root));
    assert_eq!(service.claude_config, root.join("claude/.claude.json"));
    assert_eq!(service.agent_home(), Some(root.join("home").as_path()));
    assert_eq!(service.claude_logins.runner, Some(root.join("home/.claude")));
    assert_eq!(service.log_home(), Some(root.join("home")));
    assert_eq!(service.log_roots(), vec![root.join("home/.claude/projects"), root.join("home/.codex/sessions"), root.join("home/.cursor/projects")]);
    assert_eq!(service.rep_cli(), Some(Path::new(&cli_binary(std::env::current_exe().ok().as_deref()))));
    assert!(root.join("farcooler.db").exists());
    std::fs::create_dir_all(root.join("claude")).unwrap();
    std::fs::write(&service.claude_config, r#"{"oauthAccount":{"emailAddress":"fixture@example.invalid"}}"#).unwrap();
    assert_eq!(service.claude_account().as_deref(), Some("fixture@example.invalid"));
    assert!(service.authorized_keys().starts_with(&root));
    drop(service);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn runtime_selection_default_binding_keeps_installed_precedence_without_opening_it() {
    let (_dir, root) = private_root();
    let mut service = Service::open_in(root.clone()).await.unwrap();
    let selected = paths::RuntimeSelection::from_override(farcooler_protocol::CHANNEL, None).unwrap();
    // Bind names only: do not open the installed runtime, credentials or logs.
    service.bind_runtime_paths(&selected);
    assert!(!service.private_runtime);
    assert_eq!(service.authorized_keys(), default_authorized_keys());
    assert_eq!(service.claude_config, crate::people::runner_claude_config());
    assert_eq!(service.agent_home, user_home());
    assert_eq!(service.claude_logins.runner, crate::claude_logins::runner_dir(std::env::var_os("CLAUDE_CONFIG_DIR"), user_home().as_deref()));
    assert_eq!(service.log_roots(), crate::log_watch::roots(service.claude_logins.runner.as_deref()));
    assert_eq!(service.rep_cli(), Some(Path::new(&cli_binary(std::env::current_exe().ok().as_deref()))));
    drop(service);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn runtime_selection_private_fence_migration_caps_allowlist_and_revoke() {
    use farcooler_fence as fence;
    use farcooler_protocol::v1::Scope;
    use ssh_key::public::{Ed25519PublicKey, KeyData};
    let (_dir, root) = private_root();
    let selected = paths::RuntimeSelection::from_override(farcooler_protocol::CHANNEL, Some(root.clone().into_os_string())).unwrap();
    let service = Arc::new(Service::open_selected(&selected).await.unwrap());
    // Synthetic public keys, no enrollment/tunnel/provider entry point.
    let lines = [(1, "reader", Scope::Read), (2, "control", Scope::Control)].map(|(n, id, scope)| {
        let key = ssh_key::PublicKey::new(KeyData::Ed25519(Ed25519PublicKey([n; 32])), "fixture").to_openssh().unwrap();
        fence::render(&key, "fixture", id, scope, fence::Grant::FarCooler, None).unwrap()
    });
    fence::write(service.authorized_keys(), fence::AUTHORIZED_KEYS, &lines, &[], fence::Placement::Last).unwrap();
    crate::access_migration::migrate_at_start(&service).await;
    assert!(service.store.access_migrated().unwrap());
    let entries = fence::read(service.authorized_keys(), fence::AUTHORIZED_KEYS).unwrap();
    assert_eq!(entries.iter().map(|e| e.scope).collect::<Vec<_>>(), vec![Scope::Read, Scope::Control]);
    let access = service.store.stored_access().unwrap();
    assert_eq!(access.caps.iter().find(|(id, _)| id == "reader").map(|(_, cap)| *cap), Some(farcooler_store::access::Cap::Viewer));
    assert!(crate::allowlist::from_entries(&entries).is_none(), "no node keys must admit nobody");
    let session = service.sessions().open(Some("reader".into()));
    assert_eq!(crate::enrollment::revoke_many(&service, vec!["reader".into()], |_| true, false).await.unwrap(), vec!["reader"]);
    assert!(session.is_closed());
    let entries = fence::read(service.authorized_keys(), fence::AUTHORIZED_KEYS).unwrap();
    assert_eq!(entries.iter().map(|e| e.client_id.as_str()).collect::<Vec<_>>(), vec!["control"]);
    assert!(!service.tailcat_key().exists(), "no tunnel identity generated");
    drop(service);
    std::fs::remove_dir_all(root).unwrap();
}
