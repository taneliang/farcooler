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
        pending_why: None,
    }
}

/// A browser's offer, asking for its reply sealed to `key`.
fn asking(key: &ReplyKey) -> Offer {
    Offer { reply_key: Some(key.public().to_string()), ..offer("Chrome", "acct_1", KEY_A, None) }
}

fn sealed_for(o: &Offer) -> (Manifest, String) {
    let (m, sealed, _) = sealed_with_code(o);
    (m, sealed)
}

fn sealed_with_code(o: &Offer) -> (Manifest, String, String) {
    let m = manifest(o, vec![runner("SHA256:iDqoxaySm9gzxtvLrNXXpM5PimPLeBknaaNj0Rg7vz4")]);
    let (sealed, code) = seal_reply(o, &m).unwrap();
    (m, sealed, code)
}

#[test]
fn a_sealed_reply_opens_to_the_manifest_it_sealed() {
    let key = ReplyKey::generate();
    let o = asking(&key);
    let (m, sealed) = sealed_for(&o);

    assert_eq!(open_reply(&sealed, &key, &o).unwrap().0, encode_manifest(&m));
    assert_eq!(take_sealed_reply(&sealed, &key, &o, false, Duration::ZERO).unwrap().0, m);
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
    let o = asking(&key);
    let (_, sealed) = sealed_for(&o);

    // The same offer in every echo, the same ceremony id included, naming
    // another key: only the key differs, so only the key can refuse it.
    let other = ReplyKey::generate();
    let theirs = Offer { reply_key: Some(other.public().into()), ..o.clone() };
    assert_eq!(open_reply(&sealed, &other, &theirs), Err(CeremonyError::Unopened));
    assert!(open_reply(&sealed, &key, &o).is_ok(), "the right key must open it, or this tests nothing");
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
    let o = asking(&key);
    let (_, sealed) = sealed_for(&o);
    // The same ceremony, so the AAD matches and the key would open it: only
    // the offer's own reply key refuses it.
    let none = Offer { reply_key: None, ..o.clone() };
    assert!(matches!(open_reply(&sealed, &key, &none), Err(CeremonyError::Malformed(_))));
    let other = Offer { reply_key: Some(ReplyKey::generate().public().into()), ..o.clone() };
    assert!(matches!(open_reply(&sealed, &key, &other), Err(CeremonyError::Malformed(_))));
}

#[test]
fn a_reply_is_sealed_only_for_the_offer_it_answers() {
    let key = ReplyKey::generate();
    let o = asking(&key);
    let m = manifest(&o, vec![runner("SHA256:x")]);
    assert!(seal_reply(&o, &m).is_ok());

    let no_key = Offer { reply_key: None, ..o.clone() };
    assert!(matches!(seal_reply(&no_key, &m), Err(CeremonyError::Malformed(_))));
    for other in [
        Offer { ceremony: "f".repeat(32), ..o.clone() },
        Offer { account: "acct_2".into(), ..o.clone() },
        Offer { channel: format!("{}x", o.channel), ..o.clone() },
        Offer { key_a: KEY_B.into(), ..o.clone() },
    ] {
        assert!(matches!(seal_reply(&other, &m), Err(CeremonyError::Malformed(_))), "{other:?}");
    }
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

    let sealed = seal_reply(&o, &m).unwrap();
    // The body a phone posts: the relay refuses anything over 4096 bytes.
    let body = serde_json::json!({ "ceremony": o.ceremony, "account": "a".repeat(64), "sealed": sealed }).to_string();
    assert!(body.len() <= 4096, "a sealed manifest at budget posts {} bytes", body.len());

    m.runners[0].label.push('x');
    assert_eq!(seal_reply(&o, &m), Err(CeremonyError::TooLarge));
}

#[test]
fn a_reply_key_that_is_not_a_key_is_refused() {
    let o = offer("Chrome", "acct_1", KEY_A, None);
    let m = manifest(&o, vec![]);
    for bad in ["", "AAAA", "not base64 at all", &"A".repeat(44)] {
        let asked = Offer { reply_key: Some(bad.into()), ..o.clone() };
        assert!(matches!(seal_reply(&asked, &m), Err(CeremonyError::Malformed(_))), "{bad}");
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
    let (opened, _) = open_reply(&f.sealed, &key, &f.offer).unwrap();
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
    let (sealed, _) = seal_reply(&o, &m).unwrap();
    let f = Fixture { secret: key.secret(), offer: o, manifest: encode_manifest(&m), host_key: FIXTURE_HOST_KEY.into(), sealed };
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../services/relay/test/fixtures/ceremony-reply.json");
    std::fs::write(path, serde_json::to_string_pretty(&f).unwrap() + "\n").unwrap();
}

#[test]
fn an_offer_whose_reply_key_is_not_a_key_does_not_decode() {
    let o = offer("Chrome", "acct_1", KEY_A, None);
    for bad in ["", "AAAA", "not base64 at all", &"A".repeat(44), "YwbAvkATmjREjY8yCsUr87XR8t4kv8l2bK516AhoEWQ="] {
        let asked = Offer { reply_key: Some(bad.into()), ..o.clone() };
        assert!(matches!(decode_offer(&encode_offer(&asked)), Err(CeremonyError::Malformed(_))), "{bad}");
    }
    let key = ReplyKey::generate();
    let asked = asking(&key);
    assert_eq!(decode_offer(&encode_offer(&asked)).unwrap(), asked);
    assert_eq!(decode_offer(&encode_offer(&o)).unwrap(), o);
}

#[test]
fn the_comparison_code_is_six_characters_a_person_can_read() {
    let key = ReplyKey::generate();
    let (_, _, code) = sealed_with_code(&asking(&key));
    assert_eq!(code.len(), CODE_LENGTH);
    assert!(code.bytes().all(|c| CODE_ALPHABET.contains(&c)), "{code}");
    assert!(!code.contains(['0', 'O', '1', 'I']), "{code}");
}

/// Both ends draw it from the exporter: the phone at the seal, the browser at
/// the opening. They must agree for an honest reply.
#[test]
fn the_opened_reply_shows_the_code_the_phone_shows() {
    let key = ReplyKey::generate();
    let o = asking(&key);
    let (m, sealed, code) = sealed_with_code(&o);
    let (taken, shown) = take_sealed_reply(&sealed, &key, &o, false, Duration::ZERO).unwrap();
    assert_eq!(taken, m);
    assert_eq!(shown, code);
    assert_eq!(open_reply(&sealed, &key, &o).unwrap().1, code);
}

/// The reason the code is not a hash of the manifest: everyone who can post
/// can predict the manifest. Two seals of the very same one show two codes
/// (one in a billion chance of a false failure, per run).
#[test]
fn the_code_is_not_a_function_of_the_manifest() {
    let key = ReplyKey::generate();
    let o = asking(&key);
    let m = manifest(&o, vec![runner("SHA256:iDqoxaySm9gzxtvLrNXXpM5PimPLeBknaaNj0Rg7vz4")]);
    let codes: std::collections::HashSet<String> = (0..8).map(|_| seal_reply(&o, &m).unwrap().1).collect();
    assert!(codes.len() > 1, "eight seals of one manifest all showed {codes:?}");
}

/// A reply sealed to another key shows another code, or does not open for
/// this one; so does a reply with a changed ciphertext, which does not open.
#[test]
fn a_reply_to_another_key_or_changed_does_not_show_this_code() {
    let key = ReplyKey::generate();
    let o = asking(&key);
    let (m, sealed, code) = sealed_with_code(&o);

    let other = ReplyKey::generate();
    let theirs = Offer { reply_key: Some(other.public().into()), ..o.clone() };
    let (_, forged, forged_code) = sealed_with_code(&theirs);
    assert_ne!(forged_code, code, "(one in a billion)");
    assert_eq!(open_reply(&forged, &key, &o), Err(CeremonyError::Unopened), "sealed to another key");
    assert_eq!(open_reply(&sealed, &other, &theirs), Err(CeremonyError::Unopened));

    let mut bytes = Base64UrlUnpadded::decode_vec(&sealed).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    let changed = Base64UrlUnpadded::encode_string(&bytes);
    assert_eq!(open_reply(&changed, &key, &o), Err(CeremonyError::Unopened), "changed ciphertext");

    // The honest one still opens, or the refusals above test nothing.
    assert_eq!(take_sealed_reply(&sealed, &key, &o, false, Duration::ZERO).unwrap(), (m, code));
}

/// Pinned, so a change to how the code is drawn is a change somebody made on
/// purpose: a phone and a browser on two builds would show two codes for one
/// honest reply. The relay's fixture is one sealed reply, so opening it with
/// its key always draws the same code.
#[test]
fn the_relays_fixture_has_a_known_code() {
    let f: Fixture = serde_json::from_str(FIXTURE).unwrap();
    let key = ReplyKey::from_secret(&f.secret).unwrap();
    let (opened, code) = open_reply(&f.sealed, &key, &f.offer).unwrap();
    assert_eq!(opened, f.manifest);
    assert_eq!(code, "ZQ3YER");
}
