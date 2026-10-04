//! gh's launch through a package manager's shim, and after a git upgrade
//! (ov-187): the two cases the exec allowlist could break gh on.
//!
//! Each test builds its own world in a temp dir (a `bin` on the `PATH` gh is
//! handed, symlinks into a fake Cellar) because the real `gh_launch` caches
//! for the process and reads the real `PATH`.

use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use super::*;

/// A real, non-script executable at `at`: a link to a system program, since a
/// macOS system binary copied elsewhere is killed on launch. Each test uses
/// a different one for each role, so what one role's list allows says
/// nothing about another's. Returns where it resolves.
fn program(at: &Path, system: &str) -> PathBuf {
    std::fs::create_dir_all(at.parent().unwrap()).unwrap();
    let system = ["/usr/bin", "/bin"].iter().map(|d| Path::new(d).join(system)).find(|p| p.exists()).unwrap();
    symlink(&system, at).unwrap();
    std::fs::canonicalize(at).unwrap()
}

/// git's own launch, as a stand-in: one program, no sandbox of its own.
fn git_at(at: &Path) -> Launch {
    Launch { program: program(at, "dirname"), sandbox: None }
}

/// Run `program` confined to `launch`'s list; whether it ran.
fn runs(launch: &Launch, program: &Path) -> bool {
    let mut cmd = std::process::Command::new(program);
    launch.sandbox.as_ref().expect("an exec sandbox on this host").confine(&mut cmd).unwrap();
    cmd.arg("hello").output().is_ok_and(|o| o.status.success())
}

/// gh found as a symlink to a differently named program, as mise and snap
/// install it. It is started by the path found, a multicall shim dispatching
/// on `argv[0]`, and both that and its target are on the list.
#[test]
fn gh_behind_a_shim_is_started_by_the_path_it_was_found_at() {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let target = program(&root.join("mise/shims/mise"), "basename");
    let gh = root.join("bin/gh");
    std::fs::create_dir_all(root.join("bin")).unwrap();
    symlink(&target, &gh).unwrap();
    let git = git_at(&root.join("git/git"));
    let (launch, watched) = build_gh_launch(gh.clone(), &git, root.join("bin").as_os_str());
    assert_eq!(launch.program, gh, "started as the shim, not as what it resolves to");
    assert_eq!(watched, std::slice::from_ref(&gh));
    let sandbox = launch.sandbox.as_ref().expect("an exec sandbox on this host");
    assert!(sandbox.allowed().contains(&gh), "the shim path: {:?}", sandbox.allowed());
    assert!(sandbox.allowed().contains(&target), "its target: {:?}", sandbox.allowed());
    assert!(runs(&launch, &launch.program), "gh through its shim starts under the sandbox");
}

/// `brew upgrade git` repoints the `git` on gh's `PATH` at a new Cellar
/// directory. A launch built before it doesn't allow the new git, so gh would
/// be unable to start it; the resolve notices and a rebuilt launch allows it.
#[test]
fn gh_still_starts_git_after_a_git_upgrade() {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let old = program(&root.join("Cellar/git/1/bin/git"), "echo");
    let new = program(&root.join("Cellar/git/2/bin/git"), "true");
    std::fs::create_dir_all(root.join("bin")).unwrap();
    let on_path = root.join("bin/git");
    symlink(&old, &on_path).unwrap();
    let gh = root.join("bin/gh");
    program(&gh, "basename");
    let git = git_at(&root.join("git/git"));
    let path = root.join("bin");

    let (before, watched) = build_gh_launch(gh.clone(), &git, path.as_os_str());
    let resolved = crate::git_sandbox::Resolved::of(&watched);
    assert!(resolved.unchanged(), "nothing has moved yet");
    assert!(runs(&before, &old), "the git gh finds today runs");
    assert!(!runs(&before, &new), "and the one after the upgrade doesn't, on the old list");

    std::fs::remove_file(&on_path).unwrap();
    symlink(&new, &on_path).unwrap();
    assert!(!resolved.unchanged(), "the upgrade is noticed, so the launch is rebuilt");
    let (after, _) = build_gh_launch(gh, &git, path.as_os_str());
    assert!(runs(&after, &new), "the rebuilt list lets gh start the upgraded git");
}
