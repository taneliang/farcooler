//! `host.health` as a phone's runner settings read it.

use super::health_json;

/// **The owner's key warning reaches the phone's runner settings** (ov-651):
/// the sentence for the names the runner sent, null for none and from an older
/// runner. The names only; nothing here has a value.
#[test]
fn host_health_carries_the_claude_key_warning() {
    let with = health_json(farcooler_protocol::v1::Host { rc_claude_keys: vec!["ANTHROPIC_API_KEY".into()], ..Default::default() });
    assert_eq!(
        with["claudeKeyWarning"],
        "Your shell’s startup files set ANTHROPIC_API_KEY, so a member typing claude in their own shell would use your account."
    );
    assert!(health_json(farcooler_protocol::v1::Host::default())["claudeKeyWarning"].is_null());
}
