//! Bounded private-Git and malformed-receipt regressions for the security delta.
use super::*;
use super::super::{coding_tests::configured, tests::{enqueue, finish}};

async fn git(repo: &Path, args: &[&str]) -> String {
    let out = tokio::process::Command::new("/usr/bin/git").current_dir(repo).env_clear()
        .env("PATH", "/usr/bin:/bin").env("GIT_CONFIG_NOSYSTEM", "1").env("GIT_CONFIG_GLOBAL", "/dev/null")
        .args(["-c", "core.hooksPath=/dev/null", "-c", "commit.gpgSign=false", "-c", "tag.gpgSign=false",
            "-c", "user.name=Test", "-c", "user.email=test@localhost", "-c", "protocol.file.allow=always"])
        .args(args).output().await.unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap().trim().into()
}

async fn push_config(key: &str) {
    let (_dir, mut runtime) = super::tests::remote().await;
    let repo = runtime.repository();
    git(&repo, &["tag", "-a", "unselected", "-m", "must remain local"]).await;
    git(&repo, &["config", key, "true"]).await;
    let remote = runtime.coding.as_ref().unwrap().landing.as_ref().unwrap().repository.clone();
    let base = git(&remote, &["rev-parse", "refs/heads/main"]).await;
    git(&remote, &["update-ref", "refs/heads/unselected", &base]).await;
    let unrelated = |refs: String| refs.lines().filter(|line| !line.starts_with("refs/heads/main "))
        .map(str::to_owned).collect::<Vec<_>>();
    let refs = unrelated(git(&remote, &["for-each-ref", "--format=%(refname) %(objectname)"]).await);
    let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
    assert_eq!(finish(&mut runtime, "w").await.items["one"].phase, Phase::Done);
    assert_eq!(unrelated(git(&remote, &["for-each-ref", "--format=%(refname) %(objectname)"]).await), refs);
    assert_eq!(git(&remote, &["rev-parse", "refs/heads/main"]).await,
        runtime.state("w").unwrap().unwrap().items["one"].candidate.clone().unwrap());
}

#[tokio::test]
async fn workflow_coding_security_push_conserves_tags() { push_config("push.followTags").await; }

#[tokio::test]
async fn workflow_coding_security_push_disables_signing() { push_config("push.gpgSign").await; }

#[tokio::test]
async fn workflow_coding_security_push_conserves_submodule_destination() {
    let (dir, mut runtime) = super::tests::remote().await;
    let repo = runtime.repository();
    let secondary = dir.path().join("secondary.git"); std::fs::create_dir(&secondary).unwrap();
    git(&secondary, &["init", "--bare", "--quiet"]).await;
    let sub = dir.path().join("sub"); std::fs::create_dir(&sub).unwrap();
    git(&sub, &["init", "--quiet"]).await;
    git(&sub, &["commit", "--allow-empty", "-m", "seed"]).await;
    git(&sub, &["remote", "add", "origin", secondary.to_str().unwrap()]).await;
    git(&sub, &["push", "-u", "origin", "HEAD"]).await;
    git(&repo, &["submodule", "add", sub.to_str().unwrap(), "nested"]).await;
    let nested = repo.join("nested");
    git(&nested, &["remote", "set-url", "origin", secondary.to_str().unwrap()]).await;
    git(&repo, &["add", "nested", ".gitmodules"]).await;
    git(&repo, &["commit", "-m", "pin nested candidate"]).await;
    let base = git(&repo, &["rev-parse", "HEAD"]).await;
    git(&repo, &["update-ref", runtime.landing_ref(), &base]).await;
    let remote = runtime.coding.as_ref().unwrap().landing.as_ref().unwrap().repository.clone();
    git(&repo, &["push", remote.to_str().unwrap(), "HEAD:refs/heads/main"]).await;
    git(&nested, &["commit", "--allow-empty", "-m", "unpublished submodule commit"]).await;
    let unpublished = git(&nested, &["rev-parse", "HEAD"]).await;
    // The newly built candidate must introduce the unpublished gitlink; an
    // unchanged gitlink in the base never exercises push's on-demand path.
    let script = dir.path().join("adapter.py");
    let source = std::fs::read_to_string(&script).unwrap().replace("if stage == 'build':",
        &format!("if stage == 'build':\n    subprocess.check_call(['/usr/bin/git', 'update-index', '--cacheinfo', '160000,{unpublished},nested'])"));
    std::fs::write(script, source).unwrap();
    git(&repo, &["remote", "add", "selected", remote.to_str().unwrap()]).await;
    git(&repo, &["config", "push.recurseSubmodules", "on-demand"]).await;
    let before = git(&secondary, &["for-each-ref", "--format=%(refname) %(objectname)"]).await;
    let config = runtime.coding.as_mut().unwrap(); config.base = base;
    std::fs::write(runtime.root.join("coding.json"), serde_json::to_vec(config).unwrap()).unwrap();
    let root = runtime.root.clone(); drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
    assert_eq!(finish(&mut runtime, "w").await.items["one"].phase, Phase::Done);
    assert_eq!(git(&secondary, &["for-each-ref", "--format=%(refname) %(objectname)"]).await, before);
    // Conservation alone did not fail under the retained guard environment
    // when the explicit override was removed. Also require its recorded finite
    // argv contract; do not claim the probe demonstrated secondary writes.
    let land = runtime.store.workflow_outbox("w").unwrap().into_iter()
        .find(|s| matches!(s.effect.kind, EffectKind::Execute(StageKind::Land))).unwrap();
    let fact: serde_json::Value = serde_json::from_slice(&std::fs::read(runtime.root.join("receipts")
        .join(format!("{}.push-process.json", digest(&land.effect)))).unwrap()).unwrap();
    assert!(fact["argv"].as_array().unwrap().contains(&serde_json::json!("--recurse-submodules=no")));
    assert_eq!(git(&remote, &["rev-parse", "refs/heads/main"]).await,
        runtime.state("w").unwrap().unwrap().items["one"].candidate.clone().unwrap());
}

#[test]
fn workflow_coding_security_fifo_receipts_are_bounded_and_never_replayed() {
    const CHILD: &str = "FARCOOLER_TEST_FIFO_CHILD";
    if std::env::var_os(CHILD).is_some() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            for mode in ["ci-fifo", "review-fifo"] {
                let (_dir, mut runtime) = configured(mode == "review-fifo", mode).await;
                let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
                for time in 2..15 { runtime.pump(time).await.unwrap(); }
                assert_eq!(runtime.state("w").unwrap().unwrap().items["one"].phase, Phase::Uncertain);
                let snapshot = |runtime: &Runtime| std::fs::read_dir(runtime.root.join("receipts")).unwrap()
                    .map(|entry| entry.unwrap()).filter(|entry| entry.file_type().unwrap().is_file()
                        && !entry.file_name().to_string_lossy().ends_with(".git.jsonl"))
                    .map(|entry| (entry.file_name(), std::fs::read(entry.path()).unwrap()))
                    .collect::<std::collections::BTreeMap<_, _>>();
                let facts = snapshot(&runtime);
                let root = runtime.root.clone(); drop(runtime);
                let mut runtime = Runtime::open(&root).unwrap(); runtime.pump(200).await.unwrap();
                assert_eq!(runtime.state("w").unwrap().unwrap().items["one"].phase, Phase::Uncertain);
                assert_eq!(runtime.pump(201).await.unwrap(), 0);
                assert_eq!(snapshot(&runtime), facts);
            }
            let (_dir, mut runtime) = configured(true, "pass").await;
            let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
            assert_eq!(finish(&mut runtime, "w").await.items["one"].phase, Phase::Done);
        });
        return;
    }
    // A Tokio timeout cannot interrupt a synchronous FIFO open. A separate
    // process bounds the actual runtime owner, and is killed/reaped on failure.
    let name = concat!(module_path!(), "::workflow_coding_security_fifo_receipts_are_bounded_and_never_replayed");
    let name = name.split_once("::").unwrap().1;
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"]).env(CHILD, "1").spawn().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().unwrap() { assert!(status.success()); break; }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap(); child.wait().unwrap();
            panic!("receipt runtime exceeded 10s without any FIFO writer");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[tokio::test]
async fn workflow_coding_security_receipt_file_identity_and_size() {
    let (dir, runtime) = configured(false, "pass").await;
    let file = dir.path().join("receipt"); std::fs::write(&file, b"regular").unwrap();
    assert_eq!(runtime.bytes(&file).unwrap(), b"regular");
    let alias = dir.path().join("alias"); std::fs::hard_link(&file, &alias).unwrap();
    assert!(runtime.bytes(&file).is_err()); std::fs::remove_file(&alias).unwrap();
    std::os::unix::fs::symlink(&file, &alias).unwrap(); assert!(runtime.bytes(&alias).is_err());
    assert!(runtime.bytes(dir.path()).is_err());
    let f = std::fs::OpenOptions::new().write(true).open(&file).unwrap();
    f.set_len(8 * 1024 * 1024).unwrap(); assert_eq!(runtime.bytes(&file).unwrap().len(), 8 * 1024 * 1024);
    f.set_len(8 * 1024 * 1024 + 1).unwrap(); assert!(runtime.bytes(&file).is_err());
}
