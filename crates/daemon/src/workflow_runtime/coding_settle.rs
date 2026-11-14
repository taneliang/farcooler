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
pub(super) static AFTER_LANDED_READ: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, Box<dyn FnOnce() + Send>>>> =
    std::sync::LazyLock::new(Default::default);
