//! A person's word on a push whose outcome the runtime lost (ov-956).
use super::*;
use super::coding::sha;
use super::coding_verbs::on_remote;

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
        if on_remote(&identity.repository, &identity.target, &candidate).await? != landed {
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
