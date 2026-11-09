//! A landing target that is a network remote (ov-1026), reached over `ssh://` with the runner user's own git and
//! ssh setup. The remote is a private bare repository; the "ssh" is a stand-in named by `core.sshCommand` in a
//! scratch home's own global git config, which runs the remote's end locally with an empty environment, as a
//! real ssh would. Nothing here reaches the network.
use super::*;
use super::coding_verbs_tests::{at_join, batched_push, command, lander, outbox, phase_of, until};
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
    at_join(&mut r.runtime, "one", &definition).await;
    command(&r.runtime, Command::Queue { item: "one".into() });
    until(&mut r.runtime, |s| phase_of(s, "one") == Some(Phase::Failed)).await;
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
