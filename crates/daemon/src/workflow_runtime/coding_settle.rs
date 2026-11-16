//! A person's word on a push whose outcome the runtime lost (ov-956).
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
        let candidate = stored.effect.candidate.clone().filter(|c| sha(c)).ok_or(Error::Refused("missing push candidate"))?;
        // The item's own configuration: the runtime serves many repositories and holds the last one it launched.
        let mut exec = self.exec.clone();
        if self.service_mode {
            let bundle = self.store.workflow_bundle(&item.policy_pin)?.ok_or(Error::Refused("missing item bundle"))?;
            exec.coding = bundle.adapter.map(|farcooler_protocol::workflow_config::AdapterConfiguration::Coding(config)| config);
            exec.base = exec.coding.as_ref().map_or_else(|| "0".repeat(40), |c| c.base.clone());
        }
        let identity = exec.landing()?;
        // Read the remote first; a claim it contradicts is refused, and a remote that cannot be read settles nothing.
        let log = exec.root.join("receipts").join(format!("{}.settle.fetch.output", digest(&stored.effect)));
        if exec.on_target(identity, &candidate, &log).await? != landed {
            return Err(Error::Refused("the remote disagrees"));
        }
        let base = exec.item_base(workspace, &stored.effect)?;
        let binding = Binding::new(workspace, &stored.effect,
            stored.resource_generation.ok_or(Error::Refused("the push was never admitted"))?, item.iteration, &base);
        exec.settle_fact(&binding, if landed { "passed" } else { "base_moved" }, &candidate,
            serde_json::json!({ "settled_by": "person", "landed": landed }))?;
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
        let build = provision.effect.kind == EffectKind::Execute(StageKind::Build);
        if provision.effect.kind != EffectKind::Execute(StageKind::Provision) && !build {
            return Err(refused(format!("Only a build, or a step that makes a run's worktree, can be resolved this way; \
                {item_id}'s uncertain step is neither.")));
        }
        let mut exec = self.exec.clone();
        if self.service_mode {
            let bundle = self.store.workflow_bundle(&item.policy_pin)?.ok_or(Error::Refused("missing item bundle"))?;
            exec.coding = bundle.adapter.map(|farcooler_protocol::workflow_config::AdapterConfiguration::Coding(config)| config);
            exec.base = exec.coding.as_ref().map_or_else(|| "0".repeat(40), |c| c.base.clone());
        }
        let base = exec.item_base(workspace, &provision.effect)?;
        let binding = Binding::new(workspace, &provision.effect,
            provision.resource_generation.ok_or(Error::Refused("the provision was never admitted"))?, item.iteration, &base);
        if build {
            exec.resolve_build(item_id, &binding).await?;
            // The settlement just written is read by the next sweep, not after the effect's backoff (ov-1301).
            self.recovery.wake();
            if let Some(wake) = &self.notice_wake { wake.notify_one(); }
            return Ok(());
        }
        if exec.git_settlement(&binding).try_exists()? {
            return Err(refused(format!("{item_id}'s worktree step finished and left its receipt, which settles it; nothing to resolve.")));
        }
        let (path, made) = match &provision.effect.tree {
            Some(tree) => (PathBuf::from(&tree.path), (!tree.adopted).then(|| tree.branch.clone())),
            None => (exec.tree(&binding), Some(format!("wf-{}", &digest(&(workspace, &provision.effect.item))[4..]))),
        };
        // Under the repository's lock, as Provision makes a named tree, so nothing is made while it looks. Taken only
        // if free: this runs inside the runtime's loop, which is what polls a Provision holding the lock, so waiting
        // for it here would wait for good and stop every command and effect with it.
        let lock = match (&exec.host, Uuid::parse_str(workspace).ok().and_then(|w| self.store.get_workspace(w).ok())) {
            (Some(host), Some(board)) => Some(host.lock(board.repository_id)),
            _ => None,
        };
        let _held = match &lock {
            Some(lock) => Some(lock.try_lock().map_err(|_| refused(format!("{item_id} can't be resolved while the runner is making \
                another run's worktree in this repository. Resolve it again in a moment.")))?),
            None => None,
        };
        if let Some(there) = named_tree::what_is_there(&exec.repository(), &path, made.as_deref()).await {
            return Err(refused(format!("{item_id} can't be resolved yet: {there}. Take that away by hand if the run made it, then resolve again.")));
        }
        exec.refuse_unstarted(&binding, "Git never finished making the run's worktree, and nothing was made; the orchestrator \
            resolved it. Retry the run.".into())?;
        self.recovery.wake();
        if let Some(wake) = &self.notice_wake { wake.notify_one(); }
        Ok(())
    }
}

/// The orchestrator settling a build left Uncertain whose process has ended (ov-1301), as it settles a lost
/// Provision: a daemon that restarted or lost the build's receipt held it there, and only an owner's word cleared it.
impl Exec {
    /// Believed only once the runner looks: a process fact (written only after the process ended), or a native
    /// agent whose pane is gone with nothing on its terminal, or one that was never let out. Then the build is
    /// recorded as having ended with no result, and the next sweep fails the step with that (it runs again in the
    /// same worktree if it has attempts left), or ends the cancelled run waiting on it. Nothing is killed.
    pub(super) async fn resolve_build(&self, item_id: &str, binding: &Binding) -> Result<()> {
        let refused = |sentence: String| Error::Domain(farcooler_core::DomainError::refused(sentence));
        if self.unstarted_recorded(binding)? {
            return Err(refused(format!("{item_id}'s build is already resolved; the next look settles it.")));
        }
        let ended = if self.process_fact_recorded(binding)? { None }
            else if self.is_native_agent(binding) { self.native_ended(binding).await? }
            // A command build keeps no owner record: only one that never wrote its log provably never ran.
            else if self.process_log(binding).try_exists()? { Some("its command may still be running, and the runner can't prove it ended") }
            else { None };
        if let Some(running) = ended {
            return Err(refused(format!("{item_id} can't be resolved yet: {running}. Cancel it, or wait for it to end, then resolve again.")));
        }
        self.refuse_unstarted(binding, "The build ended with no result the runner could read, and nothing of it is still \
            running; the orchestrator resolved it. Its work stays in the worktree for the step's next attempt, a retry or \
            a send-back.".into())?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "coding_resolve_build_tests.rs"]
mod resolve_build_tests;
