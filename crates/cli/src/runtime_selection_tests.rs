//! The CLI consumes the daemon library's captured selector; no daemon launch.
#[test]
fn runtime_selection_cli_uses_the_shared_validated_capture() {
    use farcooler_daemon::paths;
    let selected = paths::selection().unwrap();
    assert_eq!(paths::runtime_dir().unwrap(), selected.root());
    assert_eq!(paths::runtime_dir_for(farcooler_protocol::CHANNEL).unwrap(), selected.root());
    for invalid in ["", "relative", "/tmp/../bad"] {
        assert!(paths::RuntimeSelection::from_override(farcooler_protocol::CHANNEL, Some(invalid.into())).is_err());
    }
}
