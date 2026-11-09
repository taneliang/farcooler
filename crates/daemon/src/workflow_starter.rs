//! The adapter settings a runner suggests for a workspace that has never published a workflow (ov-1010).
//!
//! A draft pins a definition to adapter settings, so a workspace with none had nothing to draft against and
//! `workspace workflow-draft` refused. The runner knows what a first draft needs: the workspace's checkout, the
//! branch it lands on, its own Claude hook settings, and the GitHub repository CI runs in. It says them here,
//! in `workflow get`'s reply, and suggests only: nothing is written. The orchestrator drafts on top of them and
//! the owner reads every setting on the draft's card before anything publishes (`workflow_drafts`).
use crate::landing_templates::GATE_PATH;
use crate::service::Service;
use crate::workflow_runtime::{CodingCi, CodingCommand, CodingConfig, CodingHarness, LaunchFile, MAX_COMMAND_MS};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// The harness a first draft runs its agents with.
pub const HARNESS: &str = "claude";
/// The models a build step may run with, its default first: Sonnet builds.
pub const BUILD_MODELS: [&str; 2] = ["claude:sonnet", "claude:opus"];
/// The models an agent's review gate may run with, its default first: Opus reviews.
pub const REVIEW_MODELS: [&str; 2] = ["claude:opus", "claude:sonnet"];
/// The CI workflow a first draft waits for, by its name on GitHub.
pub const CI_WORKFLOW: &str = "CI";
/// What a build agent is told, after the card it works (`briefed` leads with that).
pub const PROMPT: &str = "Work this ticket to its acceptance items and within its constraints. Run the tests your change \
    can affect. Commit your work on this branch, with a message that names the ticket, then end your turn.";

/// One look at CI on `$FARCOOLER_CANDIDATE`: the newest `$CI_WORKFLOW` run GitHub has for that commit, written
/// as the receipt the CI step reads. No run yet is no answer yet: it exits non-zero and writes nothing.
const CI_LOOK: &str = r#"out=$("$CI_GH" api "repos/$CI_REPOSITORY/actions/runs?head_sha=$FARCOOLER_CANDIDATE&event=$CI_EVENT&per_page=50" --jq '[.workflow_runs[] | select(.name == env.CI_WORKFLOW)] | max_by(.id) // empty | {binding: env.FARCOOLER_OPERATION_BINDING, repository: env.CI_REPOSITORY, run: {id, name, head_sha, status, conclusion, html_url, created_at, display_title, event, updated_at}}') || exit 1
[ -n "$out" ] || exit 1
printf '%s\n' "$out" > "$FARCOOLER_RECEIPT""#;

/// Settings for `workspace` on this runner, or why there are none to suggest, in a sentence a person reads.
pub async fn suggest(svc: &Service, workspace: Uuid) -> Result<CodingConfig, String> {
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
    let ci_repository = github_repository(&origin).ok_or("Its origin isn’t on GitHub, so there’s no CI for a draft to wait on. Write the settings yourself and publish them with `workspace set-workflow`.")?;
    let gh = farcooler_core::programs::find("gh").ok_or("This runner has no gh, which the CI step looks at GitHub with.")?;
    let settings = crate::service::write_claude_hook_settings(svc.root_dir()).ok_or("Far Cooler couldn’t write its Claude hook settings on this runner.")?;
    let settings = LaunchFile { digest: farcooler_workflow::digest(&std::fs::read(&settings).map_err(|_| "Far Cooler couldn’t read its Claude hook settings.")?),
        path: settings.canonicalize().map_err(|_| "Far Cooler couldn’t read its Claude hook settings.")? };
    let environment = crate::landing_templates::gate_environment(&[]).map_err(|e| e.to_string())?;
    let mut agent = environment.clone();
    if let Some(home) = environment.get("HOME") { agent.insert("PATH".into(), format!("{GATE_PATH}:{home}/.local/bin:{home}/.cargo/bin")); }
    let mut look = environment;
    look.extend([("CI_GH".to_string(), gh.to_string_lossy().into_owned()), ("CI_REPOSITORY".into(), ci_repository.clone()),
        ("CI_WORKFLOW".into(), CI_WORKFLOW.into()), ("CI_EVENT".into(), "push".into())]);
    Ok(CodingConfig { repository, base, landing_ref, landing: None, pull_request: None,
        harness: CodingHarness::Native { preset: HARNESS.into(), settings: Some(settings), project_hooks: None, plugin_dir: None, prompt: PROMPT.into(),
            process: CodingCommand { argv: Vec::new(), environment: agent, timeout_ms: MAX_COMMAND_MS } },
        checks: Vec::new(), reviewer: None,
        ci: CodingCi { repository: ci_repository, workflow: CI_WORKFLOW.into(), event: "push".into(),
            command: CodingCommand { argv: vec!["/bin/sh".into(), "-c".into(), CI_LOOK.into()], environment: look, timeout_ms: 120_000 } } })
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
                project_hooks: None, plugin_dir: None, prompt: super::PROMPT.into(), process: CodingCommand { argv: vec![], ..cmd() } },
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
