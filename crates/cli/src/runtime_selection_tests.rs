//! The CLI and daemon share one captured runtime selection.

#[test]
fn runtime_selection_cli_uses_the_shared_validated_capture() {
    use farcooler_daemon_contracts::paths;
    let selected = paths::selection().unwrap();
    assert_eq!(paths::runtime_dir().unwrap(), selected.root());
    assert_eq!(paths::runtime_dir_for(farcooler_protocol::CHANNEL).unwrap(), selected.root());
    for invalid in ["", "relative", "/tmp/../bad"] {
        assert!(paths::RuntimeSelection::from_override(farcooler_protocol::CHANNEL, Some(invalid.into())).is_err());
    }
}

#[test]
fn runtime_selection_daemon_exports_the_same_capture() {
    let daemon: &farcooler_daemon::paths::RuntimeSelection =
        farcooler_daemon::paths::selection().unwrap();
    let contracts = farcooler_daemon_contracts::paths::selection().unwrap();
    assert!(std::ptr::eq(daemon, contracts));
}
