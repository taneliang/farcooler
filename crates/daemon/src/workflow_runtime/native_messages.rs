//! Codex and Claude tool hooks deliver direction before the tool runs. The final queue
//! check at Stop or after Claude redraws its idle prompt also closes admission.
use super::*;
use farcooler_agent_hooks::wire::{HookLine, TAKES_FOLLOWUP};
use farcooler_core::activity::Registry;
use farcooler_protocol::v1::AgentActivity;
use farcooler_store::{PendingWake, models::Actor};
use tokio::io::AsyncWriteExt;

#[derive(Default)]
pub(super) struct ClaudeStop(Option<Duration>);

impl ClaudeStop {
    pub(super) async fn idle(&mut self, tmux: &farcooler_tmux::TmuxServer, pane: &str, now: Duration, heard: Duration) -> bool {
        static ACTIVITY: std::sync::LazyLock<Registry> = std::sync::LazyLock::new(Registry::built_in);
        if !tmux.capture_screen(pane).await.is_ok_and(|screen| ACTIVITY.classify("claude", &screen) == AgentActivity::Idle) {
            self.0 = None;
            return false;
        }
        // Let Claude redraw after Stop and start a new turn when a background shell exits.
        now.saturating_sub((*self.0.get_or_insert(now)).max(heard)) >= Duration::from_millis(500)
    }
}

pub(super) enum IdleDirection {
    Empty,
    Waiting,
    Submitted,
}

enum QueueRead {
    Peek,
    Claim,
}

pub(super) enum Followup {
    NoMessage,
    Sent(Vec<u8>),
    Disconnected,
}

/// A hook can give up waiting for its reply while the agent itself is still working.
pub(super) async fn send_native_reply(binding: &Binding, line: &HookLine,
    stream: &mut tokio::net::UnixStream, reply: &[u8]) -> Result<bool> {
    let sent = async { stream.write_all(reply).await?; stream.write_all(b"\n").await }.await;
    match sent {
        Ok(()) => Ok(true),
        Err(error) if matches!(error.kind(), std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset) => {
            tracing::warn!(effect = %binding.effect.id, hook = %line.event, %error, "native hook disconnected before receiving its reply");
            Ok(false)
        }
        Err(error) => Err(error.into()),
    }
}

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
        stream: &mut tokio::net::UnixStream, closing: bool) -> Result<Followup> {
        let tool = matches!(line.agent, farcooler_agent_hooks::Agent::Claude | farcooler_agent_hooks::Agent::Codex) && line.event == "PreToolUse"
            && line.takes_updated_input();
        let stop = line.event == "Stop" && line.takes.iter().any(|t| t == TAKES_FOLLOWUP);
        if !(tool || stop) || line.payload.get("agent_id").is_some() { return Ok(Followup::NoMessage); }
        let claimed = self.native_direction(binding, QueueRead::Claim, closing && stop)?;
        if claimed.is_empty() { return Ok(Followup::NoMessage); }
        let text = direction_text(&claimed);
        // Deny this tool, handing the direction to the agent before it can make the old change.
        // A plain allow would bypass a permission check, and a Stop continuation would arrive late.
        // Informational runner notices (ov-1171) provide context; explicit direction still fences the tool.
        let context = tool && line.agent == farcooler_agent_hooks::Agent::Claude
            && line.takes.iter().any(|t| t == farcooler_agent_hooks::wire::TAKES_CONTEXT)
            && claimed.iter().all(|(wake, _)| wake.actor == Actor::Runner);
        let response = if context { serde_json::json!({"additional_context":text}) }
            else if tool { serde_json::json!({"decision":{"behavior":"deny", "message":text}}) }
            else { serde_json::json!({"followup_message":text}) };
        let reply = serde_json::to_vec(&response)?;
        let sent = send_native_reply(binding, line, stream, &reply).await;
        let record = (!matches!(sent, Ok(true))).then_some("Couldn't confirm the engine agent got the message");
        for (wake, _) in claimed {
            self.store.finish_wake(&wake, record).map_err(Error::Domain)?;
        }
        let sent = sent?;
        if !sent && closing && stop {
            // This Stop had no continuation. Close admission before normal settlement.
            let id = super::coding_native::terminal(binding);
            crate::engine_steering::inbox(id, |open| { *open = false; Ok(()) }).map_err(Error::Domain)?;
        }
        Ok(if sent { Followup::Sent(reply) } else { Followup::Disconnected })
    }

    /// Once Claude has redrawn after Stop, that hook has returned. Continue through
    /// the same private pane, leaving the message queued if a safe prompt is unavailable.
    pub(super) async fn idle_claude_direction(&self, binding: &Binding, tmux: &farcooler_tmux::TmuxServer,
        pane: &str, tty: &str, closing: bool) -> Result<IdleDirection> {
        if self.native_direction(binding, QueueRead::Peek, closing)?.is_empty() { return Ok(IdleDirection::Empty); }
        let id = super::coding_native::terminal(binding);
        let Ok(Some(prompt)) = crate::watch::answer_wake::engine::IdlePrompt::claude(tmux, pane, id, tty).await
            else { return Ok(IdleDirection::Waiting) };
        let claimed = self.native_direction(binding, QueueRead::Claim, false)?;
        if claimed.is_empty() { return Ok(IdleDirection::Waiting); }
        let sent = matches!(prompt.submit(&direction_text(&claimed)).await, Ok(true));
        for (wake, _) in claimed {
            self.store.finish_wake(&wake, (!sent).then_some("Couldn't confirm the engine agent got the message")).map_err(Error::Domain)?;
        }
        if sent { Ok(IdleDirection::Submitted) }
        else { Err(Error::Refused("couldn't confirm Claude received direction after its background work")) }
    }

    fn native_direction(&self, binding: &Binding, read: QueueRead, closing: bool) -> Result<Vec<(PendingWake, String)>> {
        let id = super::coding_native::terminal(binding);
        let workspace = Uuid::parse_str(&binding.workspace).ok();
        // Serialize the empty queue check and admission closure with message.send.
        crate::engine_steering::inbox(id, |open| {
            let mut queued = Vec::new();
            for wake in self.store.pending_message_wakes()? {
                if wake.to != Some(id) || wake.claimed_at.is_some() { continue; }
                let task = self.store.get_task(wake.task)?;
                if Some(task.workspace_id) == workspace && (matches!(read, QueueRead::Peek) || self.store.claim_wake(&wake)?) {
                    queued.push((wake, task.key));
                }
            }
            if closing && queued.is_empty() { *open = false; }
            Ok(queued)
        }).map_err(Error::Domain)
    }
}

fn direction_text(queued: &[(PendingWake, String)]) -> String {
    queued.iter().map(|(wake, key)| {
        let from = match wake.actor { Actor::Manager => "the orchestrator", Actor::User => "the owner", _ => "a collaborator" };
        format!("[from {from} on {key}] {}", wake.body)
    }).collect::<Vec<_>>().join("\n")
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
            assert!(matches!(runtime.native_followup(&binding, &line, &mut writer, true).await.unwrap(), Followup::NoMessage), "an old hook cannot read a response");
            line.takes.push(TAKES_FOLLOWUP.into());
            assert!(matches!(runtime.native_followup(&binding, &line, &mut writer, true).await.unwrap(), Followup::Sent(_)));
            let mut text = String::new();
            tokio::io::BufReader::new(reader).read_line(&mut text).await.unwrap();
            let response:serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_eq!(response["followup_message"], format!("[from the orchestrator on {}] Use the new API", task.key));
            assert!(matches!(runtime.native_followup(&binding, &line, &mut writer, true).await.unwrap(), Followup::NoMessage), "never sent twice");
            let pending = svc.store.pending_message_wakes().unwrap();
            assert!(!pending.iter().any(|w| w.note == note.id));
            let other = pending.iter().find(|w| w.note == foreign.id).unwrap();
            svc.store.finish_wake(other, None).unwrap();
        }
    }
    #[tokio::test]
    async fn native_direction_reaches_both_harnesses_before_their_next_tool_runs() {
        for (agent, wake_on_answer) in [Agent::Claude, Agent::Codex].into_iter()
            .flat_map(|agent| [true, false].map(|wake| (agent, wake))) {
            let (_dir, mut runtime) = super::super::coding_tests::configured(false, "pass").await;
            let definition = runtime.configured_definition().unwrap();
            super::super::tests::enqueue(&mut runtime, "w", "one", definition);
            runtime.pump(2).await.unwrap();
            let mut binding = super::super::coding_tests::admitted(&runtime).await;
            let (_board_dir, svc, repo) = crate::test_support::fixture().await;
            let board = svc.store.ensure_main_workspace(repo).unwrap();
            let board = svc.store.set_workspace_wake_on_answer(board.id, board.resource_version, wake_on_answer).unwrap();
            let workspace = board.id;
            let task = svc.store.create_task(workspace, "Steer active work", Actor::User).unwrap();
            binding.workspace = workspace.to_string(); runtime.store = svc.store.clone();
            let id = super::super::coding_native::terminal(&binding);
            let (send, _orders) = mpsc::channel(8);
            let _listed = crate::engine_steering::publish(&binding.workspace, &binding.effect.item, id, agent.as_str(), None,
                false, vec![task.key.clone()], send);
            let watcher = crate::watch::Watcher::new(svc.clone());
            let reply = watcher.message_send(farcooler_protocol::v1::Request { method:"message.send".into(),
                payload:Some(farcooler_protocol::v1::request::Payload::MessageSend(farcooler_protocol::v1::MessageSend {
                    workspace_id:crate::wire::id_bytes(workspace), to:task.key.clone(), actor:"user".into(),
                    text:"Use the new API".into(), ..Default::default() })), ..Default::default() },
                crate::people::Person::Owner).await.unwrap();
            assert!(matches!(reply, farcooler_protocol::v1::result::Value::MessageSent(_)));
            let pending = svc.store.pending_message_wakes().unwrap();
            assert_eq!(pending.len(), 1);
            assert_eq!(pending[0].to, Some(id));
            let (mut writer, reader) = tokio::net::UnixStream::pair().unwrap();
            let line = HookLine { agent, event:"PreToolUse".into(), payload:serde_json::json!({}),
                takes:vec![farcooler_agent_hooks::wire::TAKES_UPDATED_INPUT.into()], launch:None };
            let mut foreign = line.clone(); foreign.payload["agent_id"] = "subagent".into();
            assert!(matches!(runtime.native_followup(&binding, &foreign, &mut writer, true).await.unwrap(), Followup::NoMessage));
            let mut old = line.clone(); old.takes.clear();
            assert!(matches!(runtime.native_followup(&binding, &old, &mut writer, true).await.unwrap(), Followup::NoMessage));
            assert!(matches!(runtime.native_followup(&binding, &line, &mut writer, true).await.unwrap(), Followup::Sent(_)), "{agent:?}, wake_on_answer={wake_on_answer}: direction must arrive before the tool call, without a Stop");
            let mut text = String::new();
            tokio::io::BufReader::new(reader).read_line(&mut text).await.unwrap();
            let response:serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_eq!(response["decision"]["behavior"], "deny");
            assert!(response["decision"]["message"].as_str().unwrap().contains("Use the new API"));
            assert!(svc.store.pending_message_wakes().unwrap().is_empty());
            assert!(matches!(runtime.native_followup(&binding, &line, &mut writer, false).await.unwrap(), Followup::NoMessage),
                "direction is consumed once");
        }
    }

    #[tokio::test]
    async fn native_final_stop_rejects_messages_before_capture_and_settlement_for_both_harnesses() {
        for (agent, wake_on_answer) in [Agent::Claude, Agent::Codex].into_iter()
            .flat_map(|agent| [true, false].map(|wake| (agent, wake))) {
            let (_dir, mut runtime) = super::super::coding_tests::configured(false, "pass").await;
            let definition = runtime.configured_definition().unwrap();
            super::super::tests::enqueue(&mut runtime, "w", "one", definition);
            runtime.pump(2).await.unwrap();
            let mut binding = super::super::coding_tests::admitted(&runtime).await;
            let (_board_dir, svc, repo) = crate::test_support::fixture().await;
            let board = svc.store.ensure_main_workspace(repo).unwrap();
            let board = svc.store.set_workspace_wake_on_answer(board.id, board.resource_version, wake_on_answer).unwrap();
            let workspace = board.id;
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
            assert!(matches!(runtime.native_followup(&binding, &line, &mut writer, true).await.unwrap(), Followup::Sent(_)));
            let mut text = String::new();
            tokio::io::BufReader::new(reader).read_line(&mut text).await.unwrap();
            assert!(text.contains("Direction at handoff"));
            // Hold the live row past the last Stop, as capture/quiescence do in production.
            assert!(matches!(runtime.native_followup(&binding, &line, &mut writer, true).await.unwrap(), Followup::NoMessage));
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
