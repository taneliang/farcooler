//! A landing target that is a network remote (ov-1026), reached over `ssh://` with the runner user's own git and
//! ssh setup. The remote is a private bare repository; the "ssh" is a stand-in named by `core.sshCommand` in a
//! scratch home's own global git config, which runs the remote's end locally with an empty environment, as a
//! real ssh would. Nothing here reaches the network.
use super::*;
use super::coding_verbs_tests::{at_join, batched_push, candidate_of, command, lander, outbox, phase_of, until};
use super::coding_tests::admitted;

struct Remote { _dir: tempfile::TempDir, runtime: Runtime, bare: PathBuf, calls: PathBuf }

/// The stand-in ssh: it logs what it was asked to run, refuses a push when `REFUSE` exists and every call when
/// `REFUSE_ALL` does, and otherwise runs the remote's command (the last argument) with nothing of the caller's
/// environment.
const SSH: &str = r#"#!/bin/sh
for a; do last="$a"; done
printf '%s\n' "$last" >> "$CALLS_FILE"
if [ -e "$REFUSE_FILE-all" ]; then echo 'git@farcooler-test: Permission denied (publickey).' >&2; exit 255; fi
case "$last" in git-receive-pack*) if [ -e "$REFUSE_FILE" ]; then echo 'git@farcooler-test: Permission denied (publickey).' >&2; exit 255; fi;; esac
exec /usr/bin/env -i PATH=/usr/bin:/bin /bin/sh -c "$last"
"#;

async fn remote(mode: &str) -> Remote { remote_at(mode, None).await }

/// A remote reached at `github` (a landing URL and the path it names), which the stand-in ssh answers from the scratch
/// bare repository: the URL a first draft suggests, taken as it is, with no GitHub behind it.
async fn remote_at(mode: &str, github: Option<(&str, &str)>) -> Remote {
    let (dir, runtime) = lander(mode).await;
    let bare = dir.path().join("private-remote.git").canonicalize().unwrap();
    let home = dir.path().join("home"); std::fs::create_dir(&home).unwrap();
    let calls = dir.path().join("ssh-calls");
    let ssh = dir.path().join("ssh");
    let mut script = SSH.to_string();
    if let Some((_, path)) = github {
        script = script.replace("exec /usr/bin/env", &format!("last=$(printf '%s' \"$last\" | sed \"s#'{path}'#'{}'#\")\nexec /usr/bin/env", bare.display()));
    }
    std::fs::write(&ssh, script.replace("$CALLS_FILE", &calls.to_string_lossy()).replace("$REFUSE_FILE", &dir.path().join("refuse").to_string_lossy())).unwrap();
    std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o755)).unwrap();
    // The runner user's own global config: their ssh command is theirs to choose.
    std::fs::write(home.join(".gitconfig"), format!("[core]\n\tsshCommand = {}\n", ssh.display())).unwrap();
    let root = runtime.root.clone();
    let mut config = runtime.coding.clone().unwrap();
    config.landing = Some(CodingLanding { job: "landing-1026".into(), repository: github.map_or_else(|| format!("ssh://git@farcooler-test{}", bare.display()), |(url, _)| url.to_string()).into(),
        target: "refs/heads/main".into(), config_digest: String::new() });
    std::fs::write(root.join("coding.json"), serde_json::to_vec(&config).unwrap()).unwrap();
    drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    runtime.user_home = Some(home);
    Remote { _dir: dir, runtime, bare, calls }
}
/// Test setup, as a person at a terminal would run it.
fn plain_git(dir: &Path, args: &[&str]) {
    assert!(std::process::Command::new("/usr/bin/git").current_dir(dir).args(args).status().unwrap().success(), "{args:?}");
}
async fn head(bare: &Path) -> String { adapters::git(bare, &["rev-parse", "refs/heads/main"]).await.unwrap() }
fn calls(remote: &Remote) -> String { std::fs::read_to_string(&remote.calls).unwrap_or_default() }
/// The push step's settlement: its label and why.
fn push_fact(runtime: &Runtime) -> serde_json::Value { fact(runtime, StageKind::Push) }
/// The last settlement of a `kind` step: its label and why.
fn fact(runtime: &Runtime, kind: StageKind) -> serde_json::Value {
    let push = runtime.store.workflow_outbox("w").unwrap().into_iter().rev()
        .find(|s| s.effect.kind == EffectKind::Execute(kind.clone())).expect("the step ran");
    let path = runtime.root.join("receipts").join(format!("{}.verb.json", digest(&push.effect)));
    serde_json::from_slice(&std::fs::read(path).expect("the push settled")).unwrap()
}

#[tokio::test]
async fn a_ticket_lands_on_an_ssh_remote_through_the_users_own_ssh_and_nothing_else() {
    let mut r = remote("pass").await;
    let definition = batched_push(&r.runtime, "any", 5000);
    let first = head(&r.bare).await;
    at_join(&mut r.runtime, "one", &definition).await;
    assert!(command(&r.runtime, Command::Queue { item: "one".into() }).result.is_ok());
    until(&mut r.runtime, |s| phase_of(s, "one") == Some(Phase::Done)).await;
    let pushed = head(&r.bare).await;
    assert_ne!(pushed, first, "the remote's main moved");
    assert_eq!(adapters::git(&r.bare, &["rev-parse", &format!("{pushed}^")]).await.unwrap(), first, "one commit on the base it was gated on");
    let calls = calls(&r);
    assert!(calls.contains("git-upload-pack") && calls.contains("git-receive-pack"), "fetched and pushed through the user's ssh: {calls}");
    // What the push ran with is all in its receipt: no credential of the engine's own, nothing inherited.
    let push = r.runtime.store.workflow_outbox("w").unwrap().into_iter().find(|s| s.effect.kind == EffectKind::Execute(StageKind::Push)).unwrap();
    let process: serde_json::Value = serde_json::from_slice(&std::fs::read(r.runtime.root.join("receipts")
        .join(format!("{}.push-process.json", digest(&push.effect)))).unwrap()).unwrap();
    for key in process["environment"].as_object().unwrap().keys() {
        assert!(key.starts_with("GIT_") || ["HOME", "PATH", "SSH_AUTH_SOCK"].contains(&key.as_str()), "unexpected {key}");
    }
    let config: Vec<&str> = process["environment"].as_object().unwrap().iter()
        .filter(|(k, _)| k.starts_with("GIT_CONFIG_KEY_")).filter_map(|(_, v)| v.as_str()).collect();
    assert!(config.contains(&"protocol.ssh.allow") && !config.iter().any(|k| k.starts_with("protocol.https") || k.starts_with("credential.")), "{config:?}");
    // A plain fast-forward: no force, no lease, no `+`. The remote refuses anything else on its own.
    let argv: Vec<&str> = process["argv"].as_array().unwrap().iter().filter_map(|a| a.as_str()).collect();
    assert!(!argv.iter().any(|a| a.starts_with("--force") || a.starts_with('+') || *a == "-f"), "{argv:?}");
}

/// ov-1032: the landing URL a first draft suggests for a GitHub origin, taken exactly as `workflow_starter` writes it,
/// takes a ticket from its build to landed on a scratch remote. The stand-in ssh answers `github.com` from a scratch
/// bare repository, so the real origin is never touched.
#[tokio::test]
async fn the_landing_url_a_first_draft_suggests_takes_a_ticket_to_landed_on_a_scratch_remote() {
    let url = crate::workflow_starter::landing_url("git@github.com:scratch/repo.git", "scratch/repo");
    assert_eq!(url, "ssh://git@github.com/scratch/repo.git");
    let mut r = remote_at("pass", Some((&url, "/scratch/repo.git"))).await;
    let definition = batched_push(&r.runtime, "any", 5000);
    let first = head(&r.bare).await;
    at_join(&mut r.runtime, "one", &definition).await;
    assert!(command(&r.runtime, Command::Queue { item: "one".into() }).result.is_ok());
    until(&mut r.runtime, |s| phase_of(s, "one") == Some(Phase::Done)).await;
    assert_ne!(head(&r.bare).await, first, "the scratch remote's main moved");
    let calls = calls(&r);
    assert!(calls.contains("git-upload-pack '/scratch/repo.git'") && calls.contains("git-receive-pack '/scratch/repo.git'"), "{calls}");
}

#[tokio::test]
async fn a_push_the_remote_turns_down_at_sign_in_fails_the_run_with_a_sentence() {
    let mut r = remote("pass").await;
    let definition = batched_push(&r.runtime, "any", 5000);
    let first = head(&r.bare).await;
    at_join(&mut r.runtime, "one", &definition).await;
    std::fs::write(r._dir.path().join("refuse"), b"").unwrap();
    command(&r.runtime, Command::Queue { item: "one".into() });
    until(&mut r.runtime, |s| phase_of(s, "one") == Some(Phase::Failed)).await;
    assert_eq!(head(&r.bare).await, first, "nothing landed");
    assert_eq!(outbox(&r.runtime, StageKind::Push), 1, "a sign-in failure isn't tried again from the base");
    let fact = push_fact(&r.runtime);
    assert_eq!(fact["label"], "rejected");
    assert_eq!(fact["detail"]["why"], "The landing remote turned down this runner's sign-in, so nothing was sent. Check its git and ssh credentials.");
}

#[tokio::test]
async fn a_protected_branch_that_refuses_the_push_fails_the_run_with_a_sentence_not_a_retry() {
    let mut r = remote("pass").await;
    let hook = r.bare.join("hooks/pre-receive");
    std::fs::write(&hook, "#!/bin/sh\necho 'GH006: Protected branch update failed for refs/heads/main.' >&2\nexit 1\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    let definition = batched_push(&r.runtime, "any", 5000);
    let first = head(&r.bare).await;
    at_join(&mut r.runtime, "one", &definition).await;
    command(&r.runtime, Command::Queue { item: "one".into() });
    until(&mut r.runtime, |s| phase_of(s, "one") == Some(Phase::Failed)).await;
    assert_eq!(head(&r.bare).await, first);
    assert_eq!(outbox(&r.runtime, StageKind::Push), 1, "a refusal with the target unmoved is not a moved base");
    let fact = push_fact(&r.runtime);
    assert_eq!(fact["label"], "rejected");
    // GitHub's protected branch is a hook it runs; its own words name it.
    assert_eq!(fact["detail"]["why"], "The landing remote's branch protection refused the push. Nothing landed.");
}

#[tokio::test]
async fn config_an_agent_writes_in_the_shared_repository_steers_no_call_to_the_remote() {
    let mut r = remote("pass").await;
    let definition = batched_push(&r.runtime, "any", 5000);
    let first = head(&r.bare).await;
    at_join(&mut r.runtime, "one", &definition).await;
    // An agent's worktree shares this config. Every key here would send a call elsewhere or run a program next to
    // the runner's credentials, if the call read this config.
    let decoy = r._dir.path().join("decoy.git");
    plain_git(&r.runtime.repository(), &["init", "--bare", "--quiet", decoy.to_str().unwrap()]);
    let url = r.runtime.coding.as_ref().unwrap().landing.as_ref().unwrap().repository.to_string_lossy().into_owned();
    let marker = r._dir.path().join("ran");
    // An alternates file is what makes git run core.alternateRefsCommand on a fetch into this repository.
    let common = r.runtime.repository().join(".git");
    std::fs::write(common.join("objects/info/alternates"), format!("{}\n", decoy.join("objects").display())).unwrap();
    for (key, value) in [(format!("remote.{url}.pushurl"), decoy.to_string_lossy().into_owned()), (format!("remote.{url}.url"), decoy.to_string_lossy().into_owned()),
        ("core.sshCommand".into(), format!("touch {}", marker.display())), ("credential.helper".into(), format!("!touch {}", marker.display())),
        ("core.alternateRefsCommand".into(), format!("touch {}", marker.display()))] {
        plain_git(&r.runtime.repository(), &["config", "--local", &key, &value]);
    }
    command(&r.runtime, Command::Queue { item: "one".into() });
    until(&mut r.runtime, |s| phase_of(s, "one") == Some(Phase::Done)).await;
    assert_ne!(head(&r.bare).await, first, "the push went to the configured remote");
    assert!(adapters::git(&decoy, &["rev-parse", "--verify", "--quiet", "refs/heads/main"]).await.is_err(), "and nowhere else");
    assert!(!marker.exists(), "no program the shared config named ran");
    assert!(calls(&r).contains("git-receive-pack"), "through the user's own ssh");
}

#[tokio::test]
async fn a_url_rewrite_in_the_users_own_config_stops_the_remote_before_any_call() {
    let mut r = remote("pass").await;
    let home = r.runtime.user_home.clone().unwrap();
    let mut config = std::fs::read_to_string(home.join(".gitconfig")).unwrap();
    config.push_str("[url \"ssh://elsewhere/\"]\n\tinsteadOf = ssh://git@farcooler-test/\n");
    std::fs::write(home.join(".gitconfig"), config).unwrap();
    let definition = batched_push(&r.runtime, "any", 5000);
    let first = head(&r.bare).await;
    // The ticket's start is the first read of the remote (ov-1059), so it stops there, before any build.
    super::concurrency_tests::enqueue_item(&mut r.runtime, "w", "one", definition);
    until(&mut r.runtime, |s| phase_of(s, "one") == Some(Phase::Failed)).await;
    assert_eq!(outbox(&r.runtime, StageKind::Build), 0);
    assert_eq!(head(&r.bare).await, first);
    assert_eq!(calls(&r), "", "no call reached the remote");
}

#[test]
fn a_remote_landing_is_an_ssh_or_https_url_with_no_credentials_and_no_config_digest() {
    use super::coding::sound_remote_url;
    for good in ["ssh://git@github.com/taneliang/farcooler.git", "https://github.com/taneliang/farcooler.git", "ssh://github.com:22/o/r"] {
        assert!(sound_remote_url(good), "{good}");
    }
    for bad in ["git@github.com:taneliang/farcooler.git", "https://x-access-token:ghp_secret@github.com/o/r", "https://ghp_secret@github.com/o/r",
        "ssh://git:hunter2@github.com/o/r", "ssh://git%3Atoken@github.com/o/r", "ssh://-oProxyCommand=evil/o/r", "ssh://-x@github.com/o/r", "git://github.com/o/r", "file:///tmp/r.git",
        "ext::sh -c evil", "https://github.com", "https://github.com/o/r?x=1", "https://github.com/o/r#x", "https://github.com/o r", "ssh://github.com/o/r\n"] {
        assert!(!sound_remote_url(bad), "{bad}");
    }
}

#[test]
fn the_private_repository_keeps_what_init_wrote_and_nothing_sends_the_url_elsewhere() {
    use super::coding_landing::steering;
    assert!(steering("system\tcredential.helper=osxkeychain\nglobal\tcore.sshcommand=ssh -i k\nglobal\thttp.proxy=x\nglobal\tremote.origin.url=x\nlocal\tcore.bare=true\nlocal\tcore.repositoryformatversion=0\n").is_ok());
    for refused in ["global\turl.git@github.com:.insteadof=https://github.com/", "system\turl.x.pushinsteadof=ssh://y/", "global\tremote.ssh://github.com/o/r.pushurl=ssh://e/x",
        "local\tcredential.helper=!evil", "worktree\tcore.sshcommand=evil", "local\tcore.alternaterefscommand=evil", "local\tremote.origin.url=x"] {
        assert!(steering(refused).is_err(), "{refused}");
    }
}


#[tokio::test]
async fn a_hook_named_in_the_shared_config_after_the_guard_listing_runs_on_neither_transfer() {
    let r = remote("pass").await;
    let shared = r.runtime.repository();
    let head = adapters::git(&shared, &["rev-parse", "HEAD"]).await.unwrap();
    // The environment is read first, as a call does; the hook lands after, as an agent racing the listing would.
    let (program, environment) = r.runtime.transfer_environment().await.unwrap();
    let marker = r._dir.path().join("hook-ran");
    for event in ["reference-transaction", "pre-push", "post-checkout", "post-index-change"] {
        plain_git(&shared, &["config", "--local", &format!("hook.raced-{event}.event"), event]);
        plain_git(&shared, &["config", "--local", &format!("hook.raced-{event}.command"), &format!("touch {}", marker.display())]);
    }
    // Into the shared repository: objects only, no ref, so no event fires.
    let private = r.runtime.landing_repository().await.unwrap();
    plain_git(&private, &["fetch", "-q", shared.to_str().unwrap(), &format!("{head}:refs/heads/staged")]);
    assert!(!marker.exists(), "set-up ran it");
    assert!(r.runtime.import(&head, &environment, &program).await.unwrap());
    assert!(!marker.exists(), "the import ran it");
    // Into the private one: the shared repository only serves objects.
    r.runtime.stage(&head, "refs/farcooler/push/raced").await.unwrap();
    assert!(!marker.exists(), "a hook the guard listing never saw ran");
}

#[tokio::test]
async fn a_sign_in_the_remote_turns_down_on_every_call_ends_the_wait_with_a_sentence_not_a_timeout() {
    let mut r = remote("pass").await;
    let definition = batched_push(&r.runtime, "any", 3000);
    let first = head(&r.bare).await;
    at_join(&mut r.runtime, "one", &definition).await;
    std::fs::write(r._dir.path().join("refuse-all"), b"").unwrap();
    command(&r.runtime, Command::Queue { item: "one".into() });
    until(&mut r.runtime, |s| phase_of(s, "one") == Some(Phase::Failed)).await;
    assert_eq!(head(&r.bare).await, first);
    let fact = fact(&r.runtime, StageKind::AwaitBase);
    assert_eq!(fact["label"], "rejected", "{fact}");
    assert_eq!(fact["detail"]["polls"], 1, "it didn't wait out its patience");
    assert_eq!(fact["detail"]["why"], "The landing remote turned down this runner's sign-in, so nothing was sent. Check its git and ssh credentials.");
    assert_eq!(outbox(&r.runtime, StageKind::Push), 0);
}

/// The push of a group whose candidate an agent swapped after the gate for `forged`: a commit with the gated tree
/// that doesn't build on the target's head. With `graft`, the shared repository is told it does. Returns the
/// binding the push runs with, and the target's head.
async fn forged(runtime: &mut Runtime, graft: bool) -> (Binding, String) {
    let definition = batched_push(runtime, "any", 5000);
    at_join(runtime, "one", &definition).await;
    command(runtime, Command::Queue { item: "one".into() });
    for time in 2..5 { runtime.pump(time).await.unwrap(); }
    let mut binding = admitted(runtime).await;
    assert_eq!(binding.effect.kind, EffectKind::Execute(StageKind::Push));
    let base = binding.base.clone();
    let gated = binding.effect.candidate.clone().unwrap();
    let shared = runtime.repository();
    let forged = adapters::git(&shared, &["-c", "user.name=Agent", "-c", "user.email=agent@localhost", "commit-tree",
        &format!("{gated}^{{tree}}"), "-m", "forged"]).await.unwrap();
    if graft { plain_git(&shared, &["replace", "--graft", &forged, &base]); }
    let tree = runtime.tree(&binding);
    let branch = adapters::git(&tree, &["symbolic-ref", "--short", "HEAD"]).await.unwrap();
    plain_git(&tree, &["checkout", "--quiet", "-B", &branch, &forged]);
    std::fs::write(runtime.fold_index(&binding, &forged), serde_json::to_vec(&serde_json::json!({
        "item": binding.effect.item, "base": base, "folded": forged })).unwrap()).unwrap();
    binding.effect.candidate = Some(forged);
    (binding, base)
}
async fn refuses_a_forged_candidate(runtime: &mut Runtime, bare: &Path, graft: bool) {
    let (binding, base) = forged(runtime, graft).await;
    let shared = runtime.repository();
    if graft {
        let forged = binding.effect.candidate.as_deref().unwrap();
        assert!(crate::git::git(&shared, &["merge-base", "--is-ancestor", &base, forged]).await.unwrap().ok, "the shared repository believes the graft");
    }
    let (_hold, cancel) = tokio::sync::watch::channel(false);
    let _ = runtime.execute_verb(&binding, &StageKind::Push, cancel).await;
    assert_eq!(head(bare).await, base, "main lost nothing");
    assert!(!runtime.root.join("receipts").join(format!("{}.push-process.json", digest(&binding.effect))).exists(), "nothing was sent");
    let fact: serde_json::Value = serde_json::from_slice(&std::fs::read(runtime.verb_path(&binding)).unwrap()).unwrap();
    assert_eq!(fact["label"], "rejected");
    assert_eq!(fact["detail"]["why"], "The landing candidate doesn't build on the target's head, so nothing was sent.");
}

#[tokio::test]
async fn a_candidate_off_the_target_is_refused_before_the_push_even_when_the_shared_repository_is_grafted_to_say_otherwise() {
    for graft in [true, false] {
        let mut r = remote("pass").await;
        refuses_a_forged_candidate(&mut r.runtime, &r.bare.clone(), graft).await;
        assert!(!calls(&r).contains("git-receive-pack"), "graft {graft}: no push reached the remote");
        let (dir, mut runtime) = lander("pass").await;
        refuses_a_forged_candidate(&mut runtime, &dir.path().join("private-remote.git"), graft).await;
    }
}

#[tokio::test]
async fn whether_a_commit_landed_is_read_in_the_private_repository_not_the_grafted_shared_one() {
    let mut r = remote("pass").await;
    let definition = batched_push(&r.runtime, "any", 5000);
    at_join(&mut r.runtime, "one", &definition).await;
    command(&r.runtime, Command::Queue { item: "one".into() });
    until(&mut r.runtime, |s| phase_of(s, "one") == Some(Phase::Done)).await;
    let landed = head(&r.bare).await;
    let shared = r.runtime.repository();
    let never = adapters::git(&shared, &["-c", "user.name=Agent", "-c", "user.email=agent@localhost", "commit-tree",
        &format!("{landed}^{{tree}}"), "-m", "never pushed"]).await.unwrap();
    // The shared repository now says the remote's head is built on a commit that was never pushed.
    let parent = adapters::git(&shared, &["rev-parse", &format!("{landed}^")]).await.unwrap();
    plain_git(&shared, &["replace", "--graft", &landed, &parent, &never]);
    assert!(crate::git::git(&shared, &["merge-base", "--is-ancestor", &never, &landed]).await.unwrap().ok);
    let identity = r.runtime.coding.as_ref().unwrap().landing.clone().unwrap();
    let log = r._dir.path().join("on-target.output");
    assert!(r.runtime.on_target(&identity, &landed, &log).await.unwrap());
    assert!(!r.runtime.on_target(&identity, &never, &log).await.unwrap(), "a graft in the shared repository is not a landing");
}

#[tokio::test]
async fn an_alternate_refs_command_in_the_shared_config_runs_on_no_fetch_into_it() {
    let (dir, mut runtime) = lander("pass").await;
    let definition = batched_push(&runtime, "any", 5000);
    at_join(&mut runtime, "one", &definition).await;
    // A local landing fetches the target into the shared repository; with an alternates file there, git asks
    // this program for the alternate's refs. An agent can write both.
    let shared = runtime.repository();
    let decoy = dir.path().join("decoy.git");
    plain_git(&shared, &["init", "--bare", "--quiet", decoy.to_str().unwrap()]);
    std::fs::write(shared.join(".git/objects/info/alternates"), format!("{}\n", decoy.join("objects").display())).unwrap();
    let marker = dir.path().join("ran");
    plain_git(&shared, &["config", "--local", "core.alternateRefsCommand", &format!("touch {}", marker.display())]);
    command(&runtime, Command::Queue { item: "one".into() });
    until(&mut runtime, |s| phase_of(s, "one") == Some(Phase::Done)).await;
    assert!(!marker.exists(), "the shared config's alternate refs command ran");
}

/// Another lander moves the remote's main on, and this repository never hears of it: its own main, and the
/// configured base read from it, are now behind. Answers (that local main, the remote's new head).
async fn move_remote(r: &Remote) -> (String, String) {
    let config = r.runtime.coding.clone().unwrap();
    let other = r._dir.path().join("elsewhere");
    // The bare's HEAD names whatever branch git defaults to (`master` on CI's runners), not `main`: clone `main` by name.
    plain_git(r._dir.path(), &["clone", "--quiet", "--branch", "main", r.bare.to_str().unwrap(), other.to_str().unwrap()]);
    std::fs::write(other.join("landed-elsewhere.txt"), "newer main\n").unwrap();
    plain_git(&other, &["add", "landed-elsewhere.txt"]);
    plain_git(&other, &["-c", "user.name=T", "-c", "user.email=t@localhost", "-c", "commit.gpgsign=false", "commit", "--quiet", "-m", "newer main"]);
    plain_git(&other, &["-c", "core.hooksPath=/dev/null", "push", "--quiet", "origin", "HEAD:refs/heads/main"]);
    let moved = head(&r.bare).await;
    let local = adapters::git(&config.repository, &["rev-parse", &config.landing_ref]).await.unwrap();
    assert_eq!(local, config.base, "the configured base is the local main");
    assert_ne!(moved, config.base, "the remote is ahead of it");
    assert!(adapters::git(&config.repository, &["cat-file", "-e", &format!("{moved}^{{commit}}")]).await.is_err(), "and the local repository doesn't have it");
    (local, moved)
}

/// ov-1058: the shared repository's own main is weeks behind the remote's, as a checkout nobody pulls is. The batch
/// still lands on the remote's main as it is now: the base step fetches it, the fold builds on it, and neither the
/// configured base nor the local main it was read from goes anywhere near the push.
#[tokio::test]
async fn a_batch_folds_onto_the_remotes_main_when_the_local_main_is_stale() {
    let mut r = remote("pass").await;
    let definition = batched_push(&r.runtime, "any", 5000);
    let config = r.runtime.coding.clone().unwrap();
    let (local, moved) = move_remote(&r).await;
    at_join(&mut r.runtime, "one", &definition).await;
    assert!(command(&r.runtime, Command::Queue { item: "one".into() }).result.is_ok());
    until(&mut r.runtime, |s| phase_of(s, "one") == Some(Phase::Done)).await;
    let pushed = head(&r.bare).await;
    assert_eq!(adapters::git(&r.bare, &["rev-parse", &format!("{pushed}^")]).await.unwrap(), moved, "the batch built on the remote's main, not the stale one");
    assert_eq!(fact(&r.runtime, StageKind::Fold)["detail"]["base"], moved.as_str(), "{}", fact(&r.runtime, StageKind::Fold));
    assert!(adapters::git(&r.bare, &["cat-file", "-e", &format!("{pushed}:landed-elsewhere.txt")]).await.is_ok(), "the other lander's work is kept");
    assert_eq!(adapters::git(&config.repository, &["rev-parse", &config.landing_ref]).await.unwrap(), local, "the local main is left as it was");
}

/// ov-1059: a ticket on a workspace whose own main is stale starts from the remote's main as it is now. Its base
/// is pinned there, and its build is one commit on top of it, not on the month-old local main.
#[tokio::test]
async fn a_ticket_on_a_stale_local_main_starts_at_the_remotes_main() {
    let mut r = remote("pass").await;
    let definition = batched_push(&r.runtime, "any", 5000);
    let config = r.runtime.coding.clone().unwrap();
    let (local, moved) = move_remote(&r).await;
    at_join(&mut r.runtime, "one", &definition).await;
    let provision = r.runtime.store.workflow_outbox("w").unwrap().into_iter()
        .find(|s| s.effect.kind == EffectKind::Execute(StageKind::Provision)).unwrap();
    assert_eq!(r.runtime.item_base("w", &provision.effect).unwrap(), moved, "the ticket's base is the remote's main");
    let state = r.runtime.state("w").unwrap().unwrap();
    let built = candidate_of(&state, "one").unwrap().to_string();
    assert_eq!(adapters::git(&config.repository, &["rev-parse", &format!("{built}^")]).await.unwrap(), moved, "the build is one commit on it");
    assert!(adapters::git(&config.repository, &["cat-file", "-e", &format!("{built}:landed-elsewhere.txt")]).await.is_ok(), "with the newer main's work in its tree");
    assert_eq!(adapters::git(&config.repository, &["rev-parse", &config.landing_ref]).await.unwrap(), local, "the local main is left as it was");
}

/// ov-1059: a landing remote that can't be read when a ticket starts stops it with a sentence that says so. A
/// Start is refused before anything is made; a ticket already past Start fails at Provision, with no base pinned,
/// no worktree on the stale local main, and no build.
#[tokio::test]
async fn a_remote_that_cant_be_read_at_start_stops_the_ticket_and_says_why() {
    use farcooler_protocol::{workflow_commands::{Action, Mutation}, workflow_config::AdapterConfiguration};
    let mut r = remote("pass").await;
    let definition = batched_push(&r.runtime, "any", 5000);
    let config = r.runtime.coding.clone().unwrap();
    let adapter = Some(Box::new(AdapterConfiguration::Coding(config.clone())));
    let pin = farcooler_store::workflow_publication::configuration_pin(&definition, adapter.as_deref());
    let published = r.runtime.wire_command("w", "owner", Action::Publish { notifications: None, adapter, expected_pin: None,
        mutation: Mutation { id: "publish".into(), revision: 0, epoch: 1, sequence: 1 }, definition: definition.clone() }).unwrap();
    assert!(published.pointer("/receipt/result/Ok").is_some(), "{published}");
    let start = |id: &str| Action::Start { mutation: Mutation { id: id.into(), revision: 1, epoch: 1, sequence: 2 }, item: "ov-7".into(),
        run: "r".into(), pin: "p".into(), start_pin: pin.clone(), models: Default::default(), worktree: None, with: vec![] };
    // A remote that answers: the Start goes ahead (a private tree is made at Provision).
    assert!(r.runtime.prepare_start("w", "orchestrator", &start("answered")).await.unwrap().is_none());
    std::fs::write(r._dir.path().join("refuse-all"), b"").unwrap();
    match r.runtime.prepare_start("w", "orchestrator", &start("unanswered")).await {
        Err(Error::Domain(refused)) => assert_eq!(refused.to_string(), super::coding_landing::UNREAD_TARGET),
        other => panic!("the Start must be refused with a sentence: {other:?}"),
    }
    // A ticket that reaches Provision with the remote unreadable fails there.
    super::concurrency_tests::enqueue_item(&mut r.runtime, "w", "one", definition);
    until(&mut r.runtime, |s| phase_of(s, "one") == Some(Phase::Failed)).await;
    assert_eq!(outbox(&r.runtime, StageKind::Build), 0, "nothing was built");
    let provision = r.runtime.store.workflow_outbox("w").unwrap().into_iter()
        .find(|s| s.effect.kind == EffectKind::Execute(StageKind::Provision)).unwrap();
    assert!(!r.runtime.root.join(format!("base-{}.json", digest(&("w", &provision.effect.item)))).exists(), "no base was pinned");
    assert!(!r.runtime.item_tree("w", &provision.effect.item).exists(), "no tree was made");
    assert_eq!(r.runtime.state("w").unwrap().unwrap().items["one"].failure.as_deref(), Some(super::coding_landing::UNREAD_TARGET), "the ticket says why");
}

/// ov-1058 review (F7): a network landing never writes a ref in the shared repository, whose main is its owner's.
/// Here the owner is up to date, so the local main sits at the base the push started from; the push still leaves it.
#[tokio::test]
async fn a_batched_push_to_a_remote_leaves_the_shared_repositorys_main_alone() {
    let mut r = remote("pass").await;
    let definition = batched_push(&r.runtime, "any", 5000);
    let config = r.runtime.coding.clone().unwrap();
    let before = adapters::git(&config.repository, &["rev-parse", &config.landing_ref]).await.unwrap();
    assert_eq!(before, head(&r.bare).await, "the owner's main is level with the remote's");
    at_join(&mut r.runtime, "one", &definition).await;
    assert!(command(&r.runtime, Command::Queue { item: "one".into() }).result.is_ok());
    until(&mut r.runtime, |s| phase_of(s, "one") == Some(Phase::Done)).await;
    assert_ne!(head(&r.bare).await, before, "the batch landed");
    assert_eq!(adapters::git(&config.repository, &["rev-parse", &config.landing_ref]).await.unwrap(), before, "the owner's main stayed where it was");
}

/// ov-1058 review (F4, F7): a ticket landed directly (no group) onto a remote, from a stale local main, lands. It
/// is not refused for the local main being elsewhere, and it moves no ref in the shared repository.
#[tokio::test]
async fn a_direct_land_on_a_remote_needs_no_local_main_and_moves_none() {
    let mut r = remote("pass").await;
    let config = r.runtime.coding.clone().unwrap();
    let definition = config.definition().unwrap();
    let (local, moved) = move_remote(&r).await;
    super::concurrency_tests::enqueue_item(&mut r.runtime, "w", "one", definition);
    until(&mut r.runtime, |s| phase_of(s, "one") == Some(Phase::Done)).await;
    let pushed = head(&r.bare).await;
    assert_eq!(adapters::git(&r.bare, &["rev-parse", &format!("{pushed}^")]).await.unwrap(), moved, "it landed on the remote's main");
    assert_eq!(adapters::git(&config.repository, &["rev-parse", &config.landing_ref]).await.unwrap(), local, "the shared repository's main is untouched");
}

/// ov-1058 review (F1): a read of the remote takes a ref of its own in the private repository, so a Provision and
/// a wait's poll reading together can't fail for each other's lock on one shared ref. Here that shared ref is
/// locked, as a read in progress would hold it, and another read must not mind; and none is left behind.
#[tokio::test]
async fn a_read_of_the_remote_does_not_need_the_ref_another_read_holds() {
    let r = remote("pass").await;
    let identity = r.runtime.coding.as_ref().unwrap().landing.clone().unwrap();
    let url = super::coding::landing_url(&identity).unwrap().to_string();
    let expected = head(&r.bare).await;
    let log = |name: &str| r.runtime.root.join("receipts").join(name);
    assert_eq!(r.runtime.fetch_target(&identity, &url, &log("first.output")).await.unwrap().as_deref(), Some(expected.as_str()));
    let private = r.runtime.landing_repository().await.unwrap();
    let held = private.join(format!("{}.lock", Exec::base_ref(&identity)));
    std::fs::create_dir_all(held.parent().unwrap()).unwrap();
    std::fs::write(&held, b"").unwrap();
    let (log_a, log_b) = (log("a.output"), log("b.output"));
    let (a, b) = tokio::join!(r.runtime.fetch_target(&identity, &url, &log_a), r.runtime.fetch_target(&identity, &url, &log_b));
    assert_eq!(a.unwrap().as_deref(), Some(expected.as_str()));
    assert_eq!(b.unwrap().as_deref(), Some(expected.as_str()));
    let left = adapters::git(&private, &["for-each-ref", "refs/farcooler/read", "refs/farcooler/base"]).await.unwrap();
    assert!(left.is_empty(), "reads left refs behind: {left}");
}

/// ov-1058 review (F2): a remote that never answers ends a Start's read in seconds, not minutes.
#[tokio::test]
async fn a_remote_that_hangs_is_given_up_on_at_start_well_inside_the_dispatch_lease() {
    let r = remote("pass").await;
    // The stand-in ssh sleeps instead of answering.
    let ssh = r._dir.path().join("ssh");
    let script = std::fs::read_to_string(&ssh).unwrap().replace("exec /usr/bin/env", "exec sleep 600 #");
    std::fs::write(&ssh, script).unwrap();
    let log = r.runtime.root.join("receipts").join("hang.output");
    let started = std::time::Instant::now();
    assert!(r.runtime.start_head(&log).await.is_err());
    assert!(started.elapsed() < Duration::from_secs(10), "gave up after {:?}", started.elapsed());
}

/// ov-1058 review (F3): a named tree starts at the remote's main too, not at the stale local one.
#[tokio::test]
async fn a_named_tree_starts_at_the_remotes_main_when_the_local_main_is_stale() {
    let r = remote("pass").await;
    let config = r.runtime.coding.clone().unwrap();
    let (_, moved) = move_remote(&r).await;
    let log = r.runtime.root.join("receipts").join("named.output");
    assert_eq!(r.runtime.named_base(&config, &log).await.unwrap(), moved);
}

/// Slow the stand-in remote's pack step only: it answers a head question at once, and takes `seconds` to send objects.
fn slow_pack(r: &Remote, seconds: u64) {
    let hook = r._dir.path().join("slow-pack.sh");
    std::fs::write(&hook, format!("#!/bin/sh\nsleep {seconds}\ntee '{}' | \"$@\"\n", r._dir.path().join("pack-request").display())).unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    let global = r._dir.path().join("remote-global.gitconfig");
    std::fs::write(&global, format!("[uploadpack]\n\tpackObjectsHook = {}\n", hook.display())).unwrap();
    let ssh = r._dir.path().join("ssh");
    let script = std::fs::read_to_string(&ssh).unwrap().replace("-i PATH=", &format!("-i GIT_CONFIG_GLOBAL={} PATH=", global.display()));
    std::fs::write(&ssh, script).unwrap();
}

/// A remote whose pack step waits until the test writes `pack-release`, then fails, as a download cut off part way
/// does. The test decides when, so no clock is raced.
fn failing_pack(r: &Remote) {
    slow_pack(r, 0);
    let release = r._dir.path().join("pack-release");
    std::fs::write(r._dir.path().join("slow-pack.sh"), format!("#!/bin/sh\nwhile [ ! -f '{}' ]; do sleep 0.05; done\nexit 1\n", release.display())).unwrap();
}

/// ov-1058 re-review: a download that outlasts a Start's cap must not stop Main starting tickets. A Start asks the
/// remote only for its head's hash, which a slow download doesn't delay; the objects are fetched by Provision, with
/// the long limit, and the worktree is made on them.
#[tokio::test]
async fn a_slow_download_of_the_remotes_main_does_not_stop_a_ticket_starting() {
    let r = remote("pass").await;
    let config = r.runtime.coding.clone().unwrap();
    let (_, moved) = move_remote(&r).await;
    slow_pack(&r, 5);
    let log = |n: &str| r.runtime.root.join("receipts").join(n);
    let started = std::time::Instant::now();
    assert_eq!(r.runtime.start_head(&log("start.output")).await.unwrap().as_deref(), Some(moved.as_str()), "a Start learns the head");
    assert!(started.elapsed() < Duration::from_secs(3), "the Start waited {:?} for the download", started.elapsed());
    assert_eq!(r.runtime.named_base(&config, &log("named.output")).await.unwrap(), moved);
    // Provision then brings the objects, taking longer than a Start may.
    let fetching = std::time::Instant::now();
    r.runtime.exec.ensure_start_objects(&moved, &log("provision.output")).await.unwrap();
    assert!(fetching.elapsed() >= Duration::from_secs(5), "the stand-in was not slow: {:?}", fetching.elapsed());
    assert!(adapters::git(&config.repository, &["cat-file", "-e", &format!("{moved}^{{commit}}")]).await.is_ok(), "the shared repository holds the base");
}

/// ov-1058 review 3: a download of the remote's main that outlasts Provision's limit fails the ticket plainly. It
/// is tried again, then reads Failed with a sentence that says why, and not Uncertain, which would wait on a person;
/// no tree is made on the stale local main.
#[tokio::test]
async fn a_download_that_outlasts_its_limit_fails_the_ticket_with_a_sentence() {
    let mut r = remote("pass").await;
    let definition = batched_push(&r.runtime, "any", 5000);
    let config = r.runtime.coding.clone().unwrap();
    let (_, moved) = move_remote(&r).await;
    slow_pack(&r, 30);
    super::concurrency_tests::enqueue_item(&mut r.runtime, "w", "one", definition);
    let state = until(&mut r.runtime, |s| matches!(phase_of(s, "one"), Some(Phase::Failed | Phase::Uncertain))).await;
    assert_eq!(phase_of(&state, "one"), Some(Phase::Failed), "a failed download is a plain failure");
    assert_eq!(state.items["one"].failure.as_deref(), Some(super::coding_landing::UNFETCHED_TARGET));
    let provisions: Vec<_> = r.runtime.store.workflow_outbox("w").unwrap().into_iter()
        .filter(|s| s.effect.kind == EffectKind::Execute(StageKind::Provision)).collect();
    assert_eq!(provisions.len(), 2, "tried again before failing");
    assert_eq!(outbox(&r.runtime, StageKind::Build), 0, "nothing was built");
    let item = &provisions[0].effect.item;
    assert_eq!(r.runtime.item_base("w", &provisions[0].effect).unwrap(), moved, "the base was the remote's main");
    assert!(!r.runtime.item_tree("w", item).exists(), "no tree was made");
    let branches = adapters::git(&config.repository, &["for-each-ref", "--format=%(refname)", "refs/heads/wf-*"]).await.unwrap();
    assert!(branches.is_empty(), "no branch was made: {branches}");
    // A restart reads the same answer from what the failed step kept, without downloading again.
    let last = provisions.last().unwrap();
    let binding = Binding::new("w", &last.effect, last.resource_generation.unwrap(), state.items["one"].iteration, &moved);
    assert_eq!(r.runtime.reconcile_coding(&binding).await.unwrap(), Outcome::SafeFailureBecause(super::coding_landing::UNFETCHED_TARGET.into()));
}

/// ov-1058 review 3: Provisions that start together download the remote's main once. The second waits for the
/// first and finds the base already there.
#[tokio::test]
async fn provisions_that_start_together_download_the_remotes_main_once() {
    let r = remote("pass").await;
    let (_, moved) = move_remote(&r).await;
    slow_pack(&r, 2);
    let log = |n: &str| r.runtime.root.join("receipts").join(n);
    let (a, b) = (log("a.output"), log("b.output"));
    let (first, second) = tokio::join!(r.runtime.exec.ensure_start_objects(&moved, &a), r.runtime.exec.ensure_start_objects(&moved, &b));
    first.unwrap(); second.unwrap();
    let asked = calls(&r).lines().filter(|l| l.starts_with("git-upload-pack")).count();
    assert_eq!(asked, 1, "the remote was asked {asked} times: {}", calls(&r));
}

/// ov-1061: Provisions that queued behind a download that failed get its sentence at once, and don't each download
/// again with the full limit while holding one of the four effect slots. The remote holds the download until the test
/// sees all three Provisions hold the remote's record (the leader's and two waiters', queued in the same step on this
/// single thread), then fails it. The count of downloads is the whole answer, with no clock to race.
#[tokio::test]
async fn provisions_waiting_on_a_download_that_fails_fail_with_it_at_once() {
    let r = remote("pass").await;
    let (_, moved) = move_remote(&r).await;
    failing_pack(&r);
    let log = |n: &str| r.runtime.root.join("receipts").join(n);
    let (a, b, c) = (log("a.output"), log("b.output"), log("c.output"));
    let exec = &r.runtime.exec;
    let release = async {
        // The map's own handle and one per Provision: all three are in the queue when the count reaches 4.
        while exec.seeding.lock().unwrap().values().map(std::sync::Arc::strong_count).max() != Some(4) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        std::fs::write(r._dir.path().join("pack-release"), "").unwrap();
    };
    let (first, second, third, ()) = tokio::time::timeout(Duration::from_secs(60), async {
        tokio::join!(exec.ensure_start_objects(&moved, &a), exec.ensure_start_objects(&moved, &b),
            exec.ensure_start_objects(&moved, &c), release)
    }).await.expect("the three Provisions never queued behind one download");
    for failed in [first, second, third] { assert_eq!(failed.unwrap_err(), super::coding_landing::UNFETCHED_TARGET); }
    let asked = calls(&r).lines().filter(|l| l.starts_with("git-upload-pack")).count();
    assert_eq!(asked, 1, "the remote was asked {asked} times: {}", calls(&r));
    // A Provision that comes once the failed download has ended tries for itself.
    slow_pack(&r, 0);
    exec.ensure_start_objects(&moved, &log("d.output")).await.unwrap();
    assert_eq!(calls(&r).lines().filter(|l| l.starts_with("git-upload-pack")).count(), 2);
}

/// ov-1058 re-review: the fetch carries only what is new. The local main is staged into the private repository first,
/// so the remote is asked for the commits past it and not for all of main.
#[tokio::test]
async fn the_first_download_is_seeded_from_the_shared_repositorys_own_main() {
    let r = remote("pass").await;
    let (local, moved) = move_remote(&r).await;
    let log = r.runtime.root.join("receipts").join("seed.output");
    slow_pack(&r, 0);
    r.runtime.exec.ensure_start_objects(&moved, &log).await.unwrap();
    let asked = std::fs::read_to_string(r._dir.path().join("pack-request")).expect("the remote was asked for a pack");
    assert!(asked.lines().any(|l| l == local), "the fetch did not offer the local main as already held: {asked}");
    let private = r.runtime.exec.landing_repository().await.unwrap();
    assert!(adapters::git(&private, &["cat-file", "-e", &format!("{local}^{{commit}}")]).await.is_ok(), "the local main is already in the private repository");
    let staged = adapters::git(&private, &["for-each-ref", "--format=%(objectname)", "refs/farcooler/shared/local"]).await.unwrap();
    assert_eq!(staged, local);
}

/// ov-1058 re-review (F3): a Start that names a worktree is prepared through `prepare_start`, which pins the remote's
/// main as the tree's base and not the stale local one.
#[tokio::test]
async fn a_start_that_names_a_worktree_pins_the_remotes_main_not_the_stale_one() {
    use farcooler_protocol::{workflow_commands::{Action, Mutation, NamedTree}, workflow_config::AdapterConfiguration};
    let mut r = remote("pass").await;
    let definition = batched_push(&r.runtime, "any", 5000);
    let config = r.runtime.coding.clone().unwrap();
    let (_, moved) = move_remote(&r).await;
    let host_id = Uuid::now_v7();
    let root = r.runtime.store.create_repository_root(host_id, config.repository.to_str().unwrap(), 0).unwrap();
    let repository = r.runtime.store.create_repository(host_id, root.id, "demo", config.repository.join(".git").to_str().unwrap(), "").unwrap();
    let board = r.runtime.store.create_workspace(repository.id, "Demo", "dem").unwrap().id.to_string();
    r.runtime.host = Some(WorktreeHost { dir: r._dir.path().join("trees"), install_id: "i".into(), locks: Default::default() });
    let adapter = Some(Box::new(AdapterConfiguration::Coding(config.clone())));
    let pin = farcooler_store::workflow_publication::configuration_pin(&definition, adapter.as_deref());
    let published = r.runtime.wire_command(&board, "owner", Action::Publish { notifications: None, adapter, expected_pin: None,
        mutation: Mutation { id: "publish".into(), revision: 0, epoch: 1, sequence: 1 }, definition: definition.clone() }).unwrap();
    assert!(published.pointer("/receipt/result/Ok").is_some(), "{published}");
    let start = Action::Start { mutation: Mutation { id: "named".into(), revision: 1, epoch: 1, sequence: 2 }, item: "ov-7".into(),
        run: "r".into(), pin: "p".into(), start_pin: pin, models: Default::default(),
        worktree: Some(NamedTree { name: "ov-7".into(), branch: "ov-7-work".into(), adopt: false }), with: vec![] };
    let tree = r.runtime.prepare_start(&board, "orchestrator", &start).await.unwrap().expect("a named tree");
    assert_eq!(tree.base, moved, "the tree's base is the remote's main");
}
