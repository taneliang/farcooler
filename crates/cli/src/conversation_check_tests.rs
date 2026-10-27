use super::*;

fn served() -> Vec<String> {
    vec!["agent_rows".into(), "agent_compose".into(), "projector_setting".into()]
}

/// The owner's orchestrator as `terminal.list` gives it (ov-830): claude
/// typed into a shell, labeled with its session's title, then adopted.
fn adopted() -> Facts {
    Facts {
        pane_mode: "terminal".into(),
        state: "running".into(),
        running_agent: Some("claude".into()),
        program: "shell".into(),
        title: "User test issues and onboarding flow (2)".into(),
        role: "orchestrator".into(),
        agent_session_id: None,
        offered: served(),
        runner_build: "test".into(),
        rows: Some(Rows { epoch: 3, count: 20 }),
        remote: false,
    }
}

fn failed(lines: &[Line]) -> Vec<&'static str> {
    lines.iter().filter(|l| l.pass == Some(false)).map(|l| l.check).collect()
}

/// Goes red with the agent read off `program` (launched as `shell`).
#[test]
fn an_adopted_orchestrator_passes_every_check() {
    let (lines, reason) = assess(&adopted());
    assert_eq!(reason, None, "{lines:?}");
    assert!(failed(&lines).is_empty(), "{lines:?}");
    let pane = lines.iter().find(|l| l.check == "Pane").unwrap();
    assert!(pane.saw.contains("runner sees claude running") && pane.saw.contains("launched as shell"), "{}", pane.saw);
    let role = lines.iter().find(|l| l.check == "Role").unwrap();
    assert!(role.saw.starts_with("orchestrator"), "{}", role.saw);
}

#[test]
fn a_launched_orchestrator_still_passes() {
    let facts = Facts { program: "claude".into(), title: "claude".into(), agent_session_id: Some("s1".into()), ..adopted() };
    assert_eq!(assess(&facts).1, None);
}

/// A runner from before ov-830 with its projector setting off: the check is
/// named and the note is the one AgentKit's `Unavailable.note` makes.
#[test]
fn an_old_runner_with_its_setting_off_fails_the_runner_version_check() {
    let facts = Facts { offered: vec!["agent_compose".into(), "projector_setting".into()], rows: None, ..adopted() };
    let (lines, reason) = assess(&facts);
    assert_eq!(reason, Some(Reason::RunnerNeedsUpdate));
    assert_eq!(failed(&lines), vec!["Runner version"]);
    assert_eq!(reason.unwrap().note(), "Runner version: This runner needs an update to show the conversation.");
    let conversation = lines.iter().find(|l| l.check == "Conversation").unwrap();
    assert_eq!(conversation.pass, None, "not read where rows aren't served");
}

#[test]
fn the_pane_comes_first() {
    let shell = Facts { running_agent: Some(String::new()), ..adopted() };
    assert_eq!(assess(&shell).1, Some(Reason::NotAnAgent), "the runner looked and nothing runs");
    let exited = Facts { state: "exited".into(), ..adopted() };
    assert_eq!(assess(&exited).1, Some(Reason::NotRunning));
    let codex = Facts { running_agent: Some("codex".into()), ..adopted() };
    assert_eq!(assess(&codex).1, Some(Reason::RunnerNeedsUpdate), "a runner from before codex's view");
}

/// Offered, with no conversation found: the apps open the view, and this
/// says it will be empty, rather than passing it silently.
#[test]
fn a_pane_whose_conversation_isnt_found_says_the_view_opens_empty() {
    let facts = Facts { rows: Some(Rows { epoch: 0, count: 0 }), ..adopted() };
    let (lines, reason) = assess(&facts);
    assert_eq!(reason, None);
    assert_eq!(failed(&lines), vec!["Conversation"]);
}
