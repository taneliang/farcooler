//! The browser's SSH key is a passkey (ov-767, plan-763 slice 4; owner R-88).
//!
//! OpenSSH takes a WebAuthn passkey as an `sk-ecdsa-sha2-nistp256@openssh.com`
//! key whose application string is the passkey's RP ID, and verifies the
//! `webauthn-sk-ecdsa-sha2-nistp256@openssh.com` signatures a browser makes
//! with it (PROTOCOL.u2f, OpenSSH 8.4 and newer). The page makes the passkey
//! and asks it to sign (`navigator.credentials`); this module turns what the
//! browser hands back into the key line and the signature sshd reads.
//!
//! **The RP ID is this build's app host, never the page's location**
//! ([`rp_id`]). A canary build signs for `app.canary.farcooler.com` and a
//! stable one for `app.farcooler.com`, the one host the fence admits a passkey
//! line for on that channel (`farcooler_fence::app_host_for`, the same
//! `Channel::app_host`). Never the parent `farcooler.com`: a passkey scoped
//! there could be asked to sign by any page on any `*.farcooler.com` host.
//!
//! **Nothing else may ever be served under the app host.** A browser lets a
//! page on any host beneath the RP ID (`*.app.canary.farcooler.com`) use this
//! passkey, and a `/.well-known/webauthn` related-origins file on the app host
//! would extend it to whatever origins that file names. sshd doesn't check the
//! origin, only the RP ID's hash, so either would let another page sign in to
//! every runner this browser can reach. Today nothing serves either: the
//! Worker routes the exact host only, and a test holds `/.well-known/webauthn`
//! to a 404 (`services/web/test/headers.test.ts`). Never add a subdomain under
//! an app host, or that file.
//!
//! **What an assertion must say before it is sent** ([`signature_blob`]). sshd
//! checks the signature against its own copy of the signed data and the
//! line's application, so a wrong assertion would only fail there. These are
//! checked here as well so this browser never offers a runner a signature that
//! isn't plainly its own: for this build's RP ID, with the person's touch
//! (user presence, R-89; user verification is not asked for), over exactly the
//! data SSH asked to have signed, from this build's own origin and not from a
//! page framing it.

use std::future::Future;
use std::pin::Pin;

use base64ct::{Base64UrlUnpadded, Encoding as _};
use russh::keys::agent::AgentIdentity;
use russh::keys::{HashAlg, PublicKey, ssh_key};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};

use crate::error::WebError;
use crate::ssh::SignError;

/// The algorithm of every signature a passkey makes for SSH.
pub const SIGNATURE_ALGORITHM: &str = "webauthn-sk-ecdsa-sha2-nistp256@openssh.com";

/// The authenticatorData flag for user presence: the person touched it.
const USER_PRESENT: u8 = 0x01;

/// This build's RP ID: the host its channel's web client is served from.
pub fn rp_id() -> &'static str {
    farcooler_protocol::CHANNEL.app_host()
}

/// The fixed DER prefix of a P-256 public key's SubjectPublicKeyInfo:
/// id-ecPublicKey, prime256v1, and a 66-byte BIT STRING with no unused bits.
/// What `AuthenticatorAttestationResponse.getPublicKey()` answers for ES256.
const P256_SPKI_PREFIX: [u8; 26] = [
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce,
    0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
];

/// A new passkey's public key (its SPKI, as the browser gives it) as the
/// OpenSSH line an enrollment offer carries in `key_a`.
///
/// Only an uncompressed P-256 point in exactly that SPKI: ES256 is the only
/// algorithm the page asks for and the only one sshd's sk keys have.
pub fn public_line(spki: &[u8], comment: &str) -> Result<String, WebError> {
    public_line_for(rp_id(), spki, comment)
}

/// [`public_line`] as a build for `rp_id` would answer, so a test can read
/// the relay's fixture whichever channel it is built for.
pub fn public_line_for(rp_id: &str, spki: &[u8], comment: &str) -> Result<String, WebError> {
    let point = spki
        .strip_prefix(&P256_SPKI_PREFIX[..])
        .filter(|point| point.len() == 65 && point[0] == 0x04)
        .ok_or_else(|| WebError::Signer("the passkey's public key is not an uncompressed P-256 point".into()))?;
    let point = ssh_key::public::EcdsaPublicKey::from_sec1_bytes(point).map_err(|e| WebError::Signer(e.to_string()))?;
    let ssh_key::public::EcdsaPublicKey::NistP256(point) = point else {
        return Err(WebError::Signer("the passkey's public key is not on P-256".into()));
    };
    let sk = ssh_key::public::SkEcdsaSha2NistP256::new(point, rp_id);
    let key = PublicKey::new(ssh_key::public::KeyData::SkEcdsaSha2NistP256(sk), comment);
    key.to_openssh().map_err(|e| WebError::Signer(e.to_string()))
}

/// What `navigator.credentials.get` answers, as raw bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assertion {
    pub authenticator_data: Vec<u8>,
    pub client_data_json: Vec<u8>,
    /// ES256's DER `Ecdsa-Sig-Value`.
    pub signature: Vec<u8>,
}

/// The fields of clientDataJSON this checks. Anything else is ignored, as the
/// spec says a verifier must.
#[derive(Deserialize)]
struct ClientData {
    #[serde(rename = "type")]
    kind: String,
    challenge: String,
    origin: String,
    #[serde(rename = "crossOrigin", default)]
    cross_origin: bool,
    #[serde(rename = "topOrigin")]
    top_origin: Option<String>,
}

/// `assertion`, made over `data`, as the SSH signature
/// `webauthn-sk-ecdsa-sha2-nistp256@openssh.com` (PROTOCOL.u2f):
///
/// ```text
/// string  "webauthn-sk-ecdsa-sha2-nistp256@openssh.com"
/// string  mpint r ‖ mpint s
/// byte    flags
/// uint32  counter
/// string  origin
/// string  clientDataJSON
/// string  extensions
/// ```
///
/// Refused, with why, unless the assertion is for [`rp_id`] with the person's
/// touch, over exactly `data`, from `https://` + [`rp_id`] in a top-level page.
pub fn signature_blob(data: &[u8], assertion: &Assertion) -> Result<Vec<u8>, String> {
    signature_blob_for(rp_id(), data, assertion)
}

/// [`signature_blob`] as a build for `rp_id` would answer.
pub fn signature_blob_for(rp_id: &str, data: &[u8], assertion: &Assertion) -> Result<Vec<u8>, String> {
    let auth = &assertion.authenticator_data;
    if auth.len() < 37 {
        return Err(format!("the authenticator data is {} bytes, shorter than 37", auth.len()));
    }
    if auth[..32] != Sha256::digest(rp_id.as_bytes())[..] {
        return Err(format!("the assertion is not for {rp_id}"));
    }
    let flags = auth[32];
    if flags & USER_PRESENT == 0 {
        return Err("the assertion has no touch (user presence)".into());
    }

    let client: ClientData = serde_json::from_slice(&assertion.client_data_json)
        .map_err(|e| format!("the client data is not JSON: {e}"))?;
    if client.kind != "webauthn.get" {
        return Err(format!("the client data is a {}, not a webauthn.get", client.kind));
    }
    if client.challenge != Base64UrlUnpadded::encode_string(data) {
        return Err("the assertion signed something other than this login".into());
    }
    let origin = format!("https://{rp_id}");
    if client.origin != origin {
        return Err(format!("the assertion came from {}, not {origin}", client.origin));
    }
    if client.cross_origin || client.top_origin.is_some() {
        return Err("the assertion came from a page inside a frame".into());
    }

    let (r, s) = der_signature(&assertion.signature)?;
    let mut inner = Vec::new();
    mpint(&mut inner, r);
    mpint(&mut inner, s);

    let mut out = Vec::new();
    string(&mut out, SIGNATURE_ALGORITHM.as_bytes());
    string(&mut out, &inner);
    out.push(flags);
    out.extend_from_slice(&auth[33..37]);
    string(&mut out, client.origin.as_bytes());
    string(&mut out, &assertion.client_data_json);
    string(&mut out, &auth[37..]);
    Ok(out)
}

fn string(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
}

/// A non-negative integer as an SSH mpint: no leading zero bytes, and one zero
/// byte in front when the top bit is set.
fn mpint(out: &mut Vec<u8>, mut value: &[u8]) {
    while let [0, rest @ ..] = value {
        value = rest;
    }
    let pad = value.first().is_some_and(|b| b & 0x80 != 0);
    out.extend_from_slice(&((value.len() + pad as usize) as u32).to_be_bytes());
    if pad {
        out.push(0);
    }
    out.extend_from_slice(value);
}

/// r and s out of a DER `SEQUENCE { INTEGER r, INTEGER s }`, short-form
/// lengths only (a P-256 signature is at most 72 bytes), with nothing after.
fn der_signature(der: &[u8]) -> Result<(&[u8], &[u8]), String> {
    let malformed = || "the passkey's signature is not a DER ECDSA signature".to_string();
    let [0x30, len, body @ ..] = der else { return Err(malformed()) };
    if *len as usize != body.len() || *len >= 0x80 {
        return Err(malformed());
    }
    fn integer(at: &[u8]) -> Option<(&[u8], usize)> {
        let [0x02, len, rest @ ..] = at else { return None };
        let len = *len as usize;
        // At most 33 bytes: 32 and a sign byte.
        if len == 0 || len > 33 || rest.len() < len {
            return None;
        }
        Some((&rest[..len], 2 + len))
    }
    let (r, used) = integer(body).ok_or_else(malformed)?;
    let (s, rest) = integer(&body[used..]).ok_or_else(malformed)?;
    if used + rest != body.len() {
        return Err(malformed());
    }
    Ok((r, s))
}

/// Asks the passkey to sign, and answers what the browser handed back.
///
/// In a browser that is `navigator.credentials.get` with the SSH data as its
/// challenge; in the tests a P-256 key in memory. The future must be `Send`
/// because russh's signer is; a browser's is not, and `js.rs` says why it may
/// be marked so there.
pub trait Authenticate {
    fn assert(&mut self, challenge: Vec<u8>) -> Pin<Box<dyn Future<Output = Result<Assertion, SignError>> + Send>>;
}

/// An [`Authenticate`] presented to russh as the passkey's signer.
pub struct PasskeySigner<A>(pub A);

impl<A: Authenticate> russh::Signer for PasskeySigner<A> {
    type Error = SignError;

    fn auth_sign(
        &mut self,
        key: &AgentIdentity,
        _hash_alg: Option<HashAlg>,
        to_sign: Vec<u8>,
    ) -> impl Future<Output = Result<Vec<u8>, SignError>> + Send {
        // Only this build's passkey: a key for another host is one this page
        // must never ask an authenticator about.
        let ours = key.public_key().key_data().sk_ecdsa_p256().is_some_and(|sk| sk.application() == rp_id());
        let asking = ours.then(|| self.0.assert(to_sign.clone()));
        async move {
            let Some(asking) = asking else {
                return Err(SignError::Sign(format!("only a passkey for {} signs here", rp_id())));
            };
            let assertion = asking.await?;
            let blob = signature_blob(&to_sign, &assertion).map_err(SignError::Sign)?;
            let mut signed = to_sign;
            string(&mut signed, &blob);
            Ok(signed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_mpint_drops_leading_zeros_and_pads_a_set_top_bit() {
        let mut out = Vec::new();
        mpint(&mut out, &[0, 0, 0x7f, 1]);
        assert_eq!(out, [0, 0, 0, 2, 0x7f, 1]);
        out.clear();
        mpint(&mut out, &[0x80, 1]);
        assert_eq!(out, [0, 0, 0, 3, 0, 0x80, 1]);
        out.clear();
        mpint(&mut out, &[0, 0x80]);
        assert_eq!(out, [0, 0, 0, 2, 0, 0x80]);
    }

    #[test]
    fn a_der_signature_is_read_strictly() {
        let good = [0x30, 6, 2, 1, 7, 2, 1, 9];
        assert_eq!(der_signature(&good), Ok((&[7u8][..], &[9u8][..])));
        // A length that disagrees with the bytes, trailing bytes, an empty
        // integer, a 34-byte integer, and something that isn't a sequence.
        assert!(der_signature(&[0x30, 7, 2, 1, 7, 2, 1, 9]).is_err());
        assert!(der_signature(&[0x30, 7, 2, 1, 7, 2, 1, 9, 0]).is_err());
        assert!(der_signature(&[0x30, 5, 2, 0, 2, 1, 9]).is_err());
        let mut long = vec![0x30, 39, 2, 34];
        long.extend([1; 34]);
        long.extend([2, 1, 9]);
        assert!(der_signature(&long).is_err());
        assert!(der_signature(&[0x31, 6, 2, 1, 7, 2, 1, 9]).is_err());
    }
}
