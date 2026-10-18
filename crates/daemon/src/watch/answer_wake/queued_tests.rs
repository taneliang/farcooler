//! A person's plain text by codex's queue where typing would be refused, and
//! typed everywhere else (ov-627; `queued`). A stand-in `codex` logs what it's
//! run with; the real codex never runs. On the board and tmux server of
//! `tests`.

use super::codex_turn_tests::running_rollout;
use super::deliver_tests::queue_stand_in;
use super::route::{Delivered, Delivery, Part};
use super::*;

const THREAD: &str = "01a117d0-fd5c-7fe3-8f0f-5f2afdf7906d";

/// A codex pane mid-turn, as its rollout says, with a stand-in `codex` to
/// queue through: the pane, its stand-in, and the queue's log.
async fn working_codex(b: &Board) -> (Terminal, StandIn, std::path::PathBuf) {
    let agent = b.agent("Agent 2", "codex").await;
    let (rollout, _end) = running_rollout(b);
    let si = b.stand_in_with(&agent, "codex", "codex", &format!("STAND_IN_HOLD='{}'", rollout.display())).await;
    b.doing(agent.id, AgentActivity::Working).await;
    let log = queue_stand_in(b, &agent, 0, "", true);
    (agent, si, log)
}

/// A person's message to a busy codex goes by its queue, unwrapped, the same
/// role:user turn as typing. Round 5 held it until the turn ended.
#[tokio::test]
async fn a_persons_message_to_a_busy_codex_goes_by_the_queue() {
    let b = board().await;
    let (_agent, si, log) = working_codex(&b).await;
    b.send("user", &b.task.key, "Rename foo to bar").await.unwrap();
    b.pump().await;
    let ran = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(ran.contains(&format!("queue --thread {THREAD} --message=[from the terminal] Rename foo to bar")), "{ran}");
    assert!(b.messages().is_empty(), "settled: {:?}", b.messages());
    assert!(!si.log().contains("PASTE"), "{}", si.log());
}

/// A person's message to an idle codex is typed, never queued.
#[tokio::test]
async fn a_persons_message_to_an_idle_codex_is_typed() {
    let b = board().await;
    let agent = b.agent("Agent 2", "codex").await;
    let (rollout, end) = running_rollout(&b);
    let si = b.stand_in_with(&agent, "codex", "codex", &format!("STAND_IN_HOLD='{}'", rollout.display())).await;
    b.doing(agent.id, AgentActivity::Idle).await;
    let log = queue_stand_in(&b, &agent, 0, "", true);
    let mut file = std::fs::OpenOptions::new().append(true).open(&rollout).unwrap();
    std::io::Write::write_all(&mut file, format!("{end}\n").as_bytes()).unwrap();
    b.send("user", &b.task.key, "Rename foo to bar").await.unwrap();
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), ["[from the terminal] Rename foo to bar"], "{}", si.log());
    assert!(!log.exists(), "queued: {}", std::fs::read_to_string(&log).unwrap_or_default());
}

/// Live, a person's plain text to a busy codex goes by the queue and is
/// answered Queued; an image or a slash command is never queued, and keeps
/// compose's refusal. Round 5 refused all three as `busy`.
#[tokio::test]
async fn a_live_plain_message_to_a_busy_codex_is_queued_and_nothing_else_is() {
    let b = board().await;
    let (agent, si, log) = working_codex(&b).await;
    let image = Delivery { parts: vec![Part::Image("image/png".into(), vec![0x89, b'P', b'N', b'G'])], ..Delivery::prompt("look") };
    assert!(b.watcher.deliver(agent.id, image).await.is_err(), "an image is typed or refused");
    assert!(b.watcher.deliver(agent.id, Delivery::prompt("/review")).await.is_err(), "a slash command is typed or refused");
    assert!(!log.exists(), "queued: {}", std::fs::read_to_string(&log).unwrap_or_default());
    let said = b.watcher.deliver(agent.id, Delivery::prompt("fix the flaky test")).await;
    assert!(matches!(said, Ok(Delivered::Queued)), "{said:?}");
    let ran = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(ran.contains(&format!("queue --thread {THREAD} --message=fix the flaky test")), "{ran}");
    assert!(!si.log().contains("PASTE"), "{}", si.log());
}

/// A person's message to an idle codex with a draft in its box goes by the
/// queue, and the draft stays (measured, ov-627). Round 5 held it.
#[tokio::test]
async fn a_persons_message_to_codex_behind_a_draft_goes_by_the_queue() {
    let b = board().await;
    let agent = b.agent("Agent 2", "codex").await;
    let (rollout, end) = running_rollout(&b);
    let si = b.stand_in_with(&agent, "codex", "codex", &format!("STAND_IN_HOLD='{}'", rollout.display())).await;
    let mut file = std::fs::OpenOptions::new().append(true).open(&rollout).unwrap();
    std::io::Write::write_all(&mut file, format!("{end}\n").as_bytes()).unwrap();
    si.show("draft:half a thought").await;
    b.screen_with(agent.id, "half a thought").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    let log = queue_stand_in(&b, &agent, 0, "", true);
    b.send("user", &b.task.key, "Rename foo to bar").await.unwrap();
    b.pump().await;
    let ran = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(ran.contains(&format!("queue --thread {THREAD} --message=[from the terminal] Rename foo to bar")), "{ran}");
    assert!(!si.log().contains("PASTE"), "{}", si.log());
}

/// A person's message the codex queue took but codex never shows run is
/// withdrawn on the pane's rows, and its card, which said it was told, says
/// so too, in the same words (re-review 5).
#[tokio::test]
async fn a_queued_message_never_seen_run_is_withdrawn_on_its_card_too() {
    let b = board().await;
    b.watcher.with_timing(|t| t.queue_watch = crate::watch::Timing::SHORT);
    let (_agent, _si, log) = working_codex(&b).await;
    b.answer("Drill in");
    b.pump().await;
    assert!(std::fs::read_to_string(&log).unwrap_or_default().contains("queue --thread"), "queued");
    let why = super::compose::held::why_not_sent("Codex", &b.told("Drill in"), "queued_unseen");
    let since = std::time::Instant::now();
    while !b.progress().contains(&why) {
        assert!(since.elapsed() < Duration::from_secs(30), "never said: {:?}", b.progress());
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(b.progress().iter().any(|n| n.starts_with("Told Agent 2 about the decision")), "{:?}", b.progress());
}
