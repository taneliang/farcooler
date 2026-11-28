use crate::workflow_runtime::{Runtime, build_cleanup::CleanupTestProbe, coding_tests::configured};
use std::collections::BTreeSet;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// A runtime whose scratch base is a folder of its own, so a sweep sees only what a test made.
async fn sweeper() -> (tempfile::TempDir, Runtime, PathBuf) {
    let (dir, mut runtime) = configured(false, "pass").await;
    let base = dir.path().canonicalize().unwrap().join("scratch-base");
    std::fs::create_dir(&base).unwrap();
    runtime.scratch_base = Some(base.clone());
    (dir, runtime, base)
}

fn scratch_name(fill: char) -> String { format!("farcooler-scratch-wf1-{}", String::from(fill).repeat(64)) }

fn folder(base: &Path, name: &str) -> PathBuf {
    let path = base.join(name);
    std::fs::create_dir(&path).unwrap();
    path
}

fn age(path: &Path, modified: SystemTime) {
    std::fs::File::open(path).unwrap().set_modified(modified).unwrap();
}

/// A time at which everything made so far is over a day old.
fn tomorrow() -> SystemTime { SystemTime::now() + DAY + Duration::from_secs(60) }

#[tokio::test]
async fn an_empty_engine_scratch_folder_goes_at_a_day_old_and_not_before() {
    let (_dir, runtime, base) = sweeper().await;
    let (old, young) = (folder(&base, &scratch_name('a')), folder(&base, &scratch_name('b')));
    let at = SystemTime::now();
    age(&old, at - DAY);
    age(&young, at - DAY + Duration::from_secs(1));
    assert_eq!(runtime.sweep_empty_scratch(&BTreeSet::new(), at).await.unwrap(), 1);
    assert!(!old.exists(), "empty and a day old");
    assert!(young.exists(), "a second short of a day");
}

#[tokio::test]
async fn only_empty_folders_with_the_engines_own_scratch_name_are_swept() {
    let (_dir, runtime, base) = sweeper().await;
    let hex = "a".repeat(64);
    let others: Vec<_> = [
        "build".to_string(), "farcooler-scratch-".into(), format!("farcooler-scratch-{hex}"),
        format!("farcooler-scratch-wf1-{}", &hex[1..]), format!("farcooler-scratch-wf1-{hex}a"),
        format!("farcooler-scratch-wf1-{}", "A".repeat(64)), format!("farcooler-scratch-wf1-{}", "g".repeat(64)),
        format!("farcooler-scratch-wf1-{hex}-copy"), format!("old-farcooler-scratch-wf1-{hex}"),
        format!("output-wf1-{hex}"),
    ].iter().map(|name| folder(&base, name)).collect();
    let full = folder(&base, &scratch_name('b'));
    std::fs::write(full.join(".hidden"), "").unwrap();
    let nested = folder(&base, &scratch_name('c'));
    std::fs::create_dir(nested.join("empty")).unwrap();
    let file = base.join(scratch_name('d'));
    std::fs::write(&file, "").unwrap();
    let target = folder(&base, "linked-to");
    let link = base.join(scratch_name('e'));
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let swept = folder(&base, &scratch_name('f'));
    assert_eq!(runtime.sweep_empty_scratch(&BTreeSet::new(), tomorrow()).await.unwrap(), 1);
    assert!(!swept.exists());
    for kept in others.iter().chain([&full, &nested, &file, &target]) {
        assert!(kept.exists(), "{kept:?}");
    }
    assert!(full.join(".hidden").exists() && nested.join("empty").exists());
    assert!(link.symlink_metadata().is_ok(), "a link with a scratch name stays");
}

/// Killed on drop, so a failed assertion leaves no process behind.
struct Holder(std::process::Child);
impl Drop for Holder {
    fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); }
}

#[tokio::test]
async fn a_folder_a_process_works_in_or_has_open_stays_until_it_lets_go() {
    let (_dir, runtime, base) = sweeper().await;
    let [working, open, free] = ['a', 'b', 'c'].map(|fill| folder(&base, &scratch_name(fill)));
    let holder = Holder(std::process::Command::new("/bin/sleep").arg("600").current_dir(&working).spawn().unwrap());
    let handle = std::fs::File::open(&open).unwrap();
    assert_eq!(runtime.sweep_empty_scratch(&BTreeSet::new(), tomorrow()).await.unwrap(), 1);
    assert!(!free.exists(), "a held neighbor doesn't keep a free folder");
    assert!(working.exists(), "a process's working directory");
    assert!(open.exists(), "a folder a process has open");
    drop((holder, handle));
    assert_eq!(runtime.sweep_empty_scratch(&BTreeSet::new(), tomorrow()).await.unwrap(), 2);
    assert!(!working.exists() && !open.exists());
}

#[tokio::test]
async fn a_folder_a_pending_record_names_is_left_to_the_record() {
    let (_dir, runtime, base) = sweeper().await;
    let recorded = folder(&base, &scratch_name('a'));
    assert_eq!(runtime.sweep_empty_scratch(&BTreeSet::from([recorded.clone()]), tomorrow()).await.unwrap(), 0);
    assert!(recorded.exists());
}

#[tokio::test]
async fn nothing_goes_when_the_open_file_check_cannot_answer() {
    for script in [
        "exit 2",
        "echo 'lsof: cannot read the process table' >&2; exit 1",
        // A holder reported under a name the sweep didn't ask about.
        "printf 'p1\\nfcwd\\nn/elsewhere\\n'; exit 1",
        // Success means every name was found open, so no names can't be read as none held.
        "exit 0",
    ] {
        let (dir, mut runtime, base) = sweeper().await;
        let swept = folder(&base, &scratch_name('a'));
        let program = dir.path().join("lsof");
        std::fs::write(&program, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        runtime.cleanup_test_probe = Some(CleanupTestProbe { program, walking: Default::default() });
        assert_eq!(runtime.sweep_empty_scratch(&BTreeSet::new(), tomorrow()).await.unwrap(), 0, "{script}");
        assert!(swept.exists(), "{script}");
    }
}

#[tokio::test]
async fn the_runner_sweeps_at_start_and_again_each_hour() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let store = std::sync::Arc::new(farcooler_store::Store::open(root.join("farcooler.db")).unwrap());
    let mut runtime = Runtime::open_service(&root.join("engine"), store).unwrap();
    let base = root.join("scratch-base");
    std::fs::create_dir(&base).unwrap();
    runtime.scratch_base = Some(base.clone());
    let yesterday = SystemTime::now() - DAY - Duration::from_secs(60);
    let make = |fill| { let path = folder(&base, &scratch_name(fill)); age(&path, yesterday); path };

    // A pending record keeps its folder: its owner isn't known to have ended.
    let recorded = make('a');
    runtime.store.workflow_cleanup_enqueue(&farcooler_workflow::cleanup::CleanupEffect {
        id: "recorded".into(), workspace: "w".into(), revision: 0, item: "one".into(), run: "run".into(),
        restart: None, tree: None, folders: Vec::new(), uncertain: false,
        directory: Some(farcooler_workflow::cleanup::OwnedDirectory { path: recorded.to_string_lossy().into_owned(),
            device: 0, inode: 0, owner_effect: "missing".into(), retain_until_ms: 0 }),
    }).unwrap();

    let at_start = make('b');
    runtime.pump(1).await.unwrap();
    assert!(!at_start.exists(), "swept at runner start");
    assert!(recorded.exists());

    let later = make('c');
    runtime.pump(2).await.unwrap();
    assert!(later.exists(), "not on every cleanup round");
    let due = runtime.next_process_log_prune.unwrap();
    let wait = due.saturating_duration_since(tokio::time::Instant::now());
    assert!(wait <= Duration::from_secs(3600) && wait > Duration::from_secs(3500), "hourly: {wait:?}");
    runtime.next_process_log_prune = Some(tokio::time::Instant::now());
    runtime.pump(3).await.unwrap();
    assert!(!later.exists(), "swept again once the hour is up");
}

/// One sweep of a real scratch base, with the folders to keep listed one per line in
/// `FARCOOLER_SWEEP_KEEP` (a runner's pending recorded paths).
#[tokio::test]
#[ignore = "deletes from the base named by FARCOOLER_SWEEP_BASE"]
async fn one_sweep_of_a_named_scratch_base() {
    let base = PathBuf::from(std::env::var_os("FARCOOLER_SWEEP_BASE").expect("scratch base")).canonicalize().unwrap();
    let keep: BTreeSet<_> = std::fs::read_to_string(std::env::var_os("FARCOOLER_SWEEP_KEEP").expect("keep list"))
        .unwrap().lines().filter(|line| !line.is_empty()).map(PathBuf::from).collect();
    let count = || std::fs::read_dir(&base).unwrap().filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("farcooler-scratch-")).count();
    let (_dir, mut runtime, _) = sweeper().await;
    runtime.scratch_base = Some(base.clone());
    let before = count();
    let started = std::time::Instant::now();
    let removed = runtime.sweep_empty_scratch(&keep, SystemTime::now()).await.unwrap();
    eprintln!("scratch folders before={before} removed={removed} after={} kept-by-record={} took={:?}",
        count(), keep.len(), started.elapsed());
}
