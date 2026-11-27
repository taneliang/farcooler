use super::Ledger;
use crate::workflow_runtime::{Command, CodingCommand, Phase, Runtime, adapters};
use crate::workflow_runtime::coding_verbs_tests::{at_join, batched_push, command, lander_with, phase_of, until};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

const UNIT: &str = "target/debug/deps/libthird-0123456789abcdef.rlib";

/// A seed command that writes one third-party unit holding the commit it was built from, or fails.
fn seed_command(dir: &Path, fails: bool) -> CodingCommand {
    let script = dir.join(if fails { "seed-fails.sh" } else { "seed.sh" });
    let body = if fails { "exit 3".to_string() } else { format!(
        "mkdir -p \"$FARCOOLER_SEED/{parent}\"\n/usr/bin/git rev-parse HEAD > \"$FARCOOLER_SEED/{UNIT}\"",
        parent = Path::new(UNIT).parent().unwrap().display()) };
    std::fs::write(&script, format!("#!/bin/sh\nset -e\n{body}\n")).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    CodingCommand { argv: vec![script.to_string_lossy().into_owned()],
        environment: [("PATH".to_string(), "/usr/bin:/bin".to_string())].into(), timeout_ms: 60_000 }
}

async fn seeded(fails: bool) -> (tempfile::TempDir, Runtime) {
    let (dir, runtime) = lander_with("pass", |_| {}).await;
    let root = runtime.root.clone();
    let mut config = runtime.coding.clone().unwrap();
    config.dependency_seed = Some(seed_command(&dir.path().canonicalize().unwrap(), fails));
    std::fs::write(root.join("coding.json"), serde_json::to_vec(&config).unwrap()).unwrap();
    drop(runtime);
    (dir, Runtime::open(&root).unwrap())
}

async fn head(runtime: &Runtime) -> String {
    adapters::git(&runtime.repository(), &["rev-parse", runtime.landing_ref()]).await.unwrap()
}

fn seeds(runtime: &Runtime) -> Vec<String> {
    let mut names: Vec<_> = std::fs::read_dir(runtime.root.join(super::SEEDS)).unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
}

async fn seed_now(runtime: &Runtime, commit: &str) {
    runtime.refresh_seed(commit).expect("a seed starts").await.unwrap();
}

#[tokio::test]
async fn a_landing_seeds_the_next_builds_scratch_and_retires_the_old_seed() {
    let (dir, mut runtime) = seeded(false).await;
    let base = head(&runtime).await;
    seed_now(&runtime, &base).await;
    assert_eq!(runtime.seed_ledger().unwrap(), Ledger { current: Some(base.clone()), retired: vec![] });
    assert_eq!(seeds(&runtime), [base.clone(), format!("{base}.log"), "ledger.json".into()]);
    assert!(runtime.refresh_seed(&base).is_none(), "a commit with a seed is not built again");

    // The build stage records what its scratch held when the agent started.
    let adapter = dir.path().join("adapter.py");
    let seen = dir.path().join("seen");
    let script = std::fs::read_to_string(&adapter).unwrap().replace("if stage == 'build':\n", &format!(concat!(
        "if stage == 'build':\n",
        "    unit = pathlib.Path(os.environ['FARCOOLER_SCRATCH']) / '{unit}'\n",
        "    with open('{seen}', 'a') as f: f.write(unit.read_text() if unit.exists() else 'cold\\n')\n",
        "    if unit.exists(): unit.write_text('written by the builder')\n"), unit = UNIT, seen = seen.display()));
    std::fs::write(&adapter, script).unwrap();

    let definition = batched_push(&runtime, "any", 5000);
    at_join(&mut runtime, "one", &definition).await;
    assert_eq!(std::fs::read_to_string(&seen).unwrap(), format!("{base}\n"));
    assert_eq!(std::fs::read_to_string(runtime.root.join(super::SEEDS).join(&base).join(UNIT)).unwrap(), format!("{base}\n"),
        "a builder writes to its own clone, never to the seed");

    assert!(command(&runtime, Command::Queue { item: "one".into() }).result.is_ok());
    until(&mut runtime, |state| phase_of(state, "one") == Some(Phase::Done)).await;
    let landed = crate::workflow_runtime::coding_verbs_tests::remote_head(&runtime).await;
    assert_ne!(landed, base);
    for _ in 0..400 {
        if runtime.seed_ledger().unwrap().current.as_deref() == Some(landed.as_str()) { break; }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    let ledger = runtime.seed_ledger().unwrap();
    assert_eq!(ledger.current.as_deref(), Some(landed.as_str()), "the landing started a seed of the landed commit");
    assert!(ledger.retired.is_empty(), "{ledger:?}");
    assert!(!runtime.root.join(super::SEEDS).join(&base).exists(), "the previous seed went once the new one was ready");
    assert!(!runtime.root.join(super::SEEDS).join(format!("{landed}.checkout")).exists());

    at_join(&mut runtime, "two", &definition).await;
    assert_eq!(std::fs::read_to_string(&seen).unwrap(), format!("{base}\n{landed}\n"));
}

#[tokio::test]
async fn a_failed_seed_leaves_builders_cold_and_is_not_rebuilt_for_the_same_commit() {
    let (_dir, runtime) = seeded(true).await;
    let base = head(&runtime).await;
    seed_now(&runtime, &base).await;
    let ledger = runtime.seed_ledger().unwrap();
    assert_eq!(ledger.current, None);
    assert!(ledger.retired.is_empty(), "{ledger:?}");
    assert_eq!(seeds(&runtime), [format!("{base}.log"), "ledger.json".into()], "only the log is left to say why");
    assert!(runtime.refresh_seed(&base).is_none(), "the same commit isn't tried again");

    let scratch = runtime.root.parent().unwrap().join("scratch");
    std::fs::create_dir(&scratch).unwrap();
    runtime.seed_scratch(&scratch, &base).await;
    assert_eq!(std::fs::read_dir(&scratch).unwrap().count(), 0, "with no seed the build starts cold");
}

#[tokio::test]
async fn a_seed_waits_for_the_disk_a_build_needs() {
    let (_dir, mut runtime) = seeded(false).await;
    let base = head(&runtime).await;
    runtime.build_free_bytes = Some(super::super::builder_resources::MIN_FREE - 1);
    seed_now(&runtime, &base).await;
    assert_eq!(runtime.seed_ledger().unwrap(), Ledger::default());
    assert!(seeds(&runtime).is_empty(), "nothing was made: {:?}", seeds(&runtime));
    runtime.build_free_bytes = Some(u64::MAX);
    seed_now(&runtime, &base).await;
    assert_eq!(runtime.seed_ledger().unwrap().current, Some(base));
}

#[tokio::test]
async fn only_folders_the_ledger_names_are_removed_and_never_while_open() {
    let (_dir, runtime) = seeded(false).await;
    let base = head(&runtime).await;
    seed_now(&runtime, &base).await;
    let folder = runtime.root.join(super::SEEDS);
    let unrecorded = folder.join("not-ours");
    std::fs::create_dir(&unrecorded).unwrap();
    let outside: PathBuf = runtime.root.join("outside-the-seed-folder");
    std::fs::create_dir(&outside).unwrap();
    let open = folder.join("open.partial");
    std::fs::create_dir(&open).unwrap();
    let held = std::fs::File::create(open.join("held")).unwrap();
    let text = |p: &Path| p.to_string_lossy().into_owned();
    runtime.write_seed_ledger(&Ledger { current: Some(base.clone()), retired: vec![text(&outside), text(&open)] }).unwrap();

    runtime.remove_retired_seeds().await;
    assert!(outside.exists(), "a ledger entry outside the seed folder is left alone");
    assert!(unrecorded.exists(), "a folder the ledger never named is left alone");
    assert!(open.exists(), "an open file keeps a retired folder");
    assert_eq!(runtime.seed_ledger().unwrap().retired, [text(&open)], "the entry outside is dropped from the ledger, the open one kept");

    drop(held);
    runtime.remove_retired_seeds().await;
    assert!(!open.exists());
    assert_eq!(runtime.seed_ledger().unwrap(), Ledger { current: Some(base.clone()), retired: vec![] });
    assert!(folder.join(&base).exists(), "the current seed stays");
}
