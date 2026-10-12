use super::*;

const SHELL: Scope = Scope::HostAdmin;
const FIXTURE: &str = include_str!("../../../services/relay/test/fixtures/runner-proof.json");

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fc-runner-key-{name}-{}", uuid::Uuid::now_v7().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn field(name: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
    v[name].as_str().unwrap_or_else(|| panic!("fixture has no {name}")).to_string()
}

fn fixture_key() -> RunnerKey {
    let seed: Vec<u8> = (0..32)
        .map(|i| u8::from_str_radix(&field("seedHex")[i * 2..i * 2 + 2], 16).unwrap())
        .collect();
    RunnerKey::from_seed(&seed.try_into().unwrap())
}

fn fixture_issued() -> u64 {
    let v: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
    v["issuedAt"].as_u64().unwrap()
}

/// The same fixture is checked by the relay's own verifier
/// (`services/relay/test/directory.test.ts`). Changing either side's format
/// turns one of the two red.
#[test]
fn signs_exactly_the_fixture_the_relay_verifies() {
    let key = fixture_key();
    assert_eq!(key.public_line(), field("publicKey"));
    assert_eq!(
        statement(Action::Register, &field("account"), &field("team"), &field("install"), fixture_issued()),
        field("statement")
    );
    let proof = key
        .sign(SHELL, Action::Register, &field("account"), &field("team"), &field("install"), fixture_issued())
        .unwrap();
    assert_eq!(proof, field("proof"));
}

#[test]
fn a_claim_is_a_different_statement_from_a_registration() {
    let key = fixture_key();
    let a = key.sign(SHELL, Action::Register, "a", "t", "i", 1).unwrap();
    let b = key.sign(SHELL, Action::Claim, "a", "t", "i", 1).unwrap();
    assert_ne!(a, b);
    assert!(statement(Action::Claim, "a", "t", "i", 1).starts_with("farcooler claim-runner\n"));
}

#[test]
fn the_signature_verifies_in_its_namespace_only() {
    let key = fixture_key();
    let armored = key.sign(SHELL, Action::Register, "a", "t", "i", 7).unwrap();
    let sig = ssh_key::SshSig::from_pem(&armored).unwrap();
    let public = key.0.public_key();
    let message = statement(Action::Register, "a", "t", "i", 7);
    public.verify(NAMESPACE, message.as_bytes(), &sig).unwrap();
    assert!(public.verify("farcooler-grant-v1", message.as_bytes(), &sig).is_err());
    let other = statement(Action::Register, "b", "t", "i", 7);
    assert!(public.verify(NAMESPACE, other.as_bytes(), &sig).is_err());
}

#[test]
fn the_key_is_made_once_and_kept_private() {
    let dir = scratch("once");
    let first = RunnerKey::load_or_create_in(&dir).unwrap();
    let mode = std::fs::metadata(key_path(&dir)).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let second = RunnerKey::load_or_create_in(&dir).unwrap();
    assert_eq!(first.public_line(), second.public_line());
    let names: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(names.len(), 1, "a temporary file was left behind: {names:?}");
}

#[test]
fn two_runners_do_not_share_a_key() {
    let a = RunnerKey::load_or_create_in(&scratch("a")).unwrap();
    let b = RunnerKey::load_or_create_in(&scratch("b")).unwrap();
    assert_ne!(a.public_line(), b.public_line());
}

#[test]
fn a_key_other_users_can_read_is_refused_not_used() {
    let dir = scratch("exposed");
    RunnerKey::load_or_create_in(&dir).unwrap();
    std::fs::set_permissions(key_path(&dir), std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(matches!(RunnerKey::load_or_create_in(&dir), Err(KeyError::Exposed)));
}

#[test]
fn a_garbled_key_file_is_an_error_never_overwritten() {
    let dir = scratch("garbled");
    std::fs::write(key_path(&dir), "not a key").unwrap();
    std::fs::set_permissions(key_path(&dir), std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(RunnerKey::load_or_create_in(&dir).is_err());
    assert_eq!(std::fs::read_to_string(key_path(&dir)).unwrap(), "not a key");
}

#[test]
fn names_that_could_start_another_line_are_refused() {
    let key = fixture_key();
    for bad in ["", "a\nteam x", "a b", "a\r", "é"] {
        assert!(key.sign(SHELL, Action::Register, bad, "t", "i", 1).is_err(), "account {bad:?}");
        assert!(key.sign(SHELL, Action::Register, "a", bad, "i", 1).is_err(), "team {bad:?}");
        assert!(key.sign(SHELL, Action::Register, "a", "t", bad, 1).is_err(), "install {bad:?}");
    }
}

#[test]
fn debug_names_the_fingerprint_and_nothing_secret() {
    let key = fixture_key();
    let shown = format!("{key:?}");
    assert!(shown.starts_with("RunnerKey(SHA256:"));
    let seed = field("seedHex");
    assert!(!shown.contains(&seed));
}

#[test]
fn only_a_shell_may_have_the_box_sign() {
    let key = fixture_key();
    for scope in [Scope::Read, Scope::Control] {
        for action in [Action::Register, Action::Claim] {
            assert!(
                matches!(key.sign(scope, action, "a", "t", "i", 1), Err(KeyError::NotShell)),
                "{scope:?} {action:?} was signed for"
            );
        }
    }
    assert!(key.sign(Scope::HostAdmin, Action::Claim, "a", "t", "i", 1).is_ok());
}
