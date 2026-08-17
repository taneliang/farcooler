//! Far Cooler's block in `~/.ssh/config`, as the Mac app reaches it: through
//! the C boundary.
//!
//! `~/.ssh/config` is not a Far Cooler file. It is a file somebody may have been
//! hand-editing for years, and it is read by Zed, by git, by `scp` and by every
//! `ssh` anybody types — so getting a write wrong here does not break a feature,
//! it breaks all of those at once. That is why the bytes are written by
//! `farcooler_daemon::fence`, the same routine that owns `authorized_keys`, and
//! not by a second implementation in Swift.
//!
//! These tests are about the boundary rather than the writer. The writer's own
//! tests (`crates/daemon/tests/the_fence_cannot_lose_your_access.rs`) prove the
//! descriptor-anchored traversal, the lock, the fsync and the backup. What is
//! proven here is that Swift can reach it at all, that the block lands where
//! `ssh_config` needs it — **first**, because the file is first-match-wins per
//! keyword — and that a refusal arrives as a stable word rather than a Rust
//! error string. This repo renders error strings from these layers in Settings,
//! so a `std::io::Error` message crossing here is one `Text(error)` away from a
//! person's screen.

use std::ffi::CString;
use std::path::{Path, PathBuf};

use farcooler_client::ffi::farcooler_client_ssh_config_write;
// The markers come from the writer's own constants rather than being spelled
// again here. A test that spelled them itself would still pass on the day the
// client and the daemon stopped agreeing about what Far Cooler's block looks
// like — which is the one change that turns an existing fence into "a file with
// no fence" and enrolls a second block beneath the first.
use farcooler_fence::{BEGIN, END};
use serde_json::Value;

/// A home directory with a real `.ssh` in it, and the `config` inside it.
fn a_home() -> (tempfile::TempDir, PathBuf) {
    let home = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(home.path().join(".ssh")).expect("mkdir .ssh");
    let path = home.path().join(".ssh").join("config");
    (home, path)
}

/// Call the entry point the way the Mac app does: ask for the size, size a
/// buffer, call once.
///
/// Once, deliberately. This call has a side effect — it rewrites a file — so
/// the usual "call, discover it was short, call again" dance would perform the
/// write twice and the second write's backup would be a copy of the first
/// write's output rather than of the file the person had. The entry point is
/// what makes one call enough: a sizing call touches nothing.
fn write_entries_json(path: &Path, entries_json: &str) -> Value {
    let c_path = CString::new(path.to_str().expect("utf-8 path")).expect("path");
    let entries = CString::new(entries_json).expect("entries");

    let needed = unsafe {
        farcooler_client_ssh_config_write(
            c_path.as_ptr(),
            entries.as_ptr(),
            std::ptr::null_mut(),
            0,
        )
    };
    assert!(needed > 0, "the entry point asked for no buffer at all");

    let mut buffer = vec![0u8; needed];
    let written = unsafe {
        farcooler_client_ssh_config_write(
            c_path.as_ptr(),
            entries.as_ptr(),
            buffer.as_mut_ptr(),
            buffer.len(),
        )
    };
    assert!(
        written > 0 && written <= needed,
        "{written} bytes into the {needed} it asked for"
    );
    serde_json::from_slice(&buffer[..written]).expect("the answer is json")
}

/// The same, for the ordinary case of a block composed line by line.
fn write_block(path: &Path, lines: &[&str]) -> Value {
    write_entries_json(path, &serde_json::to_string(&lines).expect("json"))
}

/// One runner's block, exactly as `apps/macos/.../SshConfig.swift` composes it:
/// one line per entry, no blank lines, no markers.
fn a_runner_block() -> Vec<&'static str> {
    vec![
        "Host box",
        "  HostName box.tail-1234.ts.net",
        "  User you",
        "  Port 22",
        "  IdentityFile ~/.ssh/farcooler-macbook-air",
        "  IdentitiesOnly yes",
    ]
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).expect("read back")
}

/// The word in a refusal, or a failure that says what arrived instead.
fn error(answer: &Value) -> String {
    answer
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("not a refusal: {answer}"))
        .to_string()
}

/// The block goes ABOVE an `Include`, and that is the whole point.
///
/// `ssh_config` is first-match-wins per keyword. A block appended below an
/// `Include ~/.ssh/config.d/*` or a `Host *` is silently overridden — the
/// `IdentityFile` Far Cooler wrote is never offered, ssh tries the wrong key,
/// and Zed reports an authentication failure for a runner that was enrolled
/// perfectly. Appending is the one placement that reliably does nothing.
#[test]
fn a_new_block_lands_above_an_existing_include() {
    let (_home, path) = a_home();
    let theirs = "Include ~/.ssh/config.d/*\n\nHost *\n  User someone\n";
    std::fs::write(&path, theirs).expect("write");

    let answer = write_block(&path, &a_runner_block());
    assert_eq!(answer, serde_json::json!({ "ok": true }), "the write was refused");

    let after = read(&path);
    let fence_at = after.find(BEGIN).expect("no fence was written");
    let include_at = after.find("Include").expect("their Include was lost");
    assert!(fence_at < include_at, "the block landed below the Include:\n{after}");
    assert!(after.ends_with(theirs), "their config was not carried through:\n{after}");
    assert!(after.contains("  IdentityFile ~/.ssh/farcooler-macbook-air"), "no key:\n{after}");
}

/// Every byte outside the fence comes back exactly as it was.
///
/// The lines above and below are somebody's own configuration, and one of them
/// may be the `Host github.com` every push in their day depends on.
#[test]
fn every_byte_outside_the_fence_survives() {
    let (_home, path) = a_home();
    let before = format!(
        "# my own notes\n\
         Host github.com\n  \tIdentityFile ~/.ssh/id_ed25519\n\
         \n\
         {BEGIN}\n\
         Host old\n  HostName old.example\n\
         {END}\n\
         Include ~/.ssh/work/*\n\
         Host *\n  ServerAliveInterval 60\n"
    );
    std::fs::write(&path, &before).expect("write");

    write_block(&path, &a_runner_block());

    let after = read(&path);
    // Compared as the bytes outside the two markers rather than line by line,
    // so an added space or a lost tab fails this too.
    let outside = |text: &str| -> (String, String) {
        let begin = text.find(BEGIN).expect("no fence");
        let end = text.find(END).expect("no closing marker") + END.len();
        (text[..begin].to_string(), text[end..].to_string())
    };
    assert_eq!(outside(&before), outside(&after), "a byte outside the fence changed");
    assert!(after.contains("Host box"), "the new block is not there:\n{after}");
    assert!(!after.contains("Host old"), "last time's block survived:\n{after}");
}

/// A fence that cannot be understood is refused, and the file is not touched.
///
/// The alternative is guessing where the block ends, and a wrong guess rewrites
/// lines Far Cooler did not write. Refusing costs the feature; guessing costs
/// somebody the `ssh` they use for work.
#[test]
fn a_damaged_fence_answers_damaged_and_changes_nothing() {
    let (_home, path) = a_home();
    // Two opening markers: an interrupted write, or a merge conflict resolved by
    // hand. Either way this is not a shape to write into.
    let before = format!("{BEGIN}\nHost a\n{BEGIN}\nHost b\n{END}\nHost theirs\n");
    std::fs::write(&path, &before).expect("write");

    let answer = write_block(&path, &a_runner_block());
    assert_eq!(error(&answer), "damaged", "a damaged fence was not reported as damaged");
    assert_eq!(read(&path), before, "a damaged file was rewritten anyway");
    // No backup either: nothing was attempted, so there is nothing beside the
    // file to explain and no `config.farcooler-backup` for a person to wonder at.
    assert!(!path.with_extension("farcooler-backup").exists(), "a refusal left a backup");
}

/// A refusal is a word, never a Rust error string.
///
/// `FenceError::Damaged` carries a sentence for the daemon's log — "3 opening
/// and 1 closing markers" — and it must stop at this boundary. What crosses is
/// `damaged`, and the app owns the sentence a person reads.
#[test]
fn a_refusal_carries_no_rust_error_string() {
    let (_home, path) = a_home();
    std::fs::write(&path, format!("{END}\nHost theirs\n")).expect("write");

    let answer = write_block(&path, &a_runner_block());
    assert_eq!(error(&answer), "damaged");
    let object = answer.as_object().expect("an object");
    assert_eq!(object.len(), 1, "a refusal carried more than a word: {answer}");
}

/// A path whose directory does not exist answers `missing`, not `io`.
///
/// Two levels down, because one level is not a failure: `fence` creates a
/// missing `.ssh` at 0700 as a side effect of anchoring the open, which is the
/// mode sshd's `StrictModes` wants anyway. What cannot be recovered from is a
/// path whose PARENT's parent is not there — which is what an app passing a home
/// directory it guessed wrong would produce.
#[test]
fn a_missing_parent_directory_answers_missing() {
    let (home, _) = a_home();
    let path = home.path().join("nowhere").join(".ssh").join("config");

    let answer = write_entries_json(&path, r#"["Host box"]"#);
    assert_eq!(error(&answer), "missing");
    assert!(!path.exists(), "a file appeared under a directory that does not exist");
    assert!(!home.path().join("nowhere").exists(), "a whole tree was created");
}

/// A path that names no file at all is `missing` rather than a panic.
#[test]
fn an_empty_path_answers_missing() {
    let answer = write_entries_json(Path::new(""), r#"["Host box"]"#);
    assert_eq!(error(&answer), "missing");
}

/// An entry carrying a newline is refused, and nothing is written.
///
/// One entry in must never be two lines out. `ssh_config` is line-oriented and
/// this block is written above everything else in the file, so a smuggled second
/// line is a `Host *` or an `IdentityFile` that wins every keyword — and nothing
/// about the resulting file is malformed, so the write would succeed and the
/// read-back would look fine.
#[test]
fn an_entry_carrying_a_newline_is_refused_and_the_file_is_untouched() {
    let (_home, path) = a_home();
    let before = "Host theirs\n  User them\n";
    std::fs::write(&path, before).expect("write");

    let answer = write_entries_json(&path, r#"["Host box\nHost *\n  IdentityFile /tmp/theirs"]"#);
    assert!(answer.get("ok").is_none(), "a newline was accepted: {answer}");
    assert_eq!(read(&path), before, "a refused write changed the file");
    assert!(!read(&path).contains(BEGIN), "a refused write left a fence behind");
}

/// An entry that IS a marker is refused for the same reason.
///
/// A block containing its own closing marker is a file whose fence this refuses
/// to read afterwards — which would mean no runner could be added or removed
/// again without somebody hand-editing `~/.ssh/config`.
#[test]
fn an_entry_that_is_a_marker_is_refused_and_the_file_is_untouched() {
    let (_home, path) = a_home();
    let before = "Host theirs\n";
    std::fs::write(&path, before).expect("write");

    for marker in [BEGIN, END] {
        let entries = serde_json::to_string(&vec!["Host box", marker]).expect("json");
        let answer = write_entries_json(&path, &entries);
        assert!(answer.get("ok").is_none(), "a marker was accepted as an entry: {answer}");
        assert_eq!(read(&path), before, "a refused write changed the file");
    }
}

/// A blank entry is refused too, and for a reason worth knowing.
///
/// The writer counts the non-empty lines it reads back against the entries it
/// was given, so a blank separator makes the file fail to read back as what was
/// written — and rather than writing a file it cannot verify, the writer refuses.
/// That is why `SshConfig.swift` composes one line per element and no blank ones:
/// a block "separated for readability" would never be written at all.
#[test]
fn a_blank_entry_is_refused_rather_than_written_unverifiably() {
    let (_home, path) = a_home();
    let before = "Host theirs\n";
    std::fs::write(&path, before).expect("write");

    let answer = write_block(&path, &["Host box", "", "Host other"]);
    assert!(answer.get("ok").is_none(), "a blank entry was accepted: {answer}");
    assert_eq!(read(&path), before, "a refused write changed the file");
}

/// An empty array removes the block, and leaves everything else.
///
/// A Mac with every runner un-enrolled should not carry two comment lines in
/// `~/.ssh/config` forever.
#[test]
fn an_empty_array_removes_the_block() {
    let (_home, path) = a_home();
    std::fs::write(&path, "Host theirs\n").expect("write");

    write_block(&path, &a_runner_block());
    assert!(read(&path).contains(BEGIN), "no block to remove");

    let answer = write_block(&path, &[]);
    assert_eq!(answer, serde_json::json!({ "ok": true }));
    let after = read(&path);
    assert!(!after.contains(BEGIN), "the markers outlived the block:\n{after}");
    assert!(!after.contains(END), "the closing marker outlived the block:\n{after}");
    assert_eq!(after, "Host theirs\n", "their line did not survive:\n{after}");
}

/// The buffer contract, including the part that only a side-effecting call has:
/// a call that cannot answer must not write the FILE either.
///
/// Every other entry point here is a pure function, so "nothing is written when
/// the buffer is short" means nothing is written into `out` and a caller simply
/// calls again. This one rewrites `~/.ssh/config`, and a second write's backup
/// is a copy of the first write's output — so a caller that sized its buffer
/// short would lose the copy of the file it had before Far Cooler touched it.
/// A short buffer therefore leaves the file alone as well.
#[test]
fn a_short_buffer_and_a_null_one_ask_for_the_size_and_touch_nothing() {
    let (_home, path) = a_home();
    let before = "Host theirs\n";
    std::fs::write(&path, before).expect("write");

    let c_path = CString::new(path.to_str().unwrap()).unwrap();
    let entries = CString::new(r#"["Host box"]"#).unwrap();

    // NULL asks for the size.
    let needed = unsafe {
        farcooler_client_ssh_config_write(
            c_path.as_ptr(),
            entries.as_ptr(),
            std::ptr::null_mut(),
            0,
        )
    };
    assert!(needed > 0);
    assert_eq!(read(&path), before, "a sizing call rewrote the file");

    // One byte short of what it asked for: nothing in the buffer, nothing in the
    // file, and the same size reported again.
    let mut buffer = vec![0xAAu8; needed];
    let again = unsafe {
        farcooler_client_ssh_config_write(
            c_path.as_ptr(),
            entries.as_ptr(),
            buffer.as_mut_ptr(),
            needed - 1,
        )
    };
    assert_eq!(again, needed, "a short call reported a different size");
    assert!(buffer.iter().all(|&b| b == 0xAA), "a short buffer was written into");
    assert_eq!(read(&path), before, "a short call rewrote the file");

    // And with the size it asked for, exactly one write.
    let written = unsafe {
        farcooler_client_ssh_config_write(
            c_path.as_ptr(),
            entries.as_ptr(),
            buffer.as_mut_ptr(),
            buffer.len(),
        )
    };
    assert!(written > 0 && written <= needed);
    assert!(read(&path).contains("Host box"), "the write did not happen");
}

/// Every answer fits the size the entry point asks for.
///
/// The size is a constant rather than a measurement, because measuring it would
/// mean performing the write to find out how long "ok" is. So the constant has
/// to cover the longest answer, and this is what says so — including for a word
/// somebody adds later.
#[test]
fn the_answer_never_outgrows_the_size_it_asks_for() {
    let (_home, path) = a_home();

    let needed = unsafe {
        let c_path = CString::new(path.to_str().unwrap()).unwrap();
        let entries = CString::new("[]").unwrap();
        farcooler_client_ssh_config_write(
            c_path.as_ptr(),
            entries.as_ptr(),
            std::ptr::null_mut(),
            0,
        )
    };

    // One case per word this can answer, each measured as JSON.
    for answer in [
        serde_json::json!({ "ok": true }),
        serde_json::json!({ "error": "damaged" }),
        serde_json::json!({ "error": "missing" }),
        serde_json::json!({ "error": "io" }),
    ] {
        let bytes = answer.to_string().len();
        assert!(bytes <= needed, "{answer} needs {bytes} bytes and callers size {needed}");
    }
}

/// Nothing is written for a payload that is not an array of lines.
///
/// A caller that sent an object, or an array of numbers, has a bug — and the
/// file it was about to rewrite is the one everybody's `ssh` reads.
#[test]
fn a_payload_that_is_not_an_array_of_lines_writes_nothing() {
    let (_home, path) = a_home();
    let before = "Host theirs\n";
    std::fs::write(&path, before).expect("write");

    for payload in [r#"{"entries":["Host box"]}"#, "[1,2,3]", "not json at all", ""] {
        let answer = write_entries_json(&path, payload);
        assert!(answer.get("ok").is_none(), "{payload} was accepted: {answer}");
        assert_eq!(read(&path), before, "{payload} changed the file");
    }
}
