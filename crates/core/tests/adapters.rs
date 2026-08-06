//! The built-in adapters, checked against the world they depend on.
//!
//! Both tests here exist because of one concrete near-miss: every current
//! search result and third-party document names `@zed-industries/codex-acp`
//! for codex, npm reports it deprecated in favour of
//! `@agentclientprotocol/codex-acp`, and it stalled at 0.16.0 against the live
//! 1.1.9. No amount of unit testing can catch that — only asking the outside
//! world can.

use farcooler_core::activity::Registry;

/// The npm package an adapter runs, if it runs one.
fn npm_package(spec: &farcooler_core::activity::AdapterSpec) -> Option<&str> {
    if spec.program != "npx" {
        return None;
    }
    spec.args
        .iter()
        .find(|a| !a.starts_with('-'))
        .map(|s| s.as_str())
}

#[test]
fn no_built_in_adapter_is_deprecated_on_npm() {
    for rules in Registry::built_in().all() {
        let Some(spec) = &rules.adapter else { continue };
        let Some(package) = npm_package(spec) else { continue };

        let out = std::process::Command::new("npm")
            .args(["view", package, "deprecated"])
            .output()
            .expect("npm must be installed to verify the adapters");
        assert!(
            out.status.success(),
            "{} names a package npm cannot resolve: {package}",
            rules.preset
        );
        let notice = String::from_utf8_lossy(&out.stdout);
        assert!(
            notice.trim().is_empty(),
            "{} uses a DEPRECATED package {package}: {}",
            rules.preset,
            notice.trim()
        );
    }
}

#[test]
fn every_built_in_adapter_completes_an_acp_handshake() {
    // A cold `npx` fetches a package on first use, so this is slow the first
    // time and fast afterwards. A missing program is a FAILURE, not a skip: on
    // a machine without the agent installed, silently passing would mean the
    // one test that can catch a broken adapter never runs where it matters.
    //
    // The handshake itself lives in `farcooler_core::activity` now rather than
    // here, so this test and the Test button in the machine-settings editor are
    // one implementation. Everything that used to be explained here — the
    // 90-second bound, the off-thread read, skipping chatter before the answer —
    // is documented on `handshake`, and unit-tested there against fakes that
    // need nothing from the network.
    use farcooler_core::activity::{HANDSHAKE_TIMEOUT, handshake};

    let mut failures = Vec::new();
    for rules in Registry::built_in().all() {
        let Some(spec) = &rules.adapter else { continue };
        if let Err(e) = handshake(spec, HANDSHAKE_TIMEOUT) {
            failures.push(format!("{}: {e}", rules.preset));
        }
    }
    assert!(
        failures.is_empty(),
        "adapters that could not handshake:\n{}",
        failures.join("\n")
    );
}
