//! Answering a decision wakes the agent waiting on it.
//!
//! Every answer, from every client, is an ANSWER note written through
//! `task.note` (`task_ops::note`): `farcooler task note --kind answer`, the
//! Mac's board and Needs You, both phones' Needs You and task screens. That
//! one handler calls `Watcher::answered`, which queues the answer in the
//! store (`answer_wakes`, keyed by the note) when the task's workspace has
//! the switch on. `pump_wakes` then tells it, right away and again on every
//! sampling tick until it's told:
//!
//! - **Whom.** A running agent terminal in the task's workspace, preferring
//!   one opened for the task (`Terminal.task_id`), newest first, then one in
//!   the task's worktree. Else the workspace's running orchestrator. Never a
//!   terminal in another workspace, a shell, a Changes pane, or whoever wrote
//!   the answer. Nobody: the task gets "Nobody to tell about the decision".
//! - **When.** Only while the watcher reads that terminal as Idle or Done.
//!   Working, Blocked or not yet read: it waits, and the next tick tries
//!   again, which is what makes it land at the next turn to idle. And never
//!   while someone is typing there: input marked in the last
//!   `QUIET_MS` (`QUIET_WATCHED_MS` while a client has the pane in front of
//!   a person) holds it back until the typing stops.
//! - **How.** A chat pane gets it as a prompt on its agent channel; a TUI
//!   gets it typed, then Enter, through `terminal send`'s path.
//! - **What.** One line: `Decision on ov-79 ("Drill-in layout"): Drill in.
//!   Continue.` The answer is the person's own words, flattened to one line,
//!   cut to `LONGEST_ANSWER`, with every control character written out as
//!   text so nothing in it acts on the terminal.
//!
//! Once told, the queue row is marked done and "Told <terminal> about the
//! decision" goes on the task as the runner, in one transaction. A crash
//! between the typing and that write tells it again after the restart;
//! every other restart tells it exactly once.

use std::sync::atomic::Ordering;
use std::time::Duration;

use farcooler_agent::link::DaemonMessage;
use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::AgentActivity;
use farcooler_store::PendingWake;
use farcooler_store::models::{Actor, PaneMode, Task, TaskNote, Terminal, TerminalRole};

use super::{Watcher, anyone_watching, now_millis};
use crate::runtime::{Runtime, last_input};

/// How long after the last keystroke a pane counts as quiet.
pub(crate) const QUIET_MS: i64 = 5_000;
/// The same, while a client says the pane is in front of a person
/// (`terminal.watching`): someone looking at it may only be pausing.
pub(crate) const QUIET_WATCHED_MS: i64 = 15_000;
/// Long enough for any answer picked from options and most written ones;
/// short enough to stay one line in the agent's context.
const LONGEST_ANSWER: usize = 280;
const LONGEST_TITLE: usize = 80;
/// Between the text and its Enter. Sent together, a TUI that reads a burst
/// of input as a paste takes the Enter as part of it and never submits.
const ENTER_AFTER: Duration = Duration::from_millis(200);

/// What the agent is told.
pub(crate) fn message(key: &str, title: &str, answer: &str) -> String {
    let answer = one_line(answer, LONGEST_ANSWER);
    let title = one_line(title, LONGEST_TITLE);
    let stop = if answer.ends_with(['.', '!', '?', '…']) { "" } else { "." };
    format!("Decision on {key} (\"{title}\"): {answer}{stop} Continue.")
}

/// `raw` on one line, safe to type: line breaks and tabs become spaces, runs
/// of space become one, and every other control character (C0, DEL and C1,
/// which is where ESC and CSI live) is written out as `\u{1b}` rather than
/// sent. Cut to `longest` characters, the last of them `…`.
pub(crate) fn one_line(raw: &str, longest: usize) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        match c {
            '\n' | '\r' | '\t' => out.push(' '),
            c if c.is_control() => out.extend(c.escape_unicode()),
            c => out.push(c),
        }
    }
    let out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.chars().count() <= longest {
        return out;
    }
    let cut: String = out.chars().take(longest.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

/// What a wake came to on one pass.
#[derive(Debug, PartialEq, Eq)]
enum Pass {
    /// Told, or settled as nobody's to tell: done for good.
    Settled,
    /// Not yet: the terminal is busy, someone is typing, or the send failed.
    Waiting,
}

impl Watcher {
    /// An answer was written: queue it, if its workspace wakes on answers,
    /// and try it now. Only a person's or the orchestrator's answer wakes
    /// anyone; an agent answering its own question has nothing to learn.
    pub fn answered(&self, task: &Task, note: &TaskNote) {
        if !matches!(note.actor, Actor::User | Actor::Manager) {
            return;
        }
        let on = self.service.store.get_workspace(task.workspace_id).is_ok_and(|w| w.wake_on_answer);
        if !on {
            return;
        }
        match self.service.store.enqueue_answer_wake(note.id, task.id) {
            Ok(_) => {
                self.wakes_hint.store(true, Ordering::SeqCst);
                self.spawn_wake_pump();
            }
            Err(e) => tracing::warn!(task = %task.id, error = %e, "couldn't queue an answer to tell"),
        }
    }

    /// `pump_wakes` off the caller's path, when there is a runtime.
    pub(super) fn spawn_wake_pump(&self) {
        if !self.wakes_hint.load(Ordering::SeqCst) {
            return;
        }
        let Some(me) = self.me.upgrade() else { return };
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move { me.pump_wakes().await });
        }
    }

    /// Tell every queued answer whose terminal is ready. One pass at a time:
    /// a second caller while one runs returns at once, so two passes can't
    /// both type the same answer.
    pub async fn pump_wakes(&self) {
        let Ok(_one_pass) = self.wake_pump.try_lock() else { return };
        // Cleared before the read, so an answer queued during this pass sets
        // it again and the next tick reads it.
        self.wakes_hint.store(false, Ordering::SeqCst);
        let pending = match self.service.store.pending_answer_wakes() {
            Ok(pending) => pending,
            Err(e) => {
                tracing::warn!(error = %e, "couldn't read the answers waiting to be told");
                self.wakes_hint.store(true, Ordering::SeqCst);
                return;
            }
        };
        for wake in pending {
            if self.wake(&wake).await == Pass::Waiting {
                self.wakes_hint.store(true, Ordering::SeqCst);
            }
        }
    }

    async fn wake(&self, wake: &PendingWake) -> Pass {
        let store = &self.service.store;
        let task = match store.get_task(wake.task) {
            Ok(task) => task,
            Err(DomainError::NotFound) => return self.settle(wake, None, None),
            Err(_) => return Pass::Waiting,
        };
        // Turned off since it was queued: let it go, saying nothing.
        if !store.get_workspace(task.workspace_id).is_ok_and(|w| w.wake_on_answer) {
            return self.settle(wake, None, None);
        }
        let Some(to) = self.recipient(&task, wake.actor).await else {
            // The orchestrator answered and no agent is there: it knows.
            if wake.actor == Actor::Manager {
                return self.settle(wake, None, None);
            }
            return self.settle(wake, Some(&task), Some("Nobody to tell about the decision".into()));
        };
        let (activity, _, _) = self.activity(to.id).await;
        if !matches!(activity, AgentActivity::Idle | AgentActivity::Done) {
            return Pass::Waiting;
        }
        if self.typed_lately(to.id, now_millis()) {
            return Pass::Waiting;
        }
        let text = message(&task.key, &task.title, &wake.body);
        if let Err(e) = self.tell(&to, &text).await {
            tracing::warn!(terminal = %to.id, error = %e, "couldn't tell an agent about a decision; trying again");
            return Pass::Waiting;
        }
        self.settle(wake, Some(&task), Some(format!("Told {} about the decision", spoken_name(&to))))
    }

    /// Mark `wake` done, with the note that says how, and announce the task
    /// so its feed shows the note.
    fn settle(&self, wake: &PendingWake, task: Option<&Task>, record: Option<String>) -> Pass {
        match self.service.store.finish_answer_wake(wake.note, record.as_deref()) {
            Ok(Some(Some(_))) => {
                if let Some(task) = task {
                    self.announce_task_changed(task, None, Actor::Runner);
                }
                Pass::Settled
            }
            Ok(_) => Pass::Settled,
            Err(e) => {
                tracing::warn!(note = %wake.note, error = %e, "couldn't record an answer as told");
                Pass::Waiting
            }
        }
    }

    /// Whom to tell about an answer on `task` written by `writer`. See this
    /// module's docs.
    async fn recipient(&self, task: &Task, writer: Actor) -> Option<Terminal> {
        let store = &self.service.store;
        let mut candidates = store.terminals_for_task(task.id).ok()?;
        candidates.reverse();
        if let Some(lane) = task.worktree_id {
            let mut lane = store.list_terminals_for_worktree(lane).ok()?;
            lane.sort_by_key(|t| std::cmp::Reverse(t.id));
            for t in lane {
                if !candidates.iter().any(|c| c.id == t.id) {
                    candidates.push(t);
                }
            }
        }
        for t in candidates {
            let mine = t.workspace_id == Some(task.workspace_id);
            let an_agent = t.role == TerminalRole::Agent && t.pane_mode != PaneMode::Changes && t.command_preset != "shell";
            if !mine || !an_agent || writer == (Actor::Agent { terminal: t.id }) || !self.service.is_running(&t) {
                continue;
            }
            // A pane the watcher reads as a plain shell has no agent in it
            // any more, and the answer typed there would run as a command.
            if self.activity(t.id).await.0 == AgentActivity::None {
                continue;
            }
            return Some(t);
        }
        if writer == Actor::Manager {
            return None;
        }
        self.service
            .live_orchestrator(task.workspace_id)
            .ok()
            .flatten()
            .filter(|t| t.workspace_id == Some(task.workspace_id) && self.service.is_running(t))
    }

    /// Whether someone typed into `terminal` too recently to type over.
    fn typed_lately(&self, terminal: uuid::Uuid, now: i64) -> bool {
        let watched = {
            let watched = self.watched.lock().unwrap_or_else(|e| e.into_inner());
            anyone_watching(&watched, terminal, now)
        };
        let quiet = if watched { QUIET_WATCHED_MS } else { QUIET_MS };
        last_input(self.service.root_dir(), terminal).is_some_and(|at| now - at < quiet)
    }

    /// Put `text` in front of the agent in `to`.
    async fn tell(&self, to: &Terminal, text: &str) -> Result<()> {
        if to.pane_mode == PaneMode::Agent {
            let prompt = DaemonMessage::Prompt { text: text.to_string(), images: Vec::new() };
            return if self.service.agents().send(to.id, prompt) {
                Ok(())
            } else {
                Err(DomainError::AgentNotConnected)
            };
        }
        // Unmarked: the runner typing isn't someone typing.
        let runtime = Runtime { marks: None, ..self.service.runtime() };
        runtime.send_input(to.id, text).await?;
        tokio::time::sleep(ENTER_AFTER).await;
        runtime.send_bytes_hex(to.id, "0d").await
    }
}

/// How the record names a terminal: the orchestrator as such, an agent by
/// its title.
fn spoken_name(t: &Terminal) -> String {
    if t.role == TerminalRole::Orchestrator {
        return "the orchestrator".into();
    }
    let title = one_line(&t.title, LONGEST_TITLE);
    if title.is_empty() { "the agent".into() } else { title }
}

#[cfg(test)]
mod tests {
    //! On a real tmux server with the stub agent (`agent_stub`): a `sleep`
    //! whose terminal echoes what's typed, so what was told is on the
    //! screen. The watcher's sampling loop doesn't run here, so each test
    //! says what the agent is doing (`observe_for_tests`) and pumps.

    use std::sync::Arc;

    use farcooler_protocol::v1 as pb;
    use farcooler_store::models::{NoteKind, Worktree};
    use uuid::Uuid;

    use super::*;
    use crate::needs_you::Observation;
    use crate::service::Service;
    use crate::test_support::ScratchDir;

    struct Board {
        dir: ScratchDir,
        svc: Arc<Service>,
        watcher: Arc<Watcher>,
        lane: Worktree,
        task: Task,
    }

    async fn board() -> Board {
        let (dir, svc, repo) = crate::test_support::fixture().await;
        crate::reconcile::repository(&svc, repo).await.unwrap();
        let lane = svc.store.list_worktrees_for_repository(repo).unwrap().remove(0);
        let main = svc.store.ensure_main_workspace(repo).unwrap().id;
        let task = svc.store.create_task(main, "Drill-in layout", Actor::Manager).unwrap();
        let watcher = Watcher::new(svc.clone());
        Board { dir, svc, watcher, lane, task }
    }

    impl Board {
        async fn agent(&self, title: &str) -> Terminal {
            self.svc
                .create_terminal_with_prompt(self.lane.id, title, "claude", None, Some(self.task.id))
                .await
                .expect("an agent pane")
        }

        async fn orchestrator(&self) -> Terminal {
            self.svc.start_orchestrator(self.task.workspace_id, "claude", false, None).await.expect("an orchestrator")
        }

        async fn doing(&self, terminal: Uuid, activity: AgentActivity) {
            let seen = Observation {
                activity,
                state_since: now_millis(),
                blocked_question: None,
                turn_failed: false,
                command: "claude".into(),
                chat_capable: false,
            };
            self.watcher.observe_for_tests(terminal, seen).await;
        }

        /// Answer as the person at a client, through `task.note` itself.
        fn answer(&self, body: &str) {
            let req = pb::TaskNoteAppend {
                task_id: self.task.id.as_bytes().to_vec().into(),
                kind: pb::TaskNoteKind::Answer as i32,
                body: body.into(),
                ..Default::default()
            };
            crate::task_ops::note(&self.svc, &self.watcher, &req).expect("answered");
        }

        /// Pump until nothing is mid-pass, the way the tick would.
        async fn pump(&self) {
            for _ in 0..50 {
                // The pass `answered` spawned may hold the lock: wait it out.
                if self.watcher.wake_pump.try_lock().is_ok() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            self.watcher.wakes_hint.store(true, Ordering::SeqCst);
            self.watcher.pump_wakes().await;
        }

        async fn screen(&self, terminal: Uuid) -> String {
            self.svc.screen(terminal).await.expect("a screen").0
        }

        /// The screen once `told` is on it, or as it is after two seconds.
        async fn screen_with(&self, terminal: Uuid, told: &str) -> String {
            for _ in 0..40 {
                let screen = self.screen(terminal).await;
                if screen.contains(told) {
                    return screen;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            self.screen(terminal).await
        }

        fn progress(&self) -> Vec<TaskNote> {
            self.svc.store.notes_for(self.task.id, Some(NoteKind::Progress)).unwrap()
        }

        fn told(&self) -> String {
            message(&self.task.key, "Drill-in layout", "Drill in")
        }
    }

    #[test]
    fn the_message_is_one_line_with_the_answer_and_no_control_characters() {
        assert_eq!(message("ov-79", "Drill-in layout", "Drill in"), r#"Decision on ov-79 ("Drill-in layout"): Drill in. Continue."#);
        assert_eq!(message("ov-79", "Layout", "  Yes!  "), r#"Decision on ov-79 ("Layout"): Yes! Continue."#);
        let said = message("ov-1", "T\x1b]0;x\x07", "Red\x1b[31m now\r\nplease\u{9b}2J\x7f");
        assert!(!said.chars().any(char::is_control), "{said:?}");
        assert!(said.contains(r"Red\u{1b}[31m now please\u{9b}2J\u{7f}"), "{said}");
        assert!(said.contains(r#"("T\u{1b}]0;x\u{7}")"#), "{said}");
        let long = message("ov-1", "T", &"word ".repeat(200));
        assert!(long.chars().count() < LONGEST_ANSWER + 60, "{long}");
        assert!(long.contains("…"), "{long}");
    }

    /// An idle agent working the task is told at once, and the task says so.
    #[tokio::test]
    async fn an_answer_is_typed_into_the_idle_agent() {
        let b = board().await;
        let agent = b.agent("Agent 2").await;
        b.doing(agent.id, AgentActivity::Idle).await;
        b.answer("Drill in");
        b.pump().await;

        let screen = b.screen_with(agent.id, &b.told()).await;
        assert!(screen.contains(&b.told()), "{screen}");
        let progress = b.progress();
        assert_eq!(progress.len(), 1, "{progress:?}");
        assert_eq!((progress[0].actor, progress[0].body.as_str()), (Actor::Runner, "Told Agent 2 about the decision"));
        assert!(b.svc.store.pending_answer_wakes().unwrap().is_empty());
    }

    /// While the agent works the answer waits, and lands when it goes idle.
    #[tokio::test]
    async fn an_answer_waits_for_a_working_agent_to_go_idle() {
        let b = board().await;
        let agent = b.agent("Agent 2").await;
        b.doing(agent.id, AgentActivity::Working).await;
        b.answer("Drill in");
        b.pump().await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!b.screen(agent.id).await.contains("Decision on"), "typed over a working agent");
        assert_eq!(b.svc.store.pending_answer_wakes().unwrap().len(), 1);
        assert!(b.progress().is_empty());

        b.doing(agent.id, AgentActivity::Done).await;
        b.pump().await;
        let screen = b.screen_with(agent.id, &b.told()).await;
        assert!(screen.contains(&b.told()), "{screen}");
        assert_eq!(b.progress().len(), 1);
    }

    /// Someone typing holds it back: a mark in the last five seconds, or the
    /// last fifteen while the pane is on someone's screen.
    #[tokio::test]
    async fn nothing_is_typed_while_someone_is_typing() {
        let b = board().await;
        let agent = b.agent("Agent 2").await;
        b.doing(agent.id, AgentActivity::Idle).await;
        let mark = crate::runtime::input_mark(b.svc.root_dir(), agent.id);
        std::fs::create_dir_all(mark.parent().unwrap()).unwrap();
        std::fs::write(&mark, now_millis().to_string()).unwrap();
        b.answer("Drill in");
        b.pump().await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!b.screen(agent.id).await.contains("Decision on"), "typed over a typist");

        // Eight seconds quiet, but the pane is in front of somebody.
        std::fs::write(&mark, (now_millis() - 8_000).to_string()).unwrap();
        b.watcher.report_watching("-", vec![agent.id]);
        b.pump().await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!b.screen(agent.id).await.contains("Decision on"), "typed while watched and recent");

        b.watcher.report_watching("-", vec![]);
        b.pump().await;
        let screen = b.screen_with(agent.id, &b.told()).await;
        assert!(screen.contains(&b.told()), "{screen}");
    }

    /// Queued before a restart, told once after it, and never again.
    #[tokio::test]
    async fn an_answer_is_told_exactly_once_across_a_restart() {
        let b = board().await;
        let agent = b.agent("Agent 2").await;
        b.doing(agent.id, AgentActivity::Working).await;
        b.answer("Drill in");
        b.pump().await;
        let Board { dir, svc, watcher, task, .. } = b;
        let root = svc.root_dir().to_path_buf();
        drop(watcher);
        drop(svc);

        for _ in 0..2 {
            let svc = Arc::new(Service::open_in(root.clone()).await.expect("the daemon again"));
            let watcher = Watcher::new(svc.clone());
            let seen = Observation {
                activity: AgentActivity::Idle,
                state_since: now_millis(),
                blocked_question: None,
                turn_failed: false,
                command: "claude".into(),
                chat_capable: false,
            };
            watcher.observe_for_tests(agent.id, seen).await;
            watcher.pump_wakes().await;
            watcher.pump_wakes().await;
            tokio::time::sleep(Duration::from_millis(500)).await;
            let screen = svc.screen(agent.id).await.unwrap().0;
            assert_eq!(screen.matches("Decision on").count(), 1, "{screen}");
            assert_eq!(svc.store.notes_for(task.id, Some(NoteKind::Progress)).unwrap().len(), 1);
        }
        drop(dir);
    }

    /// No agent on the task: the orchestrator is told.
    #[tokio::test]
    async fn with_no_agent_the_orchestrator_is_told() {
        let b = board().await;
        let orchestrator = b.orchestrator().await;
        b.doing(orchestrator.id, AgentActivity::Idle).await;
        b.answer("Drill in");
        b.pump().await;
        let screen = b.screen_with(orchestrator.id, &b.told()).await;
        assert!(screen.contains(&b.told()), "{screen}");
        assert_eq!(b.progress()[0].body, "Told the orchestrator about the decision");
    }

    /// Nobody running: the task says so, and nothing waits.
    #[tokio::test]
    async fn with_nobody_there_the_task_says_nobody_was_told() {
        let b = board().await;
        b.answer("Drill in");
        b.pump().await;
        let progress = b.progress();
        assert_eq!(progress.len(), 1, "{progress:?}");
        assert_eq!((progress[0].actor, progress[0].body.as_str()), (Actor::Runner, "Nobody to tell about the decision"));
        assert!(b.svc.store.pending_answer_wakes().unwrap().is_empty());
    }

    /// Switched off, an answer is only a note: nothing typed, queued or said.
    #[tokio::test]
    async fn with_the_switch_off_nobody_is_told() {
        let b = board().await;
        let agent = b.agent("Agent 2").await;
        b.doing(agent.id, AgentActivity::Idle).await;
        let ws = b.svc.store.get_workspace(b.task.workspace_id).unwrap();
        b.svc.store.set_workspace_wake_on_answer(ws.id, ws.resource_version, false).unwrap();
        b.answer("Drill in");
        b.pump().await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!b.screen(agent.id).await.contains("Decision on"));
        assert!(b.svc.store.pending_answer_wakes().unwrap().is_empty());
        assert!(b.progress().is_empty());
    }

    /// An answer carrying escape sequences reaches the pane as text: the
    /// screen shows them written out, not acted on.
    #[tokio::test]
    async fn control_characters_in_an_answer_arrive_as_text() {
        let b = board().await;
        let agent = b.agent("Agent 2").await;
        b.doing(agent.id, AgentActivity::Idle).await;
        b.answer("Red\x1b[31m\x1b]0;owned\x07 please");
        b.pump().await;
        let screen = b.screen_with(agent.id, "please. Continue.").await;
        assert!(screen.contains(r"Red\u{1b}[31m\u{1b}]0;owned\u{7} please. Continue."), "{screen}");
    }

    /// A keystroke through the runner marks the pane as typed in; the
    /// runner's own typing of an answer doesn't.
    #[tokio::test]
    async fn typing_marks_the_pane_and_telling_does_not() {
        let b = board().await;
        let agent = b.agent("Agent 2").await;
        // Not the task's, so the answer still goes to Agent 2.
        let typist = b.svc.create_terminal(b.lane.id, "Agent 3", "claude").await.expect("a pane");
        b.doing(agent.id, AgentActivity::Idle).await;
        b.answer("Drill in");
        b.pump().await;
        assert!(b.screen_with(agent.id, &b.told()).await.contains(&b.told()));
        assert_eq!(last_input(b.svc.root_dir(), agent.id), None, "the runner marked its own telling");

        b.svc.send_bytes(typist.id, b"x").await.unwrap();
        let at = last_input(b.svc.root_dir(), typist.id).expect("a mark");
        assert!(now_millis() - at < 5_000, "{at}");
    }
}
