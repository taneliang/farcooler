//! The adapter settings a runner suggests for a workspace that has never published a workflow (ov-1010).
//!
//! A draft pins a definition to adapter settings, so a workspace with none had nothing to draft against and
//! `workspace workflow-draft` refused. The runner knows what a first draft needs: the workspace's checkout, the
//! branch it lands on, its own Claude hook settings and Codex hooks, and the GitHub repository CI runs in. It says them
//! here, in `workflow get`'s reply, and suggests only. The things written are the engine's Claude hook settings file
//! (`engine_hook_settings`) and its Codex hooks file (`engine_codex_hooks`, ov-1088), each named for its content
//! (`app_owned_file`, ov-1259) and written only when missing, even when the suggestion then refuses: they are the runner's own files, the same for every workspace, and
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

pub use farcooler_daemon_contracts::landing_templates::{BUILD_MODELS, HARNESS, REVIEW_MODELS, default_models};

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
jobs=$("$CI_GH" api --paginate --slurp "repos/$CI_REPOSITORY/actions/runs/$run/attempts/$attempt/jobs?per_page=100" --jq '[.[].jobs[] | select(.conclusion == "failure" or .conclusion == "timed_out" or .conclusion == "action_required" or .conclusion == "startup_failure" or .conclusion == "stale") | .name]') || exit 1
failing=$("$CI_GH" api --paginate --slurp "repos/$CI_REPOSITORY/actions/runs/$run/attempts/$attempt/jobs?per_page=100" --jq '[.[].jobs[] | select(.conclusion == "failure")][:8][] | [(.id | tostring), .name, ([.steps[]? | select(.conclusion == "failure")][0].name // "")] | @tsv') || exit 1
CI_FAILED_STEPS=$(printf '%s\n' "$failing" | while IFS='	' read -r id name step; do
  [ -n "$id" ] || continue
  action=$("$CI_GH" run view "$run" --attempt "$attempt" --repo "$CI_REPOSITORY" --job "$id" --log-failed </dev/null 2>/dev/null | sed -n 's/.*##\[end-action id=\([^;]*\);outcome=failure;.*/\1/p' | head -n 1)
  printf '%s\t%s\t%s\n' "$name" "$step" "$action"
done)
export CI_FAILED_STEPS CI_FAILURE_JOB='' CI_FAILURE_LINE=''
failed=$("$CI_GH" api --paginate --slurp "repos/$CI_REPOSITORY/actions/runs/$run/attempts/$attempt/jobs?per_page=100" --jq '[.[].jobs[] | select(.conclusion == "failure")][0] // empty | (.id | tostring) + "|" + .name') || exit 1
if [ -n "$failed" ]; then
  CI_FAILURE_JOB=${failed#*|}
  log=$("$CI_GH" run view "$run" --attempt "$attempt" --repo "$CI_REPOSITORY" --job "${failed%%|*}" --log-failed) || exit 1
  CI_FAILURE_LINE=$(printf '%s\n' "$log" | awk '
    { line=$0; sub(/^[^\t]*\t[^\t]*\t/, "", line) }
    line ~ /[^[:space:]]/ && !first { first=line }
    !error && tolower(line) ~ /(^|[^[:alnum:]_])(error|failed|failure|fatal|panicked)([^[:alnum:]_]|$)/ { error=line }
    END { print error ? error : first }')
fi
out=$("$CI_GH" api "repos/$CI_REPOSITORY/actions/runs/$run/attempts/$attempt" --jq '{binding: env.FARCOOLER_OPERATION_BINDING, repository: env.CI_REPOSITORY, run: {id, name, head_sha, status, conclusion, html_url, created_at, display_title, event, updated_at, run_attempt}, failure: (if env.CI_FAILURE_JOB == "" then null else {job: env.CI_FAILURE_JOB, firstLine: env.CI_FAILURE_LINE} end), failed_steps: [env.CI_FAILED_STEPS | split("\n")[] | split("\t") | select(length == 3 and .[0] != "" and .[1] != "") | {job: .[0], step: .[1], action: (if .[2] == "" then null else .[2] end)}]}') || exit 1
printf '%s,"failed_jobs":%s}\n' "${out%?}" "$jobs" > "$FARCOOLER_RECEIPT""#;

/// `workflow get`'s reply, for a workspace that has never published: the settings this runner suggests for its first
/// draft as `suggested_adapter`, or why it can't as `suggested_adapter_refused`. Only a `get` reply carries a
/// `configuration`, and a published workspace's isn't null, so every other reply is left alone.
pub(crate) async fn add_to_get(svc: &Service, workspace: Uuid, mut value: serde_json::Value) -> serde_json::Value {
    // Every reply, a published workspace's too: Codex's hooks are what a published adapter pins to add Codex models (ov-1088).
    if value.get("configuration").is_some() {
        if let (Some(reply), Some(hooks)) = (value.as_object_mut(), engine_codex_hooks(svc.root_dir(), true)) {
            reply.insert("engine_codex_hooks".into(), serde_json::json!(hooks));
        }
    }
    // The published adapter pinned to its files as they are now, when that differs (ov-1259): what `workspace re-pin`
    // and `landing-template` draft on, so neither copies a digest that stopped being true.
    if let Some(Ok(farcooler_protocol::workflow_config::AdapterConfiguration::Coding(config))) = value.pointer("/configuration/adapter")
        .filter(|a| !a.is_null()).map(|a| serde_json::from_value(a.clone()))
    {
        let next = repin(svc, &config, Repin::Everything);
        let definition = value.pointer("/configuration/definition").and_then(|d| serde_json::from_value(d.clone()).ok())
            .and_then(|d| repin_definition(&d, &config, &next));
        if next != config && let Some(definition) = definition && let Some(reply) = value.as_object_mut() {
            reply.insert("repinned_adapter".into(), serde_json::json!(farcooler_protocol::workflow_config::AdapterConfiguration::Coding(next)));
            reply.insert("repinned_definition".into(), serde_json::json!(definition));
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
    let settings = engine_hook_settings(svc.root_dir(), true).ok_or("Far Cooler couldn’t write the engine’s Claude hook settings on this runner.")?;
    let codex_hooks = engine_codex_hooks(svc.root_dir(), true).ok_or("Far Cooler couldn’t write the engine’s Codex hooks on this runner.")?;
    let environment = crate::landing_templates::gate_environment(&[]).map_err(|e| e.to_string())?;
    let mut agent = environment.clone();
    if let Some(home) = environment.get("HOME") { agent.insert("PATH".into(), format!("{GATE_PATH}:{home}/.local/bin:{home}/.cargo/bin")); }
    let mut look = environment;
    look.extend([("CI_GH".to_string(), gh.to_string_lossy().into_owned()), ("CI_REPOSITORY".into(), ci_repository.clone()),
        ("CI_WORKFLOW".into(), CI_WORKFLOW.into()), ("CI_EVENT".into(), "push".into())]);
    Ok(CodingConfig { security_reviews: vec![], repository, base, landing_ref, landing: Some(landing), pull_request: None,
        harness: CodingHarness::Native { preset: HARNESS.into(), settings: Some(settings), project_hooks: None, codex_hooks: Some(codex_hooks), plugin_dir: None, prompt: PROMPT.into(),
            process: CodingCommand { argv: Vec::new(), environment: agent, timeout_ms: MAX_COMMAND_MS } },
        checks: Vec::new(), reviewer: None,
        ci: CodingCi { repository: ci_repository, workflow: CI_WORKFLOW.into(), event: "push".into(),
            command: CodingCommand { argv: vec!["/bin/sh".into(), "-c".into(), CI_LOOK.into()], environment: look, timeout_ms: 120_000 }, unrun: None } })
}

/// Claude's hook settings for the agents the engine starts: they report to the engine's own socket, which publishing
/// checks (`native_agent_hooks`), not to the one the runner's terminals use. An app-owned file (`app_owned_file`),
/// written only when `write`.
fn engine_hook_settings(root: &Path, write: bool) -> Option<LaunchFile> {
    let engine = root.join("workflow").canonicalize().ok()?;
    let text = serde_json::to_string_pretty(&crate::hook_install::claude_settings(&engine.join("h.sock"))).ok()?;
    app_owned_file(root, CLAUDE_HOOKS_STEM, &text, write)
}

/// Codex's hooks for the agents the engine starts, pinned like Claude's settings and reporting to the same socket. Codex
/// has no flag for them, so the engine copies this file into each worktree it runs Codex in (`install_codex_hooks`), and
/// no repository has to commit one (ov-1088). An app-owned file (`app_owned_file`), written only when `write`.
fn engine_codex_hooks(root: &Path, write: bool) -> Option<LaunchFile> {
    let engine = root.join("workflow").canonicalize().ok()?;
    let text = serde_json::to_string_pretty(&crate::hook_install::native_codex_hooks(&engine.join("h.sock"))).ok()?;
    app_owned_file(root, CODEX_HOOKS_STEM, &text, write)
}

/// The names the engine's own hook files start with, under the runtime root.
const CLAUDE_HOOKS_STEM: &str = "workflow-claude-hooks";
const CODEX_HOOKS_STEM: &str = "workflow-codex-hooks";

/// A file this build writes and a published workflow pins by digest. It is named for its content
/// (`<stem>-<digest>.json`), so a new build that writes different hooks writes a new file beside the old one and never
/// changes a file a published workflow pins: an app update can't make the engine refuse its running items (ov-1259).
/// Adopting the new file is a re-pin, which publishes like any other change (`repin`, `file_repin_drafts`). Written
/// through a temporary file and a rename so no reader sees half of it. The unnamed `<stem>.json` an older build wrote
/// is never written again, so a workflow that pins it keeps running too. Without `write` the file is only named, never
/// written: a permission check asks what this build would pin and must change nothing on disk (ov-1267).
fn app_owned_file(root: &Path, stem: &str, text: &str, write: bool) -> Option<LaunchFile> {
    let digest = farcooler_workflow::digest(&text.as_bytes().to_vec());
    let name: String = digest.trim_start_matches("wf1:").chars().take(16).collect();
    let path = root.canonicalize().ok()?.join(format!("{stem}-{name}.json"));
    if write && std::fs::read_to_string(&path).ok().as_deref() != Some(text) { crate::skill_install::write_atomically(&path, text).ok()?; }
    Some(LaunchFile { digest, path })
}

/// Which of the engine's own hook files `path` is, when it is one: a file under the runtime root this build or an
/// older one wrote (`app_owned_file`), with or without its content in its name.
fn app_owned_stem(root: &Path, path: &Path) -> Option<&'static str> {
    let root = root.canonicalize().ok()?;
    if path.parent() != Some(root.as_path()) { return None; }
    let name = path.file_name()?.to_str()?.strip_suffix(".json")?;
    [CLAUDE_HOOKS_STEM, CODEX_HOOKS_STEM].into_iter().find(|stem| name.strip_prefix(stem)
        .is_some_and(|rest| rest.is_empty() || rest.strip_prefix('-').is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_hexdigit()))))
}

/// What re-pinning changes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Repin {
    /// Only the engine's own hook files, moved to the ones this build writes: what the engine re-pins by itself.
    AppOwned,
    /// Those, and every other pinned file's digest computed from the file as it is now: what a person asks for with
    /// `workspace re-pin`, and reads on the draft before it publishes.
    Everything,
}

/// `config` pinned to its files as they are now (ov-1259), so a draft never carries a digest that was true once.
/// The engine's own hook files move to the ones this build writes; with `Repin::Everything`, every other pinned file
/// (Claude settings, a repository's hooks, a local landing's config) is digested again from where it is. A file that
/// can't be read keeps its old pin, which publishing then refuses with the file's name. Nothing is published here;
/// the engine's own hook files this build writes are written, so the draft or publish that follows finds them.
pub fn repin(svc: &Service, config: &CodingConfig, scope: Repin) -> CodingConfig { repin_with(svc.root_dir(), config, scope, true) }

/// `repin`, writing the engine's own hook files only when `write`.
fn repin_with(root: &Path, config: &CodingConfig, scope: Repin, write: bool) -> CodingConfig {
    let mut next = config.clone();
    let fresh = |file: &LaunchFile| -> LaunchFile {
        match app_owned_stem(root, &file.path) {
            Some(CLAUDE_HOOKS_STEM) => engine_hook_settings(root, write),
            Some(_) => engine_codex_hooks(root, write),
            None if scope == Repin::Everything => std::fs::read(&file.path).ok()
                .map(|bytes| LaunchFile { path: file.path.clone(), digest: farcooler_workflow::digest(&bytes) }),
            None => None,
        }.unwrap_or_else(|| file.clone())
    };
    if let CodingHarness::Native { settings, project_hooks, codex_hooks, .. } = &mut next.harness {
        for file in [settings, project_hooks, codex_hooks].into_iter().flatten() { *file = fresh(file); }
    }
    if scope == Repin::Everything
        && let Some(landing) = next.landing.as_mut().filter(|l| landing_is_local(l))
        && let Ok(bytes) = std::fs::read(landing.repository.join("config"))
    {
        landing.config_digest = farcooler_workflow::digest(&bytes);
    }
    next
}
/// `published` with the engine's own hook files moved to the ones this build writes (`Repin::AppOwned`) and its
/// definition re-pinned to match, or `None` when nothing moves. What an orchestrator may publish on its own after an
/// app update (ov-1267): the build fixes those files' content, so the move changes no command the owner didn't run.
/// Read only: it runs inside the publish permission check, before anything is accepted, so it names the files this
/// build writes without writing them. Publishing then reads them as pinned (`native_agent_hooks`), and refuses a
/// missing one, so the orchestrator publishes what `workspace re-pin` or the engine's own re-pin draft wrote.
pub fn engine_repinned(root: &Path, published: &farcooler_store::workflow_publication::Configuration)
    -> Option<farcooler_store::workflow_publication::Configuration>
{
    let Some(farcooler_protocol::workflow_config::AdapterConfiguration::Coding(config)) = &published.adapter else { return None };
    let next = repin_with(root, config, Repin::AppOwned, false);
    if &next == config { return None; }
    let definition = repin_definition(&published.definition, config, &next)?;
    Some(farcooler_store::workflow_publication::Configuration { pin: published.pin.clone(), definition,
        adapter: Some(farcooler_protocol::workflow_config::AdapterConfiguration::Coding(next)), notifications: published.notifications.clone() })
}
/// A landing that is a bare repository on this runner, whose config file is pinned (not a network remote).
fn landing_is_local(landing: &CodingLanding) -> bool {
    let to = landing.repository.to_string_lossy();
    !(to.starts_with("ssh://") || to.starts_with("https://"))
}

/// Why the engine filed a re-pin draft, in its own words on the card.
pub const REPIN_REASON: &str = "This Far Cooler update writes new hooks for the agents the engine starts. The published \
    workflow still pins the previous ones. Approve this to move new runs and the next unstarted steps of existing runs to the new hooks.";

/// At start, for every workspace whose published workflow pins one of the engine's own hook files that this build
/// writes differently, file the re-pin draft itself (ov-1259): the same definition, the hooks moved to this build's.
/// A person publishes it as any draft. Nothing is filed while the workspace already has a draft pending, or when a
/// draft of exactly this bundle was filed before, so a declined one isn't asked again at every start. Returns the
/// drafts filed.
pub fn file_repin_drafts(svc: &Service, watcher: &crate::watch::Watcher) -> Vec<crate::workflow_drafts::Draft> {
    let mut filed = Vec::new();
    for workspace in svc.store.list_workspaces(None).unwrap_or_default() {
        let Ok(Some(current)) = svc.store.workflow_configuration(&workspace.id.to_string()) else { continue };
        let Some(farcooler_protocol::workflow_config::AdapterConfiguration::Coding(config)) = &current.adapter else { continue };
        let next = repin(svc, config, Repin::AppOwned);
        // Moving to a file with the same content is no change: compare what's pinned, not where.
        if pinned_digests(&next) == pinned_digests(config) { continue; }
        let Some(definition) = repin_definition(&current.definition, config, &next) else { continue };
        let adapter = farcooler_protocol::workflow_config::AdapterConfiguration::Coding(next);
        let drafts = crate::workflow_drafts::list(svc, Some(workspace.id));
        if drafts.iter().any(|d| d.state == crate::workflow_drafts::State::Pending
            || (d.adapter == adapter && d.definition == definition)) { continue; }
        match crate::workflow_drafts::propose(svc, watcher, workspace.id, REPIN_REASON, definition, adapter) {
            Ok(draft) => filed.push(draft),
            Err(error) => tracing::warn!(workspace = %workspace.id, %error, "could not file the re-pin draft"),
        }
    }
    filed
}
/// `definition` with every operation that names `before`'s pin naming `after`'s instead: each step's operation carries
/// the adapter's pin as its first argument, in its groups' steps too, so an adapter can't change under a definition
/// that still names the old one. `None` when the definition names no step by `before`'s pin, as when it was read with
/// hidden values: then there is nothing sound to re-pin.
pub fn repin_definition(definition: &farcooler_workflow::Definition, before: &CodingConfig, after: &CodingConfig) -> Option<farcooler_workflow::Definition> {
    let mut next = definition.clone();
    let (before, after) = (before.pin(), after.pin());
    let mut found = false;
    for stage in next.stages.iter_mut().chain(next.groups.values_mut().flat_map(|g| &mut g.steps)) {
        if let Some(pin) = stage.operation.arguments.first_mut().filter(|pin| **pin == before) {
            *pin = after.clone();
            found = true;
        }
    }
    found.then_some(next)
}

fn pinned_digests(config: &CodingConfig) -> [Option<&str>; 3] {
    match &config.harness {
        CodingHarness::Native { settings, project_hooks, codex_hooks, .. } =>
            [settings, project_hooks, codex_hooks].map(|file| file.as_ref().map(|f| f.digest.as_str())),
        CodingHarness::Command(_) => [None; 3],
    }
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

#[cfg(test)]
mod tests {
    use super::github_repository as gh;
    use crate::landing_templates::{self as lt, Extra, Options};
    use crate::workflow_runtime::{CodingCi, CodingCommand, CodingConfig, CodingHarness, LaunchFile};
    use farcooler_workflow::{Reviewer, StageKind};

    fn config(preset: &str) -> CodingConfig {
        let cmd = || CodingCommand { argv: vec!["/usr/bin/true".into()], environment: Default::default(), timeout_ms: 1000 };
        CodingConfig { security_reviews: vec![], repository: "/r".into(), base: "a".repeat(40), landing_ref: "refs/heads/main".into(), landing: None, pull_request: None,
            harness: CodingHarness::Native { preset: preset.into(), settings: Some(LaunchFile { path: "/s.json".into(), digest: "wf1:s".into() }),
                project_hooks: None, codex_hooks: None, plugin_dir: None, prompt: super::PROMPT.into(), process: CodingCommand { argv: vec![], ..cmd() } },
            checks: vec![], reviewer: None, ci: CodingCi { repository: "o/r".into(), workflow: "CI".into(), event: "push".into(), command: cmd(), unrun: None } }
    }
    /// ov-1259: a build that writes the engine's hooks differently writes a new file beside the old one, so a file a
    /// published workflow pins keeps the bytes it was pinned with, and re-pinning moves only to this build's file.
    #[tokio::test]
    async fn the_engines_own_files_are_named_for_their_content_and_never_rewritten_under_a_pin() {
        use super::{app_owned_file, app_owned_stem, repin, Repin, CODEX_HOOKS_STEM, CLAUDE_HOOKS_STEM};
        let dir = tempfile::tempdir().unwrap();
        let svc = crate::service::Service::open_in(dir.path().join("state")).await.unwrap();
        std::fs::create_dir_all(svc.root_dir().join("workflow")).unwrap();
        let old = app_owned_file(svc.root_dir(), CODEX_HOOKS_STEM, "{\"hooks\":\"before the update\"}", true).unwrap();
        let new = app_owned_file(svc.root_dir(), CODEX_HOOKS_STEM, "{\"hooks\":\"after the update\"}", true).unwrap();
        assert_ne!(old.path, new.path);
        assert_eq!(farcooler_workflow::digest(&std::fs::read(&old.path).unwrap()), old.digest, "the pinned file is untouched");
        assert_eq!(app_owned_file(svc.root_dir(), CODEX_HOOKS_STEM, "{\"hooks\":\"after the update\"}", true).unwrap(), new, "the same content, the same file");
        // Only the engine's own names under its root are its files; an unnamed one is an older build's.
        let root = svc.root_dir().canonicalize().unwrap();
        assert_eq!(app_owned_stem(svc.root_dir(), &root.join("workflow-claude-hooks.json")), Some(CLAUDE_HOOKS_STEM));
        assert_eq!(app_owned_stem(svc.root_dir(), &old.path), Some(CODEX_HOOKS_STEM));
        assert_eq!(app_owned_stem(svc.root_dir(), &root.join("workflow-codex-hooks-xyz.json")), None);
        assert_eq!(app_owned_stem(svc.root_dir(), &dir.path().join("workflow-codex-hooks.json")), None);

        // A re-pin by the engine moves its own files and nothing else; one a person asks for digests every file again.
        let mine = dir.path().join("settings.json");
        std::fs::write(&mine, "settings as published").unwrap();
        let mut c = config("claude");
        let crate::workflow_runtime::CodingHarness::Native { settings, codex_hooks, .. } = &mut c.harness else { unreachable!() };
        *settings = Some(LaunchFile { path: mine.clone(), digest: farcooler_workflow::digest(&b"settings as published".to_vec()) });
        *codex_hooks = Some(old.clone());
        std::fs::write(&mine, "settings edited since").unwrap();
        let engine = repin(&svc, &c, Repin::AppOwned);
        let crate::workflow_runtime::CodingHarness::Native { settings, codex_hooks, .. } = &engine.harness else { unreachable!() };
        assert_eq!(settings.as_ref().unwrap().digest, farcooler_workflow::digest(&b"settings as published".to_vec()));
        let current = codex_hooks.clone().unwrap();
        assert!(current != old && app_owned_stem(svc.root_dir(), &current.path) == Some(CODEX_HOOKS_STEM), "{current:?}");
        let person = repin(&svc, &c, Repin::Everything);
        let crate::workflow_runtime::CodingHarness::Native { settings, codex_hooks, .. } = &person.harness else { unreachable!() };
        assert_eq!(settings.as_ref().unwrap().digest, farcooler_workflow::digest(&b"settings edited since".to_vec()));
        assert_eq!(codex_hooks.as_ref(), Some(&current));
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
# Run 43745907131's failed jobs, their steps and failed logs, as GitHub reported them (ov-1280).
failed = [j for j in json.load(open(os.environ['FIXTURE']))['jobs'] if j['conclusion'] == 'failure']
if args[0] == 'run':
    job = args[args.index('--job') + 1]
    lines = next((j['log_failed'] for j in failed if str(j['id']) == job), ['job\tstep\terror: test failed'])
    print('\n'.join(lines))
    sys.exit(0)
endpoint = next(a for a in args if a.startswith('repos/'))
if '/jobs?' in endpoint:
    assert endpoint == 'repos/o/r/actions/runs/73/attempts/2/jobs?per_page=100', args
    assert '--paginate' in args and '--slurp' in args, args
    if '@tsv' in args[-1]:
        for j in failed:
            print('\t'.join([str(j['id']), j['name'], next(s['name'] for s in j['steps'] if s['conclusion'] == 'failure')]))
    else:
        print('91|iOS UI tests' if '(.id | tostring)' in args[-1] else json.dumps(['iOS UI tests', 'Other failed job']))
elif '/attempts/' in endpoint:
    assert endpoint == 'repos/o/r/actions/runs/73/attempts/2', args
    steps = [dict(zip(['job', 'step', 'action'], l.split('\t'))) for l in os.environ['CI_FAILED_STEPS'].split('\n') if l]
    print(json.dumps(dict(binding=os.environ['FARCOOLER_OPERATION_BINDING'], repository='o/r', run=dict(id=73, run_attempt=2),
        failed_steps=[dict(s, action=s['action'] or None) for s in steps])))
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
                .env("FARCOOLER_CI_RUN", target).env("FARCOOLER_RECEIPT", &receipt)
                .env("FIXTURE", concat!(env!("CARGO_MANIFEST_DIR"), "/../../test/fixtures/landing/run43745907131-jobs.json"))
                .output().await.unwrap();
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            let result: crate::workflow_runtime::CodingCiReceipt = serde_json::from_slice(&std::fs::read(&receipt).unwrap()).unwrap();
            assert_eq!(result.binding, "bound");
            assert_eq!(result.run["run_attempt"], 2);
            assert_eq!(result.failed_jobs.unwrap(), ["iOS UI tests", "Other failed job"]);
            // Each failed job's failed step, and the innermost action its log ends in failure.
            let step = |job: &str, step: &str, action: &str| farcooler_workflow::FailedStep { job: job.into(), step: step.into(), action: Some(action.into()) };
            assert_eq!(result.failed_steps.unwrap(), [
                step("iOS", "Run ./.github/actions/ios-ui-build", "__self.__dtolnay_rust-toolchain.__run_4"),
                step("iOS UI (smoke)", "Run ./.github/actions/ios-ui-shard", "__self.__actions_download-artifact")]);
        }
    }
    #[test]
    fn the_ci_provider_keeps_the_failed_job_and_first_failed_log_line() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let gh = dir.path().join("gh");
        std::fs::write(&gh, r#"#!/bin/sh
case "$*" in
  *'/actions/runs?'*) printf '19\t1\n' ;;
  *'/19/attempts/1/jobs?'*) case "$*" in
    *'(.id | tostring)'*) printf '91|Rust tests\n' ;;
    *) printf '["Rust tests"]\n' ;;
  esac ;;
  *'--job 91 --log-failed'*)
    [ "${CI_TEST_LOG_UNAVAILABLE:-}" != 1 ] || exit 1
    printf 'Rust tests\tFailure recovery tests\t2026-11-12T01:00:00Z running tests\nRust tests\tFailure recovery tests\t2026-11-12T01:00:01Z error: first failed assertion\nRust tests\tFailure recovery tests\t2026-11-12T01:00:02Z error: later assertion\n' ;;
  *'--log-failed'*) printf 'Other job Check 2026-11-12T01:00:00Z error: unrelated failure\n' ;;
  *'/actions/runs/19/attempts/1'*) printf '{"binding":"bound","repository":"owner/repo","run":{"id":19},"failure":{"job":"%s","firstLine":"%s"}}' "$CI_FAILURE_JOB" "$CI_FAILURE_LINE" ;;
  *) exit 1 ;;
esac
"#).unwrap();
        std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o700)).unwrap();
        let receipt = dir.path().join("receipt.json");
        let mut look = std::process::Command::new("/bin/sh");
        look.arg("-c").arg(super::CI_LOOK)
            .env("CI_GH", gh).env("CI_REPOSITORY", "owner/repo").env("CI_WORKFLOW", "CI")
            .env("CI_EVENT", "push").env("FARCOOLER_CANDIDATE", "candidate")
            .env("FARCOOLER_OPERATION_BINDING", "bound").env("FARCOOLER_RECEIPT", &receipt);
        assert!(look.status().unwrap().success());
        let kept: crate::workflow_runtime::CodingCiReceipt = serde_json::from_slice(&std::fs::read(&receipt).unwrap()).unwrap();
        let failure = kept.failure.unwrap();
        assert_eq!(failure.job, "Rust tests");
        assert_eq!(failure.first_line, "2026-11-12T01:00:01Z error: first failed assertion");
        std::fs::remove_file(&receipt).unwrap();
        assert!(!look.env("CI_TEST_LOG_UNAVAILABLE", "1").status().unwrap().success());
        assert!(!receipt.exists(), "unavailable logs must be looked at again, not kept as an empty failure");
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
