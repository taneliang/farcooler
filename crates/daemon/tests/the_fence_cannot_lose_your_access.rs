//! The one file Far Cooler edits that somebody's access already depends on.
//!
//! `~/.ssh/authorized_keys` is not a Far Cooler file. It is a file a person may
//! have been using for years, with keys in it that have nothing to do with this
//! program, and the failure mode of getting a write wrong is not a broken
//! feature — it is being locked out of your own runner, which
//! `docs/farcooler-design.md:1017` calls release-blocking. So the writer is
//! tested for what it must never do, not for what it does.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use farcooler_daemon::fence::{self, AUTHORIZED_KEYS, FenceError};
use farcooler_protocol::v1::Scope;

/// A synthetic but structurally valid ed25519 key.
const KEY: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const OTHER_KEY: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIBERERERERERERERERERERERERERERERERERERERERER";

/// A home directory with a real `.ssh` in it, and the path inside it.
fn a_runner() -> (tempfile::TempDir, PathBuf) {
    let home = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(home.path().join(".ssh")).expect("mkdir .ssh");
    let path = home.path().join(".ssh").join("authorized_keys");
    (home, path)
}

fn entry(key: &str, client_id: &str) -> String {
    fence::render(&format!("ssh-ed25519 {key} x"), "iPhone", client_id, Scope::Control)
        .expect("render")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).expect("read back")
}

/// Everything that is not ours comes back byte for byte.
///
/// The keys above and below the fence are somebody's access to this machine.
/// Rewriting a file is how you preserve them exactly; appending is how you
/// lose one to a missing newline.
#[test]
fn every_byte_outside_the_fence_survives() {
    let (_home, path) = a_runner();
    let before = format!(
        "# their own comment\n\
         ssh-rsa AAAAtheirs me@laptop\n\
         \n\
         {}\n\
         {}\n\
         {}\n\
         ssh-ed25519 {OTHER_KEY} someone-else@laptop\n",
        fence::BEGIN,
        entry(KEY, "old"),
        fence::END,
    );
    std::fs::write(&path, &before).expect("write");

    fence::write(&path, AUTHORIZED_KEYS, &[entry(KEY, "new")], &[]).expect("fence write");

    let after = read(&path);
    let outside = |text: &str| -> String {
        let mut keep = String::new();
        let mut inside = false;
        for line in text.lines() {
            if line == fence::BEGIN {
                inside = true;
            }
            if !inside {
                keep.push_str(line);
                keep.push('\n');
            }
            if line == fence::END {
                inside = false;
            }
        }
        keep
    };
    assert_eq!(outside(&before), outside(&after), "a line outside the fence changed");
    let entries = fence::parse(&after).expect("parse");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].client_id, "new");
}

/// The classic append bug, which this write path does not have.
///
/// A file whose last line has no newline turns `>>` into a line that reads as
/// one key with a very long comment: the last key already there stops working,
/// and the one being added never starts. Rewriting the whole file rather than
/// appending is what avoids it, so the assertion is about the count.
#[test]
fn a_file_with_no_trailing_newline_still_gets_a_separate_entry() {
    let (_home, path) = a_runner();
    std::fs::write(&path, "ssh-rsa AAAAtheirs me@laptop").expect("write");

    fence::write(&path, AUTHORIZED_KEYS, &[entry(KEY, "c1")], &[]).expect("fence write");

    let after = read(&path);
    assert!(after.starts_with("ssh-rsa AAAAtheirs me@laptop\n"), "their key was joined: {after:?}");
    assert_eq!(fence::parse(&after).expect("parse").len(), 1);
    assert_eq!(after.lines().filter(|l| l.starts_with("ssh-")).count(), 1);
}

/// A `.ssh` that is a symlink is not a `.ssh` this will write through.
///
/// `O_NOFOLLOW` on the final path alone guards only that component: an
/// attacker who replaces the directory redirects the write somewhere they can
/// read, which for this file means handing them a key that logs in here.
#[test]
fn a_symlinked_ssh_directory_refuses() {
    let home = tempfile::tempdir().expect("tempdir");
    let elsewhere = home.path().join("elsewhere");
    std::fs::create_dir(&elsewhere).expect("mkdir");
    std::os::unix::fs::symlink(&elsewhere, home.path().join(".ssh")).expect("symlink");
    let path = home.path().join(".ssh").join("authorized_keys");

    let refused = fence::write(&path, AUTHORIZED_KEYS, &[entry(KEY, "c1")], &[]);

    assert!(refused.is_err(), "a symlinked .ssh was written through");
    assert!(!elsewhere.join("authorized_keys").exists(), "it wrote to the symlink's target");
    assert_eq!(std::fs::read_dir(&elsewhere).expect("read_dir").count(), 0, "it left something");
}

/// A file whose fence cannot be understood is left exactly as it is.
///
/// Refusing loses a feature. Guessing where an unterminated block ends
/// rewrites lines Far Cooler did not write.
#[test]
fn a_damaged_fence_refuses_and_changes_nothing() {
    let (_home, path) = a_runner();
    let before =
        format!("{}\n{}\nssh-rsa AAAAtheirs me@laptop\n", fence::BEGIN, entry(KEY, "c1"));
    std::fs::write(&path, &before).expect("write");

    let refused = fence::write(&path, AUTHORIZED_KEYS, &[entry(KEY, "c2")], &[]);

    assert!(matches!(refused, Err(FenceError::Damaged(_))), "damage was accepted: {refused:?}");
    assert_eq!(read(&path), before, "a damaged file was modified anyway");
}

/// Whatever else happens, the file as it was is still on disk.
///
/// With a checksum beside it, because a backup nobody can tell is intact is
/// not evidence — and the format is `shasum -c`'s, so recovering does not
/// need Far Cooler to be running or even installed.
#[test]
fn a_backup_is_left_beside_the_file() {
    let (_home, path) = a_runner();
    let before = format!("ssh-rsa AAAAtheirs me@laptop\n{}\n{}\n", fence::BEGIN, fence::END);
    std::fs::write(&path, &before).expect("write");

    fence::write(&path, AUTHORIZED_KEYS, &[entry(KEY, "c1")], &[]).expect("fence write");

    let backup = path.with_file_name("authorized_keys.farcooler-backup");
    assert_eq!(std::fs::read_to_string(&backup).expect("backup"), before);

    let sums = path.with_file_name("authorized_keys.farcooler-backup.sha256");
    let sum = std::fs::read_to_string(sums).expect("checksum");
    let expected = {
        use sha2::Digest as _;
        let mut hasher = sha2::Sha256::new();
        hasher.update(before.as_bytes());
        hasher.finalize().iter().map(|b| format!("{b:02x}")).collect::<String>()
    };
    assert!(sum.starts_with(&expected), "checksum {sum:?} is not of the backed-up bytes");
    assert!(sum.contains("authorized_keys.farcooler-backup"), "checksum names no file: {sum:?}");
}

/// Two writers produce one of the two files, never a mixture of both.
///
/// Read-modify-write on a shared file is a lost update at best; here the lost
/// update is a device that Settings says is enrolled and that cannot connect.
#[test]
fn two_concurrent_writes_do_not_interleave() {
    let (_home, path) = a_runner();
    std::fs::write(&path, "ssh-rsa AAAAtheirs me@laptop\n").expect("write");

    let first = vec![entry(KEY, "aaa"), entry(OTHER_KEY, "bbb")];
    let second = vec![entry(KEY, "ccc")];
    let (one, two) = (path.clone(), path.clone());
    let (a, b) = (first.clone(), second.clone());
    let left = std::thread::spawn(move || fence::write(&one, AUTHORIZED_KEYS, &a, &[]));
    let right = std::thread::spawn(move || fence::write(&two, AUTHORIZED_KEYS, &b, &[]));
    left.join().expect("join").expect("fence write");
    right.join().expect("join").expect("fence write");

    let after = read(&path);
    let ids: Vec<String> =
        fence::parse(&after).expect("parse").into_iter().map(|e| e.client_id).collect();
    assert!(
        ids == ["aaa", "bbb"] || ids == ["ccc"],
        "the two writes interleaved: {ids:?} in {after:?}"
    );
    assert!(after.starts_with("ssh-rsa AAAAtheirs me@laptop\n"), "their key was lost: {after:?}");
}

/// A fence written into a file that had none, next to what was already there.
#[test]
fn a_file_with_no_fence_at_all_gets_one() {
    let (_home, path) = a_runner();
    let mut file = std::fs::File::create(&path).expect("create");
    file.write_all(b"ssh-rsa AAAAtheirs me@laptop\n").expect("write");
    drop(file);

    fence::write(&path, AUTHORIZED_KEYS, &[entry(KEY, "c1")], &[]).expect("fence write");

    let after = read(&path);
    assert_eq!(fence::parse(&after).expect("parse").len(), 1);
    assert!(after.contains(fence::BEGIN) && after.contains(fence::END));
}
