//! Finite trusted-local configuration. No registry, Service activation or login writes.
use super::*;
use farcooler_daemon_contracts::coding_pins;
use crate::hook_install::CODEX_HOOKS;

pub use farcooler_protocol::workflow_config::*;
trait CommandValidation { fn validate(&self) -> Result<()>; }
pub(super) trait FileValidation { fn verify(&self) -> Result<()>; }
/// Host-side validation and optional archetype for the shared configuration DTO.
pub trait CodingConfigExt {
    fn validate(&self) -> Result<()>;
    fn operation(&self, version: &str, argument: &str) -> OperationSpec;
    fn definition(&self) -> Result<Definition>;
    fn stage(&self, id: &str, kind: StageKind, version: &str, argument: &str) -> Stage;
    fn validate_definition(&self, definition: &Definition) -> Result<()>;
    /// What every group holds while it works on the landing target: one group at a time, across boards.
    fn landing_resource(&self) -> Option<String>;
    /// A step that waits: how often it looks, and how long before it gives up (`timeout`).
    fn wait_operation(&self, version: &str, argument: &str, every_ms: u64, patience_ms: u64) -> OperationSpec;
}
impl CommandValidation for CodingCommand {
    fn validate(&self) -> Result<()> {
        if self.argv.is_empty() || self.argv.len() > 32 || !Path::new(&self.argv[0]).is_absolute()
            || self.argv.iter().any(|a| a.len() > 4096 || a.contains('\0'))
            || self.timeout_ms == 0 || self.timeout_ms > MAX_COMMAND_MS
            || self.environment.len() > 24 || self.environment.iter().any(|(k,v)|
                k.is_empty() || !k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                || k.starts_with("FARCOOLER_") || v.len() > 4096 || v.contains('\0'))
        { return Err(Error::Refused("invalid configured command")); }
        let executable = std::fs::metadata(&self.argv[0])?;
        if !executable.is_file() || executable.permissions().mode() & 0o111 == 0 {
            return Err(Error::Refused("configured executable is not executable"));
        }
        Ok(())
    }
}
pub(crate) trait PinnedDefinition {
    fn validate_pinned_definition(&self, definition: &Definition) -> Result<()>;
    /// The steps of a group item, which is a definition of its own.
    fn validate_pinned_group(&self, definition: &Definition) -> Result<()>;
}
impl CodingConfigExt for CodingConfig {
    fn validate(&self) -> Result<()> {
        let repository = || -> Result<()> {
            let git = self.repository.join(".git");
            if self.repository.canonicalize()? != self.repository || !git.is_dir() || git.canonicalize()? != git {
                return Err(Error::Refused("must name the runner's canonical checkout with a local .git directory"));
            }
            Ok(())
        };
        repository().map_err(|error| error.at_field("repository"))?;
        if !sha(&self.base) { return Err(Error::Validation("base must be a 40-character commit hash".into())); }
        if !self.landing_ref.starts_with("refs/heads/") || self.landing_ref.contains("..") || !valid_id(&self.landing_ref) {
            return Err(Error::Validation("landing_ref must be a valid branch ref".into()));
        }
        if self.checks.len() > 8 { return Err(Error::Validation("checks may contain at most eight commands".into())); }
        for (field, value) in [("ci.repository", &self.ci.repository), ("ci.workflow", &self.ci.workflow), ("ci.event", &self.ci.event)] {
            if value.is_empty() { return Err(Error::Validation(format!("{field} must be set"))); }
        }
        if let Some(landing) = &self.landing {
            validate_landing_identity(landing).map_err(|error| error.at_field("landing"))?;
        }
        if let Some(seed) = &self.dependency_seed {
            seed.validate().map_err(|error| error.at_field("dependency_seed"))?;
        }
        match &self.harness {
            CodingHarness::Command(command) => command.validate().map_err(|error| error.at_field("harness.Command"))?,
            CodingHarness::Native { preset, settings, project_hooks, plugin_dir, prompt, process, codex_hooks } => {
                if crate::orchestrator::harness_of(preset).is_none() || prompt.len() > 4096
                    || !process.argv.is_empty() || plugin_dir.is_some()
                { return Err(Error::Refused("native launch requires existing settings; plugins need separate allocation")); }
                // Empty argv is intentional: the retained builder, not config text,
                // supplies the native launch command.
                let mut policy = process.clone(); policy.argv = vec!["/bin/sh".into()]; policy.validate().map_err(|error| error.at_field("harness.Native.process"))?;
                if let Some(settings) = settings { settings.verify().map_err(|error| error.at_field("harness.Native.settings"))?; }
                if preset.split(':').next() == Some("claude") && settings.is_none() {
                    return Err(Error::Refused("claude requires pinned existing hook settings"));
                }
                // Codex's or Cursor's, whichever the path says, so a run may pick either harness (ov-966).
                if let Some(hooks) = project_hooks {
                    if !["codex", "cursor"].iter().any(|h| project_hooks_path(h).is_some_and(|r| hooks.path == self.repository.join(r))) {
                        return Err(Error::Refused("foreign project hook path"));
                    }
                    hooks.verify().map_err(|error| error.at_field("harness.Native.project_hooks"))?;
                }
                // The runner's own copy of Codex's hooks (ov-1088), which the engine writes into each worktree. One
                // inside the repository is a committed file by another name, and two Codex hooks files can't both be it.
                if let Some(hooks) = codex_hooks {
                    if hooks.path.starts_with(&self.repository) { return Err(Error::Refused("codex hooks inside the repository")); }
                    if project_hooks.as_ref().is_some_and(|h| h.path == self.repository.join(CODEX_HOOKS)) {
                        return Err(Error::Refused("two codex hooks files"));
                    }
                    hooks.verify().map_err(|error| error.at_field("harness.Native.codex_hooks"))?;
                }
                if !launchable(self, preset) { return Err(Error::Refused("native project hooks must be pinned")); }
            }
        }
        if let Some(pr) = &self.pull_request {
            let name = |part: &str| !part.is_empty() && part.len() <= 100
                && part.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b));
            let sound_repository = pr.repository.split_once('/').is_some_and(|(owner, repo)| name(owner) && name(repo));
            // Pull requests are pushed to the landing transport, and `gh` is one program with no arguments of its own.
            if !sound_repository || !valid_id(&pr.base) || pr.base.starts_with('-') || pr.base.contains("..")
                || self.landing.as_ref().is_none_or(|l| l.target != format!("refs/heads/{}", pr.base)) || pr.gh.argv.len() != 1 || pr.gh.argv[0].contains('\'')
            { return Err(Error::Refused("invalid pull request configuration")); }
            pr.gh.validate().map_err(|error| error.at_field("pull_request.gh"))?;
            if let Some(remote) = &pr.remote {
                let path = std::path::Path::new(remote).is_absolute()
                    && remote.len() <= 500 && !remote.bytes().any(|b| b.is_ascii_control() || b == b' ' || b == b'?' || b == b'#');
                if !(path || sound_remote_url(remote)) {
                    return Err(Error::Refused("invalid pull request remote"));
                }
            }
        }
        for (index, check) in self.checks.iter().enumerate() {
            check.validate().map_err(|error| error.at_field(&format!("checks[{index}]")))?;
        }
        if let Some(reviewer) = &self.reviewer {
            if !valid_id(&reviewer.identity) { return Err(Error::Refused("invalid reviewer identity")); }
            reviewer.command.validate().map_err(|error| error.at_field("reviewer.command"))?;
        }
        self.ci.command.validate().map_err(|error| error.at_field("ci.command"))
    }
    fn operation(&self, version: &str, argument: &str) -> OperationSpec {
        coding_pins::operation(self, version, argument)
    }
    fn definition(&self) -> Result<Definition> {
        self.validate()?;
        let mut stages = vec![self.stage("provision", StageKind::Provision, "coding-worktree/v1", ""),
            self.stage("build", StageKind::Build, "coding-harness/v1", "")];
        for index in 0..self.checks.len() {
            stages.push(self.stage(&format!("check-{index}"), StageKind::Check, "coding-check/v1", &index.to_string()));
        }
        if let Some(reviewer) = &self.reviewer {
            stages.push(self.stage("review", StageKind::Review(Reviewer::Dispatch), "coding-review/v1", &reviewer.identity));
        }
        stages.push(self.stage("land", StageKind::Land, "coding-land/v1", &self.landing_ref));
        stages.push(self.stage("ci", StageKind::Ci, "coding-ci/v1", &self.ci.repository));
        Ok(Definition { schema: 1, source: "configured-coding/v1".into(), stages,
            max_reworks: if self.reviewer.is_some() { 2 } else { 0 },
            rework_target: self.reviewer.as_ref().map(|_| "build".into()), groups: Default::default(), models: Default::default(), fallbacks: Default::default(), send_backs: None, build_folders: None, script_policies: Default::default(), start_policy: None, limits: None, policies: Default::default() })
    }
    fn stage(&self, id: &str, kind: StageKind, version: &str, argument: &str) -> Stage {
        Stage { id: id.into(), kind, operation: self.operation(version, argument),
            progression: Progression::Automatic, max_attempts: 1,
            routes: Default::default(), limit: 0 }
    }
    fn validate_definition(&self, definition: &Definition) -> Result<()> {
        self.validate()?;
        self.validate_pinned_definition(definition)
    }
    fn landing_resource(&self) -> Option<String> {
        coding_pins::landing_resource(self)
    }
    fn wait_operation(&self, version: &str, argument: &str, every_ms: u64, patience_ms: u64) -> OperationSpec {
        coding_pins::wait_operation(self, version, argument, every_ms, patience_ms)
    }
}
/// The identity an agent's review gate pins, and the one its verdict receipt carries (ov-969).
pub const AGENT_REVIEWER: &str = "agent";
/// The longest any configured command may run: the landing job's own allowance for a gate.
pub const MAX_COMMAND_MS: u64 = 3 * 3_600_000;
/// How often a waiting step looks, and the longest it may wait.
pub(super) const WAIT_EVERY_MS: std::ops::RangeInclusive<u64> = 10..=3_600_000;
pub(super) const WAIT_PATIENCE_MS: std::ops::RangeInclusive<u64> = 10..=14 * 24 * 3_600_000;
/// The poll interval and patience a waiting step pinned, if it pinned sound ones.
pub(super) fn wait_times(operation: &OperationSpec) -> Option<(u64, u64)> {
    // A CI wait may pin one more: the other workflows it waits on (ov-1071).
    let (every, patience) = match operation.arguments.as_slice() { [_, _, every, patience] | [_, _, every, patience, _] => (every, patience), _ => return None };
    let (every, patience) = (every.parse::<u64>().ok()?, patience.parse::<u64>().ok()?);
    (WAIT_EVERY_MS.contains(&every) && WAIT_PATIENCE_MS.contains(&patience)).then_some((every, patience))
}
/// The most other workflows one CI wait may name.
pub const MAX_ALSO_WORKFLOWS: usize = 8;
/// The other workflows a CI wait pinned: a JSON list of 1 to 8 distinct names, each as GitHub shows it.
pub(super) fn also_workflows(text: &str) -> Option<Vec<String>> {
    let names: Vec<String> = serde_json::from_str(text).ok()?;
    let distinct = names.iter().collect::<std::collections::BTreeSet<_>>().len() == names.len();
    let sound = !names.is_empty() && names.len() <= MAX_ALSO_WORKFLOWS && distinct
        && names.iter().all(|n| !n.trim().is_empty() && n.len() <= 100 && !n.chars().any(char::is_control));
    sound.then_some(names)
}
impl PinnedDefinition for CodingConfig {
    fn validate_pinned_definition(&self, definition: &Definition) -> Result<()> { self.pinned(definition, false) }
    fn validate_pinned_group(&self, definition: &Definition) -> Result<()> { self.pinned(definition, true) }
}
trait Pinned { fn pinned(&self, definition: &Definition, in_group: bool) -> Result<()>; }
impl Pinned for CodingConfig {
    fn pinned(&self, definition: &Definition, in_group: bool) -> Result<()> {
        if in_group { definition.validate_group().map_err(|_| Error::Refused("invalid group definition"))?; }
        else { definition.validate_detailed().map_err(Error::Validation)?; }
        if !definition.policies.is_empty() {
            self.ci.command.environment.get("CI_GH").filter(|p| std::path::Path::new(p).is_absolute())
                .ok_or(Error::Refused("CI rerun policies need a pinned CI_GH"))?;
        }
        let (mut provisioned, mut built, mut landed) = (false, false, false);
        let (mut based, mut folded, mut pushed) = (false, false, false);
        let (mut branched, mut opened, mut merged) = (false, false, false);
        let mut candidate_branched = false;
        for stage in &definition.stages {
            let problem = |field, rule| Error::Validation(format!("stage {:?} field {field}: {rule}", stage.id));
            let gate_index = |i: Option<&String>| i.and_then(|s| s.parse::<usize>().ok()).filter(|i| *i < self.checks.len())
                .ok_or_else(|| problem("operation.arguments[1]", "check index must name a configured check"));
            let landing = self.landing.as_ref().ok_or_else(|| problem("kind", "landing verbs need an adapter landing identity"));
            let pull_request = self.pull_request.as_ref().ok_or_else(|| problem("kind", "pull request verbs need an adapter pull request identity"));
            let expected = match &stage.kind {
                StageKind::Provision if !provisioned && !built => { provisioned = true; self.operation("coding-worktree/v1", "") },
                StageKind::Build if provisioned && !landed => { built = true; self.operation("coding-harness/v1", "") },
                StageKind::Check if built => self.operation("coding-check/v1", &gate_index(stage.operation.arguments.get(1))?.to_string()),
                // A person decides and nothing runs, so this form needs no reviewer configured. A person
                // review that pins the configured reviewer's identity (the older form) falls to the next arm.
                StageKind::Review(Reviewer::Person(person)) if built && !landed
                    && stage.operation == self.operation("coding-review/v1", &format!("person:{person}")) => stage.operation.clone(),
                // The orchestrator decides, however it chooses to review, and nothing runs: no reviewer is configured (ov-969).
                StageKind::Review(Reviewer::Orchestrator(principal)) if built && !landed
                    && stage.operation == self.operation("coding-review/v1", &format!("orchestrator:{principal}")) => stage.operation.clone(),
                // An agent the engine starts, which only a native harness can launch. No review command runs (ov-969).
                StageKind::Review(Reviewer::Agent) if built && !landed => {
                    if !matches!(self.harness, CodingHarness::Native { .. }) {
                        return Err(problem("kind", "an agent's review gate needs a native harness"));
                    }
                    let mut expected = self.operation("coding-review/v1", AGENT_REVIEWER);
                    // Options of the workflow's own (ov-1071): each model a path rule names is one the step lists, so
                    // the launchable check below covers it.
                    if let Some(text) = stage.operation.arguments.get(2) {
                        let options = crate::review_agent::ReviewOptions::read(text).ok_or_else(|| problem("operation.arguments[2]", "unsound review options"))?;
                        if options.check.is_some_and(|i| i >= self.checks.len()) {
                            return Err(problem("operation.arguments[2]", "foreign parallel review check"));
                        }
                        let listed = definition.models.get(&stage.id);
                        if options.models.iter().any(|(_, model)| !listed.is_some_and(|l| l.contains(model))) {
                            return Err(problem("operation.arguments[2]", "a review's path rule names a model its step doesn't list"));
                        }
                        expected.arguments.push(text.clone());
                    }
                    expected
                },
                StageKind::Review(_) if built && (!landed || definition.max_reworks == 0) =>
                    self.operation("coding-review/v1", &self.reviewer.as_ref().ok_or_else(|| problem("kind", "missing reviewer"))?.identity),
                StageKind::Land if built && !landed => { landed = true; self.operation("coding-land/v1", &self.landing_ref) },
                StageKind::Ci if landed => self.operation("coding-ci/v1", &self.ci.repository),
                // A route reads the ticket's change and answers a label the definition routes. Its rules are the
                // step's own to choose, so any readable, bounded set of them is pinned; the pin is the rest of it.
                StageKind::Route if built && !landed && !in_group => {
                    let rules = stage.operation.arguments.get(1).filter(|a| crate::path_routes::PathRules::read(a).is_some()
                        || crate::review_agent::ReviewRoute::read(a).is_some())
                        .ok_or_else(|| problem("operation.arguments[1]", "unsound path rules"))?;
                    self.operation("coding-route/v1", rules)
                },
                StageKind::Join { group, .. } if built && !in_group => self.operation("coding-join/v1", group),
                StageKind::AwaitBase if in_group && !based && !pushed => {
                    based = true; landing?;
                    let mode = stage.operation.arguments.get(1).filter(|m| matches!(m.as_str(), "any" | "green"))
                        .ok_or_else(|| problem("operation.arguments[1]", "unknown base wait"))?;
                    let (every, patience) = wait_times(&stage.operation).ok_or_else(|| problem("operation.arguments[2..4]", "unsound wait times"))?;
                    self.wait_operation("coding-await-base/v1", mode, every, patience)
                },
                StageKind::Fold if in_group && based && !folded && !pushed => { folded = true; self.operation("coding-fold/v1", "") },
                StageKind::Run if in_group && folded && !pushed => self.operation("coding-gate/v1", &gate_index(stage.operation.arguments.get(1))?.to_string()),
                StageKind::Push if in_group && folded && !pushed => {
                    pushed = true;
                    self.operation("coding-push/v1", &landing?.target)
                },
                // The group's candidate on its own branch, for CI to run on before the push (ov-1269).
                StageKind::PushCandidate if in_group && folded && !pushed && !candidate_branched => {
                    candidate_branched = true; landing?;
                    self.operation("coding-push-candidate/v1", "")
                },
                StageKind::PushBranch if built && !in_group && !branched && !landed => {
                    branched = true; landing?; pull_request?;
                    self.operation("coding-push-branch/v1", "")
                },
                StageKind::OpenPr if branched && !opened => {
                    opened = true;
                    let mode = stage.operation.arguments.get(1).filter(|m| matches!(m.as_str(), "draft" | "ready"))
                        .ok_or_else(|| problem("operation.arguments[1]", "unknown pull request mode"))?;
                    self.operation("coding-open-pr/v1", mode)
                },
                StageKind::PrReady if opened => self.operation("coding-pr-ready/v1", ""),
                StageKind::AwaitPr if opened && !merged => {
                    merged = true;
                    let (every, patience) = wait_times(&stage.operation).ok_or_else(|| problem("operation.arguments[2..4]", "unsound wait times"))?;
                    self.wait_operation("coding-await-pr/v1", "", every, patience)
                },
                StageKind::AwaitCi if pushed || merged || candidate_branched => {
                    let (every, patience) = wait_times(&stage.operation).ok_or_else(|| problem("operation.arguments[2..4]", "unsound wait times"))?;
                    let mut expected = self.wait_operation("coding-await-ci/v1", &self.ci.repository, every, patience);
                    // The other workflows the step waits on besides the configured one (ov-1071).
                    if let Some(text) = stage.operation.arguments.get(4) {
                        if also_workflows(text).is_none_or(|also| also.contains(&self.ci.workflow)) {
                            return Err(problem("operation.arguments[4]", "unsound CI workflows"));
                        }
                        expected.arguments.push(text.clone());
                    }
                    expected
                },
                _ => return Err(problem("kind", "unsupported operation or unsafe prerequisites")),
            };
            if stage.operation.adapter_version != expected.adapter_version {
                return Err(problem("operation.adapter_version", "does not match the stage's configured operation"));
            }
            if stage.operation.arguments != expected.arguments {
                let actual = &stage.operation.arguments;
                let index = actual.iter().zip(&expected.arguments).position(|(a, b)| a != b)
                    .unwrap_or_else(|| actual.len().min(expected.arguments.len()));
                let rule = if index == 0 { "must match the adapter pin" } else { "does not match the configured operation" };
                return Err(Error::Validation(format!("stage {:?} field operation.arguments[{index}]: {rule}", stage.id)));
            }
            if stage.operation.environment != expected.environment {
                return Err(problem("operation.environment", "does not match the configured operation"));
            }
        }
        if in_group { return Ok(()); }
        super::coding_review_policy::validate(self, definition)?;
        // A model the runtime can't launch never reaches a run: a typo, or a harness whose hooks aren't pinned (ov-966).
        if let Some((step, _)) = definition.models.iter().find(|(_, models)| models.iter().any(|model| !launchable(self, model))) {
            return Err(Error::Validation(format!("stage {step:?} field models: a listed harness isn't set up on this runner")));
        }
        // Every group works on the one landing target, so they take turns for it.
        for name in definition.groups.keys() {
            let group = &definition.groups[name];
            if self.landing_resource().as_ref() != Some(&group.exclusive) {
                return Err(Error::Validation(format!("group {name:?} field exclusive must hold the adapter landing target")));
            }
            let child = definition.group_definition(name).expect("validated group");
            self.pinned(&child, true).map_err(|error| error.at_field(&format!("group {name:?}")))?;
        }
        Ok(())
    }
}
impl FileValidation for LaunchFile {
    fn verify(&self) -> Result<()> {
        // A missing or unreadable file is a changed one too, and says which (ov-1259).
        let same = self.path.canonicalize().is_ok_and(|p| p == self.path)
            && bounded_file(&self.path).is_ok_and(|bytes| digest(&bytes) == self.digest);
        if !same { return Err(Error::Changed { path: self.path.clone(), field: None }); }
        Ok(())
    }
}
/// Where Codex and Cursor read a project's hooks, relative to the repository. `None` for Claude, whose hooks
/// are its settings file.
pub(super) fn project_hooks_path(harness: &str) -> Option<&'static str> {
    match harness { "codex" => Some(CODEX_HOOKS), "cursor" => Some(crate::hook_install::CURSOR_HOOKS), _ => None }
}
/// Whether this configuration can launch `preset` (`harness` or `harness:model`): a harness Far Cooler runs
/// and, for a native one, with that harness's hooks pinned, since its turn is read from them.
pub(super) fn launchable(config: &CodingConfig, preset: &str) -> bool {
    if crate::orchestrator::harness_of(preset).is_none() { return false; }
    let CodingHarness::Native { settings, .. } = &config.harness else { return true };
    match preset.split(':').next().unwrap_or("") {
        "claude" => settings.is_some(),
        agent => worktree_hooks(config, agent).is_some(),
    }
}
/// The file a Codex or Cursor run's worktree must hold at `project_hooks_path`, as pinned: the runner's own copy of
/// Codex's hooks (`codex_hooks`, which the engine writes there, ov-1088), else the repository's committed one.
pub(super) fn worktree_hooks<'a>(config: &'a CodingConfig, agent: &str) -> Option<&'a LaunchFile> {
    let CodingHarness::Native { project_hooks, codex_hooks, .. } = &config.harness else { return None };
    let relative = project_hooks_path(agent)?;
    codex_hooks.as_ref().filter(|_| agent == "codex")
        .or_else(|| project_hooks.as_ref().filter(|h| h.path == config.repository.join(relative)))
}
/// The URL of a landing that is a network remote (ov-1026), or `None` for a local bare repository.
pub(crate) fn landing_url(landing: &CodingLanding) -> Option<&str> {
    landing.repository.to_str().filter(|r| r.starts_with("ssh://") || r.starts_with("https://"))
}
/// An `ssh://` or `https://` URL the engine may hand git. A token in the URL would be written to receipts and
/// process listings, so a URL that carries one is refused: credentials come from the runner's own setup. A host
/// that starts with `-` would read as an option to ssh.
pub(super) fn sound_remote_url(remote: &str) -> bool {
    let Some(rest) = remote.strip_prefix("https://").or_else(|| remote.strip_prefix("ssh://")) else { return false };
    let authority = rest.split('/').next().unwrap_or("");
    let (user, host) = authority.rsplit_once('@').map_or(("", authority), |(u, h)| (u, h));
    // Git decodes a percent-encoded user before it connects, so `%3A` would smuggle a password past the check.
    let credentials = authority.contains('%') || if remote.starts_with("https://") { authority.contains('@') } else { user.contains(':') };
    !credentials && !host.is_empty() && !host.starts_with('-') && !user.starts_with('-')
        && rest.len() > authority.len() + 1 && remote.len() <= 500
        && !remote.bytes().any(|b| b.is_ascii_control() || b == b' ' || b == b'?' || b == b'#' || b == b'\\')
}
pub(super) fn sha(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

impl Runtime {
    /// Trusted local opt-in only. The selected repository must be a private scratch
    /// repository for this allocation; no installed Service calls this constructor.
    pub async fn create_configured(root: &Path, config: CodingConfig) -> Result<Self> {
        config.validate()?;
        if adapters::git(&config.repository, &["rev-parse", &config.landing_ref]).await? != config.base {
            return Err(Error::Refused("configured landing ref differs from selected base"));
        }
        // A remote's head is read by the first step that builds on it, not here: this makes no network call.
        if let Some(landing) = config.landing.as_ref().filter(|l| landing_url(l).is_none()) {
            if adapters::git(&landing.repository, &["rev-parse", &landing.target]).await? != config.base {
                return Err(Error::Refused("remote landing ref differs from selected base"));
            }
        }
        std::fs::create_dir(root)?;
        std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))?;
        std::fs::create_dir(root.join("receipts"))?;
        durable(&root.join("coding.json"), &serde_json::to_vec(&config)?)?;
        Self::open(root)
    }
    pub fn configured_definition(&self) -> Result<Definition> {
        self.coding.as_ref().ok_or(Error::Refused("not a configured runtime"))?.definition()
    }
}
impl Exec {
    pub(super) fn validate_definition(&self, definition: &Definition) -> Result<()> {
        match &self.coding {
            Some(config) => config.validate_definition(definition),
            None => adapters::validate(definition),
        }
    }
    pub(super) fn repository(&self) -> PathBuf {
        self.coding.as_ref().map_or_else(|| self.root.join("repo"), |c| c.repository.clone())
    }
    pub(super) fn landing_ref(&self) -> &str {
        self.coding.as_ref().map_or("refs/heads/landed", |c| c.landing_ref.as_str())
    }
}

pub(super) fn validate_landing_identity(landing: &CodingLanding) -> Result<()> {
    if !valid_id(&landing.job) || !landing.target.starts_with("refs/heads/") || !valid_id(&landing.target)
        || landing.target.contains("..") {
        return Err(Error::Refused("invalid or changed local landing identity"));
    }
    match landing_url(landing) {
        // A remote has no config file here to pin; what it is, is its URL (ov-1026).
        Some(url) => if !sound_remote_url(url) || !landing.config_digest.is_empty() {
            return Err(Error::Refused("invalid landing remote"));
        },
        None => if landing.repository.canonicalize()? != landing.repository
            || !landing.repository.join("HEAD").is_file() || landing.repository.join(".git").exists()
            || digest(&bounded_file(&landing.repository.join("config"))?) != landing.config_digest {
            return Err(Error::Refused("invalid or changed local landing identity"));
        },
    }
    Ok(())
}

/// Configuration inputs must never block on a FIFO or allocate unbounded bytes.
fn bounded_file(path: &Path) -> Result<Vec<u8>> {
    use std::io::Read;
    let file = OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW).open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.nlink() != 1 || metadata.len() > 60 * 1024 {
        return Err(Error::Refused("nonregular, aliased or oversized configuration input"));
    }
    let mut bytes = Vec::new(); file.take(60 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 60 * 1024 { return Err(Error::Refused("oversized configuration input")); }
    Ok(bytes)
}
