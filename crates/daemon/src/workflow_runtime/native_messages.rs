//! Claude's next tool hook delivers direction before the tool runs. Stop remains the continuation
//! boundary for Codex and for Claude between tools; its final queue check also closes admission.
use super::*;
use farcooler_agent_hooks::wire::{HookLine, TAKES_FOLLOWUP};

/// Any exit (timeout, cancellation, failed capture, or normal settlement) closes admission and
/// records undelivered direction on its original ticket. The durable queue remains recoverable if
/// the process dies or a store write fails; the ordinary wake pump handles an unlisted recipient.
pub(super) struct Inbox<'a> { pub(super) store: &'a Store, pub(super) id: Uuid }
impl Drop for Inbox<'_> {
    fn drop(&mut self) {
        let result = crate::engine_steering::inbox(self.id, |open| {
            *open = false;
            for wake in self.store.pending_message_wakes()? {
                if wake.to != Some(self.id) { continue; }
                let record = format!("Not delivered to the engine agent: {}. The step ended; send this direction again to the current agent.", wake.body);
                self.store.finish_wake(&wake, Some(&record))?;
            }
            Ok(())
        });
        if let Err(error) = result { tracing::warn!(%error, terminal = %self.id, "couldn't settle native messages; the durable queue retains them"); }
    }
}

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
        stream: &mut tokio::net::UnixStream, closing: bool) -> Result<bool> {
        use farcooler_store::models::Actor;
        let tool = line.agent == farcooler_agent_hooks::Agent::Claude && line.event == "PreToolUse"
            && line.takes_updated_input();
        let stop = line.event == "Stop" && line.takes.iter().any(|t| t == TAKES_FOLLOWUP);
        if !(tool || stop) || line.payload.get("agent_id").is_some() { return Ok(false); }
        let id = super::coding_native::terminal(binding);
        let workspace = Uuid::parse_str(&binding.workspace).ok();
        if !workspace.and_then(|w| self.store.get_workspace(w).ok()).is_some_and(|w| w.wake_on_answer) {
            return Ok(false);
        }
        let claimed = crate::engine_steering::inbox(id, |open| {
            let mut claimed = Vec::new();
            for wake in self.store.pending_message_wakes()? {
                if wake.to != Some(id) || wake.claimed_at.is_some() { continue; }
                let task = self.store.get_task(wake.task)?;
                if Some(task.workspace_id) != workspace { continue; }
                if self.store.claim_wake(&wake)? { claimed.push((wake, task.key)); }
            }
            if closing && stop && claimed.is_empty() { *open = false; }
            Ok(claimed)
        }).map_err(Error::Domain)?;
        if claimed.is_empty() { return Ok(false); }
        let text = claimed.iter().map(|(w, key)| {
            let from = match w.actor { Actor::Manager => "the orchestrator", Actor::User => "the owner", _ => "a collaborator" };
            format!("[from {from} on {key}] {}", w.body)
        }).collect::<Vec<_>>().join("\n");
        use tokio::io::AsyncWriteExt;
        // Deny this tool, handing the direction to Claude before it can make the old change.
        // A plain allow would bypass a permission check, and a Stop continuation would arrive late.
        let response = if tool { serde_json::json!({"decision":{"behavior":"deny", "message":text}}) }
            else { serde_json::json!({"followup_message":text}) };
        let reply = serde_json::to_vec(&response)?;
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
            let (send, _orders) = mpsc::channel(8);
            let _listed = crate::engine_steering::publish(&binding.workspace, &binding.effect.item, id, agent.as_str(), None,
                false, vec![task.key.clone()], send);
            let note = svc.store.send_manual_message(task.id, Actor::Manager, "Use the new API", Some(id), serde_json::json!({})).unwrap();
            let foreign = svc.store.send_manual_message(task.id, Actor::Manager, "Another step", Some(Uuid::now_v7()), serde_json::json!({})).unwrap();
            let (mut writer, reader) = tokio::net::UnixStream::pair().unwrap();
            let mut line = HookLine { agent, event:"Stop".into(), payload:serde_json::json!({}), takes:vec![], launch:None };
            assert!(!runtime.native_followup(&binding, &line, &mut writer, true).await.unwrap(), "an old hook cannot read a response");
            line.takes.push(TAKES_FOLLOWUP.into());
            assert!(runtime.native_followup(&binding, &line, &mut writer, true).await.unwrap());
            let mut text = String::new();
            tokio::io::BufReader::new(reader).read_line(&mut text).await.unwrap();
            let response:serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_eq!(response["followup_message"], format!("[from the orchestrator on {}] Use the new API", task.key));
            assert!(!runtime.native_followup(&binding, &line, &mut writer, true).await.unwrap(), "never sent twice");
            let pending = svc.store.pending_message_wakes().unwrap();
            assert!(!pending.iter().any(|w| w.note == note.id));
            let other = pending.iter().find(|w| w.note == foreign.id).unwrap();
            svc.store.finish_wake(other, None).unwrap();
        }
    }
    #[tokio::test]
    async fn native_direction_reaches_claude_before_its_next_tool_runs() {
        let (_dir, mut runtime) = super::super::coding_tests::configured(false, "pass").await;
        let definition = runtime.configured_definition().unwrap();
        super::super::tests::enqueue(&mut runtime, "w", "one", definition);
        runtime.pump(2).await.unwrap();
        let mut binding = super::super::coding_tests::admitted(&runtime).await;
        let (_board_dir, svc, repo) = crate::test_support::fixture().await;
        let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
        let task = svc.store.create_task(workspace, "Steer active work", Actor::User).unwrap();
        binding.workspace = workspace.to_string(); runtime.store = svc.store.clone();
        let id = super::super::coding_native::terminal(&binding);
        let (send, _orders) = mpsc::channel(8);
        let _listed = crate::engine_steering::publish(&binding.workspace, &binding.effect.item, id, "claude", None,
            false, vec![task.key.clone()], send);
        svc.store.send_manual_message(task.id, Actor::Manager, "Use the new API", Some(id), serde_json::json!({})).unwrap();
        let (mut writer, reader) = tokio::net::UnixStream::pair().unwrap();
        let line = HookLine { agent:Agent::Claude, event:"PreToolUse".into(), payload:serde_json::json!({}),
            takes:vec![farcooler_agent_hooks::wire::TAKES_UPDATED_INPUT.into()], launch:None };
        assert!(runtime.native_followup(&binding, &line, &mut writer, true).await.unwrap(), "direction must arrive before the tool call, without a Stop");
        let mut text = String::new();
        tokio::io::BufReader::new(reader).read_line(&mut text).await.unwrap();
        let response:serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(response["decision"]["behavior"], "deny");
        assert!(response["decision"]["message"].as_str().unwrap().contains("Use the new API"));
        assert!(svc.store.pending_message_wakes().unwrap().is_empty());
    }

    #[tokio::test]
    async fn native_final_stop_rejects_messages_before_capture_and_settlement_for_both_harnesses() {
        for agent in [Agent::Claude, Agent::Codex] {
            let (_dir, mut runtime) = super::super::coding_tests::configured(false, "pass").await;
            let definition = runtime.configured_definition().unwrap();
            super::super::tests::enqueue(&mut runtime, "w", "one", definition);
            runtime.pump(2).await.unwrap();
            let mut binding = super::super::coding_tests::admitted(&runtime).await;
            let (_board_dir, svc, repo) = crate::test_support::fixture().await;
            let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
            let task = svc.store.create_task(workspace, "Final handoff", Actor::User).unwrap();
            binding.workspace = workspace.to_string(); binding.effect.item = task.key.clone();
            runtime.store = svc.store.clone();
            let id = super::super::coding_native::terminal(&binding);
            let (send, _orders) = mpsc::channel(8);
            let _listed = crate::engine_steering::publish(&binding.workspace, &task.key, id, agent.as_str(), None,
                false, vec![task.key.clone()], send);
            let watcher = crate::watch::Watcher::new(svc.clone());
            let request = || farcooler_protocol::v1::Request { method:"message.send".into(),
                payload:Some(farcooler_protocol::v1::request::Payload::MessageSend(farcooler_protocol::v1::MessageSend {
                    workspace_id:crate::wire::id_bytes(workspace), to:task.key.clone(), actor:"manager".into(),
                    text:"Direction at handoff".into(), ..Default::default() })), ..Default::default() };
            let (mut writer, reader) = tokio::net::UnixStream::pair().unwrap();
            let line = HookLine { agent, event:"Stop".into(), payload:serde_json::json!({}),
                takes:vec![TAKES_FOLLOWUP.into()], launch:None };
            // A message admitted before the final check must extend this exact session.
            watcher.message_send(request(), crate::people::Person::Owner).await.unwrap();
            assert!(runtime.native_followup(&binding, &line, &mut writer, true).await.unwrap());
            let mut text = String::new();
            tokio::io::BufReader::new(reader).read_line(&mut text).await.unwrap();
            assert!(text.contains("Direction at handoff"));
            // Hold the live row past the last Stop, as capture/quiescence do in production.
            assert!(!runtime.native_followup(&binding, &line, &mut writer, true).await.unwrap());
            assert!(watcher.message_send(request(), crate::people::Person::Owner).await.is_err(),
                "{agent:?}: a final Stop must close admission before settlement starts");
            assert!(svc.store.pending_message_wakes().unwrap().is_empty());
        }
    }

    #[tokio::test]
    async fn native_exit_records_undelivered_direction_and_closes_stale_admission_for_both_harnesses() {
        for agent in ["claude", "codex"] {
            let (_dir, svc, repo) = crate::test_support::fixture().await;
            let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
            let task = svc.store.create_task(workspace, "Interrupted direction", Actor::User).unwrap();
            let id = Uuid::now_v7();
            let (send, _orders) = mpsc::channel(8);
            let listed = crate::engine_steering::publish(&workspace.to_string(), &task.key, id, agent, None,
                false, vec![task.key.clone()], send);
            let append = || svc.store.send_manual_message(task.id, Actor::Manager, "Use the new API", Some(id), serde_json::json!({}));
            let inbox = Inbox { store:&svc.store, id };
            crate::engine_steering::queue_message(id, append).unwrap();
            drop(inbox); // The same guard runs on timeout, stop, cancellation, and failed capture.
            assert!(svc.store.pending_message_wakes().unwrap().is_empty());
            let notes = svc.store.notes_for(task.id, None).unwrap();
            assert!(notes.iter().any(|n| n.body.contains("Not delivered") && n.body.contains("Use the new API")
                && n.body.contains("send this direction again")), "{agent}: {notes:?}");
            assert!(crate::engine_steering::queue_message(id, append).is_err());
            drop(listed);
            assert!(crate::engine_steering::queue_message(id, append).is_err(), "a resolved recipient cannot outlive its capability");
        }
    }

}
