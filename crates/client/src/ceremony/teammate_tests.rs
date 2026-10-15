use serde_json::{Value, json};

use super::super::{
    CeremonyError, accept_manifest, accept_offer, encode_manifest, encode_offer, fingerprint,
    fit_manifest, fit_manifest_from, offer,
};
use super::*;

const KEY_A: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIB1iLbeqDzK4CDeUC3t+ffVPDI9Gk+sBwIZqJZW1NfS5 device-a";
const KEY_B: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDMdwe233CUbxjpEHkissIUGdCxhkTsDE/Zg7f+LB6S+ device-b";

/// Alice, scanning, is an admin of Platform and a member of Infra. Bob is in
/// Platform, Carol in Infra, Dan is Platform's billing contact, and Erin's
/// team (Design) is one Alice isn't in.
fn directory() -> Value {
    json!({
        "me": {
            "user": { "id": "user_alice", "email": "alice@example.com" },
            "teams": [
                { "id": "t-personal", "name": "Personal", "personal": true, "role": "admin" },
                { "id": "t-platform", "name": "Platform", "personal": false, "role": "admin" },
                { "id": "t-infra", "name": "Infra", "personal": false, "role": "member" },
            ],
        },
        "rosters": [
            { "team": { "id": "t-platform", "name": "Platform", "personal": false }, "members": [
                { "user": "user_alice", "email": "alice@example.com", "role": "admin" },
                { "user": "user_bob", "email": "bob@example.com", "role": "member" },
                { "user": "user_dan", "email": "dan@example.com", "role": "billing" },
            ]},
            { "team": { "id": "t-infra", "name": "Infra", "personal": false }, "members": [
                { "user": "user_carol", "email": "carol@example.com", "role": "admin" },
                { "user": "user_alice", "email": "alice@example.com", "role": "member" },
            ]},
            { "team": { "id": "t-design", "name": "Design", "personal": false }, "members": [
                { "user": "user_erin", "email": "erin@example.com", "role": "admin" },
            ]},
        ],
    })
}

fn code_from(account: &str) -> String {
    encode_offer(&offer("Bob’s iPhone", account, KEY_A, None))
}

fn scan(code: &str, directory: &Value) -> Result<TeammateOffer, CeremonyError> {
    accept_teammate_offer(code, "user_alice", Some(directory), true)
}

/// A teammate's code is accepted, and who they are is the directory's word:
/// the email shown comes from it, never from the code.
#[test]
fn a_teammates_code_is_accepted_with_the_directorys_email() {
    let bob = scan(&code_from("user_bob"), &directory()).expect("Bob shares Platform");
    assert_eq!(bob.teammate.email, "bob@example.com");
    assert_eq!(bob.teammate.team, "Platform");
    assert_eq!(bob.offer.account, "user_bob");
    let carol = scan(&code_from("user_carol"), &directory()).expect("Carol shares Infra");
    assert_eq!((carol.teammate.email.as_str(), carol.teammate.team.as_str()), ("carol@example.com", "Infra"));
}

/// The same-account door is unchanged: another account is still refused
/// there, whoever the directory says they are.
#[test]
fn the_same_account_door_still_refuses_a_teammate() {
    assert!(matches!(accept_offer(&code_from("user_bob"), "user_alice"), Err(CeremonyError::WrongAccount)));
}

/// No directory, no teammate: the check never passes because it couldn't run.
#[test]
fn an_unreachable_directory_is_a_refusal() {
    assert!(matches!(
        accept_teammate_offer(&code_from("user_bob"), "user_alice", None, true),
        Err(CeremonyError::DirectoryUnreachable)
    ));
}

/// An answer about someone else, or one missing a shared team's roster, can't
/// say who is a teammate.
#[test]
fn an_answer_that_cannot_decide_is_a_refusal() {
    let mut someone_else = directory();
    someone_else["me"]["user"]["id"] = json!("user_mallory");
    assert!(matches!(scan(&code_from("user_bob"), &someone_else), Err(CeremonyError::DirectoryUnreachable)));

    let mut missing = directory();
    missing["rosters"].as_array_mut().unwrap().remove(1);
    // Bob is in the roster that is there, but Infra's is missing: the answer
    // is incomplete, so it isn't used at all.
    assert!(matches!(scan(&code_from("user_bob"), &missing), Err(CeremonyError::DirectoryUnreachable)));

    for junk in [json!({}), json!({ "me": {}, "rosters": [] }), json!(null)] {
        assert!(matches!(scan(&code_from("user_bob"), &junk), Err(CeremonyError::DirectoryUnreachable)), "{junk}");
    }
    assert!(matches!(
        accept_teammate_offer(&code_from("user_bob"), "", Some(&directory()), true),
        Err(CeremonyError::DirectoryUnreachable)
    ));
}

/// An account in none of the scanning person's teams is refused, whatever
/// else the directory knows about it.
#[test]
fn an_account_outside_every_shared_team_is_refused() {
    assert!(matches!(scan(&code_from("user_mallory"), &directory()), Err(CeremonyError::NotATeammate)));
    // Erin is in a team the directory sent a roster for, but it isn't one of
    // Alice's: a roster is evidence only for a team `me` lists.
    assert!(matches!(scan(&code_from("user_erin"), &directory()), Err(CeremonyError::NotATeammate)));
}

/// `billing` grants nothing, either way round.
#[test]
fn a_billing_role_vouches_for_nobody() {
    assert!(matches!(scan(&code_from("user_dan"), &directory()), Err(CeremonyError::NotATeammate)));
    let mut billing = directory();
    billing["me"]["teams"][1]["role"] = json!("billing");
    assert!(matches!(scan(&code_from("user_bob"), &billing), Err(CeremonyError::NotATeammate)));
}

/// A personal team holds only its owner, so a roster claiming otherwise is
/// not believed.
#[test]
fn a_personal_team_vouches_for_nobody() {
    let mut odd = directory();
    odd["rosters"].as_array_mut().unwrap().push(json!({
        "team": { "id": "t-personal", "name": "Personal", "personal": true },
        "members": [{ "user": "user_mallory", "email": "m@example.com", "role": "admin" }],
    }));
    assert!(matches!(scan(&code_from("user_mallory"), &odd), Err(CeremonyError::NotATeammate)));
}

/// Your own devices go through the same-account door, and a code naming no
/// account is nobody's teammate.
#[test]
fn your_own_account_or_none_is_not_a_teammate() {
    assert!(matches!(scan(&code_from("user_alice"), &directory()), Err(CeremonyError::NotATeammate)));
    assert!(matches!(scan(&code_from(""), &directory()), Err(CeremonyError::NotATeammate)));
}

/// A teammate is never granted a shell, so a Mac that asked for one is
/// refused rather than half-granted.
#[test]
fn a_teammate_asking_for_a_shell_is_refused() {
    let mac = encode_offer(&offer("Bob’s MacBook", "user_bob", KEY_A, Some(KEY_B)));
    assert!(matches!(scan(&mac, &directory()), Err(CeremonyError::TeammateShell)));
}

/// Once the directory lists a teammate's registered keys, the code's Key A
/// must be one of them.
#[test]
fn a_registered_teammate_must_show_a_registered_key() {
    let mut registered = directory();
    registered["rosters"][0]["members"][1]["keys"] = json!([fingerprint(KEY_B).unwrap()]);
    assert!(matches!(scan(&code_from("user_bob"), &registered), Err(CeremonyError::UnregisteredKey)));
    registered["rosters"][0]["members"][1]["keys"] = json!([fingerprint(KEY_B).unwrap(), fingerprint(KEY_A).unwrap()]);
    assert!(scan(&code_from("user_bob"), &registered).is_ok());
}

/// A directory that lists a teammate's keys and lists none means they have no
/// device anyone vouched for: every code naming them is refused (ov-542).
#[test]
fn a_teammate_with_no_registered_key_is_refused() {
    let mut none = directory();
    none["rosters"][0]["members"][1]["keys"] = json!([]);
    assert!(matches!(scan(&code_from("user_bob"), &none), Err(CeremonyError::UnregisteredKey)));
}

/// Someone in two shared teams is checked if either roster told their keys:
/// the roster that didn't can't switch the check off, in either order.
#[test]
fn a_roster_that_lists_keys_holds_whatever_another_left_out() {
    for told in [0, 1] {
        let mut both = directory();
        both["rosters"][1]["members"].as_array_mut().unwrap().push(json!(
            { "user": "user_bob", "email": "bob@example.com", "role": "member" }
        ));
        both["rosters"][told]["members"].as_array_mut().unwrap().iter_mut()
            .filter(|m| m["user"] == "user_bob")
            .for_each(|m| m["keys"] = json!([fingerprint(KEY_B).unwrap()]));
        assert!(matches!(scan(&code_from("user_bob"), &both), Err(CeremonyError::UnregisteredKey)), "told in roster {told}");
    }
}

/// Two rosters that list different keys for one person pass only a key both
/// list: a key only one of them names is refused (review L1).
#[test]
fn two_rosters_that_disagree_pass_only_what_both_list() {
    let mut both = directory();
    both["rosters"][1]["members"].as_array_mut().unwrap().push(json!(
        { "user": "user_bob", "email": "bob@example.com", "role": "member", "keys": [fingerprint(KEY_B).unwrap()] }
    ));
    both["rosters"][0]["members"][1]["keys"] = json!([fingerprint(KEY_A).unwrap()]);
    assert!(matches!(scan(&code_from("user_bob"), &both), Err(CeremonyError::UnregisteredKey)));
    // Both listing it, beside others, passes.
    both["rosters"][1]["members"][2]["keys"] = json!([fingerprint(KEY_B).unwrap(), fingerprint(KEY_A).unwrap()]);
    assert!(scan(&code_from("user_bob"), &both).is_ok());
}

/// `"keys": null` is "not told", the same as no field: the in-person check
/// stands alone, as it does with the relay's flag off (R-72).
#[test]
fn null_keys_are_not_told() {
    let mut null = directory();
    null["rosters"][0]["members"][1]["keys"] = Value::Null;
    assert!(scan(&code_from("user_bob"), &null).is_ok());
}

/// A `keys` field that isn't a list of strings can't say whose key is whose.
#[test]
fn keys_in_any_other_shape_are_an_answer_that_cannot_decide() {
    for keys in [json!("SHA256:x"), json!([1]), json!({}), json!(true)] {
        let mut odd = directory();
        odd["rosters"][0]["members"][1]["keys"] = keys.clone();
        assert!(matches!(scan(&code_from("user_bob"), &odd), Err(CeremonyError::DirectoryUnreachable)), "{keys}");
    }
}

/// What every code is held to still comes first.
#[test]
fn version_and_channel_still_come_first() {
    let mut other_channel = offer("Bob’s iPhone", "user_bob", KEY_A, None);
    other_channel.channel = "a-channel-we-are-not".into();
    assert!(matches!(
        accept_teammate_offer(&encode_offer(&other_channel), "user_alice", None, true),
        Err(CeremonyError::Channel(_))
    ));
}

/// A teammate's reply names whose runners they are, and the teammate's own
/// device takes it; a same-account reply carries no such field at all.
#[test]
fn a_teammates_reply_names_who_granted_it() {
    let bobs = offer("Bob’s iPhone", "user_bob", KEY_A, None);
    let reply = fit_manifest_from(&bobs, Vec::new(), Some("alice@example.com"), super::super::BUDGET).unwrap();
    let taken = accept_manifest(&encode_manifest(&reply.manifest), &bobs, false).expect("Bob's phone takes it");
    assert_eq!(taken.granted_by.as_deref(), Some("alice@example.com"));

    let own = fit_manifest(&bobs, Vec::new(), super::super::BUDGET).unwrap();
    assert!(!encode_manifest(&own.manifest).contains("granted_by"), "a same-account reply changed shape");
}

/// A teammate the directory gives no email is an answer that can't be used:
/// the email is what the admin confirms in person (review F4).
#[test]
fn a_teammate_with_no_email_is_an_answer_that_cannot_decide() {
    for email in [json!(null), json!(""), json!("   "), json!("bob")] {
        let mut blank = directory();
        blank["rosters"][0]["members"][1]["email"] = email.clone();
        assert!(
            matches!(scan(&code_from("user_bob"), &blank), Err(CeremonyError::DirectoryUnreachable)),
            "{email}"
        );
    }
}

/// A key Platform's roster holds after a trust reset (R-76) is accepted, with
/// the hold named for the admin to confirm, and a key it lists isn't held.
#[test]
fn a_held_key_is_accepted_with_its_hold_named() {
    let mut held = directory();
    held["rosters"][0]["members"][1]["keys"] = json!([fingerprint(KEY_B).unwrap()]);
    held["rosters"][0]["members"][1]["held"] = json!([{ "key": fingerprint(KEY_A).unwrap(), "until": 1_700_000_000_000_i64 }]);
    let bob = scan(&code_from("user_bob"), &held).expect("a held key is the roster's word");
    assert_eq!(
        bob.held,
        vec![HeldKey { key: fingerprint(KEY_A).unwrap(), team: "t-platform".into(), until: 1_700_000_000_000 }]
    );

    held["rosters"][0]["members"][1]["keys"] = json!([fingerprint(KEY_A).unwrap()]);
    held["rosters"][0]["members"][1]["held"] = json!([{ "key": fingerprint(KEY_B).unwrap(), "until": 1 }]);
    assert_eq!(scan(&code_from("user_bob"), &held).expect("listed").held, vec![]);
}

/// Held by one roster and listed by the other, the key passes with only the
/// holding team named; held by neither and listed by one, it's refused.
#[test]
fn a_held_key_names_only_the_teams_that_hold_it() {
    let mut both = directory();
    both["rosters"][1]["members"].as_array_mut().unwrap().push(json!(
        { "user": "user_bob", "email": "bob@example.com", "role": "member",
          "keys": [fingerprint(KEY_A).unwrap()], "held": [] }
    ));
    both["rosters"][0]["members"][1]["keys"] = json!([]);
    both["rosters"][0]["members"][1]["held"] = json!([{ "key": fingerprint(KEY_A).unwrap(), "until": 5 }]);
    let bob = scan(&code_from("user_bob"), &both).expect("every roster admits it");
    assert_eq!(bob.held.iter().map(|h| h.team.as_str()).collect::<Vec<_>>(), vec!["t-platform"]);

    // A roster that neither lists nor holds it still refuses it.
    both["rosters"][0]["members"][1]["held"] = json!([{ "key": fingerprint(KEY_B).unwrap(), "until": 5 }]);
    assert!(matches!(scan(&code_from("user_bob"), &both), Err(CeremonyError::UnregisteredKey)));
}

/// Held keys in any other shape, or from a roster that didn't tell `keys`,
/// can't decide.
#[test]
fn held_keys_in_any_other_shape_are_an_answer_that_cannot_decide() {
    for held in [json!("x"), json!([{ "key": "SHA256:x" }]), json!([{ "until": 5 }]), json!([{ "key": 1, "until": 5 }]), json!({})] {
        let mut odd = directory();
        odd["rosters"][0]["members"][1]["keys"] = json!([]);
        odd["rosters"][0]["members"][1]["held"] = held.clone();
        assert!(matches!(scan(&code_from("user_bob"), &odd), Err(CeremonyError::DirectoryUnreachable)), "{held}");
    }
    let mut untold = directory();
    untold["rosters"][0]["members"][1]["held"] = json!([{ "key": fingerprint(KEY_A).unwrap(), "until": 5 }]);
    assert!(matches!(scan(&code_from("user_bob"), &untold), Err(CeremonyError::DirectoryUnreachable)));
}

/// A caller that won't confirm a hold in person gets `held_key` for a held
/// key, never a pass; a listed key passes either way (fix round 1, Low 1).
#[test]
fn a_held_key_is_refused_unless_the_caller_will_confirm_it() {
    let mut held = directory();
    held["rosters"][0]["members"][1]["keys"] = json!([]);
    held["rosters"][0]["members"][1]["held"] = json!([{ "key": fingerprint(KEY_A).unwrap(), "until": 5 }]);
    assert!(matches!(
        accept_teammate_offer(&code_from("user_bob"), "user_alice", Some(&held), false),
        Err(CeremonyError::HeldKey)
    ));
    assert_eq!(CeremonyError::HeldKey.code(), "held_key");
    let mut listed = directory();
    listed["rosters"][0]["members"][1]["keys"] = json!([fingerprint(KEY_A).unwrap()]);
    assert!(accept_teammate_offer(&code_from("user_bob"), "user_alice", Some(&listed), false).is_ok());
}
