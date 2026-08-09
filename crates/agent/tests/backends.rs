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
fn every_built_in_backend_completes_a_handshake() {
    // A cold `npx` fetches a package on first use, so this is slow the first
    // time and fast afterwards. A missing program is a FAILURE, not a skip: on
    // a machine without the agent installed, silently passing would mean the
    // one test that can catch a broken adapter never runs where it matters.
    //
    // The handshake itself lives in the backend that performs it, and
    // `dispatch::handshake` chooses between them, so this test and the Test
    // button in the machine-settings editor are one implementation. Everything
    // that used to be explained here — the 90-second bound, the off-thread
    // read, skipping chatter before the answer — is documented on the ACP
    // handshake, and unit-tested there against fakes that need no network.
    //
    // It moved out of `crates/core/tests/` with the handshake: core sits below
    // every backend crate, so it cannot dispatch to one without inverting the
    // dependency graph.
    use farcooler_agent::dispatch::{HANDSHAKE_TIMEOUT, handshake};

    let mut failures = Vec::new();
    for rules in Registry::built_in().all() {
        let Some(spec) = &rules.adapter else { continue };
        if let Err(e) = handshake(&rules.preset, spec, HANDSHAKE_TIMEOUT) {
            failures.push(format!("{}: {e}", rules.preset));
        }
    }
    assert!(
        failures.is_empty(),
        "adapters that could not handshake:\n{}",
        failures.join("\n")
    );
}

#[test]
fn the_codex_native_backend_handshakes_against_the_installed_binary() {
    // The point of building this backend rather than stubbing it: it is the
    // only thing that proves the seam is cut in the right place before the
    // transcript work commits to it.
    //
    // A missing program is a FAILURE, not a skip, for the same reason the ACP
    // handshake test gives above: silently passing means the one test that can
    // catch this never runs where it matters.
    use farcooler_agent::dispatch::{HANDSHAKE_TIMEOUT, handshake};
    use farcooler_core::activity::{AdapterBackend, AdapterSpec};

    let spec = AdapterSpec {
        backend: AdapterBackend::Native,
        program: "codex".into(),
        // Empty: `app-server` belongs to the backend, not to config.
        args: Vec::new(),
        env: Default::default(),
    };
    let reported = handshake("codex", &spec, HANDSHAKE_TIMEOUT)
        .unwrap_or_else(|e| panic!("codex app-server handshake failed: {e}"));
    assert!(reported.contains("app-server"), "names what answered: {reported}");
}

#[test]
fn the_claude_native_backend_handshakes_against_the_installed_binary() {
    // As above: a missing program is a FAILURE, not a skip.
    //
    // This test is also the only automated proof that the CLAUDECODE scrub
    // works, because it runs from inside a Claude Code session more often than
    // not — without the scrub the CLI would neither answer nor exit and this
    // would fail on the timeout rather than on anything informative.
    use farcooler_agent::dispatch::{HANDSHAKE_TIMEOUT, handshake};
    use farcooler_core::activity::{AdapterBackend, AdapterSpec};

    let spec = AdapterSpec {
        backend: AdapterBackend::Native,
        program: "claude".into(),
        // Empty: the stream-json flags belong to the backend, not to config.
        args: Vec::new(),
        env: Default::default(),
    };
    let reported = handshake("claude", &spec, HANDSHAKE_TIMEOUT)
        .unwrap_or_else(|e| panic!("claude stream-json handshake failed: {e}"));
    assert!(reported.contains("stream-json"), "names what answered: {reported}");
}
