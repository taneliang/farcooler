//! The adapter settings a runner suggests for a workspace that has never published a workflow (ov-1010).
//!
//! A draft pins a definition to adapter settings, so a workspace with none had nothing to draft against and
//! `workspace workflow-draft` refused. The runner knows what a first draft needs: the workspace's checkout, the
//! branch it lands on, its own Claude hook settings and Codex hooks, and the GitHub repository CI runs in. It says them
//! here, in `workflow get`'s reply, and suggests only. The things written are the engine's Claude hook settings file
//! (`engine_hook_settings`) and its Codex hooks file (`engine_codex_hooks`, ov-1088), atomically and only when they
//! differ, even when the suggestion then refuses: they are the runner's own files, the same for every workspace, and
//! hold nothing of this one. Every `workflow get` reply, a published workspace's too, says the Codex one as
//! `engine_codex_hooks`, which is what a published adapter adds to run Codex models. The orchestrator drafts on top of them and
//! the owner reads every setting on the draft's card before anything publishes (`workflow_drafts`).
//!
//! **It lands on origin.** Every template lands through a group, and a group holds a landing transport. Since the
//! engine can push to a remote (ov-1026), the suggestion's is origin itself: an `ssh://` URL for an ssh origin and an
//! `https://` one for an https origin, signed in with the runner user's own git and ssh (ov-1032). A workspace whose
//! origin can't be read as a GitHub repository has no CI to wait on and nothing to land on, so the suggestion says that.
use crate::landing_templates::GATE_PATH;
use crate::service::Service;
use farcooler_protocol::workflow_config::CodingLanding;
use crate::workflow_runtime::{CodingCi, CodingCommand, CodingConfig, CodingHarness, LaunchFile, MAX_COMMAND_MS};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// The harness a first draft runs its agents with.
pub const HARNESS: &str = "claude";
/// The models a build step may run with, its default first: Sonnet builds.
pub const BUILD_MODELS: [&str; 2] = ["claude:sonnet", "claude:opus"];
/// The models an agent's review gate may run with, its default first: Opus reviews.
pub const REVIEW_MODELS: [&str; 2] = ["claude:opus", "claude:sonnet"];
/// The name of the landing a first draft pushes through.
pub const LANDING_JOB: &str = "landing-main";
/// The CI workflow a first draft waits for, by its name on GitHub.
pub const CI_WORKFLOW: &str = "CI";
/// What a build agent is told, after the card it works (`briefed` leads with that).
pub const PROMPT: &str = "Work this ticket to its acceptance items and within its constraints. Run the tests your change \
    can affect. Commit your work on this branch, with a message that names the ticket, then end your turn.";

/// One look at CI on `$FARCOOLER_CANDIDATE`: the newest run GitHub has for that commit of the workflow the engine names
/// (`$FARCOOLER_CI_WORKFLOW`, ov-1071), else of `$CI_WORKFLOW`, or the exact run a CI retry follows (`$FARCOOLER_CI_RUN`),
/// written as the receipt the CI step reads. A look that fails exits non-zero; one that finds no run for the commit
/// succeeds and writes nothing, which a CI wait reads as a workflow this push didn't start.
pub const CI_LOOK: &str = r#"if [ -n "$FARCOOLER_CI_RUN" ]; then
ids=$("$CI_GH" api "repos/$CI_REPOSITORY/actions/runs/$FARCOOLER_CI_RUN" --jq '[.id, .run_attempt] | @tsv') || exit 1
[ -n "$ids" ] || exit 1
else
ids=$("$CI_GH" api "repos/$CI_REPOSITORY/actions/runs?head_sha=$FARCOOLER_CANDIDATE&event=$CI_EVENT&per_page=50" --jq '[.workflow_runs[] | select(.name == (env.FARCOOLER_CI_WORKFLOW // env.CI_WORKFLOW))] | max_by(.id) // empty | [.id, .run_attempt] | @tsv') || exit 1
[ -n "$ids" ] || exit 0
fi
read -r run attempt <<IDS
$ids
IDS
out=$("$CI_GH" api "repos/$CI_REPOSITORY/actions/runs/$run/attempts/$attempt" --jq '{binding: env.FARCOOLER_OPERATION_BINDING, repository: env.CI_REPOSITORY, run: {id, name, head_sha, status, conclusion, html_url, created_at, display_title, event, updated_at, run_attempt}}') || exit 1
jobs=$("$CI_GH" api --paginate --slurp "repos/$CI_REPOSITORY/actions/runs/$run/attempts/$attempt/jobs?per_page=100" --jq '[.[].jobs[] | select(.conclusion == "failure" or .conclusion == "timed_out" or .conclusion == "action_required" or .conclusion == "startup_failure" or .conclusion == "stale") | .name]') || exit 1
printf '%s,"failed_jobs":%s}\n' "${out%?}" "$jobs" > "$FARCOOLER_RECEIPT""#;

/// `workflow get`'s reply, for a workspace that has never published: the settings this runner suggests for its first
/// draft as `suggested_adapter`, or why it can't as `suggested_adapter_refused`. Only a `get` reply carries a
/// `configuration`, and a published workspace's isn't null, so every other reply is left alone.
pub(crate) async fn add_to_get(svc: &Service, workspace: Uuid, mut value: serde_json::Value) -> serde_json::Value {
    // Every reply, a published workspace's too: Codex's hooks are what a published adapter pins to add Codex models (ov-1088).
    if value.get("configuration").is_some() {
        if let (Some(reply), Some(hooks)) = (value.as_object_mut(), engine_codex_hooks(svc)) {
            reply.insert("engine_codex_hooks".into(), serde_json::json!(hooks));
        }
    }
    if value.get("configuration").is_none_or(|c| !c.is_null()) { return value; }
    let said = match settings(svc, workspace).await {
        Ok(config) => ("suggested_adapter", serde_json::json!(farcooler_protocol::workflow_config::AdapterConfiguration::Coding(config))),
        Err(why) => ("suggested_adapter_refused", why.into()),
    };
    if let Some(reply) = value.as_object_mut() { reply.insert(said.0.into(), said.1); }
    value
}

/// Everything a first draft pins to, landing on origin's default branch, or why there are none, in a sentence a person reads.
pub async fn settings(svc: &Service, workspace: Uuid) -> Result<CodingConfig, String> {
    let gone = |_| "Far Cooler couldn’t read this workspace.".to_string();
    let ws = svc.store.get_workspace(workspace).map_err(gone)?;
    let repo = svc.store.get_repository(ws.repository_id).map_err(gone)?;
    let git_dir = PathBuf::from(&repo.canonical_git_dir);
    let repository = checkout(&git_dir).ok_or_else(|| format!("{} isn’t a checkout Far Cooler can work in: its .git must be a directory, not a worktree’s file.", git_dir.display()))?;
    let git = farcooler_core::programs::find("git").ok_or("This runner has no git.")?;
    let branch = default_branch(&git, &repository).await;
    let landing_ref = format!("refs/heads/{branch}");
    let base = run(&git, &repository, &["rev-parse", "--verify", "--quiet", &format!("{landing_ref}^{{commit}}")]).await
        .ok_or_else(|| format!("This repository has no {branch} branch to land on."))?;
    let origin = run(&git, &repository, &["remote", "get-url", "origin"]).await.unwrap_or_default();
    // Where pushes go: origin's own push URL when it has one, else its fetch URL. CI is looked at on the fetch URL's repository.
    let push = run(&git, &repository, &["remote", "get-url", "--push", "origin"]).await.unwrap_or_else(|| origin.clone());
    for url in [&origin, &push] { if carries_credentials(url) { return Err(TOKEN_IN_URL.into()); } }
    let ci_repository = github_repository(&origin).ok_or("Its origin isn’t on GitHub, so there’s no CI for a draft to wait on. Write the settings yourself and publish them with `workspace set-workflow`.")?;
    let landing_repository = github_repository(&push).ok_or("Its origin pushes somewhere other than GitHub, so a draft has nowhere to land. Write the settings yourself and publish them with `workspace set-workflow`.")?;
    let landing = CodingLanding { job: LANDING_JOB.into(), repository: landing_url(&push, &landing_repository).into(),target: landing_ref.clone(), config_digest: String::new() };
    let gh = farcooler_core::programs::find("gh").ok_or("This runner has no gh, which the CI step looks at GitHub with.")?;
    let settings = engine_hook_settings(svc).ok_or("Far Cooler couldn’t write the engine’s Claude hook settings on this runner.")?;
    let settings = LaunchFile { digest: farcooler_workflow::digest(&std::fs::read(&settings).map_err(|_| "Far Cooler couldn’t read its Claude hook settings.")?),
        path: settings.canonicalize().map_err(|_| "Far Cooler couldn’t read its Claude hook settings.")? };
    let codex_hooks = engine_codex_hooks(svc).ok_or("Far Cooler couldn’t write the engine’s Codex hooks on this runner.")?;
    let environment = crate::landing_templates::gate_environment(&[]).map_err(|e| e.to_string())?;
    let mut agent = environment.clone();
    if let Some(home) = environment.get("HOME") { agent.insert("PATH".into(), format!("{GATE_PATH}:{home}/.local/bin:{home}/.cargo/bin")); }
    let mut look = environment;
    look.extend([("CI_GH".to_string(), gh.to_string_lossy().into_owned()), ("CI_REPOSITORY".into(), ci_repository.clone()),
        ("CI_WORKFLOW".into(), CI_WORKFLOW.into()), ("CI_EVENT".into(), "push".into())]);
    Ok(CodingConfig { repository, base, landing_ref, landing: Some(landing), pull_request: None,
        harness: CodingHarness::Native { preset: HARNESS.into(), settings: Some(settings), project_hooks: None, codex_hooks: Some(codex_hooks), plugin_dir: None, prompt: PROMPT.into(),
            process: CodingCommand { argv: Vec::new(), environment: agent, timeout_ms: MAX_COMMAND_MS } },
        checks: Vec::new(), reviewer: None,
        ci: CodingCi { repository: ci_repository, workflow: CI_WORKFLOW.into(), event: "push".into(),
            command: CodingCommand { argv: vec!["/bin/sh".into(), "-c".into(), CI_LOOK.into()], environment: look, timeout_ms: 120_000 } } })
}

/// Claude's hook settings for the agents the engine starts: they report to the engine's own socket, which publishing
/// checks (`native_agent_hooks`), not to the one the runner's terminals use. Written once, through a temporary file
/// and a rename so no reader sees half of it, and left as it is after, so the digest a published workflow pins stays true.
fn engine_hook_settings(svc: &Service) -> Option<PathBuf> {
    let engine = svc.root_dir().join("workflow").canonicalize().ok()?;
    let text = serde_json::to_string_pretty(&crate::hook_install::claude_settings(&engine.join("h.sock"))).ok()?;
    let path = svc.root_dir().join("workflow-claude-hooks.json");
    if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) { crate::skill_install::write_atomically(&path, &text).ok()?; }
    path.canonicalize().ok()
}

/// Codex's hooks for the agents the engine starts, pinned like Claude's settings and reporting to the same socket. Codex
/// has no flag for them, so the engine copies this file into each worktree it runs Codex in (`install_codex_hooks`), and
/// no repository has to commit one (ov-1088). Written as `engine_hook_settings` is, and left as it is after.
fn engine_codex_hooks(svc: &Service) -> Option<LaunchFile> {
    let engine = svc.root_dir().join("workflow").canonicalize().ok()?;
    let text = crate::hook_install::merge_codex("", &engine.join("h.sock"));
    let path = svc.root_dir().join("workflow-codex-hooks.json");
    if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) { crate::skill_install::write_atomically(&path, &text).ok()?; }
    Some(LaunchFile { digest: farcooler_workflow::digest(&std::fs::read(&path).ok()?), path: path.canonicalize().ok()? })
}

/// The checkout a `.git` directory belongs to, when it is a main checkout's own directory.
fn checkout(git_dir: &Path) -> Option<PathBuf> {
    let git_dir = git_dir.canonicalize().ok()?;
    (git_dir.is_dir() && git_dir.file_name()? == ".git").then(|| git_dir.parent().map(Path::to_path_buf))?
}

/// The branch origin's HEAD names, else `main`.
async fn default_branch(git: &Path, repository: &Path) -> String {
    run(git, repository, &["symbolic-ref", "--quiet", "--short", "refs/remotes/origin/HEAD"]).await
        .and_then(|r| r.strip_prefix("origin/").map(String::from)).filter(|b| !b.is_empty()).unwrap_or_else(|| "main".into())
}

async fn run(git: &Path, repository: &Path, args: &[&str]) -> Option<String> {
    let out = tokio::process::Command::new(git).arg("-C").arg(repository).args(["-c", "core.hooksPath=/dev/null"]).args(args)
        .stdin(std::process::Stdio::null()).kill_on_drop(true).output().await.ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|s| !s.is_empty())
}

/// Said when a remote URL has a sign-in written into it; never the URL itself, which holds the secret.
const TOKEN_IN_URL: &str = "Its origin has a token written into its URL, and Far Cooler doesn’t use that. Switch origin to an ssh URL, or sign in through a git credential helper.";

/// Whether an https remote URL carries a user or token before its host.
fn carries_credentials(url: &str) -> bool {
    url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")).is_some_and(|r| r.split('/').next().is_some_and(|authority| authority.contains('@')))
}

/// The URL a landing pushes to for a GitHub origin: https for an https origin, else ssh, as `git@github.com:o/n` is.
pub(crate) fn landing_url(origin: &str, repository: &str) -> String {
    if origin.starts_with("https://") { format!("https://github.com/{repository}.git") } else { format!("ssh://git@github.com/{repository}.git") }
}

/// `owner/name` for an origin on github.com, in any of the forms git writes it.
pub fn github_repository(origin: &str) -> Option<String> {
    let rest = origin.strip_prefix("git@github.com:").or_else(|| origin.strip_prefix("https://github.com/"))
        .or_else(|| origin.strip_prefix("ssh://git@github.com/"))?;
    let rest = rest.trim_end_matches('/');
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let (owner, name) = rest.split_once('/')?;
    let part = |p: &str| !p.is_empty() && p.len() <= 100 && p.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b));
    (part(owner) && part(name)).then(|| format!("{owner}/{name}"))
}

/// The models each agent step of `definition` may run with: builds Sonnet first, agent reviews Opus first.
pub fn default_models(definition: &mut farcooler_workflow::Definition) {
    use farcooler_workflow::{Reviewer, StageKind};
    for stage in &definition.stages {
        let list = match stage.kind {
            StageKind::Build => BUILD_MODELS,
            StageKind::Review(Reviewer::Agent) => REVIEW_MODELS,
            _ => continue,
        };
        definition.models.insert(stage.id.clone(), list.iter().map(|m| m.to_string()).collect());
    }
}

#[cfg(test)]
mod tests {
    use super::github_repository as gh;
    use crate::landing_templates::{self as lt, Extra, Options};
    use crate::workflow_runtime::{CodingCi, CodingCommand, CodingConfig, CodingHarness, LaunchFile};
    use farcooler_workflow::{Reviewer, StageKind};

    fn config(preset: &str) -> CodingConfig {
        let cmd = || CodingCommand { argv: vec!["/usr/bin/true".into()], environment: Default::default(), timeout_ms: 1000 };
        CodingConfig { repository: "/r".into(), base: "a".repeat(40), landing_ref: "refs/heads/main".into(), landing: None, pull_request: None,
            harness: CodingHarness::Native { preset: preset.into(), settings: Some(LaunchFile { path: "/s.json".into(), digest: "wf1:s".into() }),
                project_hooks: None, codex_hooks: None, plugin_dir: None, prompt: super::PROMPT.into(), process: CodingCommand { argv: vec![], ..cmd() } },
            checks: vec![], reviewer: None, ci: CodingCi { repository: "o/r".into(), workflow: "CI".into(), event: "push".into(), command: cmd() } }
    }
    #[test]
    fn a_claude_draft_builds_with_sonnet_and_reviews_with_opus_and_lists_the_other_for_the_orchestrator() {
        let b = lt::template("batched-push", &config("claude"), &["make check".into()], &[Extra::Agent], &Options { batch: Some(4), ..Default::default() }).unwrap();
        b.definition.validate().unwrap();
        let build = b.definition.stages.iter().find(|s| s.kind == StageKind::Build).unwrap();
        let review = b.definition.stages.iter().find(|s| s.kind == StageKind::Review(Reviewer::Agent)).unwrap();
        assert_eq!(b.definition.models[&build.id], ["claude:sonnet", "claude:opus"]);
        assert_eq!(b.definition.models[&review.id], ["claude:opus", "claude:sonnet"]);
        assert_eq!(b.definition.models.len(), 2, "only agent steps list models: {:?}", b.definition.models);
        // Another harness's runner lists none: its preset is whatever it was set up with.
        let b = lt::template("batched-push", &config("codex"), &["make check".into()], &[Extra::Agent], &Options::default()).unwrap();
        assert!(b.definition.models.is_empty());
    }
    #[tokio::test]
    async fn the_ci_look_records_jobs_and_attempt_and_can_follow_the_exact_rerun() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let gh = dir.path().join("gh");
        std::fs::write(&gh, r#"#!/usr/bin/python3
import json, os, sys
args = sys.argv[1:]
endpoint = next(a for a in args if a.startswith('repos/'))
if '/jobs?' in endpoint:
    assert endpoint == 'repos/o/r/actions/runs/73/attempts/2/jobs?per_page=100', args
    assert '--paginate' in args and '--slurp' in args, args
    print(json.dumps(['iOS UI tests', 'Other failed job']))
elif '/attempts/' in endpoint:
    assert endpoint == 'repos/o/r/actions/runs/73/attempts/2', args
    print(json.dumps(dict(binding=os.environ['FARCOOLER_OPERATION_BINDING'], repository='o/r', run=dict(id=73, run_attempt=2))))
else:
    if os.environ.get('FARCOOLER_CI_RUN'):
        assert endpoint == 'repos/o/r/actions/runs/73', args
    else:
        assert endpoint.startswith('repos/o/r/actions/runs?head_sha='), args
    print('73\t2')
"#).unwrap();
        std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
        for target in ["", "73"] {
            let receipt = dir.path().join("receipt.json");
            let output = tokio::process::Command::new("/bin/sh").args(["-c", super::CI_LOOK]).env_clear()
                .env("CI_GH", &gh).env("CI_REPOSITORY", "o/r").env("CI_WORKFLOW", "CI").env("CI_EVENT", "push")
                .env("FARCOOLER_CANDIDATE", "a".repeat(40)).env("FARCOOLER_OPERATION_BINDING", "bound")
                .env("FARCOOLER_CI_RUN", target).env("FARCOOLER_RECEIPT", &receipt).output().await.unwrap();
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            let result: crate::workflow_runtime::CodingCiReceipt = serde_json::from_slice(&std::fs::read(&receipt).unwrap()).unwrap();
            assert_eq!(result.binding, "bound");
            assert_eq!(result.run["run_attempt"], 2);
            assert_eq!(result.failed_jobs.unwrap(), ["iOS UI tests", "Other failed job"]);
        }
    }
    #[test]
    fn the_ci_look_stays_within_a_configured_commands_bounds() {
        assert!(super::CI_LOOK.len() <= 4096 && super::PROMPT.len() <= 4096);
    }
    #[test]
    fn an_origin_on_github_names_its_repository_in_every_form_git_writes() {
        for origin in ["git@github.com:taneliang/farcooler.git", "https://github.com/taneliang/farcooler", "https://github.com/taneliang/farcooler.git/",
            "ssh://git@github.com/taneliang/farcooler.git"] {
            assert_eq!(gh(origin).as_deref(), Some("taneliang/farcooler"), "{origin}");
        }
        for origin in ["", "/srv/bare.git", "git@gitlab.com:a/b.git", "https://github.com/a", "https://github.com/a/b/c", "https://github.com/a/b c"] {
            assert_eq!(gh(origin), None, "{origin}");
        }
    }
}
