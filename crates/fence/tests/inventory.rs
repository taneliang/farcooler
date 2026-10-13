//! The inventory of `authorized_keys` (ov-510): every key in the file, inside
//! the fence and outside it, read without writing a byte.

use std::os::unix::fs::MetadataExt as _;
use std::path::PathBuf;

use farcooler_fence::{self as fence, Listed, AUTHORIZED_KEYS};
use farcooler_protocol::v1::Scope;

const KEY: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const OTHER_KEY: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIBERERERERERERERERERERERERERERERERERERERERER";
const THIRD_KEY: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8g";

fn ours(key: &str, client: &str, scope: Scope) -> String {
    fence::render(&format!("ssh-ed25519 {key} x"), "iPhone", client, scope, fence::Grant::FarCooler, None).expect("render")
}

/// A file with a hand-added key above the block, a Far Cooler line and a foreign
/// line inside it, a planted key below, comments, options, and two bad lines.
fn a_busy_file() -> String {
    format!(
        "# keys I added myself\n\
         ssh-ed25519 {KEY} me@laptop\n\
         \n\
         from=\"10.0.0.0/8\",no-port-forwarding ssh-ed25519 {OTHER_KEY} ci@build\n\
         this is not a key\n\
         {}\n\
         {}\n\
         # a comment inside the block\n\
         ssh-ed25519 {THIRD_KEY} pasted-into-the-block\n\
         {}\n\
         ssh-rsa AAAAnotreal planted@elsewhere\n",
        fence::BEGIN,
        ours(KEY, "dev-1", Scope::Control).replace(KEY, "AAAAC3NzaC1lZDI1NTE5AAAAIAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8h"),
        fence::END,
    )
}

fn by_comment<'a>(found: &'a [Listed], comment: &str) -> &'a Listed {
    found.iter().find(|e| e.comment == comment).unwrap_or_else(|| panic!("no entry commented {comment}: {found:?}"))
}

#[test]
fn it_lists_keys_inside_and_outside_the_fence_with_their_comments_and_scopes() {
    let found = fence::inventory_of(&a_busy_file(), AUTHORIZED_KEYS).expect("inventory");
    let list = &found.entries;
    assert_eq!(list.len(), 4, "{list:?}");

    let mine = by_comment(list, "me@laptop");
    assert!(!mine.inside, "above the fence");
    assert_eq!(mine.scope, "other", "a line Far Cooler didn't write");

    // Options before the key don't hide it, and the comment is the key's.
    let ci = by_comment(list, "ci@build");
    assert!(!ci.inside);
    assert!(ci.fingerprint.starts_with("SHA256:"), "{ci:?}");

    // A foreign line pasted into the block is inside it, and still not ours.
    let pasted = by_comment(list, "pasted-into-the-block");
    assert!(pasted.inside);
    assert_eq!(pasted.scope, "other");

    // Our own line reports the scope its forced command carries.
    let own = list.iter().find(|e| e.scope == "control").expect("our control line");
    assert!(own.inside, "{own:?}");

    // The comment lines and the blank line are not keys and not damage. The
    // sentence and the bad `ssh-rsa` line are counted but never named.
    assert_eq!(found.unparsed, 2);
    assert!(list.iter().all(|e| !e.comment.contains("planted@elsewhere")));
}

#[test]
fn a_key_that_sits_in_the_file_twice_is_listed_twice() {
    let text = format!("ssh-ed25519 {KEY} a\n{}\nssh-ed25519 {KEY} b\n{}\n", fence::BEGIN, fence::END);
    let found = fence::inventory_of(&text, AUTHORIZED_KEYS).expect("inventory");
    assert_eq!(found.entries.len(), 2);
    assert_eq!(found.entries[0].fingerprint, found.entries[1].fingerprint);
}

#[test]
fn a_file_with_no_fence_is_all_outside_and_a_damaged_one_is_refused() {
    let plain = format!("ssh-ed25519 {KEY} a\n");
    let found = fence::inventory_of(&plain, AUTHORIZED_KEYS).expect("inventory");
    assert!(!found.entries[0].inside);
    let damaged = format!("{}\nssh-ed25519 {KEY} a\n", fence::BEGIN);
    assert!(matches!(fence::inventory_of(&damaged, AUTHORIZED_KEYS), Err(fence::FenceError::Damaged(_))));
}

#[test]
fn the_digest_follows_the_keys_and_not_their_order() {
    let a = fence::inventory_of(&format!("ssh-ed25519 {KEY} a\nssh-ed25519 {OTHER_KEY} b\n"), AUTHORIZED_KEYS).unwrap();
    let b = fence::inventory_of(&format!("ssh-ed25519 {OTHER_KEY} b\nssh-ed25519 {KEY} a\n"), AUTHORIZED_KEYS).unwrap();
    assert_eq!(a.digest(), b.digest(), "same keys, same hash");
    let c = fence::inventory_of(&format!("ssh-ed25519 {OTHER_KEY} b\nssh-ed25519 {KEY} renamed\n"), AUTHORIZED_KEYS).unwrap();
    assert_ne!(a.digest(), c.digest(), "a changed comment is a changed inventory");
    let d = fence::inventory_of(
        &format!("ssh-ed25519 {OTHER_KEY} b\n{}\nssh-ed25519 {KEY} a\n{}\n", fence::BEGIN, fence::END),
        AUTHORIZED_KEYS,
    )
    .unwrap();
    assert_ne!(a.digest(), d.digest(), "moving a key into the fence is a change");
    let e = fence::inventory_of(&format!("ssh-ed25519 {KEY} a\nssh-ed25519 {OTHER_KEY} b\nnot a key\n"), AUTHORIZED_KEYS).unwrap();
    assert_ne!(a.digest(), e.digest(), "a new unreadable line is a change");
}

fn a_runner(text: &str) -> (tempfile::TempDir, PathBuf) {
    let home = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(home.path().join(".ssh")).expect("mkdir .ssh");
    let path = home.path().join(".ssh").join("authorized_keys");
    std::fs::write(&path, text).expect("write");
    (home, path)
}

/// Reading never writes: the file's bytes, mode and times are the same after,
/// and nothing appears beside it (no lock, no backup).
///
/// Mutation: make `inventory` call `fence::write` or take the lock, or open
/// the directory with `create`, and this goes red.
#[test]
fn reading_the_inventory_changes_nothing() {
    let (home, path) = a_runner(&a_busy_file());
    let before = std::fs::metadata(&path).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    // Back-date the file so an untouched mtime can't be the clock not moving.
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    std::fs::File::options().write(true).open(&path).unwrap().set_modified(old).unwrap();
    let before_mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
    let dir_before = std::fs::read_dir(home.path().join(".ssh")).unwrap().count();

    let found = fence::inventory(&path, AUTHORIZED_KEYS).expect("inventory");
    assert_eq!(found.entries.len(), 4);

    let after = std::fs::metadata(&path).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), bytes, "bytes");
    assert_eq!(after.modified().unwrap(), before_mtime, "mtime");
    assert_eq!(after.mode(), before.mode(), "mode");
    assert_eq!(std::fs::read_dir(home.path().join(".ssh")).unwrap().count(), dir_before, "no lock or backup beside it");
}

#[test]
fn a_runner_with_no_ssh_directory_is_an_empty_inventory_and_stays_without_one() {
    let home = tempfile::tempdir().expect("tempdir");
    let path = home.path().join(".ssh").join("authorized_keys");
    let found = fence::inventory(&path, AUTHORIZED_KEYS).expect("inventory");
    assert!(found.entries.is_empty());
    assert!(!home.path().join(".ssh").exists(), "looking must not make .ssh");
    // And with `.ssh` but no file: empty, and still no file.
    std::fs::create_dir(home.path().join(".ssh")).unwrap();
    assert!(fence::inventory(&path, AUTHORIZED_KEYS).unwrap().entries.is_empty());
    assert!(!path.exists());
}
