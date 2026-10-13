//! The inventory of `authorized_keys` and `authorized_keys2` (ov-510): every
//! key sshd would accept, inside the fence and outside it, read without
//! writing a byte. The keys are made by the real `ssh-keygen`, in scratch
//! directories only, and every fingerprint is checked against what
//! `ssh-keygen -lf` prints.

use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use farcooler_fence::{self as fence, Listed, AUTHORIZED_KEYS};
use farcooler_protocol::v1::Scope;

/// A real key: its public line (`type blob comment`) and the fingerprint
/// `ssh-keygen -lf` prints for it.
struct Real {
    line: String,
    fingerprint: String,
}

fn fingerprint_of(dir: &Path, line: &str) -> String {
    let file = dir.join("check.pub");
    std::fs::write(&file, format!("{line}\n")).unwrap();
    let out = Command::new("ssh-keygen").arg("-lf").arg(&file).output().expect("ssh-keygen");
    assert!(out.status.success(), "ssh-keygen -lf refused {line}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap().split_whitespace().nth(1).unwrap().to_string()
}

fn real(dir: &Path, kind: &str, comment: &str) -> Real {
    let file = dir.join(format!("{kind}-{comment}"));
    let made = Command::new("ssh-keygen")
        .args(["-q", "-t", kind, "-N", "", "-C", comment, "-f"])
        .arg(&file)
        .output()
        .expect("ssh-keygen");
    assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stderr));
    let line = std::fs::read_to_string(format!("{}.pub", file.display())).unwrap().trim().to_string();
    Real { fingerprint: fingerprint_of(dir, &line), line }
}

/// A security-key blob (`sk-ssh-ed25519@openssh.com`), which `ssh-keygen`
/// cannot make without a device: built by hand, then fingerprinted by the real
/// `ssh-keygen -lf`.
fn sk(dir: &Path) -> Real {
    fn string(out: &mut Vec<u8>, bytes: &[u8]) {
        out.extend((bytes.len() as u32).to_be_bytes());
        out.extend(bytes);
    }
    let mut blob = Vec::new();
    string(&mut blob, b"sk-ssh-ed25519@openssh.com");
    string(&mut blob, &[7u8; 32]);
    string(&mut blob, b"ssh:");
    let b64 = {
        use std::io::Write as _;
        let mut child = Command::new("openssl")
            .args(["base64", "-A"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(&blob).unwrap();
        String::from_utf8(child.wait_with_output().unwrap().stdout).unwrap()
    };
    let line = format!("sk-ssh-ed25519@openssh.com {b64} yubikey");
    Real { fingerprint: fingerprint_of(dir, &line), line }
}

fn ours(key: &Real, client: &str, scope: Scope) -> String {
    let mut parts = key.line.splitn(3, ' ');
    let (kind, blob) = (parts.next().unwrap(), parts.next().unwrap());
    fence::render(&format!("{kind} {blob} x"), "iPhone", client, scope, fence::Grant::FarCooler, None).expect("render")
}

fn by_comment<'a>(found: &'a [Listed], comment: &str) -> &'a Listed {
    found.iter().find(|e| e.comment == comment).unwrap_or_else(|| panic!("no entry commented {comment}: {found:?}"))
}

/// Every key type sshd takes by default, plus a security key: all fingerprinted,
/// each equal to `ssh-keygen -lf`'s. An ECDSA key used to be only counted.
#[test]
fn every_key_type_is_fingerprinted_as_ssh_keygen_does() {
    let dir = tempfile::tempdir().unwrap();
    let keys = [real(dir.path(), "ed25519", "ed"), real(dir.path(), "ecdsa", "ec"), real(dir.path(), "rsa", "rs"), sk(dir.path())];
    let text: String = keys.iter().map(|k| format!("{}\n", k.line)).collect();
    let found = fence::inventory_of(&text, AUTHORIZED_KEYS);
    assert_eq!(found.unparsed, 0);
    assert_eq!(found.entries.len(), 4, "{:?}", found.entries);
    for (key, listed) in keys.iter().zip(&found.entries) {
        assert_eq!(listed.fingerprint, key.fingerprint, "{}", key.line);
    }
    assert!(found.entries[1].fingerprint.starts_with("SHA256:"));
}

/// sshd splits fields on spaces AND tabs. A planted key behind tabs used to be
/// only a rise in `unparsed`.
#[test]
fn tab_separated_fields_are_a_key() {
    let dir = tempfile::tempdir().unwrap();
    let key = real(dir.path(), "ed25519", "tabbed");
    let fields: Vec<&str> = key.line.split(' ').collect();
    let text = format!("{}\t{}\t\t{}\nno-pty\t{}\t{}\t{}\n", fields[0], fields[1], fields[2], fields[0], fields[1], "with options");
    let found = fence::inventory_of(&text, AUTHORIZED_KEYS);
    assert_eq!(found.unparsed, 0);
    assert_eq!(found.entries.len(), 2);
    assert_eq!(found.entries[0].fingerprint, key.fingerprint);
    assert_eq!(found.entries[0].comment, "tabbed");
    assert_eq!(found.entries[1].options, "no-pty");
    assert_eq!(found.entries[1].comment, "with options");
}

#[test]
fn a_cert_authority_line_is_scope_ca_wherever_it_sits() {
    let dir = tempfile::tempdir().unwrap();
    let key = real(dir.path(), "ed25519", "ca");
    let text = format!(
        "cert-authority,principals=\"deploy\" {}\n{}\ncert-authority {}\ncommand=\"echo cert-authority\" {}\n",
        key.line,
        fence::BEGIN,
        key.line,
        key.line
    ) + fence::END;
    let found = fence::inventory_of(&text, AUTHORIZED_KEYS);
    let scopes: Vec<(&str, bool)> = found.entries.iter().map(|e| (e.scope, e.inside)).collect();
    assert_eq!(scopes, [("ca", false), ("ca", true), ("other", true)]);
    assert!(found.entries[0].options.contains("principals"));
}

/// Far Cooler never writes outside the fence, so a forged forced command there
/// is just text: `other`, with its options beside it. Inside, it is a scope.
#[test]
fn a_forged_scope_outside_the_fence_is_other_with_its_options() {
    let dir = tempfile::tempdir().unwrap();
    let key = real(dir.path(), "ed25519", "forged");
    let mine = ours(&key, "dev-1", Scope::Control);
    let text = format!("{mine}\n{}\n{mine}\n{}\n", fence::BEGIN, fence::END);
    let found = fence::inventory_of(&text, AUTHORIZED_KEYS);
    assert_eq!(found.entries.len(), 2);
    let (outside, inside) = (&found.entries[0], &found.entries[1]);
    assert_eq!((outside.inside, outside.scope), (false, "other"));
    assert!(outside.options.contains("--scope control"), "{}", outside.options);
    assert_eq!((inside.inside, inside.scope), (true, "control"));
}

#[test]
fn comments_options_malformed_lines_and_foreign_lines_inside_the_fence() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b, c) = (real(dir.path(), "ed25519", "me@laptop"), real(dir.path(), "ecdsa", "ci@build"), real(dir.path(), "ed25519", "pasted"));
    let text = format!(
        "# keys I added myself\n{}\n\nfrom=\"10.0.0.0/8\",no-port-forwarding {}\nthis is not a key\nssh-ed25519 AAAAnotreal planted\n{}\n{}\n# a comment inside the block\n{}\n{}\n",
        a.line,
        b.line,
        fence::BEGIN,
        ours(&a, "dev-1", Scope::Read).replace(&a.line.split(' ').nth(1).unwrap().to_string(), c.line.split(' ').nth(1).unwrap()),
        c.line,
        fence::END,
    );
    let found = fence::inventory_of(&text, AUTHORIZED_KEYS);
    assert_eq!(found.error, None);
    assert_eq!(found.unparsed, 2, "the sentence and the bad blob are counted, never named");
    assert!(!by_comment(&found.entries, "me@laptop").inside);
    let ci = by_comment(&found.entries, "ci@build");
    assert_eq!((ci.fingerprint.as_str(), ci.options.as_str()), (b.fingerprint.as_str(), "from=\"10.0.0.0/8\",no-port-forwarding"));
    let pasted = by_comment(&found.entries, "pasted");
    assert_eq!((pasted.inside, pasted.scope), (true, "other"));
    assert!(found.entries.iter().any(|e| e.inside && e.scope == "read"), "{:?}", found.entries);
    assert!(found.entries.iter().all(|e| !e.comment.contains("planted")));
}

/// A damaged fence is reported, and still lists every key, all as outside it:
/// sshd reads the file the same either way.
#[test]
fn a_damaged_fence_is_named_and_every_key_is_still_listed() {
    let dir = tempfile::tempdir().unwrap();
    let key = real(dir.path(), "ed25519", "a");
    let found = fence::inventory_of(&format!("{}\n{}\n", fence::BEGIN, key.line), AUTHORIZED_KEYS);
    assert_eq!(found.error, Some("damaged"));
    assert_eq!(found.entries.len(), 1);
    assert!(!found.entries[0].inside);
    assert_ne!(found.digest(), fence::inventory_of(&format!("{}\n", key.line), AUTHORIZED_KEYS).digest(), "the error is part of the hash");
}

#[test]
fn the_digest_follows_the_keys_and_not_their_order() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (real(dir.path(), "ed25519", "a"), real(dir.path(), "ecdsa", "b"));
    let x = fence::inventory_of(&format!("{}\n{}\n", a.line, b.line), AUTHORIZED_KEYS);
    let y = fence::inventory_of(&format!("{}\n{}\n", b.line, a.line), AUTHORIZED_KEYS);
    assert_eq!(x.digest(), y.digest());
    let renamed = fence::inventory_of(&format!("{}\n{}\n", b.line, a.line.replace(" a", " renamed")), AUTHORIZED_KEYS);
    assert_ne!(x.digest(), renamed.digest());
    let moved = fence::inventory_of(&format!("{}\n{}\n{}\n{}\n", b.line, fence::BEGIN, a.line, fence::END), AUTHORIZED_KEYS);
    assert_ne!(x.digest(), moved.digest());
    let extra = fence::inventory_of(&format!("{}\n{}\nnot a key\n", a.line, b.line), AUTHORIZED_KEYS);
    assert_ne!(x.digest(), extra.digest());
    let with_options = fence::inventory_of(&format!("{}\nno-pty {}\n", a.line, b.line), AUTHORIZED_KEYS);
    assert_ne!(x.digest(), with_options.digest());
}

fn a_runner(text: &str) -> (tempfile::TempDir, PathBuf) {
    let home = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(home.path().join(".ssh")).expect("mkdir .ssh");
    let path = home.path().join(".ssh").join("authorized_keys");
    std::fs::write(&path, text).expect("write");
    (home, path)
}

/// `authorized_keys2` is in sshd's default `AuthorizedKeysFile`, so a key
/// planted there works. Listed read-only, labeled by file, never inside.
#[test]
fn authorized_keys2_is_read_too() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (real(dir.path(), "ed25519", "one"), real(dir.path(), "ecdsa", "two"));
    let (home, path) = a_runner(&format!("{}\n", a.line));
    let second = home.path().join(".ssh").join("authorized_keys2");
    std::fs::write(&second, format!("{}\n{}\n{}\n", fence::BEGIN, b.line, fence::END)).unwrap();
    let found = fence::inventory(&path, AUTHORIZED_KEYS);
    assert_eq!(found.error, None);
    let files: Vec<(&str, &str, bool)> = found.entries.iter().map(|e| (e.comment.as_str(), e.file, e.inside)).collect();
    assert_eq!(files, [("one", "authorized_keys", false), ("two", "authorized_keys2", false)]);
}

/// Reading never writes: the files' bytes, mode and times are the same after,
/// and nothing appears beside them (no lock, no backup).
///
/// Mutation: make `inventory` call `fence::write`, take the lock, or open the
/// directory with `create`, and this goes red.
#[test]
fn reading_the_inventory_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let key = real(dir.path(), "ed25519", "a");
    let (home, path) = a_runner(&format!("{}\n{}\n{}\n{}\n", key.line, fence::BEGIN, key.line, fence::END));
    let second = home.path().join(".ssh").join("authorized_keys2");
    std::fs::write(&second, format!("{}\n", key.line)).unwrap();
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    for file in [&path, &second] {
        std::fs::File::options().write(true).open(file).unwrap().set_modified(old).unwrap();
    }
    let snapshot = |file: &PathBuf| {
        let m = std::fs::metadata(file).unwrap();
        (std::fs::read(file).unwrap(), m.modified().unwrap(), m.mode())
    };
    let before = (snapshot(&path), snapshot(&second));
    let dir_before = std::fs::read_dir(home.path().join(".ssh")).unwrap().count();

    let found = fence::inventory(&path, AUTHORIZED_KEYS);
    assert_eq!(found.entries.len(), 3);

    assert_eq!((snapshot(&path), snapshot(&second)), before, "bytes, mtime and mode");
    assert_eq!(std::fs::read_dir(home.path().join(".ssh")).unwrap().count(), dir_before, "no lock or backup beside them");
}

#[test]
fn a_runner_with_no_ssh_directory_is_an_empty_inventory_and_stays_without_one() {
    let home = tempfile::tempdir().expect("tempdir");
    let path = home.path().join(".ssh").join("authorized_keys");
    assert_eq!(fence::inventory(&path, AUTHORIZED_KEYS), fence::Inventory::default());
    assert!(!home.path().join(".ssh").exists(), "looking must not make .ssh");
    std::fs::create_dir(home.path().join(".ssh")).unwrap();
    assert!(fence::inventory(&path, AUTHORIZED_KEYS).entries.is_empty());
    assert!(!path.exists());
}

/// A file over the cap is named, not read. An `.ssh` others can write is
/// refused, as `read` refuses it, and named.
#[test]
fn a_file_over_the_cap_or_a_writable_directory_is_an_error_not_silence() {
    let (_home, path) = a_runner(&"# padding\n".repeat(fence::MAX_FILE_BYTES as usize / 10 + 10));
    let found = fence::inventory(&path, AUTHORIZED_KEYS);
    assert_eq!((found.error, found.entries.len()), (Some("too_large"), 0));

    let (home, path) = a_runner("# fine\n");
    std::fs::set_permissions(home.path().join(".ssh"), std::os::unix::fs::PermissionsExt::from_mode(0o777)).unwrap();
    assert_eq!(fence::inventory(&path, AUTHORIZED_KEYS).error, Some("refused"));
}
