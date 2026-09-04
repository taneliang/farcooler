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

/// Build the tree `npx` runs, before the handshake starts timing it.
///
/// The handshake's 90-second bound is there to catch an adapter that starts and
/// never answers, and it used to describe itself as "generous enough that a cold
/// `npx` fetching a package on first use is not killed mid-download". That
/// quietly stopped being safe to lean on. These packages carry their agent's
/// whole runtime now — the trees `npx` builds measure 261MB for claude and
/// 320MB for codex — and none of that work happens until the first run, so the
/// bound was covering an unbounded install and a protocol exchange at once.
///
/// Which one it was actually measuring showed up the day CI went red: this test
/// took 20s on one commit and 258s on the next, and the next touched only Swift.
/// All three adapters failed together with "the adapter started and then went
/// silent" — a sentence about OUR integration, for a run in which every one of
/// them answers `initialize` correctly the moment it is installed. Three
/// packages, from two orgs plus a standalone, do not break in lockstep; an
/// install budget they all share does. Ten hours later the same commit passed on
/// Linux and still failed on macOS, and by then only claude — the largest — was
/// over the line. That is a bound sitting on a threshold, not a broken adapter.
///
/// Installing first splits the two questions that failure was conflating. This
/// step is npm's to be slow at and is not bounded by us; the handshake after it
/// is timing the adapter and nothing else. Neither is a skip — an adapter whose
/// package will not install still FAILS this test, and has to, for the reason
/// the test gives three times over below — but the two no longer get blamed for
/// each other, and a red main now says which of them it was.
///
/// It has to be `npx` running the adapter's own package, because that is the
/// only thing that fills the `_npx` tree the handshake's `npx` then reuses.
/// `npm install` populates the tarball cache and leaves the tree still to build,
/// and `npm exec --package` skips installing altogether when the command it is
/// handed already exists. `--version` is what makes this terminate: the adapter
/// itself does not exit when its stdin closes — codex sits there indefinitely —
/// whereas every built-in answers `--version` and exits within a second.
fn install(spec: &farcooler_core::activity::AdapterSpec, package: &str) -> Result<(), String> {
    // Resolved through `dispatch::resolve` rather than spawned by name, so this
    // runs the very command the handshake is about to run — same `npx`, same
    // `PATH` — and cannot warm a tree some other node install would not find.
    let launch = farcooler_agent::dispatch::resolve(spec)?;
    let out = std::process::Command::new(&launch.program)
        .args(&launch.args)
        .arg("--version")
        .envs(launch.env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        // Bounds through npm's own config rather than a thread and a kill, and
        // through the environment rather than the argument list, which past the
        // package name belongs to the adapter. A registry that accepts the
        // connection and then stops answering has no timeout of its own, and
        // this repo has been bitten by that exact shape before — see the
        // `apt-get update` step of the workflow that runs this test, which hung
        // for an hour on two consecutive runs against a mirror doing it.
        .env("npm_config_fetch_timeout", "60000")
        .env("npm_config_fetch_retries", "3")
        // An install is not scoped to a project and should not be able to touch
        // one — the same reason the handshake itself runs in a temp directory.
        .current_dir(std::env::temp_dir())
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("could not start `{}`: {e}", launch.program.display()))?;
    if out.status.success() {
        return Ok(());
    }
    // npm puts the useful half in the first two lines — a code, then the URL or
    // the registry's own words. The rest is the "complete log" footer.
    let stderr = String::from_utf8_lossy(&out.stderr);
    let lines: Vec<&str> =
        stderr.lines().map(str::trim).filter(|l| !l.is_empty()).take(2).collect();
    let why =
        if lines.is_empty() { "npm said nothing".to_string() } else { lines.join("; ") };
    Err(format!(
        "npm could not install {package}, which is npm being unavailable rather than a \
         broken adapter: {why}"
    ))
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
    // A cold `npx` builds a several-hundred-megabyte tree on first use, so this
    // is slow the first time and fast afterwards. That install is done as its
    // own step by `install` above rather than inside the handshake's timeout —
    // see there for what it cost to have those two share one bound. A missing
    // program is a FAILURE, not a skip: on a machine without the agent
    // installed, silently passing would mean the one test that can catch a
    // broken adapter never runs where it matters.
    //
    // The handshake itself lives in the backend that performs it, and
    // `dispatch::handshake` chooses between them, so this test and the Test
    // button in the runner-settings editor are one implementation. Everything
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
        // Installing is npm's to be slow at; the handshake below is timing the
        // adapter. An adapter that cannot be installed is still a failure, so
        // nothing here can pass by being unable to try — it just says which of
        // the two went wrong instead of reporting npm's bad hour as ours.
        if let Some(package) = npm_package(spec) {
            if let Err(e) = install(spec, package) {
                failures.push(format!("{}: {e}", rules.preset));
                continue;
            }
        }
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
