//! A working claude is typed into, as its CLI takes a message mid-turn, and
//! keeps it in its own queue (ov-360): `terminal tell`, an answer, a hold
//! that ended and a draft alike, and never Enter onto a dialog. The stand-in
//! queues on Enter while it works, as claude 2.1.290 does, and submits at
//! idle; its session is "stand-in", and a test says when a hook was heard
//! from it. On the board and tmux server of `tests`, whose helpers these use.

use super::tell_tests::refused_with;
use super::*;

/// The stand-in's own claude config directory (`Board::stand_in`).
fn config_of(b: &Board, terminal: &Terminal) -> PathBuf {
    b.dir.path().join(format!("si-{}", terminal.id.simple())).join("config")
}

/// The session every claude stand-in names in its registry.
const SESSION: &str = "stand-in";

/// Show the stand-in working, as the watcher reads it too, its session's
/// hooks heard from.
async fn working(b: &Board, terminal: &Terminal, si: &StandIn, mode: &str) {
    b.svc.hooks().asks().heard(SESSION, false);
    si.show(mode).await;
    b.screen_with(terminal.id, "esc to interrupt").await;
    b.doing(terminal.id, AgentActivity::Working).await;
}

/// The transcript the stand-in keeps, as `mid_turn` finds it.
fn transcript(b: &Board, terminal: &Terminal) -> String {
    let sessions = config_of(b, terminal).join("sessions");
    let registry = std::fs::read_dir(&sessions).unwrap().next().expect("a session registry").unwrap().path();
    let pid: i32 = registry.file_stem().unwrap().to_str().unwrap().parse().unwrap();
    let (_, path) = super::mid_turn::transcript_in(&config_of(b, terminal), pid).expect("the transcript");
    std::fs::read_to_string(path).unwrap_or_default()
}

/// A working claude takes the message: pasted, read back, Enter, and its
/// transcript's `enqueue` record is what says it was queued. It reaches the
/// agent when the turn ends.
#[tokio::test]
async fn a_message_reaches_a_working_claude_through_its_queue() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    working(&b, &orchestrator, &si, "working").await;
    let turn = b.watcher.tell_into(orchestrator.id, "land ov-360 after the train").await.expect("queued");
    assert_eq!(turn, Turn::During);
    assert!(si.log().contains("QUEUED land ov-360 after the train"), "{}", si.log());
    assert!(si.submitted().is_empty(), "submitted mid-turn: {}", si.log());
    assert!(transcript(&b, &orchestrator).contains(r#""operation":"enqueue","content":"land ov-360 after the train""#));

    // A second while the first waits: the box shows the queue's hint, with
    // the cursor on it, and reads empty.
    b.screen_with(orchestrator.id, "ress up to edit queued messages").await;
    assert_eq!(b.watcher.tell_into(orchestrator.id, "and then the docs").await.expect("queued"), Turn::During);

    si.show("idle").await;
    si.submits(2).await;
    assert_eq!(si.submitted(), ["land ov-360 after the train", "and then the docs"], "{}", si.log());
}

/// A working codex queues too, but raises its approvals with no hook to
/// say so, so an Enter could answer one: it's left to finish its turn.
#[tokio::test]
async fn a_working_codex_is_left_to_finish() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "codex", "codex").await;
    working(&b, &orchestrator, &si, "working").await;
    assert_eq!(refused_with(b.watcher.tell_into(orchestrator.id, "land ov-360").await), "busy");
    nothing_typed(&si);
}

/// A dialog announced after the paste began, and before the Enter: no Enter,
/// and the text is left in the box. The gate is heard the moment the paste
/// reaches the stand-in, well before the read-back's first look.
#[tokio::test]
async fn a_dialog_announced_during_the_paste_gets_no_enter() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    working(&b, &orchestrator, &si, "working").await;
    let asks = b.svc.hooks().asks().clone();
    let log = si.log.clone();
    let gate = tokio::spawn(async move {
        for _ in 0..1_000 {
            if std::fs::read_to_string(&log).unwrap_or_default().contains("PASTE ") {
                asks.heard(SESSION, true);
                return true;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        false
    });
    assert_eq!(refused_with(b.watcher.tell_into(orchestrator.id, "hello").await), "dialog");
    assert!(gate.await.unwrap(), "the paste never reached the stand-in");
    assert!(!si.log().contains("ENTER"), "{}", si.log());

    // An answer the same: left in the box, and said so.
    let agent = b.agent("Agent 2", "claude").await;
    let other = b.stand_in(&agent, "claude", "claude").await;
    working(&b, &agent, &other, "working").await;
    b.svc.hooks().asks().heard(SESSION, true);
    let asks = b.svc.hooks().asks().clone();
    let log = other.log.clone();
    let gate = tokio::spawn(async move {
        for _ in 0..1_000 {
            if std::fs::read_to_string(&log).unwrap_or_default().contains("PASTE ") {
                asks.heard(SESSION, true);
                return;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    });
    b.answer("Drill in");
    b.pump().await;
    gate.await.unwrap();
    assert_eq!(b.settled(), ["Paste left in the composer; not sent"]);
    assert!(!other.log().contains("ENTER"), "{}", other.log());
}

/// The last two checks before the Enter, one at a time: a dialog on the
/// screen, a gate begun since the paste, an ask held on the pane, and a box
/// that no longer holds the text each stop it; with none, it goes.
#[tokio::test]
async fn the_enter_waits_on_the_screen_and_the_hooks() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    let enter = |witness: super::mid_turn::Witness| {
        let (watcher, to) = (b.watcher.clone(), orchestrator.clone());
        async move { watcher.enter(&to, "claude", &witness, "hello").await }
    };
    use super::mid_turn::{NoEnter, Witness};

    si.show("menu").await;
    b.screen_with(orchestrator.id, "Tab to amend").await;
    assert_eq!(enter(Witness::for_tests(SESSION, &path)).await, Err(NoEnter::Dialog), "a dialog on screen");
    si.show("draft:hello").await;
    b.screen_with(orchestrator.id, "hello").await;
    let witness = Witness::for_tests(SESSION, &path);
    b.svc.hooks().asks().heard(SESSION, true);
    assert_eq!(enter(witness).await, Err(NoEnter::Dialog), "a gate since the paste");
    let (_, _held) = b.svc.hooks().asks().hold(orchestrator.id);
    assert_eq!(enter(Witness::for_tests(SESSION, &path)).await, Err(NoEnter::Dialog), "an ask held");
    b.svc.hooks().asks().forget(orchestrator.id);
    si.show("draft:hello there").await;
    b.screen_with(orchestrator.id, "hello there").await;
    assert_eq!(enter(Witness::for_tests(SESSION, &path)).await, Err(NoEnter::Moved), "another text");
    assert!(!si.log().contains("ENTER"), "{}", si.log());
    si.show("draft:hello").await;
    b.screen_with(orchestrator.id, "hello").await;
    let witness = Witness::for_tests(SESSION, &path);
    assert_eq!(enter(witness).await, Ok(()));
    si.submits(1).await;
    assert_eq!(si.submitted(), ["hello"], "{}", si.log());
}

/// An answer is told to a working agent too, and the task says it's queued.
#[tokio::test]
async fn an_answer_reaches_a_working_claude_through_its_queue() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    working(&b, &agent, &si, "working").await;
    b.answer("Drill in");
    b.pump().await;
    assert_eq!(b.progress(), ["Told Agent 2 about the decision. It was working, so it's queued for when it's ready"]);
    assert!(b.pending().is_empty());
    assert!(si.log().contains(&format!("QUEUED {}", b.told("Drill in"))), "{}", si.log());
    si.show("idle").await;
    si.submits(1).await;
    assert_eq!(si.submitted(), [b.told("Drill in")], "{}", si.log());
}

/// A draft goes into a working orchestrator's box, never with Enter: the
/// person's Return queues it.
#[tokio::test]
async fn a_draft_is_pasted_into_a_working_orchestrator() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    working(&b, &orchestrator, &si, "working").await;
    b.watcher.draft_into(orchestrator.id, "About ov-1 (“Fix”): ").await.expect("pasted");
    si.pasted().await;
    assert!(!si.log().contains("ENTER"), "{}", si.log());
}

/// A queue that never shows the message: Enter was pressed, so it may be
/// there, and that's what's said, never "queued".
#[tokio::test]
async fn a_queue_that_never_shows_the_message_is_unconfirmed() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    working(&b, &orchestrator, &si, "working-quiet").await;
    assert_eq!(refused_with(b.watcher.tell_into(orchestrator.id, "hello").await), "unconfirmed");
    assert!(si.log().contains("QUEUED hello"), "{}", si.log());

    let agent = b.agent("Agent 2", "claude").await;
    let other = b.stand_in(&agent, "claude", "claude").await;
    working(&b, &agent, &other, "working-quiet").await;
    b.answer("Drill in");
    b.pump().await;
    assert_eq!(b.settled(), ["Couldn't confirm the agent got the decision"]);
}

/// A working claude whose session can't be found is never typed into: no
/// telling a queued message from a lost one. A message is refused, and an
/// answer waits for the turn to end, as before.
#[tokio::test]
async fn a_working_claude_with_no_session_registry_is_left_to_finish() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    for registry in std::fs::read_dir(config_of(&b, &agent).join("sessions")).unwrap() {
        std::fs::remove_file(registry.unwrap().path()).unwrap();
    }
    working(&b, &agent, &si, "working").await;
    b.answer("Drill in");
    b.pump().await;
    b.untouched(&si);
    assert_eq!(b.progress(), ["Waiting to tell Agent 2 about the decision: it's busy."]);
    si.show("idle").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    b.pump().await;
    si.submits(1).await;
    assert_eq!(b.settled(), ["Told Agent 2 about the decision"]);

    let orchestrator = b.adopted_shell().await;
    let quiet = b.stand_in(&orchestrator, "claude", "claude").await;
    for registry in std::fs::read_dir(config_of(&b, &orchestrator).join("sessions")).unwrap() {
        std::fs::remove_file(registry.unwrap().path()).unwrap();
    }
    working(&b, &orchestrator, &quiet, "working").await;
    assert_eq!(refused_with(b.watcher.tell_into(orchestrator.id, "hello").await), "busy");
    nothing_typed(&quiet);
}

/// A working claude whose session no hook was ever heard from has nothing
/// to say a dialog is coming, so it's never typed into mid-turn.
#[tokio::test]
async fn a_working_claude_no_hook_was_heard_from_is_left_to_finish() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    si.show("working").await;
    b.screen_with(orchestrator.id, "esc to interrupt").await;
    b.doing(orchestrator.id, AgentActivity::Working).await;
    assert_eq!(refused_with(b.watcher.tell_into(orchestrator.id, "hello").await), "busy");
    nothing_typed(&si);
    b.svc.hooks().asks().heard("another-session", false);
    assert_eq!(refused_with(b.watcher.tell_into(orchestrator.id, "hello").await), "busy");
    b.svc.hooks().asks().heard(SESSION, false);
    assert_eq!(b.watcher.tell_into(orchestrator.id, "hello").await.expect("queued"), Turn::During);
}

/// The watcher behind: it says working, the screen says the turn is over.
/// The fresh screen decides, and the message is the next prompt.
#[tokio::test]
async fn the_screen_not_the_watcher_says_whether_a_turn_runs() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Working).await;
    assert_eq!(b.watcher.tell_into(orchestrator.id, "hello").await.expect("sent"), Turn::Between);
    si.submits(1).await;
    assert_eq!(si.submitted(), ["hello"], "{}", si.log());
}

/// What a transcript has to hold, past where it was, to say a message was
/// queued: an `enqueue` record with the text, or the text as the prompt it
/// became, unless the same text was queued before the mark, whose prompt
/// that could be. Earlier records, other text and other records don't count.
#[test]
fn only_a_new_record_of_the_text_says_it_was_queued() {
    use super::mid_turn::{enqueued_before, recorded};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    let enqueue = |text: &str| format!("{{\"type\":\"queue-operation\",\"operation\":\"enqueue\",\"content\":{text:?}}}\n");
    let prompt = |text: &str| format!("{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":{text:?}}}}}\n");
    std::fs::write(&path, enqueue("hello")).unwrap();
    let from = std::fs::metadata(&path).unwrap().len();
    assert!(!recorded(&path, from, "hello", false), "an earlier record");
    std::fs::write(&path, format!("{}{}", enqueue("hello"), enqueue("hello there"))).unwrap();
    assert!(!recorded(&path, from, "hello", false), "other text");
    let dequeue = "{\"type\":\"queue-operation\",\"operation\":\"dequeue\",\"content\":\"hello\"}\n";
    std::fs::write(&path, format!("{}{dequeue}", enqueue("hello"))).unwrap();
    assert!(!recorded(&path, from, "hello", false), "not an enqueue");
    std::fs::write(&path, format!("{}{}", enqueue("hello"), enqueue("hel lo"))).unwrap();
    assert!(recorded(&path, from, "hello", false), "whitespace aside");
    std::fs::write(&path, format!("{}{}", enqueue("hello"), prompt("hello"))).unwrap();
    assert!(recorded(&path, from, "hello", false), "sent as a prompt");
    assert!(!recorded(&path, from, "hello", true), "the prompt the earlier one became");
    std::fs::write(&path, format!("{}{}{}", enqueue("hello"), prompt("hello"), enqueue("hello"))).unwrap();
    assert!(recorded(&path, from, "hello", true), "queued again");
    assert!(!recorded(&dir.path().join("none.jsonl"), 0, "hello", false));

    assert!(enqueued_before(&path, from, "hello"));
    assert!(!enqueued_before(&path, from, "hello there"));
    assert!(!enqueued_before(&path, 0, "hello"), "nothing before the start");
    std::fs::write(&path, format!("{}{}", prompt("hello"), enqueue("hello"))).unwrap();
    let mark = prompt("hello").len() as u64;
    assert!(!enqueued_before(&path, mark, "hello"), "a prompt isn't a queued message, nor is a record past the mark");
}

/// claude's config directory: its own variable, else its HOME's `.claude`,
/// the last of a name winning, as `ps -E` writes the environment after the
/// command's own arguments. A registry naming a path, not a session, finds
/// nothing.
#[test]
fn claudes_transcript_is_found_from_its_process() {
    let env = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<Vec<_>>();
    let config_dir = super::mid_turn::config_dir;
    assert_eq!(config_dir(&env(&[("HOME", "/h"), ("CLAUDE_CONFIG_DIR", "/c")])), Some(PathBuf::from("/c")));
    assert_eq!(config_dir(&env(&[("HOME", "/h"), ("CLAUDE_CONFIG_DIR", "")])), Some(PathBuf::from("/h/.claude")));
    assert_eq!(config_dir(&env(&[("PATH", "/bin")])), None);
    let ps = "claude --model haiku CLAUDE_CONFIG_DIR=/not/this HOME=/h CLAUDE_CONFIG_DIR=/c TERM=xterm";
    let words = super::mid_turn::pairs(ps.split_whitespace().map(str::to_string));
    assert_eq!(config_dir(&words), Some(PathBuf::from("/c")));

    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("sessions")).unwrap();
    let registry = |session: &str| format!("{{\"pid\":42,\"sessionId\":{session:?},\"cwd\":\"/private/tmp/x.y\"}}");
    std::fs::write(dir.path().join("sessions/42.json"), registry("0a-b1")).unwrap();
    assert_eq!(
        super::mid_turn::transcript_in(dir.path(), 42),
        Some(("0a-b1".to_string(), dir.path().join("projects/-private-tmp-x-y/0a-b1.jsonl")))
    );
    std::fs::write(dir.path().join("sessions/42.json"), registry("../../etc/passwd")).unwrap();
    assert_eq!(super::mid_turn::transcript_in(dir.path(), 42), None);
    assert_eq!(super::mid_turn::transcript_in(dir.path(), 43), None);
}
