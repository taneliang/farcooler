use farcooler_core::session_log::{TaskStatus, TurnEvent, TurnOutcome};
use farcooler_protocol::v1::Terminal;

use super::super::Signals;

fn spawned(signals: &mut Signals, id: &str, description: &str) {
    signals.saw(&TurnEvent::Subagent { id: id.into(), description: description.into(), running: true });
}

fn stamped(signals: &Signals) -> Terminal {
    let mut message = Terminal::default();
    signals.stamp_subagents(&mut message);
    message
}

/// Three background agents launched, then the turn ends: the names go with
/// the turn, and the count claude states for what it left running is what
/// the record says, so the card doesn't collapse while they run.
#[test]
fn the_count_survives_the_turn_that_started_them() {
    let mut signals = Signals::default();
    signals.saw(&TurnEvent::Started { at_ms: None });
    spawned(&mut signals, "toolu_1", "Auditing the redaction rules");
    spawned(&mut signals, "toolu_2", "Checking the join");
    spawned(&mut signals, "toolu_3", "Reading the tray");
    let during = stamped(&signals);
    assert_eq!(during.subagents.len(), 3);
    assert_eq!(during.subagents_running, Some(3));

    signals.saw(&TurnEvent::Ended { at_ms: None, duration_ms: None, outcome: TurnOutcome::Finished });
    signals.saw(&TurnEvent::BackgroundAgents(3));
    let after = stamped(&signals);
    assert!(after.subagents.is_empty(), "the names are the turn's, and it ended");
    assert!(after.subagent_ids.is_empty());
    assert_eq!(after.subagents_running, Some(3), "the agents still running were dropped from the count");
}

/// Each name carries the id of the call behind it, in order, so a client
/// joins its rows on the id. The name is cleaned on the way out (a path in
/// it is narrowed), so it no longer reads as what the rows say.
#[test]
fn each_name_carries_its_calls_id() {
    let mut signals = Signals::default();
    signals.saw(&TurnEvent::Started { at_ms: None });
    spawned(&mut signals, "toolu_a", "Read /Users/someone/Dev/acme/crates/daemon/src/watch.rs closely");
    spawned(&mut signals, "toolu_b", "");
    spawned(&mut signals, "toolu_c", "Checking the join");
    let message = stamped(&signals);
    assert_eq!(message.subagent_ids, vec!["toolu_a", "toolu_c"], "an unnamed agent has no name to carry an id for");
    assert_eq!(message.subagents.len(), message.subagent_ids.len());
    assert_ne!(
        message.subagents[0], "Read /Users/someone/Dev/acme/crates/daemon/src/watch.rs closely",
        "the name went out uncleaned"
    );
    assert_eq!(message.subagents_running, Some(3), "the unnamed one still counts");
}

/// Woken by one of the agents it left running, the orchestrator's next turn
/// starts a fourth while three still run: the three carried over from the
/// last turn's count and this turn's one, four in all, not the larger of the
/// two and not the three dropped at the start of the turn.
#[test]
fn a_new_turn_adds_its_spawns_to_the_agents_still_running() {
    let mut signals = Signals::default();
    signals.saw(&TurnEvent::Started { at_ms: None });
    spawned(&mut signals, "toolu_1", "Auditing the redaction rules");
    spawned(&mut signals, "toolu_2", "Checking the join");
    spawned(&mut signals, "toolu_3", "Reading the tray");
    signals.saw(&TurnEvent::Ended { at_ms: None, duration_ms: None, outcome: TurnOutcome::Finished });
    signals.saw(&TurnEvent::BackgroundAgents(3));

    signals.saw(&TurnEvent::Started { at_ms: None });
    assert_eq!(stamped(&signals).subagents_running, Some(3), "the three still running were dropped as the turn began");
    spawned(&mut signals, "toolu_4", "Writing the fix");
    // A task list, which is what puts the count on the line.
    signals.saw(&TurnEvent::TaskState { id: None, active_form: Some("Fixing the tray".into()), status: None });
    signals.saw(&TurnEvent::TaskState { id: Some("1".into()), active_form: None, status: Some(TaskStatus::InProgress) });
    let message = stamped(&signals);
    assert_eq!(message.subagents, vec!["Writing the fix"], "the names are this turn's");
    assert_eq!(message.subagents_running, Some(4));
    assert_eq!(signals.line(true).as_deref(), Some("0/1 · 4 agents"), "the line says another count");

    // The turn ends and claude restates the count: it replaces the carried one.
    signals.saw(&TurnEvent::Ended { at_ms: None, duration_ms: None, outcome: TurnOutcome::Finished });
    signals.saw(&TurnEvent::BackgroundAgents(2));
    assert_eq!(stamped(&signals).subagents_running, Some(2));
}
