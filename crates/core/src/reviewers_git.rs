//! The git half of whom to ask (ov-580): reads what `reviewers::suggest` is
//! drawn from out of a repository. Run by `farcooler plan lane pr-body` in the
//! lane's directory and by the runner in the repository's checkout.
//!
//! **Threat model.** Hostile: a branch's own files (a pull request that edits
//! CODEOWNERS to name itself), commit emails (anyone can commit as anyone's
//! noreply address), and path names. So CODEOWNERS and the reviewer map are
//! read from `base`, never from the branch, as GitHub reads CODEOWNERS; a
//! forged commit email can only put a name in a *suggestion*, which a person
//! reads (nothing here posts or requests); and paths go after `--` as
//! `:(literal)` pathspecs, so a file named `:(top)x` is a file, not a pathspec.
//! Bounded: a file over a mebibyte is ignored, history reads at most 300
//! commits over at most 200 paths.

use std::path::Path;
use std::process::{Command, Stdio};

use crate::reviewers::{history_of, login_from_email, login_ok};

/// Where GitHub looks for CODEOWNERS, in its order.
const CODEOWNERS: [&str; 3] = [".github/CODEOWNERS", "CODEOWNERS", "docs/CODEOWNERS"];
/// The reviewer map: CODEOWNERS' shape, for asks GitHub doesn't enforce.
pub const MAP: &str = ".farcooler/reviewers";

const FILE_MAX: usize = 1 << 20;
const COMMITS: &str = "300";
const PATHS: usize = 200;

/// What `suggest` is drawn from, as read from git.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Gathered {
    pub codeowners: Option<String>,
    pub map: Option<String>,
    /// Commit counts by login over the touched files, as of `base`.
    pub history: Vec<(String, u32)>,
    /// Logins that wrote the branch's own commits, and the local git identity:
    /// whoever they are, they wrote this change and don't review it.
    pub committers: Vec<String>,
}

/// A ref git could read as a flag, or that is empty, is not asked about.
fn plain_ref(r: &str) -> bool {
    !r.is_empty() && !r.starts_with('-') && !r.contains(char::is_whitespace)
}

/// Runs `git <args>` and gives its stdout when it succeeded: where the git
/// comes from is the caller's. The CLI's is the git on its path; the runner's
/// is its guarded, sandboxed, time-bounded one (`crate::git`), so a repository's
/// own config (a planted `gpg.program`, a hook name) can't run anything.
pub type Run<'a> = dyn FnMut(&[&str]) -> Option<String> + 'a;

/// The git on the path, in `dir` (or the current directory).
fn path_git(dir: Option<&Path>, args: &[&str]) -> Option<String> {
    let mut cmd = Command::new("git");
    if let Some(dir) = dir {
        cmd.current_dir(dir);
    }
    cmd.args(["-c", "core.quotepath=false"]).args(args);
    let out = cmd.stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    (out.status.success() && out.stdout.len() <= FILE_MAX * 4).then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// A file of `base`'s tree, when it is there and not huge.
fn show(run: &mut Run, base: &str, path: &str) -> Option<String> {
    let text = run(&["show", &format!("{base}:{path}")])?;
    (text.len() <= FILE_MAX).then_some(text)
}

/// `gather` with the git of the caller's choosing. The subcommand comes first
/// in what `run` is given, as the daemon's guard expects; paths are literal
/// pathspecs (`:(literal)`), so a file named `:(top)x` is a file.
pub fn gather_with(run: &mut Run, base: &str, branch: &str, paths: &[String]) -> Gathered {
    if !plain_ref(base) {
        return Gathered::default();
    }
    let mut out = Gathered {
        codeowners: CODEOWNERS.iter().find_map(|p| show(run, base, p)),
        map: show(run, base, MAP),
        ..Default::default()
    };
    let specs: Vec<String> = paths.iter().take(PATHS).map(|p| format!(":(literal){p}")).collect();
    if !specs.is_empty() {
        let mut args: Vec<&str> = vec!["log", "--no-merges", "-n", COMMITS, "--format=%ae", base, "--"];
        args.extend(specs.iter().map(String::as_str));
        if let Some(log) = run(&args) {
            out.history = history_of(log.lines());
        }
    }
    let mut emails: Vec<String> = Vec::new();
    if plain_ref(branch)
        && let Some(log) = run(&["log", "--format=%ae", &format!("{base}..{branch}")])
    {
        emails.extend(log.lines().map(str::to_string));
    }
    if let Some(me) = run(&["config", "user.email"]) {
        emails.push(me.trim().to_string());
    }
    for login in emails.iter().filter_map(|e| login_from_email(e)) {
        if login_ok(&login) && !out.committers.iter().any(|c| c.eq_ignore_ascii_case(&login)) {
            out.committers.push(login);
        }
    }
    out
}

/// Read everything for a change to `paths` that `branch` makes on `base`,
/// with the git on the path. Anything git can't answer is left out, never
/// guessed.
pub fn gather(dir: Option<&Path>, base: &str, branch: &str, paths: &[String]) -> Gathered {
    gather_with(&mut |args| path_git(dir, args), base, branch, paths)
}
