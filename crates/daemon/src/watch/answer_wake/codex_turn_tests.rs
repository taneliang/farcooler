//! codex's rollout as the gate's check that a turn runs (ov-378): a codex
//! stand-in whose screen reads between turns, holding open a real rollout
//! (codex-cli 0.153.4, sandbox) cut where a turn is running. On the board
//! and tmux server of `tests`.

use std::path::PathBuf;

use super::*;
use crate::watch::answer_wake::codex_turn::said;
use crate::watch::answer_wake::registry_turn::Said;

const ROLLOUT: &str = include_str!(
    "../../../../core/fixtures/session-logs/codex-tui-0.153.4/rollout-2026-10-07T12-22-14-01a117d0-fd5c-7fe3-8f0f-5f2afdf7906d.jsonl"
);

/// A codex killed mid-turn and then resumed (`codex resume`): its last
/// turn's `task_started` with nothing after it, as 0.153.4 left it.
const RESUMED: &str = include_str!(
    "../../../../core/fixtures/session-logs/codex-tui-0.153.4/rollout-2026-10-07T13-15-48-01a11802-07e1-7ca3-bfce-b553cb29fa89.jsonl"
);

/// A turn's start dated after any process a test starts: the turn is this
/// process's own, not a dead one's (`said`'s `started`).
fn dated_now(line: &str) -> String {
    let at = line.find(r#""timestamp":""#).unwrap() + r#""timestamp":""#.len();
    format!("{}2099-01-01T00:00:00.000Z{}", &line[..at], &line[at + "2026-10-07T19:28:23.057Z".len()..])
}

fn is(line: &str, kind: &str) -> bool {
    line.contains(r#""type":"event_msg""#) && line.contains(&format!(r#""payload":{{"type":"{kind}""#))
}

/// The rollout up to its last turn's start, where codex keeps it, and the
/// record that ends that turn.
fn running_rollout(b: &Board) -> (PathBuf, String) {
    let lines: Vec<&str> = ROLLOUT.lines().collect();
    let start = lines.iter().rposition(|l| is(l, "task_started")).unwrap();
    let end = lines[start..].iter().find(|l| is(l, "task_complete")).unwrap();
    let path = b.dir.path().join(".codex/sessions/2026/10/07/rollout-2026-10-07T12-22-14-01a117d0-fd5c-7fe3-8f0f-5f2afdf7906d.jsonl");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut text = lines[..start].join("\n");
    text.push('\n');
    text.push_str(&dated_now(lines[start]));
    text.push('\n');
    std::fs::write(&path, text).unwrap();
    (path, end.to_string())
}

/// The screen says between turns; the rollout codex holds says one runs.
/// Nothing is typed until codex writes the turn's end.
#[tokio::test]
async fn an_idle_codex_screen_waits_while_its_rollout_says_a_turn_runs() {
    let b = board().await;
    let agent = b.agent("Agent 2", "codex").await;
    let (rollout, end) = running_rollout(&b);
    let si = b.stand_in_with(&agent, "codex", "codex", &format!("STAND_IN_HOLD='{}'", rollout.display())).await;
    b.doing(agent.id, AgentActivity::Idle).await;
    b.answer("Drill in");
    b.pump().await;
    b.untouched(&si);
    b.pump().await;
    b.untouched(&si);

    let mut file = std::fs::OpenOptions::new().append(true).open(&rollout).unwrap();
    std::io::Write::write_all(&mut file, format!("{end}\n").as_bytes()).unwrap();
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), [b.told("Drill in")], "{}", si.log());
}

/// What each reading of a rollout tells the gate: only a closed turn is
/// idle; one being written is not; no rollout, or none readable, is nothing.
#[test]
fn only_a_closed_turn_is_idle() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rollout-x.jsonl");
    let lines: Vec<&str> = ROLLOUT.lines().collect();
    let start = lines.iter().rposition(|l| is(l, "task_started")).unwrap();
    std::fs::write(&path, lines[..=start].join("\n") + "\n").unwrap();
    assert_eq!(said(Some(&path), None), Said::NotIdle, "a turn running");
    std::fs::write(&path, ROLLOUT).unwrap();
    assert_eq!(said(Some(&path), None), Said::Idle, "between turns");
    std::fs::write(&path, &ROLLOUT[..ROLLOUT.len() - 10]).unwrap();
    assert_eq!(said(Some(&path), None), Said::NotIdle, "a record half written");
    std::fs::write(&path, lines[0].to_string() + "\n").unwrap();
    assert_eq!(said(Some(&path), None), Said::Nothing, "no turn yet");
    assert_eq!(said(None, None), Said::Nothing, "no rollout held");
}

/// An answer a dialog left pasted in codex's box (ov-385) gets its Enter only
/// once the rollout says the turn is over, however idle the screen reads.
#[tokio::test]
async fn a_pasted_answer_is_entered_only_once_codex_s_turn_ends() {
    let b = board().await;
    let agent = b.agent("Agent 2", "codex").await;
    let (rollout, end) = running_rollout(&b);
    let si = b.stand_in_with(&agent, "codex", "codex", &format!("STAND_IN_HOLD='{}'", rollout.display())).await;
    b.doing(agent.id, AgentActivity::Idle).await;
    b.answer("Drill in");
    let wake = b.pending().remove(0);
    si.show(&format!("draft:{}", b.told("Drill in"))).await;
    b.screen_with(agent.id, "Continue.").await;
    assert!(b.svc.store.claim_wake(&wake).unwrap());
    assert!(b.svc.store.mark_wake_pasted(&wake, now_millis() - 1).unwrap());
    b.pump().await;
    assert!(!si.log().contains("ENTER"), "an Enter mid-turn: {}", si.log());
    assert!(b.settled().is_empty(), "{:?}", b.settled());

    let mut file = std::fs::OpenOptions::new().append(true).open(&rollout).unwrap();
    std::io::Write::write_all(&mut file, format!("{end}\n").as_bytes()).unwrap();
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), [b.told("Drill in")], "{}", si.log());
    assert!(!si.log().contains("PASTE "), "pasted: {}", si.log());
}

/// A turn left open by a codex that was killed is over once a later process
/// holds the file (review 1, M1): `codex resume` writes nothing until the
/// next prompt, and its screen shows the turn interrupted. One begun after
/// the process started is that process's own, and runs.
#[test]
fn a_dead_process_s_open_turn_is_over() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rollout-resumed.jsonl");
    std::fs::write(&path, RESUMED).unwrap();
    let start = RESUMED.lines().rfind(|l| is(l, "task_started")).unwrap();
    let begun: serde_json::Value = serde_json::from_str(start).unwrap();
    let begun = begun["payload"]["started_at"].as_i64().unwrap();
    // The resumed codex started at 13:16:04 local, three seconds after.
    assert_eq!(said(Some(&path), Some(begun + 3)), Said::Idle, "resumed after the turn began");
    assert_eq!(said(Some(&path), Some(begun)), Said::NotIdle, "the same second: its own, perhaps");
    assert_eq!(said(Some(&path), None), Said::NotIdle, "no start known");
}

/// The call site: a codex stand-in holding a rollout whose last turn a dead
/// process began is typed into once its screen is idle.
#[tokio::test]
async fn a_resumed_codex_is_told_over_its_dead_turn() {
    let b = board().await;
    let agent = b.agent("Agent 2", "codex").await;
    let path = b.dir.path().join(".codex/sessions/2026/10/07/rollout-2026-10-07T13-15-48-01a11802-07e1-7ca3-bfce-b553cb29fa89.jsonl");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, RESUMED).unwrap();
    let si = b.stand_in_with(&agent, "codex", "codex", &format!("STAND_IN_HOLD='{}'", path.display())).await;
    b.doing(agent.id, AgentActivity::Idle).await;
    b.answer("Drill in");
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), [b.told("Drill in")], "{}", si.log());
}

/// Against a real codex, by pid (`FARCOOLER_CODEX_PID`): which rollout it
/// holds and what the gate says. For a live check in a sandbox only.
/// `cargo test -p farcooler-daemon --lib live_codex_gate -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn live_codex_gate() {
    let Some(pid) = std::env::var("FARCOOLER_CODEX_PID").ok().and_then(|p| p.parse::<i32>().ok()) else { return };
    let rollout = crate::log_join::codex_rollout_of(pid);
    let said = super::super::codex_turn::said_of(pid).await;
    println!("rollout {rollout:?}\nsaid {said:?}");
}

/// A codex turn that starts while the answer's paste is read back gets no
/// Enter (review 1, L1): the answer waits, pasted, and is entered once the
/// turn ends.
#[tokio::test]
async fn a_turn_begun_during_the_read_back_holds_the_enter() {
    let b = board().await;
    let agent = b.agent("Agent 2", "codex").await;
    let (rollout, end) = running_rollout(&b);
    let lines: Vec<&str> = ROLLOUT.lines().collect();
    let start = lines.iter().rposition(|l| is(l, "task_started")).unwrap();
    let begun = dated_now(lines[start]);
    // Between turns to begin with.
    std::fs::write(&rollout, ROLLOUT).unwrap();
    let si = b.stand_in_with(&agent, "codex", "codex", &format!("STAND_IN_HOLD='{}'", rollout.display())).await;
    si.show("slow").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    let (log, path) = (si.log.clone(), rollout.clone());
    let starts = tokio::spawn(async move {
        for _ in 0..2_500 {
            if std::fs::read_to_string(&log).unwrap_or_default().contains("PASTE ") {
                let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
                std::io::Write::write_all(&mut file, format!("{begun}\n").as_bytes()).unwrap();
                return true;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        false
    });
    b.answer("Drill in");
    b.pump().await;
    assert!(starts.await.unwrap(), "the paste never reached the stand-in");
    assert!(!si.log().contains("ENTER"), "an Enter into a running turn: {}", si.log());
    assert!(b.pending()[0].pasted_at.is_some(), "not waiting, pasted: {:?}", b.pending());

    let mut file = std::fs::OpenOptions::new().append(true).open(&rollout).unwrap();
    std::io::Write::write_all(&mut file, format!("{end}\n").as_bytes()).unwrap();
    si.show("idle").await;
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.log().matches("PASTE ").count(), 1, "pasted again: {}", si.log());
}

/// `terminal tell` the same (review 1, L1): a codex turn begun during the
/// read-back leaves the text in the box, with no Enter, and says so.
#[tokio::test]
async fn a_tell_gets_no_enter_into_a_turn_begun_during_the_read_back() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let (rollout, _) = running_rollout(&b);
    let lines: Vec<&str> = ROLLOUT.lines().collect();
    let begun = dated_now(lines[lines.iter().rposition(|l| is(l, "task_started")).unwrap()]);
    std::fs::write(&rollout, ROLLOUT).unwrap();
    let si = b.stand_in_with(&orchestrator, "codex", "codex", &format!("STAND_IN_HOLD='{}'", rollout.display())).await;
    si.show("slow").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    let (log, path) = (si.log.clone(), rollout.clone());
    let starts = tokio::spawn(async move {
        for _ in 0..2_500 {
            if std::fs::read_to_string(&log).unwrap_or_default().contains("PASTE ") {
                let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
                std::io::Write::write_all(&mut file, format!("{begun}\n").as_bytes()).unwrap();
                return true;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        false
    });
    let told = b.watcher.tell_into(orchestrator.id, "hello").await;
    assert!(starts.await.unwrap(), "the paste never reached the stand-in");
    assert!(matches!(told, Err(DomainError::Conflict { what: "paste_left" })), "{told:?}");
    assert!(!si.log().contains("ENTER"), "{}", si.log());
}
