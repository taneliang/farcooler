//! The engine's Stop hook is a handoff boundary: queued steering starts another turn in this same
//! session instead of allowing the engine to settle the step and lose the direction.
use super::*;
use farcooler_agent_hooks::wire::{HookLine, TAKES_FOLLOWUP};

impl Exec {
    pub(super) fn steering_tickets(&self, binding: &Binding) -> Result<Vec<String>> {
        let mut keys: BTreeSet<String> = if binding.effect.members.is_empty() {
            std::iter::once(binding.effect.item.clone()).chain(binding.effect.with.iter().cloned()).collect()
        } else { binding.effect.members.iter().map(|m| m.item.clone()).collect() };
        // Group gates also work on the followers of each grouped ticket.
        if let Some(state) = self.store.workflow_state(&binding.workspace)? {
            for key in keys.clone() {
                if let Some(item) = state.items.get(&key) { keys.extend(item.followers.iter().cloned()); }
            }
        }
        Ok(keys.into_iter().collect())
    }
    pub(super) async fn native_followup(&self, binding: &Binding, line: &HookLine,
        stream: &mut tokio::net::UnixStream) -> Result<bool> {
        use farcooler_store::models::Actor;
        if line.event != "Stop" || !line.takes.iter().any(|t| t == TAKES_FOLLOWUP)
            || (line.agent == farcooler_agent_hooks::Agent::Claude && line.payload.get("agent_id").is_some())
        { return Ok(false); }
        let id = super::coding_native::terminal(binding);
        let workspace = Uuid::parse_str(&binding.workspace).ok();
        if !workspace.and_then(|w| self.store.get_workspace(w).ok()).is_some_and(|w| w.wake_on_answer) {
            return Ok(false);
        }
        let waiting = self.store.pending_message_wakes().map_err(Error::Domain)?;
        let mut claimed = Vec::new();
        for wake in waiting {
            if wake.to != Some(id) || wake.claimed_at.is_some() { continue; }
            let task = self.store.get_task(wake.task).map_err(Error::Domain)?;
            if Some(task.workspace_id) != workspace { continue; }
            if self.store.claim_wake(&wake).map_err(Error::Domain)? { claimed.push((wake, task.key)); }
        }
        if claimed.is_empty() { return Ok(false); }
        let text = claimed.iter().map(|(w, key)| {
            let from = match w.actor { Actor::Manager => "the orchestrator", Actor::User => "the owner", _ => "a collaborator" };
            format!("[from {from} on {key}] {}", w.body)
        }).collect::<Vec<_>>().join("\n");
        use tokio::io::AsyncWriteExt;
        let reply = serde_json::to_vec(&serde_json::json!({"followup_message":text}))?;
        let sent = async { stream.write_all(&reply).await?; stream.write_all(b"\n").await }.await;
        for (wake, _) in claimed {
            let record = sent.as_ref().err().map(|_| "Couldn't confirm the engine agent got the message");
            self.store.finish_wake(&wake, record).map_err(Error::Domain)?;
        }
        sent?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_agent_hooks::Agent;
    use farcooler_store::models::Actor;
    use tokio::io::AsyncBufReadExt;

    #[tokio::test]
    async fn native_steering_is_consumed_once_by_the_exact_steps_waiting_stop() {
        let (_dir, mut runtime) = super::super::coding_tests::configured(false, "pass").await;
        let definition = runtime.configured_definition().unwrap();
        super::super::tests::enqueue(&mut runtime, "w", "one", definition);
        runtime.pump(2).await.unwrap();
        let mut binding = super::super::coding_tests::admitted(&runtime).await;
        let (_board_dir, svc, repo) = crate::test_support::fixture().await;
        let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
        let task = svc.store.create_task(workspace, "Steer me", Actor::User).unwrap();
        binding.workspace = workspace.to_string(); binding.effect.item = "group-gate".into();
        runtime.store = svc.store.clone();
        let id = super::super::coding_native::terminal(&binding);
        for agent in [Agent::Claude, Agent::Codex] {
            let note = svc.store.send_manual_message(task.id, Actor::Manager, "Use the new API", Some(id), serde_json::json!({})).unwrap();
            let foreign = svc.store.send_manual_message(task.id, Actor::Manager, "Another step", Some(Uuid::now_v7()), serde_json::json!({})).unwrap();
            let (mut writer, reader) = tokio::net::UnixStream::pair().unwrap();
            let mut line = HookLine { agent, event:"Stop".into(), payload:serde_json::json!({}), takes:vec![], launch:None };
            assert!(!runtime.native_followup(&binding, &line, &mut writer).await.unwrap(), "an old hook cannot read a response");
            line.takes.push(TAKES_FOLLOWUP.into());
            assert!(runtime.native_followup(&binding, &line, &mut writer).await.unwrap());
            let mut text = String::new();
            tokio::io::BufReader::new(reader).read_line(&mut text).await.unwrap();
            let response:serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_eq!(response["followup_message"], format!("[from the orchestrator on {}] Use the new API", task.key));
            assert!(!runtime.native_followup(&binding, &line, &mut writer).await.unwrap(), "never sent twice");
            let pending = svc.store.pending_message_wakes().unwrap();
            assert!(!pending.iter().any(|w| w.note == note.id));
            let other = pending.iter().find(|w| w.note == foreign.id).unwrap();
            svc.store.finish_wake(other, None).unwrap();
        }
    }
}
