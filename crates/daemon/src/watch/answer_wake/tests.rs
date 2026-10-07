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

/// Whether this host's tmux reports bracketed paste (`bracket_paste_flag`,
/// new in 3.7), read from `tmux -V` rather than from the flag, so a broken
/// flag read can't switch the tests that need it off. A version it can't
/// read counts as new enough: those tests run, and fail if it isn't.
///
/// The typing tests run on both: below 3.7 the gate reads the pane's output
/// (`paste_mode`), and `settle_stand_in` waits for that record. The tests
/// of what that record can't know run only below 3.7, so Ubuntu's 3.4 leg
/// of CI drives them and macOS's current tmux drives the flag.
fn tmux_tells_bracketing() -> bool {
    static KNOWS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *KNOWS.get_or_init(|| {
        let tmux = farcooler_core::programs::find("tmux").expect("these tests need tmux");
        let out = std::process::Command::new(tmux).arg("-V").output().expect("tmux -V");
        let knows = tmux_version_tells_bracketing(&String::from_utf8_lossy(&out.stdout));
        if !knows {
            eprintln!("tmux older than 3.7: skipping the tests that need bracket_paste_flag");
        }
        knows
    })
}

/// `tmux 3.4`, `tmux 3.7c`, `tmux next-3.8`: whether that's 3.7 or later.
fn tmux_version_tells_bracketing(v: &str) -> bool {
    let v = v.trim().trim_start_matches("tmux").trim().trim_start_matches("next-");
    let mut parts = v.split('.');
    let major = parts.next().and_then(|m| m.parse::<u32>().ok());
    let minor = parts.next().map(|m| m.trim_end_matches(|c: char| !c.is_ascii_digit())).and_then(|m| m.parse::<u32>().ok());
    match (major, minor) {
        (Some(major), Some(minor)) => (major, minor) >= (3, 7),
        _ => true,
    }
}

#[test]
fn a_tmux_version_is_read_for_bracketing() {
    for (v, knows) in [("tmux 3.4\n", false), ("tmux 3.3a", false), ("tmux 3.6b", false), ("tmux 3.7", true), ("tmux 3.7c\n", true), ("tmux next-3.8", true), ("tmux 4.0", true), ("tmux master", true), ("", true)] {
        assert_eq!(tmux_version_tells_bracketing(v), knows, "{v:?}");
    }
}

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
    /// Show `mode`, and wait until the stand-in has taken it and drawn it.
    async fn show(&self, mode: &str) {
        let before = self.log().matches(&format!("MODE {mode}\n")).count();
        std::fs::write(&self.control, mode).unwrap();
        for _ in 0..750 {
            if self.log().matches(&format!("MODE {mode}\n")).count() > before {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("the stand-in never showed {mode}: {}", self.log());
    }

    /// Wait, bounded, for the stand-in to log a paste: the bytes reach it a
    /// moment after the daemon sends them, longer on a slow runner.
    async fn pasted(&self) {
        for _ in 0..750 {
            if self.log().contains("PASTE ") {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Wait, up to 15 seconds, until the stand-in has logged `n` submissions:
    /// it reads the Enter a moment after `pump` returns, longer under load.
    async fn submits(&self, n: usize) {
        for _ in 0..750 {
            if self.submitted().len() >= n {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
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

    /// Put `command` in `terminal`'s pane in place of what's there, as
    /// someone else's `respawn-pane` would, and let the daemon find it as it
    /// does in production: an answer's check drops the record of the
    /// program before, and the next sample follows the new one.
    async fn run_in(&self, terminal: &Terminal, command: &str) {
        self.run_unfollowed(terminal, command).await;
        self.refollow(terminal.id).await;
    }

    /// What production does about a pane whose record is stale: check 5
    /// drops it (`streamed_bracketed_paste`), the next sample follows the
    /// pane afresh. Waits for the subscription where production makes one.
    async fn refollow(&self, terminal: Uuid) {
        self.svc.streamed_bracketed_paste(terminal).await;
        if !self.follows(terminal) {
            return self.watcher.sample().await;
        }
        if !self.sample_until_followed(terminal).await {
            // What the pane shows says why: a program that never started
            // leaves its shell's error on a dead pane, which no sample follows.
            let alive = self.svc.inventory_snapshot().claimants(terminal).iter().any(|p| p.proves_life());
            let screen = self.svc.screen(terminal).await.map(|s| s.0).unwrap_or_default();
            panic!("the sample never followed the pane (alive: {alive}): {screen}");
        }
    }

    /// Sample, and sample again, until a live record follows `terminal`'s
    /// pane. One sample is not enough on a slow runner: the subscription
    /// can still be coming up, or a sample can land before the pane is
    /// claimed. Bounded at about a minute; false if it never follows.
    async fn sample_until_followed(&self, terminal: Uuid) -> bool {
        for _ in 0..30 {
            self.watcher.sample().await;
            for _ in 0..100 {
                if self.svc.paste_mode_followed(terminal).await {
                    return true;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
        false
    }

    /// Whether production follows `terminal`'s output for bracketed paste:
    /// where tmux can't report it, for a pane that may be typed answers
    /// (`may_be_typed_to`). Read from the record as it is now, after any
    /// adoption.
    fn follows(&self, terminal: Uuid) -> bool {
        let t = self.svc.store.get_terminal(terminal).unwrap();
        !tmux_tells_bracketing() && t.pane_mode == PaneMode::Terminal && may_be_typed_to(&t.command_preset, t.role)
    }

    /// Whether, within 15 seconds, a live record follows the program now
    /// in `terminal`'s pane.
    async fn followed(&self, terminal: Uuid) -> bool {
        for _ in 0..750 {
            if self.svc.paste_mode_followed(terminal).await {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        false
    }

    /// `run_in`, as someone else's `respawn-pane` would: nothing follows the
    /// new program's output from its start.
    async fn run_unfollowed(&self, terminal: &Terminal, command: &str) {
        let pane = self.svc.inventory_snapshot().claimants(terminal.id).into_iter().next().unwrap().pane_id.clone();
        self.svc.tmux.respawn_pane(&pane, &self.lane.worktree_path, command).await.unwrap();
        self.svc.inventory.refresh().await;
    }

    /// On a tmux that can't report bracketing, wait until the daemon's record
    /// of `terminal`'s output says `known`: the bytes reach it a moment after
    /// the program writes them. Panics if it never does.
    async fn stream_says(&self, terminal: Uuid, known: Option<bool>) {
        if tmux_tells_bracketing() {
            return;
        }
        let mut now = None;
        for _ in 0..750 {
            now = self.svc.streamed_bracketed_paste(terminal).await;
            if now == known {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("the stream record says {now:?}, not {known:?}");
    }

    /// The stand-in for `agent` in `terminal`, run as a program named
    /// `as_name`: `agent` itself, or anything else for a process that
    /// isn't one.
    async fn stand_in(&self, terminal: &Terminal, agent: &str, as_name: &str) -> StandIn {
        let dir = self.dir.path().join(format!("si-{}", terminal.id.simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let program = dir.join(as_name);
        copy_program(&farcooler_core::programs::find("perl").expect("perl"), &program);
        std::fs::write(dir.join("stand_in.pl"), STAND_IN).unwrap();
        let si = StandIn { control: dir.join("control"), log: dir.join("log") };
        std::fs::write(&si.control, "idle").unwrap();
        let q = |p: &std::path::Path| format!("'{}'", p.display());
        // Its own claude config, never the real one (`mid_turn`).
        std::fs::create_dir_all(dir.join("config")).unwrap();
        let command = format!(
            "env CLAUDE_CONFIG_DIR={} {} {} {agent} {} {}",
            q(&dir.join("config")),
            q(&program),
            q(&dir.join("stand_in.pl")),
            q(&si.control),
            q(&si.log)
        );
        self.run_in(terminal, &command).await;
        self.settle_stand_in(terminal, &si).await;
        si
    }

    /// Wait for a stand-in to draw its box, then draw it again: the redraw
    /// sets bracketing after the daemon has subscribed, which a stand-in's
    /// start may have beaten (`stand_in_set_bracketing_before_anyone_followed`
    /// is what that costs).
    async fn settle_stand_in(&self, terminal: &Terminal, si: &StandIn) {
        self.screen_with(terminal.id, "stand-in").await;
        for _ in 0..750 {
            if si.log().contains("MODE idle") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        si.show("working").await;
        si.show("idle").await;
        if self.follows(terminal.id) {
            self.stream_says(terminal.id, Some(true)).await;
        }
    }

    /// The stand-in run as Node runs an npm install: a copy of perl named
    /// `node`, running the stand-in from a script named `script`.
    async fn node_stand_in(&self, terminal: &Terminal, script: &str) -> StandIn {
        let dir = self.dir.path().join(format!("si-{}", terminal.id.simple()));
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        let node = dir.join("node");
        copy_program(&farcooler_core::programs::find("perl").expect("perl"), &node);
        let file = dir.join("bin").join(script);
        std::fs::write(&file, STAND_IN).unwrap();
        let si = StandIn { control: dir.join("control"), log: dir.join("log") };
        std::fs::write(&si.control, "idle").unwrap();
        let q = |p: &std::path::Path| format!("'{}'", p.display());
        self.run_in(terminal, &format!("{} {} claude {} {}", q(&node), q(&file), q(&si.control), q(&si.log))).await;
        self.settle_stand_in(terminal, &si).await;
        si
    }

    async fn doing(&self, terminal: Uuid, activity: AgentActivity) {
        observe(&self.watcher, terminal, activity).await;
    }

    /// Answer as the person at a client, through `task.note` itself.
    fn answer(&self, body: &str) {
        self.answer_on(self.task.id, body);
    }

    fn answer_on(&self, task: Uuid, body: &str) {
        let req = pb::TaskNoteAppend {
            task_id: task.as_bytes().to_vec().into(),
            kind: pb::TaskNoteKind::Answer as i32,
            body: body.into(),
            ..Default::default()
        };
        crate::task_ops::note(&self.svc, &self.watcher, &req).expect("answered");
    }

    /// One pass, after any pass `answered` spawned has finished.
    async fn pump(&self) {
        for _ in 0..750 {
            if self.watcher.wake_pump.try_lock().is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        self.watcher.wakes_hint.store(true, Ordering::SeqCst);
        self.watcher.pump_wakes().await;
    }

    async fn screen_with(&self, terminal: Uuid, text: &str) -> String {
        for _ in 0..300 {
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
        self.settled()
    }

    /// Nothing went near the pane: no claim, no paste, no note yet.
    fn untouched(&self, si: &StandIn) {
        let pending = self.pending();
        assert_eq!(pending.len(), 1, "{pending:?}");
        assert_eq!(pending[0].claimed_at, None, "claimed");
        assert!(!si.log().contains("PASTE") && !si.log().contains("ENTER"), "{}", si.log());
        let said = self.progress();
        assert!(said.len() <= 1 && said.iter().all(|n| n.starts_with(WAITING)), "{said:?}");
    }

    /// What the task said after the answer, the "Waiting to tell" note left
    /// out.
    fn settled(&self) -> Vec<String> {
        self.progress().into_iter().filter(|n| !n.starts_with(WAITING)).collect()
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
    assert_eq!(message("ov-79", "Drill-in layout", "Drill in"), "Decision on ov-79 (“Drill-in layout”): Drill in. Continue.");
    assert_eq!(message("ov-79", "Layout", "  Yes!  "), "Decision on ov-79 (“Layout”): Yes! Continue.");
    let said = message("ov-1", "T\x1b]0;x\x07", "Red\x1b[31m now\r\nplease\u{9b}2J\x7f\u{202e}gnp\u{200b}");
    assert!(!said.chars().any(|c| c.is_control() || invisible(c)), "{said:?}");
    assert!(said.contains(r"Red\u{1b}[31m now please\u{9b}2J\u{7f}\u{202e}gnp\u{200b}"), "{said}");
    assert!(said.contains(r"(“T\u{1b}]0;x\u{7}”)"), "{said}");
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
    // A shell claude put in the background isn't in the foreground group.
    let background = "100 1 Ss+ fish\n101 100 S+ /x/claude\n103 101 S /bin/zsh\n";
    assert_eq!(in_front(background), Some((101, "/x/claude".into())));
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
    si.submits(1).await;
    assert_eq!(si.submitted(), [b.told("Drill in")], "{}", si.log());
    si.pasted().await;
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
    si.submits(1).await;
    assert_eq!(si.submitted(), [b.told("Drill in")], "{}", si.log());
}

/// While the watcher can't say what the agent is doing the answer waits,
/// and lands once it reads idle. (Working is no hold for claude or codex,
/// which queue: `mid_turn_tests`.)
#[tokio::test]
async fn an_answer_waits_while_the_agent_is_unknown() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    b.doing(agent.id, AgentActivity::Unknown).await;
    b.answer("Drill in");
    b.pump().await;
    b.untouched(&si);
    b.doing(agent.id, AgentActivity::Done).await;
    b.pump().await;
    si.submits(1).await;
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
    si.submits(1).await;
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
    assert_eq!(b.progress(), ["Waiting to tell Agent 2 about the decision: no agent is running in its pane."]);
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
    si.show("menu").await;
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
    si.show("picker").await;
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
    si.show("draft:fix the flaky").await;
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
    si.show("mangle").await;
    b.screen_with(agent.id, "stand-in").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    b.answer("Drill in");
    b.pump().await;
    si.pasted().await;
    assert!(si.log().contains("PASTE "), "{}", si.log());
    assert!(!si.log().contains("ENTER"), "{}", si.log());
    assert_eq!(b.settled(), ["Paste left in the composer; not sent"]);
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
    b.doing(agent.id, AgentActivity::Unknown).await;
    b.answer("Drill in");
    b.pump().await;
    let note = b.pending()[0].note;
    assert!(b.svc.store.claim_answer_wake(note).unwrap());

    let after = Watcher::new(b.svc.clone());
    observe(&after, agent.id, AgentActivity::Idle).await;
    after.pump_wakes().await;
    assert_eq!(b.settled(), ["Couldn't confirm the agent got the decision"]);
    assert!(!si.log().contains("PASTE"), "{}", si.log());
    assert!(b.pending().is_empty());
}

/// Queued before a restart, told once after it, and never again.
#[tokio::test]
async fn an_answer_is_told_exactly_once_across_a_restart() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    b.doing(agent.id, AgentActivity::Unknown).await;
    b.answer("Drill in");
    b.pump().await;
    let Board { dir, svc, watcher, task, .. } = b;
    let root = svc.root_dir().to_path_buf();
    drop(watcher);
    drop(svc);

    for _ in 0..2 {
        let svc = Arc::new(Service::open_in(root.clone()).await.expect("the daemon again"));
        let watcher = Watcher::new(svc.clone());
        if !tmux_tells_bracketing() {
            // A daemon that has just started knows nothing of the agent's
            // bracketing until its first sample follows the pane and the
            // agent sets it again.
            watcher.sample().await;
            for _ in 0..750 {
                if svc.paste_mode_followed(agent.id).await {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            si.show("working").await;
            si.show("idle").await;
            for _ in 0..750 {
                if svc.streamed_bracketed_paste(agent.id).await.is_some() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
        observe(&watcher, agent.id, AgentActivity::Idle).await;
        watcher.pump_wakes().await;
        watcher.pump_wakes().await;
        si.submits(1).await;
        assert_eq!(si.submitted().len(), 1, "{}", si.log());
        // Said to be waiting once, before the restart; told once after.
        let said: Vec<String> =
            svc.store.notes_for(task.id, Some(NoteKind::Progress)).unwrap().into_iter().map(|n| n.body).collect();
        assert_eq!(said, ["Waiting to tell Agent 2 about the decision: it's busy.", "Told Agent 2 about the decision"]);
    }
    drop(dir);
}

/// Two answers on one task before either is told: only the newest is
/// told, and the older is noted as replaced.
#[tokio::test]
async fn a_newer_answer_replaces_one_not_yet_told() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    b.doing(agent.id, AgentActivity::Unknown).await;
    b.answer("Drill in");
    b.answer("Actually, tabs");
    b.pump().await;
    b.doing(agent.id, AgentActivity::Idle).await;
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), [b.told("Actually, tabs")], "{}", si.log());
    assert_eq!(b.settled(), ["Not delivered: a newer answer replaced it.", "Told Agent 2 about the decision"]);
}

/// Answers on two tasks for one terminal go one at a time, the second once
/// the spacing has passed, with no fresh sample in between: nothing waits
/// on catching the agent's turn.
#[tokio::test]
async fn two_answers_for_one_terminal_go_one_at_a_time() {
    let b = board().await;
    let other = b.svc.store.create_task(b.task.workspace_id, "Tabs or spaces", Actor::Manager).unwrap();
    let orchestrator = b.orchestrator().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Unknown).await;
    b.answer("Drill in");
    b.answer_on(other.id, "Tabs");
    b.pump().await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    b.pump().await;
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), [b.told("Drill in")], "{}", si.log());
    assert_eq!(b.pending().len(), 1);

    // A literal, so a longer spacing, or waiting on a fresh sample, fails.
    tokio::time::sleep(Duration::from_millis(2_100)).await;
    b.pump().await;
    let second = message(&other.key, "Tabs or spaces", "Tabs");
    si.submits(2).await;
    assert_eq!(si.submitted(), [b.told("Drill in"), second], "{}", si.log());
}

// ---- the process in front, and the rest of the gate ----

#[test]
fn a_node_install_is_known_by_its_script() {
    assert_eq!(agent_of_process("/opt/homebrew/bin/node", "node /opt/homebrew/bin/claude --model opus"), Some("claude"));
    assert_eq!(
        agent_of_process("/usr/local/bin/node", "node --no-warnings /usr/lib/node_modules/@anthropic-ai/claude-code/cli.js"),
        Some("claude")
    );
    assert_eq!(agent_of_process("node", "node /x/node_modules/@openai/codex/bin/codex.js"), Some("codex"));
    assert_eq!(agent_of_process("/opt/homebrew/bin/node", "node /x/server.js"), None);
    assert_eq!(agent_of_process("/opt/homebrew/bin/node", "node"), None);
    assert_eq!(agent_of_process("/bin/zsh", "zsh /x/bin/claude"), None, "only node's script counts");
    assert_eq!(agent_of_process("/x/claude/versions/2.1.237", "2.1.237"), Some("claude"));
}

/// claude under node, as npm installs it: told.
#[tokio::test]
async fn a_node_claude_is_told() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.node_stand_in(&agent, "claude").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    b.answer("Drill in");
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), [b.told("Drill in")], "{}", si.log());
}

/// node running some other script, drawing claude's screen: nothing.
#[tokio::test]
async fn a_node_running_something_else_is_never_typed_into() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.node_stand_in(&agent, "server.js").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    b.answer("Drill in");
    b.pump().await;
    b.untouched(&si);
    assert_eq!(b.give_up().await, ["Not delivered: no agent was running in the pane."]);
}

/// The note an answer that was never proven takeable ends with.
fn unproven() -> String {
    format!("Not delivered: {}.", Held::Unproven.why())
}

/// A tmux too old to report bracketed paste, and an agent that set it
/// before the daemon was following its output (an agent started before this
/// daemon): nothing proves the agent takes a paste, so nothing is typed into
/// its idle, empty box, and in the end the task says why.
#[tokio::test]
async fn stand_in_set_bracketing_before_anyone_followed() {
    if tmux_tells_bracketing() {
        return;
    }
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    // The same agent, restarted by hand: it sets bracketing as it starts,
    // and the daemon only follows it once it's up.
    b.run_unfollowed(&agent, &si_command(&b, &agent)).await;
    b.screen_with(agent.id, "stand-in").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    b.refollow(agent.id).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    b.doing(agent.id, AgentActivity::Idle).await;
    b.answer("Drill in");
    b.pump().await;
    b.untouched(&si);
    assert_eq!(b.progress(), [format!("Waiting to tell Agent 2 about the decision: {}.", Held::Unproven.now())]);
    assert_eq!(b.give_up().await, [unproven()]);
}

/// Waiting on an unproven paste names the tmux reason from the first note,
/// on any host.
#[test]
fn waiting_on_an_unproven_paste_names_tmux() {
    assert!(Held::Unproven.now().contains("tmux is older than 3.7"), "{}", Held::Unproven.now());
}

/// The task's last word on an unproven paste says what to do, not only that
/// it couldn't tell (ov-208), without naming a cause it can't know and
/// without internal words.
#[test]
fn an_undelivered_answer_says_to_restart_the_agent() {
    let why = Held::Unproven.why();
    assert!(why.contains("Restart the agent in its pane"), "{why}");
    assert!(!why.contains("followed") && !why.contains("started before"), "{why}");
    assert!(!Held::Unproven.now().contains("followed"), "{}", Held::Unproven.now());
    assert!(Held::Unproven.now().contains("Restarting the agent in its pane"), "{}", Held::Unproven.now());
}

/// A shell adopted as the orchestrator is followed for bracketed paste, and
/// once it is demoted it isn't: its pipe and reader would otherwise stay
/// until the pane died (ov-208, ov-201 review note 3). Forced to the tmux
/// 3.4 path, with the inventory read before following, so it runs the same
/// on any tmux.
#[tokio::test]
async fn a_demoted_orchestrator_is_no_longer_followed() {
    // Following pipes the pane through the daemon binary, which `--lib`
    // doesn't build. A full run does, and CI's must not skip.
    if crate::runtime::fanout_binary().is_none() {
        assert!(std::env::var_os("CI").is_none(), "no farcoolerd beside the test binary");
        eprintln!("SKIP a_demoted_orchestrator_is_no_longer_followed: no farcoolerd (`--lib` builds none)");
        return;
    }
    let b = board().await;
    b.svc.assume_tmux_cannot_report_paste_mode();
    let orchestrator = b.adopted_shell().await;
    // Adoption follows at once, but from the inventory as it was: the new
    // pane may not be in it yet. Read tmux, then follow, as the next sample
    // does, so the test doesn't depend on that race.
    b.svc.inventory.refresh().await;
    b.svc.follow_paste_mode(orchestrator.id);
    assert!(b.followed(orchestrator.id).await, "an adopted orchestrator is followed");
    let demoted = b.svc.set_terminal_role(orchestrator.id, TerminalRole::Shell).await.expect("demoted");
    assert!(!b.svc.paste_mode_followed(demoted.id).await, "a demoted shell is still followed");
}

/// What the stream said about one program says nothing about the next:
/// respawned by someone else, the new agent sets bracketing on the same
/// pipe, and the record, kept for the program before, isn't used. The
/// answer waits, and once the daemon follows the new agent and it sets
/// bracketing again, it's told.
#[tokio::test]
async fn a_record_from_before_a_respawn_is_never_used() {
    if tmux_tells_bracketing() {
        return;
    }
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    b.run_unfollowed(&agent, &si_command(&b, &agent)).await;
    b.screen_with(agent.id, "stand-in").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    b.doing(agent.id, AgentActivity::Idle).await;
    b.answer("Drill in");
    b.pump().await;
    b.untouched(&si);
    assert!(b.sample_until_followed(agent.id).await, "the sample follows the new agent");
    si.show("working").await;
    si.show("idle").await;
    b.stream_says(agent.id, Some(true)).await;
    b.doing(agent.id, AgentActivity::Idle).await;
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), [b.told("Drill in")], "{}", si.log());
}

/// A stream that broke: whatever came through the gap is unseen, so the
/// record is worth nothing, even though it last read bracketing on. The
/// answer waits rather than being given up, and is told once the stream is
/// followed again and the agent sets bracketing.
#[tokio::test]
async fn a_broken_stream_proves_nothing_until_it_is_followed_again() {
    if tmux_tells_bracketing() {
        return;
    }
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    let pane = b.svc.inventory_snapshot().claimants(agent.id).into_iter().next().unwrap().pane_id.clone();
    // Closing the pane's pipe ends its fanout, and so every subscription.
    b.svc.tmux.run(&["pipe-pane", "-t", &pane]).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    b.doing(agent.id, AgentActivity::Idle).await;
    b.answer("Drill in");
    b.pump().await;
    b.untouched(&si);
    assert!(b.sample_until_followed(agent.id).await, "the sample follows the pane again");
    si.show("working").await;
    si.show("idle").await;
    b.stream_says(agent.id, Some(true)).await;
    b.doing(agent.id, AgentActivity::Idle).await;
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), [b.told("Drill in")], "{}", si.log());
}

// ---- following, through the daemon's own entry points ----
//
// None of these call `follow_paste_mode`: each drives the place production
// starts a program and asserts the daemon followed it.

/// An agent created through the service is followed from its start.
#[tokio::test]
async fn a_created_agent_is_followed() {
    if tmux_tells_bracketing() {
        return;
    }
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    assert!(b.followed(agent.id).await);
}

/// An agent split beside another is followed from its start.
#[tokio::test]
async fn a_split_agent_is_followed() {
    if tmux_tells_bracketing() {
        return;
    }
    let b = board().await;
    let first = b.agent("Agent 2", "claude").await;
    let split = b
        .svc
        .split_terminal(b.lane.id, first.id, pb::SplitSide::Right, "Agent 3", "claude")
        .await
        .expect("a split agent");
    assert!(b.followed(split.id).await);
}

/// A restarted agent is a new program, followed from its start.
#[tokio::test]
async fn a_restarted_agent_is_followed() {
    if tmux_tells_bracketing() {
        return;
    }
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    assert!(b.followed(agent.id).await);
    b.svc.restart_terminal(agent.id).await.expect("restarted");
    b.svc.inventory.refresh().await;
    assert!(b.followed(agent.id).await, "the program after the restart");
}

/// A chat pane switched back to its terminal is followed from its start.
#[tokio::test]
async fn a_pane_switched_to_its_terminal_is_followed() {
    if tmux_tells_bracketing() {
        return;
    }
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let now = b.svc.store.get_terminal(agent.id).unwrap();
    b.svc.store.set_pane_mode(agent.id, now.resource_version, PaneMode::Agent, None, false).unwrap();
    b.svc.set_pane_mode(agent.id, PaneMode::Terminal, true).await.expect("switched");
    b.svc.inventory.refresh().await;
    assert!(b.followed(agent.id).await, "the program after the switch");
}

/// An agent pane started before this daemon is followed on its first
/// sample, the catch-up every typing test above also goes through.
#[tokio::test]
async fn a_pane_started_before_the_daemon_is_followed_on_a_sample() {
    if tmux_tells_bracketing() {
        return;
    }
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    // Creation follows in the background: done with that first.
    assert!(b.followed(agent.id).await);
    b.run_unfollowed(&agent, "sleep 30").await;
    b.svc.streamed_bracketed_paste(agent.id).await;
    assert!(!b.svc.paste_mode_followed(agent.id).await);
    assert!(b.sample_until_followed(agent.id).await);
}

/// A tmux that reports bracketing itself gets no pipe and no fanout: after
/// an agent is created and sampled, nothing is piping its pane.
#[tokio::test]
async fn a_tmux_that_reports_bracketing_starts_no_fanout() {
    if !tmux_tells_bracketing() {
        return;
    }
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    b.watcher.sample().await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let pane = b.svc.inventory_snapshot().claimants(agent.id).into_iter().next().unwrap().pane_id.clone();
    let piped = b.svc.tmux.run(&["display-message", "-p", "-t", &pane, "#{pane_pipe}"]).await.unwrap();
    assert_eq!(piped.stdout.trim(), "0", "the pane is being piped");
    assert!(!b.svc.paste_mode_followed(agent.id).await);
}

/// Copy the executable `from` to `to` in a `cp`, never a file this process
/// opens: on Linux, exec fails with ETXTBSY while any process holds the file
/// open to write, and a child another test thread forked mid-copy holds it
/// until it execs. The stand-in then never starts and its dead pane is never
/// followed (ov-251, only on CI's Linux).
fn copy_program(from: &std::path::Path, to: &std::path::Path) {
    let status = std::process::Command::new("cp").arg(from).arg(to).status().expect("cp");
    assert!(status.success(), "cp {} {}: {status}", from.display(), to.display());
}

/// The command `stand_in` put in `agent`'s pane, to run it again.
fn si_command(b: &Board, agent: &Terminal) -> String {
    let dir = b.dir.path().join(format!("si-{}", agent.id.simple()));
    let q = |p: &std::path::Path| format!("'{}'", p.display());
    format!(
        "{} {} claude {} {}",
        q(&dir.join("claude")),
        q(&dir.join("stand_in.pl")),
        q(&dir.join("control")),
        q(&dir.join("log"))
    )
}

/// Bracketed paste off: nothing.
#[tokio::test]
async fn a_pane_without_bracketed_paste_is_never_typed_into() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    si.show("nobracket").await;
    b.stream_says(agent.id, Some(false)).await;
    b.doing(agent.id, AgentActivity::Idle).await;
    b.answer("Drill in");
    b.pump().await;
    b.untouched(&si);
}

/// Someone types while the paste is being read back: no Enter.
#[tokio::test]
async fn typing_during_the_read_back_stops_the_enter() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    si.show("slow").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    let (root, id, log) = (b.svc.root_dir().to_path_buf(), agent.id, si.log.clone());
    let typist = tokio::spawn(async move {
        for _ in 0..1500 {
            if std::fs::read_to_string(&log).unwrap_or_default().contains("PASTE ") {
                crate::runtime::mark_input(&root, id);
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });
    b.answer("Drill in");
    b.pump().await;
    typist.await.unwrap();
    si.pasted().await;
    assert!(si.log().contains("PASTE "), "{}", si.log());
    assert!(!si.log().contains("ENTER"), "{}", si.log());
    assert_eq!(b.settled(), ["Paste left in the composer; not sent"]);
}

/// A paste that fails to send after the claim: never retried, and noted.
#[tokio::test]
async fn a_send_failing_after_the_claim_is_not_retried() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    b.doing(agent.id, AgentActivity::Unknown).await;
    b.answer("Drill in");
    b.pump().await;
    b.watcher.fail_sends.store(true, Ordering::SeqCst);
    b.doing(agent.id, AgentActivity::Idle).await;
    b.pump().await;
    b.pump().await;
    assert_eq!(b.settled(), ["Couldn't confirm the agent got the decision"]);
    assert!(!si.log().contains("PASTE"), "{}", si.log());
    assert!(b.pending().is_empty());
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
    si.submits(1).await;
    assert_eq!(si.submitted().len(), 1, "{}", si.log());
    assert_eq!(b.progress(), ["Told the orchestrator about the decision"]);
}

impl Board {
    /// A shell pane in the lane, as the Mac opens one, adopted as the
    /// workspace's orchestrator as the Mac's Make Orchestrator does: how
    /// the owner's orchestrators are made, with claude started by hand.
    async fn adopted_shell(&self) -> Terminal {
        let shell = self.shell_pane().await;
        self.svc.set_terminal_role(shell.id, TerminalRole::Orchestrator).await.expect("adopted")
    }

    async fn shell_pane(&self) -> Terminal {
        let t = self.svc.create_terminal_with_prompt(self.lane.id, "Terminal 1", "shell", None, None).await.expect("a shell");
        assert_eq!(t.workspace_id, Some(self.task.workspace_id));
        t
    }
}

/// An orchestrator adopted from a shell pane, running claude: told, as the
/// agent its process proves it is. Before ov-193 its launch preset kept it
/// from counting, and the task said "Nobody to tell".
#[tokio::test]
async fn a_hand_started_orchestrator_is_told() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    b.answer("Drill in");
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), [b.told("Drill in")], "{}", si.log());
    assert_eq!(b.progress(), ["Told the orchestrator about the decision"]);
}

/// The same agent in a shell pane nobody adopted is never typed into: not
/// even the only pane in the task's lane, and one whose role reads Agent.
/// Only the orchestrator's role stands in for an agent launch.
#[tokio::test]
async fn a_shell_pane_running_claude_by_hand_is_never_told() {
    let b = board().await;
    let task = b.svc.store.get_task(b.task.id).unwrap();
    let on_lane = farcooler_store::models::TaskUpdate {
        title: task.title.clone(),
        intent: task.intent.clone(),
        acceptance: task.acceptance.clone(),
        constraints: task.constraints.clone(),
        labels: task.labels.clone(),
        worktree_id: Some(b.lane.id),
    };
    b.svc.store.update_task(task.id, task.resource_version, &on_lane).unwrap();
    let shell = b.shell_pane().await;
    let shell = b.svc.set_terminal_role(shell.id, TerminalRole::Agent).await.unwrap();
    let si = b.stand_in(&shell, "claude", "claude").await;
    // Nor followed for bracketed paste where tmux can't report it: that
    // costs a pipe and a process, kept for panes that may be typed to.
    assert!(!b.svc.paste_mode_followed(shell.id).await);
    b.doing(shell.id, AgentActivity::Idle).await;
    b.answer("Drill in");
    b.pump().await;
    assert!(!si.log().contains("PASTE") && !si.log().contains("ENTER"), "{}", si.log());
    assert_eq!(b.progress(), [NOBODY]);
}

/// An adopted orchestrator with no agent running, only its shell: nothing
/// typed, the answer waits, and the task says whom it's waiting for.
#[tokio::test]
async fn an_adopted_orchestrator_at_its_shell_waits_and_says_so() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    b.screen_with(orchestrator.id, "").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    b.answer("Drill in");
    b.pump().await;
    b.pump().await;
    let pending = b.pending();
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert_eq!(pending[0].claimed_at, None, "claimed a shell");
    assert_eq!(b.progress(), ["Waiting to tell the orchestrator about the decision: no agent is running in its pane."]);
}

/// A pane launched as one agent and running another is never typed into,
/// whether it works a task or was adopted as the orchestrator: only a pane
/// launched as something that isn't an agent is read as what it runs.
#[tokio::test]
async fn a_pane_launched_as_codex_running_claude_is_never_told() {
    for adopt in [false, true] {
        let b = board().await;
        let pane = if adopt {
            let t = b.svc.create_terminal_with_prompt(b.lane.id, "Orchestrator", "codex", None, None).await.unwrap();
            b.svc.set_terminal_role(t.id, TerminalRole::Orchestrator).await.expect("adopted")
        } else {
            b.agent("Agent 2", "codex").await
        };
        let si = b.stand_in(&pane, "claude", "claude").await;
        b.doing(pane.id, AgentActivity::Idle).await;
        b.answer("Drill in");
        b.pump().await;
        b.untouched(&si);
        assert_eq!(b.give_up().await, ["Not delivered: no agent was running in the pane."], "adopted: {adopt}");
    }
}

/// The waiting note is said once per answer, not again by a restarted
/// daemon still waiting on it.
#[tokio::test]
async fn waiting_is_said_once_across_a_restart() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    b.screen_with(orchestrator.id, "").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    b.answer("Drill in");
    b.pump().await;
    let waiting = ["Waiting to tell the orchestrator about the decision: no agent is running in its pane."];
    assert_eq!(b.progress(), waiting);
    let Board { dir, svc, watcher, task, .. } = b;
    let root = svc.root_dir().to_path_buf();
    drop(watcher);
    drop(svc);
    for _ in 0..2 {
        let svc = Arc::new(Service::open_in(root.clone()).await.expect("the daemon again"));
        let watcher = Watcher::new(svc.clone());
        observe(&watcher, orchestrator.id, AgentActivity::Idle).await;
        watcher.pump_wakes().await;
        watcher.pump_wakes().await;
        assert_eq!(svc.store.pending_answer_wakes().unwrap().len(), 1);
        let said: Vec<String> =
            svc.store.notes_for(task.id, Some(NoteKind::Progress)).unwrap().into_iter().map(|n| n.body).collect();
        assert_eq!(said, waiting);
    }
    drop(dir);
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
    si.submits(1).await;
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
    si.submits(1).await;
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

#[path = "worker_tests.rs"]
mod worker_tests;

#[path = "lane_tests.rs"]
mod lane_tests;

#[path = "follow_race_tests.rs"]
mod follow_race_tests;

// ---- Ask the Orchestrator (ov-184): a draft is pasted and never sent ----

/// Nothing reached the pane: no paste and no Enter.
fn nothing_typed(si: &StandIn) {
    assert!(!si.log().contains("PASTE") && !si.log().contains("ENTER"), "{}", si.log());
}

/// What Ask the Orchestrator pastes can't carry a line break or a carriage
/// return, so the paste can't submit: control characters arrive as text, and
/// the trailing space that parks the cursor past the colon stays.
#[test]
fn a_draft_never_carries_a_line_break_or_ends_in_one() {
    for raw in ["About ov-1 (“A”): ", "About ov-1 (“A\r\nB”): \n", "About ov-1\r", "a\u{1b}[201~\rb\n"] {
        let text = draft_text(raw);
        assert!(!text.contains(['\n', '\r']), "{text:?}");
        assert!(!text.ends_with(['\n', '\r']), "{text:?}");
        assert!(!text.chars().any(char::is_control), "{text:?}");
    }
    assert_eq!(draft_text("About ov-1 (“A”): "), "About ov-1 (“A”): ");
}

/// An idle claude run by hand in the adopted orchestrator's pane gets the
/// reference pasted, and no Enter, so nothing is submitted.
#[tokio::test]
async fn a_draft_is_pasted_into_an_idle_orchestrator_and_never_submitted() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    b.watcher.draft_into(orchestrator.id, "About ov-1 (“Fix”): ", false).await.expect("pasted");
    si.pasted().await;
    assert!(si.log().contains("PASTE "), "a bracketed paste: {}", si.log());
    assert!(si.submitted().is_empty(), "Enter was pressed: {}", si.log());
}

/// Every check that fails closed for an answer fails for a draft too: an
/// error, and nothing typed, which is what sends the Mac to the clipboard.
#[tokio::test]
async fn a_draft_is_refused_where_an_answer_would_wait() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    // Its state unknown.
    b.doing(orchestrator.id, AgentActivity::Unknown).await;
    assert!(b.watcher.draft_into(orchestrator.id, "About ov-1: ", false).await.is_err());
    nothing_typed(&si);
    // A menu.
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    si.show("menu").await;
    b.screen_with(orchestrator.id, "Tab to amend").await;
    assert!(b.watcher.draft_into(orchestrator.id, "About ov-1: ", false).await.is_err());
    nothing_typed(&si);
    // Someone's draft in the box.
    si.show("draft:fix the flaky").await;
    b.screen_with(orchestrator.id, "fix the flaky").await;
    assert!(b.watcher.draft_into(orchestrator.id, "About ov-1: ", false).await.is_err());
    nothing_typed(&si);
}

/// A process that isn't an agent, drawing a perfect agent screen: refused.
#[tokio::test]
async fn a_draft_is_refused_for_a_process_that_is_not_an_agent() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "perl").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    assert!(b.watcher.draft_into(orchestrator.id, "About ov-1: ", false).await.is_err());
    nothing_typed(&si);
}

/// A send that fails is an error, so the Mac copies instead.
#[tokio::test]
async fn a_draft_whose_send_fails_is_an_error() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let _si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    b.watcher.fail_sends.store(true, Ordering::SeqCst);
    assert!(b.watcher.draft_into(orchestrator.id, "About ov-1: ", false).await.is_err());
}

#[path = "tell_tests.rs"]
mod tell_tests;

#[path = "mid_turn_tests.rs"]
mod mid_turn_tests;

#[path = "draft_hold_tests.rs"]
mod draft_hold_tests;

#[path = "compose_tests.rs"]
mod compose_tests;

#[path = "hidden_turn_tests.rs"]
mod hidden_turn_tests;
