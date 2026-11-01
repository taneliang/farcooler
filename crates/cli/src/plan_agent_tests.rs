//! `farcooler agent` (ov-503, ov-872): the agents at work, and recording a
//! subagent and its work on tickets, against the plan tests' fake runner.

use super::*;

/// `farcooler agent {args}`, run in `session`.
async fn agent_in(link: &mut Runner, args: &str, json: bool, session: &agent::Session) -> Result<String, Failed> {
    let argv = format!("farcooler agent {args} --workspace main");
    let cmd = match crate::Cli::try_parse_from(argv.split_whitespace()).unwrap_or_else(|e| panic!("{argv}: {e}")).command {
        crate::Command::Agent(a) => a.cmd,
        _ => panic!("not agent"),
    };
    agent::run_in(link, &the_board(), cmd, "manager", json, session).await
}

async fn agent_say(link: &mut Runner, args: &str) -> Result<String, Failed> {
    agent_in(link, args, false, &agent::Session::default()).await
}

/// `agent list` leads with the day's spend and lists each agent: working
/// first, its role, model, title, tickets, what it's doing now and its spend.
#[tokio::test]
async fn the_list_says_who_is_working_on_what() {
    let mut link = runner();
    let text = agent_say(&mut link, "list").await.unwrap();
    let expected = [
        "1 working · 2 agents · 3M tokens in the last 24 hours · about $20.00 API-equivalent",
        "  \u{25cf} Lander sonnet (0000a002) \u{201c}Land the Mac polish\u{201d}  ov-1  working · Running the gate  120K tokens · about $4.00 API-equivalent",
        "  \u{25cb} Orchestrator opus (0000a001)  The plan  idle · active 5m ago  2.1M tokens · about $14.20 API-equivalent",
    ];
    assert_eq!(text.lines().collect::<Vec<_>>(), expected, "{text}");
    assert_eq!(link.sent.iter().map(|r| r.method.as_str()).collect::<Vec<_>>(), ["plan.get"], "one read");
}

/// With none at work the day's spend still reads, and with nothing at all it
/// says so rather than printing nothing.
#[tokio::test]
async fn no_agents_says_so() {
    let mut link = Runner { plan: Some(pb::Plan { agents: vec![], ..the_plan() }), ..runner() };
    assert_eq!(
        agent_say(&mut link, "list").await.unwrap(),
        "No agents at work · 3M tokens in the last 24 hours · about $20.00 API-equivalent"
    );
    let mut link = Runner { plan: Some(pb::Plan { agents: vec![], agents_spend: None, ..the_plan() }), ..runner() };
    assert_eq!(agent_say(&mut link, "list").await.unwrap(), "No agents at work.");
}

/// `agent list --json` carries the same agents and spend `plan --json` does.
#[tokio::test]
async fn the_list_json_is_the_plan_s_agents() {
    let mut link = runner();
    let out = agent_in(&mut link, "list", true, &agent::Session::default()).await.unwrap();
    let v: Value = serde_json::from_str(&out).unwrap();
    let whole = farcooler_client::plan_json::plan_json(&the_plan());
    assert_eq!(v["agents"], whole["agents"]);
    assert_eq!(v["agents_spend"], whole["agents_spend"]);
    assert_eq!(v["agents"][0]["tickets"][0]["key"], "ov-1");
    assert_eq!(v["now_ms"], NOW);
}

/// `agent record` sends the subagent, its role, harness, model, title and
/// tickets by id, behind `board_line`; its parent pane and claude session are
/// read from the session it runs in.
#[tokio::test]
async fn record_takes_the_parent_and_session_from_the_pane() {
    let pane = Uuid::from_u128(0xbeef);
    let session = agent::Session { actor: Some(format!("agent:{pane}")), claude_session: Some("sess-1".into()) };
    let mut link = runner();
    let said = agent_in(&mut link, "record --role reviewer --subagent s1 --model opus --title Review --ticket ov-2", false, &session)
        .await
        .unwrap();
    assert_eq!(said, "Recorded Reviewer opus (0000a100), working ov-2.");
    let r = link.sent.iter().find(|r| r.method == "agent.record").expect("recorded");
    assert_eq!(r.required_capabilities, [capability::BOARD_LINE.to_string()]);
    let Some(request::Payload::AgentRecord(p)) = &r.payload else { panic!() };
    assert_eq!((p.role, p.harness.as_str(), p.subagent_id.as_str()), (pb::AgentRole::Reviewer as i32, "claude", "s1"));
    assert_eq!((p.model.as_deref(), p.title.as_deref()), (Some("opus"), Some("Review")));
    assert_eq!(p.task_ids, [tid(0x1002)]);
    assert_eq!(p.workspace_id, id_bytes(WORKSPACE));
    assert_eq!((p.parent_terminal_id.clone(), p.session_id.as_deref()), (Some(id_bytes(pane)), Some("sess-1")));
    assert_eq!((p.terminal_id.clone(), p.actor.as_str()), (None, "manager"));

    // A codex subagent has no claude session to name; --parent names the pane by its short id.
    let mut link = runner();
    agent_in(&mut link, "record --role builder --subagent c1 --harness Codex --parent 0000d00d", false, &session).await.unwrap();
    let r = link.sent.iter().find(|r| r.method == "agent.record").unwrap();
    let Some(request::Payload::AgentRecord(p)) = &r.payload else { panic!() };
    assert_eq!((p.harness.as_str(), p.session_id.as_deref()), ("codex", None));
    assert_eq!(p.parent_terminal_id, Some(tid(0xd00d)));

    // Outside a pane there is no parent unless one is named.
    let mut link = runner();
    agent_say(&mut link, "record --role lander --subagent l1").await.unwrap();
    let r = link.sent.iter().find(|r| r.method == "agent.record").unwrap();
    let Some(request::Payload::AgentRecord(p)) = &r.payload else { panic!() };
    assert_eq!((p.parent_terminal_id.clone(), p.session_id.clone(), p.task_ids.len()), (None, None, 0));
    assert!(crate::Cli::try_parse_from(["farcooler", "agent", "record", "--role", "builder"]).is_err(), "a subagent is named");
}

/// `agent work` names the agent by its short id and the ticket by key; with a
/// verdict it ends that work.
#[tokio::test]
async fn work_starts_and_ends_an_agent_s_work_on_a_ticket() {
    let mut link = runner();
    assert_eq!(agent_say(&mut link, "work 0000a002 --ticket ov-1 --role land").await.unwrap(), "Lander sonnet (0000a002) is landing ov-1.");
    let r = last(&link);
    assert_eq!((r.method.as_str(), r.required_capabilities.clone()), ("agent.work", vec![capability::BOARD_LINE.to_string()]));
    let Some(request::Payload::AgentWork(p)) = &r.payload else { panic!() };
    assert_eq!((p.agent_id.clone(), p.task_id.clone(), p.role, p.verdict.clone()), (tid(LANDER), tid(0x1001), pb::WorkRole::Land as i32, None));

    let said = agent_say(&mut link, "work a002 --ticket ov-1 --role review --verdict changes_asked").await.unwrap();
    assert_eq!(said, "Lander sonnet (0000a002) is done reviewing ov-1: asked for changes.");
    let Some(request::Payload::AgentWork(p)) = &last(&link).payload else { panic!() };
    assert_eq!(p.verdict.as_deref(), Some("changes_asked"));
    let full = Uuid::from_u128(LANDER).to_string();
    agent_say(&mut link, &format!("work {full} --ticket ov-1 --role land --verdict handed-back")).await.unwrap();
    let Some(request::Payload::AgentWork(p)) = &last(&link).payload else { panic!() };
    assert_eq!((p.agent_id.clone(), p.verdict.as_deref()), (tid(LANDER), Some("handed_back")));
}

/// `agent end` ends the agent and every ticket it still works with one verdict.
#[tokio::test]
async fn end_finishes_an_agent() {
    let mut link = runner();
    assert_eq!(agent_say(&mut link, "end a002 --verdict handed_back").await.unwrap(), "Lander sonnet (0000a002) has ended: handed back.");
    let Some(request::Payload::AgentEnd(p)) = &last(&link).payload else { panic!() };
    assert_eq!((p.agent_id.clone(), p.verdict.as_str(), p.actor.as_str()), (tid(LANDER), "handed_back", "manager"));
    assert!(crate::Cli::try_parse_from(["farcooler", "agent", "end", "a002", "--verdict", "approved"]).is_err(), "an agent ends, it isn't approved");
}

/// An agent nobody can find is said before anything is written, and the
/// runner's refusals read in words.
#[tokio::test]
async fn an_unknown_agent_and_a_refusal_are_said_in_words() {
    let mut link = runner();
    let err = agent_say(&mut link, "end zzzz --verdict finished").await.unwrap_err();
    assert_eq!(err.to_string(), "No agent at work here is \"zzzz\". `farcooler agent list` shows them.");
    assert!(link.sent.iter().all(|r| r.method == "plan.get"));

    link.refuse = Some("role");
    let err = agent_say(&mut link, "work a002 --ticket ov-1 --role build").await.unwrap_err();
    assert!(err.to_string().starts_with("That work doesn't fit the agent"), "{err}");
    link.refuse = Some("place");
    let err = agent_say(&mut link, "record --role builder --subagent x").await.unwrap_err();
    assert_eq!(err.to_string(), "Name the subagent with --subagent, the id from its launch result.");
    link.refuse = Some("agent_ended");
    let err = agent_say(&mut link, "end a002 --verdict finished").await.unwrap_err();
    assert_eq!(err.to_string(), "That agent has already finished.");
}

/// A runner without the line is told before anything is sent.
#[tokio::test]
async fn an_old_runner_is_told_first() {
    let mut link = runner();
    link.capabilities.retain(|c| c != capability::BOARD_LINE);
    for args in ["list", "end a002 --verdict finished", "record --role builder --subagent x"] {
        let err = agent_say(&mut link, args).await.unwrap_err();
        assert_eq!(err.to_string(), NEEDS_LINE, "{args}");
    }
    assert!(link.sent.is_empty());
}

/// `agent stop` names the agent by a ticket it works, or its short id, sends
/// the reason behind `agent_stop`, and says what happens next: a subagent is
/// told to stop at its next tool call, after any command already running; a
/// pane stops (ov-872).
#[tokio::test]
async fn stop_says_what_happens_to_a_subagent_and_a_pane() {
    let mut link = runner();
    let said = agent_say(&mut link, "stop ov-1 --reason owner").await.unwrap();
    assert_eq!(
        said,
        "Asked Lander sonnet (0000a002) to stop. Its next tool call is refused with a message telling it to stop and hand back. \
         A command it's already running finishes first, so it shows as stopping until it ends."
    );
    let r = last(&link);
    assert_eq!((r.method.as_str(), r.required_capabilities.clone()), ("agent.stop", vec![capability::AGENT_STOP.to_string()]));
    let Some(request::Payload::AgentStop(p)) = &r.payload else { panic!() };
    assert_eq!((p.agent_id.clone(), p.reason.as_deref(), p.actor.as_str()), (tid(LANDER), Some("owner"), "manager"));

    let said = agent_say(&mut link, "stop a001").await.unwrap();
    assert_eq!(said, "Stopped Orchestrator opus (0000a001): its pane has stopped.");
    let Some(request::Payload::AgentStop(p)) = &last(&link).payload else { panic!() };
    assert_eq!((p.agent_id.clone(), p.reason.clone()), (tid(ORCHESTRATOR), None));
}

/// A harness the runner can't reach is said honestly, with what to do
/// instead; a runner from before `agent.stop` is told before anything is sent.
#[tokio::test]
async fn stop_says_when_it_cant() {
    let mut link = runner();
    link.refuse = Some("stop_unsupported");
    let err = agent_say(&mut link, "stop a002").await.unwrap_err();
    assert!(err.to_string().starts_with("Far Cooler can stop a pane, or a Claude subagent, but not this harness's subagents."), "{err}");
    assert!(err.to_string().contains("stop the orchestrator's pane"), "{err}");

    let mut link = runner();
    link.capabilities.retain(|c| c != capability::AGENT_STOP);
    let err = agent_say(&mut link, "stop a002").await.unwrap_err();
    assert_eq!(err.to_string(), agent::NEEDS_STOP);
    assert!(link.sent.is_empty());
}
