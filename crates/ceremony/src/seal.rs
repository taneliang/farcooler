//! The reply, sealed for a device that cannot scan it (ov-769).
//!
//! A browser has no route to a runner and may have no camera, so the reply
//! that a phone would show as a QR code travels through the relay instead. The
//! relay must never read it: a reply is the fleet's topology, every runner's
//! address, host key and tunnel token, and the relay is the one party in the
//! ceremony that was never in the room.
//!
//! So the new device puts a fresh X25519 public key in its offer
//! ([`Offer::reply_key`]), the granting phone seals the manifest to it, and the
//! relay holds ciphertext it has no key for.
//!
//! ## The construction
//!
//! RFC 9180 HPKE in base mode, with DHKEM(X25519, HKDF-SHA256), HKDF-SHA256 and
//! ChaCha20-Poly1305, single-shot. Far Cooler implements none of it: this is
//! the `hpke` crate. Base mode, not auth mode, because the sender has no key
//! the new device could know in advance; the reply leg stays unauthenticated,
//! exactly as the module note on the QR leg says, and sealing changes who can
//! READ a reply, not who can WRITE one.
//!
//! ## What it does not stop
//!
//! The offer is public, so anyone who saw it holds everything a valid reply
//! needs: the reply key and every echo. A reply they seal OPENS, and passes
//! [`accept_manifest`], with their own machines and host keys in it. The QR
//! leg's residual risk is someone who "filmed the first code, is in the room";
//! the relay removes "in the room", leaving whoever it lets post (the account
//! and its teammates, `services/relay/src/ceremony-reply.ts`). What stops a
//! forged reply is the comparison code the browser shows (slice 8): drawn from
//! the HPKE exporter secret of this one seal, matched by the person against the
//! granting phone's screen, before the browser keeps anything. The exporter is
//! a secret only the sealer and the opener hold, so nobody who can predict the
//! manifest can predict the code.
//!
//! The associated data is the reply's four echoes, `ceremony ‖ account ‖
//! channel ‖ target`, each prefixed with its length as four big-endian bytes.
//! Plain concatenation would let `"acct_1" ‖ "canary"` and `"acct_1c" ‖
//! "anary"` be the same bytes. The opener computes them from the offer it
//! showed, never from the reply, so a reply sealed for another ceremony,
//! account, channel or key fails to open before [`accept_manifest`] judges it.
//!
//! ## On the wire
//!
//! Both the key and the sealed reply are unpadded base64url. A sealed reply is
//! the 32-byte encapsulated key, then the ciphertext and its 16-byte tag. A
//! manifest at [`BUDGET`] seals to under 2.5 KB of text, inside the relay's
//! 4 KB (`services/relay/src/ceremony-reply.ts`).
//!
//! [`accept_manifest`]: super::accept_manifest

use std::time::Duration;

use base64ct::{Base64UrlUnpadded, Encoding};
use hpke::kem::X25519HkdfSha256;
use hpke::{Deserializable, Kem as _, OpModeR, OpModeS, Serializable};

use super::{BUDGET, CeremonyError, Manifest, Offer, accept_manifest, encode_manifest, fingerprint, still_fresh};

type Kem = X25519HkdfSha256;
type Kdf = hpke::kdf::HkdfSha256;
type Aead = hpke::aead::ChaCha20Poly1305;

/// HPKE's `info`: which protocol and which use of it a key was derived for,
/// so a ciphertext from anything else that seals to an X25519 key never opens
/// here.
const INFO: &[u8] = b"farcooler ceremony reply v1";

/// An X25519 public key is 32 bytes, and so is the encapsulated key.
const KEY_BYTES: usize = 32;

/// The key a device asks for its reply to be sealed to.
///
/// Made for one ceremony and dropped with it. Its public half goes in the
/// offer; its secret half never leaves the device, and `Debug` does not print
/// it.
pub struct ReplyKey {
    secret: <Kem as hpke::Kem>::PrivateKey,
    public: String,
}

impl ReplyKey {
    /// A fresh key, from the same RNG the ceremony id comes from.
    pub fn generate() -> Self {
        let (secret, public) = Kem::gen_keypair_with_rng(&mut rand::rng());
        ReplyKey { public: Base64UrlUnpadded::encode_string(&public.to_bytes()), secret }
    }

    /// A key from its secret half, as [`ReplyKey::secret`] wrote it: for a
    /// browser that has to keep one across a reload while its code is up.
    pub fn from_secret(encoded: &str) -> Result<Self, CeremonyError> {
        let bytes = decode_exact(encoded, "reply secret")?;
        let secret = <Kem as hpke::Kem>::PrivateKey::from_bytes(&bytes)
            .map_err(|_| CeremonyError::Malformed("the reply secret is not a key".into()))?;
        let public = Base64UrlUnpadded::encode_string(&Kem::sk_to_pk(&secret).to_bytes());
        Ok(ReplyKey { secret, public })
    }

    /// The secret half, as unpadded base64url. Only for the device's own
    /// storage: anything that holds this can read the reply. Not zeroized once
    /// it's a `String`, so a browser keeps it in `sessionStorage` at most and
    /// drops it once the reply is taken or the ceremony abandoned.
    pub fn secret(&self) -> String {
        Base64UrlUnpadded::encode_string(&self.secret.to_bytes())
    }

    /// The public half, as [`Offer::reply_key`] carries it.
    pub fn public(&self) -> &str {
        &self.public
    }
}

impl std::fmt::Debug for ReplyKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReplyKey").field("public", &self.public).finish_non_exhaustive()
    }
}

/// Seal a reply to the key its offer named, for the relay to carry.
///
/// `offer` is the scanned offer and `manifest` the reply [`super::manifest`]
/// built from it. Taking both, rather than a bare key, means a reply sealed to
/// one offer's key with another's echoes can't be built: a manifest whose
/// ceremony, account, channel or target isn't the offer's is refused as
/// `Malformed`, and so is an offer that asked for no sealed reply. A manifest
/// over [`BUDGET`] is refused as `TooLarge`, as it is on a screen: the relay's
/// 4 KB is sized for one that fits.
///
/// Returns the sealed reply and its comparison code, drawn from this seal's
/// exporter secret: two seals of one manifest show two codes, and the opener
/// of this one, and only that opener, shows the same.
pub fn seal_reply(offer: &Offer, manifest: &Manifest) -> Result<(String, String), CeremonyError> {
    let Some(reply_key) = offer.reply_key.as_deref() else {
        return Err(CeremonyError::Malformed("this offer asked for no sealed reply".into()));
    };
    let target = fingerprint(&offer.key_a).unwrap_or_default();
    let echoes = (&manifest.ceremony, &manifest.account, &manifest.channel, &manifest.target);
    if echoes != (&offer.ceremony, &offer.account, &offer.channel, &target) {
        return Err(CeremonyError::Malformed("this reply answers a different offer".into()));
    }
    let plaintext = encode_manifest(manifest);
    if plaintext.len() > BUDGET {
        return Err(CeremonyError::TooLarge);
    }
    let bytes = decode_exact(reply_key, "reply key")?;
    let public = <Kem as hpke::Kem>::PublicKey::from_bytes(&bytes)
        .map_err(|_| CeremonyError::Malformed("the reply key is not a key".into()))?;
    let aad = associated(&offer.ceremony, &offer.account, &offer.channel, &target);
    // An all-zero or low-order key, which no honest device generates.
    let unsealable = |_| CeremonyError::Malformed("the reply key cannot be sealed to".into());
    let (encapped, mut context) =
        hpke::setup_sender_with_rng::<Aead, Kdf, Kem>(&OpModeS::Base, &public, INFO, &mut rand::rng())
            .map_err(unsealable)?;
    let ciphertext = context.seal(plaintext.as_bytes(), &aad).map_err(unsealable)?;
    let code = exported_code(|label, out| context.export(label, out))?;
    let mut sealed = encapped.to_bytes().to_vec();
    sealed.extend_from_slice(&ciphertext);
    Ok((Base64UrlUnpadded::encode_string(&sealed), code))
}

/// Open a sealed reply with this device's key, for the offer it showed.
///
/// Returns the manifest as its encoded text, for [`accept_manifest`] to judge
/// like one off a screen, and the comparison code from this opening's exporter
/// secret: the sealer's, for an honest reply. The associated data comes from `expecting`, never
/// from the reply, so a reply for anything but this ceremony, account,
/// channel and Key A fails here as `Unopened`, and so does one sealed to
/// another key or changed by a single byte.
///
/// `expecting` must name `key` as its reply key: an offer that asked for no
/// sealed reply, or for one to another key, takes none.
pub fn open_reply(sealed: &str, key: &ReplyKey, expecting: &Offer) -> Result<(String, String), CeremonyError> {
    if expecting.reply_key.as_deref() != Some(key.public()) {
        return Err(CeremonyError::Malformed("this offer asked for no reply sealed to this key".into()));
    }
    let bytes = Base64UrlUnpadded::decode_vec(sealed).map_err(|_| CeremonyError::Unopened)?;
    if bytes.len() < KEY_BYTES {
        return Err(CeremonyError::Unopened);
    }
    let (encapped, ciphertext) = bytes.split_at(KEY_BYTES);
    let encapped = <Kem as hpke::Kem>::EncappedKey::from_bytes(encapped).map_err(|_| CeremonyError::Unopened)?;
    let target = fingerprint(&expecting.key_a)
        .ok_or_else(|| CeremonyError::Malformed("this device's own key will not parse".into()))?;
    let aad = associated(&expecting.ceremony, &expecting.account, &expecting.channel, &target);
    let mut context = hpke::setup_receiver::<Aead, Kdf, Kem>(&OpModeR::Base, &key.secret, &encapped, INFO)
        .map_err(|_| CeremonyError::Unopened)?;
    let plaintext = context.open(ciphertext, &aad).map_err(|_| CeremonyError::Unopened)?;
    let code = exported_code(|label, out| context.export(label, out))?;
    let text = String::from_utf8(plaintext).map_err(|_| CeremonyError::Malformed("the reply is not text".into()))?;
    Ok((text, code))
}

/// Take a sealed reply on the new device's side: [`super::take_reply`]'s
/// order, with opening it between the freshness window and the reply's own
/// rules. Returns the manifest and the comparison code to show beside it.
///
/// One function for the same reason `take_reply` is one: a platform that
/// spelled the order out could forget a step.
pub fn take_sealed_reply(
    sealed: &str,
    key: &ReplyKey,
    expecting: &Offer,
    already_taken: bool,
    held_for: Duration,
) -> Result<(Manifest, String), CeremonyError> {
    if already_taken {
        return Err(CeremonyError::AlreadyTaken);
    }
    still_fresh(held_for)?;
    let (encoded, code) = open_reply(sealed, key, expecting)?;
    Ok((accept_manifest(&encoded, expecting, already_taken)?, code))
}

/// Refuse an offer whose reply key is there and is not a key (ov-770).
///
/// [`decode_offer`] calls this, so every scan of such an offer is refused as
/// `Malformed` before anything is enrolled, rather than a phone getting as far
/// as granting runners and only then finding it has nothing to seal to. An
/// offer with no reply key passes untouched: it is the QR-only ceremony.
///
/// [`decode_offer`]: super::decode_offer
pub fn check_reply_key(offer: &Offer) -> Result<(), CeremonyError> {
    let Some(reply_key) = offer.reply_key.as_deref() else { return Ok(()) };
    let bytes = decode_exact(reply_key, "reply key")?;
    <Kem as hpke::Kem>::PublicKey::from_bytes(&bytes)
        .map(|_| ())
        .map_err(|_| CeremonyError::Malformed("the reply key is not a key".into()))
}

/// What a comparison code is drawn from: no other code's alphabet.
///
/// Thirty-two symbols, so each is five bits, with 0, O, 1 and I left out
/// because a person reads them off one screen and finds them on another.
const CODE_ALPHABET: &[u8; 32] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";

/// How many symbols a comparison code has: 30 bits.
pub const CODE_LENGTH: usize = 6;

/// The HPKE exporter label a comparison code is drawn with, so no other
/// export of the same context is ever the same six characters.
const CODE_LABEL: &[u8] = b"farcooler ceremony code v1";

/// The six characters the granting phone and the browser both show for one
/// reply, for a person to match before the browser keeps anything (ov-770).
///
/// They come from the HPKE exporter secret (RFC 9180 section 5.3) of the
/// seal's own context, never from the manifest. The exporter is derived from
/// the fresh encapsulated key and the recipient's key, so only the sealer and
/// the holder of the reply key can compute it. The relay sees the encapsulated
/// key and cannot. A teammate who can predict the manifest to the byte still
/// cannot predict the code, and a reply sealed to any other key shows another
/// one. An attacker who seals a forgery knows their own exporter and so their
/// own code: each try matches the honest one with probability 2^-30.
///
/// Thirty bits is enough only because of that, and because the phone shows
/// the code once the honest reply is already posted.
fn exported_code(
    export: impl FnOnce(&[u8], &mut [u8]) -> Result<(), hpke::HpkeError>,
) -> Result<String, CeremonyError> {
    let mut exporter = [0u8; 32];
    export(CODE_LABEL, &mut exporter).map_err(|_| CeremonyError::Malformed("no code can be drawn".into()))?;
    let bits = u32::from_be_bytes([exporter[0], exporter[1], exporter[2], exporter[3]]);
    Ok((0..CODE_LENGTH)
        .map(|i| CODE_ALPHABET[((bits >> (27 - 5 * i)) & 0x1f) as usize] as char)
        .collect())
}

/// The associated data: each echo, prefixed with its length.
fn associated(ceremony: &str, account: &str, channel: &str, target: &str) -> Vec<u8> {
    let mut aad = Vec::new();
    for field in [ceremony, account, channel, target] {
        aad.extend_from_slice(&(field.len() as u32).to_be_bytes());
        aad.extend_from_slice(field.as_bytes());
    }
    aad
}

/// Exactly one 32-byte key's worth of unpadded base64url.
fn decode_exact(encoded: &str, what: &str) -> Result<Vec<u8>, CeremonyError> {
    match Base64UrlUnpadded::decode_vec(encoded) {
        Ok(bytes) if bytes.len() == KEY_BYTES => Ok(bytes),
        _ => Err(CeremonyError::Malformed(format!("the {what} is not 32 bytes of base64url"))),
    }
}

#[cfg(test)]
#[path = "seal_tests.rs"]
mod tests;
