use super::*;

fn peer(scope: Scope) -> Peer {
    Peer { client_id: None, scope }
}
fn shell() -> Peer {
    peer(Scope::HostAdmin)
}
const RELAY: &str = "https://relay.test";
const FIXTURE: &str = include_str!("../../../services/relay/test/fixtures/runner-proof.json");

fn scratch(name: &str) -> PathBuf {
    let dir = farcooler_core::scratch::dir(&format!("runner-key-{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
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
        statement(Action::Register, &field("account"), &field("team"), &field("install"), &field("relay"), fixture_issued()),
        field("statement")
    );
    let proof = key
        .sign(&shell(), Action::Register, &field("account"), &field("team"), &field("install"), &field("relay"), fixture_issued())
        .unwrap();
    assert_eq!(proof, field("proof"));
}

#[test]
fn a_claim_is_a_different_statement_from_a_registration() {
    let key = fixture_key();
    let a = key.sign(&shell(), Action::Register, "a", "t", "i", RELAY, 1).unwrap();
    let b = key.sign(&shell(), Action::Claim, "a", "t", "i", RELAY, 1).unwrap();
    assert_ne!(a, b);
    assert!(statement(Action::Claim, "a", "t", "i", RELAY, 1).starts_with("farcooler claim-runner\n"));
}

#[test]
fn the_signature_verifies_in_its_namespace_only() {
    let key = fixture_key();
    let armored = key.sign(&shell(), Action::Register, "a", "t", "i", RELAY, 7).unwrap();
    let sig = ssh_key::SshSig::from_pem(&armored).unwrap();
    let public = key.0.public_key();
    let message = statement(Action::Register, "a", "t", "i", RELAY, 7);
    public.verify(NAMESPACE, message.as_bytes(), &sig).unwrap();
    assert!(public.verify("farcooler-grant-v1", message.as_bytes(), &sig).is_err());
    let other = statement(Action::Register, "b", "t", "i", RELAY, 7);
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
        assert!(key.sign(&shell(), Action::Register, bad, "t", "i", RELAY, 1).is_err(), "account {bad:?}");
        assert!(key.sign(&shell(), Action::Register, "a", bad, "i", RELAY, 1).is_err(), "team {bad:?}");
        assert!(key.sign(&shell(), Action::Register, "a", "t", bad, RELAY, 1).is_err(), "install {bad:?}");
    }
}

#[test]
fn debug_names_the_fingerprint_and_nothing_secret() {
    let dir = scratch("debug");
    let key = RunnerKey::load_or_create_in(&dir).unwrap();
    let shown = format!("{key:?}");
    assert!(shown.starts_with("RunnerKey(SHA256:"));
    let file = std::fs::read_to_string(key_path(&dir)).unwrap();
    let body: Vec<_> = file.lines().filter(|l| l.len() > 20 && !l.starts_with("-----")).collect();
    assert!(!body.is_empty());
    for line in body {
        assert!(!shown.contains(line), "Debug printed the key file");
    }
    assert!(!shown.contains("PRIVATE"));
}

#[test]
fn only_a_host_admin_peer_may_have_the_box_sign() {
    let key = fixture_key();
    // A phone with `control` asking over the tunnel is a peer with a device id.
    let phone = Peer { client_id: Some("phone".into()), scope: Scope::Control };
    for peer in [phone, self::peer(Scope::Read), self::peer(Scope::Control)] {
        assert!(
            matches!(key.sign(&peer, Action::Register, "a", "t", "i", RELAY, 1), Err(KeyError::NotShell)),
            "{peer:?} was signed for"
        );
    }
    assert!(key.sign(&shell(), Action::Register, "a", "t", "i", RELAY, 1).is_ok());
}

#[test]
fn an_install_id_the_relay_would_refuse_is_not_signed() {
    let key = fixture_key();
    for bad in ["a.b", "a:b", "a_b", &"x".repeat(65), ""] {
        assert!(
            matches!(key.sign(&shell(), Action::Register, "a", "t", bad, RELAY, 1), Err(KeyError::Field("install"))),
            "{bad:?}"
        );
    }
    assert!(key.sign(&shell(), Action::Register, "a", "t", &"x".repeat(64), RELAY, 1).is_ok());
}

#[test]
fn creators_racing_end_with_one_key_and_nothing_else() {
    for round in 0..20 {
        let dir = scratch(&format!("race{round}"));
        let lines: Vec<String> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..8)
                .map(|_| s.spawn(|| RunnerKey::load_or_create_in(&dir).unwrap().public_line()))
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert!(lines.iter().all(|l| *l == lines[0]), "two keys were handed out: {lines:?}");
        assert_eq!(RunnerKey::load_or_create_in(&dir).unwrap().public_line(), lines[0]);
        let names: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names.len(), 1, "{names:?}");
    }
}

#[test]
fn a_temporary_left_by_a_crash_is_swept_and_the_key_stays() {
    let dir = scratch("crash");
    let key = RunnerKey::load_or_create_in(&dir).unwrap();
    let stray = dir.join(".runner-key.0123");
    std::fs::hard_link(key_path(&dir), &stray).unwrap();
    let again = RunnerKey::load_or_create_in(&dir).unwrap();
    assert!(!stray.exists(), "a second name for the private key survived");
    assert_eq!(again.public_line(), key.public_line());
}

#[test]
fn a_key_that_is_a_link_is_refused() {
    let dir = scratch("link");
    let other = scratch("elsewhere");
    let theirs = RunnerKey::load_or_create_in(&other).unwrap();
    std::os::unix::fs::symlink(key_path(&other), key_path(&dir)).unwrap();
    let got = RunnerKey::load_or_create_in(&dir);
    assert!(got.is_err(), "signed with a key found through a link: {:?}", got.map(|k| k.public_line()));
    drop(theirs);
}

#[test]
fn a_runtime_folder_others_can_enter_is_refused() {
    let dir = scratch("open-dir");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(RunnerKey::load_or_create_in(&dir), Err(KeyError::Exposed)));
    assert!(!key_path(&dir).exists());
}

/// A proof is for the relay it names (ov-497 review N3): the statement carries
/// it, so a proof printed for one relay doesn't verify against another's.
#[test]
fn a_proof_is_bound_to_the_relay_it_was_printed_for() {
    let key = fixture_key();
    let a = key.sign(&shell(), Action::Claim, "a", "t", "i", "https://relay.test", 1).unwrap();
    let b = key.sign(&shell(), Action::Claim, "a", "t", "i", "https://relay-preview.test", 1).unwrap();
    assert_ne!(a, b);
    let sig = ssh_key::SshSig::from_pem(&a).unwrap();
    let public = key.0.public_key();
    let elsewhere = statement(Action::Claim, "a", "t", "i", "https://relay-preview.test", 1);
    assert!(public.verify(NAMESPACE, elsewhere.as_bytes(), &sig).is_err());
    for bad in ["", "relay.test", "https://a\nb", "https://a b"] {
        assert!(
            matches!(key.sign(&shell(), Action::Claim, "a", "t", "i", bad, 1), Err(KeyError::Field("relay"))),
            "relay {bad:?}"
        );
    }
}
