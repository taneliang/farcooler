//! What a rep searches and reads (ov-555): tracked files, less `.env*`, over
//! a real repository laid out with every kind of file that must stay out.

use std::collections::BTreeSet;
use std::time::Duration;

use serde_json::json;

use super::*;
use crate::rep_gate::{Gate, Verdict, decide};

/// A git for the fixtures, away from the user's own configuration.
fn git(dir: &Path, args: &[&str]) {
    let mut cmd = std::process::Command::new("git");
    for (k, _) in std::env::vars_os() {
        if k.to_string_lossy().starts_with("GIT_") {
            cmd.env_remove(k);
        }
    }
    let out = cmd
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .current_dir(dir)
        .args(["-c", "core.hooksPath=/dev/null", "-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args(["-c", "commit.gpgsign=false", "-c", "init.defaultBranch=main"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn write(dir: &Path, path: &str, text: &str) {
    let full = dir.join(path);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(full, text).unwrap();
}

/// A checkout (`<scratch>/repo`) beside a file and a directory outside it.
///
/// Tracked and plain: `src/main.rs`, `docs/guide.md`, `*/star.txt` (a
/// directory named `*`). Tracked and private: `.env`, `.env.example`,
/// `.env.d/x.txt`, `config/.ENV`. Tracked links: `link.txt` to a file
/// outside, `dirlink` to a directory outside, `inlink` to the ignored
/// `secrets.json`. Ignored: `secrets.json`, `ignored/notes.txt`. Untracked:
/// `scratch.txt`, `.env.local`. Every file says `needle`.
fn repository() -> (tempfile::TempDir, PathBuf) {
    let scratch = tempfile::Builder::new().prefix("rep-grep").tempdir().unwrap();
    let base = std::fs::canonicalize(scratch.path()).unwrap();
    write(&base, "outside.txt", "needle outside");
    write(&base, "outdir/deep.txt", "needle outdir");
    let repo = base.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    write(&repo, ".gitignore", "secrets.json\nignored/\n");
    for tracked in [
        "src/main.rs", "docs/guide.md", "*/star.txt", ".env", ".env.example", ".env.d/x.txt", "config/.ENV",
    ] {
        write(&repo, tracked, "a line with needle in it\nand another\n");
    }
    write(&repo, "secrets.json", "needle token");
    write(&repo, "ignored/notes.txt", "needle ignored");
    let link = |from: &str, to: &str| std::os::unix::fs::symlink(to, repo.join(from)).unwrap();
    link("link.txt", "../outside.txt");
    link("dirlink", "../outdir");
    link("inlink", "secrets.json");
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "base"]);
    write(&repo, "scratch.txt", "needle untracked");
    write(&repo, ".env.local", "needle untracked secret");
    (scratch, repo)
}

/// The files a search named, sorted, from its `file:line:text` lines.
fn files(found: &Found) -> BTreeSet<String> {
    found.text.lines().map(|l| l.split(':').next().unwrap().to_string()).collect()
}

async fn find(repo: &Path, pattern: &str, path: &str) -> Result<Found> {
    search(Instant::now() + Duration::from_secs(60), repo, pattern, path).await
}

/// Only tracked, non-private files are searched, whatever else is on disk:
/// `.git`, an ignored file, an untracked `.env`, a committed `.env` in any
/// case, `.env.example`, a directory named `.env.d`, and a tracked link
/// (whose target is never followed).
#[tokio::test]
async fn a_search_reaches_only_tracked_files_that_are_not_private() {
    let (_scratch, repo) = repository();
    let plain = ["*/star.txt", "docs/guide.md", "src/main.rs"];
    let table: &[(&str, &str, &[&str])] = &[
        ("needle", "", &plain),
        ("needle", ".", &plain),
        ("needle", "src", &["src/main.rs"]),
        ("needle", "src/", &["src/main.rs"]),
        ("needle", "docs/guide.md", &["docs/guide.md"]),
        // The pathspec is literal: `*` names the directory called `*`.
        ("needle", "*", &["*/star.txt"]),
        // `.git`'s own text, which a grep of the disk would reach.
        ("bare = ", "", &[]),
        ("\\[core\\]", "", &[]),
        // Ignored, untracked and linked-to files.
        ("token", "", &[]),
        ("untracked", "", &[]),
        ("outside", "", &[]),
        ("outdir", "", &[]),
    ];
    for (pattern, path, expected) in table {
        let found = find(&repo, pattern, path).await.unwrap_or_else(|e| panic!("{pattern:?} in {path:?}: {e:?}"));
        let expected: BTreeSet<String> = expected.iter().map(|s| s.to_string()).collect();
        assert_eq!(files(&found), expected, "{pattern:?} in {path:?}: {}", found.text);
    }
}

/// A path that leaves the checkout, names `.git` or a `.env*` part, is
/// magic, goes through a link, or isn't there is refused, and so is a
/// pattern that is empty, long or several lines.
#[tokio::test]
async fn a_path_or_pattern_that_could_reach_further_is_refused() {
    let (_scratch, repo) = repository();
    for path in [
        "..", "../outdir", "src/../..", "/", "/etc", ".git", ".git/config", ".GIT", "src/.git/x", ".env", ".ENV",
        ".env.d", "config/.ENV", "dirlink", "dirlink/deep.txt", "link.txt", "missing", ":(top)src", ":/", "src\0",
    ] {
        assert_eq!(files_or(&find(&repo, "needle", path).await), Err("path"), "{path:?}");
    }
    let long = "a".repeat(LONGEST_PATTERN + 1);
    for pattern in ["", "a\nb", "a\rb", "a\0b", long.as_str()] {
        assert_eq!(files_or(&find(&repo, pattern, "").await), Err("pattern"), "{pattern:?}");
    }
}

fn files_or(found: &Result<Found>) -> std::result::Result<BTreeSet<String>, &'static str> {
    match found {
        Ok(found) => Ok(files(found)),
        Err(DomainError::InvalidArgument { what }) => Err(what),
        Err(other) => panic!("{other:?}"),
    }
}

/// A pattern that looks like an option is a pattern: it names no option and
/// runs no program.
#[tokio::test]
async fn a_pattern_that_looks_like_an_option_is_only_a_pattern() {
    let (scratch, repo) = repository();
    let planted = scratch.path().join("planted");
    for pattern in [
        format!("--open-files-in-pager=touch {}", planted.display()),
        "--no-index".to_string(),
        "-e".to_string(),
        "--untracked".to_string(),
    ] {
        let found = find(&repo, &pattern, "").await.unwrap();
        assert!(found.text.is_empty(), "{pattern}: {}", found.text);
    }
    assert!(!planted.exists());
}

/// The answer is capped in bytes at whole lines and in matches per file, and
/// carries no control character.
#[tokio::test]
async fn the_answer_is_capped_and_plain() {
    let (_scratch, repo) = repository();
    let many: String = (0..100).map(|i| format!("row {i} needle\n")).collect();
    write(&repo, "src/many.txt", &many);
    // Four files of twenty lines of two kilobytes each: past the cap.
    let wide: String = (0..40).map(|i| format!("wide{i} {}\n", "x".repeat(2000))).collect();
    for n in 0..4 {
        write(&repo, &format!("src/wide/{n}.txt"), &wide);
    }
    write(&repo, "src/escape.txt", "needle \u{1b}[31mred\u{7}\n");
    git(&repo, &["add", "src"]);
    let per_file = find(&repo, "row", "src/many.txt").await.unwrap();
    assert_eq!(per_file.text.lines().count(), 20, "{}", per_file.text);
    assert!(!per_file.cut);
    let cut = find(&repo, "wide", "src/wide").await.unwrap();
    assert!(cut.cut && cut.text.len() <= MOST_BYTES && cut.text.ends_with('\n'), "{} bytes", cut.text.len());
    assert!(cut.text.lines().all(|l| l.starts_with("src/wide/") && l.len() > 2000), "whole lines only");
    let escaped = find(&repo, "red", "src/escape.txt").await.unwrap();
    assert_eq!(escaped.text, "src/escape.txt:1:needle [31mred\n");
    // Text that reads differently from what it holds: a bidi override, a
    // zero-width space and a byte-order mark.
    write(&repo, "src/bidi.txt", "needle \u{202e}gnp.exe\u{200b} \u{feff}ok \u{2066}x\n");
    git(&repo, &["add", "src"]);
    let bidi = find(&repo, "gnp", "src/bidi.txt").await.unwrap();
    assert_eq!(bidi.text, "src/bidi.txt:1:needle gnp.exe ok x\n");
}

/// A tracked link is its target's text, never the target: a link to a file
/// that isn't tracked, or to one outside, adds nothing, and nor does a file
/// swapped for a link after it was committed.
#[tokio::test]
async fn a_link_is_never_followed() {
    let (_scratch, repo) = repository();
    std::fs::remove_file(repo.join("docs/guide.md")).unwrap();
    std::os::unix::fs::symlink("../secrets.json", repo.join("docs/guide.md")).unwrap();
    let found = find(&repo, "needle|token", "").await.unwrap();
    assert_eq!(files(&found), ["*/star.txt", "src/main.rs"].map(String::from).into(), "{}", found.text);
}

/// Whether git tracks a path, exactly: a file in the index and nothing else.
#[tokio::test]
async fn tracked_is_the_index_and_nothing_else() {
    let (_scratch, repo) = repository();
    let deadline = Instant::now() + Duration::from_secs(60);
    for (path, expected) in [
        ("src/main.rs", true),
        ("*/star.txt", true),
        // Tracked, and private: the gate's own rule refuses it first.
        (".env", true),
        ("link.txt", true),
        ("secrets.json", false),
        ("ignored/notes.txt", false),
        ("scratch.txt", false),
        (".env.local", false),
        ("src", false),
        ("missing", false),
        ("src/*", false),
        ("*", false),
        ("", false),
    ] {
        assert_eq!(tracked(deadline, &repo, Path::new(path)).await, Some(expected), "{path:?}");
    }
}

/// The gate, whole, over the real repository, the real file system and the
/// real index: Read reaches a tracked plain file and nothing git ignores or
/// doesn't know, even by a name that isn't secret-looking, and not through a
/// link to either.
#[tokio::test(flavor = "multi_thread")]
async fn a_read_reaches_only_tracked_files() {
    let (_scratch, repo) = repository();
    let cli = "/opt/fc/farcooler";
    let tracked = |within: &Path| tracked_blocking(&repo, within);
    let gate = Gate { checkout: &repo, cli, is_file: &|p: &Path| p.is_file(), tracked: &tracked };
    let grep = |path: &str, mode: Option<&str>| {
        let mut call = json!({ "pattern": "^needle", "path": path });
        if let Some(mode) = mode {
            call["output_mode"] = json!(mode);
        }
        decide(&gate, "Grep", &call, &|p: &Path| std::fs::canonicalize(p).ok())
    };
    // A one-file Grep is an oracle over the file, a prefix at a time, so it
    // meets the index as Read does, in each mode.
    for mode in [None, Some("files_with_matches"), Some("count")] {
        assert_eq!(grep("src/main.rs", mode), Verdict::Allow, "{mode:?}");
        for path in ["secrets.json", "ignored/notes.txt", "scratch.txt", "inlink", "link.txt"] {
            assert!(matches!(grep(path, mode), Verdict::Deny(_)), "Grep {mode:?} read {path}");
        }
    }
    assert!(matches!(grep("secrets.json", Some("content")), Verdict::Deny(_)));
    let read = |path: &str| decide(&gate, "Read", &json!({ "file_path": path }), &|p: &Path| std::fs::canonicalize(p).ok());
    let allowed = ["src/main.rs", "docs/guide.md", "*/star.txt"];
    for path in allowed {
        assert_eq!(read(path), Verdict::Allow, "{path}");
        assert_eq!(read(&repo.join(path).display().to_string()), Verdict::Allow, "{path} by its full path");
    }
    for path in [
        // Ignored, with plain names.
        "secrets.json", "ignored/notes.txt",
        // Untracked.
        "scratch.txt", ".env.local",
        // Tracked and private, and git itself.
        ".env", ".env.example", ".env.d/x.txt", "config/.ENV", ".git/config", ".git/HEAD",
        // Links: out of the checkout, and in, to an ignored file.
        "link.txt", "dirlink/deep.txt", "inlink",
        // Out of the checkout and not there.
        "../outside.txt", "missing", "src",
    ] {
        assert!(matches!(read(path), Verdict::Deny(_)), "{path} was read");
    }
}

/// `search '<pattern>' [--path <dir>]` is one plain command, and the path is
/// a plain relative one.
#[test]
fn search_is_one_plain_command_with_a_plain_path() {
    let cli = "/opt/fc/farcooler";
    for command in [
        "/opt/fc/farcooler search 'fn main'",
        "/opt/fc/farcooler search 'where .* (is|was) it$' --path crates/cli",
        "/opt/fc/farcooler search needle --path src/main.rs",
        "/opt/fc/farcooler search 'a|b[0-9]+\\(x\\)'",
    ] {
        assert!(crate::rep_gate::plain_cli_command(command, cli), "{command}");
    }
    for command in [
        "/opt/fc/farcooler search",
        "/opt/fc/farcooler search ''",
        "/opt/fc/farcooler search '-e'",
        "/opt/fc/farcooler search --path src",
        "/opt/fc/farcooler search needle --path",
        "/opt/fc/farcooler search needle --path ..",
        "/opt/fc/farcooler search needle --path ../x",
        "/opt/fc/farcooler search needle --path src/../..",
        "/opt/fc/farcooler search needle --path /etc",
        "/opt/fc/farcooler search needle --path ~",
        "/opt/fc/farcooler search needle --path .git",
        "/opt/fc/farcooler search needle --path .env",
        "/opt/fc/farcooler search needle --path src/.ENV.d",
        "/opt/fc/farcooler search needle --path ':(top)src'",
        "/opt/fc/farcooler search needle --path -x",
        "/opt/fc/farcooler search needle --path src --runner other",
        "/opt/fc/farcooler search needle --other x",
        "/opt/fc/farcooler search needle extra",
        "/opt/fc/farcooler search $(id)",
        "/opt/fc/farcooler search \"$HOME\"",
        "/opt/fc/farcooler search needle; id",
        "/opt/fc/farcooler search needle > out",
        "/opt/fc/farcooler search 'a\nb'",
    ] {
        assert!(!crate::rep_gate::plain_cli_command(command, cli), "{command:?}");
    }
}

/// `worktree.grep` searches the checkout of the rep pane the caller runs in,
/// from its process, and refuses a caller that isn't in one.
#[tokio::test]
async fn only_a_reps_pane_searches_and_only_its_own_checkout() {
    let (dir, svc, repo_id) = crate::test_support::fixture().await;
    crate::reconcile::repository(&svc, repo_id).await.unwrap();
    let worktree = svc.store.list_worktrees_for_repository(repo_id).unwrap().remove(0);
    let workspace = svc.store.ensure_main_workspace(repo_id).unwrap().id;
    let checkout = PathBuf::from(&worktree.worktree_path);
    write(&checkout, "src/lib.rs", "pub fn needle() {}\n");
    write(&checkout, "secrets.json", "needle token");
    write(&checkout, ".gitignore", "secrets.json\n");
    for args in [&["add", "src", ".gitignore"][..], &["commit", "-q", "-m", "files"]] {
        crate::git::git(&checkout, args).await.unwrap();
    }
    svc.store.set_member("bob", "Bob", &["phone-b".to_string()]).unwrap();
    let rep = svc.create_terminal(worktree.id, "Bob's rep", "claude").await.expect("a pane");
    svc.store.bind_rep(rep.id, workspace, "bob").unwrap();
    let snapshot = svc.inventory.refresh().await;
    let pane_pid = snapshot.claimants(rep.id).into_iter().next().expect("the rep's pane").stamp.pid as i32;
    let ask = |pattern: &str| pb::Request {
        method: "worktree.grep".into(),
        payload: Some(request::Payload::WorktreeGrep(pb::WorktreeGrep { pattern: pattern.into(), path: String::new() })),
        ..Default::default()
    };

    let Ok(result::Value::WorktreeGrepResult(found)) = handle(&svc, Some(pane_pid), ask("needle")).await else {
        panic!("a rep's pane is answered")
    };
    assert_eq!(found.text, "src/lib.rs:1:pub fn needle() {}\n");
    let elsewhere = Some(std::process::id() as i32);
    for caller in [elsewhere, None] {
        assert!(
            matches!(handle(&svc, caller, ask("needle")).await, Err(DomainError::Conflict { what: "rep_only" })),
            "{caller:?} isn't a rep's pane"
        );
    }
    drop(dir);
}

/// Where git finds no repository, `git grep` can fall back to the files on
/// disk, untracked and ignored included. A checkout whose `.git` is gone
/// (or rewritten by whoever could write it) is searched for nothing.
#[tokio::test]
async fn a_checkout_that_is_not_a_repository_is_not_searched() {
    let (_scratch, repo) = repository();
    std::fs::remove_dir_all(repo.join(".git")).unwrap();
    let found = find(&repo, "needle", "").await;
    assert!(matches!(found, Err(DomainError::OperationFailed)), "{found:?}");
    let found = find(&repo, "needle", "src").await;
    assert!(matches!(found, Err(DomainError::OperationFailed)), "{found:?}");
}

/// A git that couldn't be asked is not "not tracked": the rep is told to try
/// again, in other words, and nothing is let through.
#[test]
fn a_git_that_fails_says_so() {
    let checkout = Path::new("/w/repo");
    let resolve = |p: &Path| Some(p.to_path_buf());
    let said = |tracked: &dyn Fn(&Path) -> Option<bool>| {
        let gate = Gate { checkout, cli: "/x/farcooler", is_file: &|_| true, tracked };
        match decide(&gate, "Read", &json!({ "file_path": "/w/repo/a.rs" }), &resolve) {
            Verdict::Deny(why) => why,
            Verdict::Allow => "allowed",
        }
    };
    let (no, failed) = (said(&|_| Some(false)), said(&|_| None));
    assert!(no.contains("only files git tracks"), "{no}");
    assert!(failed.contains("couldn't check") && failed != no, "{failed}");
    assert_eq!(said(&|_| Some(true)), "allowed");
    let gate = Gate { checkout, cli: "/x/farcooler", is_file: &|_| true, tracked: &|_| None };
    let grep = json!({ "pattern": "x", "path": "/w/repo/a.rs" });
    assert!(matches!(decide(&gate, "Grep", &grep, &resolve), Verdict::Deny(why) if why.contains("couldn't check")));
}

/// A path that stops resolving between the gate's lookups is refused, not let
/// past the index check.
#[test]
fn a_path_that_stops_resolving_is_refused() {
    let checkout = Path::new("/w/repo");
    let lookups = std::cell::Cell::new(0);
    // The file resolves for `inside` and the one-file check, then is gone.
    let resolve = |p: &Path| {
        if p == Path::new("/w/repo/a.rs") {
            lookups.set(lookups.get() + 1);
            return (lookups.get() <= 2).then(|| p.to_path_buf());
        }
        Some(p.to_path_buf())
    };
    let gate = Gate { checkout, cli: "/x/farcooler", is_file: &|_| true, tracked: &|_| Some(false) };
    let grep = json!({ "pattern": "x", "path": "/w/repo/a.rs" });
    let verdict = decide(&gate, "Grep", &grep, &resolve);
    assert!(matches!(verdict, Verdict::Deny(_)), "{verdict:?}");
    assert_eq!(lookups.get(), 3, "the third lookup is the one that failed");
    // The control: the same call with the file still there is refused for not
    // being tracked, so the gate above was refused for the lookup.
    let gate = Gate { tracked: &|_| Some(true), ..gate };
    lookups.set(-10);
    assert_eq!(decide(&gate, "Grep", &grep, &resolve), Verdict::Allow);
}
