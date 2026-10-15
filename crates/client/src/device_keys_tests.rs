use russh::keys::ssh_key::private::{Ed25519Keypair, KeypairData};
use russh::keys::ssh_key::{LineEnding, PrivateKey, SshSig};
use serde_json::Value;

use super::*;

/// Pinned by the relay's tests too (`device-registration.test.ts`), so the
/// two statements can't drift apart.
const FIXTURE: &str = include_str!("../../../services/relay/test/fixtures/device-proof.json");

fn fixture() -> Value {
    serde_json::from_str(FIXTURE).unwrap()
}

/// The OpenSSH private key for a 32-byte seed in hex, with a comment, as an
/// app stores one.
fn key_from(seed_hex: &str) -> String {
    let seed: Vec<u8> = (0..seed_hex.len()).step_by(2).map(|i| u8::from_str_radix(&seed_hex[i..i + 2], 16).unwrap()).collect();
    let pair = Ed25519Keypair::from_seed(&seed.try_into().unwrap());
    let key = PrivateKey::new(KeypairData::Ed25519(pair), "farcooler-Somebody’s MacBook").unwrap();
    key.to_openssh(LineEnding::LF).unwrap().to_string()
}

fn random_key() -> String {
    let key = PrivateKey::random(&mut rand::rng(), ssh_key::Algorithm::Ed25519).unwrap();
    key.to_openssh(LineEnding::LF).unwrap().to_string()
}

const CHALLENGE: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// The proof is exactly what the relay's tests verify: same key, same
/// account, same challenge, same bytes.
#[test]
fn a_registration_matches_the_relays_fixture() {
    let f = fixture();
    let made = registration(&key_from(f["seedHex"].as_str().unwrap()), f["account"].as_str().unwrap(), f["challenge"].as_str().unwrap())
        .unwrap();
    assert_eq!(made.key_a, f["publicKey"].as_str().unwrap());
    assert_eq!(made.proof_a, f["proof"].as_str().unwrap());
    let fingerprint = crate::ceremony::fingerprint(&made.key_a).unwrap();
    assert_eq!(
        register_statement(f["account"].as_str().unwrap(), f["challenge"].as_str().unwrap(), &fingerprint, None, None),
        f["statement"].as_str().unwrap()
    );
}

/// The signature verifies, by Key A, in the register namespace, over the
/// statement naming this account and this challenge, and over nothing else.
#[test]
fn a_registration_is_signed_by_key_a_over_the_statement() {
    let private = random_key();
    let made = registration(&private, "user_01ALICE", CHALLENGE).unwrap();
    let public = PublicKey::from_openssh(&made.key_a).unwrap();
    let signature = SshSig::from_pem(&made.proof_a).unwrap();
    let fingerprint = public.fingerprint(HashAlg::Sha256).to_string();
    let statement = register_statement("user_01ALICE", CHALLENGE, &fingerprint, None, None);
    public.verify(REGISTER_NAMESPACE, statement.as_bytes(), &signature).expect("verifies");
    assert!(public.verify(CERTIFY_NAMESPACE, statement.as_bytes(), &signature).is_err());
    let other = register_statement("user_01BOB", CHALLENGE, &fingerprint, None, None);
    assert!(public.verify(REGISTER_NAMESPACE, other.as_bytes(), &signature).is_err());
}

/// What leaves is the public key with no comment, and never the private one.
#[test]
fn only_the_bare_public_key_leaves() {
    let made = registration(&key_from(&"11".repeat(32)), "user_01ALICE", CHALLENGE).unwrap();
    assert!(made.key_a.starts_with("ssh-ed25519 ") && made.key_a.split(' ').count() == 2, "{}", made.key_a);
    assert!(!format!("{made:?}").contains("PRIVATE"));
    assert!(!made.key_a.contains("MacBook"));
}

/// A challenge is taken only in the shape the relay mints, so a relay can't
/// write a line of its own into what this device signs.
#[test]
fn a_challenge_in_any_other_shape_is_refused() {
    let key = random_key();
    let injected = format!("{}\nkey-a SHA256:somebody-elses", &CHALLENGE[..32]);
    for challenge in ["", "abc", &CHALLENGE.to_uppercase(), &format!("{CHALLENGE}0"), &CHALLENGE[1..], injected.as_str(), &format!("{} ", &CHALLENGE[1..])] {
        assert_eq!(registration(&key, "user_01ALICE", challenge), Err(DeviceKeyError::Challenge), "{challenge:?}");
    }
}

/// An account id can't end the line it is on either.
#[test]
fn an_account_in_any_other_shape_is_refused() {
    let key = random_key();
    for account in ["", "user one", "user\nchallenge 00", &"a".repeat(129), "user/one"] {
        assert_eq!(registration(&key, account, CHALLENGE), Err(DeviceKeyError::Account), "{account:?}");
    }
}

/// Only an unencrypted Ed25519 private key signs; a public key, junk, or
/// another algorithm is a refusal by name.
#[test]
fn only_an_ed25519_private_key_signs() {
    let public = PrivateKey::from_openssh(random_key()).unwrap().public_key().to_openssh().unwrap();
    for key in ["", "junk", public.as_str()] {
        assert_eq!(registration(key, "user_01ALICE", CHALLENGE), Err(DeviceKeyError::Key), "{key:?}");
    }
    let encrypted = PrivateKey::from_openssh(random_key()).unwrap().encrypt(&mut rand::rng(), "pw").unwrap();
    assert_eq!(
        registration(&encrypted.to_openssh(LineEnding::LF).unwrap(), "user_01ALICE", CHALLENGE),
        Err(DeviceKeyError::Key)
    );
}

/// A certificate is what the relay's tests verify, and covers the keys it was
/// handed, by fingerprint.
#[test]
fn a_certificate_matches_the_relays_fixture() {
    let f = &fixture()["certificate"];
    let made = certificate(
        &key_from(f["voucherSeedHex"].as_str().unwrap()),
        f["account"].as_str().unwrap(),
        f["keyA"].as_str().unwrap(),
        None,
        f["node"].as_str(),
    )
    .unwrap();
    assert_eq!(made, f["certificate"].as_str().unwrap());
}

#[test]
fn a_certificate_refuses_what_it_cannot_name() {
    let voucher = random_key();
    let other = PrivateKey::from_openssh(random_key()).unwrap().public_key().to_openssh().unwrap();
    assert_eq!(certificate(&voucher, "user_01ALICE", "junk", None, None), Err(DeviceKeyError::Key));
    assert_eq!(certificate(&voucher, "user_01ALICE", &other, Some(&other), None), Err(DeviceKeyError::Key));
    assert_eq!(certificate(&voucher, "user_01ALICE", &other, None, Some("nodekey:XY")), Err(DeviceKeyError::Node));
    assert_eq!(certificate(&voucher, "bad account", &other, None, None), Err(DeviceKeyError::Account));
    assert!(certificate(&voucher, "user_01ALICE", &other, None, None).is_ok());
}

/// What the ceremony signs for a scanned code (ov-550) is what the relay's
/// tests accept for a device that registered Key A alone: same voucher, same
/// account, same key, same bytes.
#[test]
fn a_scanned_certificate_matches_the_relays_fixture() {
    let f = &fixture()["scanned"];
    let made = certificate_for_scanned(
        &key_from(f["voucherSeedHex"].as_str().unwrap()),
        f["account"].as_str().unwrap(),
        f["keyA"].as_str().unwrap(),
    )
    .unwrap();
    assert_eq!(made, f["certificate"].as_str().unwrap());
    let fingerprint = crate::ceremony::fingerprint(f["keyA"].as_str().unwrap()).unwrap();
    assert_eq!(certify_statement(f["account"].as_str().unwrap(), &fingerprint, None, None), f["statement"].as_str().unwrap());
}

/// A code carrying this device's own key is not another device, with or
/// without the comment an app's key line carries.
#[test]
fn a_device_never_certifies_itself() {
    let voucher = random_key();
    let own = PrivateKey::from_openssh(&voucher).unwrap().public_key().to_openssh().unwrap();
    assert_eq!(certificate_for_scanned(&voucher, "user_01ALICE", &own), Err(DeviceKeyError::Key));
    assert_eq!(certificate_for_scanned(&voucher, "user_01ALICE", &format!("{own} farcooler-Mac")), Err(DeviceKeyError::Key));
    assert_eq!(certificate_for_scanned(&voucher, "user_01ALICE", "junk"), Err(DeviceKeyError::Key));
    let other = PrivateKey::from_openssh(random_key()).unwrap().public_key().to_openssh().unwrap();
    let made = certificate_for_scanned(&voucher, "user_01ALICE", &other).unwrap();
    let fingerprint = PublicKey::from_openssh(&other).unwrap().fingerprint(HashAlg::Sha256).to_string();
    let signature = SshSig::from_pem(&made).unwrap();
    PublicKey::from_openssh(&own)
        .unwrap()
        .verify(CERTIFY_NAMESPACE, certify_statement("user_01ALICE", &fingerprint, None, None).as_bytes(), &signature)
        .expect("signed by the voucher over Key A alone");
}

/// A phone's "Make a New Key" (ov-569): the proof verifies by the NEW key, in
/// the rekey namespace, over the old and new keys' fingerprints, and over
/// nothing else.
#[test]
fn a_rekey_proof_is_signed_by_the_new_key_over_both_keys() {
    let (old, new) = (random_key(), random_key());
    let old_public = PrivateKey::from_openssh(&old).unwrap().public_key().to_openssh().unwrap();
    let made = rekey_proof(&new, &old_public).unwrap();
    let public = PublicKey::from_openssh(&made.public_key).unwrap();
    assert_eq!(made.public_key, bare(&public).unwrap(), "the key carried a comment");
    assert_eq!(Some(made.client_id.clone()), crate::ceremony::client_id(&made.public_key));
    let signature = SshSig::from_pem(&made.proof).unwrap();
    let old_fp = PublicKey::from_openssh(&old_public).unwrap().fingerprint(HashAlg::Sha256).to_string();
    let new_fp = public.fingerprint(HashAlg::Sha256).to_string();
    let statement = farcooler_fence::rekey_statement(&old_fp, &new_fp).unwrap();
    public.verify(farcooler_fence::REKEY_NAMESPACE, statement.as_bytes(), &signature).expect("verifies");
    assert!(public.verify(CERTIFY_NAMESPACE, statement.as_bytes(), &signature).is_err());
    let other = PrivateKey::from_openssh(random_key()).unwrap().public_key().fingerprint(HashAlg::Sha256).to_string();
    let elsewhere = farcooler_fence::rekey_statement(&other, &new_fp).unwrap();
    assert!(public.verify(farcooler_fence::REKEY_NAMESPACE, elsewhere.as_bytes(), &signature).is_err());
}

/// Moving to the key already held moves nothing, and only Ed25519 keys sign.
#[test]
fn a_rekey_proof_refuses_the_same_key_and_other_kinds() {
    let key = random_key();
    let public = PrivateKey::from_openssh(&key).unwrap().public_key().to_openssh().unwrap();
    assert_eq!(rekey_proof(&key, &public), Err(DeviceKeyError::Key));
    assert_eq!(rekey_proof("not a key", &public), Err(DeviceKeyError::Key));
    assert_eq!(rekey_proof(&random_key(), "ssh-rsa AAAA"), Err(DeviceKeyError::Key));
}
