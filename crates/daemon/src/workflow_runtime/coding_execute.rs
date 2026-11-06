//! Configured execution and fact-based reconciliation. Nothing here grants wire authority.
use super::*;
use super::coding::sha;
use farcooler_store::board_ci::CiStatus;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProcessFact {
    binding: Binding,
    argv: Vec<String>,
    environment: std::collections::BTreeMap<String, String>,
    exit: Option<i32>,
    log_digest: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GitSettlement { binding: Binding }

/// Output of the configured trusted reviewer hook, never parsed from agent prose.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodingReviewReceipt {
    pub binding: String,
    pub identity: String,
    pub candidate: String,
    pub passed: bool,
}

/// The provider hook must supply a real provider observation (offline tests use
/// deterministic stand-ins). The embedded run is parsed by retained ci_watch.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodingCiReceipt {
    pub binding: String,
    pub repository: String,
    pub run: serde_json::Value,
}

impl Runtime {
    fn fact_path(&self, binding: &Binding) -> PathBuf {
        self.root.join("receipts").join(format!("{}.process.json", digest(&binding.effect)))
    }
    fn provider_path(&self, binding: &Binding) -> PathBuf {
        self.root.join("receipts").join(format!("{}.provider.json", digest(&binding.effect)))
    }
    fn process_log(&self, binding: &Binding) -> PathBuf {
        self.root.join("receipts").join(format!("{}.output", digest(&binding.effect)))
    }
    fn git_settlement(&self, binding: &Binding) -> PathBuf {
        self.root.join("receipts").join(format!("{}.git-settled.json", digest(&binding.effect)))
    }
    fn bytes(&self, path: &Path) -> Result<Vec<u8>> {
        let file = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(path)?;
        if file.metadata()?.nlink() != 1 || file.metadata()?.len() > 8 * 1024 * 1024 {
            return Err(Error::Refused("aliased or oversized coding receipt"));
        }
        use std::io::Read;
        let mut bytes = Vec::new(); file.take(8 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > 8 * 1024 * 1024 { return Err(Error::Refused("oversized coding receipt")); }
        Ok(bytes)
    }
    pub(super) fn coding_command(&self, binding: &Binding) -> Result<CodingCommand> {
        let config = self.coding.as_ref().ok_or(Error::Refused("missing coding config"))?;
        let operation = &binding.effect.operation;
        let EffectKind::Execute(kind) = &binding.effect.kind else { return Err(Error::Refused("not an execution")); };
        let mut command = match kind {
            StageKind::Build => match &config.harness {
                CodingHarness::Command(command) => command.clone(),
                CodingHarness::Native { preset, settings, project_hooks, plugin_dir, prompt, process } => {
                    if let Some(settings) = settings { settings.verify()?; }
                    if let Some(hooks) = project_hooks {
                        let relative = hooks.path.strip_prefix(&config.repository)
                            .map_err(|_| Error::Refused("foreign native project hooks"))?;
                        LaunchFile { path: self.tree(binding).join(relative), digest: hooks.digest.clone() }.verify()?;
                    }
                    let extras = crate::service::LaunchExtras { settings: settings.as_ref().map(|s| s.path.clone()),
                        plugin_dir: plugin_dir.clone(), prompt: Some(crate::service::LaunchPrompt::Inline(prompt.clone())),
                        ..Default::default() };
                    let launch = crate::service::preset_command_with_hooks(preset, None, &extras);
                    let mut command = process.clone();
                    command.argv = vec!["/bin/sh".into(), "-c".into(), launch]; command
                }
            },
            StageKind::Check => config.checks.get(operation.arguments.get(1)
                .and_then(|s| s.parse::<usize>().ok()).ok_or(Error::Refused("missing check index"))?)
                .ok_or(Error::Refused("foreign check"))?.clone(),
            StageKind::Review(_) => config.reviewer.as_ref().ok_or(Error::Refused("missing reviewer"))?.command.clone(),
            StageKind::Ci => config.ci.command.clone(),
            _ => return Err(Error::Refused("not a process operation")),
        };
        // Context wins over command environment and is pinned in the process fact.
        let effect = &binding.effect;
        for (key, value) in [
            ("FARCOOLER_OPERATION_BINDING", digest(binding)),
            ("FARCOOLER_WORKFLOW_RUN", effect.run.clone()),
            ("FARCOOLER_WORKFLOW_FENCE", effect.fence.to_string()),
            ("FARCOOLER_RESOURCE_GENERATION", binding.generation.to_string()),
            ("FARCOOLER_DEFINITION_PIN", effect.definition_pin.clone()),
            ("FARCOOLER_POLICY_PIN", effect.policy_pin.clone()),
            ("FARCOOLER_CANDIDATE", effect.candidate.clone().unwrap_or_default()),
            ("FARCOOLER_BASE", binding.base.clone()),
            ("FARCOOLER_ITERATION", binding.iteration.to_string()),
            ("FARCOOLER_RECEIPT", self.provider_path(binding).to_string_lossy().into_owned()),
        ] { command.environment.insert(key.into(), value); }
        Ok(command)
    }
    async fn run_coding(&self, binding: &Binding, command: CodingCommand,
        cancel: tokio::sync::watch::Receiver<bool>) -> Result<()> {
        let log = self.process_log(binding);
        // Each admitted effect is invoked at most once. A preexisting artifact is
        // evidence to reconcile, never permission to append a new invocation.
        durable(&log, b"")?;
        let exit = crate::effect_runner::configured(&self.tree(binding), &command.argv, &command.environment,
            &log, Duration::from_millis(command.timeout_ms), cancel).await?;
        let fact = ProcessFact { binding: binding.clone(), argv: command.argv,
            environment: command.environment, exit, log_digest: digest(&self.bytes(&log)?) };
        durable(&self.fact_path(binding), &serde_json::to_vec(&fact)?)
    }
    fn check_binding(&self, workspace: &str, binding: &Binding) -> Result<()> {
        if binding.workspace != workspace || binding.effect.resource != self.resource
            || binding.base != self.item_base(workspace, &binding.effect)? {
            return Err(Error::Refused("foreign coding binding"));
        }
        let state = self.state(workspace)?.ok_or(Error::Refused("missing workspace"))?;
        let item = state.items.get(&binding.effect.item).ok_or(Error::Refused("missing item"))?;
        self.validate_definition(&item.definition)?;
        if let EffectKind::Execute(_) = &binding.effect.kind {
            if !item.definition.stages.iter().any(|s| s.operation == binding.effect.operation) {
                return Err(Error::Refused("foreign coding operation"));
            }
        }
        Ok(())
    }
    async fn stop_coding(&self, binding: &Binding, operation: &str) -> Result<Outcome> {
        let effects = self.store.workflow_outbox(&binding.workspace)?;
        let original = effects.iter().find(|s| s.effect.id == operation)
            .ok_or(Error::Refused("missing original coding operation"))?;
        if original.dispatch_state == "suppressed" || original.resource_generation.is_none() {
            return self.stop(&binding.workspace, binding, operation);
        }
        if !matches!(original.effect.kind, EffectKind::Execute(_)) {
            return Err(Error::Refused("stop cannot settle another stop"));
        }
        let original_binding = Binding::new(&binding.workspace, &original.effect,
            original.resource_generation.ok_or(Error::Refused("missing original generation"))?,
            binding.iteration, &binding.base);
        // Reconcile the original facts even if Cancel fenced its Observe or a
        // crash lost its journal. Never execute it, and never infer that external
        // acceptance was undone. Box breaks the Stop -> original reconciliation
        // recursion; original is required to be Execute above.
        let outcome = Box::pin(self.reconcile_coding(&original_binding)).await?;
        if outcome == Outcome::Uncertain { return Err(Error::Refused("original coding acceptance remains unknown")); }
        Ok(Outcome::Stopped { operation: operation.into(), receipt: digest(&("coding-quiescent/v1", binding, &original_binding, outcome)) })
    }
    pub(super) async fn execute_coding(&self, workspace: &str, binding: &Binding,
        cancel: tokio::sync::watch::Receiver<bool>) -> Result<Outcome> {
        self.check_binding(workspace, binding)?;
        if let EffectKind::Stop { operation } = &binding.effect.kind { return self.stop_coding(binding, operation).await; }
        let tree = self.tree(binding);
        let EffectKind::Execute(kind) = &binding.effect.kind else { unreachable!() };
        match kind {
            StageKind::Provision => {
                let branch = format!("wf-{}", &digest(&(workspace, &binding.effect.item))[4..]);
                crate::git::create_worktree_with(&self.repository(), &branch, &binding.base, &tree, true).await
                    .map_err(|_| Error::Refused("configured worktree provisioning uncertain"))?;
            }
            StageKind::Build | StageKind::Check | StageKind::Review(_) | StageKind::Ci => {
                if !matches!(kind, StageKind::Build) { self.verify_candidate(binding, &tree).await?; }
                if matches!(kind, StageKind::Ci) { self.verify_landing(binding).await?; }
                self.run_coding(binding, self.coding_command(binding)?, cancel).await?;
            }
            StageKind::Land => {
                self.verify_candidate(binding, &tree).await?;
                let candidate = binding.effect.candidate.as_deref().ok_or(Error::Refused("missing landing candidate"))?;
                self.git_recorded(binding, &self.repository(), &["update-ref", "-m", &format!("workflow:{}", digest(binding)),
                    self.landing_ref(), candidate, &binding.base]).await?;
            }
            _ => return Err(Error::Refused("unsupported configured stage")),
        }
        if matches!(kind, StageKind::Provision | StageKind::Land) {
            // Written only after the retained bounded Git operation has returned.
            // A visible ref/worktree alone cannot prove its old process is done.
            durable(&self.git_settlement(binding), &serde_json::to_vec(&GitSettlement { binding: binding.clone() })?)?;
        }
        self.reconcile_coding(binding).await
    }
    async fn verify_tree(&self, binding: &Binding) -> Result<String> {
        let tree = self.tree(binding);
        let common = self.git_recorded(binding, &tree, &["rev-parse", "--path-format=absolute", "--git-common-dir"]).await?;
        if Path::new(&common).canonicalize()? != self.repository().join(".git").canonicalize()? {
            return Err(Error::Refused("worktree belongs to another repository"));
        }
        if !self.git_recorded(binding, &tree, &["status", "--porcelain", "--untracked-files=all"]).await?.is_empty() {
            return Err(Error::Refused("configured candidate is dirty"));
        }
        let candidate = self.git_recorded(binding, &tree, &["rev-parse", "HEAD"]).await?;
        if !sha(&candidate) { return Err(Error::Refused("invalid candidate SHA")); }
        let branch = self.git_recorded(binding, &tree, &["symbolic-ref", "HEAD"]).await?;
        if branch != format!("refs/heads/wf-{}", &digest(&(&binding.workspace, &binding.effect.item))[4..]) {
            return Err(Error::Refused("worktree branch is not owned by this item"));
        }
        Ok(candidate)
    }
    async fn verify_landing(&self, binding: &Binding) -> Result<()> {
        if self.git_recorded(binding, &self.repository(), &["rev-parse", self.landing_ref()]).await?
            != binding.effect.candidate.as_deref().unwrap_or("") {
            return Err(Error::Refused("landed SHA differs from CI candidate"));
        }
        Ok(())
    }
    /// Reconcile facts only. This function can never spawn an adapter command.
    pub(super) async fn reconcile_coding(&self, binding: &Binding) -> Result<Outcome> {
        self.check_binding(&binding.workspace, binding)?;
        if let EffectKind::Stop { operation } = &binding.effect.kind { return self.stop_coding(binding, operation).await; }
        let EffectKind::Execute(kind) = &binding.effect.kind else { unreachable!() };
        let mut facts = Vec::new();
        if matches!(kind, StageKind::Provision | StageKind::Land) {
            let raw = self.bytes(&self.git_settlement(binding))?;
            let settlement: GitSettlement = serde_json::from_slice(&raw)?;
            if settlement.binding != *binding { return Err(Error::Refused("foreign Git settlement")); }
            facts.extend(raw);
        }
        if matches!(kind, StageKind::Build | StageKind::Check | StageKind::Review(_) | StageKind::Ci) {
            let raw = self.bytes(&self.fact_path(binding))?;
            let fact: ProcessFact = serde_json::from_slice(&raw)?;
            let command = self.coding_command(binding)?;
            if fact.binding != *binding || fact.argv != command.argv || fact.environment != command.environment
                || fact.log_digest != digest(&self.bytes(&self.process_log(binding))?) {
                return Err(Error::Refused("configured process receipt mismatch"));
            }
            // Even a dirty interrupted build can settle cancellation once this
            // exact owned group has been reaped; it supplies no candidate success.
            if fact.exit != Some(0) { return Ok(Outcome::SafeFailure); }
            facts.extend(raw);
        }
        let tree = self.tree(binding);
        let mut candidate = self.verify_tree(binding).await?;
        let mut passed = true;
        match kind {
            StageKind::Provision => {
                if candidate != binding.base { return Err(Error::Refused("provisioned base changed")); }
            }
            StageKind::Land => {
                self.verify_candidate(binding, &tree).await?;
                self.verify_landing(binding).await?;
                let receipt = self.git_recorded(binding, &self.repository(), &["reflog", "show", "-1", "--format=%H:%gs", self.landing_ref()]).await?;
                if receipt != format!("{}:workflow:{}", candidate, digest(binding)) {
                    return Err(Error::Refused("landing lacks matching Git acceptance fact"));
                }
                facts = receipt.into_bytes();
            }
            StageKind::Build | StageKind::Check | StageKind::Review(_) | StageKind::Ci => {
                if matches!(kind, StageKind::Build) {
                    if candidate == binding.base || binding.effect.candidate.as_ref() == Some(&candidate) {
                        return Err(Error::Refused("harness did not produce a new candidate commit"));
                    }
                    self.git_recorded(binding, &tree, &["merge-base", "--is-ancestor", &binding.base, &candidate]).await?;
                    if let Some(previous) = &binding.effect.candidate {
                        self.git_recorded(binding, &tree, &["merge-base", "--is-ancestor", previous, &candidate]).await?;
                    }
                } else {
                    self.verify_candidate(binding, &tree).await?;
                    candidate = binding.effect.candidate.clone().ok_or(Error::Refused("missing candidate"))?;
                }
                if matches!(kind, StageKind::Review(_) | StageKind::Ci) {
                    let raw = self.bytes(&self.provider_path(binding))?;
                    let config = self.coding.as_ref().ok_or(Error::Refused("missing config"))?;
                    if matches!(kind, StageKind::Review(_)) {
                        let receipt: CodingReviewReceipt = serde_json::from_slice(&raw)?;
                        if receipt.binding != digest(binding) || receipt.candidate != candidate
                            || Some(receipt.identity.as_str()) != config.reviewer.as_ref().map(|r| r.identity.as_str()) {
                            return Err(Error::Refused("foreign review receipt"));
                        }
                        passed = receipt.passed;
                    } else {
                        self.verify_landing(binding).await?;
                        let receipt: CodingCiReceipt = serde_json::from_slice(&raw)?;
                        if receipt.binding != digest(binding) || receipt.repository != config.ci.repository {
                            return Err(Error::Refused("foreign CI provider identity"));
                        }
                        let run = crate::ci_watch::parse_run(&serde_json::to_vec(&receipt.run)?)
                            .ok_or(Error::Refused("invalid CI receipt"))?;
                        if run.id == 0 || run.head_sha != candidate || run.name != config.ci.workflow || run.event != config.ci.event {
                            return Err(Error::Refused("foreign CI run provenance"));
                        }
                        let read = crate::ci_watch::summarize(&format!("sha:{candidate}"), &[(run, None)]);
                        if !matches!(read.status, CiStatus::Passed | CiStatus::Failed) { return Ok(Outcome::Uncertain); }
                        passed = read.sha == candidate && read.status == CiStatus::Passed;
                    }
                    facts.extend(raw);
                }
            }
            _ => return Err(Error::Refused("unsupported configured reconciliation")),
        }
        let manifest = digest(&(binding, &candidate, &facts, passed));
        let evidence = Evidence { candidate, manifest: manifest.clone(), receipt: digest(&("coding-receipt/v1", manifest)),
            policy_pin: binding.effect.policy_pin.clone() };
        Ok(if passed { Outcome::Passed(evidence) } else { Outcome::Rejected(evidence) })
    }
}
