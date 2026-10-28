//! The passkey's signature, read against what a real browser made (ov-767).
//!
//! `services/relay/test/fixtures/passkey-proof.json` holds two device
//! registration proofs signed by Chromium's WebAuthn stack with its virtual
//! authenticator (`services/relay/scripts/passkey-fixture.mjs`, ov-766): one
//! passkey for `app.canary.farcooler.com` and one scoped to the parent,
//! `farcooler.com`. Each proof is an SSHSIG whose signature is the same
//! `webauthn-sk-ecdsa-sha2-nistp256@openssh.com` blob an SSH login carries.
//!
//! An ECDSA signature is random, so this can't make the fixture's bytes from
//! scratch. Instead it takes the fixture apart into what the browser handed
//! the page (authenticatorData, clientDataJSON and the DER signature) and asks
//! `passkey::signature_blob_for` to put it back together: it must give the
//! fixture's bytes exactly, which are bytes the relay verifies. And the key
//! line made from the passkey's public key must be the fixture's line.
//!
//! The way `crates/client/src/device_keys_tests.rs` reads `device-proof.json`.

use base64ct::{Base64, Encoding as _};
use farcooler_web::passkey::{self, Assertion};
use serde_json::Value;
use sha2::{Digest as _, Sha256, Sha512};

const FIXTURE: &str = include_str!("../../../services/relay/test/fixtures/passkey-proof.json");

fn fixture() -> Value {
    serde_json::from_str(FIXTURE).unwrap()
}

/// Reads SSH wire values off the front of a byte slice.
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> &'a [u8] {
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        head
    }
    fn u32(&mut self) -> u32 {
        u32::from_be_bytes(self.take(4).try_into().unwrap())
    }
    fn string(&mut self) -> &'a [u8] {
        let n = self.u32() as usize;
        self.take(n)
    }
}

fn put_string(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
}

/// One fixture entry, taken apart.
struct Proof {
    rp_id: String,
    /// The bytes the passkey signed over: SSHSIG's signed data.
    data: Vec<u8>,
    /// The signature blob inside the SSHSIG, as the relay verified it.
    signature: Vec<u8>,
    /// What `navigator.credentials.get` answered, rebuilt from that blob.
    assertion: Assertion,
    /// The passkey's uncompressed point, from the fixture's key line.
    point: Vec<u8>,
    public_key: String,
}

fn proof(entry: &Value) -> Proof {
    let armored = entry["proof"].as_str().unwrap();
    let body: String = armored.lines().filter(|l| !l.starts_with("-----")).collect();
    let blob = Base64::decode_vec(&body).unwrap();
    let mut sshsig = Reader(&blob);
    assert_eq!(sshsig.take(6), b"SSHSIG");
    assert_eq!(sshsig.u32(), 1);
    let _key = sshsig.string();
    let namespace = sshsig.string().to_vec();
    let reserved = sshsig.string().to_vec();
    let hash = sshsig.string().to_vec();
    let signature = sshsig.string().to_vec();
    assert!(sshsig.0.is_empty());
    assert_eq!(hash, b"sha512");

    let mut data = b"SSHSIG".to_vec();
    put_string(&mut data, &namespace);
    put_string(&mut data, &reserved);
    put_string(&mut data, &hash);
    put_string(&mut data, &Sha512::digest(entry["statement"].as_str().unwrap().as_bytes()));

    let mut sig = Reader(&signature);
    assert_eq!(sig.string(), passkey::SIGNATURE_ALGORITHM.as_bytes());
    let mut ecdsa = Reader(sig.string());
    let (r, s) = (ecdsa.string().to_vec(), ecdsa.string().to_vec());
    let flags = sig.take(1)[0];
    let counter = sig.take(4).to_vec();
    let _origin = sig.string();
    let client_data_json = sig.string().to_vec();
    let extensions = sig.string().to_vec();
    assert!(sig.0.is_empty());

    let rp_id = entry["rpId"].as_str().unwrap().to_string();
    let mut authenticator_data = Sha256::digest(rp_id.as_bytes()).to_vec();
    authenticator_data.push(flags);
    authenticator_data.extend_from_slice(&counter);
    authenticator_data.extend_from_slice(&extensions);
    // An mpint is a DER INTEGER's content already: minimal, sign byte and all.
    let mut der = Vec::new();
    for int in [&r, &s] {
        der.extend_from_slice(&[0x02, int.len() as u8]);
        der.extend_from_slice(int);
    }
    der.splice(0..0, [0x30, der.len() as u8]);

    let public_key = entry["publicKey"].as_str().unwrap().to_string();
    let key_blob = Base64::decode_vec(public_key.split(' ').nth(1).unwrap()).unwrap();
    let mut key = Reader(&key_blob);
    let _ = (key.string(), key.string());
    let point = key.string().to_vec();

    Proof {
        rp_id,
        data,
        signature,
        assertion: Assertion { authenticator_data, client_data_json, signature: der },
        point,
        public_key,
    }
}

/// The P-256 SPKI `getPublicKey()` answers, around `point`.
fn spki(point: &[u8]) -> Vec<u8> {
    let mut out = vec![
        0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce,
        0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
    ];
    out.extend_from_slice(point);
    out
}

/// What Chromium handed the page, put back together by the page's encoder,
/// is the signature the relay verified, byte for byte.
#[test]
fn a_real_browsers_assertion_encodes_to_the_fixtures_signature() {
    let f = fixture();
    let p = proof(&f["exact"]);
    assert_eq!(p.rp_id, "app.canary.farcooler.com");
    let made = passkey::signature_blob_for(&p.rp_id, &p.data, &p.assertion).expect("the browser's own assertion");
    assert_eq!(made, p.signature);
}

/// The passkey's public key becomes the fixture's line: the key the relay
/// registered and the fence writes.
#[test]
fn a_real_passkeys_public_key_becomes_the_fixtures_line() {
    let f = fixture();
    let p = proof(&f["exact"]);
    let line = passkey::public_line_for(&p.rp_id, &spki(&p.point), "farcooler-web").unwrap();
    assert_eq!(line, format!("{} farcooler-web", p.public_key));
}

/// The fixture's other passkey is scoped to `farcooler.com`, which a browser
/// lets app.canary.farcooler.com use. This build's encoder will not send it.
#[test]
fn a_passkey_for_the_parent_domain_is_never_sent() {
    let f = fixture();
    let p = proof(&f["parent"]);
    assert_eq!(p.rp_id, "farcooler.com");
    let refused = passkey::signature_blob_for("app.canary.farcooler.com", &p.data, &p.assertion).unwrap_err();
    assert!(refused.contains("not for app.canary.farcooler.com"), "{refused}");
}

/// ov-771: the directory registration this browser builds is the fixture's
/// register statement and its armored proof, byte for byte: the relay verified
/// those bytes, so a proof built the same way registers.
#[test]
fn a_real_passkeys_registration_is_the_fixtures_statement_and_proof() {
    use farcooler_web::register;

    let f = fixture();
    let (account, challenge) = (f["account"].as_str().unwrap(), f["challenge"].as_str().unwrap());
    let entry = &f["exact"];
    let p = proof(entry);
    let statement = register::statement(account, challenge, &p.public_key).unwrap();
    assert_eq!(statement, entry["statement"].as_str().unwrap());
    // What the passkey is asked to sign is the data the browser really signed.
    assert_eq!(register::signed_data(account, challenge, &p.public_key).unwrap(), p.data);
    let armored = register::proof(account, challenge, &p.public_key, &p.assertion).unwrap();
    assert_eq!(armored, entry["proof"].as_str().unwrap());
}

/// A challenge, account or assertion that isn't exactly right builds nothing:
/// the relay can't slip a line into the statement, and an assertion over some
/// other data, or for the parent domain, makes no proof.
#[test]
fn a_registration_refuses_a_challenge_an_account_or_an_assertion_that_is_not_its_own() {
    use farcooler_web::register::{self, RegisterError};

    let f = fixture();
    let (account, challenge) = (f["account"].as_str().unwrap(), f["challenge"].as_str().unwrap());
    let p = proof(&f["exact"]);
    let key = &p.public_key;
    let injected = format!("{challenge}\nkey-b SHA256:x");
    assert_eq!(register::statement(account, &injected, key), Err(RegisterError::Challenge));
    assert_eq!(register::statement(account, &challenge.to_uppercase(), key), Err(RegisterError::Challenge));
    assert_eq!(register::statement("user_1\nchallenge x", challenge, key), Err(RegisterError::Account));
    assert_eq!(register::statement("", challenge, key), Err(RegisterError::Account));
    assert_eq!(register::statement(account, challenge, "not a key"), Err(RegisterError::Key));
    // A different challenge changes the data, so the old assertion doesn't fit.
    let other = "0".repeat(64);
    assert_eq!(register::proof(account, &other, key, &p.assertion), Err(RegisterError::Assertion));
    // The parent domain's assertion is for another host.
    let parent = proof(&f["parent"]);
    assert_eq!(register::proof(account, challenge, key, &parent.assertion), Err(RegisterError::Assertion));
}
