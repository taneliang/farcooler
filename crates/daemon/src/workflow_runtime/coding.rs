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
}
impl CommandValidation for CodingCommand {
    fn validate(&self) -> Result<()> {
        if self.argv.is_empty() || self.argv.len() > 32 || !Path::new(&self.argv[0]).is_absolute()
            || self.argv.iter().any(|a| a.len() > 4096 || a.contains('\0'))
            || self.timeout_ms == 0 || self.timeout_ms > 3_600_000
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
pub(super) trait PinnedDefinition { fn validate_pinned_definition(&self, definition: &Definition) -> Result<()>; }
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
            rework_target: self.reviewer.as_ref().map(|_| "build".into()) })
    }
    fn stage(&self, id: &str, kind: StageKind, version: &str, argument: &str) -> Stage {
        Stage { id: id.into(), kind, operation: self.operation(version, argument),
            progression: Progression::Automatic, max_attempts: 1 }
    }
    fn validate_definition(&self, definition: &Definition) -> Result<()> {
        self.validate()?;
        self.validate_pinned_definition(definition)
    }
}
impl PinnedDefinition for CodingConfig {
    fn validate_pinned_definition(&self, definition: &Definition) -> Result<()> {
        definition.validate().map_err(|_| Error::Refused("invalid definition"))?;
        let mut provisioned = false;
        let mut built = false;
        let mut landed = false;
        for stage in &definition.stages {
            let (version, argument) = match &stage.kind {
                StageKind::Provision if !provisioned && !built => { provisioned = true; ("coding-worktree/v1", String::new()) },
                StageKind::Build if provisioned && !landed => { built = true; ("coding-harness/v1", String::new()) },
                StageKind::Check if built => {
                    let index = stage.operation.arguments.get(1).and_then(|s| s.parse::<usize>().ok())
                        .filter(|i| *i < self.checks.len()).ok_or(Error::Refused("unknown configured check"))?;
                    ("coding-check/v1", index.to_string())
                },
                StageKind::Review(_) if built && (!landed || definition.max_reworks == 0) =>
                    ("coding-review/v1", self.reviewer.as_ref().ok_or(Error::Refused("missing reviewer"))?.identity.clone()),
                StageKind::Land if built && !landed => { landed = true; ("coding-land/v1", self.landing_ref.clone()) },
                StageKind::Ci if landed => ("coding-ci/v1", self.ci.repository.clone()),
                _ => return Err(Error::Refused("unsupported operation or unsafe prerequisites")),
            };
            if stage.operation != self.operation(version, &argument) {
                return Err(Error::Refused("definition does not pin configured operations"));
            }
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
