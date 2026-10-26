//! A passkey in software, answering as `navigator.credentials.get` does: a
//! P-256 key that signs `authenticatorData ‖ SHA-256(clientDataJSON)`, where
//! authenticatorData starts with SHA-256 of the RP ID and clientDataJSON
//! carries the SSH data as its challenge. What it hands back is what the page
//! hands crates/web: the three byte strings, the signature in DER.
//!
//! Each [`Tamper`] is one way an assertion can be wrong, for the tests that
//! the encoder refuses it and that sshd does.
#![allow(dead_code)]

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use base64ct::{Base64UrlUnpadded, Encoding as _};
use farcooler_web::passkey::{Assertion, Authenticate};
use farcooler_web::ssh::SignError;
use p256::ecdsa::signature::Signer as _;
use p256::ecdsa::{Signature, SigningKey};
use sha2::{Digest as _, Sha256};

/// What to break in an assertion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tamper {
    None,
    /// The user-presence flag left off: nobody touched it.
    NoTouch,
    /// Made by a passkey for `farcooler.com`, the parent domain.
    ParentRpId,
    /// Signed over data other than what SSH asked for.
    OtherChallenge,
    /// From a page on another origin.
    OtherOrigin,
    /// From a page inside a frame, as Chromium marks one.
    CrossOrigin,
    /// From a page inside a frame, as the newer spec marks one.
    TopOrigin,
    /// A `webauthn.create` client data, not a `webauthn.get`.
    Create,
    /// authenticatorData cut short.
    Short,
    /// The DER signature with a byte too many.
    BadDer,
    /// One bit of `s` flipped: well formed, and false.
    FlipS,
}

pub struct SoftPasskey {
    key: SigningKey,
    pub tamper: Tamper,
    /// How many times it was asked to sign: one touch each.
    pub touches: Arc<AtomicUsize>,
}

impl SoftPasskey {
    /// A fixed scalar, so a failure reproduces.
    pub fn new() -> Self {
        let scalar: [u8; 32] = Sha256::digest(b"ov-767 a software passkey").into();
        Self {
            key: SigningKey::from_slice(&scalar).expect("a valid P-256 scalar"),
            tamper: Tamper::None,
            touches: Arc::default(),
        }
    }

    pub fn with(tamper: Tamper) -> Self {
        Self { tamper, ..Self::new() }
    }

    /// What `getPublicKey()` answers: the point in a P-256 SPKI.
    pub fn spki(&self) -> Vec<u8> {
        let mut out = vec![
            0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a, 0x86, 0x48,
            0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
        ];
        out.extend_from_slice(self.key.verifying_key().to_sec1_point(false).as_bytes());
        out
    }

    /// The assertion a browser on `https://<rp_id>` would hand back for
    /// `data`, broken as `self.tamper` says.
    pub fn assertion(&self, rp_id: &str, data: &[u8]) -> Assertion {
        let t = self.tamper;
        let challenge = match t {
            Tamper::OtherChallenge => Base64UrlUnpadded::encode_string(b"something else"),
            _ => Base64UrlUnpadded::encode_string(data),
        };
        let origin = match t {
            Tamper::OtherOrigin => "https://canary.farcooler.com".to_string(),
            _ => format!("https://{rp_id}"),
        };
        let kind = if t == Tamper::Create { "webauthn.create" } else { "webauthn.get" };
        let framed = match t {
            Tamper::CrossOrigin => r#","crossOrigin":true"#.to_string(),
            Tamper::TopOrigin => r#","crossOrigin":false,"topOrigin":"https://evil.example""#.to_string(),
            _ => r#","crossOrigin":false"#.to_string(),
        };
        let client_data_json =
            format!(r#"{{"type":"{kind}","challenge":"{challenge}","origin":"{origin}"{framed}}}"#).into_bytes();

        let signed_for = if t == Tamper::ParentRpId { "farcooler.com" } else { rp_id };
        let mut authenticator_data = Sha256::digest(signed_for.as_bytes()).to_vec();
        authenticator_data.push(if t == Tamper::NoTouch { 0x00 } else { 0x01 });
        authenticator_data.extend_from_slice(&7u32.to_be_bytes());

        let mut message = authenticator_data.clone();
        message.extend_from_slice(&Sha256::digest(&client_data_json));
        let signature: Signature = self.key.sign(&message);
        let signature = if t == Tamper::FlipS {
            let (r, mut s) = signature.split_bytes();
            s[31] ^= 1;
            Signature::from_scalars(r, s).expect("still a valid scalar")
        } else {
            signature
        };
        let mut der = signature.to_der().as_bytes().to_vec();
        if t == Tamper::BadDer {
            der.push(0);
        }
        if t == Tamper::Short {
            authenticator_data.truncate(36);
        }
        Assertion { authenticator_data, client_data_json, signature: der }
    }

    /// As crates/web's russh signer sees it: for this build's RP ID.
    pub fn authenticator(&self) -> Soft {
        Soft { key: SoftPasskey { key: self.key.clone(), tamper: self.tamper, touches: self.touches.clone() } }
    }
}

/// [`SoftPasskey`] as the `Authenticate` the page's would be.
pub struct Soft {
    key: SoftPasskey,
}

impl Authenticate for Soft {
    fn assert(&mut self, challenge: Vec<u8>) -> Pin<Box<dyn Future<Output = Result<Assertion, SignError>> + Send>> {
        self.key.touches.fetch_add(1, Ordering::SeqCst);
        let assertion = self.key.assertion(farcooler_web::passkey::rp_id(), &challenge);
        Box::pin(async move { Ok(assertion) })
    }
}
