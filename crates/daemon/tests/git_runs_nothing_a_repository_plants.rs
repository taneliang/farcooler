//! A repository's own config cannot make the daemon's git run a program.
//!
//! An agent can write anything its worktree holds, `.git` included: the
//! `gitdir:` file that says where the repository is, and through it a config
//! of the agent's choosing. The daemon runs git in that worktree as the user,
//! so every key below would be code running as the user, started by Far
//! Cooler rather than by the agent. Each test plants one of them, with a
//! program that leaves a marker file when it runs, drives the daemon's own
//! function for an ordinary review or worktree act, and asserts the marker is
//! not there.
//!
//! Real git, because the question is what git honors. Each test builds its own
//! repository, so a key planted for one cannot answer for another.
//!
//! And each test first runs the same act through plain git and asserts the
//! probe DID run (`control`). Without that, a git that stopped reading a key,
//! or a fixture that stopped reaching it, would leave a test that passes
//! because nothing could have run.

use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::path::{Path, PathBuf};
use std::process::Command;

use farcooler_daemon::change_set::{apply_uncommitted_counts, commits_since, working_tree};
use farcooler_daemon::file_diff::{Selector, file_diff};
use farcooler_daemon::git::create_worktree;
use tempfile::TempDir;

/// git as the test's own setup runs it: unhardened, and with every `GIT_`
/// variable of the environment the test runner inherited removed.
fn run(dir: &Path, args: &[&str]) -> String {
    let mut cmd = Command::new("git");
    for (k, _) in std::env::vars_os() {
        if k.to_string_lossy().starts_with("GIT_") {
            cmd.env_remove(k);
        }
    }
    let out = cmd
        .current_dir(dir)
        .args(["-c", "core.hooksPath=/dev/null"])
        // An identity on every call, not only in the repositories that set
        // one: a CI runner has no global user.name, so any commit the setup
        // makes elsewhere (a superproject, a moved submodule) would refuse.
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("git {args:?}: {e}"));
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// git with nothing of the daemon's: what the planted config is FOR. Its exit
/// status is not the point and is ignored; a `process` filter that does not
/// speak the protocol fails the act it was run for.
fn plain(dir: &Path, args: &[&str]) {
    let mut cmd = Command::new("git");
    for (k, _) in std::env::vars_os() {
        if k.to_string_lossy().starts_with("GIT_") {
            cmd.env_remove(k);
        }
    }
    let _ = cmd.current_dir(dir).args(args).stdin(std::process::Stdio::null()).output().expect("git starts");
}

/// A scratch directory with a repository at `repo` (one commit of `a.txt`
/// marked `diff=evil filter=evil`) and a `probe` program beside it.
struct Planted {
    dir: TempDir,
}

impl Planted {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let p = Self { dir };
        let probe = p.root().join("probe");
        // Every role in one program. It records that it ran, then does the
        // least its role needs so git carries on: a textconv prints the file,
        // a filter passes its input through, an fsmonitor reports nothing.
        std::fs::write(
            &probe,
            "#!/bin/sh\n\
             touch \"$(dirname \"$0\")/ran-$1\"\n\
             case \"$1\" in\n\
             textconv) cat \"$2\" ;;\n\
             fsmonitor) printf '\\0' ;;\n\
             clean|smudge) cat ;;\n\
             esac\n\
             exit 0\n",
        )
        .unwrap();
        std::fs::set_permissions(&probe, std::fs::Permissions::from_mode(0o755)).unwrap();

        let repo = p.repo();
        std::fs::create_dir_all(&repo).unwrap();
        run(&repo, &["init", "-q", "-b", "main"]);
        run(&repo, &["config", "user.email", "t@example.com"]);
        run(&repo, &["config", "user.name", "t"]);
        run(&repo, &["config", "commit.gpgsign", "false"]);
        std::fs::write(repo.join("a.txt"), "one\n").unwrap();
        std::fs::write(repo.join(".gitattributes"), "*.txt diff=evil filter=evil\n").unwrap();
        run(&repo, &["add", "-A"]);
        run(&repo, &["commit", "-q", "-m", "base"]);
        p
    }

    fn root(&self) -> PathBuf {
        self.dir.path().canonicalize().unwrap()
    }

    fn repo(&self) -> PathBuf {
        self.root().join("repo")
    }

    /// The command line that runs the probe in `role`.
    fn probe(&self, role: &str) -> String {
        format!("{} {role}", self.root().join("probe").display())
    }

    fn plant(&self, key: &str, value: &str) {
        run(&self.repo(), &["config", "--add", key, value]);
    }

    /// A hooks directory with each named hook, all of them the probe.
    fn hooks(&self, names: &[&str]) -> String {
        let dir = self.root().join("hooks");
        std::fs::create_dir_all(&dir).unwrap();
        for name in names {
            let hook = dir.join(name);
            std::fs::write(&hook, format!("#!/bin/sh\n{}\n", self.probe(&format!("hook-{name}")))).unwrap();
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        dir.to_string_lossy().into_owned()
    }

    /// Rewrite `a.txt` with what it already holds. Same size, new mtime: git
    /// cannot trust the index's stat data and has to read the file, which is
    /// the read a clean filter sits in.
    fn restat(&self) {
        let a = self.repo().join("a.txt");
        let text = std::fs::read(&a).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(&a, text).unwrap();
    }

    /// Run `args` through plain git in `dir`, assert the planted config
    /// made a probe run, and clear the record for the guarded act.
    fn control(&self, dir: &Path, args: &[&str]) {
        plain(dir, args);
        let ran = self.ran();
        assert!(!ran.is_empty(), "plain `git {args:?}` ran no probe: the fixture plants nothing");
        for role in ran {
            std::fs::remove_file(self.root().join(format!("ran-{role}"))).unwrap();
        }
    }

    /// Every probe that ran, by role.
    fn ran(&self) -> Vec<String> {
        let mut ran: Vec<String> = std::fs::read_dir(self.root())
            .unwrap()
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter_map(|n| n.strip_prefix("ran-").map(str::to_string))
            .collect();
        ran.sort();
        ran
    }
}

#[tokio::test]
async fn a_review_of_the_working_tree_starts_no_fsmonitor_and_no_index_hook() {
    let p = Planted::new();
    p.plant("core.fsmonitor", &p.probe("fsmonitor"));
    let hooks = p.hooks(&["post-index-change"]);
    p.plant("core.hooksPath", &hooks);
    std::fs::write(p.repo().join("a.txt"), "one\ntwo\n").unwrap();
    p.control(&p.repo(), &["status", "--porcelain=v2"]);

    let mut wt = working_tree(&p.repo()).await.expect("status runs");
    apply_uncommitted_counts(&p.repo(), &mut wt).await.expect("numstat runs");

    assert_eq!(p.ran(), Vec::<String>::new());
    assert_eq!(wt.unstaged.len(), 1, "the change is still seen: {wt:?}");
}

#[tokio::test]
async fn a_review_of_the_working_tree_runs_no_clean_filter() {
    let p = Planted::new();
    p.plant("filter.evil.clean", &p.probe("clean"));
    p.plant("filter.evil.process", &p.probe("process"));
    p.plant("filter.evil.required", "true");
    p.restat();
    // `--no-optional-locks`, so the control leaves the index's stat data as
    // stale as it found it, and the guarded status still has to read the file.
    p.control(&p.repo(), &["--no-optional-locks", "status", "--porcelain=v2"]);

    let wt = working_tree(&p.repo()).await;
    let counted = match wt {
        Ok(mut wt) => apply_uncommitted_counts(&p.repo(), &mut wt).await.map(|()| wt),
        Err(e) => Err(e),
    };

    // What ran first: a `process` filter that does not speak the protocol
    // fails the status too, and the failure would hide the reason.
    assert_eq!(p.ran(), Vec::<String>::new());
    let wt = counted.expect("status and numstat run");
    assert!(!wt.is_dirty(), "an unchanged file is not a change: {wt:?}");
}

#[tokio::test]
async fn a_file_diff_runs_no_textconv_and_no_external_diff() {
    let mut ran = Vec::new();
    for key in ["diff.evil.textconv", "diff.evil.command", "diff.external"] {
        let p = Planted::new();
        p.plant(key, &p.probe(key));
        std::fs::write(p.repo().join("a.txt"), "one\ntwo\n").unwrap();
        run(&p.repo(), &["commit", "-q", "-am", "two"]);
        let head = run(&p.repo(), &["rev-parse", "HEAD"]);
        std::fs::write(p.repo().join("a.txt"), "one\ntwo\nthree\n").unwrap();
        p.control(&p.repo(), &["diff", "--", "a.txt"]);

        let local = file_diff(&p.repo(), &Selector::Local, "a.txt", 0, 3).await;
        let commit = file_diff(&p.repo(), &Selector::Commit { sha: head }, "a.txt", 0, 3).await;

        // Every key tried before any verdict, so one run names all of them.
        if !p.ran().is_empty() {
            ran.extend(p.ran());
            continue;
        }
        let (local, commit) = (local.expect("diff runs"), commit.expect("diff runs"));
        // And the diff is git's own: one hunk each, which an external
        // program's output would not parse into.
        assert_eq!(local.diff.hunks.len(), 1, "{key}: the local change is still a hunk");
        assert_eq!(commit.diff.hunks.len(), 1, "{key}: the commit is still a hunk");
    }
    assert_eq!(ran, Vec::<String>::new());
}

#[tokio::test]
async fn a_branch_history_runs_no_signature_program() {
    let p = Planted::new();
    let repo = p.repo();
    let base = run(&repo, &["rev-parse", "HEAD"]);
    let tree = run(&repo, &["rev-parse", "HEAD^{tree}"]);
    // A commit carrying a signature header. `log.showSignature` hands every
    // such commit to `gpg.program`; the signature needs only to look like one.
    let commit = format!(
        "tree {tree}\nparent {base}\nauthor t <t@example.com> 1 +0000\n\
         committer t <t@example.com> 1 +0000\ngpgsig -----BEGIN PGP SIGNATURE-----\n \
         x\n -----END PGP SIGNATURE-----\n\nsigned\n"
    );
    let file = p.root().join("commit");
    std::fs::write(&file, commit).unwrap();
    let signed = run(&repo, &["hash-object", "-t", "commit", "-w", file.to_str().unwrap()]);
    run(&repo, &["update-ref", "refs/heads/main", &signed]);
    // `gpg.program` is run directly, not through a shell, so it gets a
    // program of its own rather than the probe with an argument.
    let gpg = p.root().join("gpg");
    std::fs::write(&gpg, format!("#!/bin/sh\ntouch {}/ran-gpg\nexit 1\n", p.root().display())).unwrap();
    std::fs::set_permissions(&gpg, std::fs::Permissions::from_mode(0o755)).unwrap();
    p.plant("log.showSignature", "true");
    p.plant("gpg.program", gpg.to_str().unwrap());

    p.control(&repo, &["log", "-1", "--format=%H"]);

    let commits = commits_since(&repo, &base).await.expect("log runs");

    assert_eq!(p.ran(), Vec::<String>::new());
    assert_eq!(commits.len(), 1);
}

#[tokio::test]
async fn a_new_worktree_runs_no_hook_and_no_smudge_filter() {
    let p = Planted::new();
    let hooks = p.hooks(&["post-checkout", "reference-transaction", "post-index-change"]);
    p.plant("core.hooksPath", &hooks);
    p.plant("filter.evil.smudge", &p.probe("smudge"));
    // A hook named in config rather than found in a directory (git 2.54),
    // which `core.hooksPath` does not reach. One of them has a `=` in its
    // name, which `git -c` cannot spell.
    for name in ["planted", "a=b"] {
        p.plant(&format!("hook.{name}.command"), &p.probe(&format!("cfghook-{}", name.len())));
        p.plant(&format!("hook.{name}.event"), "post-checkout");
        p.plant(&format!("hook.{name}.event"), "reference-transaction");
    }

    let control = p.root().join("control");
    p.control(&p.repo(), &["worktree", "add", "-q", "-b", "control", control.to_str().unwrap()]);

    let dest = p.root().join("side");
    create_worktree(&p.repo(), "side", "HEAD", &dest).await.expect("worktree add runs");

    assert_eq!(p.ran(), Vec::<String>::new());
    assert_eq!(std::fs::read_to_string(dest.join("a.txt")).unwrap(), "one\n");
}

#[tokio::test]
async fn a_worktree_whose_git_file_points_at_a_planted_repository_runs_nothing() {
    // The agent cannot reach the repository's own `.git/config` from a
    // sandbox that only lets it write its worktree. It can rewrite the
    // worktree's `.git` FILE, though, to point at a repository it made inside
    // the worktree, whose config is entirely its own.
    let p = Planted::new();
    let dest = p.root().join("side");
    run(&p.repo(), &["worktree", "add", "-q", "-b", "side", dest.to_str().unwrap()]);
    let planted = dest.join("planted.git");
    run(&p.root(), &["clone", "-q", "--bare", p.repo().to_str().unwrap(), planted.to_str().unwrap()]);
    run(&planted, &["config", "core.bare", "false"]);
    run(&planted, &["config", "core.fsmonitor", &p.probe("fsmonitor")]);
    run(&planted, &["config", "filter.evil.clean", &p.probe("clean")]);
    let hooks = p.hooks(&["post-index-change"]);
    run(&planted, &["config", "core.hooksPath", &hooks]);
    std::fs::write(dest.join(".git"), format!("gitdir: {}\n", planted.display())).unwrap();
    // The planted repository has no index; build one so status has stat data
    // to distrust, then distrust it.
    let mut cmd = Command::new("git");
    cmd.current_dir(&dest).args(["-c", "core.hooksPath=/dev/null", "-c", "core.fsmonitor=false"]);
    cmd.args(["-c", "filter.evil.clean=", "read-tree", "HEAD"]);
    assert!(cmd.status().unwrap().success());
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(dest.join("a.txt"), "one\n").unwrap();

    p.control(&dest, &["--no-optional-locks", "status", "--porcelain=v2"]);
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(dest.join("a.txt"), "one\n").unwrap();

    let _ = working_tree(&dest).await.expect("status runs");
    let mut wt = working_tree(&dest).await.expect("status runs");
    apply_uncommitted_counts(&dest, &mut wt).await.expect("numstat runs");

    assert_eq!(p.ran(), Vec::<String>::new());
}

/// A repository inside the worktree, registered as a submodule, with a
/// config the daemon's listing never reads.
fn with_planted_submodule(p: &Planted) -> PathBuf {
    let inner = p.root().join("inner");
    std::fs::create_dir_all(&inner).unwrap();
    run(&inner, &["init", "-q", "-b", "main"]);
    run(&inner, &["config", "user.email", "t@example.com"]);
    run(&inner, &["config", "user.name", "t"]);
    std::fs::write(inner.join("s.txt"), "s\n").unwrap();
    std::fs::write(inner.join(".gitattributes"), "*.txt filter=evil\n").unwrap();
    run(&inner, &["add", "-A"]);
    run(&inner, &["commit", "-q", "-m", "s"]);
    let repo = p.repo();
    run(&repo, &["-c", "protocol.file.allow=always", "submodule", "add", "-q", inner.to_str().unwrap(), "sub"]);
    run(&repo, &["commit", "-q", "-m", "sub"]);
    let sub = repo.join("sub");
    let gitdir = run(&sub, &["rev-parse", "--absolute-git-dir"]);
    let config = format!("{gitdir}/config");
    for (key, value) in [
        ("filter.evil.clean", p.probe("sub-clean")),
        ("hook.x.command", p.probe("sub-hook")),
        ("hook.x.event", "post-index-change".to_string()),
        ("diff.external", p.probe("sub-external")),
    ] {
        run(&repo, &["config", "-f", &config, "--add", key, &value]);
    }
    // What makes git start a diff in the submodule rather than print the
    // two commits: the parent repository's own choice, so the agent's too.
    p.plant("diff.submodule", "diff");
    p.plant("submodule.sub.ignore", "none");
    sub
}

#[tokio::test]
async fn a_review_does_not_run_a_submodules_own_config() {
    let p = Planted::new();
    let sub = with_planted_submodule(&p);
    // Same size, new mtime, and one line more: a submodule with work in it.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(sub.join("s.txt"), "s\n").unwrap();
    std::fs::write(sub.join("t.txt"), "t\n").unwrap();
    p.control(&p.repo(), &["--no-optional-locks", "status", "--porcelain=v2"]);
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(sub.join("s.txt"), "s\n").unwrap();

    let mut wt = working_tree(&p.repo()).await.expect("status runs");
    apply_uncommitted_counts(&p.repo(), &mut wt).await.expect("numstat runs");
    assert_eq!(p.ran(), Vec::<String>::new());

    // A submodule moved to a new commit: still shown, from the gitlink alone.
    std::fs::write(sub.join("s.txt"), "s\nmore\n").unwrap();
    run(&sub, &["commit", "-q", "-am", "moved"]);
    p.control(&p.repo(), &["diff", "--", "sub"]);
    let wt = working_tree(&p.repo()).await.expect("status runs");
    let diff = file_diff(&p.repo(), &Selector::Local, "sub", 0, 3).await;

    assert_eq!(p.ran(), Vec::<String>::new());
    assert!(wt.unstaged.iter().any(|f| f.path == "sub"), "the moved submodule is still a change: {wt:?}");
    assert!(diff.is_ok(), "{diff:?}");
}

#[tokio::test]
async fn a_new_worktree_runs_nothing_its_branch_includes() {
    // `includeIf "onbranch:…"` is false for the git started in the main
    // checkout on `main`, so the guard's listing there sees nothing; it is
    // true for the git that checks the new worktree out on its new branch.
    let p = Planted::new();
    let evil = p.repo().join(".git").join("evil.cfg");
    let evil_cfg = format!(
        "[filter \"z\"]\n\tsmudge = {smudge}\n[hook \"y\"]\n\tcommand = {hook}\n\
         \tevent = post-checkout\n\tevent = reference-transaction\n\tevent = post-index-change\n",
        smudge = p.probe("smudge"),
        hook = p.probe("included-hook"),
    );
    std::fs::write(&evil, evil_cfg).unwrap();
    std::fs::write(p.repo().join(".gitattributes"), "*.txt filter=z\n").unwrap();
    run(&p.repo(), &["commit", "-q", "-am", "z"]);
    for branch in ["control", "side"] {
        p.plant(&format!("includeIf.onbranch:{branch}.path"), "evil.cfg");
    }
    let control = p.root().join("control");
    p.control(&p.repo(), &["worktree", "add", "-q", "-b", "control", control.to_str().unwrap()]);

    let dest = p.root().join("side");
    create_worktree(&p.repo(), "side", "HEAD", &dest).await.expect("worktree add runs");

    assert_eq!(p.ran(), Vec::<String>::new());
    assert_eq!(std::fs::read_to_string(dest.join("a.txt")).unwrap(), "one\n", "checked out");
}

/// OPEN, and ignored until it closes: the listing and the call it guards are
/// two gits, and a config that changes between them gets its hook run.
/// Measured at about one guarded status in five. Names can't close this; an
/// exec allowlist around git would (see the ov-129 report). Run it with
/// `--ignored` to watch it fail.
#[tokio::test]
#[ignore = "open: the list-then-run race, ov-129"]
async fn a_config_swapped_between_the_listing_and_the_call_runs_nothing() {
    let p = Planted::new();
    let git_dir = p.repo().join(".git");
    let clean = std::fs::read_to_string(git_dir.join("config")).unwrap();
    let evil = format!(
        "{clean}[hook \"x\"]\n\tcommand = {}\n\tevent = post-index-change\n",
        p.probe("raced-hook")
    );
    let stop = Arc::new(AtomicBool::new(false));
    let swapper = {
        let (stop, git_dir) = (stop.clone(), git_dir.clone());
        std::thread::spawn(move || {
            let mut flip = false;
            while !stop.load(Ordering::Relaxed) {
                let tmp = git_dir.join("config.swap");
                std::fs::write(&tmp, if flip { &evil } else { &clean }).unwrap();
                std::fs::rename(&tmp, git_dir.join("config")).unwrap();
                flip = !flip;
            }
        })
    };
    for _ in 0..60 {
        std::fs::write(p.repo().join("a.txt"), "one\n").unwrap();
        let _ = working_tree(&p.repo()).await;
    }
    stop.store(true, Ordering::Relaxed);
    swapper.join().unwrap();

    assert_eq!(p.ran(), Vec::<String>::new());
}
