//! Messages to engine PTYs use the retained message queue, bound to the particular run's terminal id
//! (ov-1171). The watch stream stays read-only. A later build of the same card never inherits a message.
use std::time::Duration;

use farcooler_core::{activity::Registry, composer};
use farcooler_protocol::v1::AgentActivity;
use farcooler_store::{PendingWake, WakeKind, models::{Actor, Task}};

use super::{Held, Pass, Watcher, codex_turn, foreground, now_millis};

impl Watcher {
    /// Only an actual acceptance revision on a currently running engine card warrants a notice.
    /// The short notice makes the agent re-read the card, so even a long replacement isn't truncated.
    pub(crate) fn engine_acceptance_changed(&self, task: &Task) {
        let Some((_, pane)) = crate::watch_view::lookup(&task.workspace_id.to_string(), &task.key) else { return };
        let body = format!("Acceptance changed on {}. Read `farcooler task show {}` again and work to its current acceptance items.", task.key, task.key);
        let extra = serde_json::json!({"message": {"to": format!("{}'s engine agent", task.key)}});
        match self.service.store.send_message(task.id, Actor::Runner, &body, Some(pane.terminal_id), extra) {
            Ok(_) => {
                self.announce_task_changed(task, None, Actor::Runner);
                self.message_queued();
            }
            Err(farcooler_core::DomainError::Conflict { what: "typing_off" }) => {}
            Err(error) => tracing::warn!(task = %task.key, %error, "couldn't queue acceptance change for engine agent"),
        }
    }

    /// The ordinary pump owns claims and ordering. Nothing is retried after a claim, including an
    /// uncertain queue handoff or a paste left behind. The pane must still belong to the addressed run.
    pub(super) async fn tell_engine(&self, wake: &PendingWake, task: &Task) -> Option<Pass> {
        if wake.kind != WakeKind::Message { return None; }
        let (runtime, pane) = crate::watch_view::lookup(&task.workspace_id.to_string(), &task.key)?;
        if wake.to != Some(pane.terminal_id) { return None; }
        let waiting = || Some(Pass::Waiting(Held::Busy));
        let Some((preset, pid)) = foreground(&pane.tty).await else { return waiting() };
        if runtime.tmux.pane_life(&pane.pane_id, pane.terminal_id).await.ok()
            != Some(farcooler_tmux::PaneLife::Live) { return waiting(); }
        let text = self.message_text(wake, task);
        // Codex's native queue reaches a working turn without an Enter that might answer a dialog.
        // Discover the live process's own thread and home, never a card's old transcript.
        if preset == "codex" {
            let Some(rollout) = codex_turn::rollout_now(pid).await else { return waiting() };
            let Some((thread, home)) = crate::log_join::codex_thread_of(&rollout) else { return waiting() };
            let Some(program) = queue_program(pane.terminal_id) else { return waiting() };
            match self.service.store.claim_wake(wake) {
                Ok(true) => {},
                Ok(false) => return Some(Pass::Settled),
                Err(_) => return waiting(),
            }
            let from = std::fs::metadata(&rollout).map_or(0, |m| m.len());
            let at = now_millis();
            let mut command = tokio::process::Command::new(program);
            command.args(["queue", "--thread", &thread, &format!("--message={text}")])
                .env("CODEX_HOME", home).stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).kill_on_drop(true);
            let ran = tokio::time::timeout(Duration::from_secs(10), command.status()).await;
            let handed_off = matches!(ran, Ok(Ok(status)) if status.success());
            if handed_off {
                self.watch_queued(pane.terminal_id, wake.note, &text,
                    super::queued::Queued { pid, rollout, from, at }, Some(task));
            }
            let why = (!handed_off).then(|| "Couldn't confirm the message reached its engine agent".to_string());
            return Some(self.settle(wake, Some(task), why));
        }
        let rules = Registry::built_in();
        let Ok(screen) = runtime.tmux.capture_screen(&pane.pane_id).await else { return waiting() };
        // Other agents wait for an empty box between turns. Never paste into a shell, dialog or draft.
        if !between(preset, pid).await || rules.classify(preset, &screen) != AgentActivity::Idle
            || composer::read(preset, &screen) != composer::Composer::Empty
            || runtime.tmux.pane_bracketed_paste(&pane.pane_id).await.ok() != Some(Some(true)) {
            return waiting();
        }
        match self.service.store.claim_wake(wake) {
            Ok(true) => {},
            Ok(false) => return Some(Pass::Settled),
            Err(_) => return waiting(),
        }
        let pasted = runtime.tmux.send_literal(&pane.pane_id, &format!("\x1b[200~{text}\x1b[201~")).await;
        let expected = if preset == "cursor" {
            composer::drawn::Expected::default().then_cursor_paste(&text)
        } else {
            composer::drawn::Expected::default().then_paste(&text)
        };
        if pasted.is_ok() {
            for _ in 0..50 {
                tokio::time::sleep(Duration::from_millis(20)).await;
                let Ok(screen) = runtime.tmux.capture_screen(&pane.pane_id).await else { break };
                if expected.shown_by(&composer::read(preset, &screen)) {
                    if between(preset, pid).await && rules.classify(preset, &screen) == AgentActivity::Idle
                        && foreground(&pane.tty).await == Some((preset, pid))
                        && runtime.tmux.send_bytes_hex(&pane.pane_id, "0d").await.is_ok() {
                        return Some(self.settle(wake, Some(task), None));
                    }
                    break;
                }
            }
        }
        Some(self.settle(wake, Some(task), Some("Paste left in the engine agent's composer; not sent".into())))
    }
}

// Claude's spinner can disappear before its turn does. Its live session registry wins over the screen.
async fn between(preset: &str, pid: i32) -> bool {
    if preset != "claude" { return true; }
    let Some(config) = super::registry_turn::config_of(pid).await else { return false };
    super::registry_turn::said(&config, pid) == super::registry_turn::Said::Idle
}

#[cfg(not(test))]
fn queue_program(_terminal: uuid::Uuid) -> Option<std::path::PathBuf> {
    farcooler_core::programs::find("codex")
}
#[cfg(test)]
static QUEUES: std::sync::Mutex<Vec<(uuid::Uuid, std::path::PathBuf)>> = std::sync::Mutex::new(Vec::new());
#[cfg(test)]
fn queue_program(terminal: uuid::Uuid) -> Option<std::path::PathBuf> {
    QUEUES.lock().unwrap().iter().find(|(id, _)| *id == terminal).map(|(_, p)| p.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_protocol::v1::{self as pb, request, result};
    use farcooler_tmux::TmuxServer;
    use uuid::Uuid;

    struct Scratch(TmuxServer);
    impl Drop for Scratch {
        fn drop(&mut self) { farcooler_tmux::reap_server(self.0.socket()); }
    }

    /// A real, private engine-style server, not a terminal registered in Service. The scratch
    /// agent draws Claude's composer and records submitted text, without starting a paid agent.
    async fn scratch(dir: &std::path::Path, agent: &str) -> (Scratch, farcooler_core::inventory::TaggedPane) {
        let program = dir.join(agent);
        std::fs::copy(farcooler_core::programs::find("perl").unwrap(), &program).unwrap();
        std::fs::write(dir.join("agent.pl"), include_str!("stand_in.pl")).unwrap();
        let mode = if agent == "codex" { "working" } else { "idle" };
        std::fs::write(dir.join("control"), mode).unwrap();
        let rollout = dir.join(".codex/sessions/2026/11/14/rollout-engine.jsonl");
        std::fs::create_dir_all(rollout.parent().unwrap()).unwrap();
        std::fs::write(&rollout, "{\"type\":\"session_meta\",\"payload\":{\"id\":\"01a117d0-fd5c-7fe3-8f0f-5f2afdf7906d\",\"source\":\"cli\"}}\n").unwrap();
        let tmux = TmuxServer::in_dir(&format!("engine-message-{}", Uuid::now_v7()), Uuid::now_v7(), dir);
        std::fs::create_dir_all(dir.join("config")).unwrap();
        let command = format!("exec env STAND_IN_HOLD={} CLAUDE_CONFIG_DIR={} {} {} {agent} {} {}",
            crate::service::shell_quote(&rollout.to_string_lossy()),
            crate::service::shell_quote(&dir.join("config").to_string_lossy()),
            crate::service::shell_quote(&program.to_string_lossy()),
            crate::service::shell_quote(&dir.join("agent.pl").to_string_lossy()),
            crate::service::shell_quote(&dir.join("control").to_string_lossy()),
            crate::service::shell_quote(&dir.join("log").to_string_lossy()));
        let terminal = Uuid::now_v7();
        tmux.create_terminal_window(terminal, terminal, "scratch engine", &dir.to_string_lossy(), &command).await.unwrap();
        let pane = tmux.list_tagged_panes().await.unwrap().remove(0);
        for _ in 0..300 {
            if std::fs::read_to_string(dir.join("log")).unwrap_or_default().contains(&format!("MODE {mode}")) {
                return (Scratch(tmux), pane);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("scratch agent never drew its box");
    }

    async fn send(watcher: &Watcher, task: &Task, text: &str) -> pb::MessageSent {
        let answer = watcher.message_send(pb::Request {
            method: "message.send".into(),
            payload: Some(request::Payload::MessageSend(pb::MessageSend {
                to: task.key.clone(), text: text.into(), actor: "manager".into(),
                workspace_id: task.workspace_id.as_bytes().to_vec().into(), ..Default::default()
            })), ..Default::default()
        }, crate::people::Person::Owner).await.unwrap();
        let result::Value::MessageSent(sent) = answer else { panic!("not a sent message") };
        sent
    }

    async fn delivered(watcher: &Watcher, dir: &std::path::Path, text: &str) {
        for _ in 0..300 {
            watcher.pump_wakes().await;
            if std::fs::read_to_string(dir.join("log")).unwrap_or_default()
                .lines().any(|l| l.starts_with("SUBMIT ") && l.contains(text)) { return; }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("message never submitted: {}", std::fs::read_to_string(dir.join("log")).unwrap_or_default());
    }

    #[tokio::test]
    async fn engine_message_reaches_a_scratch_agent_and_acceptance_revision_notifies_it() {
        let (dir, svc, repo) = crate::test_support::fixture().await;
        let ws = svc.store.ensure_main_workspace(repo).unwrap().id;
        let task = svc.store.create_task(ws, "engine message", Actor::Manager).unwrap();
        let watcher = Watcher::new(svc.clone());
        let (server, pane) = scratch(dir.path(), "claude").await;
        assert!(svc.store.get_terminal(pane.terminal_id).is_err(), "engine PTY isn't a legacy terminal");
        let listed = crate::watch_view::publish(&ws.to_string(), &task.key, server.0.clone(), pane.clone());
        send(&watcher, &task, "Rebase before finishing").await;
        delivered(&watcher, dir.path(), "[from the orchestrator] Rebase before finishing").await;
        let update = pb::TaskUpdate {
            task_id: task.id.as_bytes().to_vec().into(), expected_version: task.resource_version,
            title: task.title.clone(), intent: task.intent.clone(), actor: "manager".into(),
            acceptance: vec![pb::TaskAcceptanceItem { text: "New requirement".into(), ..Default::default() }],
            ..Default::default()
        };
        let revised = crate::task_ops::update(&svc, &watcher, &update).unwrap();
        delivered(&watcher, dir.path(), "[Far Cooler] Acceptance changed on").await;
        assert!(svc.store.pending_message_wakes().unwrap().is_empty());
        let before = svc.store.notes_for(task.id, None).unwrap().len();
        let unchanged = pb::TaskUpdate { expected_version: revised.resource_version,
            acceptance: revised.acceptance.clone(), ..update.clone() };
        crate::task_ops::update(&svc, &watcher, &unchanged).unwrap();
        assert_eq!(svc.store.notes_for(task.id, None).unwrap().len(), before, "no notice for unchanged acceptance");
        assert!(crate::task_ops::update(&svc, &watcher, &update).is_err(), "stale write fails");
        assert_eq!(svc.store.notes_for(task.id, None).unwrap().len(), before, "no notice for rejected update");
        // A queued message cannot jump to a replacement run of this ticket.
        drop(listed);
        let note = svc.store.send_manual_message(task.id, Actor::Manager, "old run only", Some(pane.terminal_id), serde_json::json!({})).unwrap();
        let replacement = farcooler_core::inventory::TaggedPane { terminal_id: Uuid::now_v7(), ..pane };
        let _next = crate::watch_view::publish(&ws.to_string(), &task.key, server.0.clone(), replacement);
        watcher.pump_wakes().await;
        assert!(!svc.store.pending_message_wakes().unwrap().iter().any(|w| w.note == note.id));
        assert!(!std::fs::read_to_string(dir.path().join("log")).unwrap().contains("old run only"));
    }

    #[tokio::test]
    async fn engine_message_queues_to_a_working_codex_without_touching_its_pane() {
        use std::os::unix::fs::PermissionsExt;
        for status in [0, 1] {
            let (dir, svc, repo) = crate::test_support::fixture().await;
            let ws = svc.store.ensure_main_workspace(repo).unwrap().id;
            let task = svc.store.create_task(ws, "busy engine", Actor::Manager).unwrap();
            let watcher = Watcher::new(svc.clone());
            let (server, pane) = scratch(dir.path(), "codex").await;
            let _listed = crate::watch_view::publish(&ws.to_string(), &task.key, server.0.clone(), pane.clone());
            let program = dir.path().join("queue.sh");
            let log = dir.path().join("queue.log");
            std::fs::write(&program, format!("#!/bin/sh\nprintf '%s\\n' \"$CODEX_HOME|$*\" >> '{}'\nexit {status}\n", log.display())).unwrap();
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
            QUEUES.lock().unwrap().push((pane.terminal_id, program));
            send(&watcher, &task, "The acceptance changed while you worked").await;
            for _ in 0..300 {
                watcher.pump_wakes().await;
                if svc.store.pending_message_wakes().unwrap().is_empty() { break; }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            assert!(svc.store.pending_message_wakes().unwrap().is_empty());
            let ran = std::fs::read_to_string(&log).unwrap();
            assert_eq!(ran.lines().count(), 1, "an uncertain handoff is never retried");
            assert!(ran.contains("queue --thread 01a117d0-fd5c-7fe3-8f0f-5f2afdf7906d --message=[from the orchestrator] The acceptance changed while you worked"), "{ran}");
            assert!(ran.starts_with(&dir.path().canonicalize().unwrap().join(".codex").to_string_lossy().to_string()));
            let screen_log = std::fs::read_to_string(dir.path().join("log")).unwrap();
            assert!(!screen_log.contains("PASTE") && !screen_log.contains("ENTER"), "{screen_log}");
            let notes = svc.store.notes_for(task.id, None).unwrap();
            assert_eq!(notes.iter().any(|n| n.body.contains("Couldn't confirm")), status != 0);
            watcher.pump_wakes().await;
            assert_eq!(std::fs::read_to_string(log).unwrap(), ran);
            QUEUES.lock().unwrap().retain(|(id, _)| *id != pane.terminal_id);
        }
    }


    #[tokio::test]
    async fn engine_message_waits_behind_a_draft_and_does_not_answer_a_dialog() {
        for mode in ["draft:keep my draft", "menu-on-paste"] {
            let (dir, svc, repo) = crate::test_support::fixture().await;
            let ws = svc.store.ensure_main_workspace(repo).unwrap().id;
            let task = svc.store.create_task(ws, "guarded engine", Actor::Manager).unwrap();
            let watcher = Watcher::new(svc.clone());
            let (server, pane) = scratch(dir.path(), "claude").await;
            let _listed = crate::watch_view::publish(&ws.to_string(), &task.key, server.0.clone(), pane);
            std::fs::write(dir.path().join("control"), mode).unwrap();
            for _ in 0..300 {
                if std::fs::read_to_string(dir.path().join("log")).unwrap().contains(&format!("MODE {mode}")) { break; }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            send(&watcher, &task, "A guarded message").await;
            for _ in 0..20 {
                watcher.pump_wakes().await;
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            let log = std::fs::read_to_string(dir.path().join("log")).unwrap();
            assert!(!log.contains("ENTER") && !log.contains("ANSWERED"), "{log}");
            if mode.starts_with("draft:") {
                assert!(!log.contains("PASTE"), "{log}");
                assert_eq!(svc.store.pending_message_wakes().unwrap().len(), 1);
                // Becoming idle doesn't erase a person's draft. Simulate the person clearing it first.
                std::fs::write(dir.path().join("control"), "working-cleared").unwrap();
                for _ in 0..300 {
                    if std::fs::read_to_string(dir.path().join("log")).unwrap().contains("MODE working-cleared") { break; }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                std::fs::write(dir.path().join("control"), "idle").unwrap();
                delivered(&watcher, dir.path(), "A guarded message").await;
            } else {
                assert!(log.contains("PASTE"), "the dialog appeared after the paste: {log}");
                assert!(svc.store.pending_message_wakes().unwrap().is_empty());
            }
        }
    }

}
