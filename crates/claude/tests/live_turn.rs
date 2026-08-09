//! One real turn, through the backend a pane would use.
//!
//! A missing `claude` is a FAILURE, not a skip, for the reason the ACP
//! handshake test gives: on a machine without the agent installed, silently
//! passing means the one test that can catch a broken backend never runs where
//! it matters.

use farcooler_agent_core::backend::{AgentBackend, Launch};
use farcooler_agent_core::event::{AgentEvent, Role};
use farcooler_claude::backend::ClaudeBackend;

#[tokio::test]
async fn a_real_turn_reaches_claude_and_comes_back_as_a_conversation() {
    let launch = Launch { program: which_claude(), args: Vec::new(), env: Default::default() };

    let (mut backend, prelude) = ClaudeBackend::start(&launch, std::env::temp_dir(), None)
        .await
        .unwrap_or_else(|e| panic!("claude backend would not start: {e}"));

    let started = prelude.iter().find_map(|e| match e {
        AgentEvent::SessionStarted { session_id, backend, available_commands, .. } => {
            Some((session_id, backend, available_commands))
        }
        _ => None,
    });
    let (session_id, wire, commands) = started.expect("a session has to announce itself");
    assert!(!session_id.is_empty(), "a session id is what a reconnect resumes by");
    assert_eq!(wire, "claude", "the pane has to be able to say what it is talking to");
    assert!(!commands.is_empty(), "the slash-command menu came through");

    // This also proves the CLAUDECODE scrub: the test usually runs from inside
    // a Claude Code session, and without it the CLI never answers.
    backend.prompt("Reply with exactly: hi", &[]).await.expect("a prompt has to land");

    let collected = tokio::time::timeout(std::time::Duration::from_secs(180), async {
        let mut seen = Vec::new();
        loop {
            let events = match backend.next_events().await {
                Ok(events) => events,
                Err(e) => panic!("the backend died mid-turn: {e}"),
            };
            let ended = events.iter().any(|e| matches!(e, AgentEvent::TurnEnded { .. }));
            seen.extend(events);
            if ended {
                return seen;
            }
        }
    })
    .await
    .expect("the turn has to end or a pane says Working forever");

    let spoken: String = collected
        .iter()
        .filter_map(|e| match e {
            AgentEvent::Message { role: Role::Agent, text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(!spoken.trim().is_empty(), "claude said nothing: {collected:?}");

    // The prompt must NOT come back — the client already drew it. This is the
    // doubling that showed up on codex, asserted here so it cannot appear.
    let echoed: Vec<_> = collected
        .iter()
        .filter(|e| matches!(e, AgentEvent::Message { role: Role::User, .. }))
        .collect();
    assert!(echoed.is_empty(), "the prompt was echoed back and would render twice: {echoed:?}");

    let gaps: Vec<_> = collected.iter().filter(|e| matches!(e, AgentEvent::Gap { .. })).collect();
    assert!(gaps.is_empty(), "a real turn produced unmapped frames: {gaps:?}");
}

fn which_claude() -> std::path::PathBuf {
    let out = std::process::Command::new("sh")
        .args(["-lc", "command -v claude"])
        .output()
        .expect("sh must run");
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert!(!path.is_empty(), "claude must be installed for this test to mean anything");
    std::path::PathBuf::from(path)
}
