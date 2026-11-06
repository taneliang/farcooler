//! Finite trusted-local configuration. No registry, Service activation or login writes.
use super::*;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodingCommand {
    pub argv: Vec<String>,
    pub environment: BTreeMap<String, String>,
    pub timeout_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchFile { pub path: PathBuf, pub digest: String }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum CodingHarness {
    Command(CodingCommand),
    /// Existing native launch policy, never prepared/installed by this runtime.
    Native { preset: String, settings: Option<LaunchFile>, project_hooks: Option<LaunchFile>, plugin_dir: Option<PathBuf>,
        prompt: String, process: CodingCommand },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodingReviewer { pub identity: String, pub command: CodingCommand }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodingCi {
    pub repository: String,
    pub workflow: String,
    pub event: String,
    pub command: CodingCommand,
}

/// Explicit local transport identity; no named remote lookup or network URL.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodingLanding {
    pub job: String,
    pub repository: PathBuf,
    pub target: String,
    pub config_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodingConfig {
    pub repository: PathBuf,
    pub base: String,
    pub landing_ref: String,
    #[serde(default)]
    pub landing: Option<CodingLanding>,
    pub harness: CodingHarness,
    pub checks: Vec<CodingCommand>,
    pub reviewer: Option<CodingReviewer>,
    pub ci: CodingCi,
}

impl CodingCommand {
    fn validate(&self) -> Result<()> {
        if self.argv.is_empty() || self.argv.len() > 32 || !Path::new(&self.argv[0]).is_absolute()
            || self.argv.iter().any(|a| a.len() > 4096 || a.contains('\0'))
            || self.timeout_ms == 0 || self.timeout_ms > 3_600_000
            || self.environment.len() > 24 || self.environment.iter().any(|(k,v)|
                k.is_empty() || !k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                || k.starts_with("FARCOOLER_") || v.len() > 4096 || v.contains('\0'))
        { return Err(Error::Refused("invalid configured command")); }
        Ok(())
    }
}
impl CodingConfig {
    pub fn pin(&self) -> String { digest(self) }
    pub(super) fn validate(&self) -> Result<()> {
        if self.repository.canonicalize()? != self.repository || !self.repository.join(".git").is_dir()
            || !sha(&self.base) || !self.landing_ref.starts_with("refs/heads/")
            || self.landing_ref.contains("..") || !valid_id(&self.landing_ref)
            || self.checks.is_empty() || self.checks.len() > 8
            || self.ci.repository.is_empty() || self.ci.workflow.is_empty() || self.ci.event.is_empty()
        { return Err(Error::Refused("invalid coding configuration")); }
        if let Some(landing) = &self.landing {
            if !valid_id(&landing.job) || landing.repository.canonicalize()? != landing.repository
                || !landing.repository.join("HEAD").is_file() || landing.repository.join(".git").exists()
                || !landing.target.starts_with("refs/heads/") || !valid_id(&landing.target)
                || landing.target.contains("..")
                || digest(&std::fs::read(landing.repository.join("config"))?) != landing.config_digest {
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
    pub fn definition(&self) -> Result<Definition> {
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
            rework_target: self.reviewer.as_ref().map(|_| "build".into()) })
    }
    fn stage(&self, id: &str, kind: StageKind, version: &str, argument: &str) -> Stage {
        Stage { id: id.into(), kind, operation: self.operation(version, argument),
            progression: Progression::Automatic, max_attempts: 1 }
    }
    pub(super) fn validate_definition(&self, definition: &Definition) -> Result<()> {
        definition.validate().map_err(|_| Error::Refused("invalid definition"))?;
        let expected = self.definition()?;
        if definition.source != expected.source || definition.stages.len() != expected.stages.len()
            || definition.stages.iter().zip(&expected.stages).any(|(a,b)|
                a.id != b.id || a.kind != b.kind || a.operation != b.operation)
        { return Err(Error::Refused("definition does not pin configured operations")); }
        Ok(())
    }
}
impl LaunchFile {
    pub(super) fn verify(&self) -> Result<()> {
        if self.path.canonicalize()? != self.path || digest(&std::fs::read(&self.path)?) != self.digest {
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
