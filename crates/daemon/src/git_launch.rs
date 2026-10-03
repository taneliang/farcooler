//! The programs the daemon starts on a repository's behalf, git and gh, each
//! by absolute path and inside its exec allowlist (`crate::git_sandbox`), and
//! when to resolve them again.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use farcooler_core::{DomainError, Result};

use crate::git::absolute_git;

/// A program the daemon starts on a repository's behalf, by absolute path,
/// and the exec allowlist it runs inside (`crate::git_sandbox`).
///
/// `sandbox` is `None` where the host can't confine a process; the program
/// then runs under `crate::git_guard`'s pins alone, which the sandbox module
/// has already said in the log.
#[derive(Debug)]
pub struct Launch {
    pub program: PathBuf,
    pub sandbox: Option<Arc<crate::git_sandbox::Sandbox>>,
}

impl Launch {
    /// A command for `program`, confined to its allowlist.
    pub fn command(&self) -> Result<std::process::Command> {
        let mut cmd = std::process::Command::new(&self.program);
        if let Some(sandbox) = &self.sandbox {
            sandbox.confine(&mut cmd).map_err(|e| {
                tracing::warn!(error = %e, "could not confine git to its exec allowlist");
                DomainError::OperationFailed
            })?;
        }
        Ok(cmd)
    }
}

/// The git `git_bytes` runs, and what it may run in turn.
///
/// Never by bare name. A bare name is looked up on `PATH` at spawn, after
/// the child has moved into the worktree, so a relative `PATH` entry (`.`,
/// or the empty one a stray `:` makes) finds a `git` the agent put there.
/// `programs::find` answers from absolute directories only, once.
///
/// The program is the real git behind the one found (`git_sandbox::real_git`;
/// on a Mac that skips the `/usr/bin/git` shim), and the allowlist is that
/// git, the one found, and the daemon's LFS filter (`crate::git_lfs`).
/// Resolved once, and again when the program is gone or the git found now
/// resolves elsewhere (`brew upgrade git` moves the Cellar path its symlink
/// names, and may leave the old one behind).
pub fn git_launch() -> Result<Arc<Launch>> {
    static CACHE: Mutex<Option<(Arc<Launch>, crate::git_sandbox::Resolved)>> = Mutex::new(None);
    let mut cache = CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((launch, _)) = cache.as_ref().filter(|(l, r)| l.program.exists() && r.unchanged()) {
        return Ok(launch.clone());
    }
    let found = PathBuf::from(absolute_git()?);
    let program = crate::git_sandbox::real_git(&found);
    let mut programs = vec![program.clone(), found.clone()];
    programs.extend(crate::git_lfs::helper().map(|h| h.program.clone()));
    let sandbox = crate::git_sandbox::Sandbox::new(&program, crate::git_sandbox::allowlist(&programs));
    let launch = Arc::new(Launch { program, sandbox });
    *cache = Some((launch.clone(), crate::git_sandbox::Resolved::of(&[found])));
    Ok(launch)
}

/// gh, and what it may run: git and ssh as gh will find them on the `PATH`
/// it's handed (`git remote -v`, `git config`, and `ssh -G <host>` for a
/// remote URL's host alias), and on a Mac `/usr/bin/security`, which reads
/// gh's token from the login keychain. `None` when there's no gh, or no git.
///
/// gh is started by the path found, not the one it resolves to: a multicall
/// shim (mise's, snap's) dispatches on `argv[0]`, and started as itself it
/// reads `pr list` as its own command. Both paths are on the list. Such a
/// shim starts the real gh, which isn't, so it still fails there, closed;
/// the daemon says so once.
///
/// Resolved again when any of gh, git or ssh found on the `PATH` now
/// resolves elsewhere, or git's own launch did: an upgrade that moves a
/// Cellar path would otherwise leave gh unable to start the new git.
pub fn gh_launch() -> Option<Arc<Launch>> {
    type Cached = (Arc<Launch>, Arc<Launch>, crate::git_sandbox::Resolved);
    static CACHE: Mutex<Option<Cached>> = Mutex::new(None);
    let git = git_launch().ok()?;
    let mut cache = CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((launch, _, _)) =
        cache.as_ref().filter(|(l, g, r)| l.program.exists() && Arc::ptr_eq(g, &git) && r.unchanged())
    {
        return Some(launch.clone());
    }
    let program = farcooler_core::programs::find("gh")?;
    if program.canonicalize().is_ok_and(|real| real.file_name().is_some_and(|n| n != "gh")) {
        tracing::warn!(
            gh = %program.display(),
            "gh is a shim for another program, which may start a gh that isn't on its exec allowlist"
        );
    }
    let path = crate::git_guard::child_path().unwrap_or_else(|| "/usr/bin:/bin".into());
    let (path_git, path_ssh) =
        (crate::git_sandbox::on_path("git", &path), crate::git_sandbox::on_path("ssh", &path));
    let mut programs: Vec<PathBuf> = vec![program.clone()];
    programs.extend(path_git.clone());
    programs.extend(git.sandbox.as_ref().map(|s| s.allowed().to_vec()).unwrap_or_else(|| vec![git.program.clone()]));
    programs.extend(path_ssh.clone());
    if cfg!(target_os = "macos") {
        programs.push(PathBuf::from("/usr/bin/security"));
    }
    let sandbox = crate::git_sandbox::Sandbox::new(&program, crate::git_sandbox::allowlist(&programs));
    let watched: Vec<PathBuf> = [Some(program.clone()), path_git, path_ssh].into_iter().flatten().collect();
    let launch = Arc::new(Launch { program, sandbox });
    *cache = Some((launch.clone(), git, crate::git_sandbox::Resolved::of(&watched)));
    Some(launch)
}
