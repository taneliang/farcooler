//! Closed deterministic adapters. Commands are fixtures, never provider/user scripts.
use super::*;
use farcooler_store::board_ci::CiStatus;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Binding {
    pub workspace: String,
    pub effect: Effect,
    pub generation: u64,
    pub iteration: u8,
    pub base: String,
}
impl Binding {
    pub fn new(workspace: &str, effect: &Effect, generation: u64, iteration: u8, base: &str) -> Self {
        Self { workspace: workspace.into(), effect: effect.clone(), generation, iteration, base: base.into() }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Journal { pub binding: Binding, pub outcome: Outcome }

/// Settings are pinned before enqueue; no mutable command lookup occurs on retry.
pub fn coding_definition(review: bool) -> Definition {
    let mut definition = Definition::coding();
    if !review {
        definition.stages.retain(|s| !matches!(s.kind, StageKind::Review(_)));
        definition.max_reworks = 0;
        definition.rework_target = None;
    }
    for stage in &mut definition.stages { stage.operation = spec(&stage.kind).expect("closed coding stages"); }
    definition
}
fn spec(kind: &StageKind) -> Option<OperationSpec> {
    let (name, args): (&str, Vec<&str>) = match kind {
        StageKind::Provision => ("local-worktree/v1", vec!["pinned-base"]),
        StageKind::Build => ("fixture-harness/v1", vec!["draft", "ready"]),
        StageKind::Check => ("fixture-check/v1", vec!["result.txt"]),
        StageKind::Review(_) => ("fixture-review/v1", vec!["ready"]),
        StageKind::Land => ("local-land/v1", vec!["refs/heads/landed"]),
        StageKind::Ci => ("fixture-ci/v1", vec!["fixture-check"]),
        StageKind::Tracker => ("tracker/v1", vec![]),
        _ => return None,
    };
    Some(OperationSpec { adapter_version: name.into(), arguments: args.into_iter().map(Into::into).collect(),
        environment: Default::default() })
}
pub(super) fn validate(definition: &Definition) -> Result<()> {
    definition.validate().map_err(|_| Error::Refused("invalid definition"))?;
    for stage in &definition.stages {
        if spec(&stage.kind).as_ref() != Some(&stage.operation) {
            return Err(Error::Refused("unsupported or altered pinned adapter"));
        }
    }
    Ok(())
}

pub(super) async fn git(tree: &Path, args: &[&str]) -> Result<String> {
    let output = crate::git::git(tree, args).await.map_err(|_| Error::Refused("git invocation failed"))?;
    if !output.ok { return Err(Error::Refused("git refused fixture operation")); }
    Ok(output.stdout.trim().into())
}
impl Exec {
    fn base_path(&self, workspace: &str, effect: &Effect) -> PathBuf {
        self.root.join(format!("base-{}.json", digest(&(workspace, &effect.item))))
    }
    pub(super) fn item_base(&self, workspace: &str, effect: &Effect) -> Result<String> {
        let path = self.base_path(workspace, effect);
        let bytes = match self.bytes(&path) {
            Ok(bytes) => bytes,
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => return Ok(self.base.clone()),
            Err(error) => return Err(error),
        };
        let base: String = serde_json::from_slice(&bytes)?;
        if base.len() != 40 || !base.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::Refused("invalid item base"));
        }
        Ok(base)
    }
    pub(super) async fn pin_base(&self, workspace: &str, effect: &Effect) -> Result<String> {
        let path = self.base_path(workspace, effect);
        let exists = path.try_exists()?;
        let selected = if exists { self.item_base(workspace, effect) } else {
            Err(Error::Refused("base not yet published"))
        };
        if selected.is_err() {
            if exists {
                // Only the original claimed Provision can repair an old interrupted
                // publication. Claimed cannot follow admitted, and outbox history is
                // never erased, so no previously admitted item binding is resampled.
                let effects = self.store.workflow_outbox(workspace)?;
                let item_effects: Vec<_> = effects.iter().filter(|s| s.effect.item == effect.item).collect();
                if item_effects.len() != 1 || item_effects[0].effect != *effect
                    || item_effects[0].dispatch_state != "claimed"
                    || !matches!(effect.kind, EffectKind::Execute(StageKind::Provision))
                { return Err(Error::Refused("cannot repair an admitted or historical base")); }
            }
            let base = if matches!(effect.kind, EffectKind::Execute(StageKind::Provision)) {
                git(&self.repository(), &["rev-parse", self.landing_ref()]).await?
            } else { self.base.clone() };
            // A crash while writing leaves only an ignored private temporary file.
            // The exclusive fixture writer makes replacing a proven partial pin safe.
            let temporary = self.root.join(format!("base-publish-{}.tmp", Uuid::now_v7()));
            durable(&temporary, &serde_json::to_vec(&base)?)?;
            std::fs::rename(&temporary, &path)?;
        }
        // Validate and make even a recovered pre-rename/fsync selection durable
        // before the caller can cross the admission boundary.
        let base = self.item_base(workspace, effect)?;
        OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(&path)?.sync_all()?;
        File::open(&self.root)?.sync_all()?;
        Ok(base)
    }
    pub(super) fn journal_path(&self, effect: &Effect) -> PathBuf {
        self.root.join("receipts").join(format!("{}.json", digest(effect)))
    }
    pub(super) fn read_journal(&self, binding: &Binding) -> Result<Option<Journal>> {
        let path = self.journal_path(&binding.effect);
        let bytes = match self.bytes(&path) {
            Ok(bytes) => bytes,
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let journal: Journal = serde_json::from_slice(&bytes)?;
        if journal.binding != *binding { return Err(Error::Refused("receipt binding mismatch")); }
        Ok(Some(journal))
    }
    pub(super) fn write_journal(&self, journal: &Journal) -> Result<()> {
        durable(&self.journal_path(&journal.binding.effect), &serde_json::to_vec(journal)?)
    }
    pub(super) fn tree(&self, binding: &Binding) -> PathBuf {
        self.root.join(format!("tree-{}", digest(&(&binding.workspace, &binding.effect.item))))
    }
    pub(super) async fn git_recorded(&self, binding: &Binding, tree: &Path, args: &[&str]) -> Result<String> {
        let result = crate::git::git(tree, args).await;
        let raw = match &result {
            Ok(output) => serde_json::json!({ "args": args, "ok": output.ok,
                "stdout": output.stdout, "stderr": output.stderr }),
            Err(error) => serde_json::json!({ "args": args, "error": error.to_string() }),
        };
        let path = self.root.join("receipts").join(format!("{}.git.jsonl", binding.effect.id));
        let mut file = OpenOptions::new().append(true).create(true).mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path)?;
        if !file.metadata()?.is_file() || file.metadata()?.nlink()!=1 { return Err(Error::Refused("aliased or nonregular Git log")); }
        use std::io::Write;
        writeln!(file, "{raw}")?;
        file.sync_all()?;
        let output = result.map_err(|_| Error::Refused("git failed; inspect raw receipt"))?;
        if !output.ok { return Err(Error::Refused("git refused; inspect raw receipt")); }
        Ok(output.stdout.trim().into())
    }
    pub(super) async fn execute(&self, workspace: &str, binding: &Binding) -> Result<Outcome> {
        let effect = &binding.effect;
        if !self.resource_matches(workspace, effect) || binding.base != self.item_base(workspace, effect)? || binding.workspace != workspace {
            return Err(Error::Refused("foreign execution binding"));
        }
        if let EffectKind::Stop { operation } = &effect.kind {
            return self.stop(workspace, binding, operation);
        }
        let EffectKind::Execute(kind) = &effect.kind else { unreachable!() };
        if spec(kind).as_ref() != Some(&effect.operation) { return Err(Error::Refused("unknown adapter")); }
        let tree = self.tree(binding);
        let repo = self.root.join("repo");
        let log = self.root.join("receipts").join(format!("{}.log", effect.id));
        let mut candidate = effect.candidate.clone().unwrap_or_else(|| binding.base.clone());
        let mut passed = true;
        match kind {
            StageKind::Provision => {
                let branch = format!("wf-{}", &digest(&(workspace, &effect.item))[4..]);
                crate::git::create_worktree_with(&repo, &branch, &binding.base, &tree, true).await
                    .map_err(|_| Error::Refused("worktree provisioning uncertain"))?;
                self.git_recorded(binding, &tree, &["rev-parse", "HEAD"]).await?;
            }
            StageKind::Build => {
                // The fixture harness has the same owned terminal-process lifecycle as
                // the retained gate; it deliberately invokes no signed-in provider.
                let script = if binding.iteration == 0 { "printf 'draft\\n' > result.txt; printf 'draft candidate\\n'" }
                    else { "printf 'ready\\n' > result.txt; printf 'reworked candidate\\n'" };
                if !self.run_fixture(binding, &tree, script, &log).await? {
                    return Err(Error::Refused("harness did not complete safely"));
                }
                self.git_recorded(binding, &tree, &["add", "--", "result.txt"]).await?;
                self.git_recorded(binding, &tree, &["-c", "user.name=Workflow Fixture", "-c",
                    "user.email=fixture@localhost", "-c", "commit.gpgsign=false", "commit", "--quiet",
                    "--allow-empty", "-m", &format!("fixture {}", effect.run)]).await?;
                candidate = self.git_recorded(binding, &tree, &["rev-parse", "HEAD"]).await?;
            }
            StageKind::Check | StageKind::Review(_) => {
                self.verify_candidate(binding, &tree).await?;
                let script = if matches!(kind, StageKind::Check) { "test -s result.txt" }
                    else { "test \"$(cat result.txt)\" = ready" };
                passed = self.run_fixture(binding, &tree, script, &log).await?;
                self.verify_candidate(binding, &tree).await?;
            }
            StageKind::Land => {
                self.verify_candidate(binding, &tree).await?;
                // Local analogue of retained landing's gated-base CAS; no remote push.
                self.git_recorded(binding, &repo, &["update-ref", "refs/heads/landed", &candidate, &binding.base]).await?;
                if self.git_recorded(binding, &repo, &["rev-parse", "refs/heads/landed"]).await? != candidate {
                    return Err(Error::Refused("landing moved after CAS"));
                }
            }
            StageKind::Ci => {
                if self.git_recorded(binding, &repo, &["rev-parse", "refs/heads/landed"]).await? != candidate {
                    return Err(Error::Refused("CI candidate is not landed"));
                }
                let raw = serde_json::to_vec(&serde_json::json!({ "id": binding.generation,
                    "name": "fixture-check", "head_sha": candidate, "status": "completed", "conclusion": "success",
                    "html_url": "fixture://local-ci", "created_at": "2026-01-01T00:00:00Z", "event": "push" }))?;
                durable(&log, &raw)?;
                let run = crate::ci_watch::parse_run(&raw).ok_or(Error::Refused("invalid CI receipt"))?;
                if run.head_sha != candidate || run.name != "fixture-check" || run.event != "push" {
                    return Err(Error::Refused("foreign CI provenance"));
                }
                let read = crate::ci_watch::summarize(&format!("sha:{candidate}"), &[(run, None)]);
                passed = read.sha == candidate && read.status == CiStatus::Passed;
            }
            StageKind::Tracker => {}
            _ => return Err(Error::Refused("unsupported adapter kind")),
        }
        let manifest = digest(&(binding, &candidate, passed));
        let evidence = Evidence { candidate, manifest: manifest.clone(), receipt: digest(&("receipt/v1", manifest)),
            policy_pin: effect.policy_pin.clone() };
        Ok(if passed { Outcome::Passed(evidence) } else { Outcome::Rejected(evidence) })
    }
    async fn run_fixture(&self, binding: &Binding, tree: &Path, script: &str, log: &Path) -> Result<bool> {
        let effect = &binding.effect;
        let generation = binding.generation.to_string();
        let fence = effect.fence.to_string();
        let environment = [("FARCOOLER_WORKFLOW_RUN", effect.run.as_str()),
            ("FARCOOLER_WORKFLOW_FENCE", fence.as_str()), ("FARCOOLER_RESOURCE_GENERATION", generation.as_str()),
            ("FARCOOLER_DEFINITION_PIN", effect.definition_pin.as_str()),
            ("FARCOOLER_CANDIDATE", effect.candidate.as_deref().unwrap_or(""))];
        Ok(crate::effect_runner::fixture(tree, script, log, Duration::from_secs(5), &environment).await?)
    }
    pub(super) async fn verify_candidate(&self, binding: &Binding, tree: &Path) -> Result<()> {
        let candidate = binding.effect.candidate.as_ref().ok_or(Error::Refused("missing candidate"))?;
        if self.git_recorded(binding, tree, &["rev-parse", "HEAD"]).await? != *candidate
            || !self.git_recorded(binding, tree, &["status", "--porcelain"]).await?.is_empty()
        { return Err(Error::Refused("candidate changed or dirty")); }
        Ok(())
    }
    pub(super) fn stop(&self, workspace: &str, binding: &Binding, operation: &str) -> Result<Outcome> {
        let effects = self.store.workflow_outbox(workspace)?;
        let original = effects.iter().find(|s| s.effect.id == operation)
            .ok_or(Error::Refused("missing original operation"))?;
        // Suppressed pre-admission work has never crossed the external boundary.
        // An admitted operation needs the runner's own quiescent settlement receipt.
        if original.resource_generation.is_some() && original.dispatch_state != "suppressed" {
            let path = self.journal_path(&original.effect);
            let journal: Journal = serde_json::from_slice(&self.bytes(&path)?)?;
            if journal.binding.effect != original.effect || journal.binding.workspace != workspace
                || Some(journal.binding.generation) != original.resource_generation
                || journal.binding.base != binding.base || journal.outcome == Outcome::Uncertain
            { return Err(Error::Refused("original settlement not authenticated")); }
        }
        Ok(Outcome::Stopped { operation: operation.into(), receipt: digest(&("quiescent/v1", binding, operation)) })
    }
}
