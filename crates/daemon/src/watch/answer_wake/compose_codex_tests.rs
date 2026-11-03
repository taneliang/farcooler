//! `terminal.compose` into codex (ov-416): a message typed into codex's box
//! between turns, read back as codex 0.153.4 draws it, and Sent once the
//! rollout codex holds records it; and each refusal, with nothing typed. The
//! stand-in draws codex's placeholders and writes its rollout, its head a
//! real one's (`codex-tui-0.153.4`). On the board and tmux server of `tests`.

use std::io::Write;
use std::path::PathBuf;

use super::tell_tests::refused_with;
use super::*;

/// A real 0.153.4 rollout from the sandbox the measurements were made in.
const ROLLOUT: &str = include_str!(
    "../../../../core/fixtures/session-logs/codex-tui-0.153.4/rollout-2026-10-07T16-04-08-01a1189c-23ea-7190-b6bf-1d9ac1940ade.jsonl"
);

/// A 1×1 PNG.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0x0d, 0x49, 0x48, 0x44, 0x52, 0, 0, 0, 1, 0, 0, 0, 1, 8, 2,
    0, 0, 0, 0x90, 0x77, 0x53, 0xde, 0, 0, 0, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x08, 0xd7, 0x63, 0xf8, 0xcf, 0xc0, 0, 0,
    0x03, 0x01, 0x01, 0, 0x18, 0xdd, 0x8d, 0xb0, 0, 0, 0, 0, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

/// Where the stand-in keeps its rollout, under a codex home that isn't
/// `~/.codex` (as a `CODEX_HOME` puts it): the real rollout's head, its
/// `session_meta`, and nothing else yet.
fn rollout(b: &Board) -> PathBuf {
    let path = b.dir.path().join("codex-home/sessions/2026/10/07/rollout-2026-10-07T16-04-08-01a1189c-23ea-7190-b6bf-1d9ac1940ade.jsonl");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, format!("{}\n", ROLLOUT.lines().next().unwrap())).unwrap();
    path
}

/// An idle codex stand-in that records what it takes in `rollout`, holding
/// it open from the start, or (`late`) only from its first message, as a
/// fresh codex does.
async fn idle_codex(b: &Board, rollout: &std::path::Path, late: bool) -> (Terminal, StandIn) {
    let agent = b.agent("Agent 2", "codex").await;
    let mut env = format!("STAND_IN_ROLLOUT='{}'", rollout.display());
    if late {
        std::fs::rename(rollout, rollout.with_extension("jsonl.head")).unwrap();
        env.push_str(" STAND_IN_ROLLOUT_LATE=1");
    }
    let si = b.stand_in_with(&agent, "codex", "codex", &env).await;
    b.doing(agent.id, AgentActivity::Idle).await;
    (agent, si)
}

/// The real rollout's first `task_started`, dated after any process a test
/// starts, so it's that process's turn running (`codex_turn::said`).
fn a_turn_starts() -> String {
    let line = ROLLOUT.lines().find(|l| l.contains(r#""payload":{"type":"task_started""#)).unwrap();
    let at = line.find(r#""timestamp":""#).unwrap() + r#""timestamp":""#.len();
    format!("{}2099-01-01T00:00:00.000Z{}\n", &line[..at], &line[at + "2026-10-07T23:04:12.345Z".len()..])
}

fn refused(result: Result<Turn>) -> &'static str {
    refused_with(result)
}

/// Lines with a blank one between, idle: pasted once, read back, sent, and
/// Sent once the rollout records the message.
#[tokio::test]
async fn multi_line_text_is_sent_once_the_rollout_records_it() {
    let b = board().await;
    let path = rollout(&b);
    let (agent, si) = idle_codex(&b, &path, false).await;
    let turn = b.watcher.compose_into(agent.id, "fix the flaky test\r\n\nthen land it\n", &[]).await;
    assert_eq!(turn.expect("sent"), Turn::Between, "{}", si.log());
    assert_eq!(si.log().matches("PASTE ").count(), 1, "one paste: {}", si.log());
    assert_eq!(si.submitted(), ["fix the flaky test\\n\\nthen land it"], "{}", si.log());
}

/// A backslash at the end, which claude would take as a line break: codex
/// 0.153.4 sends it as typed.
#[tokio::test]
async fn a_backslash_at_the_end_is_sent() {
    let b = board().await;
    let path = rollout(&b);
    let (agent, si) = idle_codex(&b, &path, false).await;
    assert_eq!(b.watcher.compose_into(agent.id, "look in C:\\", &[]).await.expect("sent"), Turn::Between, "{}", si.log());
    assert_eq!(si.submitted(), ["look in C:\\"], "{}", si.log());
}

/// Typed and entered, but no record of it in the rollout: not Sent.
#[tokio::test]
async fn a_send_the_rollout_never_records_is_unconfirmed() {
    let b = board().await;
    // No record is coming: a window that ends sooner says the same.
    b.watcher.with_timing(|t| t.compose_confirm = crate::watch::Timing::SHORT);
    let agent = b.agent("Agent 2", "codex").await;
    let si = b.stand_in(&agent, "codex", "codex").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    assert_eq!(refused(b.watcher.compose_into(agent.id, "one\ntwo", &[]).await), "unconfirmed");
    si.submits(1).await;
    assert_eq!(si.submitted(), ["one\\ntwo"], "it was typed: {}", si.log());
}

/// Past 1,000 characters codex shows `[Pasted Content N chars]`: read back
/// as that, the count exact, and confirmed by the whole text.
#[tokio::test]
async fn a_long_paste_is_read_back_as_its_placeholder() {
    let b = board().await;
    let path = rollout(&b);
    let (agent, si) = idle_codex(&b, &path, false).await;
    let long = "word ".repeat(220);
    assert_eq!(b.watcher.compose_into(agent.id, &long, &[]).await.expect("sent"), Turn::Between, "{}", si.log());
    assert_eq!(si.submitted(), [long.trim_end().to_string()], "{}", si.log());
}

/// An image: its path pasted alone, read back as `[Image #1]`, then the
/// text; the rollout's message carries the image.
#[tokio::test]
async fn an_image_goes_in_first_as_its_placeholder() {
    let b = board().await;
    let path = rollout(&b);
    let (agent, si) = idle_codex(&b, &path, false).await;
    let image = ("image/png".to_string(), PNG.to_vec());
    let turn = b.watcher.compose_into(agent.id, "what color is it", &[image]).await;
    assert_eq!(turn.expect("sent"), Turn::Between, "{}", si.log());
    assert_eq!(si.submitted(), ["[Image #1]  what color is it"], "{}", si.log());
    let recorded = farcooler_core::session_log::codex_prompts::prompts_from(&path, 0);
    assert_eq!(recorded.iter().map(|p| p.images).collect::<Vec<_>>(), [1], "{recorded:?}");
}

/// A codex with no rollout yet opens one at its first message: the send is
/// confirmed from the file it holds after the Enter.
#[tokio::test]
async fn a_fresh_codex_is_confirmed_by_the_rollout_it_opens() {
    let b = board().await;
    let path = rollout(&b);
    let (agent, si) = idle_codex(&b, &path, true).await;
    assert!(!path.exists());
    let turn = b.watcher.compose_into(agent.id, "hello", &[]).await;
    assert_eq!(turn.expect("sent"), Turn::Between, "{}", si.log());
    assert!(path.exists());
}

/// What codex would take as something other than a message, or can't be
/// read back, is refused before anything is typed: a command opens a popup
/// or a panel; a last word starting `@` or `$` opens a picker; `!` runs a
/// shell; a text taller than its box can't be read back.
#[tokio::test]
async fn what_codex_wouldn_t_send_is_refused_untyped() {
    let b = board().await;
    let path = rollout(&b);
    let (agent, si) = idle_codex(&b, &path, false).await;
    let tall = (1..=60).map(|n| format!("row {n}")).collect::<Vec<_>>().join("\n");
    for (text, why) in [
        ("/init focus on tests", "handoff"),
        ("/model", "handoff"),
        ("look at @src/main.rs", "picker"),
        ("use $skill-creator", "picker"),
        ("!ls", "command"),
        (tall.as_str(), "too_tall"),
    ] {
        assert_eq!(refused(b.watcher.compose_into(agent.id, text, &[]).await), why, "{text}");
    }
    nothing_typed(&si);
}

/// A turn running, as the screen says or as the rollout says while the
/// screen reads idle: `busy`, nothing typed. codex would take the Enter as a
/// steer, under approvals no hook fences.
#[tokio::test]
async fn a_codex_mid_turn_is_busy() {
    let b = board().await;
    let path = rollout(&b);
    let (agent, si) = idle_codex(&b, &path, false).await;
    si.show("working").await;
    b.doing(agent.id, AgentActivity::Working).await;
    assert_eq!(refused(b.watcher.compose_into(agent.id, "hello", &[]).await), "busy");
    si.show("idle").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(a_turn_starts().as_bytes()).unwrap();
    assert_eq!(refused(b.watcher.compose_into(agent.id, "hello", &[]).await), "busy");
    nothing_typed(&si);
}

/// The watcher lost codex (under fish its pane's command reads `fish`, and
/// its word goes to none): the fresh capture and the rollout decide. Idle and
/// nobody typing, sent; someone typing, or a turn on the screen, refused.
#[tokio::test]
async fn a_watcher_that_lost_codex_defers_to_the_screen_and_rollout() {
    let b = board().await;
    let path = rollout(&b);
    let (agent, si) = idle_codex(&b, &path, false).await;
    b.doing(agent.id, AgentActivity::None).await;
    crate::runtime::mark_input(b.svc.root_dir(), agent.id);
    assert_eq!(refused(b.watcher.compose_into(agent.id, "now", &[]).await), "typing");
    let long_ago = now_millis() - QUIET_WATCHED_MS - 1_000;
    std::fs::write(crate::runtime::input_mark(b.svc.root_dir(), agent.id), long_ago.to_string()).unwrap();
    si.show("working").await;
    b.doing(agent.id, AgentActivity::None).await;
    assert_eq!(refused(b.watcher.compose_into(agent.id, "now", &[]).await), "busy");
    nothing_typed(&si);
    si.show("idle").await;
    b.doing(agent.id, AgentActivity::None).await;
    // Idle on the screen, but the rollout says a turn runs: the rollout wins.
    std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(a_turn_starts().as_bytes()).unwrap();
    assert_eq!(refused(b.watcher.compose_into(agent.id, "now", &[]).await), "busy");
    nothing_typed(&si);
    let closed = ROLLOUT.lines().find(|l| l.contains(r#""payload":{"type":"task_complete""#)).unwrap();
    std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(format!("{closed}\n").as_bytes()).unwrap();
    assert_eq!(b.watcher.compose_into(agent.id, "now", &[]).await.expect("sent"), Turn::Between, "{}", si.log());
    assert_eq!(si.submitted(), ["now"], "{}", si.log());
}

/// A turn begun between the read-back and the Enter, as the rollout says:
/// no Enter, the text left in the box.
#[tokio::test]
async fn a_turn_begun_before_the_enter_leaves_the_paste() {
    let b = board().await;
    let path = rollout(&b);
    let (agent, si) = idle_codex(&b, &path, false).await;
    let start = a_turn_starts();
    let held = path.clone();
    *b.watcher.before_enter.lock().unwrap() = Some(Box::new(move || {
        std::fs::OpenOptions::new().append(true).open(&held).unwrap().write_all(start.as_bytes()).unwrap();
    }));
    assert_eq!(refused(b.watcher.compose_into(agent.id, "hello", &[]).await), "paste_left");
    assert!(si.log().contains("PASTE hello"), "{}", si.log());
    assert!(!si.log().contains("ENTER"), "{}", si.log());
}

/// The screen is the last thing read before the Enter: a turn that shows
/// after the rollout was checked leaves the paste.
#[tokio::test]
async fn the_screen_is_read_after_the_rollout_check() {
    let b = board().await;
    let path = rollout(&b);
    let (agent, si) = idle_codex(&b, &path, false).await;
    let shower = si.clone();
    *b.watcher.after_rollout.lock().unwrap() = Some(Box::new(move || shower.show_in_hook("working")));
    assert_eq!(refused(b.watcher.compose_into(agent.id, "hello", &[]).await), "paste_left");
    assert!(!si.log().contains("ENTER"), "{}", si.log());
}

/// The gate found codex's rollout and the lookup before the Enter finds none
/// (its file gone from under the process): held, not passed as a first prompt.
#[tokio::test]
async fn a_rollout_lost_before_the_enter_leaves_the_paste() {
    let b = board().await;
    let path = rollout(&b);
    let (agent, si) = idle_codex(&b, &path, false).await;
    let held = path.clone();
    *b.watcher.before_enter.lock().unwrap() = Some(Box::new(move || std::fs::remove_file(&held).unwrap()));
    assert_eq!(refused(b.watcher.compose_into(agent.id, "hello", &[]).await), "paste_left");
    assert!(si.log().contains("PASTE hello"), "{}", si.log());
    assert!(!si.log().contains("ENTER"), "{}", si.log());
}

/// The gate reads codex's rollout before its screen (ov-428): a turn that
/// shows on the screen after the rollout was read is the screen's word.
#[tokio::test]
async fn the_gate_reads_the_screen_after_the_rollout() {
    let b = board().await;
    let path = rollout(&b);
    let (agent, si) = idle_codex(&b, &path, false).await;
    let shower = si.clone();
    *b.watcher.after_gate_rollout.lock().unwrap() = Some(Box::new(move || shower.show_in_hook("working")));
    assert_eq!(refused(b.watcher.compose_into(agent.id, "hello", &[]).await), "busy");
    nothing_typed(&si);
}

/// A rollout the gate had (the join of a moment ago) that the lookup now
/// misses: held as a turn that may run, not passed as a first prompt.
#[tokio::test]
async fn the_gate_holds_when_a_known_rollout_is_lost() {
    let b = board().await;
    let path = rollout(&b);
    let (agent, si) = idle_codex(&b, &path, false).await;
    // Through the gate, so the rollout is known; refused after it.
    assert_eq!(refused(b.watcher.compose_into(agent.id, "/init", &[]).await), "handoff");
    std::fs::remove_file(&path).unwrap();
    assert_eq!(refused(b.watcher.compose_into(agent.id, "now", &[]).await), "busy");
    nothing_typed(&si);
}

/// The scheduler's positive control: a fixed, runner-attributed message is
/// submitted once, then confirmed only by the same thread's rollout.
#[cfg(unix)]
#[tokio::test]
async fn scheduled_idle_codex_submits_once_and_records_a_correlated_receipt() {
    let b = board().await;
    assert!(b.svc.store.arm_check_in_instance(Uuid::now_v7()).unwrap());
    b.watcher.check_in_arm.store(1, std::sync::atomic::Ordering::SeqCst);
    let path = rollout(&b);
    let completed = ROLLOUT.lines().find(|line| line.contains(r#""type":"task_complete""#)).unwrap();
    writeln!(std::fs::OpenOptions::new().append(true).open(&path).unwrap(), "{completed}").unwrap();
    let (agent, si) = idle_codex(&b, &path, false).await;
    let agent = b.svc.store.set_terminal_role(agent.id, TerminalRole::Orchestrator).unwrap();
    let agent = b.svc.store.set_pane_mode(
        agent.id, agent.resource_version, agent.pane_mode,
        Some("01a1189c-23ea-7190-b6bf-1d9ac1940ade".into()), false,
    ).unwrap();
    b.doing(agent.id, AgentActivity::Idle).await;
    let now = crate::review::now_millis();
    b.svc.store.enable_check_in(b.task.workspace_id, b.task.id, None, None, None, false, now - 3_600_000).unwrap();
    assert!(b.svc.store.tick_check_in(b.task.workspace_id, now).unwrap());
    let pending = b.svc.store.pending_check_ins().unwrap().remove(0);
    b.watcher.send_check_in(&pending, Uuid::now_v7()).await.expect("scheduled delivery");
    si.submits(1).await;
    assert_eq!(si.submitted().len(), 1, "{}", si.log());
    assert!(si.submitted()[0].starts_with("[Far Cooler] Scheduled hourly check-in"));
    let submitted = b.svc.store.submitted_check_ins().unwrap();
    assert_eq!(submitted.len(), 1);
    // A replacement at the same name cannot confirm the old attempt, even
    // when it contains the same bytes. Restore it for the positive control.
    let held = path.with_extension("pinned");
    std::fs::rename(&path, &held).unwrap();
    std::fs::copy(&held, &path).unwrap();
    b.watcher.observe_check_in(submitted[0].0, submitted[0].1).await.unwrap();
    assert_eq!(b.svc.store.check_in_status(b.task.workspace_id).unwrap().unwrap().occurrence.unwrap().state, "submitted");
    std::fs::remove_file(&path).unwrap();
    std::fs::rename(&held, &path).unwrap();
    for _ in 0..100 {
        b.watcher.observe_check_in(submitted[0].0, submitted[0].1).await.unwrap();
        if b.svc.store.check_in_status(b.task.workspace_id).unwrap().unwrap().occurrence.unwrap().state == "confirmed" { break }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let status = b.svc.store.check_in_status(b.task.workspace_id).unwrap().unwrap();
    assert_eq!(status.occurrence.unwrap().state, "confirmed");
    assert_eq!(si.submitted().len(), 1, "no duplicate prompt");
}

/// Changing the pinned rollout at either boundary must not authorize a key.
#[cfg(unix)]
#[tokio::test]
async fn scheduled_rollout_replacement_before_claim_or_enter_never_sends() {
    for boundary in 0..3 {
        let b = board().await;
        assert!(b.svc.store.arm_check_in_instance(Uuid::now_v7()).unwrap());
        b.watcher.check_in_arm.store(1, std::sync::atomic::Ordering::SeqCst);
        let path = rollout(&b);
        let completed = ROLLOUT.lines().find(|line| line.contains(r#""type":"task_complete""#)).unwrap();
        writeln!(std::fs::OpenOptions::new().append(true).open(&path).unwrap(), "{completed}").unwrap();
        let (agent, si) = idle_codex(&b, &path, false).await;
        let agent = b.svc.store.set_terminal_role(agent.id, TerminalRole::Orchestrator).unwrap();
        let agent = b.svc.store.set_pane_mode(agent.id, agent.resource_version, agent.pane_mode,
            Some("01a1189c-23ea-7190-b6bf-1d9ac1940ade".into()), false).unwrap();
        b.doing(agent.id, AgentActivity::Idle).await;
        let now = crate::review::now_millis();
        b.svc.store.enable_check_in(b.task.workspace_id, b.task.id, None, None, None, false, now - 3_600_000).unwrap();
        b.svc.store.tick_check_in(b.task.workspace_id, now).unwrap();
        let pending = b.svc.store.pending_check_ins().unwrap().remove(0);
        let changed = path.clone();
        let fired = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let marker = fired.clone();
        let replace = Box::new(move || {
            std::fs::rename(&changed, changed.with_extension("old")).unwrap();
            marker.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        match boundary {
            0 => *b.watcher.after_gate_rollout.lock().unwrap() = Some(replace),
            1 => *b.watcher.before_paste.lock().unwrap() = Some(replace),
            _ => *b.watcher.before_enter.lock().unwrap() = Some(replace),
        }
        let _ = b.watcher.send_check_in(&pending, Uuid::now_v7()).await;
        assert!(fired.load(std::sync::atomic::Ordering::SeqCst), "fixture never reached the intended boundary");
        assert!(si.submitted().is_empty(), "{}", si.log());
        assert!(!si.log().contains("ENTER"), "{}", si.log());
        if boundary < 2 { assert!(!si.log().contains("PASTE "), "{}", si.log()); }
        if boundary == 0 { assert_eq!(b.svc.store.pending_check_ins().unwrap().len(), 1); }
        else { assert_eq!(b.svc.store.check_in_status(b.task.workspace_id).unwrap().unwrap().occurrence.unwrap().state, "uncertain"); }
    }
}

/// A dialog or input arriving while the final destination proof is slow
/// invalidates the earlier composer capture, even with identity unchanged.
#[cfg(unix)]
async fn scheduled_final_discovery_transition(dialog: bool) {
    let b = board().await;
    assert!(b.svc.store.arm_check_in_instance(Uuid::now_v7()).unwrap());
    b.watcher.check_in_arm.store(1, std::sync::atomic::Ordering::SeqCst);
    let path = rollout(&b);
    let completed = ROLLOUT.lines().find(|line| line.contains(r#""type":"task_complete""#)).unwrap();
    writeln!(std::fs::OpenOptions::new().append(true).open(&path).unwrap(), "{completed}").unwrap();
    let (agent, si) = idle_codex(&b, &path, false).await;
    let agent = b.svc.store.set_terminal_role(agent.id, TerminalRole::Orchestrator).unwrap();
    let agent = b.svc.store.set_pane_mode(agent.id, agent.resource_version, agent.pane_mode,
        Some("01a1189c-23ea-7190-b6bf-1d9ac1940ade".into()), false).unwrap();
    b.doing(agent.id, AgentActivity::Idle).await;
    let now = crate::review::now_millis();
    b.svc.store.enable_check_in(b.task.workspace_id, b.task.id, None, None, None, false, now - 3_600_000).unwrap();
    b.svc.store.tick_check_in(b.task.workspace_id, now).unwrap();
    let pending = b.svc.store.pending_check_ins().unwrap().remove(0);
    let fired = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let marker = fired.clone();
    let shower = si.clone();
    let root = b.svc.root_dir().to_path_buf();
    let id = agent.id;
    let service = b.svc.clone();
    let snapshot = service.inventory_snapshot();
    assert!(snapshot.inventory_healthy);
    let claimants = snapshot.claimants(id);
    let [claimed] = claimants.as_slice() else { panic!("fixture must have exactly one pane") };
    assert!(claimed.proves_life());
    let pane = claimed.pane_id.clone();
    *b.watcher.after_rollout.lock().unwrap() = Some(Box::new(move || {
        if dialog {
            shower.show_in_hook("menu");
            std::thread::scope(|scope| {
                scope.spawn(|| {
                    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
                    let screen = runtime.block_on(service.runtime().tmux.capture_screen(&pane)).unwrap();
                    assert_eq!(service.registry().classify("codex", &screen), AgentActivity::Blocked,
                        "the pinned pane must show the approval before the sender continues: {screen}");
                }).join().unwrap();
            });
        } else { crate::runtime::mark_input(&root, id); }
        marker.store(true, std::sync::atomic::Ordering::SeqCst);
    }));
    let _ = b.watcher.send_check_in(&pending, Uuid::now_v7()).await;
    assert!(fired.load(std::sync::atomic::Ordering::SeqCst), "fixture never reached the final discovery seam");
    assert!(si.log().contains("PASTE "), "{}", si.log());
    assert!(!si.log().contains("ENTER") && si.submitted().is_empty(), "{}", si.log());
    assert_eq!(b.svc.store.check_in_status(b.task.workspace_id).unwrap().unwrap().occurrence.unwrap().state, "uncertain");
}

#[cfg(unix)]
#[tokio::test]
async fn scheduled_final_discovery_rechecks_visible_dialog_before_enter() {
    scheduled_final_discovery_transition(true).await;
}

#[cfg(unix)]
#[tokio::test]
async fn scheduled_final_discovery_rechecks_input_before_enter() {
    scheduled_final_discovery_transition(false).await;
}

#[cfg(unix)]
#[tokio::test]
async fn scheduled_stalled_discovery_reaps_child_and_shutdown_releases_gate() {
    let b = board().await;
    let path = rollout(&b);
    let completed = ROLLOUT.lines().find(|line| line.contains(r#""type":"task_complete""#)).unwrap();
    writeln!(std::fs::OpenOptions::new().append(true).open(&path).unwrap(), "{completed}").unwrap();
    let (agent, si) = idle_codex(&b, &path, false).await;
    let agent = b.svc.store.set_terminal_role(agent.id, TerminalRole::Orchestrator).unwrap();
    let agent = b.svc.store.set_pane_mode(agent.id, agent.resource_version, agent.pane_mode,
        Some("01a1189c-23ea-7190-b6bf-1d9ac1940ade".into()), false).unwrap();
    b.doing(agent.id, AgentActivity::Idle).await;
    let now = crate::review::now_millis();
    b.svc.store.enable_check_in(b.task.workspace_id, b.task.id, None, None, None, false, now - 3_600_000).unwrap();
    b.svc.store.tick_check_in(b.task.workspace_id, now).unwrap();
    let pidfile = b.dir.path().join("stalled-discovery.pid");
    let (stop, rx) = tokio::sync::watch::channel(false);
    let owner = b.watcher.clone();
    let task = tokio::spawn(crate::watch::answer_wake::codex_turn::TEST_STALLED_DISCOVERY.scope(pidfile.clone(), async move {
        owner.run_check_in_scheduler(Uuid::now_v7(), rx).await;
    }));
    let pid: i32 = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let Ok(text) = std::fs::read_to_string(&pidfile)
                && let Ok(pid) = text.trim().parse::<i32>() { break pid }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }).await.expect("discovery must start");
    stop.send(true).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), task).await.expect("scheduler must join").unwrap();
    assert!(b.watcher.check_in_cleanup_gate.try_lock().is_ok());
    assert!(!si.log().contains("PASTE ") && !si.log().contains("ENTER"), "{}", si.log());
    // SAFETY: signal 0 checks existence without delivering a signal.
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
}

#[cfg(unix)]
#[tokio::test]
async fn scheduled_discovery_panic_reaps_owned_child_before_gate_release() {
    let b = board().await;
    assert!(b.svc.store.arm_check_in_instance(Uuid::now_v7()).unwrap());
    b.watcher.check_in_arm.store(1, std::sync::atomic::Ordering::SeqCst);
    let path = rollout(&b);
    let completed = ROLLOUT.lines().find(|line| line.contains(r#""type":"task_complete""#)).unwrap();
    writeln!(std::fs::OpenOptions::new().append(true).open(&path).unwrap(), "{completed}").unwrap();
    let (agent, si) = idle_codex(&b, &path, false).await;
    let agent = b.svc.store.set_terminal_role(agent.id, TerminalRole::Orchestrator).unwrap();
    let agent = b.svc.store.set_pane_mode(agent.id, agent.resource_version, agent.pane_mode,
        Some("01a1189c-23ea-7190-b6bf-1d9ac1940ade".into()), false).unwrap();
    b.doing(agent.id, AgentActivity::Idle).await;
    let now = crate::review::now_millis();
    b.svc.store.enable_check_in(b.task.workspace_id, b.task.id, None, None, None, false, now - 3_600_000).unwrap();
    b.svc.store.tick_check_in(b.task.workspace_id, now).unwrap();
    let pending = b.svc.store.pending_check_ins().unwrap().remove(0);
    let stall_file = b.dir.path().join("panic-stall.pid");
    let child_file = b.dir.path().join("panic-child.pid");
    use crate::watch::answer_wake::codex_turn::{TEST_DISCOVERY_PANIC, TEST_STALLED_DISCOVERY};
    let result = TEST_STALLED_DISCOVERY.scope(stall_file, TEST_DISCOVERY_PANIC.scope(child_file.clone(),
        b.watcher.send_check_in(&pending, Uuid::now_v7()))).await;
    assert!(result.is_err());
    assert!(!b.watcher.check_in_is_armed());
    assert!(b.watcher.check_in_cleanup_gate.try_lock().is_ok());
    assert!(!si.log().contains("PASTE ") && !si.log().contains("ENTER"), "{}", si.log());
    let pid: i32 = std::fs::read_to_string(child_file).unwrap().parse().unwrap();
    // SAFETY: signal 0 checks existence without delivering a signal.
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
}

/// Cleanup already owns the gate: a due scheduled attempt must not even
/// claim, paste or send, and must remain available for the next tick.
#[cfg(unix)]
#[tokio::test]
async fn scheduled_cleanup_gate_blocks_claim_paste_and_enter() {
    let b = board().await;
    assert!(b.svc.store.arm_check_in_instance(Uuid::now_v7()).unwrap());
    b.watcher.check_in_arm.store(1, std::sync::atomic::Ordering::SeqCst);
    assert!(b.watcher.check_in_is_armed());
    let path = rollout(&b);
    let completed = ROLLOUT.lines().find(|line| line.contains(r#""type":"task_complete""#)).unwrap();
    writeln!(std::fs::OpenOptions::new().append(true).open(&path).unwrap(), "{completed}").unwrap();
    let (agent, si) = idle_codex(&b, &path, false).await;
    let agent = b.svc.store.set_terminal_role(agent.id, TerminalRole::Orchestrator).unwrap();
    let agent = b.svc.store.set_pane_mode(agent.id, agent.resource_version, agent.pane_mode,
        Some("01a1189c-23ea-7190-b6bf-1d9ac1940ade".into()), false).unwrap();
    b.doing(agent.id, AgentActivity::Idle).await;
    let now = crate::review::now_millis();
    b.svc.store.enable_check_in(b.task.workspace_id, b.task.id, None, None, None, false, now - 3_600_000).unwrap();
    assert!(b.svc.store.tick_check_in(b.task.workspace_id, now).unwrap());
    let pending = b.svc.store.pending_check_ins().unwrap().remove(0);
    let gate = b.watcher.check_in_cleanup_gate.lock().await;
    b.watcher.send_check_in(&pending, Uuid::now_v7()).await.unwrap();
    assert_eq!(b.svc.store.pending_check_ins().unwrap().len(), 1);
    assert!(!si.log().contains("PASTE "), "{}", si.log());
    assert!(!si.log().contains("ENTER"), "{}", si.log());
    drop(gate);
    b.watcher.send_check_in(&pending, Uuid::now_v7()).await.unwrap();
    si.submits(1).await;
    assert_eq!(si.submitted().len(), 1, "the fixture must be ready once cleanup releases: {}", si.log());
}

/// A caught sender panic leaves the admitted attempt uncertain and disarms
/// the feature; the owner reaps its Enter client before releasing the gate.
#[cfg(unix)]
#[tokio::test]
async fn scheduled_sender_panic_disarms_and_retains_uncertain_attempt() {
    let b = board().await;
    assert!(b.svc.store.arm_check_in_instance(Uuid::now_v7()).unwrap());
    b.watcher.check_in_arm.store(1, std::sync::atomic::Ordering::SeqCst);
    let path = rollout(&b);
    let completed = ROLLOUT.lines().find(|line| line.contains(r#""type":"task_complete""#)).unwrap();
    writeln!(std::fs::OpenOptions::new().append(true).open(&path).unwrap(), "{completed}").unwrap();
    let (agent, _si) = idle_codex(&b, &path, false).await;
    let agent = b.svc.store.set_terminal_role(agent.id, TerminalRole::Orchestrator).unwrap();
    let agent = b.svc.store.set_pane_mode(agent.id, agent.resource_version, agent.pane_mode,
        Some("01a1189c-23ea-7190-b6bf-1d9ac1940ade".into()), false).unwrap();
    b.doing(agent.id, AgentActivity::Idle).await;
    let now = crate::review::now_millis();
    b.svc.store.enable_check_in(b.task.workspace_id, b.task.id, None, None, None, false, now - 3_600_000).unwrap();
    assert!(b.svc.store.tick_check_in(b.task.workspace_id, now).unwrap());
    let pending = b.svc.store.pending_check_ins().unwrap().remove(0);
    b.watcher.panic_after_check_in_spawn.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(b.watcher.send_check_in(&pending, Uuid::now_v7()).await.is_err());
    assert!(!b.watcher.check_in_is_armed());
    assert_eq!(b.svc.store.check_in_status(b.task.workspace_id).unwrap().unwrap().occurrence.unwrap().state, "uncertain");
    assert!(b.watcher.check_in_cleanup_gate.try_lock().is_ok());
}

/// Hours while the orchestrator is working remain one pending occurrence and
/// never take the ordinary mid-turn delivery routes.
#[cfg(unix)]
#[tokio::test]
async fn scheduled_busy_codex_waits_without_paste_or_duplicate() {
    let b = board().await;
    assert!(b.svc.store.arm_check_in_instance(Uuid::now_v7()).unwrap());
    b.watcher.check_in_arm.store(1, std::sync::atomic::Ordering::SeqCst);
    let path = rollout(&b);
    let completed = ROLLOUT.lines().find(|line| line.contains(r#""type":"task_complete""#)).unwrap();
    writeln!(std::fs::OpenOptions::new().append(true).open(&path).unwrap(), "{completed}").unwrap();
    let (agent, si) = idle_codex(&b, &path, false).await;
    let agent = b.svc.store.set_terminal_role(agent.id, TerminalRole::Orchestrator).unwrap();
    let agent = b.svc.store.set_pane_mode(
        agent.id, agent.resource_version, agent.pane_mode,
        Some("01a1189c-23ea-7190-b6bf-1d9ac1940ade".into()), false,
    ).unwrap();
    si.show("working").await;
    b.doing(agent.id, AgentActivity::Working).await;
    let now = crate::review::now_millis();
    b.svc.store.enable_check_in(b.task.workspace_id, b.task.id, None, None, None, false, now - 3_600_000).unwrap();
    assert!(b.svc.store.tick_check_in(b.task.workspace_id, now).unwrap());
    let pending = b.svc.store.pending_check_ins().unwrap().remove(0);
    b.watcher.send_check_in(&pending, Uuid::now_v7()).await.unwrap();
    assert!(!b.svc.store.tick_check_in(b.task.workspace_id, now + 5 * 3_600_000).unwrap());
    assert_eq!(b.svc.store.pending_check_ins().unwrap().len(), 1);
    assert!(!si.log().contains("PASTE "), "{}", si.log());
    assert!(si.submitted().is_empty());
}

/// A runner-controlled stop must lose to a previously admitted Enter before
/// it asks tmux to kill the target pane. The pane here is an isolated stand-in.
#[cfg(unix)]
#[tokio::test]
async fn scheduled_admission_vetoes_stop_before_kill() {
    let b = board().await;
    let path = rollout(&b);
    let (agent, _si) = idle_codex(&b, &path, false).await;
    let agent = b.svc.store.set_terminal_role(agent.id, TerminalRole::Orchestrator).unwrap();
    let agent = b.svc.store.set_pane_mode(
        agent.id, agent.resource_version, agent.pane_mode,
        Some("01a1189c-23ea-7190-b6bf-1d9ac1940ade".into()), false,
    ).unwrap();
    let now = crate::review::now_millis();
    b.svc.store.enable_check_in(b.task.workspace_id, b.task.id, None, None, None, false, now - 3_600_000).unwrap();
    assert!(b.svc.store.tick_check_in(b.task.workspace_id, now).unwrap());
    let occurrence = b.svc.store.pending_check_ins().unwrap()[0].id;
    let (attempt, daemon) = (Uuid::now_v7(), Uuid::now_v7());
    assert!(b.svc.store.claim_check_in(occurrence, attempt, daemon, agent.id, "01a1189c-23ea-7190-b6bf-1d9ac1940ade", "%1", 123, 1, now).unwrap());
    assert!(b.svc.store.pin_check_in_rollout(occurrence, attempt, "/test-rollout", "/dev/test", 7, 1, 1, 0).unwrap());
    assert!(b.svc.store.admit_check_in_enter(occurrence, attempt, daemon, agent.id, "01a1189c-23ea-7190-b6bf-1d9ac1940ade", "%1", 123, 1, 7, now).unwrap());
    let before = b.svc.tmux.call_counts().get("kill-pane").copied().unwrap_or(0);
    let stopped = b.svc.stop_terminal(agent.id).await;
    assert!(matches!(&stopped, Err(DomainError::ResourceConflict)), "{stopped:?}");
    let after = b.svc.tmux.call_counts().get("kill-pane").copied().unwrap_or(0);
    assert_eq!(after, before, "no external kill may start after admission");
}
