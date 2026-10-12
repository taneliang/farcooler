use super::*;
use farcooler_daemon::runner_key::{statement, KeyError};

/// A runtime directory as `ensure_runtime_dir` leaves it: owner-only.
fn runtime_dir() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

fn request(dir: &std::path::Path, action: Action) -> serde_json::Value {
    serde_json::from_str(&proof_in(dir, action, "acct-1", "team-1", 1_790_000_000_000).unwrap()).unwrap()
}

/// What the relay does with the request, minus the database: parse the key it
/// was given, rebuild the statement from its own fields, verify.
fn relay_accepts(body: &serde_json::Value, action: Action, account: &str) -> bool {
    let key = ssh_key::PublicKey::from_openssh(body["runnerKey"].as_str().unwrap()).unwrap();
    let sig = ssh_key::SshSig::from_pem(body["proof"].as_str().unwrap()).unwrap();
    let message = statement(
        action,
        account,
        "team-1",
        body["install"].as_str().unwrap(),
        body["issuedAt"].as_u64().unwrap(),
    );
    key.verify("farcooler-runner-v1", message.as_bytes(), &sig).is_ok()
}

#[test]
fn registration_and_claim_each_carry_a_proof_for_their_own_statement() {
    let dir = runtime_dir();
    let register = request(dir.path(), Action::Register);
    let claim = request(dir.path(), Action::Claim);
    assert!(relay_accepts(&register, Action::Register, "acct-1"));
    assert!(relay_accepts(&claim, Action::Claim, "acct-1"));
    // A registration can't be replayed as a claim, nor a claim as a registration.
    assert!(!relay_accepts(&register, Action::Claim, "acct-1"));
    assert!(!relay_accepts(&claim, Action::Register, "acct-1"));
    // Nor for a different account.
    assert!(!relay_accepts(&claim, Action::Claim, "acct-2"));
}

#[test]
fn the_request_names_this_runners_install_and_one_key_every_time() {
    let dir = runtime_dir();
    let a = request(dir.path(), Action::Claim);
    let b = request(dir.path(), Action::Register);
    assert_eq!(a["runnerKey"], b["runnerKey"]);
    assert_eq!(a["install"], paths::load_or_create_install_id_in(dir.path()).unwrap().as_str());
}

#[test]
fn the_private_key_is_not_in_what_is_printed() {
    let dir = runtime_dir();
    let printed = proof_in(dir.path(), Action::Claim, "acct-1", "team-1", 1).unwrap();
    let file = std::fs::read_to_string(farcooler_daemon::runner_key::key_path(dir.path())).unwrap();
    assert!(file.contains("PRIVATE KEY"));
    for line in file.lines().filter(|l| l.len() > 20 && !l.starts_with("-----")) {
        assert!(!printed.contains(line), "a line of the private key was printed");
    }
    assert!(!printed.contains("PRIVATE KEY"));
}

#[test]
fn a_name_that_could_forge_a_statement_is_refused() {
    let dir = runtime_dir();
    let err = proof_in(dir.path(), Action::Claim, "acct-1\nteam other", "team-1", 1).unwrap_err();
    assert!(err.downcast_ref::<KeyError>().is_some());
}
