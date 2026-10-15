//! A message to a codex pane goes through `codex queue` (ov-563), past every
//! box check, and is typed when that can't be. A stand-in `codex` script
//! records what it's run with; the real codex never runs. On the board and
//! tmux server of `tests`.

use std::path::PathBuf;

use super::codex_turn_tests::running_rollout;
use super::deliver::{CODEX_FOR_TESTS, CODEX_ON_FOR_TESTS};
use super::*;

const THREAD: &str = "01a117d0-fd5c-7fe3-8f0f-5f2afdf7906d";

/// A `codex` for `terminal` that logs its arguments and `CODEX_HOME`, runs
/// `after`, and exits `status`. Returns the log's path. The queue is off by
/// default (R-77): this opts the pane in, as `FARCOOLER_CODEX_QUEUE=1` does.
fn queue_stand_in(b: &Board, terminal: &Terminal, status: i32, after: &str, opted_in: bool) -> PathBuf {
    let script = b.dir.path().join(format!("codex-{status}-{}.sh", after.len()));
    let log = b.dir.path().join(format!("codex-{status}-{}.log", after.len()));
    std::fs::write(&script, format!("#!/bin/sh\necho \"$CODEX_HOME|$*\" >> '{}'\n{after}\nexit {status}\n", log.display())).unwrap();
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    CODEX_FOR_TESTS.lock().unwrap().push((terminal.id, script));
    if opted_in {
        CODEX_ON_FOR_TESTS.lock().unwrap().push(terminal.id);
    }
    log
}

fn stand_in_codex(b: &Board, terminal: &Terminal, status: i32) -> PathBuf {
    queue_stand_in(b, terminal, status, "", true)
}

/// Mid-turn, where typing waits for codex to be free, the message is queued
/// as the next turn, into the thread the pane holds, with the pane's home.
#[tokio::test]
async fn a_message_to_a_working_codex_is_queued_and_settled() {
    let b = board().await;
    let agent = b.agent("Agent 2", "codex").await;
    let (rollout, _end) = running_rollout(&b);
    let si = b.stand_in_with(&agent, "codex", "codex", &format!("STAND_IN_HOLD='{}'", rollout.display())).await;
    b.doing(agent.id, AgentActivity::Working).await;
    let log = stand_in_codex(&b, &agent, 0);
    b.send("manager", &b.task.key, "Rebase on main first").await.unwrap();
    b.pump().await;
    let ran = std::fs::read_to_string(&log).unwrap_or_default();
    let home = b.dir.path().canonicalize().unwrap().join(".codex");
    assert_eq!(
        ran.trim(),
        format!("{}|queue --thread {THREAD} --message=[from the orchestrator] Rebase on main first", home.display()),
        "{}",
        si.log()
    );
    assert!(b.messages().is_empty(), "settled: {:?}", b.messages());
    assert!(!si.log().contains("PASTE") && !si.log().contains("ENTER"), "{}", si.log());
}

/// A `codex queue` that fails leaves the message to be typed.
#[tokio::test]
async fn a_failed_queue_falls_back_to_typing() {
    let b = board().await;
    let agent = b.agent("Agent 2", "codex").await;
    let (rollout, end) = running_rollout(&b);
    let si = b.stand_in_with(&agent, "codex", "codex", &format!("STAND_IN_HOLD='{}'", rollout.display())).await;
    b.doing(agent.id, AgentActivity::Idle).await;
    let log = stand_in_codex(&b, &agent, 1);
    b.send("manager", &b.task.key, "Ship it").await.unwrap();
    // Idle on the screen, a turn running in the rollout: it ends first.
    let mut file = std::fs::OpenOptions::new().append(true).open(&rollout).unwrap();
    std::io::Write::write_all(&mut file, format!("{end}\n").as_bytes()).unwrap();
    b.pump().await;
    si.submits(1).await;
    assert!(std::fs::read_to_string(&log).unwrap().contains("queue --thread"), "tried the queue");
    assert_eq!(si.submitted(), ["[from the orchestrator] Ship it"], "{}", si.log());
}

/// A pane whose rollout names no thread (its head hasn't said) is typed to,
/// with the queue opted in and a `codex` to run.
#[tokio::test]
async fn an_unknown_thread_is_typed() {
    let b = board().await;
    let agent = b.agent("Agent 2", "codex").await;
    let (rollout, _end) = running_rollout(&b);
    // The head says nothing yet; the turn is closed, so the screen is all
    // there is to go on.
    std::fs::write(&rollout, "{\"type\":\"event_msg\"}\n").unwrap();
    let si = b.stand_in_with(&agent, "codex", "codex", &format!("STAND_IN_HOLD='{}'", rollout.display())).await;
    b.doing(agent.id, AgentActivity::Idle).await;
    let log = stand_in_codex(&b, &agent, 0);
    b.send("manager", &b.task.key, "Ship it").await.unwrap();
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), ["[from the orchestrator] Ship it"], "{}", si.log());
    assert!(!log.exists(), "no thread to queue to: {}", std::fs::read_to_string(&log).unwrap_or_default());
}

/// Off by default (ruling R-77): a codex pane with a `codex` to run and a
/// known thread is still typed to until the queue is opted into.
#[tokio::test]
async fn the_codex_queue_is_off_unless_opted_into() {
    let b = board().await;
    let agent = b.agent("Agent 2", "codex").await;
    let (rollout, end) = running_rollout(&b);
    let si = b.stand_in_with(&agent, "codex", "codex", &format!("STAND_IN_HOLD='{}'", rollout.display())).await;
    b.doing(agent.id, AgentActivity::Idle).await;
    let log = queue_stand_in(&b, &agent, 0, "", false);
    b.send("manager", &b.task.key, "Ship it").await.unwrap();
    let mut file = std::fs::OpenOptions::new().append(true).open(&rollout).unwrap();
    std::io::Write::write_all(&mut file, format!("{end}\n").as_bytes()).unwrap();
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), ["[from the orchestrator] Ship it"], "{}", si.log());
    assert!(!log.exists(), "queued without the opt-in");
}

/// A `codex queue` that failed is not tried again for the same wake: the
/// message waits to be typed (the pane is working), and the command is run
/// once.
#[tokio::test]
async fn a_failed_queue_is_not_retried_on_every_pass() {
    let b = board().await;
    let agent = b.agent("Agent 2", "codex").await;
    let (rollout, _end) = running_rollout(&b);
    let si = b.stand_in_with(&agent, "codex", "codex", &format!("STAND_IN_HOLD='{}'", rollout.display())).await;
    b.doing(agent.id, AgentActivity::Working).await;
    let log = stand_in_codex(&b, &agent, 1);
    b.send("manager", &b.task.key, "Ship it").await.unwrap();
    b.pump().await;
    b.pump().await;
    b.pump().await;
    let runs = std::fs::read_to_string(&log).unwrap().lines().count();
    assert_eq!(runs, 1, "{}", si.log());
    assert_eq!(b.messages().len(), 1, "and still waiting to be typed");
}

/// A `codex queue` that never returns is given up on after its patience,
/// and the message is typed.
#[tokio::test]
async fn a_stuck_queue_is_given_up_on() {
    let b = board().await;
    let agent = b.agent("Agent 2", "codex").await;
    let (rollout, end) = running_rollout(&b);
    let si = b.stand_in_with(&agent, "codex", "codex", &format!("STAND_IN_HOLD='{}'", rollout.display())).await;
    b.doing(agent.id, AgentActivity::Idle).await;
    let log = queue_stand_in(&b, &agent, 0, "sleep 120", true);
    b.send("manager", &b.task.key, "Ship it").await.unwrap();
    let mut file = std::fs::OpenOptions::new().append(true).open(&rollout).unwrap();
    std::io::Write::write_all(&mut file, format!("{end}\n").as_bytes()).unwrap();
    b.pump().await;
    si.submits(1).await;
    assert!(log.exists(), "tried");
    assert_eq!(si.submitted(), ["[from the orchestrator] Ship it"], "{}", si.log());
}

/// The thread is the person's session's, never a subagent's, and only a bare
/// uuid becomes an argument.
#[test]
fn only_a_main_thread_with_a_plain_id_is_a_target() {
    let dir = tempfile::tempdir().unwrap();
    let rollout = dir.path().join("h/sessions/2026/10/07/rollout-x.jsonl");
    std::fs::create_dir_all(rollout.parent().unwrap()).unwrap();
    let meta = |id: &str, source: &str| format!(r#"{{"type":"session_meta","payload":{{"id":"{id}","source":{source}}}}}"#);
    let read = |text: String| {
        std::fs::write(&rollout, text + "\n").unwrap();
        crate::log_join::codex_thread_of(&rollout)
    };
    assert_eq!(read(meta(THREAD, r#""cli""#)), Some((THREAD.into(), dir.path().join("h"))));
    assert_eq!(read(meta(THREAD, r#"{"subagent":{"other":"guardian"}}"#)), None, "a guardian's");
    assert_eq!(read(meta("--thread x", r#""cli""#)), None, "not an id");
    let elsewhere = dir.path().join("rollout-y.jsonl");
    std::fs::write(&elsewhere, meta(THREAD, r#""cli""#) + "\n").unwrap();
    assert_eq!(crate::log_join::codex_thread_of(&elsewhere), None, "no sessions directory");
}
