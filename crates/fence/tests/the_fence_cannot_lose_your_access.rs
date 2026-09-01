//! The one file Far Cooler edits that somebody's access already depends on.
//!
//! `~/.ssh/authorized_keys` is not a Far Cooler file. It is a file a person may
//! have been using for years, with keys in it that have nothing to do with this
//! program, and the failure mode of getting a write wrong is not a broken
//! feature — it is being locked out of your own runner, which
//! `docs/farcooler-design.md:1030` calls release-blocking. So the writer is
//! tested for what it must never do, not for what it does.
//!
//! (The line number was `:1017` here and in `fence.rs` for a while, which is the
//! paragraph about device keypairs. The sentence this file exists for is on 1030.)

use std::io::Write as _;
use std::path::{Path, PathBuf};

use farcooler_fence::{self as fence, AUTHORIZED_KEYS, FenceError};
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
    fence::render(
        &format!("ssh-ed25519 {key} x"),
        "iPhone",
        client_id,
        Scope::Control,
        fence::Grant::FarCooler,
        None,
    )
    .expect("render")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).expect("read back")
}

/// A line back out of an entry, the way every caller of `update` rebuilds one.
fn line(entry: &fence::Entry) -> String {
    entry.line.clone()
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

    fence::write(&path, AUTHORIZED_KEYS, &[entry(KEY, "new")], &[], fence::Placement::Last).expect("fence write");

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

    fence::write(&path, AUTHORIZED_KEYS, &[entry(KEY, "c1")], &[], fence::Placement::Last).expect("fence write");

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

    let refused = fence::write(&path, AUTHORIZED_KEYS, &[entry(KEY, "c1")], &[], fence::Placement::Last);

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

    let refused = fence::write(&path, AUTHORIZED_KEYS, &[entry(KEY, "c2")], &[], fence::Placement::Last);

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

    fence::write(&path, AUTHORIZED_KEYS, &[entry(KEY, "c1")], &[], fence::Placement::Last).expect("fence write");

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
    let left = std::thread::spawn(move || fence::write(&one, AUTHORIZED_KEYS, &a, &[], fence::Placement::Last));
    let right = std::thread::spawn(move || fence::write(&two, AUTHORIZED_KEYS, &b, &[], fence::Placement::Last));
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

/// Two enrollments in the same instant, and BOTH keys are still there.
///
/// This is the property `two_concurrent_writes_do_not_interleave` above does
/// NOT have, and the difference is the whole reason `update` exists. That test
/// proves the file is never a mixture of two writes. A lost update is not a
/// mixture: the loser's write is complete, well formed, and missing a key,
/// because it rebuilt the block from a snapshot taken before the winner's write
/// landed. The symptom is a device Settings says is enrolled and that cannot
/// connect — and onboarding a Mac is two enrollments of one client id, which is
/// the easiest way in the product to arrive here.
///
/// **The sleep inside the closure is what gives this test teeth.** Under
/// `update` it runs while the lock is held, so the second caller waits and then
/// reads a file that already has the first key in it. Move the read back
/// outside the lock and the same sleep makes both callers decide from the same
/// empty snapshot, deterministically, and one key is gone. A test that only
/// raced two threads would pass either way on most runs.
#[test]
fn two_concurrent_enrollments_both_survive() {
    let (_home, path) = a_runner();
    std::fs::write(&path, "ssh-rsa AAAAtheirs me@laptop\n").expect("write");

    // Released only when both threads are in position, so neither can finish
    // before the other has started and pass by accident.
    let together = std::sync::Arc::new(std::sync::Barrier::new(2));
    let enroll = |key: &'static str, client_id: &'static str| {
        let (path, together) = (path.clone(), together.clone());
        std::thread::spawn(move || {
            together.wait();
            fence::update(&path, AUTHORIZED_KEYS, fence::Placement::Last, |current| {
                // Exactly what `enrollment::enroll` does: keep what is there,
                // add one line. Our lines and theirs are told apart the same
                // way, so a foreign line inside the block is carried through.
                let (mut ours, foreign): (Vec<String>, Vec<String>) = (
                    current.iter().filter(|e| !e.client_id.is_empty()).map(line).collect(),
                    current.iter().filter(|e| e.client_id.is_empty()).map(line).collect(),
                );
                ours.push(entry(key, client_id));
                std::thread::sleep(std::time::Duration::from_millis(80));
                Ok::<_, FenceError>((fence::Change::Write { entries: ours, foreign }, ()))
            })
        })
    };
    let left = enroll(KEY, "aaa");
    let right = enroll(OTHER_KEY, "bbb");
    left.join().expect("join").expect("the first enrollment");
    right.join().expect("join").expect("the second enrollment");

    let after = read(&path);
    let ids: Vec<String> =
        fence::parse(&after).expect("parse").into_iter().map(|e| e.client_id).collect();
    assert!(
        ids.contains(&"aaa".to_string()) && ids.contains(&"bbb".to_string()),
        "an enrollment was lost, which is a device that cannot connect: {ids:?} in {after:?}"
    );
    assert_eq!(ids.len(), 2, "one enrollment wrote a line twice: {ids:?}");
    assert!(after.starts_with("ssh-rsa AAAAtheirs me@laptop\n"), "their key was lost: {after:?}");
}

/// A caller that decides nothing needs doing leaves the file untouched.
///
/// `client.enroll` on a device that is already enrolled answers and writes
/// nothing, and it reaches that decision from inside the lock now. "Nothing"
/// has to mean nothing: no rewrite, and no backup either, because a backup
/// written on a no-op would overwrite the one copy of the file as it was before
/// the last real change.
#[test]
fn a_change_that_leaves_the_file_alone_writes_nothing_at_all() {
    let (_home, path) = a_runner();
    let before = format!(
        "ssh-rsa AAAAtheirs me@laptop\n{}\n{}\n{}\n",
        fence::BEGIN,
        entry(KEY, "c1"),
        fence::END,
    );
    std::fs::write(&path, &before).expect("write");

    let seen: usize = fence::update(&path, AUTHORIZED_KEYS, fence::Placement::Last, |current| {
        Ok::<_, FenceError>((fence::Change::Leave, current.len()))
    })
    .expect("update");

    assert_eq!(seen, 1, "the closure was handed the wrong block");
    assert_eq!(read(&path), before, "a no-op rewrote the file");
    assert!(
        !path.with_file_name("authorized_keys.farcooler-backup").exists(),
        "a no-op left a backup, which would shadow the last real one"
    );
}

/// `update` writes through the same path a `write` does, backup and all.
///
/// Not a duplicate of the tests above: those call `write`, and the enrollment
/// path calls `update`. The refusals and the durability are worth exactly what
/// the function somebody's keys actually go through has, so the shared path is
/// asserted at the door that is used rather than inferred from the source.
#[test]
fn an_update_preserves_what_is_not_ours_and_leaves_a_backup() {
    let (_home, path) = a_runner();
    let before = format!(
        "ssh-rsa AAAAtheirs me@laptop\n{}\nssh-ed25519 {OTHER_KEY} added-by-hand\n{}\n# trailing\n",
        fence::BEGIN,
        fence::END,
    );
    std::fs::write(&path, &before).expect("write");

    fence::update(&path, AUTHORIZED_KEYS, fence::Placement::Last, |current| {
        // The hand-added line inside the block comes back as foreign, and it
        // goes back in: dropping it would delete somebody's key.
        assert_eq!(current.len(), 1, "the block was not handed over as it reads");
        assert!(current[0].client_id.is_empty(), "a hand-added line was claimed as ours");
        let foreign: Vec<String> = current.iter().map(line).collect();
        Ok::<_, FenceError>((
            fence::Change::Write { entries: vec![entry(KEY, "c1")], foreign },
            (),
        ))
    })
    .expect("update");

    let after = read(&path);
    assert!(after.starts_with("ssh-rsa AAAAtheirs me@laptop\n"), "their key moved: {after:?}");
    assert!(after.ends_with("# trailing\n"), "the bytes below the fence changed: {after:?}");
    assert!(after.contains("added-by-hand"), "a hand-added line inside the block was deleted");
    assert_eq!(fence::parse(&after).expect("parse").len(), 2);
    assert_eq!(
        std::fs::read_to_string(path.with_file_name("authorized_keys.farcooler-backup"))
            .expect("backup"),
        before,
        "the file as it was is not recoverable"
    );
    assert!(
        path.with_file_name("authorized_keys.farcooler-backup.sha256").exists(),
        "a backup nobody can tell is intact is not evidence"
    );
}

/// A symlinked `.ssh` is refused by `update` too, and for the same reason.
///
/// Asserted at this door as well as at `write`'s, because this is the one the
/// enrollment path goes through: a second directory-opening path that skipped
/// the anchoring would redirect somebody's keys into a directory an attacker
/// can read, and nothing about the resulting write would look wrong.
#[test]
fn an_update_through_a_symlinked_ssh_directory_refuses() {
    let home = tempfile::tempdir().expect("tempdir");
    let elsewhere = home.path().join("elsewhere");
    std::fs::create_dir(&elsewhere).expect("mkdir");
    std::os::unix::fs::symlink(&elsewhere, home.path().join(".ssh")).expect("symlink");
    let path = home.path().join(".ssh").join("authorized_keys");

    let refused = fence::update(&path, AUTHORIZED_KEYS, fence::Placement::Last, |_| {
        Ok::<_, FenceError>((
            fence::Change::Write { entries: vec![entry(KEY, "c1")], foreign: Vec::new() },
            (),
        ))
    });

    assert!(refused.is_err(), "a symlinked .ssh was written through");
    assert_eq!(std::fs::read_dir(&elsewhere).expect("read_dir").count(), 0, "it left something");
}

/// A file whose fence cannot be understood is refused before anyone decides.
///
/// The caller is never handed a block parsed out of damage, because there is no
/// honest block to hand it: guessing where an unterminated one ends is how a
/// rewrite deletes lines Far Cooler did not write. So the closure is not called
/// at all, and the file is not touched.
#[test]
fn a_damaged_fence_is_refused_before_the_caller_is_asked_to_decide() {
    let (_home, path) = a_runner();
    let before = format!("{}\n{}\nssh-rsa AAAAtheirs me@laptop\n", fence::BEGIN, entry(KEY, "c1"));
    std::fs::write(&path, &before).expect("write");

    let mut asked = false;
    let refused = fence::update(&path, AUTHORIZED_KEYS, fence::Placement::Last, |_| {
        asked = true;
        Ok::<_, FenceError>((fence::Change::Write { entries: Vec::new(), foreign: Vec::new() }, ()))
    });

    assert!(matches!(refused, Err(FenceError::Damaged(_))), "damage was accepted: {refused:?}");
    assert!(!asked, "a caller was asked to rewrite a block that could not be read");
    assert_eq!(read(&path), before, "a damaged file was modified anyway");
}

/// A fence written into a file that had none, next to what was already there.
#[test]
fn a_file_with_no_fence_at_all_gets_one() {
    let (_home, path) = a_runner();
    let mut file = std::fs::File::create(&path).expect("create");
    file.write_all(b"ssh-rsa AAAAtheirs me@laptop\n").expect("write");
    drop(file);

    fence::write(&path, AUTHORIZED_KEYS, &[entry(KEY, "c1")], &[], fence::Placement::Last).expect("fence write");

    let after = read(&path);
    assert_eq!(fence::parse(&after).expect("parse").len(), 1);
    assert!(after.contains(fence::BEGIN) && after.contains(fence::END));
}

/// `First` puts a new block above everything, which is what ssh_config needs.
///
/// `ssh_config` takes the FIRST value it obtains for each keyword, so a block
/// appended below an `Include` or a `Host *` is silently overridden — it is the
/// one placement that reliably does nothing. This is the property the Mac's
/// Zed-and-git access depends on, so it gets a test rather than a comment.
#[test]
fn a_first_placed_fence_goes_above_an_include() {
    let (_home, dir) = a_runner();
    let path = dir.parent().expect("parent").join("config");
    let mut file = std::fs::File::create(&path).expect("create");
    file.write_all(b"Include ~/.ssh/config.d/*\n\nHost *\n  User someone\n").expect("write");
    drop(file);

    let markers = fence::Markers { begin: fence::BEGIN, end: fence::END };
    let block = vec!["Host box".to_string(), "  HostName box.example".to_string()];
    fence::write(&path, markers, &block, &[], fence::Placement::First).expect("fence write");

    let after = read(&path);
    let fence_at = after.find(fence::BEGIN).expect("no fence");
    let include_at = after.find("Include").expect("their Include was lost");
    assert!(fence_at < include_at, "the fence landed below the Include:\n{after}");
    assert!(after.contains("Host *"), "their config was lost:\n{after}");
}

/// An existing block is rewritten where it already is, whatever the placement.
///
/// Moving someone's block because a rule says so is a surprise, and in
/// `ssh_config` it would silently change which keywords win.
#[test]
fn an_existing_fence_is_not_moved_by_a_placement() {
    let (_home, dir) = a_runner();
    let path = dir.parent().expect("parent").join("config");
    let markers = fence::Markers { begin: fence::BEGIN, end: fence::END };

    let mut file = std::fs::File::create(&path).expect("create");
    file.write_all(b"Host first\n").expect("write");
    drop(file);
    fence::write(&path, markers, &["Host box".to_string()], &[], fence::Placement::Last)
        .expect("first write");

    // Now ask for First. The block must stay where it is.
    fence::write(&path, markers, &["Host box2".to_string()], &[], fence::Placement::First)
        .expect("second write");

    let after = read(&path);
    let theirs = after.find("Host first").expect("their line was lost");
    let fence_at = after.find(fence::BEGIN).expect("no fence");
    assert!(theirs < fence_at, "the block was moved above their line:\n{after}");
}
