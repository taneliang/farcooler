//! What a rep may search and read of its checkout (ov-555, R-74): the files
//! git tracks, less `.env*`, and nothing else.
//!
//! **Threat model.** A rep's model can be steered by a teammate's message, a
//! card, or a line in a file it reads, so it may ask for anything. The
//! checkout's files and its git config are the repository's, which can be
//! hostile. A rep must not reach `.git`, a `.env*` file, or a file git
//! ignores (a key, a token file, a `.npmrc`), and must not run a program.
//! So the answer never depends on a name the model chose or a walk of the
//! disk: `git grep` and `git ls-files` read the index, which lists the
//! tracked files, under the same pins, exec allowlist and deadline as every
//! other git the daemon runs (`git_capped`, `git_guard`). A symbolic link is
//! its target's text in the index and never followed (its final part; a
//! directory swapped for a link by a writer in the checkout is that writer's
//! doing); an untracked or ignored file isn't in the index, so no pattern or
//! path reaches it. The index is what git tracks now: a file `git add -f`ed
//! by another writer in the checkout is tracked, and so reachable.
//!
//! The pattern is passed after `-e`, so it can't be an option; the path is a
//! `:(literal)` pathspec, so no pathspec magic in it (`:(top)`, `*`, `:/`)
//! means anything. The output is capped in bytes and in matches per file, and
//! control characters are dropped, since a matched line is a repository's
//! text on its way to a model and a terminal.

use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{self as pb, request, result};
use tokio::time::Instant;

use crate::service::Service;

/// The most a search prints, in bytes.
pub const MOST_BYTES: usize = 48 * 1024;
/// The most matches one file prints.
const MOST_PER_FILE: &str = "--max-count=20";
/// The longest pattern, in bytes.
const LONGEST_PATTERN: usize = 1000;
/// The time one search, or one is-it-tracked check, may take.
pub const TIME: Duration = Duration::from_secs(20);

/// Why a search wasn't run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    Pattern,
    Path,
}

/// What a search printed.
#[derive(Debug, PartialEq, Eq)]
pub struct Found {
    pub text: String,
    pub cut: bool,
}

/// `path` as the plain parts it names under the checkout, or `None` when it
/// isn't one: absolute, climbing out with `..`, or through `.git` or a
/// `.env*` part. Lexical only; the file system is asked later.
fn parts(path: &str) -> Option<PathBuf> {
    if path.contains('\0') {
        return None;
    }
    let mut out = PathBuf::new();
    for part in Path::new(path).components() {
        match part {
            Component::CurDir => {}
            Component::Normal(name) if !crate::rep_gate::private(&name.to_string_lossy()) => out.push(name),
            _ => return None,
        }
    }
    Some(out)
}

/// The `git grep` arguments for `pattern` under `path` (already `parts`), in
/// the order git needs them: its `-c` first, the pattern after `-e`, the
/// pathspecs after `--`.
fn grep_args(pattern: &str, path: &Path) -> Vec<String> {
    let mut args: Vec<String> = [
        // Outside a repository `git grep` would fall back to searching every
        // file in the directory, untracked ones included.
        "-c",
        "grep.fallbackToNoIndex=false",
        "grep",
        "-n",
        "-I",
        "-E",
        "--no-color",
        "--no-column",
        "--no-recurse-submodules",
        "--no-textconv",
        MOST_PER_FILE,
        "-e",
    ]
    .map(String::from)
    .to_vec();
    args.push(pattern.to_string());
    args.push("--".into());
    if !path.as_os_str().is_empty() {
        args.push(format!(":(literal){}", path.display()));
    }
    // With no positive pathspec these mean "everything but".
    for private in [".env*", ".env*/**", ".git/**"] {
        args.push(format!(":(exclude,icase,glob)**/{private}"));
    }
    args
}

/// Search `checkout`'s tracked files for the extended regular expression
/// `pattern`, under `path` (relative to it; empty is all of it).
pub async fn search(deadline: Instant, checkout: &Path, pattern: &str, path: &str) -> Result<Found> {
    let refuse = |what| DomainError::InvalidArgument { what };
    if pattern.is_empty() || pattern.len() > LONGEST_PATTERN || pattern.contains(['\0', '\n', '\r']) {
        return Err(refuse("pattern"));
    }
    let under = parts(path).ok_or(refuse("path"))?;
    if !under.as_os_str().is_empty() && !stays_inside(checkout, &under) {
        return Err(refuse("path"));
    }
    let args = grep_args(pattern, &under);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    // 1 is "nothing matched".
    let out = crate::git_capped::git_capped_ok(deadline, checkout, &args, MOST_BYTES, &[0, 1])
        .await
        .ok_or(DomainError::OperationFailed)?;
    Ok(found(&out.stdout, out.cut))
}

/// Whether `under`, joined to the checkout, is really there and really inside
/// it once links are followed.
fn stays_inside(checkout: &Path, under: &Path) -> bool {
    let (Ok(root), Ok(real)) = (std::fs::canonicalize(checkout), std::fs::canonicalize(checkout.join(under))) else {
        return false;
    };
    real.starts_with(root)
}

/// The printed lines, whole ones only when the cap cut the last, with
/// control, bidirectional and zero-width characters other than a tab dropped.
fn found(stdout: &[u8], cut: bool) -> Found {
    let mut bytes = stdout;
    if cut {
        bytes = bytes.iter().rposition(|b| *b == b'\n').map_or(&[][..], |end| &bytes[..end + 1]);
    }
    let text = String::from_utf8_lossy(bytes)
        .chars()
        .filter(|c| matches!(c, '\n' | '\t') || !(c.is_control() || hides_text(*c)))
        .collect();
    Found { text, cut }
}

/// A format character that can make a line read differently from what it
/// holds: zero-width, bidirectional overrides and isolates, the byte-order
/// mark.
fn hides_text(c: char) -> bool {
    matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{206f}' | '\u{feff}')
}

/// Whether `relative` (a path under `checkout`, links already resolved) is a
/// file git tracks: the index lists exactly it.
///
/// `None` when git couldn't be asked (it wasn't found, a pin failed, the
/// deadline passed), so the gate can say so rather than "not tracked".
pub async fn tracked(deadline: Instant, checkout: &Path, relative: &Path) -> Option<bool> {
    let Some(name) = relative.to_str().filter(|n| !n.is_empty() && !n.contains('\0')) else { return Some(false) };
    let spec = format!(":(literal){name}");
    let args = ["-c", "core.quotePath=false", "ls-files", "-z", "--cached", "--", spec.as_str()];
    let out = crate::git_capped::git_capped(deadline, checkout, &args, 4096 + name.len()).await?;
    Some(out.stdout.split(|b| *b == 0).filter(|p| !p.is_empty()).eq([name.as_bytes()]))
}

/// [`tracked`] where no async runtime is in hand to wait on.
pub fn tracked_blocking(checkout: &Path, relative: &Path) -> Option<bool> {
    let (checkout, relative) = (checkout.to_path_buf(), relative.to_path_buf());
    let wait = async move { tracked(Instant::now() + TIME, &checkout, &relative).await };
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| handle.block_on(wait)),
        Err(_) => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .ok()
            .and_then(|rt| rt.block_on(wait)),
    }
}

/// `worktree.grep`: the checkout is the one the calling pane runs in, read
/// from its process (`people::rep_pane_of`) and never from the request. A
/// caller that isn't in a rep's pane is refused: this is a rep's search, not a
/// second way for any client to read code.
pub async fn handle(svc: &Service, caller_pid: Option<i32>, req: pb::Request) -> Result<result::Value> {
    let Some(request::Payload::WorktreeGrep(p)) = req.payload else {
        return Err(DomainError::InvalidArgument { what: "payload" });
    };
    let pane = caller_pid
        .and_then(|pid| crate::people::rep_pane_of(svc, pid))
        .ok_or(DomainError::Conflict { what: "rep_only" })?;
    let checkout = PathBuf::from(svc.store.get_worktree(svc.store.get_terminal(pane)?.worktree_id)?.worktree_path);
    let found = search(Instant::now() + TIME, &checkout, &p.pattern, &p.path).await?;
    Ok(result::Value::WorktreeGrepResult(pb::WorktreeGrepResult { text: found.text, cut: found.cut }))
}

#[cfg(test)]
#[path = "repo_grep_tests.rs"]
mod tests;
