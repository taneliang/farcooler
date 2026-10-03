//! On a real tmux server, with `stand_in.pl` in the pane: a copy of perl
//! named `claude` or `codex`, so the pane's foreground process is what the
//! real agent's is, drawing that agent's box and logging what it's sent.
//! The real agents never run. The watcher's sampling loop doesn't run here,
//! so each test says what the watcher read (`doing`) and pumps.

use std::path::PathBuf;
use std::sync::Arc;

use farcooler_protocol::v1 as pb;
use farcooler_store::models::{NoteKind, Worktree};

use super::*;
use crate::needs_you::Observation;
use crate::service::Service;
use crate::test_support::ScratchDir;

const STAND_IN: &str = include_str!("stand_in.pl");

struct Board {
    dir: ScratchDir,
    svc: Arc<Service>,
    watcher: Arc<Watcher>,
    lane: Worktree,
    task: Task,
}

/// A stand-in in a pane: the file that picks what it shows, and its log.
struct StandIn {
    control: PathBuf,
    log: PathBuf,
}

impl StandIn {
    fn show(&self, mode: &str) {
        std::fs::write(&self.control, mode).unwrap();
    }

    fn log(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    fn submitted(&self) -> Vec<String> {
        self.log().lines().filter_map(|l| l.strip_prefix("SUBMIT ")).map(str::to_string).collect()
    }
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
    async fn agent(&self, title: &str, preset: &str) -> Terminal {
        self.svc
            .create_terminal_with_prompt(self.lane.id, title, preset, None, Some(self.task.id))
            .await
            .expect("an agent pane")
    }

    async fn orchestrator(&self) -> Terminal {
        self.svc.start_orchestrator(self.task.workspace_id, "claude", false, None).await.expect("an orchestrator")
    }

    /// Put `command` in `terminal`'s pane in place of what's there.
    async fn run_in(&self, terminal: &Terminal, command: &str) {
        let pane = self.svc.inventory_snapshot().claimants(terminal.id).into_iter().next().unwrap().pane_id.clone();
        self.svc.tmux.respawn_pane(&pane, &self.lane.worktree_path, command).await.unwrap();
        self.svc.inventory.refresh().await;
    }

    /// The stand-in for `agent` in `terminal`, run as a program named
    /// `as_name`: `agent` itself, or anything else for a process that
    /// isn't one.
    async fn stand_in(&self, terminal: &Terminal, agent: &str, as_name: &str) -> StandIn {
        let dir = self.dir.path().join(format!("si-{}", terminal.id.simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let program = dir.join(as_name);
        std::fs::copy(farcooler_core::programs::find("perl").expect("perl"), &program).unwrap();
        std::fs::write(dir.join("stand_in.pl"), STAND_IN).unwrap();
        let si = StandIn { control: dir.join("control"), log: dir.join("log") };
        si.show("idle");
        let q = |p: &std::path::Path| format!("'{}'", p.display());
        let command =
            format!("{} {} {agent} {} {}", q(&program), q(&dir.join("stand_in.pl")), q(&si.control), q(&si.log));
        self.run_in(terminal, &command).await;
        self.screen_with(terminal.id, "stand-in").await;
        si
    }

    async fn doing(&self, terminal: Uuid, activity: AgentActivity) {
        observe(&self.watcher, terminal, activity).await;
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

    /// One pass, after any pass `answered` spawned has finished.
    async fn pump(&self) {
        for _ in 0..100 {
            if self.watcher.wake_pump.try_lock().is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        self.watcher.wakes_hint.store(true, Ordering::SeqCst);
        self.watcher.pump_wakes().await;
    }

    async fn screen_with(&self, terminal: Uuid, text: &str) -> String {
        for _ in 0..60 {
            let screen = self.svc.screen(terminal).await.expect("a screen").0;
            if screen.contains(text) {
                return screen;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        self.svc.screen(terminal).await.unwrap().0
    }

    fn progress(&self) -> Vec<String> {
        self.svc.store.notes_for(self.task.id, Some(NoteKind::Progress)).unwrap().into_iter().map(|n| n.body).collect()
    }

    fn pending(&self) -> Vec<PendingWake> {
        self.svc.store.pending_answer_wakes().unwrap()
    }

    fn told(&self, answer: &str) -> String {
        message(&self.task.key, "Drill-in layout", answer)
    }

    /// Wait out the deadline, pump, and say what the task was told.
    async fn give_up(&self) -> Vec<String> {
        farcooler_store::testing::backdate_answer_wakes(&self.svc.store, GIVE_UP_AFTER_MS + 1);
        self.pump().await;
        self.progress()
    }

    /// Nothing went near the pane: no claim, no paste, no note yet.
    fn untouched(&self, si: &StandIn) {
        let pending = self.pending();
        assert_eq!(pending.len(), 1, "{pending:?}");
        assert_eq!(pending[0].claimed_at, None, "claimed");
        assert!(!si.log().contains("PASTE") && !si.log().contains("ENTER"), "{}", si.log());
        assert!(self.progress().is_empty(), "{:?}", self.progress());
    }
}

async fn observe(watcher: &Watcher, terminal: Uuid, activity: AgentActivity) {
    let seen = Observation {
        activity,
        state_since: now_millis(),
        blocked_question: None,
        turn_failed: false,
        command: "claude".into(),
        chat_capable: false,
    };
    watcher.observe_for_tests(terminal, seen).await;
}

// ---- the words ----

#[test]
fn the_message_is_one_line_with_the_answer_and_nothing_that_acts() {
    assert_eq!(message("ov-79", "Drill-in layout", "Drill in"), r#"Decision on ov-79 ("Drill-in layout"): Drill in. Continue."#);
    assert_eq!(message("ov-79", "Layout", "  Yes!  "), r#"Decision on ov-79 ("Layout"): Yes! Continue."#);
    let said = message("ov-1", "T\x1b]0;x\x07", "Red\x1b[31m now\r\nplease\u{9b}2J\x7f\u{202e}gnp\u{200b}");
    assert!(!said.chars().any(|c| c.is_control() || invisible(c)), "{said:?}");
    assert!(said.contains(r"Red\u{1b}[31m now please\u{9b}2J\u{7f}\u{202e}gnp\u{200b}"), "{said}");
    assert!(said.contains(r#"("T\u{1b}]0;x\u{7}")"#), "{said}");
    let long = message("ov-1", "T", &"word ".repeat(200));
    assert!(long.contains("…") && long.contains("(Full answer: farcooler task show ov-1.)"), "{long}");
    assert!(long.chars().count() < LONGEST_ANSWER + 100, "{long}");
}

#[test]
fn an_agent_is_known_by_its_executable() {
    assert_eq!(agent_of_executable("/Users/me/.local/share/claude/versions/2.1.237"), Some("claude"));
    assert_eq!(agent_of_executable("/usr/local/bin/claude"), Some("claude"));
    assert_eq!(agent_of_executable("/opt/homebrew/Caskroom/codex/0.147.0/bin/codex"), Some("codex"));
    assert_eq!(agent_of_executable("codex-aarch64-apple-darwin"), Some("codex"));
    for other in ["/bin/zsh", "-zsh", "/usr/bin/fish", "/bin/sh", "node", "/usr/bin/perl", "2.1.237", "claude-wrapper.sh"] {
        assert_eq!(agent_of_executable(other), None, "{other}");
    }
}

/// What's in front of a tty: the agent under the shell that launched it,
/// not its children; a shell alone; and nothing when it's ambiguous.
#[test]
fn the_process_in_front_is_the_one_under_the_shell() {
    let fish_claude = "100 1 Ss+ fish\n101 100 S+ /x/claude\n102 101 S+ /x/mcp-server\n";
    assert_eq!(in_front(fish_claude), Some((101, "/x/claude".into())));
    assert_eq!(in_front("100 1 Ss+ -zsh\n"), Some((100, "-zsh".into())));
    // A wrapper script waits on the agent, which is what reads the keys.
    let wrapped = "100 1 Ss+ fish\n101 100 S+ /bin/bash\n102 101 S+ /x/codex\n";
    assert_eq!(in_front(wrapped), Some((102, "/x/codex".into())));
    // A wrapper that isn't a shell is what's in front, and isn't an agent.
    let node = "100 1 Ss+ fish\n101 100 S+ node\n102 101 S+ /x/codex\n";
    assert_eq!(in_front(node), Some((101, "node".into())));
    assert_eq!(in_front("100 1 Ss zsh\n101 100 S /x/claude\n"), None, "nothing in the foreground");
    assert_eq!(in_front("100 1 S+ /x/claude\n200 1 S+ /x/codex\n"), None, "two in front");
}

// ---- telling ----

/// An idle claude with an empty box gets the answer pasted, read back and
/// submitted, once, and the task says so.
#[tokio::test]
async fn an_answer_is_pasted_into_an_idle_claude_and_submitted() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    b.answer("Drill in");
    b.pump().await;
    assert_eq!(si.submitted(), [b.told("Drill in")], "{}", si.log());
    assert!(si.log().contains("PASTE "), "a bracketed paste: {}", si.log());
    assert_eq!(b.progress(), ["Told Agent 2 about the decision"]);
    assert!(b.pending().is_empty());
}

/// codex too: its placeholder is dim, so its box reads empty.
#[tokio::test]
async fn an_answer_is_pasted_into_an_idle_codex() {
    let b = board().await;
    let agent = b.agent("Agent 2", "codex").await;
    let si = b.stand_in(&agent, "codex", "codex").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    b.answer("Drill in");
    b.pump().await;
    assert_eq!(si.submitted(), [b.told("Drill in")], "{}", si.log());
}

/// While the watcher reads it working the answer waits, and lands once it
/// reads idle.
#[tokio::test]
async fn an_answer_waits_for_a_working_agent_to_go_idle() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    b.doing(agent.id, AgentActivity::Working).await;
    b.answer("Drill in");
    b.pump().await;
    b.untouched(&si);
    b.doing(agent.id, AgentActivity::Done).await;
    b.pump().await;
    assert_eq!(si.submitted().len(), 1, "{}", si.log());
}

/// Someone typing holds it back: a mark in the last five seconds, or the
/// last fifteen while the pane is on someone's screen.
#[tokio::test]
async fn nothing_is_typed_while_someone_is_typing() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    let mark = crate::runtime::input_mark(b.svc.root_dir(), agent.id);
    std::fs::create_dir_all(mark.parent().unwrap()).unwrap();
    std::fs::write(&mark, now_millis().to_string()).unwrap();
    b.answer("Drill in");
    b.pump().await;
    b.untouched(&si);

    std::fs::write(&mark, (now_millis() - 8_000).to_string()).unwrap();
    b.watcher.report_watching("-", vec![agent.id]);
    b.pump().await;
    b.untouched(&si);

    b.watcher.report_watching("-", vec![]);
    b.pump().await;
    assert_eq!(si.submitted().len(), 1, "{}", si.log());
}

// ---- never into a shell, a menu or a draft ----

/// A pane whose foreground is a shell gets nothing, however much like claude
/// its screen looks, and in the end the task says why.
#[tokio::test]
async fn a_shell_in_the_foreground_is_never_typed_into() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let capture = format!("{}/../core/captures/claude-idle-fresh.txt", env!("CARGO_MANIFEST_DIR"));
    // A shell with bracketed paste on, so only the process check stands
    // between the answer and the shell.
    let shell = ["/bin/zsh", "/usr/bin/zsh", "/bin/bash"].into_iter().find(|s| std::path::Path::new(s).exists()).unwrap();
    let flags = if shell.ends_with("zsh") { "-f -i" } else { "--norc -i" };
    b.run_in(&agent, &format!("/bin/sh -c 'cat {capture}; exec {shell} {flags}'")).await;
    b.screen_with(agent.id, "for shortcuts").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    b.answer("Drill in");
    b.pump().await;
    let pending = b.pending();
    assert_eq!(pending[0].claimed_at, None, "claimed a shell");
    assert!(b.progress().is_empty());
    assert_eq!(b.give_up().await, ["Not delivered: no agent was running in the pane."]);
}

/// A perfect claude screen drawn by a process that isn't claude: nothing.
#[tokio::test]
async fn an_unknown_process_is_never_typed_into() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "perl").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    b.answer("Drill in");
    b.pump().await;
    b.untouched(&si);
    assert_eq!(b.give_up().await, ["Not delivered: no agent was running in the pane."]);
}

/// A permission prompt on screen: nothing, even with the watcher behind.
#[tokio::test]
async fn a_permission_menu_is_never_typed_into() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    si.show("menu");
    b.screen_with(agent.id, "Tab to amend").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    b.answer("Drill in");
    b.pump().await;
    b.untouched(&si);
    assert_eq!(b.give_up().await, ["Not delivered: a question or menu was showing."]);
}

/// A screen with claude's furniture and no box it knows: nothing.
#[tokio::test]
async fn an_unrecognized_screen_is_never_typed_into() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    si.show("picker");
    b.screen_with(agent.id, "Select model").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    b.answer("Drill in");
    b.pump().await;
    b.untouched(&si);
    assert_eq!(b.give_up().await, ["Not delivered: the agent's screen wasn't one Far Cooler recognizes."]);
}

/// A draft in the box is someone's: nothing is pasted beside it.
#[tokio::test]
async fn a_draft_is_never_touched() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    si.show("draft:fix the flaky");
    b.screen_with(agent.id, "fix the flaky").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    b.answer("Drill in");
    b.pump().await;
    b.untouched(&si);
    assert_eq!(b.give_up().await, ["Not delivered: there was a draft in the agent's box."]);
}

/// A paste that doesn't show up in the box as sent gets no Enter, and is
/// left there rather than erased.
#[tokio::test]
async fn a_paste_the_box_doesnt_hold_exactly_is_not_sent() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    si.show("mangle");
    b.screen_with(agent.id, "stand-in").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    b.answer("Drill in");
    b.pump().await;
    assert!(si.log().contains("PASTE "), "{}", si.log());
    assert!(!si.log().contains("ENTER"), "{}", si.log());
    assert_eq!(b.progress(), ["Paste left in the composer; not sent"]);
    assert!(b.pending().is_empty(), "and never tried again");
}

// ---- once, in order, and not forever ----

/// Claimed and never finished, as a crash mid-typing leaves it: never typed
/// again, and the task says it couldn't be confirmed.
#[tokio::test]
async fn a_claim_left_by_a_crash_is_never_typed_again() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    b.doing(agent.id, AgentActivity::Working).await;
    b.answer("Drill in");
    b.pump().await;
    let note = b.pending()[0].note;
    assert!(b.svc.store.claim_answer_wake(note).unwrap());

    let after = Watcher::new(b.svc.clone());
    observe(&after, agent.id, AgentActivity::Idle).await;
    after.pump_wakes().await;
    assert_eq!(b.progress(), ["Couldn't confirm the agent got the decision"]);
    assert!(!si.log().contains("PASTE"), "{}", si.log());
    assert!(b.pending().is_empty());
}

/// Queued before a restart, told once after it, and never again.
#[tokio::test]
async fn an_answer_is_told_exactly_once_across_a_restart() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
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
        observe(&watcher, agent.id, AgentActivity::Idle).await;
        watcher.pump_wakes().await;
        watcher.pump_wakes().await;
        assert_eq!(si.submitted().len(), 1, "{}", si.log());
        assert_eq!(svc.store.notes_for(task.id, Some(NoteKind::Progress)).unwrap().len(), 1);
    }
    drop(dir);
}

/// Two answers for one agent: the first is told; the second waits until the
/// agent has come back idle since, then is told on its own.
#[tokio::test]
async fn two_answers_for_one_agent_go_one_at_a_time() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    b.answer("Drill in");
    b.answer("Actually, tabs");
    b.pump().await;
    b.pump().await;
    assert_eq!(si.submitted(), [b.told("Drill in")], "{}", si.log());
    assert_eq!(b.pending().len(), 1);

    tokio::time::sleep(Duration::from_millis(5)).await;
    b.doing(agent.id, AgentActivity::Idle).await;
    b.pump().await;
    assert_eq!(si.submitted(), [b.told("Drill in"), b.told("Actually, tabs")], "{}", si.log());
}

// ---- whom ----

/// No agent on the task: the orchestrator is told.
#[tokio::test]
async fn with_no_agent_the_orchestrator_is_told() {
    let b = board().await;
    let orchestrator = b.orchestrator().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    b.answer("Drill in");
    b.pump().await;
    assert_eq!(si.submitted().len(), 1, "{}", si.log());
    assert_eq!(b.progress(), ["Told the orchestrator about the decision"]);
}

/// Nobody running: the task says so, and nothing waits.
#[tokio::test]
async fn with_nobody_there_the_task_says_nobody_was_told() {
    let b = board().await;
    b.answer("Drill in");
    b.pump().await;
    assert_eq!(b.progress(), [NOBODY]);
    assert!(b.pending().is_empty());
}

/// The task's agent, moved to another workspace, is never told.
#[tokio::test]
async fn an_agent_in_another_workspace_is_never_told() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    let billing = b.svc.store.create_workspace(b.lane.repository_id, "Billing", "bil").unwrap();
    b.svc.store.set_terminal_workspace(agent.id, billing.id).unwrap();
    b.answer("Drill in");
    b.pump().await;
    assert!(!si.log().contains("PASTE"), "{}", si.log());
    assert_eq!(b.progress(), [NOBODY]);
}

/// Switched off, an answer is only a note: nothing typed, queued or said.
#[tokio::test]
async fn with_the_switch_off_nobody_is_told() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    let ws = b.svc.store.get_workspace(b.task.workspace_id).unwrap();
    b.svc.store.set_workspace_wake_on_answer(ws.id, ws.resource_version, false).unwrap();
    b.answer("Drill in");
    b.pump().await;
    assert!(!si.log().contains("PASTE"));
    assert!(b.pending().is_empty());
    assert!(b.progress().is_empty());
}

/// An answer carrying escape sequences reaches the agent as text.
#[tokio::test]
async fn control_characters_in_an_answer_arrive_as_text() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    b.answer("Red\x1b[31m\x1b]0;owned\x07 please");
    b.pump().await;
    let sent = si.submitted();
    assert_eq!(sent.len(), 1, "{}", si.log());
    assert!(sent[0].contains(r"Red\u{1b}[31m\u{1b}]0;owned\u{7} please. Continue."), "{sent:?}");
}

/// A keystroke through the runner marks the pane as typed in; the runner's
/// own paste and Enter don't.
#[tokio::test]
async fn typing_marks_the_pane_and_telling_does_not() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    b.answer("Drill in");
    b.pump().await;
    assert_eq!(si.submitted().len(), 1, "{}", si.log());
    assert_eq!(last_input(b.svc.root_dir(), agent.id), None, "the runner marked its own telling");

    b.svc.send_bytes(agent.id, b"x").await.unwrap();
    let at = last_input(b.svc.root_dir(), agent.id).expect("a mark");
    assert!(now_millis() - at < 5_000, "{at}");
}

/// A mark that can't be read is someone typing just now: doubt means no.
#[test]
fn an_unreadable_mark_is_typing_now() {
    let dir = tempfile::tempdir().unwrap();
    let terminal = Uuid::now_v7();
    assert_eq!(last_input(dir.path(), terminal), None);
    let mark = crate::runtime::input_mark(dir.path(), terminal);
    std::fs::create_dir_all(mark.parent().unwrap()).unwrap();
    std::fs::write(&mark, "").unwrap();
    assert!(last_input(dir.path(), terminal).is_some_and(|at| now_millis() - at < 1_000));
    crate::runtime::mark_input(dir.path(), terminal);
    assert!(last_input(dir.path(), terminal).is_some_and(|at| now_millis() - at < 1_000));
}
