//! A browser's passkey, enrolled by the fence, logging in through a real sshd
//! (ov-765).
//!
//! A browser can't hold an ed25519 SSH key it may not export and also keep it
//! in a passkey; what it can do is sign with a WebAuthn passkey. OpenSSH takes
//! that as an `sk-ecdsa-sha2-nistp256@openssh.com` key whose application is the
//! passkey's RP ID, and verifies `webauthn-sk-ecdsa-sha2-nistp256@openssh.com`
//! signatures from it (PROTOCOL.u2f, OpenSSH 8.4 and newer). Whether the line
//! `fence::render` writes for one is a line sshd honors, forced command and
//! all, is a property of sshd, so this asks a real one, in the shape of
//! `a_real_sshd_forces_the_scope.rs` and through the same runner
//! (`real_sshd::start_with`).
//!
//! The passkey is made in software here: a P-256 key, and an assertion built
//! the way an authenticator and a browser build one. The authenticator signs
//! `authenticatorData ‖ SHA-256(clientDataJSON)`, where authenticatorData
//! starts with SHA-256 of the RP ID and clientDataJSON carries the SSH
//! session's signed data as its challenge. sshd rebuilds both from the key's
//! application and its own copy of that data, so a signature that verifies
//! here is one a browser's passkey would produce. The spike measured the same
//! wire format from Chromium's virtual authenticator (ov-761 §6).
//!
//! The sshd config is the runner's default: no `PubkeyAcceptedAlgorithms`
//! line, because a runner has none either.

use std::future::Future;
use std::sync::Arc;

use farcooler_protocol::v1::{result, Scope};
use farcooler_transport::{request, Client};
use p256::ecdsa::signature::Signer as _;
use p256::ecdsa::{Signature, SigningKey};
use russh::keys::agent::AgentIdentity;
use russh::keys::{HashAlg, PublicKey};
use sha2::{Digest as _, Sha256};

mod common;
mod real_sshd;

/// The canary app's host: the RP ID the passkey is made for.
const HOST: &str = "app.canary.farcooler.com";

/// The client id the line carries.
const DEVICE: &str = "farcooler-0123456789ab";

/// The authenticatorData flag for user presence, which sshd demands unless
/// the line says `no-touch-required`, and a Key A line never does.
const USER_PRESENT: u8 = 0x01;

/// A passkey logs in, and the session is the device and the scope its line
/// names.
#[tokio::test]
async fn a_passkey_logs_in_through_a_real_sshd() {
    let passkey = Passkey::new();
    let runner = real_sshd::start_with(Some(&passkey.public_line()), DEVICE, Scope::Control).await;

    let mut session = connect(&runner).await;
    let auth = session
        .authenticate_publickey_with(user(), passkey.public_key(), None, &mut passkey.signer(Tamper::None))
        .await
        .expect("the auth exchange");
    assert!(auth.success(), "sshd refused the passkey: {auth:?}");

    let channel = session.channel_open_session().await.expect("a session channel");
    // What the product's own client asks for. The forced command wins either way.
    channel.exec(true, "farcoolerd --stdio").await.expect("exec");
    let (read, write) = tokio::io::split(channel.into_stream());
    let mut client = Client::over(read, write, "test-client", "0.0.0").await.expect("handshake over a passkey session");
    assert_eq!(client.server_hello().granted_scope, Scope::Control as i32, "the line's scope did not arrive");

    let listed = client.call(request("client.list")).await.expect("a control session may list clients");
    let Some(result::Value::ClientList(list)) = listed.value else { panic!("wrong result") };
    assert!(
        list.items.iter().any(|c| c.client_id == DEVICE),
        "the fence sshd read does not hold the passkey's line: {:?}",
        list.items
    );
}

/// The same assertion with one bit of `s` flipped is refused: sshd verifies
/// the signature, and the login above is not sshd taking any sk key on trust.
#[tokio::test]
async fn a_passkey_signature_with_a_flipped_bit_is_refused() {
    assert_refused(Tamper::FlipS).await;
}

/// Without the user-presence flag, a valid signature is refused: the line
/// carries no `no-touch-required`, so every login needs a touch.
#[tokio::test]
async fn a_passkey_assertion_without_a_touch_is_refused() {
    assert_refused(Tamper::NoTouch).await;
}

async fn assert_refused(tamper: Tamper) {
    let passkey = Passkey::new();
    let runner = real_sshd::start_with(Some(&passkey.public_line()), DEVICE, Scope::Control).await;
    let mut session = connect(&runner).await;
    let auth = session
        .authenticate_publickey_with(user(), passkey.public_key(), None, &mut passkey.signer(tamper))
        .await
        .expect("the auth exchange");
    assert!(!auth.success(), "sshd took a {tamper:?} assertion: {auth:?}");
}

/// An SSH connection to the runner's sshd, its host key taken on trust: a
/// throwaway key on a throwaway port, as `a_real_sshd_forces_the_scope.rs`
/// takes it.
async fn connect(runner: &real_sshd::Runner) -> russh::client::Handle<AnyHost> {
    let config = Arc::new(russh::client::Config::default());
    russh::client::connect(config, ("127.0.0.1", runner.port), AnyHost).await.expect("an SSH connection to sshd")
}

struct AnyHost;

impl russh::client::Handler for AnyHost {
    type Error = russh::Error;

    async fn check_server_key(&mut self, _key: &PublicKey) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

/// This account's name, which is the only one a non-root sshd logs in.
fn user() -> String {
    // SAFETY: getpwuid's result is read before any other call could reuse it.
    unsafe {
        let entry = libc::getpwuid(libc::getuid());
        assert!(!entry.is_null(), "no passwd entry for this uid");
        std::ffi::CStr::from_ptr((*entry).pw_name).to_string_lossy().into_owned()
    }
}

/// A software passkey for `HOST`.
struct Passkey {
    key: SigningKey,
}

/// What to break in an assertion.
#[derive(Debug, Clone, Copy)]
enum Tamper {
    None,
    /// One bit of the signature's `s`.
    FlipS,
    /// The user-presence flag left off.
    NoTouch,
}

impl Passkey {
    /// A fixed scalar, so a failure reproduces.
    fn new() -> Self {
        let scalar: [u8; 32] = Sha256::digest(b"ov-765 a software passkey").into();
        Self { key: SigningKey::from_slice(&scalar).expect("a valid P-256 scalar") }
    }

    /// The key as OpenSSH writes it: `sk-ecdsa-sha2-nistp256@openssh.com`,
    /// the curve, the uncompressed point, and the RP ID as its application.
    fn public_line(&self) -> String {
        let point = self.key.verifying_key().to_sec1_point(false);
        let mut blob = Vec::new();
        put_string(&mut blob, b"sk-ecdsa-sha2-nistp256@openssh.com");
        put_string(&mut blob, b"nistp256");
        put_string(&mut blob, point.as_bytes());
        put_string(&mut blob, HOST.as_bytes());
        format!("sk-ecdsa-sha2-nistp256@openssh.com {}", base64(&blob, STANDARD, true))
    }

    fn public_key(&self) -> PublicKey {
        PublicKey::from_openssh(&self.public_line()).expect("the passkey's public key parses")
    }

    fn signer(&self, tamper: Tamper) -> Assertions {
        Assertions { key: self.key.clone(), tamper }
    }
}

/// russh's signer, answering with a WebAuthn assertion as PROTOCOL.u2f
/// encodes one, as a browser's page does.
struct Assertions {
    key: SigningKey,
    tamper: Tamper,
}

#[derive(Debug)]
struct Unsent;

impl From<russh::SendError> for Unsent {
    fn from(_: russh::SendError) -> Self {
        Unsent
    }
}

impl russh::Signer for Assertions {
    type Error = Unsent;

    fn auth_sign(
        &mut self,
        _key: &AgentIdentity,
        _hash_alg: Option<HashAlg>,
        to_sign: Vec<u8>,
    ) -> impl Future<Output = Result<Vec<u8>, Unsent>> + Send {
        let blob = self.assertion(&to_sign);
        async move {
            // What russh's own agent signer does: the data, then the
            // signature blob as one SSH string.
            let mut signed = to_sign;
            put_string(&mut signed, &blob);
            Ok(signed)
        }
    }
}

impl Assertions {
    /// `webauthn-sk-ecdsa-sha2-nistp256@openssh.com` over `data`, the SSH
    /// session's signed data, which a browser passes as the challenge.
    fn assertion(&self, data: &[u8]) -> Vec<u8> {
        let origin = format!("https://{HOST}");
        let client_data = format!(
            r#"{{"type":"webauthn.get","challenge":"{}","origin":"{origin}","crossOrigin":false}}"#,
            base64(data, URL_SAFE, false)
        );
        let flags = match self.tamper {
            Tamper::NoTouch => 0,
            _ => USER_PRESENT,
        };
        let counter: u32 = 1;

        let mut authenticator_data = Sha256::digest(HOST.as_bytes()).to_vec();
        authenticator_data.push(flags);
        authenticator_data.extend_from_slice(&counter.to_be_bytes());
        let mut signed = authenticator_data;
        signed.extend_from_slice(&Sha256::digest(client_data.as_bytes()));
        let signature: Signature = self.key.sign(&signed);
        let (r, mut s) = signature.split_bytes();
        if let Tamper::FlipS = self.tamper {
            s[31] ^= 1;
        }

        let mut ecdsa = Vec::new();
        put_mpint(&mut ecdsa, &r);
        put_mpint(&mut ecdsa, &s);
        let mut blob = Vec::new();
        put_string(&mut blob, b"webauthn-sk-ecdsa-sha2-nistp256@openssh.com");
        put_string(&mut blob, &ecdsa);
        blob.push(flags);
        blob.extend_from_slice(&counter.to_be_bytes());
        put_string(&mut blob, origin.as_bytes());
        put_string(&mut blob, client_data.as_bytes());
        // No extensions, so the ED flag is off and this is empty.
        put_string(&mut blob, b"");
        blob
    }
}

fn put_string(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
}

/// An SSH mpint of a positive big-endian number: no leading zero bytes, and
/// one zero byte back when the top bit is set.
fn put_mpint(out: &mut Vec<u8>, bytes: &[u8]) {
    let trimmed: &[u8] = &bytes[bytes.iter().take_while(|b| **b == 0).count()..];
    let mut value = Vec::new();
    if trimmed.first().is_some_and(|b| b & 0x80 != 0) {
        value.push(0);
    }
    value.extend_from_slice(trimmed);
    put_string(out, &value);
}

const STANDARD: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const URL_SAFE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Base64 in `alphabet`: the standard one, padded, for a key line, and the
/// URL-safe one, unpadded, for a WebAuthn challenge.
fn base64(bytes: &[u8], alphabet: &[u8; 64], pad: bool) -> String {
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..=chunk.len() {
            out.push(alphabet[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
        if pad {
            out.extend(std::iter::repeat_n('=', 3 - chunk.len()));
        }
    }
    out
}

#[test]
fn base64_matches_the_rfc_4648_vectors() {
    for (plain, padded, url) in [("", "", ""), ("f", "Zg==", "Zg"), ("fo", "Zm8=", "Zm8"), ("foo", "Zm9v", "Zm9v"), ("foobar", "Zm9vYmFy", "Zm9vYmFy")] {
        assert_eq!(base64(plain.as_bytes(), STANDARD, true), padded);
        assert_eq!(base64(plain.as_bytes(), URL_SAFE, false), url);
    }
    assert_eq!(base64(&[0xfb, 0xff], URL_SAFE, false), "-_8");
}
