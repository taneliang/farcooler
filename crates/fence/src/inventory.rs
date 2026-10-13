//! A read-only inventory of every key sshd would accept from `authorized_keys`
//! and `authorized_keys2`, inside Far Cooler's fence and outside it (ov-510,
//! design ov-480 §3).
//!
//! The fence is the part of the file Far Cooler writes. sshd obeys the whole
//! file, so a key added by hand above the block, or planted later by a user
//! with a shell, reaches the runner just as surely and is invisible to
//! `parse`. This lists all of them so the directory can say which belong to
//! whom.
//!
//! **Nothing here writes.** It does not take the lock, make a backup, create
//! `.ssh`, or touch the file's mode or times: it opens the directory without
//! `mkdir`, opens each file `O_RDONLY|O_NOFOLLOW`, and reads at most
//! `MAX_FILE_BYTES`. It never fails: whatever it could not read is named in
//! `Inventory::error`, so a runner that cannot be read says so rather than
//! going quiet (a damaged fence still lists every key, all as outside it).
//!
//! The fingerprint is `SHA256` of the key blob, whatever the key type, exactly
//! what `ssh-keygen -lf` prints. Fields are split on spaces and tabs, as sshd
//! does. Only a line inside the fence that Far Cooler wrote carries a Far
//! Cooler scope; anywhere else the forced command is just text, so it is
//! reported as `other` with its options beside it.

use std::path::Path;

use sha2::Digest as _;

use super::{entry_from_line, fence_span, scope_word, split_options, FenceError, Markers};
use farcooler_protocol::v1::Scope;

/// The most of either file this reads. Over it, the inventory says
/// `too_large` and lists nothing from that file.
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;

/// One key in the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    /// `SHA256:…`, as `ssh-keygen -lf` prints it.
    pub fingerprint: String,
    /// The key's comment. Whatever the line says; nothing authenticates it.
    pub comment: String,
    /// Whether the line sits between the fence markers.
    pub inside: bool,
    /// What the line grants through Far Cooler: `read`, `control`,
    /// `host_admin`, or `shell` for a Key B line of ours, written inside the
    /// fence. `ca` for a `cert-authority` line, which trusts every certificate
    /// its key signs. `other` for everything else, wherever it sits.
    pub scope: &'static str,
    /// Which file: `authorized_keys` or `authorized_keys2`.
    pub file: &'static str,
    /// The options field before the key, verbatim (`from=…`, `command=…`),
    /// empty when there is none.
    pub options: String,
}

/// Every key sshd would accept, in file order, primary file first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inventory {
    pub entries: Vec<Listed>,
    /// Lines that are not blank, not comments, and not a key this could read
    /// (a malformed line, a damaged blob). Counted so a runner with one is not
    /// reported as clean; their text is never sent.
    pub unparsed: u32,
    /// Why part or all of it could not be read: `damaged` (the fence markers
    /// are wrong; every key is listed as outside), `refused` (the directory or
    /// file is not the runner user's own, or is writable by others),
    /// `too_large`, or `unreadable`.
    pub error: Option<&'static str>,
    /// The file `error` is about: `authorized_keys`, or `authorized_keys2`
    /// when the primary file was read in full and only that one was not.
    pub error_file: Option<&'static str>,
}

impl Inventory {
    /// A fingerprint of the whole inventory, order-independent: the same set
    /// of keys hashes the same however the file is arranged. This is what a
    /// heartbeat carries in place of the list.
    pub fn digest(&self) -> String {
        let mut lines: Vec<String> = self
            .entries
            .iter()
            .map(|e| {
                format!("{}\t{}\t{}\t{}\t{}\t{}", e.fingerprint, u8::from(e.inside), e.scope, e.file, e.comment, e.options)
            })
            .collect();
        lines.sort();
        let mut hash = sha2::Sha256::new();
        hash.update(format!(
            "unparsed {} error {} {}\n",
            self.unparsed,
            self.error.unwrap_or(""),
            self.error_file.unwrap_or("")
        ));
        for line in &lines {
            hash.update(line.as_bytes());
            hash.update(b"\n");
        }
        hash.finalize().iter().map(|b| format!("{b:02x}")).collect()
    }
}

const FILE: &str = "authorized_keys";
const FILE2: &str = "authorized_keys2";

/// The inventory of the primary file's contents. Pure.
pub fn inventory_of(contents: &str, markers: Markers<'_>) -> Inventory {
    let mut found = Inventory::default();
    scan(contents, Some(markers), FILE, &mut found);
    found
}

/// Whether `token` names a key type sshd would take: plain, certificate, or
/// security-key (`ssh-…`, `ecdsa-sha2-…`, `sk-…`).
fn is_key_type(token: &str) -> bool {
    token.starts_with("ssh-") || token.starts_with("ecdsa-sha2-") || token.starts_with("sk-")
}

fn is_blank(c: char) -> bool {
    c == ' ' || c == '\t'
}

/// The next field and what follows it, splitting on spaces and tabs.
fn field(text: &str) -> (&str, &str) {
    let text = text.trim_start_matches(is_blank);
    match text.find(is_blank) {
        Some(at) => (&text[..at], text[at..].trim_start_matches(is_blank)),
        None => (text, ""),
    }
}

fn unbase64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut bits, mut have) = (0u32, 0u32);
    let body = text.trim_end_matches('=');
    if text.len() - body.len() > 2 {
        return None;
    }
    for c in body.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        bits = (bits << 6) | u32::from(v);
        have += 6;
        if have >= 8 {
            have -= 8;
            out.push((bits >> have) as u8);
            bits &= (1 << have) - 1;
        }
    }
    Some(out)
}

fn base64_unpadded(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16) | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8) | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..=chunk.len() {
            out.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

/// A key line as sshd reads it: the options before the key type, the key
/// type, the base64 blob (whose own first string must be that type), then the
/// comment. `None` for anything sshd would skip as malformed.
struct Parsed<'a> {
    options: &'a str,
    fingerprint: String,
    comment: &'a str,
}

fn parse_line(line: &str) -> Option<Parsed<'_>> {
    let line = line.trim_matches(|c| is_blank(c) || c == '\r');
    let (first, _) = field(line);
    let (options, rest) = if is_key_type(first) { ("", line) } else { split_options(line)? };
    let (key_type, rest) = field(rest);
    if !is_key_type(key_type) {
        return None;
    }
    let (blob, comment) = field(rest);
    let bytes = unbase64(blob)?;
    let named = bytes.get(..4).map(|n| u32::from_be_bytes([n[0], n[1], n[2], n[3]]) as usize)?;
    if bytes.get(4..4 + named)? != key_type.as_bytes() {
        return None;
    }
    let digest = sha2::Sha256::digest(&bytes);
    Some(Parsed { options, fingerprint: format!("SHA256:{}", base64_unpadded(&digest)), comment })
}

/// Whether the options field has `name` as one of its comma-separated
/// options (quote-aware: `command="a,cert-authority"` has none).
fn has_option(options: &str, name: &str) -> bool {
    let (mut quoted, mut start) = (false, 0);
    let hit = |piece: &str| piece.trim().eq_ignore_ascii_case(name);
    for (at, c) in options.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => {
                if hit(&options[start..at]) {
                    return true;
                }
                start = at + 1;
            }
            _ => {}
        }
    }
    hit(&options[start..])
}

/// Add every key in `contents` to `found`. `markers` is `None` for a file
/// Far Cooler never writes (`authorized_keys2`), where nothing is inside.
fn scan(contents: &str, markers: Option<Markers<'_>>, file: &'static str, found: &mut Inventory) {
    let lines: Vec<&str> = contents.lines().collect();
    let span = match markers.map(|m| fence_span(&lines, m)) {
        Some(Ok(span)) => span,
        Some(Err(_)) => {
            found.error = Some("damaged");
            found.error_file = Some(FILE);
            None
        }
        None => None,
    };
    for (at, line) in lines.iter().enumerate() {
        let text = line.trim_matches(|c| is_blank(c) || c == '\r');
        if text.is_empty() || text.starts_with('#') {
            continue;
        }
        // The markers themselves are comments to sshd.
        let Some(parsed) = parse_line(line) else {
            found.unparsed += 1;
            continue;
        };
        let inside = span.is_some_and(|(begin, end)| at > begin && at < end);
        let scope = if has_option(parsed.options, "cert-authority") {
            "ca"
        } else if inside {
            // Only a line Far Cooler wrote carries its scope.
            // ...and only one whose options are exactly what `render` writes.
            // A forced command that merely mentions `--scope read` is someone
            // else's, whatever its flags say (`no-pty,command="$SSH_ORIGINAL_COMMAND
            // --client c --scope read"` runs whatever the caller asks).
            let entry = entry_from_line(line);
            let node = (!entry.node_key.is_empty()).then_some(entry.node_key.as_str());
            if entry.fingerprint != parsed.fingerprint || entry.client_id.is_empty() {
                "other"
            } else if entry.shell_access {
                if parsed.options.is_empty() { "shell" } else { "other" }
            } else if entry.scope == Scope::Unspecified
                || parsed.options != super::key_a_options(&entry.client_id, entry.scope, node)
            {
                "other"
            } else {
                scope_word(entry.scope)
            }
        } else {
            "other"
        };
        found.entries.push(Listed {
            fingerprint: parsed.fingerprint,
            comment: parsed.comment.to_string(),
            inside,
            scope,
            file,
            options: parsed.options.to_string(),
        });
    }
}

fn error_code(error: &FenceError) -> &'static str {
    match error {
        FenceError::Io(e) if e.kind() == std::io::ErrorKind::FileTooLarge => "too_large",
        FenceError::Io(e) if e.kind() == std::io::ErrorKind::PermissionDenied => "refused",
        FenceError::Damaged(_) => "damaged",
        _ => "unreadable",
    }
}

/// The inventory of the file at `path` and its `authorized_keys2` sibling,
/// without writing anything.
///
/// A file that isn't there, or a `.ssh` that isn't, is an empty inventory: the
/// ordinary state of a runner nobody has enrolled a device on. The same
/// ownership and mode checks as `read` apply, and a file they refuse is named
/// in `error` (`refused`), not skipped.
pub fn inventory(path: &Path, markers: Markers<'_>) -> Inventory {
    let mut found = Inventory::default();
    let (Some(name), Some(parent)) = (path.file_name(), path.parent()) else {
        found.error = Some("unreadable");
        found.error_file = Some(FILE);
        return found;
    };
    let directory = match super::open_dir(parent, false) {
        Ok(directory) => directory,
        Err(FenceError::Missing) => return found,
        Err(e) => {
            found.error = Some(error_code(&e));
            found.error_file = Some(FILE);
            return found;
        }
    };
    let mut read = |name: &std::ffi::OsStr, markers: Option<Markers<'_>>, file: &'static str| {
        match super::read_at_most(&directory, name, Some(MAX_FILE_BYTES)) {
            Ok(Some((text, _))) => scan(&text, markers, file, &mut found),
            Ok(None) => {}
            Err(e) if found.error.is_none() => {
                found.error = Some(error_code(&e));
                found.error_file = Some(file);
            }
            Err(_) => {}
        }
    };
    read(name, Some(markers), FILE);
    // sshd's default `AuthorizedKeysFile` names both.
    if name == FILE {
        read(std::ffi::OsStr::new(FILE2), None, FILE2);
    }
    found
}
