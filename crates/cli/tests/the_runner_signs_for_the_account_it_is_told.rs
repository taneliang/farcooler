//! `farcooler runner claim` and `sign-registration`, as a person or the app
//! runs them: the real binary, a scratch runtime directory, and the relay's
//! own statement spelled out here rather than imported, so a drift in either
//! shows up.

use std::process::{Command, Output};

fn run(home: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_farcooler"))
        .env("FARCOOLER_HOME", home)
        .args(["runner"])
        .args(args)
        .output()
        .expect("run farcooler")
}

fn verifies(out: &Output, statement_word: &str) -> bool {
    let body: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json on stdout");
    let key = ssh_key::PublicKey::from_openssh(body["runnerKey"].as_str().unwrap()).unwrap();
    let sig = ssh_key::SshSig::from_pem(body["proof"].as_str().unwrap()).unwrap();
    let message = format!(
        "farcooler {statement_word}-runner\naccount acct-1\nteam team-1\ninstall {}\nrelay {}\nissued {}\n",
        body["install"].as_str().unwrap(),
        body["relay"].as_str().unwrap(),
        body["issuedAt"]
    );
    key.verify("farcooler-runner-v1", message.as_bytes(), &sig).is_ok()
}

#[test]
fn sign_registration_signs_a_registration_and_only_that() {
    let home = tempfile::tempdir().unwrap();
    let register = run(home.path(), &["sign-registration", "--account", "acct-1", "--team", "team-1"]);
    assert!(register.status.success(), "{}", String::from_utf8_lossy(&register.stderr));
    assert!(verifies(&register, "register"));
    assert!(!verifies(&register, "claim"));
}

/// The owner's decision (ov-497): anyone who runs as the runner's Unix user is
/// trusted, so `runner claim` prints a claim at once.
#[test]
fn claim_signs_a_claim_and_never_a_registration() {
    let home = tempfile::tempdir().unwrap();
    let out = run(home.path(), &["claim", "--account", "acct-1", "--team", "team-1"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(verifies(&out, "claim"));
    assert!(!verifies(&out, "register"));
}

/// The proof names the relay it is for, this build's own unless told.
#[test]
fn a_proof_names_its_relay_and_only_verifies_for_it() {
    let home = tempfile::tempdir().unwrap();
    let out = run(home.path(), &["claim", "--account", "acct-1", "--team", "team-1", "--relay", "https://relay-staging.test"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let body: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(body["relay"], "https://relay-staging.test");
    let default = run(home.path(), &["claim", "--account", "acct-1", "--team", "team-1"]);
    let body: serde_json::Value = serde_json::from_slice(&default.stdout).unwrap();
    assert!(body["relay"].as_str().unwrap().starts_with("https://relay"), "{body}");
    assert!(body["relay"] != "https://relay-staging.test");
}

#[test]
fn nothing_secret_is_printed_and_the_key_stays_private() {
    use std::os::unix::fs::PermissionsExt as _;
    let home = tempfile::tempdir().unwrap();
    let out = run(home.path(), &["sign-registration", "--account", "acct-1", "--team", "team-1"]);
    let file = std::fs::read_to_string(home.path().join("runner-key")).expect("the key was made");
    let mode = std::fs::metadata(home.path().join("runner-key")).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    let seen = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(!seen.contains("PRIVATE KEY"));
    for line in file.lines().filter(|l| l.len() > 20 && !l.starts_with("-----")) {
        assert!(!seen.contains(line), "a line of the private key was printed");
    }
}

#[test]
fn a_name_that_could_forge_a_statement_is_refused_and_prints_no_proof() {
    let home = tempfile::tempdir().unwrap();
    let out = run(home.path(), &["sign-registration", "--account", "acct-1\nteam other", "--team", "team-1"]);
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
}

/// With `--runner`, the box signs: the command goes over ssh to that runner's
/// own CLI and its answer comes back on stdout. A stand-in `ssh` records what
/// it was asked and answers with a marker, so nothing leaves this machine.
#[test]
fn with_runner_the_signature_is_asked_of_that_runner_over_ssh() {
    for verb in ["sign-registration", "claim"] {
        asked_over_ssh(verb);
    }
}

fn asked_over_ssh(verb: &str) {
    use std::os::unix::fs::PermissionsExt as _;
    let bin = tempfile::tempdir().unwrap();
    let log = bin.path().join("asked");
    let ssh = bin.path().join("ssh");
    std::fs::write(
        &ssh,
        format!("#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\"; done > '{}'\nprintf '{{\"proof\":\"from-the-box\"}}'\n", log.display()),
    )
    .unwrap();
    std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o755)).unwrap();
    let home = tempfile::tempdir().unwrap();
    let path = format!("{}:{}", bin.path().display(), std::env::var("PATH").unwrap_or_default());

    let out = Command::new(env!("CARGO_BIN_EXE_farcooler"))
        .env("FARCOOLER_HOME", home.path())
        .env("PATH", path)
        .args(["--runner", "dev@box", "runner", verb, "--account", "acct-1", "--team", "team-1"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8_lossy(&out.stdout), r#"{"proof":"from-the-box"}"#);
    let asked = std::fs::read_to_string(&log).unwrap();
    assert!(asked.lines().any(|l| l == "dev@box"), "{asked}");
    assert!(asked.contains(&format!("runner {verb} --account 'acct-1' --team 'team-1'")), "{asked}");
    // Nothing was signed here: the scratch runtime directory has no key.
    assert!(!home.path().join("runner-key").exists());
}
