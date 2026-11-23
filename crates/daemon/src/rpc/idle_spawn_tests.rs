//! A full idle minute with the daemon's background jobs and instrumented executables.
use std::path::{Path, PathBuf};
use std::time::Duration;
use farcooler_core::page_doc::{self, Caps};
use farcooler_store::{models::Actor, pages::{Anchor, PageWrite}};
use std::os::unix::fs::PermissionsExt;

const SHIMS: &str = "FARCOOLER_TEST_IDLE_SHIMS";

fn write_shim(root: &Path, name: &str, body: &str) {
    let log = root.join(format!("{name}.log"));
    let file = root.join(name);
    std::fs::write(&file, format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> {log:?}\n{body}\n")).unwrap();
    std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o700)).unwrap();
}

fn launches(root: &Path, program: &str) -> Vec<String> {
    std::fs::read_to_string(root.join(format!("{program}.log"))).unwrap_or_default()
        .lines().map(str::to_owned).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "measures a real minute with the production background schedules"]
async fn idle_board_spawn_budget() {
    let Some(shims) = std::env::var_os(SHIMS).map(PathBuf::from) else {
        // Keep the PATH override and the executable lookup caches in a process
        // of their own. No other daemon test sees the shims or the mock remote.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let git = farcooler_core::programs::find("git").unwrap();
        let tmux = farcooler_core::programs::find("tmux").unwrap();
        write_shim(&root, "lsof", "printf 'p1234\\n'");
        write_shim(&root, "git", &format!(r#"
case "$*" in
  --exec-path|*"ls-remote --refs"*) exit 1 ;;
esac
exec {git:?} "$@""#));
        write_shim(&root, "tmux", &format!("exec {tmux:?} \"$@\""));
        write_shim(&root, "gh", r#"
case "$*" in
  *"/actions/runs/73/jobs"*) printf '%s\n' '[{"name":"Rust","status":"completed","conclusion":"success"}]' ;;
  *"/actions/runs/73"*) printf '%s\n' '{"id":73,"name":"CI","head_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","status":"completed","conclusion":"success","html_url":"https://example.invalid/73","created_at":"2026-01-01T00:00:00Z"}' ;;
  *) exit 1 ;;
esac"#);
        let path = std::env::join_paths(std::iter::once(root.clone()).chain(
            std::env::split_paths(&std::env::var_os("PATH").unwrap()))).unwrap();
        let status = tokio::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "rpc::idle_spawn_tests::idle_board_spawn_budget", "--ignored", "--nocapture"])
            .env(SHIMS, &root).env("PATH", path).env("HOME", &root).env_remove("SSH_AUTH_SOCK").status().await.unwrap();
        assert!(status.success(), "idle launch census failed");
        return;
    };

    let (dir, svc, repo) = crate::test_support::short_fixture().await;
    let board = svc.store.ensure_main_workspace(repo).unwrap().id;
    let ws = board.to_string();
    let repository = dir.path().join("repo").canonicalize().unwrap();
    let head = crate::git::git(&repository, &["rev-parse", "HEAD"]).await.unwrap();
    assert!(head.ok);
    let base = head.stdout.trim().to_owned();
    let (_config_dir, mut runtime) = crate::workflow_runtime::read_perf_tests::idle_board_fixture(
        &svc, &ws, &repository, base, dir.path(),
    ).await;
    let main = svc.list_worktrees().unwrap().remove(0);
    svc.create_terminal(main.id, "Idle pane", "shell").await.unwrap();

    let page = page_doc::parse(r#"{"v":1,"title":"CI","blocks":[{"type":"links","items":[{"label":"Finished","ci":"run:73"}]}]}"#, &Caps::default()).unwrap();
    svc.store.set_page(board, &PageWrite { slot: "ci", page: &page, anchor: Anchor::Clear, ordinal: None,
        if_revision: None }, Actor::Runner).unwrap();
    let watcher = crate::watch::Watcher::new(svc.clone());
    let mut memo = crate::ci_watch::Memo::default();
    crate::ci_watch::read_all(&svc, &watcher, &mut memo).await;
    assert!(svc.store.ci_read(board, "run:73").unwrap().unwrap().status.is_finished());
    assert_eq!(launches(&shims, "gh").len(), 2, "warmup reads the run and its jobs once");
    // The run finished before the old watcher's ten-minute refresh cutoff.
    let db = rusqlite::Connection::open(svc.root_dir().join("farcooler.db")).unwrap();
    db.execute("UPDATE board_ci SET fetched_at = 1 WHERE workspace_id = ?1 AND subject = 'run:73'",
        [board.as_bytes()]).unwrap();
    drop(db);

    let before = ["lsof", "git", "gh", "tmux"].map(|p| launches(&shims, p).len());
    let end = tokio::time::Instant::now() + Duration::from_secs(60);
    let (_stop, shutdown) = tokio::sync::watch::channel(false);
    let engine = tokio::spawn(async move { runtime.run(shutdown).await.unwrap() });
    let inventory = tokio::spawn(watcher.clone().run());
    let ci = tokio::spawn(crate::ci_watch::run(svc.clone(), watcher.clone()));
    let prs = tokio::spawn(crate::pr_watch::run(svc.clone(), watcher.clone()));
    let reviews = tokio::spawn(crate::pr_review::run(svc.clone(), watcher.clone()));
    let landing = tokio::spawn(crate::landing_read::run(svc.clone(), watcher.clone()));
    let mut ticks = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            biased;
            _ = tokio::time::sleep_until(end) => break,
            // Subject notifications may wake the watcher even on an idle board.
            _ = ticks.tick() => crate::ci_watch::kick(),
        }
    }
    let jobs = [engine, inventory, ci, prs, reviews, landing];
    for job in &jobs { job.abort(); }
    for job in jobs {
        if let Err(error) = job.await { assert!(error.is_cancelled(), "{error}"); }
    }
    let [lsof, git, gh, tmux] = ["lsof", "git", "gh", "tmux"].map(|p| launches(&shims, p));
    let lsof = &lsof[before[0]..];
    let git = &git[before[1]..];
    let gh = &gh[before[2]..];
    let tmux = &tmux[before[3]..];
    let heads = git.iter().filter(|args| args.starts_with("ls-remote --refs")).count();
    let recursive = lsof.iter().filter(|args| args.starts_with("-nP -F p +D")).count();
    let finished = gh.iter().filter(|args| args.contains("/actions/runs/73")).count();
    println!("idle 60s launches: lsof={} (recursive={recursive}) git={} (ls-remote={heads}) gh={} (finished run={finished}) tmux={}",
        lsof.len(), git.len(), gh.len(), tmux.len());

    assert!((1..=12).contains(&recursive), "recursive lsof launches: {recursive}");
    assert!(lsof.len() <= 80, "all lsof launches: {}", lsof.len());
    assert!((1..=2).contains(&heads), "ls-remote launches: {heads}");
    // This includes repository guard and tracked-file checks before each
    // cleanup probe, alongside the independently bounded remote-head calls.
    assert!(git.len() <= 100, "all git launches: {}", git.len());
    assert_eq!(finished, 0, "a completed run was polled again");
    assert!(gh.len() <= 4, "all gh launches: {}", gh.len());
    assert!((1..=150).contains(&tmux.len()), "tmux launches: {}", tmux.len());
}
