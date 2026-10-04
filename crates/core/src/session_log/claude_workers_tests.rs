//! What a claude session says about its subagents, read off records cut from
//! a real one (`fixtures/session-logs/claude-subagent-notifications.jsonl`:
//! prompts, summaries and results removed, ids and times synthetic).

use super::claude::parse_line;
use super::{SubagentStatus, TurnEvent};

const SESSION: &str = include_str!("../../fixtures/session-logs/claude-subagent-notifications.jsonl");

fn line(n: usize) -> &'static str {
    SESSION.lines().nth(n).expect("the fixture has that many lines")
}

fn ended(n: usize) -> Vec<(String, SubagentStatus)> {
    parse_line(line(n))
        .into_iter()
        .filter_map(|e| match e {
            TurnEvent::SubagentEnded { agent_id, status } => Some((agent_id, status)),
            _ => None,
        })
        .collect()
}

/// The spawn names the subagent, and its result gives the id everything else
/// uses: `SendMessage`, the transcript's file name, a notification's `task-id`.
#[test]
fn a_background_launch_gives_the_agent_id_for_the_spawn() {
    match parse_line(line(0)).as_slice() {
        [TurnEvent::Did { .. }, TurnEvent::Subagent { id, description, running: true }] => {
            assert_eq!(id, "toolu_000000000000000000000001");
            assert_eq!(description, "ov-12: polish the sidebar");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        parse_line(line(1)).into_iter().filter(|e| matches!(e, TurnEvent::SubagentLaunched { .. })).collect::<Vec<_>>(),
        [TurnEvent::SubagentLaunched {
            id: "toolu_000000000000000000000001".into(),
            agent_id: "a0000000000000001".into()
        }]
    );
    assert!(ended(1).is_empty(), "a launch is not an ending");
}

/// Each of the four statuses a notification carries, as written, with the id
/// the subagent is known by.
#[test]
fn a_notification_ends_the_agent_it_names_with_each_of_the_four_statuses() {
    assert_eq!(ended(2), [("a0000000000000001".into(), SubagentStatus::Completed)]);
    assert_eq!(ended(3), [("a0000000000000002".into(), SubagentStatus::Failed)]);
    assert_eq!(ended(4), [("a0000000000000003".into(), SubagentStatus::Killed)]);
    assert_eq!(ended(5), [("a0000000000000004".into(), SubagentStatus::Stopped)]);
}

/// The same notification is written three times (a `queue-operation`, an
/// `attachment`, a `user` turn), and any one of them says it.
#[test]
fn the_attachment_and_the_user_turn_say_it_too() {
    assert_eq!(ended(7), [("a0000000000000001".into(), SubagentStatus::Completed)]);
    assert_eq!(ended(8), [("a0000000000000001".into(), SubagentStatus::Completed)]);
}

/// A monitor's notification has an `<event>` and no `<status>`: nothing
/// stopped.
#[test]
fn a_monitors_notification_ends_nothing() {
    assert!(parse_line(line(6)).is_empty());
}

/// A person quoting a notification in a prompt is not one: it opens with the
/// tag or it isn't read. And an `enqueue`'s twin, `remove`, carries no text.
#[test]
fn only_a_record_that_opens_with_the_tag_is_a_notification() {
    let quoted = r#"{"type":"user","message":{"role":"user","content":"why did <task-notification><task-id>a1</task-id><status>failed</status></task-notification> fire"}}"#;
    assert!(ended_in(quoted).is_empty());
    let remove = r#"{"type":"queue-operation","operation":"remove","timestamp":"2026-09-30T06:12:00.000Z"}"#;
    assert!(parse_line(remove).is_empty());
}

fn ended_in(record: &str) -> Vec<TurnEvent> {
    parse_line(record).into_iter().filter(|e| matches!(e, TurnEvent::SubagentEnded { .. })).collect()
}

/// A message sent to a stopped subagent comes back naming the one it woke.
#[test]
fn a_resume_names_the_agent_that_is_working_again() {
    assert!(parse_line(line(9)).contains(&TurnEvent::SubagentResumed { agent_id: "a0000000000000001".into() }));
}
