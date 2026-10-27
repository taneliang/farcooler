use super::*;
use crate::{Reach, RunnerEntry, decode_offer, encode_offer, manifest, offer};

/// Real keys, because the target is a fingerprint and a fingerprint of
/// something that will not parse is nothing at all. The same two the
/// crate's own tests use.
const KEY_A: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIB1iLbeqDzK4CDeUC3t+ffVPDI9Gk+sBwIZqJZW1NfS5 device-a";
const KEY_B: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDMdwe233CUbxjpEHkissIUGdCxhkTsDE/Zg7f+LB6S+ device-b";

/// The host key the relay's fixture reply carries, and which the relay's
/// test looks for in D1 and in the logs. Distinctive, so finding it anywhere
/// means the plaintext got there.
const FIXTURE_HOST_KEY: &str = "SHA256:ov769FixtureHostKeyMustNeverReachTheRelay00";

/// The relay's copy of a real sealed reply: `ceremony-reply.json`.
const FIXTURE: &str = include_str!("../../../services/relay/test/fixtures/ceremony-reply.json");

fn runner(host_key: &str) -> RunnerEntry {
    RunnerEntry {
        id: "0198f0c3-0000-7000-8000-00000000000a".into(),
        label: "box".into(),
        alias: "box".into(),
        user: "you".into(),
        host_key: host_key.into(),
        reach: Reach::Direct { host: "box.tail-1234.ts.net".into(), port: 22 },
        alternative: None,
        pending: false,
    }
}

/// A browser's offer, asking for its reply sealed to `key`.
fn asking(key: &ReplyKey) -> Offer {
    Offer { reply_key: Some(key.public().to_string()), ..offer("Chrome", "acct_1", KEY_A, None) }
}

fn sealed_for(o: &Offer) -> (Manifest, String) {
    let m = manifest(o, vec![runner("SHA256:iDqoxaySm9gzxtvLrNXXpM5PimPLeBknaaNj0Rg7vz4")]);
    let sealed = seal_reply(&m, o.reply_key.as_deref().unwrap()).unwrap();
    (m, sealed)
}

#[test]
fn a_sealed_reply_opens_to_the_manifest_it_sealed() {
    let key = ReplyKey::generate();
    let o = asking(&key);
    let (m, sealed) = sealed_for(&o);

    assert_eq!(open_reply(&sealed, &key, &o).unwrap(), encode_manifest(&m));
    assert_eq!(take_sealed_reply(&sealed, &key, &o, false, Duration::ZERO).unwrap(), m);
}

#[test]
fn the_ciphertext_holds_none_of_the_reply() {
    let key = ReplyKey::generate();
    let o = asking(&key);
    let (_, sealed) = sealed_for(&o);
    let bytes = Base64UrlUnpadded::decode_vec(&sealed).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    for plain in ["SHA256:", "box.tail-1234", "acct_1", &o.ceremony] {
        assert!(!sealed.contains(plain) && !text.contains(plain), "{plain} in a sealed reply");
    }
}

#[test]
fn a_wrong_key_does_not_open_it() {
    let key = ReplyKey::generate();
    let (_, sealed) = sealed_for(&asking(&key));

    // Another device, showing the same ceremony with its own key.
    let other = ReplyKey::generate();
    let theirs = Offer { reply_key: Some(other.public().into()), ..asking(&key) };
    assert_eq!(open_reply(&sealed, &other, &theirs), Err(CeremonyError::Unopened));
}

#[test]
fn a_reply_for_another_ceremony_account_channel_or_key_does_not_open() {
    let key = ReplyKey::generate();
    let o = asking(&key);
    let (_, sealed) = sealed_for(&o);

    let mut ceremony = o.ceremony.clone().into_bytes();
    ceremony[0] ^= 1;
    let changed = [
        Offer { ceremony: String::from_utf8(ceremony).unwrap(), ..o.clone() },
        Offer { account: "acct_2".into(), ..o.clone() },
        Offer { channel: format!("{}x", o.channel), ..o.clone() },
        Offer { key_a: KEY_B.into(), ..o.clone() },
    ];
    for expecting in changed {
        assert_eq!(open_reply(&sealed, &key, &expecting), Err(CeremonyError::Unopened), "{expecting:?}");
    }
}

/// Every byte of the associated data, one at a time, under the real seal.
#[test]
fn any_changed_aad_byte_fails_to_open() {
    let key = ReplyKey::generate();
    let o = asking(&key);
    let (m, sealed) = sealed_for(&o);
    let bytes = Base64UrlUnpadded::decode_vec(&sealed).unwrap();
    let (encapped, ciphertext) = bytes.split_at(KEY_BYTES);
    let encapped = <Kem as hpke::Kem>::EncappedKey::from_bytes(encapped).unwrap();
    let aad = associated(&m.ceremony, &m.account, &m.channel, &m.target);

    let open = |aad: &[u8]| hpke::single_shot_open::<Aead, Kdf, Kem>(&OpModeR::Base, &key.secret, &encapped, INFO, ciphertext, aad);
    assert!(open(&aad).is_ok(), "the unchanged AAD must open, or nothing below is tested");
    for i in 0..aad.len() {
        let mut changed = aad.clone();
        changed[i] ^= 0x01;
        assert!(open(&changed).is_err(), "AAD byte {i} changed and it still opened");
    }
}

#[test]
fn the_aad_frames_each_field() {
    assert_ne!(associated("c", "acct_1", "canary", "t"), associated("c", "acct_1c", "anary", "t"));
}

#[test]
fn any_changed_ciphertext_byte_fails_to_open() {
    let key = ReplyKey::generate();
    let o = asking(&key);
    let (_, sealed) = sealed_for(&o);
    let bytes = Base64UrlUnpadded::decode_vec(&sealed).unwrap();
    for i in 0..bytes.len() {
        let mut changed = bytes.clone();
        changed[i] ^= 0x80;
        let tampered = Base64UrlUnpadded::encode_string(&changed);
        assert_eq!(open_reply(&tampered, &key, &o), Err(CeremonyError::Unopened), "byte {i}");
    }
    assert_eq!(open_reply("", &key, &o), Err(CeremonyError::Unopened));
    assert_eq!(open_reply("not base64!", &key, &o), Err(CeremonyError::Unopened));
}

#[test]
fn an_offer_that_named_another_key_takes_no_sealed_reply() {
    let key = ReplyKey::generate();
    let (_, sealed) = sealed_for(&asking(&key));
    let plain = offer("Chrome", "acct_1", KEY_A, None);
    assert!(matches!(open_reply(&sealed, &key, &plain), Err(CeremonyError::Malformed(_))));
}

#[test]
fn taking_a_sealed_reply_keeps_take_replys_order() {
    let key = ReplyKey::generate();
    let o = asking(&key);
    let (_, sealed) = sealed_for(&o);
    // Taken already, before anything is opened: even garbage says so.
    assert_eq!(take_sealed_reply("garbage", &key, &o, true, Duration::ZERO), Err(CeremonyError::AlreadyTaken));
    // Stale, before anything is opened.
    let late = FRESHNESS_PLUS_ONE;
    assert_eq!(take_sealed_reply("garbage", &key, &o, false, late), Err(CeremonyError::Stale));
    assert_eq!(take_sealed_reply(&sealed, &key, &o, false, late), Err(CeremonyError::Stale));
}

const FRESHNESS_PLUS_ONE: Duration = Duration::from_secs(crate::FRESHNESS.as_secs() + 1);

#[test]
fn a_manifest_over_budget_is_not_sealed_and_one_at_budget_fits_the_relay() {
    let key = ReplyKey::generate();
    let o = asking(&key);
    let mut m = manifest(&o, vec![runner("SHA256:x")]);
    let room = BUDGET - encode_manifest(&m).len();
    m.runners[0].label.push_str(&"x".repeat(room));
    assert_eq!(encode_manifest(&m).len(), BUDGET);

    let sealed = seal_reply(&m, key.public()).unwrap();
    // The body a phone posts: the relay refuses anything over 4096 bytes.
    let body = serde_json::json!({ "ceremony": o.ceremony, "account": "a".repeat(64), "sealed": sealed }).to_string();
    assert!(body.len() <= 4096, "a sealed manifest at budget posts {} bytes", body.len());

    m.runners[0].label.push('x');
    assert_eq!(seal_reply(&m, key.public()), Err(CeremonyError::TooLarge));
}

#[test]
fn a_reply_key_that_is_not_a_key_is_refused() {
    let m = manifest(&offer("Chrome", "acct_1", KEY_A, None), vec![]);
    for bad in ["", "AAAA", "not base64 at all", &"A".repeat(44)] {
        assert!(matches!(seal_reply(&m, bad), Err(CeremonyError::Malformed(_))), "{bad}");
    }
}

#[test]
fn an_offer_without_a_reply_key_is_unchanged_on_the_wire() {
    let o = offer("iPhone 17", "acct_1", KEY_A, None);
    assert!(!encode_offer(&o).contains("reply_key"));

    let key = ReplyKey::generate();
    let asked = asking(&key);
    let decoded = decode_offer(&encode_offer(&asked)).unwrap();
    assert_eq!(decoded.reply_key.as_deref(), Some(key.public()));
    assert_eq!(key.public().len(), 43);
}

#[test]
fn a_key_survives_its_secret_and_never_prints_it() {
    let key = ReplyKey::generate();
    let again = ReplyKey::from_secret(&key.secret()).unwrap();
    assert_eq!(again.public(), key.public());
    assert!(!format!("{key:?}").contains(&key.secret()));
    assert!(matches!(ReplyKey::from_secret("short"), Err(CeremonyError::Malformed(_))));
}

#[derive(serde::Deserialize, serde::Serialize)]
struct Fixture {
    secret: String,
    offer: Offer,
    manifest: String,
    host_key: String,
    sealed: String,
}

/// The relay's tests post this sealed reply and look for its host key in D1
/// and the logs. It has to be a real one, sealed by this code, or a relay
/// that stored the plaintext would pass by never having been given any.
#[test]
fn the_relays_fixture_is_a_real_sealed_reply() {
    let f: Fixture = serde_json::from_str(FIXTURE).unwrap();
    let key = ReplyKey::from_secret(&f.secret).unwrap();
    let opened = open_reply(&f.sealed, &key, &f.offer).unwrap();
    assert_eq!(opened, f.manifest);
    assert_eq!(f.host_key, FIXTURE_HOST_KEY);
    assert!(opened.contains(FIXTURE_HOST_KEY), "the fixture's reply must carry the key the relay looks for");
    assert!(!f.sealed.contains(FIXTURE_HOST_KEY));
}

/// Writes the fixture above. Run once, by hand:
/// `FARCOOLER_WRITE_FIXTURE=1 cargo test -p farcooler-ceremony write_the_relays_fixture -- --ignored`.
#[test]
#[ignore]
fn write_the_relays_fixture() {
    if std::env::var("FARCOOLER_WRITE_FIXTURE").is_err() {
        return;
    }
    let key = ReplyKey::generate();
    let o = Offer { ceremony: "0123456789abcdef0123456789abcdef".into(), ..asking(&key) };
    let m = manifest(&o, vec![runner(FIXTURE_HOST_KEY)]);
    let sealed = seal_reply(&m, key.public()).unwrap();
    let f = Fixture { secret: key.secret(), offer: o, manifest: encode_manifest(&m), host_key: FIXTURE_HOST_KEY.into(), sealed };
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../services/relay/test/fixtures/ceremony-reply.json");
    std::fs::write(path, serde_json::to_string_pretty(&f).unwrap() + "\n").unwrap();
}
