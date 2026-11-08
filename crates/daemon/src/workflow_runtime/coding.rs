//! Finite trusted-local configuration. No registry, Service activation or login writes.
use super::*;
use std::collections::BTreeMap;

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
pub(super) trait PinnedDefinition {
    fn validate_pinned_definition(&self, definition: &Definition) -> Result<()>;
    /// The steps of a group item, which is a definition of its own.
    fn validate_pinned_group(&self, definition: &Definition) -> Result<()>;
}
impl CodingConfigExt for CodingConfig {
    fn validate(&self) -> Result<()> {
        if self.repository.canonicalize()? != self.repository || (!self.repository.join(".git").is_dir() || self.repository.join(".git").canonicalize()? != self.repository.join(".git"))
            || !sha(&self.base) || !self.landing_ref.starts_with("refs/heads/")
            || self.landing_ref.contains("..") || !valid_id(&self.landing_ref)
            || self.checks.len() > 8
            || self.ci.repository.is_empty() || self.ci.workflow.is_empty() || self.ci.event.is_empty()
        { return Err(Error::Refused("invalid coding configuration")); }
        if let Some(landing) = &self.landing {
            if !valid_id(&landing.job) || landing.repository.canonicalize()? != landing.repository
                || !landing.repository.join("HEAD").is_file() || landing.repository.join(".git").exists()
                || !landing.target.starts_with("refs/heads/") || !valid_id(&landing.target)
                || landing.target.contains("..")
                || digest(&bounded_file(&landing.repository.join("config"))?) != landing.config_digest {
                return Err(Error::Refused("invalid or changed local landing identity"));
            }
        }
        match &self.harness {
            CodingHarness::Command(command) => command.validate()?,
            CodingHarness::Native { preset, settings, project_hooks, plugin_dir, prompt, process } => {
                if crate::orchestrator::harness_of(preset).is_none() || prompt.len() > 4096
                    || !process.argv.is_empty() || plugin_dir.is_some()
                { return Err(Error::Refused("native launch requires existing settings; plugins need separate allocation")); }
                // Empty argv is intentional: the retained builder, not config text,
                // supplies the native launch command.
                let mut policy = process.clone(); policy.argv = vec!["/bin/sh".into()]; policy.validate()?;
                if let Some(settings) = settings { settings.verify()?; }
                let harness = preset.split(':').next().unwrap_or("");
                if harness == "claude" && settings.is_none() {
                    return Err(Error::Refused("claude requires pinned existing hook settings"));
                }
                let relative = match harness { "codex" => Some(".codex/hooks.json"), "cursor" => Some(".cursor/hooks.json"), _ => None };
                if let Some(relative) = relative {
                    let hooks = project_hooks.as_ref().ok_or(Error::Refused("native project hooks must be pinned"))?;
                    if hooks.path != self.repository.join(relative) { return Err(Error::Refused("foreign project hook path")); }
                    hooks.verify()?;
                }
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
            pr.gh.validate()?;
            if let Some(remote) = &pr.remote {
                let url = remote.starts_with("https://") || remote.starts_with("ssh://");
                let path = std::path::Path::new(remote).is_absolute();
                // A token in the URL would be written to receipts and process listings: credentials come from `gh`.
                let authority = remote.split_once("//").map_or("", |(_, rest)| rest.split('/').next().unwrap_or(""));
                let credentials = if remote.starts_with("https://") { authority.contains('@') }
                    else if url { authority.split_once('@').is_some_and(|(user, _)| user.contains(':')) } else { false };
                if !(url || path) || credentials || remote.len() > 500 || remote.bytes().any(|b| b.is_ascii_control() || b == b' ' || b == b'?' || b == b'#') {
                    return Err(Error::Refused("invalid pull request remote"));
                }
            }
        }
        for check in &self.checks { check.validate()?; }
        if let Some(reviewer) = &self.reviewer {
            if !valid_id(&reviewer.identity) { return Err(Error::Refused("invalid reviewer identity")); }
            reviewer.command.validate()?;
        }
        self.ci.command.validate()
    }
    fn operation(&self, version: &str, argument: &str) -> OperationSpec {
        OperationSpec { adapter_version: version.into(), arguments: vec![self.pin(), argument.into()], environment: BTreeMap::new() }
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
            rework_target: self.reviewer.as_ref().map(|_| "build".into()), groups: Default::default(), models: Default::default() })
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
        self.landing.as_ref().map(|l| digest(&("landing-target/v1", &l.repository, &l.target)))
    }
    fn wait_operation(&self, version: &str, argument: &str, every_ms: u64, patience_ms: u64) -> OperationSpec {
        OperationSpec { adapter_version: version.into(), environment: BTreeMap::new(),
            arguments: vec![self.pin(), argument.into(), every_ms.to_string(), patience_ms.to_string()] }
    }
}
/// The longest any configured command may run: the landing job's own allowance for a gate.
pub const MAX_COMMAND_MS: u64 = 3 * 3_600_000;
/// How often a waiting step looks, and the longest it may wait.
pub(super) const WAIT_EVERY_MS: std::ops::RangeInclusive<u64> = 10..=3_600_000;
pub(super) const WAIT_PATIENCE_MS: std::ops::RangeInclusive<u64> = 10..=14 * 24 * 3_600_000;
/// The poll interval and patience a waiting step pinned, if it pinned sound ones.
pub(super) fn wait_times(operation: &OperationSpec) -> Option<(u64, u64)> {
    let [_, _, every, patience] = operation.arguments.as_slice() else { return None };
    let (every, patience) = (every.parse::<u64>().ok()?, patience.parse::<u64>().ok()?);
    (WAIT_EVERY_MS.contains(&every) && WAIT_PATIENCE_MS.contains(&patience)).then_some((every, patience))
}
impl PinnedDefinition for CodingConfig {
    fn validate_pinned_definition(&self, definition: &Definition) -> Result<()> { self.pinned(definition, false) }
    fn validate_pinned_group(&self, definition: &Definition) -> Result<()> { self.pinned(definition, true) }
}
trait Pinned { fn pinned(&self, definition: &Definition, in_group: bool) -> Result<()>; }
impl Pinned for CodingConfig {
    fn pinned(&self, definition: &Definition, in_group: bool) -> Result<()> {
        if in_group { definition.validate_group() } else { definition.validate() }
            .map_err(|_| Error::Refused("invalid definition"))?;
        let (mut provisioned, mut built, mut landed) = (false, false, false);
        let (mut based, mut folded, mut pushed) = (false, false, false);
        let (mut branched, mut opened, mut merged) = (false, false, false);
        for stage in &definition.stages {
            let gate_index = |i: Option<&String>| i.and_then(|s| s.parse::<usize>().ok()).filter(|i| *i < self.checks.len())
                .ok_or(Error::Refused("unknown configured check"));
            let landing = self.landing.as_ref().ok_or(Error::Refused("landing verbs need a landing identity"));
            let pull_request = self.pull_request.as_ref().ok_or(Error::Refused("pull request verbs need a pull request identity"));
            let expected = match &stage.kind {
                StageKind::Provision if !provisioned && !built => { provisioned = true; self.operation("coding-worktree/v1", "") },
                StageKind::Build if provisioned && !landed => { built = true; self.operation("coding-harness/v1", "") },
                StageKind::Check if built => self.operation("coding-check/v1", &gate_index(stage.operation.arguments.get(1))?.to_string()),
                // A person decides and nothing runs, so this form needs no reviewer configured. A person
                // review that pins the configured reviewer's identity (the older form) falls to the next arm.
                StageKind::Review(Reviewer::Person(person)) if built && !landed
                    && stage.operation == self.operation("coding-review/v1", &format!("person:{person}")) => stage.operation.clone(),
                StageKind::Review(_) if built && (!landed || definition.max_reworks == 0) =>
                    self.operation("coding-review/v1", &self.reviewer.as_ref().ok_or(Error::Refused("missing reviewer"))?.identity),
                StageKind::Land if built && !landed => { landed = true; self.operation("coding-land/v1", &self.landing_ref) },
                StageKind::Ci if landed => self.operation("coding-ci/v1", &self.ci.repository),
                // A route reads the ticket's change and answers a label the definition routes. Its rules are the
                // step's own to choose, so any readable, bounded set of them is pinned; the pin is the rest of it.
                StageKind::Route if built && !landed && !in_group => {
                    let rules = stage.operation.arguments.get(1).filter(|a| crate::path_routes::PathRules::read(a).is_some())
                        .ok_or(Error::Refused("unsound path rules"))?;
                    self.operation("coding-route/v1", rules)
                },
                StageKind::Join { group, .. } if built && !in_group => self.operation("coding-join/v1", group),
                StageKind::AwaitBase if in_group && !based && !pushed => {
                    based = true; landing?;
                    let mode = stage.operation.arguments.get(1).filter(|m| matches!(m.as_str(), "any" | "green"))
                        .ok_or(Error::Refused("unknown base wait"))?;
                    let (every, patience) = wait_times(&stage.operation).ok_or(Error::Refused("unsound wait times"))?;
                    self.wait_operation("coding-await-base/v1", mode, every, patience)
                },
                StageKind::Fold if in_group && based && !folded && !pushed => { folded = true; self.operation("coding-fold/v1", "") },
                StageKind::Run if in_group && folded && !pushed => self.operation("coding-gate/v1", &gate_index(stage.operation.arguments.get(1))?.to_string()),
                StageKind::Push if in_group && folded && !pushed => {
                    pushed = true;
                    self.operation("coding-push/v1", &landing?.target)
                },
                StageKind::PushBranch if built && !in_group && !branched && !landed => {
                    branched = true; landing?; pull_request?;
                    self.operation("coding-push-branch/v1", "")
                },
                StageKind::OpenPr if branched && !opened => {
                    opened = true;
                    let mode = stage.operation.arguments.get(1).filter(|m| matches!(m.as_str(), "draft" | "ready"))
                        .ok_or(Error::Refused("unknown pull request mode"))?;
                    self.operation("coding-open-pr/v1", mode)
                },
                StageKind::PrReady if opened => self.operation("coding-pr-ready/v1", ""),
                StageKind::AwaitPr if opened && !merged => {
                    merged = true;
                    let (every, patience) = wait_times(&stage.operation).ok_or(Error::Refused("unsound wait times"))?;
                    self.wait_operation("coding-await-pr/v1", "", every, patience)
                },
                StageKind::AwaitCi if pushed || merged => {
                    let (every, patience) = wait_times(&stage.operation).ok_or(Error::Refused("unsound wait times"))?;
                    self.wait_operation("coding-await-ci/v1", &self.ci.repository, every, patience)
                },
                _ => return Err(Error::Refused("unsupported operation or unsafe prerequisites")),
            };
            if stage.operation != expected {
                return Err(Error::Refused("definition does not pin configured operations"));
            }
        }
        if in_group { return Ok(()); }
        // Every group works on the one landing target, so they take turns for it.
        for name in definition.groups.keys() {
            let group = &definition.groups[name];
            if self.landing_resource().as_ref() != Some(&group.exclusive) {
                return Err(Error::Refused("group does not hold the landing target"));
            }
            self.pinned(&definition.group_definition(name).ok_or(Error::Refused("invalid definition"))?, true)?;
        }
        Ok(())
    }
}
impl FileValidation for LaunchFile {
    fn verify(&self) -> Result<()> {
        if self.path.canonicalize()? != self.path || digest(&bounded_file(&self.path)?) != self.digest {
            return Err(Error::Refused("native settings changed"));
        }
        Ok(())
    }
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
        if let Some(landing) = &config.landing {
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
