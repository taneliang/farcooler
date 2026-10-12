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
        "farcooler {statement_word}-runner\naccount acct-1\nteam team-1\ninstall {}\nissued {}\n",
        body["install"].as_str().unwrap(),
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

/// Held for an owner decision (ov-497 review B1): nothing produces a claim.
#[test]
fn there_is_no_claim_command_and_no_claim_proof_is_printed() {
    let home = tempfile::tempdir().unwrap();
    let out = run(home.path(), &["claim", "--account", "acct-1", "--team", "team-1"]);
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
    assert!(!home.path().join("runner-key").exists(), "a refused claim still made a key");
}

/// This signs with the key of the box it runs on, so `--runner` must not be
/// dropped on the floor: the Mac's key would sign for the owner's Mac.
#[test]
fn naming_another_runner_is_refused_not_ignored() {
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_farcooler"))
        .env("FARCOOLER_HOME", home.path())
        .args(["--runner", "team-box", "runner", "sign-registration", "--account", "acct-1", "--team", "team-1"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(out.stdout.is_empty(), "printed a proof for the wrong box");
    assert!(String::from_utf8_lossy(&out.stderr).contains("without --runner"));
    assert!(!home.path().join("runner-key").exists());
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
