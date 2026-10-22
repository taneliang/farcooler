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
        let si = StandIn::new(dir.join("control"), dir.join("log")).in_pane(&self.svc, pane.id);
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

    /// Sample until the watcher reads `pane` as on the trust prompt.
    async fn offered(&self, pane: Uuid) -> bool {
        for _ in 0..50 {
            self.watcher.sample().await;
            if self.on_trust_prompt(pane).await {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        false
    }

    /// `terminal.agent_answer` for the trust prompt, through the RPC, from
    /// a local caller naming `actor`.
    async fn answer_over_rpc(&self, pane: Uuid, ask: &str, actor: &str) -> Option<pb::response::Outcome> {
        let factory = crate::rpc::RpcFactory::new(
            self.svc.clone(),
            self.watcher.clone(),
            Arc::new(tokio::sync::Notify::new()),
            farcooler_transport::Peer { client_id: None, scope: pb::Scope::Control },
        );
        let req = pb::Request {
            method: "terminal.agent_answer".into(),
            payload: Some(pb::request::Payload::AgentAnswer(pb::AgentAnswer {
                terminal_id: crate::wire::id_bytes(pane),
                request_id: ask.into(),
                option_id: TRUST_OPTION.into(),
                actor: actor.into(),
                ..Default::default()
            })),
            ..Default::default()
        };
        farcooler_transport::Handler::handle(&factory, req).await.outcome
    }

    /// Whether Needs You offers Trust Folder for `pane`.
    async fn listed_as_trust(&self, pane: Uuid) -> bool {
        let id = crate::wire::id_bytes(pane);
        self.needs_you().await.iter().any(|i| {
            i.terminal.as_ref().is_some_and(|t| t.id == id) && i.ask_id.as_deref().is_some_and(|a| a.starts_with(TRUST_ASK_PREFIX))
        })
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
    assert_eq!(item.question, crate::needs_you::TRUST_LINE);
    let folder = "/private/tmp/claude-502/-Users-e-liang-Dev-overnight/9dd767ba-acbf-4124-87b3-c7d8662f60e7/scratchpad/probe";
    assert_eq!(item.detail.as_deref(), Some(folder), "the row shows what a tap grants");
    let ask = item.ask_id.clone().expect("an ask id");
    assert!(ask.starts_with(TRUST_ASK_PREFIX), "{ask}");
    assert_eq!(item.actions.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), [TRUST_OPTION]);

    assert_eq!(refused(b.watcher.answer_trust(pane.id, &ask, "deny", "user").await), "option_id");
    assert_eq!(refused(b.watcher.answer_trust(pane.id, &format!("{ask}0"), TRUST_OPTION, "user").await), "not_held");
    assert!(!si.log().contains("ENTER"), "a refused answer pressed a key: {}", si.log());

    b.watcher.answer_trust(pane.id, &ask, TRUST_OPTION, "user").await.expect("trusted");
    assert_eq!(si.log().matches("TRUSTED").count(), 1, "{}", si.log());
    assert_eq!(refused(b.watcher.answer_trust(pane.id, &ask, TRUST_OPTION, "user").await), "too_soon");

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
    // The prompt answered, the orchestrator hears nothing more: not a
    // "stopped on a prompt" for the tick the fold still read Blocked.
    for _ in 0..5 {
        b.watcher.sample().await;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(b.notices().len(), 1, "{:?}", b.notices());
}

/// Only a person's client answers the trust prompt: an agent, the
/// orchestrator, a shell in a runner's pane or a caller that names nobody is
/// refused, over the RPC as well, and nothing is pressed.
#[tokio::test]
async fn an_agent_cant_answer_the_trust_prompt() {
    let b = board().await;
    let (pane, si) = b.lane_on_trust_prompt("guard").await;
    assert!(b.offered(pane.id).await, "never read as the trust prompt");
    let ask = crate::needs_you::trust_ask_id(pane.id, b.watcher.activity(pane.id).await.1.unwrap());
    for actor in [format!("agent:{}", pane.id), "manager".into(), "pane".into()] {
        assert_eq!(refused(b.watcher.answer_trust(pane.id, &ask, TRUST_OPTION, &actor).await), "not_a_person", "{actor:?}");
    }
    let outcome = b.answer_over_rpc(pane.id, &ask, &format!("agent:{}", pane.id)).await;
    let Some(pb::response::Outcome::Error(e)) = outcome else { panic!("{outcome:?}") };
    assert_eq!(e.what, "not_a_person");
    assert!(!si.log().contains("ENTER"), "{}", si.log());
}

/// An app from before `AgentAnswer.actor` sends none: its caller is a
/// person, and Trust Folder works from it.
#[tokio::test]
async fn an_older_app_with_no_actor_answers_the_trust_prompt() {
    let b = board().await;
    let (pane, si) = b.lane_on_trust_prompt("old-app").await;
    assert!(b.offered(pane.id).await, "never read as the trust prompt");
    let ask = crate::needs_you::trust_ask_id(pane.id, b.watcher.activity(pane.id).await.1.unwrap());
    let outcome = b.answer_over_rpc(pane.id, &ask, "").await;
    assert!(matches!(outcome, Some(pb::response::Outcome::Result(_))), "{outcome:?}");
    assert_eq!(si.log().matches("TRUSTED").count(), 1, "{}", si.log());
}

/// A permission dialog whose command quotes the trust prompt, in place of
/// the prompt the item was made for, is never answered by Trust Folder.
#[tokio::test]
async fn trust_folder_never_answers_a_permission_dialog() {
    let b = board().await;
    let (pane, si) = b.lane_on_trust_prompt("lookalike").await;
    assert!(b.offered(pane.id).await, "never read as the trust prompt");
    let ask = crate::needs_you::trust_ask_id(pane.id, b.watcher.activity(pane.id).await.1.unwrap());
    si.show("trust-menu").await;
    b.screen_with(pane.id, "Tab to amend").await;
    assert_eq!(refused(b.watcher.answer_trust(pane.id, &ask, TRUST_OPTION, "user").await), "not_held");
    b.watcher.sample().await;
    assert!(!b.on_trust_prompt(pane.id).await, "the dialog read as the trust prompt");
    assert!(!b.listed_as_trust(pane.id).await, "Needs You offers Trust Folder for it");
    assert!(!si.log().contains("ENTER"), "{}", si.log());
}

/// A notice for a lane that hasn't started isn't dropped behind one of the
/// runner's still waiting for the orchestrator: the two are told as one.
#[tokio::test]
async fn a_not_started_notice_folds_into_one_still_waiting() {
    let b = board().await;
    let orchestrator = b.orchestrator().await;
    let _osi = b.stand_in(&orchestrator, "claude", "claude").await;
    let earlier = b.svc.store.send_message(b.task.id, Actor::Runner, "The card's last agent stopped.", None, serde_json::json!({})).unwrap();
    messages::hold_for_tests(earlier.id);
    let (_pane, _si) = b.lane_on_trust_prompt("again").await;
    let mut told = Vec::new();
    for _ in 0..50 {
        b.watcher.sample().await;
        told = b.svc.store.pending_message_wakes().unwrap().into_iter().map(|w| w.body).collect();
        if told.iter().any(|t| t.contains("hasn't started")) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    messages::release_for_tests(earlier.id);
    assert_eq!(told.len(), 1, "{told:?}");
    assert!(told[0].contains("hasn't started: Claude Code is asking") && told[0].ends_with("Before that: The card's last agent stopped."), "{told:?}");
}

/// The trust answer comes through `terminal.agent_answer`, the path every
/// app's Needs You row already takes for an ask.
#[tokio::test]
async fn the_trust_answer_is_terminal_agent_answer() {
    let b = board().await;
    let (pane, si) = b.lane_on_trust_prompt("site").await;
    assert!(b.offered(pane.id).await, "never read as the trust prompt");
    let since = b.watcher.activity(pane.id).await.1.unwrap();
    let outcome = b.answer_over_rpc(pane.id, &crate::needs_you::trust_ask_id(pane.id, since), "user").await;
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
    let told = tokio::time::timeout(Duration::from_secs(60), async {
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
    assert!(said.contains("hasn't begun its first turn in the 60 seconds since it opened"), "{said}");
    let short = &pane.id.simple().to_string()[24..];
    assert!(said.contains(&format!("`farcooler terminal screen {short}`")), "{said}");
    let items = b.needs_you().await;
    let line = crate::needs_you::not_started_line(&NotStarted::NoTurn).unwrap();
    assert!(items.iter().any(|i| i.question == line), "{items:#?}");
}

fn tick(activity: AgentActivity, now: i64) -> Tick<'static> {
    Tick { activity, sample: activity, turn_logged: false, ended: false, trust: false, question: None, now }
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
    // Answered: the fold still reads Blocked for a tick, but the sample
    // doesn't, so nothing is reported; and the window starts over.
    let answered = Tick { sample: AgentActivity::Working, ..tick(AgentActivity::Blocked, START_WINDOW_MS + 5_000) };
    assert_eq!(start.step(answered), Step::Cleared);
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
    let mut done = Start::Waiting { since: 0 };
    assert_eq!(done.step(tick(AgentActivity::Done, 1_000)), Step::Quiet);
    assert!(done.watching(), "Done with no turn logged isn't a start: Blocked then Idle folds to it");
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
