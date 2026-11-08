//! Configured execution and fact-based reconciliation. Nothing here grants wire authority.
use super::*;
use super::coding::FileValidation;
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

/// Write an agent gate's verdict where its step reads it (ov-969): `farcooler workflow verdict`, run by the reviewing
/// agent, whose environment the step pinned. `env` reads that environment. Returns where the receipt went.
pub fn write_agent_verdict(env: impl Fn(&str) -> Option<String>, passed: bool) -> Result<PathBuf> {
    let var = |key: &str| env(key).filter(|v| !v.is_empty()).ok_or(Error::Refused("not run by an agent's review gate"));
    let receipt = PathBuf::from(var("FARCOOLER_RECEIPT")?);
    let parent = receipt.parent().filter(|p| receipt.is_absolute() && p.is_dir())
        .ok_or(Error::Refused("not run by an agent's review gate"))?;
    let body = serde_json::to_vec(&CodingReviewReceipt { binding: var("FARCOOLER_OPERATION_BINDING")?,
        identity: AGENT_REVIEWER.into(), candidate: var("FARCOOLER_CANDIDATE")?, passed })?;
    // A fresh file renamed into place: the step reads one whole verdict, the last one given, and never follows a link.
    let draft = parent.join(format!(".verdict-{}", Uuid::now_v7()));
    let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600).custom_flags(libc::O_NOFOLLOW).open(&draft)?;
    std::io::Write::write_all(&mut file, &body)?;
    file.sync_all()?;
    std::fs::rename(&draft, &receipt)?;
    Ok(receipt)
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

impl Exec {
    fn fact_path(&self, binding: &Binding) -> PathBuf {
        self.root.join("receipts").join(format!("{}.process.json", digest(&binding.effect)))
    }
    pub(super) fn owner_path(&self, binding: &Binding) -> PathBuf {
        self.root.join("receipts").join(format!("{}.owner.json", digest(&binding.effect)))
    }
    fn provider_path(&self, binding: &Binding) -> PathBuf {
        self.root.join("receipts").join(format!("{}.provider.json", digest(&binding.effect)))
    }
    /// The Run gate's ledger of what it changed in a named tree (ov-979). Kept per tree, so the gate that runs
    /// again after a crash, a new effect, finds what the dead one left.
    pub(super) fn gate_files_path(&self, binding: &Binding) -> PathBuf {
        self.root.join("receipts").join(format!("{}.gate-files.json", digest(&(&binding.workspace, &binding.effect.item))))
    }
    /// Why a named tree's Run gate last stopped the run (ov-979), until a clean tree lets it go on.
    pub(super) fn tree_stop_path(&self, workspace: &str, item: &str) -> PathBuf {
        self.root.join("receipts").join(format!("{}.tree-stop.json", digest(&(workspace, item))))
    }
    pub(super) fn process_log(&self, binding: &Binding) -> PathBuf {
        self.root.join("receipts").join(format!("{}.output", digest(&binding.effect)))
    }
    fn git_settlement(&self, binding: &Binding) -> PathBuf {
        self.root.join("receipts").join(format!("{}.git-settled.json", digest(&binding.effect)))
    }
    pub(super) fn bytes(&self, path: &Path) -> Result<Vec<u8>> {
        // A producer may leave a FIFO: no-follow alone still blocks in open.
        // Inspect the opened descriptor before any read, as charter_read does.
        let file = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.nlink() != 1 || metadata.len() > 8 * 1024 * 1024 {
            return Err(Error::Refused("nonregular, aliased or oversized coding receipt"));
        }
        use std::io::Read;
        let mut bytes = Vec::new(); file.take(8 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > 8 * 1024 * 1024 { return Err(Error::Refused("oversized coding receipt")); }
        Ok(bytes)
    }
    pub(super) fn coding_command(&self, binding: &Binding) -> Result<CodingCommand> {
        self.bound_command(binding, true)
    }
    // Reconstruct immutable expected argv/environment without reading mutable
    // launch files. Only fresh execution validates those files.
    fn bound_command(&self, binding: &Binding, launch: bool) -> Result<CodingCommand> {
        let config = self.coding.as_ref().ok_or(Error::Refused("missing coding config"))?;
        let operation = &binding.effect.operation;
        let EffectKind::Execute(kind) = &binding.effect.kind else { return Err(Error::Refused("not an execution")); };
        let mut command = match kind {
            // A build, or an agent's review gate (ov-969): both start the agent the run picked.
            StageKind::Build | StageKind::Review(Reviewer::Agent) => match &config.harness {
                CodingHarness::Command(command) if matches!(kind, StageKind::Build) => command.clone(),
                CodingHarness::Command(_) => return Err(Error::Refused("an agent's review gate needs a native harness")),
                CodingHarness::Native { preset, settings, project_hooks, plugin_dir, prompt: _, process } => {
                    // The model the run picked at Start, or the configured preset when the workflow lists none (ov-966).
                    let preset = binding.effect.model.as_deref().unwrap_or(preset);
                    let claude = preset.split(':').next() == Some("claude");
                    if let Some(settings) = settings.as_ref().filter(|_| launch && claude) { settings.verify()?; }
                    if let Some(hooks) = project_hooks.as_ref().filter(|_| launch && !claude) {
                        let relative = hooks.path.strip_prefix(&config.repository)
                            .map_err(|_| Error::Refused("foreign native project hooks"))?;
                        LaunchFile { path: self.tree(binding).join(relative), digest: hooks.digest.clone() }.verify()?;
                    }
                    let extras = crate::service::LaunchExtras { settings: settings.as_ref().map(|s| s.path.clone()),
                        plugin_dir: plugin_dir.clone(), prompt: Some(crate::service::LaunchPrompt::Inline(self.native_prompt(binding)?)),
                        ..Default::default() };
                    let launch = crate::service::preset_command_with_hooks(preset, Some(&Self::native_session(binding)), &extras);
                    let mut command = process.clone();
                    command.argv = vec!["/bin/sh".into(), "-c".into(), launch];
                    #[cfg(test)]
                    if let Some(argv) = super::coding_native::TEST_ARGV.with(|p| p.borrow().clone()) {
                        command.argv = argv;
                        // The fake agent reports what it was launched with, as a real one would, so the turn check can't drift.
                        command.environment.insert("TEST_LAUNCHED_PROMPT".into(), self.native_prompt(binding)?);
                    }
                    command.environment.entry("TERM".into()).or_insert_with(|| "xterm-256color".into());
                    command.environment.entry("COLORTERM".into()).or_insert_with(|| "truecolor".into());
                    command.environment.insert("FARCOOLER_NATIVE_SESSION".into(), Self::native_session(binding));
                    command.environment.insert("FARCOOLER_NATIVE_HOOK_SOCKET".into(), self.native_socket(binding).to_string_lossy().into_owned());
                    command
                }
            },
            StageKind::Check | StageKind::Run => config.checks.get(operation.arguments.get(1)
                .and_then(|s| s.parse::<usize>().ok()).ok_or(Error::Refused("missing check index"))?)
                .ok_or(Error::Refused("foreign check"))?.clone(),
            StageKind::Review(_) => config.reviewer.as_ref().ok_or(Error::Refused("missing reviewer"))?.command.clone(),
            StageKind::Ci => config.ci.command.clone(),
            _ => return Err(Error::Refused("not a process operation")),
        };
        self.pin_context(&mut command, binding, &binding.effect.candidate.clone().unwrap_or_default(), &self.provider_path(binding));
        Ok(command)
    }
    /// What a native agent is launched with: a build's published prompt, or the review brief for an agent's gate.
    pub(super) fn native_prompt(&self, binding: &Binding) -> Result<String> {
        let Some(CodingConfig { harness: CodingHarness::Native { prompt, .. }, .. }) = &self.coding else {
            return Err(Error::Refused("not a native harness"));
        };
        Ok(match &binding.effect.kind {
            EffectKind::Execute(StageKind::Review(Reviewer::Agent)) => Self::reviewing(&binding.effect.item),
            _ => Self::briefed(prompt, &binding.effect),
        })
    }
    /// An agent gate's brief (ov-969). Fixed text, so a recovery rebuilds the same command. The verdict is the
    /// receipt `farcooler workflow verdict` writes: whatever the agent says in its turn decides nothing.
    pub(super) fn reviewing(item: &str) -> String {
        format!("You are reviewing ticket {item}. Read it first with `farcooler task show {item}`. The work under review is \
            the commits from $FARCOOLER_BASE to $FARCOOLER_CANDIDATE in this worktree. Review it against the ticket. Don't \
            change, commit or stage anything here.\n\nWhen you've decided, record your verdict with \
            `farcooler workflow verdict --passed true` or `farcooler workflow verdict --passed false`, then end your turn. \
            Only that command's verdict counts. If you end your turn without one, the review hasn't happened, and it \
            runs again.")
    }
    /// The published prompt, led by the card this run is for. The card's own words are read by the agent
    /// (`farcooler task show`), not copied here, so a recovery rebuilds the same command whatever the card says now.
    /// What the effect itself carries is said: an adopted branch's earlier work, and why the run was sent back (ov-968).
    pub(super) fn briefed(prompt: &str, effect: &farcooler_workflow::Effect) -> String {
        let item = &effect.item;
        let mut brief = if effect.with.is_empty() {
            format!("You are working on ticket {item}. Read it first with `farcooler task show {item}`, then do the work it describes.")
        } else {
            // One run for several tickets (ov-970): one branch, and one pull request, for all of them.
            let all: Vec<&str> = std::iter::once(item.as_str()).chain(effect.with.iter().map(String::as_str)).collect();
            let (last, rest) = all.split_last().expect("at least two");
            format!("You are working on tickets {} and {last}, together, in this one worktree and branch. Read each first with \
                `farcooler task show <ticket>`, then do the work they describe.", rest.join(", "))
        };
        if let Some(tree) = effect.tree.as_ref().filter(|t| t.adopted) {
            brief.push_str(&format!("\n\nThis worktree is on the branch {}, which already has work on it. Read it first with \
                `git log --stat $FARCOOLER_BASE..HEAD`, carry on from it, and bring the branch onto $FARCOOLER_BASE (main as it was when \
                the run started) before you finish.", tree.branch));
        }
        if let Some(reason) = &effect.brief {
            let (said, cut) = cut_at(reason, MAX_BRIEF_REASON_BYTES);
            brief.push_str(&format!("\n\nThis ticket was sent back to you. Your earlier work is in this worktree; carry on from it. \
                The reason it was sent back:\n\n{said}"));
            if cut { brief.push_str(&format!("…\n\n(The reason is cut short here. Read all of it with `farcooler workflow get`, under {item}.)")); }
        }
        format!("{brief}\n\n{prompt}")
    }
    /// Context wins over command environment and is pinned in the process fact.
    pub(super) fn pin_context(&self, command: &mut CodingCommand, binding: &Binding, candidate: &str, receipt: &Path) {
        let effect = &binding.effect;
        for (key, value) in [
            ("FARCOOLER_OPERATION_BINDING", digest(binding)),
            ("FARCOOLER_WORKFLOW_RUN", effect.run.clone()),
            ("FARCOOLER_WORKFLOW_FENCE", effect.fence.to_string()),
            ("FARCOOLER_RESOURCE_GENERATION", binding.generation.to_string()),
            ("FARCOOLER_DEFINITION_PIN", effect.definition_pin.clone()),
            ("FARCOOLER_POLICY_PIN", effect.policy_pin.clone()),
            ("FARCOOLER_CANDIDATE", candidate.to_string()),
            ("FARCOOLER_BASE", binding.base.clone()),
            ("FARCOOLER_ITERATION", binding.iteration.to_string()),
            ("FARCOOLER_RECEIPT", receipt.to_string_lossy().into_owned()),
            // Which card the run is for (ov-962): the published prompt is the same for every ticket on the board.
            ("FARCOOLER_WORKFLOW_ITEM", effect.item.clone()),
            ("FARCOOLER_WORKSPACE", binding.workspace.clone()),
        ] { command.environment.insert(key.into(), value); }
        // The model the run picked at Start (ov-966); a native harness launches it, a command one is told.
        if let Some(model) = &effect.model { command.environment.insert("FARCOOLER_WORKFLOW_MODEL".into(), model.clone()); }
    }
    /// A gate that was ended mid-run (a restart, a crash) may have left files in the tree. In a private tree
    /// they are ours and the candidate is a commit, so start the run from exactly the commit. A tree the app
    /// lists is one people edit in: any change there stops the run, and the person keeps or discards it; the run
    /// never removes or puts back a file there itself (ov-967, ov-979).
    pub(super) async fn settle_run_tree(&self, binding: &Binding, tree: &Path) -> Result<()> {
        if binding.effect.tree.is_some() {
            let stop = self.tree_stop_path(&binding.workspace, &binding.effect.item);
            match named_tree::stop_on_changes(tree, &self.gate_files_path(binding), &binding.effect.run,
                binding.effect.candidate.as_deref()).await {
                // Kept for the app, which shows the person why the run stopped and what changed.
                Err(Error::Kept(text)) => {
                    durable(&stop, &serde_json::to_vec(&named_tree::TreeStop { run: binding.effect.run.clone(), text: text.clone() })?)?;
                    Err(Error::Kept(text))
                }
                Ok(()) => named_tree::end_gate(&stop),
                other => other,
            }
        } else {
            self.git_recorded(binding, tree, &["reset", "--hard", "--quiet"]).await?;
            self.git_recorded(binding, tree, &["clean", "-fdq"]).await?;
            Ok(())
        }
    }
    async fn run_coding(&self, binding: &Binding, command: CodingCommand,
        cancel: tokio::sync::watch::Receiver<bool>) -> Result<()> {
        let log = self.process_log(binding);
        // Each admitted effect is invoked at most once. A preexisting artifact is
        // evidence to reconcile, never permission to append a new invocation.
        durable(&log, b"")?;
        // A named tree is one people edit in: what a Run gate changes there is noted as it runs, so a crash that
        // stops the run can name what changed while the gate ran (ov-979). They are only named, never cleared.
        let ledger = (binding.effect.tree.is_some() && matches!(binding.effect.kind, EffectKind::Execute(StageKind::Run)))
            .then(|| self.gate_files_path(binding));
        let looking = match &ledger {
            Some(ledger) => {
                named_tree::begin_gate(&self.tree(binding), ledger, &binding.effect.run, binding.effect.candidate.as_deref()).await?;
                let (tree, ledger) = (self.tree(binding), ledger.clone());
                Some(tokio::spawn(async move { loop {
                    tokio::time::sleep(named_tree::GATE_LOOK).await;
                    if let Err(error) = named_tree::note_gate_files(&tree, &ledger).await {
                        tracing::warn!(%error, "the gate's changes could not be noted");
                    }
                }}))
            }
            None => None,
        };
        let exit = async {
            Ok::<_, Error>(if self.is_native_build(binding) {
                self.run_native(binding, &command, cancel).await?
            } else {
                // A gate can run for hours and leave the daemon behind it: its owner is recorded so a
                // recovery can end it before it runs again (a group's gate holds the whole landing target).
                let owner = matches!(binding.effect.kind, EffectKind::Execute(StageKind::Run)).then(|| self.owner_path(binding));
                crate::effect_runner::owned(&self.tree(binding), &command.argv, &command.environment,
                    &log, Duration::from_millis(command.timeout_ms), cancel, owner.as_deref()).await?
            })
        }.await;
        // However the gate ended, its ledger goes with it; only a crash leaves one behind.
        if let Some(looking) = looking { looking.abort(); let _ = looking.await; }
        if let Some(ledger) = &ledger { named_tree::end_gate(ledger)?; }
        let exit = exit?;
        let fact = ProcessFact { binding: binding.clone(), argv: command.argv,
            environment: command.environment, exit, log_digest: digest(&self.bytes(&log)?) };
        durable(&self.fact_path(binding), &serde_json::to_vec(&fact)?)
    }
    fn check_binding(&self, workspace: &str, binding: &Binding) -> Result<()> {
        if binding.workspace != workspace || !self.resource_matches(workspace, &binding.effect)
            || binding.base != self.item_base(workspace, &binding.effect)? {
            return Err(Error::Refused("foreign coding binding"));
        }
        let state = self.state(workspace)?.ok_or(Error::Refused("missing workspace"))?;
        let item = state.items.get(&binding.effect.item).ok_or(Error::Refused("missing item"))?;
        self.validate_item_definition(item)?;
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
        if self.is_native_agent(&original_binding) && !self.fact_path(&original_binding).try_exists()? {
            self.cancel_interrupted_native(&original_binding).await?;
            let command = self.bound_command(&original_binding, false)?;
            let fact = ProcessFact { binding: original_binding.clone(), argv: command.argv,
                environment: command.environment, exit: None,
                log_digest: digest(&self.bytes(&self.process_log(&original_binding))?) };
            durable(&self.fact_path(&original_binding), &serde_json::to_vec(&fact)?)?;
        }
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
        self.coding.as_ref().ok_or(Error::Refused("missing coding config"))?.validate()?;
        self.validate_item_definition(&self.state(workspace)?.ok_or(Error::Refused("missing workspace"))?.items[&binding.effect.item])?;
        let tree = self.tree(binding);
        let EffectKind::Execute(kind) = &binding.effect.kind else { unreachable!() };
        match kind {
            StageKind::Provision => match &binding.effect.tree {
                Some(named) => self.provision_named(workspace, binding, named).await?,
                None => {
                    let branch = format!("wf-{}", &digest(&(workspace, &binding.effect.item))[4..]);
                    crate::git::create_worktree_with(&self.repository(), &branch, &binding.base, &tree, true).await
                        .map_err(|_| Error::Refused("configured worktree provisioning uncertain"))?;
                }
            },
            StageKind::AwaitBase | StageKind::Fold | StageKind::Push | StageKind::AwaitCi | StageKind::Route =>
                return self.execute_verb(binding, kind, cancel).await,
            StageKind::PushBranch | StageKind::OpenPr | StageKind::PrReady | StageKind::AwaitPr =>
                return self.execute_pr_verb(binding, kind, cancel).await,
            StageKind::Build | StageKind::Check | StageKind::Run | StageKind::Review(_) | StageKind::Ci => {
                if matches!(kind, StageKind::Run) && tree.try_exists()? { self.settle_run_tree(binding, &tree).await?; }
                if !matches!(kind, StageKind::Build) { self.verify_candidate(binding, &tree).await?; }
                if matches!(kind, StageKind::Ci) { self.verify_landing(binding).await?; }
                self.run_coding(binding, self.coding_command(binding)?, cancel).await?;
            }
            StageKind::Land => {
                self.verify_candidate(binding, &tree).await?;
                let candidate = binding.effect.candidate.as_deref().ok_or(Error::Refused("missing landing candidate"))?;
                self.land_remote(binding, cancel).await?;
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
    /// A gate's answer. It judges the candidate commit: a nonzero exit, or a tree the gate changed, is
    /// the candidate failing (`rejected`, which the definition routes: retry, split or fail). It is never
    /// a reason to leave the target stuck. With no fact the gate died with the daemon: whatever of it
    /// still runs is ended, and the step runs anew on a clean tree.
    async fn reconcile_run(&self, binding: &Binding) -> Result<Outcome> {
        let raw = match self.bytes(&self.fact_path(binding)) {
            Ok(raw) => raw,
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                if !crate::effect_runner::Owner::reap(&self.owner_path(binding)).await? {
                    return Err(Error::Refused("the gate's process could not be ended"));
                }
                return Ok(Outcome::SafeFailure);
            }
            Err(error) => return Err(error),
        };
        let fact: ProcessFact = serde_json::from_slice(&raw)?;
        let command = self.bound_command(binding, false)?;
        if fact.binding != *binding || fact.argv != command.argv || fact.environment != command.environment
            || fact.log_digest != digest(&self.bytes(&self.process_log(binding))?) {
            return Err(Error::Refused("configured process receipt mismatch"));
        }
        // Killed, timed out or cancelled: no verdict on the candidate.
        let Some(code) = fact.exit else { return Ok(Outcome::SafeFailure); };
        let candidate = binding.effect.candidate.clone().filter(|c| sha(c)).ok_or(Error::Refused("missing candidate"))?;
        let unchanged = code == 0 && self.verify_candidate(binding, &self.tree(binding)).await.is_ok();
        let manifest = digest(&(binding, &candidate, &raw, unchanged));
        let evidence = Evidence { candidate, manifest: manifest.clone(), receipt: digest(&("coding-receipt/v1", manifest)),
            policy_pin: binding.effect.policy_pin.clone() };
        Ok(if unchanged { Outcome::Passed(evidence) } else { Outcome::Rejected(evidence) })
    }
    pub(super) async fn verify_tree(&self, binding: &Binding) -> Result<String> {
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
        let owned = binding.effect.tree.as_ref().map_or_else(|| format!("wf-{}", &digest(&(&binding.workspace, &binding.effect.item))[4..]), |t| t.branch.clone());
        if branch != format!("refs/heads/{owned}") {
            return Err(Error::Refused("worktree branch is not owned by this item"));
        }
        Ok(candidate)
    }
    async fn verify_landing(&self, binding: &Binding) -> Result<()> {
        self.verify_remote_landing(binding).await?;
        // Another item may have landed on top since; this item's candidate only has to be in
        // the ref's history, not be its tip.
        let candidate = binding.effect.candidate.as_deref().unwrap_or("");
        if !sha(candidate) || self.git_recorded(binding, &self.repository(),
            &["merge-base", "--is-ancestor", candidate, self.landing_ref()]).await.is_err() {
            return Err(Error::Refused("landed SHA differs from CI candidate"));
        }
        Ok(())
    }
    /// Reconcile facts only. This function can never spawn an adapter command.
    pub(super) async fn reconcile_coding(&self, binding: &Binding) -> Result<Outcome> {
        self.check_binding(&binding.workspace, binding)?;
        if let EffectKind::Stop { operation } = &binding.effect.kind { return self.stop_coding(binding, operation).await; }
        let EffectKind::Execute(kind) = &binding.effect.kind else { unreachable!() };
        if super::coding_verbs::is_verb(kind) { return self.reconcile_verb(binding, kind).await; }
        if matches!(kind, StageKind::Run) { return self.reconcile_run(binding).await; }
        let mut facts = Vec::new();
        if matches!(kind, StageKind::Provision | StageKind::Land) {
            let raw = self.bytes(&self.git_settlement(binding))?;
            let settlement: GitSettlement = serde_json::from_slice(&raw)?;
            if settlement.binding != *binding { return Err(Error::Refused("foreign Git settlement")); }
            facts.extend(raw);
        }
        if matches!(kind, StageKind::Build | StageKind::Check | StageKind::Run | StageKind::Review(_) | StageKind::Ci) {
            let raw = self.bytes(&self.fact_path(binding))?;
            let fact: ProcessFact = serde_json::from_slice(&raw)?;
            let command = self.bound_command(binding, false)?;
            if fact.binding != *binding || fact.argv != command.argv || fact.environment != command.environment
                || fact.log_digest != digest(&self.bytes(&self.process_log(binding))?) {
                return Err(Error::Refused("configured process receipt mismatch"));
            }
            if self.is_native_agent(binding) { self.verify_native_settlement(binding, fact.exit)?; }
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
                // An adopted branch starts at its own work (ov-968); the build brings it onto the base.
                let adopted = binding.effect.tree.as_ref().is_some_and(|t| t.adopted);
                if candidate != binding.base && !adopted { return Err(Error::Refused("provisioned base changed")); }
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
            StageKind::Build | StageKind::Check | StageKind::Run | StageKind::Review(_) | StageKind::Ci => {
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
                    let agent = matches!(kind, StageKind::Review(Reviewer::Agent));
                    let raw = match self.bytes(&self.provider_path(binding)) {
                        // An agent that ended its turn without a verdict gave none: it never passes, and it runs again (ov-969).
                        Err(Error::Io(error)) if agent && error.kind() == std::io::ErrorKind::NotFound => return Ok(Outcome::SafeFailure),
                        raw => raw?,
                    };
                    let config = self.coding.as_ref().ok_or(Error::Refused("missing config"))?;
                    if matches!(kind, StageKind::Review(_)) {
                        let receipt: CodingReviewReceipt = serde_json::from_slice(&raw)?;
                        let identity = if agent { Some(AGENT_REVIEWER) } else { config.reviewer.as_ref().map(|r| r.identity.as_str()) };
                        if receipt.binding != digest(binding) || receipt.candidate != candidate
                            || Some(receipt.identity.as_str()) != identity {
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

/// The most of a send-back reason the launch command carries, so it stays inside what tmux takes in one command.
pub(super) const MAX_BRIEF_REASON_BYTES: usize = 1500;
/// `text` cut to at most `max` bytes on a character boundary, and whether it was cut.
fn cut_at(text: &str, max: usize) -> (&str, bool) {
    if text.len() <= max { return (text, false); }
    let end = (0..=max).rev().find(|&i| text.is_char_boundary(i)).unwrap_or(0);
    (&text[..end], true)
}
