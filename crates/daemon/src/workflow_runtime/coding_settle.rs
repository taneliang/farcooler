//! Fenced recovery of worktree, build, and push effects with uncertain outcomes.
use super::*;
use super::coding::sha;
use super::coding::sha as sha_ok;

impl Runtime {
    /// The wire's Settle: the reply is a receipt-shaped answer, but no engine state is written, so a
    /// settle that succeeded and is sent again is refused because the item is no longer uncertain.
    pub(super) async fn settle_wire(&self, workspace: &str, item: &str, run: &str, pin: &str, landed: bool, epoch: u64)
        -> Result<serde_json::Value>
    {
        let state = self.state(workspace)?.ok_or(Error::Refused("missing workspace"))?;
        let answer = if state.epoch != epoch || !state.items.get(item).is_some_and(|i| i.run == run && i.pin == pin) {
            Err("Stale".to_string())
        } else {
            self.settle_push(&self.controller(), workspace, item, landed).await.map_err(|error| error.to_string())
        };
        Ok(serde_json::json!({ "schema": 1, "receipt": { "result": match answer {
            Ok(()) => serde_json::json!({ "Ok": null }), Err(why) => serde_json::json!({ "Err": why }) } } }))
    }
    /// Whether a Failed run may close as landed with `sha` (ov-1231): `None` to go on, or the sentence that refuses.
    /// The run must be the one named and Failed; the commit must be an ancestor of the target on the remote, read
    /// now. A remote that cannot be read is an error, never a "no", and a run in any other phase is left to the
    /// reducer to refuse, so no network read is spent on it.
    pub(super) async fn landed_check(&self, workspace: &str, item_id: &str, run: &str, pin: &str, sha: &str) -> Result<Option<String>> {
        let Some(state) = self.state(workspace)? else { return Ok(None) };
        let Some(item) = state.items.get(item_id).filter(|i| i.run == run && i.pin == pin && i.phase == Phase::Failed) else { return Ok(None) };
        if !sha_ok(sha) { return Ok(None); }
        let mut exec = self.exec.clone();
        if self.service_mode {
            let bundle = self.store.workflow_bundle(&item.policy_pin)?.ok_or(Error::Refused("missing item bundle"))?;
            exec.coding = bundle.adapter.map(|farcooler_protocol::workflow_config::AdapterConfiguration::Coding(config)| config);
            exec.base = exec.coding.as_ref().map_or_else(|| "0".repeat(40), |c| c.base.clone());
        }
        let identity = exec.landing()?;
        let log = exec.root.join("receipts").join(format!("{}.landed.fetch.output", digest(&(workspace, item_id, run, sha))));
        let on_target = exec.on_target(identity, &sha.to_ascii_lowercase(), &log).await?;
        #[cfg(test)]
        if let Some(change) = AFTER_LANDED_READ.lock().unwrap().remove(workspace) { change(); }
        Ok(if on_target { None } else { Some("The commit is not on the target branch on the remote.".to_string()) })
    }
    /// A person's word on an uncertain push, believed only after the remote is read: `landed` needs the
    /// pushed commit on the target and `not landed` needs it absent. The word is written as the push's own
    /// settlement, replacing whatever settlement the crash left; the next sweep resolves the item from it,
    /// so a landed push goes on and one that did not starts over from the base. Nothing is pushed.
    pub async fn settle_push(&self, controller: &Controller, workspace: &str, item_id: &str, landed: bool) -> Result<()> {
        self.authorize(controller.instance)?;
        let state = self.state(workspace)?.ok_or(Error::Refused("missing workspace"))?;
        let item = state.items.get(item_id).ok_or(Error::Refused("no such item"))?;
        if item.phase != Phase::Uncertain { return Err(Error::Refused("only an uncertain item can be settled")); }
        let pending = item.pending.as_ref().ok_or(Error::Refused("nothing is pending"))?;
        let stored = self.store.workflow_outbox(workspace)?.into_iter()
            .find(|s| &s.effect.id == pending && s.dispatch_state == "uncertain")
            .ok_or(Error::Refused("the effect is not uncertain"))?;
        if stored.effect.kind != EffectKind::Execute(StageKind::Push) { return Err(Error::Refused("only a push can be settled")); }
        self.refuse_active_probe(workspace, &stored.effect)?;
        let exec = self.settlement_exec(item)?;
        let base = exec.item_base(workspace, &stored.effect)?;
        let binding = Binding::new(workspace, &stored.effect,
            stored.resource_generation.ok_or(Error::Refused("the push was never admitted"))?, item.iteration, &base);
        exec.check_push(&binding, Some(landed)).await?;
        self.recovery.wake();
        if let Some(wake) = &self.notice_wake { wake.notify_one(); }
        Ok(())
    }
}

/// Test hook: what runs, once, the moment the remote read for a workspace's `landed` finishes, standing in for the
/// caller losing authority while that read was pending.
#[cfg(test)]
#[allow(clippy::type_complexity)] // Test-only one-shot callbacks, keyed by workspace to isolate parallel tests.
pub(super) static AFTER_LANDED_READ: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, Box<dyn FnOnce() + Send>>>> =
    std::sync::LazyLock::new(Default::default);

/// The runner's own look settling a Provision left Uncertain (ov-1220).
impl Runtime {
    /// The wire's Resolve, answered as Settle's is: no engine state is written here, so a resolve that went through
    /// and is sent again is refused because the step is no longer uncertain.
    pub(super) async fn resolve_wire(&self, workspace: &str, item: &str, run: &str, pin: &str, epoch: u64) -> Result<serde_json::Value> {
        let state = self.state(workspace)?.ok_or(Error::Refused("missing workspace"))?;
        let answer = if state.epoch != epoch || !state.items.get(item).is_some_and(|i| i.run == run && i.pin == pin) {
            Err("Stale".to_string())
        } else {
            self.resolve_provision(&self.controller(), workspace, item).await.map_err(|error| match error {
                Error::Domain(refused) => refused.to_string(),
                other => other.to_string(),
            })
        };
        Ok(serde_json::json!({ "schema": 1, "receipt": { "result": match answer {
            Ok(()) => serde_json::json!({ "Ok": null }), Err(why) => serde_json::json!({ "Err": why }) } } }))
    }
    /// Settle a Provision whose git never finished and left no receipt (a git killed at its timeout on a busy runner,
    /// before ov-1220 looked for itself), or the Stop of a cancelled run waiting on one. Believed only once the runner
    /// looks and finds nothing of the tree: no directory at its path, no worktree git lists there, and none of the
    /// branch it would have made. Then the Provision is recorded as having made nothing, and the next sweep fails the
    /// step with that (it can run again), or ends the cancelled run. Anything there is named for a person to clear first.
    pub async fn resolve_provision(&self, controller: &Controller, workspace: &str, item_id: &str) -> Result<()> {
        self.authorize(controller.instance)?;
        let refused = |sentence: String| Error::Domain(farcooler_core::DomainError::refused(sentence));
        let state = self.state(workspace)?.ok_or(Error::Refused("missing workspace"))?;
        let item = state.items.get(item_id).ok_or(Error::Refused("no such item"))?;
        if item.phase != Phase::Uncertain {
            return Err(refused(format!("{item_id} isn't uncertain, so there is nothing to resolve.")));
        }
        let pending = item.pending.as_ref().ok_or(Error::Refused("nothing is pending"))?;
        let outbox = self.store.workflow_outbox(workspace)?;
        let stored = outbox.iter().find(|s| &s.effect.id == pending).ok_or(Error::Refused("missing pending effect"))?;
        let provision = match &stored.effect.kind {
            EffectKind::Stop { operation } => outbox.iter().find(|s| &s.effect.id == operation).ok_or(Error::Refused("missing stopped effect"))?,
            _ => stored,
        };
        self.refuse_active_probe(workspace, &provision.effect)?;
        let build = provision.effect.kind == EffectKind::Execute(StageKind::Build);
        if provision.effect.kind != EffectKind::Execute(StageKind::Provision) && !build {
            return Err(refused(format!("Only a build, or a step that makes a run's worktree, can be resolved this way; \
                {item_id}'s uncertain step is neither.")));
        }
        let exec = self.settlement_exec(item)?;
        let base = exec.item_base(workspace, &provision.effect)?;
        let binding = Binding::new(workspace, &provision.effect,
            provision.resource_generation.ok_or(Error::Refused("the provision was never admitted"))?, item.iteration, &base);
        if build {
            exec.resolve_build(item_id, &binding).await?;
        } else {
            exec.check_provision(&binding, false).await?;
        }
        self.recovery.wake();
        if let Some(wake) = &self.notice_wake { wake.notify_one(); }
        Ok(())
    }

    fn refuse_active_probe(&self, workspace: &str, effect: &Effect) -> Result<()> {
        if self.activities.lock().unwrap().contains_key(&(workspace.to_string(), effect.id.clone())) {
            return Err(Error::Refused("the runner is checking this effect"));
        }
        Ok(())
    }

    /// The item's own configuration, even if the runtime last selected another repository.
    fn settlement_exec(&self, item: &farcooler_workflow::Item) -> Result<Exec> {
        let mut exec = self.exec.clone();
        if self.service_mode {
            let bundle = self.store.workflow_bundle(&item.policy_pin)?.ok_or(Error::Refused("missing item bundle"))?;
            exec.coding = bundle.adapter.map(|farcooler_protocol::workflow_config::AdapterConfiguration::Coding(config)| config);
            exec.base = exec.coding.as_ref().map_or_else(|| "0".repeat(40), |c| c.base.clone());
        }
        Ok(exec)
    }

}

impl Exec {
    /// The delayed probe runs only after receipt recovery has failed.
    pub(super) async fn auto_settle(&self, binding: &Binding) -> Result<Outcome> {
        match binding.effect.kind {
            EffectKind::Execute(StageKind::Push) => self.check_push(binding, None).await,
            EffectKind::Execute(StageKind::Provision) => self.check_provision(binding, true).await,
            EffectKind::Execute(StageKind::Build) => {
                // A usable process receipt still governs the result. A failed Git read is
                // no reason to discard that receipt and retry the build.
                if self.has_verified_process_receipt(binding) {
                    return Err(Error::Refused("the build ended, but its checkout or result could not be verified"));
                }
                self.check_build(&binding.effect.item, binding, "the runner checked it").await
            }
            _ => unreachable!("only supported uncertain steps are probed"),
        }
    }

    /// Read the pinned landing target before retaining any conclusion about a lost push.
    async fn check_push(&self, binding: &Binding, expected: Option<bool>) -> Result<Outcome> {
        let candidate = binding.effect.candidate.as_deref().filter(|c| sha(c)).ok_or(Error::Refused("missing push candidate"))?;
        let identity = self.landing()?;
        let log = self.root.join("receipts").join(format!("{}.settle.fetch.output", digest(&binding.effect)));
        let landed = self.on_target(identity, candidate, &log).await?;
        if expected.is_some_and(|claimed| claimed != landed) {
            return Err(Error::Refused("the remote disagrees"));
        }
        // After a restart, receipt recovery reads the target again before accepting a pass.
        self.settle_push_fact(binding, landed, if expected.is_some() { "person" } else { "runner" })
    }

    async fn check_provision(&self, binding: &Binding, automatic: bool) -> Result<Outcome> {
        let item_id = &binding.effect.item;
        let refused = |sentence: String| Error::Domain(farcooler_core::DomainError::refused(sentence));
        if self.git_settlement(binding).try_exists()? {
            return Err(refused(format!("{item_id}'s worktree receipt exists, but its result could not be verified.")));
        }
        let path = self.tree(binding);
        let made = match &binding.effect.tree {
            Some(tree) => (!tree.adopted).then(|| tree.branch.clone()),
            None => Some(adapters::private_branch(&binding.workspace, &binding.effect)),
        };
        // Under the repository's lock, as Provision makes a named tree, so nothing is made while it looks. Taken only
        // if free: this runs inside the runtime's loop, which is what polls a Provision holding the lock, so waiting
        // for it here would wait for good and stop every command and effect with it.
        let lock = if let Some(host) = &self.host {
            let board = Uuid::parse_str(&binding.workspace).map_err(|_| Error::Refused("invalid workspace"))?;
            let board = self.store.get_workspace(board).map_err(Error::Domain)?;
            Some(host.lock(board.repository_id))
        } else { None };
        let _held = match &lock {
            Some(lock) => Some(lock.try_lock().map_err(|_| refused(format!("{item_id} can't be resolved while the runner is making \
                another run's worktree in this repository. Resolve it again in a moment.")))?),
            None => None,
        };
        let _quiescent = match self.lock_provision(binding) {
            Ok(file) => Some(file),
            // A controller can still resolve older provisions after checking
            // them by hand. The runner has no process evidence for these.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && !automatic => None,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound =>
                return Err(Error::Refused("no provisioning process lock was recorded, so Git may still be running")),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock =>
                return Err(Error::Refused("the provisioning Git process may still be running")),
            Err(error) => return Err(error.into()),
        };
        if automatic && path.symlink_metadata().is_ok_and(|meta| meta.is_dir()) && self.provision_intact(binding, &path).await? {
            self.record_git_settlement(binding)?;
            return self.reconcile_coding(binding).await;
        }
        if let Some(there) = named_tree::what_is_there(&self.repository(), &path, made.as_deref()).await {
            return Err(refused(format!("{item_id} can't be resolved yet: {there}. Take that away by hand if the run made it, then resolve again.")));
        }
        let checked = if automatic { "the runner checked" } else { "the orchestrator resolved it" };
        self.refuse_unstarted(binding, format!("Git never finished making the run's worktree, and nothing was made; {checked}. Retry the run."))
    }

    /// Believed only once the runner looks: a process fact (written only after the process ended), or a native
    /// agent whose pane is gone with nothing on its terminal, or one that was never let out. Then the build is
    /// recorded as having ended with no result, and the next sweep fails the step with that (it runs again in the
    /// same worktree if it has attempts left), or ends the cancelled run waiting on it. Nothing is killed.
    pub(super) async fn resolve_build(&self, item_id: &str, binding: &Binding) -> Result<()> {
        self.check_build(item_id, binding, "the orchestrator resolved it").await?;
        Ok(())
    }

    async fn check_build(&self, item_id: &str, binding: &Binding, checked: &str) -> Result<Outcome> {
        let refused = |sentence: String| Error::Domain(farcooler_core::DomainError::refused(sentence));
        if self.unstarted_recorded(binding)? {
            return Err(refused(format!("{item_id}'s build is already resolved; the next look settles it.")));
        }
        let ended = if self.process_fact_recorded(binding)? {
                self.ended_process(binding)?;
                None
            }
            else if self.is_native_agent(binding) { self.native_ended(binding).await? }
            // A command build keeps no owner record: only one that never wrote its log provably never ran.
            else if self.process_log(binding).try_exists()? { Some("its command may still be running, and the runner can't prove it ended") }
            else { None };
        if let Some(running) = ended {
            return Err(refused(format!("{item_id} can't be resolved yet: {running}. Cancel it, or wait for it to end, then resolve again.")));
        }
        self.refuse_unstarted(binding, format!("The build ended with no result the runner could read, and nothing of it is still \
            running; {checked}. Any work in the worktree carries over to the next build attempt."))
    }
}

#[cfg(test)]
#[path = "coding_resolve_build_tests.rs"]
mod resolve_build_tests;
