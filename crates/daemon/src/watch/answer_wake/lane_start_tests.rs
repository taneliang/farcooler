//! A dispatched agent is seen to start (ov-493), on the board and tmux
//! server of `tests`, with `stand_in.pl` drawing claude's folder-trust
//! prompt as `crates/core/captures/claude-trust-gate.txt` holds it.

use super::*;
use crate::needs_you::{NotStarted, TRUST_ASK_PREFIX, TRUST_OPTION};
use crate::watch::lane_start::{START_WINDOW_MS, Start, Step, Tick};
use farcooler_store::plan::{LaneCard, NewLane};

const GATE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../core/captures/claude-trust-gate.txt");

impl Board {
    /// A lane named `name` holding the task, worked by a claude pane opened
    /// for it as a lane dispatch opens one, which then shows claude's trust
    /// prompt before any sample has seen it: what a new worktree's first
    /// launch draws.
    async fn lane_on_trust_prompt(&self, name: &str) -> (Terminal, StandIn) {
        let cards = [LaneCard { task_id: self.task.id, slice: String::new() }];
        let new = NewLane { name: name.into(), ..Default::default() };
        self.svc.store.create_lane(self.task.workspace_id, &new, &cards, None, Actor::Manager).unwrap();
        let pane = self.svc.create_terminal_in_lane(self.lane.id, "Lane", "claude", None, Some(self.task.id), Some(name)).await.unwrap();
        let dir = self.dir.path().join(format!("si-{}", pane.id.simple()));
        std::fs::create_dir_all(dir.join("config")).unwrap();
        let program = dir.join("claude");
        copy_program(&farcooler_core::programs::find("perl").expect("perl"), &program);
        std::fs::write(dir.join("stand_in.pl"), STAND_IN).unwrap();
        let si = StandIn { control: dir.join("control"), log: dir.join("log") };
        std::fs::write(&si.control, "trust").unwrap();
        let q = |p: &std::path::Path| format!("'{}'", p.display());
        let command = format!(
            "env STAND_IN_TRUST='{GATE}' CLAUDE_CONFIG_DIR={} {} {} claude {} {}",
            q(&dir.join("config")),
            q(&program),
            q(&dir.join("stand_in.pl")),
            q(&si.control),
            q(&si.log)
        );
        self.run_unfollowed(&pane, &command).await;
        let screen = self.screen_with(pane.id, "Yes, I trust this folder").await;
        assert!(farcooler_core::activity::trust::yes_selected(&screen), "{screen}");
        (pane, si)
    }

    /// Sample until a notice is filed, up to about ten seconds.
    async fn sample_until_told(&self) -> bool {
        for _ in 0..50 {
            self.watcher.sample().await;
            if !self.notices().is_empty() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        false
    }

    async fn needs_you(&self) -> Vec<pb::NeedsYouItem> {
        let inputs = crate::needs_you::gather(&self.svc, &self.watcher).await.unwrap();
        crate::needs_you::assemble(&inputs, std::time::SystemTime::now())
    }

    /// The runner's notices filed for the orchestrator, told or not.
    fn notices(&self) -> Vec<String> {
        let notes = self.svc.store.notes_for(self.task.id, None).unwrap();
        notes.into_iter().filter(|n| n.actor == Actor::Runner && n.extra.get("message").is_some()).map(|n| n.body).collect()
    }

    /// Whether the watcher reads `pane` as on claude's trust prompt.
    async fn on_trust_prompt(&self, pane: Uuid) -> bool {
        self.watcher.state.lock().await.get(&pane).is_some_and(|o| o.trust_prompt && o.activity == AgentActivity::Blocked)
    }

    /// Whether Needs You has an item for `pane`.
    async fn listed(&self, pane: Uuid) -> bool {
        let id = crate::wire::id_bytes(pane);
        self.needs_you().await.iter().any(|i| i.terminal.as_ref().is_some_and(|t| t.id == id))
    }
}

fn refused(r: Result<()>) -> &'static str {
    match r {
        Err(DomainError::Conflict { what } | DomainError::InvalidArgument { what }) => what,
        other => panic!("not refused: {other:?}"),
    }
}

/// A lane's agent that opens on claude's trust prompt is told to its
/// orchestrator at once, with the reason, and shows in Needs You as an ask
/// whose one answer, Trust Folder, presses Enter there and nothing else.
/// The runner never presses it itself.
#[tokio::test]
async fn a_lane_on_the_trust_prompt_is_told_and_answered_from_needs_you() {
    let b = board().await;
    let orchestrator = b.orchestrator().await;
    let _osi = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    let (pane, si) = b.lane_on_trust_prompt("cleanup").await;

    assert!(b.sample_until_told().await, "the orchestrator was never told");
    let key = &b.task.key;
    assert_eq!(
        b.notices(),
        [format!(
            "The lane cleanup's agent, on {key}, hasn't started: Claude Code is asking whether to trust its \
             folder. That's the owner's call, never an agent's: tell them it's waiting in Needs You."
        )]
    );
    for _ in 0..5 {
        b.watcher.sample().await;
    }
    assert_eq!(b.notices().len(), 1, "told once");
    assert!(!si.log().contains("ENTER"), "the runner answered the prompt itself: {}", si.log());

    let items = b.needs_you().await;
    let id = crate::wire::id_bytes(pane.id);
    let item = items.iter().find(|i| i.terminal.as_ref().is_some_and(|t| t.id == id)).expect("an item");
    assert_eq!(item.kind(), pb::NeedsYouKind::Ask, "{item:#?}");
    assert_eq!(item.question, "Trust this folder?");
    let ask = item.ask_id.clone().expect("an ask id");
    assert!(ask.starts_with(TRUST_ASK_PREFIX), "{ask}");
    assert_eq!(item.actions.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), [TRUST_OPTION]);

    assert_eq!(refused(b.watcher.answer_trust(pane.id, &ask, "deny").await), "option_id");
    assert_eq!(refused(b.watcher.answer_trust(pane.id, &format!("{ask}0"), TRUST_OPTION).await), "not_held");
    assert!(!si.log().contains("ENTER"), "a refused answer pressed a key: {}", si.log());

    b.watcher.answer_trust(pane.id, &ask, TRUST_OPTION).await.expect("trusted");
    assert_eq!(si.log().matches("TRUSTED").count(), 1, "{}", si.log());
    assert_eq!(refused(b.watcher.answer_trust(pane.id, &ask, TRUST_OPTION).await), "too_soon");

    // claude starts its turn: the item goes, and the lane counts as started.
    let mut gone = false;
    for _ in 0..30 {
        b.watcher.sample().await;
        if !b.listed(pane.id).await {
            gone = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert!(gone, "the item stayed after the folder was trusted");
    assert_eq!(si.log().matches("ENTER").count(), 1, "{}", si.log());
}

/// The trust answer comes through `terminal.agent_answer`, the path every
/// app's Needs You row already takes for an ask.
#[tokio::test]
async fn the_trust_answer_is_terminal_agent_answer() {
    let b = board().await;
    let (pane, si) = b.lane_on_trust_prompt("site").await;
    let mut offered = false;
    for _ in 0..50 {
        b.watcher.sample().await;
        if b.on_trust_prompt(pane.id).await {
            offered = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert!(offered, "never read as the trust prompt");
    let since = b.watcher.activity(pane.id).await.1.unwrap();
    let factory = crate::rpc::RpcFactory::new(
        b.svc.clone(),
        b.watcher.clone(),
        Arc::new(tokio::sync::Notify::new()),
        farcooler_transport::Peer { client_id: None, scope: pb::Scope::Control },
    );
    let req = pb::Request {
        method: "terminal.agent_answer".into(),
        payload: Some(pb::request::Payload::AgentAnswer(pb::AgentAnswer {
            terminal_id: crate::wire::id_bytes(pane.id),
            request_id: crate::needs_you::trust_ask_id(pane.id, since),
            option_id: TRUST_OPTION.into(),
            ..Default::default()
        })),
        ..Default::default()
    };
    let outcome = farcooler_transport::Handler::handle(&factory, req).await.outcome;
    assert!(matches!(outcome, Some(pb::response::Outcome::Result(_))), "{outcome:?}");
    assert_eq!(si.log().matches("TRUSTED").count(), 1, "{}", si.log());
}

/// A prompt already up when a new pane is first seen is a transition, as
/// one drawn a tick later is: the phone hears of it.
#[tokio::test]
async fn a_new_pane_first_seen_on_a_prompt_is_announced() {
    let b = board().await;
    let mut taps = b.watcher.tap_notices();
    let (pane, _si) = b.lane_on_trust_prompt("first").await;
    b.watcher.sample().await;
    let key = b.task.key.clone();
    let told = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(t) = taps.recv().await {
            if t.terminal == Some(pane.id) || t.task.as_deref() == Some(key.as_str()) {
                return true;
            }
        }
        false
    })
    .await;
    assert_eq!(told, Ok(true), "no notice for a pane first seen on its trust prompt");
}

/// An agent that never begins a turn, and asks nothing, is reported once
/// the window is out, with how to look at it.
#[tokio::test]
async fn an_agent_that_never_starts_is_reported_after_the_window() {
    let b = board().await;
    let orchestrator = b.orchestrator().await;
    let _osi = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    let (pane, si) = b.lane_on_trust_prompt("quiet").await;
    si.show("idle").await;
    b.watcher.sample().await;
    b.watcher.state.lock().await.get_mut(&pane.id).unwrap().start = Start::Waiting { since: now_millis() - START_WINDOW_MS };
    assert!(b.sample_until_told().await, "never told");
    let said = &b.notices()[0];
    assert!(said.contains("hasn't begun its first turn 60 seconds after it opened"), "{said}");
    let short = &pane.id.simple().to_string()[24..];
    assert!(said.contains(&format!("`farcooler terminal screen {short}`")), "{said}");
    let items = b.needs_you().await;
    assert!(items.iter().any(|i| i.question == crate::needs_you::NOT_STARTED), "{items:#?}");
}

fn tick(activity: AgentActivity, now: i64) -> Tick<'static> {
    Tick { activity, turn_logged: false, ended: false, trust: false, question: None, now }
}

/// The way to a first turn, one tick at a time: a prompt is reported once,
/// a new reason again, a cleared one starts the window over, and a first
/// turn ends the watch.
#[test]
fn a_start_is_reported_once_per_reason() {
    let mut start = Start::Waiting { since: 0 };
    assert_eq!(start.step(tick(AgentActivity::Idle, 1_000)), Step::Quiet);
    let trust = Tick { trust: true, ..tick(AgentActivity::Blocked, 2_000) };
    assert_eq!(start.step(trust), Step::Report(NotStarted::TrustPrompt));
    let again = Tick { trust: true, ..tick(AgentActivity::Blocked, 3_000) };
    assert_eq!(start.step(again), Step::Quiet, "once");
    assert_eq!(start.not_started(), Some(NotStarted::TrustPrompt));
    // Answered: the window starts over, so a moment of idle isn't a report.
    assert_eq!(start.step(tick(AgentActivity::Idle, START_WINDOW_MS + 5_000)), Step::Cleared);
    assert_eq!(start.step(tick(AgentActivity::Idle, START_WINDOW_MS + 6_000)), Step::Quiet);
    assert_eq!(start.step(tick(AgentActivity::Idle, 2 * START_WINDOW_MS + 5_000)), Step::Report(NotStarted::NoTurn));
    let exited = Tick { ended: true, ..tick(AgentActivity::None, 2 * START_WINDOW_MS + 6_000) };
    assert_eq!(start.step(exited), Step::Report(NotStarted::Exited), "a new reason is told");
    let mut working = Start::Reported { since: 0, why: NotStarted::NoTurn };
    assert_eq!(working.step(tick(AgentActivity::Working, 70_000)), Step::Cleared);
    assert_eq!(working, Start::Started);
    assert_eq!(working.step(tick(AgentActivity::Blocked, 80_000)), Step::Quiet, "started is done watching");
    let mut logged = Start::Waiting { since: 0 };
    assert_eq!(logged.step(Tick { turn_logged: true, ..tick(AgentActivity::Idle, 90_000) }), Step::Quiet);
    assert_eq!(logged, Start::Started, "a turn in its log is a start");
}

/// A pane made before the window, as at a restart, isn't watched; one made
/// now is.
#[test]
fn only_a_new_pane_is_watched() {
    let now = now_millis();
    assert!(Start::first_seen(Uuid::now_v7(), now).watching());
    assert!(!Start::first_seen(Uuid::now_v7(), now + START_WINDOW_MS + 1).watching());
    assert!(!Start::first_seen(Uuid::from_u128(7), now).watching(), "no time in it");
}
