//! One real turn, through the backend a pane would use.
//!
//! The fixture test proves the normalizer against frames captured earlier.
//! This proves the whole path — spawn, initialize, thread/start, turn/start,
//! and the events coming back — against the codex installed right now.
//!
//! A missing `codex` is a FAILURE, not a skip, for the reason the ACP
//! handshake test gives: on a machine without the agent installed, silently
//! passing would mean the one test that can catch a broken backend never runs
//! where it matters.
//!
//! Only on request: it is `#[ignore]` and also needs `FARCOOLER_LIVE_AGENTS=1`
//! (see `test/live_agents.rs`), because a lane running the crate's tests must
//! never start a real, signed-in codex.
//!
//! ```text
//! FARCOOLER_LIVE_AGENTS=1 cargo test -p farcooler-codex --test live_turn -- --ignored
//! ```

#[path = "../../../test/live_agents.rs"]
mod live;

use farcooler_agent_core::backend::{AgentBackend, Launch};
use farcooler_agent_core::event::{AgentEvent, Role};
use farcooler_codex::backend::CodexBackend;

#[tokio::test]
#[ignore = "starts the real codex; run with FARCOOLER_LIVE_AGENTS=1 and --ignored"]
async fn a_real_turn_reaches_the_agent_and_comes_back_as_a_conversation() {
    if !live::enabled("a_real_turn_reaches_the_agent_and_comes_back_as_a_conversation") {
        return;
    }
    // The installed codex, resolved the way the daemon resolves it.
    let program = live::installed("codex");
    let worktree = std::env::temp_dir();
    let launch = Launch { program, args: Vec::new(), env: Default::default() };

    let (mut backend, prelude) = CodexBackend::start(&launch, worktree, None)
        .await
        .unwrap_or_else(|e| panic!("codex backend would not start: {e}"));

    // The session announces itself, and says which protocol is carrying it.
    let started = prelude.iter().find_map(|e| match e {
        AgentEvent::SessionStarted { session_id, backend, .. } => Some((session_id, backend)),
        _ => None,
    });
    let (session_id, wire) = started.expect("a session has to announce itself");
    assert!(!session_id.is_empty(), "a thread id is what a reconnect resumes by");
    assert_eq!(wire, "codex", "the pane has to be able to say what it is talking to");

    assert!(backend.capabilities().native_steer, "turn/steer is real, so do not emulate it");

    backend
        .prompt(live::PROMPT, &[])
        .await
        .expect("a prompt has to reach the agent");

    // Bounded: a turn that never ends is the failure this would otherwise
    // present as a hung test with nothing to read.
    let collected = tokio::time::timeout(std::time::Duration::from_secs(120), async {
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
    assert!(!spoken.trim().is_empty(), "the agent said nothing: {collected:?}");
    // Some text is not an answer: a logged-out or failing codex turn ends
    // normally with its error as agent text. See `live::assert_answered`.
    live::assert_answered("codex", &spoken);

    let gaps: Vec<_> = collected
        .iter()
        .filter(|e| matches!(e, AgentEvent::Gap { .. }))
        .collect();
    assert!(gaps.is_empty(), "a real turn produced unmapped frames: {gaps:?}");
}
