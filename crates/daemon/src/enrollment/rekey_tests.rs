//! `client.rekey`'s rules, on a block in memory: whose line moves, what it
//! keeps, and every refusal leaving the block as it was.

use super::*;
use ssh_key::private::Ed25519Keypair;
use ssh_key::{HashAlg, LineEnding, PrivateKey};

fn key(seed: u8) -> PrivateKey {
    PrivateKey::from(Ed25519Keypair::from_seed(&[seed; 32]))
}

fn public(key: &PrivateKey) -> String {
    ssh_key::PublicKey::new(key.public_key().key_data().clone(), "").to_openssh().unwrap()
}

fn fingerprint(key: &PrivateKey) -> String {
    key.public_key().fingerprint(HashAlg::Sha256).to_string()
}

const NODE: &str = "nodekeyAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

/// One rendered line, read back as an entry.
fn line(key: &PrivateKey, name: &str, id: &str, scope: Scope, grant: fence::Grant, node: Option<&str>) -> Entry {
    let text = fence::render(&public(key), name, id, scope, grant, node).unwrap();
    read_back(&text).unwrap()
}

/// The new key's proof over `old` → `new`.
fn proof(new: &PrivateKey, old_fingerprint: &str) -> ssh_key::SshSig {
    let statement = fence::rekey_statement(old_fingerprint, &fingerprint(new)).unwrap();
    let armored = new.sign(fence::REKEY_NAMESPACE, HashAlg::Sha512, statement.as_bytes()).unwrap();
    ssh_key::SshSig::from_pem(armored.to_pem(LineEnding::LF).unwrap()).unwrap()
}

/// Every device the owner's: no member binds one.
fn owner(_: &str) -> Option<Option<String>> {
    Some(None)
}

fn ask(new: &PrivateKey, id: &str) -> ClientRekey {
    ClientRekey { public_key: public(new), client_id: id.into(), proof: String::new() }
}

/// The block a rewrite produced.
fn written(change: fence::Change) -> (Vec<String>, Vec<String>) {
    match change {
        fence::Change::Write { entries, foreign } => (entries, foreign),
        fence::Change::Leave => panic!("nothing was written"),
    }
}

fn phone_and_friends() -> (PrivateKey, Vec<Entry>) {
    let phone = key(1);
    let other = key(2);
    let mut foreign = line(&key(3), "x", "unused", Scope::Read, fence::Grant::FarCooler, None);
    foreign.client_id.clear();
    foreign.line = format!("ssh-ed25519 {} someone@laptop", public(&key(3)).split(' ').nth(1).unwrap());
    let entries = vec![
        line(&other, "iPad", "farcooler-ipad", Scope::Read, fence::Grant::FarCooler, None),
        line(&phone, "iPhone", "farcooler-old", Scope::Control, fence::Grant::FarCooler, Some(NODE)),
        foreign,
    ];
    (phone, entries)
}

#[test]
fn the_callers_line_moves_to_the_new_key_and_keeps_everything_else() {
    let (phone, entries) = phone_and_friends();
    let new = key(9);
    let (change, entry) = rewrite(
        &entries, "farcooler-old", new.public_key(), &fingerprint(&new),
        &proof(&new, &fingerprint(&phone)), &ask(&new, "farcooler-new"), &owner,
    )
    .unwrap_or_else(|_| panic!("refused"));
    assert_eq!(entry.client_id, "farcooler-old", "the id changed, and with it whose device this is");
    assert_eq!(entry.fingerprint, fingerprint(&new));
    assert_eq!(entry.scope, Scope::Control, "the scope changed");
    assert_eq!(entry.node_key, NODE, "the node key was dropped");
    assert!(!entry.shell_access, "the line gained a shell");
    assert_eq!(fence::device_name(&entry.label), Some("iPhone"));

    let (ours, foreign) = written(change);
    assert_eq!(ours.len(), 2);
    assert_eq!(ours[0], entries[0].line, "another device's line was touched");
    assert_eq!(ours[1], entry.line);
    assert!(!ours.contains(&entries[1].line), "the old line is still there");
    assert_eq!(foreign, vec![entries[2].line.clone()], "a hand-added line was not carried through");
}

#[test]
fn only_the_connections_own_line_can_move() {
    let (phone, entries) = phone_and_friends();
    let new = key(9);
    // The iPad's connection, with a proof the phone made: the statement is
    // rebuilt from the iPad's line, so it doesn't verify.
    let refused = rewrite(
        &entries, "farcooler-ipad", new.public_key(), &fingerprint(&new),
        &proof(&new, &fingerprint(&phone)), &ask(&new, "farcooler-new"), &owner,
    );
    assert!(matches!(refused, Err(Refusal(DomainError::InvalidArgument { what: "proof" }))));
    // A connection with no line here.
    let refused = rewrite(
        &entries, "farcooler-gone", new.public_key(), &fingerprint(&new),
        &proof(&new, &fingerprint(&phone)), &ask(&new, "farcooler-new"), &owner,
    );
    assert!(matches!(refused, Err(Refusal(DomainError::NotFound))));
}

#[test]
fn a_proof_by_any_other_key_is_refused() {
    let (phone, entries) = phone_and_friends();
    let new = key(9);
    let impostor = key(8);
    // Signed by a key that isn't the one being moved to.
    let statement = fence::rekey_statement(&fingerprint(&phone), &fingerprint(&new)).unwrap();
    let armored = impostor.sign(fence::REKEY_NAMESPACE, HashAlg::Sha512, statement.as_bytes()).unwrap();
    let refused = rewrite(
        &entries, "farcooler-old", new.public_key(), &fingerprint(&new), &armored, &ask(&new, "farcooler-new"), &owner,
    );
    assert!(matches!(refused, Err(Refusal(DomainError::InvalidArgument { what: "proof" }))));
    // The right key, the wrong namespace.
    let armored = new.sign("farcooler-certify-v1", HashAlg::Sha512, statement.as_bytes()).unwrap();
    let refused = rewrite(
        &entries, "farcooler-old", new.public_key(), &fingerprint(&new), &armored, &ask(&new, "farcooler-new"), &owner,
    );
    assert!(matches!(refused, Err(Refusal(DomainError::InvalidArgument { what: "proof" }))));
}

#[test]
fn a_mac_with_a_shell_line_is_refused() {
    let mac = key(4);
    let shell = key(5);
    let entries = vec![
        line(&mac, "Mac", "mac-1", Scope::HostAdmin, fence::Grant::FarCooler, None),
        line(&shell, "Mac", "mac-1", Scope::HostAdmin, fence::Grant::Shell, None),
    ];
    let new = key(9);
    let refused = rewrite(
        &entries, "mac-1", new.public_key(), &fingerprint(&new), &proof(&new, &fingerprint(&mac)),
        &ask(&new, "mac-2"), &owner,
    );
    assert!(matches!(refused, Err(Refusal(DomainError::InvalidArgument { what: "client_id" }))));
}

#[test]
fn the_requests_id_is_never_written() {
    let (phone, entries) = phone_and_friends();
    let new = key(9);
    // Asking for the iPad's id, or any other, changes nothing about whose line it is.
    let (change, entry) = rewrite(
        &entries, "farcooler-old", new.public_key(), &fingerprint(&new), &proof(&new, &fingerprint(&phone)),
        &ask(&new, "farcooler-ipad"), &owner,
    )
    .unwrap_or_else(|_| panic!("refused"));
    assert_eq!(entry.client_id, "farcooler-old");
    let (ours, _) = written(change);
    assert_eq!(ours[0], entries[0].line, "the iPad's line changed");
}

#[test]
fn a_new_key_already_here_as_someone_elses_line_is_refused() {
    let (phone, mut entries) = phone_and_friends();
    let new = key(9);
    // A member's phone; the new key's line, from a ceremony, is bound to nobody.
    entries.push(line(&new, "iPhone", "farcooler-new", Scope::Control, fence::Grant::FarCooler, None));
    let whose = |id: &str| if id == "farcooler-old" { Some(Some("sam".to_string())) } else { Some(None) };
    let refused = rewrite(
        &entries, "farcooler-old", new.public_key(), &fingerprint(&new), &proof(&new, &fingerprint(&phone)),
        &ask(&new, "farcooler-new"), &whose,
    );
    assert!(matches!(refused, Err(Refusal(DomainError::InvalidArgument { what: "client_id" }))));
    // Nor when the store can't say.
    let refused = rewrite(
        &entries, "farcooler-old", new.public_key(), &fingerprint(&new), &proof(&new, &fingerprint(&phone)),
        &ask(&new, "farcooler-new"), &|_| None,
    );
    assert!(refused.is_err());
}

#[test]
fn a_new_key_already_here_only_takes_the_old_line_off() {
    let (phone, mut entries) = phone_and_friends();
    let new = key(9);
    // Someone scanned the phone's new code from a Mac first.
    entries.push(line(&new, "iPhone", "farcooler-new", Scope::Read, fence::Grant::FarCooler, None));
    let (change, entry) = rewrite(
        &entries, "farcooler-old", new.public_key(), &fingerprint(&new), &proof(&new, &fingerprint(&phone)),
        &ask(&new, "farcooler-new"), &owner,
    )
    .unwrap_or_else(|_| panic!("refused"));
    assert_eq!(entry.scope, Scope::Read, "the line the Mac wrote was rewritten");
    let (ours, _) = written(change);
    assert_eq!(ours, vec![entries[0].line.clone(), entries[3].line.clone()]);
}

#[test]
fn a_scope_this_build_cant_read_is_left_alone() {
    let (phone, mut entries) = phone_and_friends();
    entries[1].scope = Scope::Unspecified;
    let new = key(9);
    let refused = rewrite(
        &entries, "farcooler-old", new.public_key(), &fingerprint(&new), &proof(&new, &fingerprint(&phone)),
        &ask(&new, "farcooler-new"), &owner,
    );
    assert!(matches!(refused, Err(Refusal(DomainError::InvalidArgument { what: "scope" }))));
}
