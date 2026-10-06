//! A working agent is typed into, as its CLI takes a message mid-turn, and
//! keeps it in its own queue (ov-360): `terminal tell`, an answer, a hold
//! that ended and a draft alike. The stand-in queues on Enter while it
//! works, as claude 2.1.290 and codex 0.153.4 do, and submits at idle. On
//! the board and tmux server of `tests`, whose helpers these use.

use super::tell_tests::refused_with;
use super::*;

/// The stand-in's own claude config directory (`Board::stand_in`).
fn config_of(b: &Board, terminal: &Terminal) -> PathBuf {
    b.dir.path().join(format!("si-{}", terminal.id.simple())).join("config")
}

/// Show the stand-in working, as the watcher reads it too.
async fn working(b: &Board, terminal: &Terminal, si: &StandIn, mode: &str) {
    si.show(mode).await;
    b.screen_with(terminal.id, "esc to interrupt").await;
    b.doing(terminal.id, AgentActivity::Working).await;
}

/// The transcript the stand-in keeps, as `mid_turn` finds it.
fn transcript(b: &Board, terminal: &Terminal) -> String {
    let sessions = config_of(b, terminal).join("sessions");
    let registry = std::fs::read_dir(&sessions).unwrap().next().expect("a session registry").unwrap().path();
    let pid: i32 = registry.file_stem().unwrap().to_str().unwrap().parse().unwrap();
    let path = super::mid_turn::transcript_in(&config_of(b, terminal), pid).expect("the transcript");
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

/// codex too: its box's footer is the hint that Tab queues while it works,
/// and the message drawn under its queue is what says it was queued.
#[tokio::test]
async fn a_message_reaches_a_working_codex_through_its_queue() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "codex", "codex").await;
    working(&b, &orchestrator, &si, "working").await;
    assert_eq!(b.watcher.tell_into(orchestrator.id, "land ov-360").await.expect("queued"), Turn::During);
    assert!(si.log().contains("QUEUED land ov-360"), "{}", si.log());
    si.show("idle").await;
    si.submits(1).await;
    assert_eq!(si.submitted(), ["land ov-360"], "{}", si.log());
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
/// became. Earlier records, other text and other records don't count.
#[test]
fn only_a_new_record_of_the_text_says_it_was_queued() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    let enqueue = |text: &str| format!("{{\"type\":\"queue-operation\",\"operation\":\"enqueue\",\"content\":{text:?}}}\n");
    std::fs::write(&path, enqueue("hello")).unwrap();
    let from = std::fs::metadata(&path).unwrap().len();
    assert!(!super::mid_turn::recorded(&path, from, "hello"), "an earlier record");
    std::fs::write(&path, format!("{}{}", enqueue("hello"), enqueue("hello there"))).unwrap();
    assert!(!super::mid_turn::recorded(&path, from, "hello"), "other text");
    let dequeue = "{\"type\":\"queue-operation\",\"operation\":\"dequeue\",\"content\":\"hello\"}\n";
    std::fs::write(&path, format!("{}{dequeue}", enqueue("hello"))).unwrap();
    assert!(!super::mid_turn::recorded(&path, from, "hello"), "not an enqueue");
    std::fs::write(&path, format!("{}{}", enqueue("hello"), enqueue("hel lo"))).unwrap();
    assert!(super::mid_turn::recorded(&path, from, "hello"), "whitespace aside");
    let prompt = "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hello\"}}\n";
    std::fs::write(&path, format!("{}{prompt}", enqueue("hello"))).unwrap();
    assert!(super::mid_turn::recorded(&path, from, "hello"), "sent as a prompt");
    assert!(!super::mid_turn::recorded(&dir.path().join("none.jsonl"), 0, "hello"));
}

/// claude's config directory: its own variable, else its HOME's `.claude`.
/// A registry naming a path, not a session, finds nothing.
#[test]
fn claudes_transcript_is_found_from_its_process() {
    let env = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<Vec<_>>();
    let config_dir = super::mid_turn::config_dir;
    assert_eq!(config_dir(&env(&[("HOME", "/h"), ("CLAUDE_CONFIG_DIR", "/c")])), Some(PathBuf::from("/c")));
    assert_eq!(config_dir(&env(&[("HOME", "/h"), ("CLAUDE_CONFIG_DIR", "")])), Some(PathBuf::from("/h/.claude")));
    assert_eq!(config_dir(&env(&[("PATH", "/bin")])), None);

    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("sessions")).unwrap();
    let registry = |session: &str| format!("{{\"pid\":42,\"sessionId\":{session:?},\"cwd\":\"/private/tmp/x.y\"}}");
    std::fs::write(dir.path().join("sessions/42.json"), registry("0a-b1")).unwrap();
    assert_eq!(
        super::mid_turn::transcript_in(dir.path(), 42),
        Some(dir.path().join("projects/-private-tmp-x-y/0a-b1.jsonl"))
    );
    std::fs::write(dir.path().join("sessions/42.json"), registry("../../etc/passwd")).unwrap();
    assert_eq!(super::mid_turn::transcript_in(dir.path(), 42), None);
    assert_eq!(super::mid_turn::transcript_in(dir.path(), 43), None);
}
