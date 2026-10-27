//! `terminal conversation-check <terminal>` (ov-830): every check an app
//! makes before it offers a pane the conversation view, with what this
//! runner says for each, pass or fail, and the reason an app shows. Each
//! check is named as the apps name it in their dimmed switch's reason
//! (AgentKit's and Android's `Unavailable.check`), so a reason read off a
//! screen finds its line here.
//!
//! It reads the runner's own word: the pane's record (`terminal.list`), the
//! hello's capabilities, and one page of the pane's rows. That page is what
//! a view asks for first, so it opens the pane's projector as a view would,
//! for the runner's linger (`session_projectors::LINGER`), and nothing else.

use std::time::Duration;

use farcooler_protocol::capability;
use farcooler_protocol::v1::{AgentRowPage, PaneMode, Terminal, TerminalRole, TerminalState, request, result};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::daemon_link::Link;
use crate::{expect_value, id_bytes, req, with};

/// Why an app doesn't offer the view, in the words and under the check names
/// AgentKit's `AgentConversation.Unavailable` uses for the same case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reason {
    NotAnAgent,
    NotRunning,
    RunnerNeedsUpdate,
}

impl Reason {
    pub(crate) fn check(self) -> &'static str {
        match self {
            Reason::NotAnAgent => "Pane",
            Reason::NotRunning => "Agent running",
            Reason::RunnerNeedsUpdate => "Runner version",
        }
    }

    pub(crate) fn sentence(self) -> &'static str {
        match self {
            Reason::NotAnAgent => "This isn’t a Claude or Codex pane.",
            Reason::NotRunning => "The agent in this pane isn’t running.",
            Reason::RunnerNeedsUpdate => "This runner needs an update to show the conversation.",
        }
    }

    /// What an app's dimmed switch says (`Unavailable.note`).
    pub(crate) fn note(self) -> String {
        format!("{}: {}", self.check(), self.sentence())
    }
}

/// What the runner said about the pane, as the apps read it.
#[derive(Debug, Clone, Default)]
pub(crate) struct Facts {
    pub pane_mode: String,
    pub state: String,
    pub running_agent: Option<String>,
    /// What the pane was launched as (`command_preset`).
    pub program: String,
    pub title: String,
    pub role: String,
    pub agent_session_id: Option<String>,
    pub offered: Vec<String>,
    pub runner_build: String,
    /// The pane's rows: `None` where the runner doesn't serve them.
    pub rows: Option<Rows>,
    /// The runner was reached over `--runner`, where the Mac pairs.
    pub remote: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Rows {
    pub epoch: u64,
    pub count: usize,
}

/// One check: passed, failed, or not one an app makes (`None`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Line {
    pub check: &'static str,
    pub pass: Option<bool>,
    pub saw: String,
}

/// The agent the apps offer the view by (`AgentConversation.agent`): what
/// the runner sees running, else what the pane was launched as.
pub(crate) fn agent(f: &Facts) -> String {
    f.running_agent.clone().unwrap_or_else(|| if f.program.is_empty() { f.title.clone() } else { f.program.clone() })
}

/// Every check, in the order the apps make them, and the first that fails.
pub(crate) fn assess(f: &Facts) -> (Vec<Line>, Option<Reason>) {
    let agent = agent(f);
    let has = |c: &str| f.offered.iter().any(|o| o == c);
    let is_agent = agent.starts_with("claude") || agent.starts_with("codex");
    let pane = f.pane_mode == "terminal" && is_agent;
    let running = f.state == "running";
    let codex = agent.starts_with("codex");
    let served = has(capability::AGENT_ROWS) && has(capability::AGENT_COMPOSE) && (!codex || has(capability::CODEX_VIEW));
    let said = |a: &str| if a.is_empty() { "nothing".to_string() } else { a.to_string() };
    let running_said = match &f.running_agent {
        Some(a) => format!("runner sees {} running", said(a)),
        None => "runner doesn't say what runs".to_string(),
    };
    let yes = |c: &str| format!("{c} {}", if has(c) { "yes" } else { "no" });
    let mut runner = vec![yes(capability::AGENT_ROWS), yes(capability::AGENT_COMPOSE)];
    if codex {
        runner.push(yes(capability::CODEX_VIEW));
    }
    runner.push(format!("build {}", f.runner_build));
    let mut lines = vec![
        Line {
            check: "Pane",
            pass: Some(pane),
            saw: format!("{} pane; {running_said}; launched as {}", f.pane_mode, said(&f.program)),
        },
        Line { check: "Agent running", pass: Some(running), saw: f.state.clone() },
        Line { check: "Runner version", pass: Some(served), saw: runner.join(", ") },
        Line { check: "Connection", pass: Some(true), saw: if f.remote { "reached over ssh".into() } else { "reached this Mac's runner".into() } },
        Line {
            check: "Pairing",
            pass: None,
            saw: if f.remote {
                "the Mac pairs its own key with this runner: see Settings, under Devices".into()
            } else {
                "none needed on this Mac's own runner".into()
            },
        },
    ];
    lines.push(Line {
        check: "Conversation",
        pass: f.rows.map(|r| r.epoch != 0),
        saw: match f.rows {
            None => "not read: the runner doesn't serve rows".into(),
            Some(r) if r.epoch == 0 => "no conversation found for this pane yet: the view opens empty".into(),
            Some(r) => format!("found, {} row{} in the first page", r.count, if r.count == 1 { "" } else { "s" }),
        },
    });
    lines.push(Line {
        check: "Role",
        pass: None,
        saw: format!(
            "{}; session {}",
            if f.role.is_empty() { "none" } else { &f.role },
            f.agent_session_id.as_deref().filter(|s| !s.is_empty()).unwrap_or("none the runner launched it on")
        ),
    });
    let reason = if !pane {
        Some(Reason::NotAnAgent)
    } else if !running {
        Some(Reason::NotRunning)
    } else if !served {
        Some(Reason::RunnerNeedsUpdate)
    } else {
        None
    };
    (lines, reason)
}

fn pane_mode(t: &Terminal) -> &'static str {
    match PaneMode::try_from(t.pane_mode) {
        Ok(PaneMode::Terminal) | Ok(PaneMode::Unspecified) => "terminal",
        Ok(PaneMode::Changes) => "changes",
        Ok(PaneMode::Web) => "web",
        _ => "other",
    }
}

fn state(t: &Terminal) -> &'static str {
    match TerminalState::try_from(t.state) {
        Ok(TerminalState::Running) => "running",
        Ok(TerminalState::Starting) => "starting",
        Ok(TerminalState::Exited) => "exited",
        Ok(TerminalState::Error) => "error",
        Ok(TerminalState::Lost) => "lost",
        _ => "unknown",
    }
}

fn role(t: &Terminal) -> &'static str {
    match TerminalRole::try_from(t.role) {
        Ok(TerminalRole::Orchestrator) => "orchestrator",
        Ok(TerminalRole::Agent) => "agent",
        Ok(TerminalRole::Shell) => "shell",
        _ => "",
    }
}

async fn page(link: &mut Link, id: Uuid) -> Result<AgentRowPage, Box<dyn std::error::Error>> {
    let mut ask = with(
        req("agent.rows"),
        request::Payload::AgentRowsPage(farcooler_protocol::v1::AgentRowsPage { terminal_id: id_bytes(id), before: None, limit: 20, hint_rows: false, agent_id: String::new() }),
    );
    ask.required_capabilities.push(capability::AGENT_ROWS.to_string());
    match expect_value(link.call(ask).await?.value)? {
        result::Value::AgentRowPage(page) => Ok(page),
        _ => Err(crate::daemon_link::UNREADABLE.into()),
    }
}

/// Read what the runner `link` reaches says of `t`, check it, and print it.
pub(crate) async fn run(mut link: Link, t: &Terminal, remote: bool, json_out: bool) -> Result<(), Box<dyn std::error::Error>> {
    let id = crate::uuid_of(&t.id);
    let offered = link.daemon_capabilities().to_vec();
    let rows = if offered.iter().any(|c| c == capability::AGENT_ROWS) {
        let mut first = page(&mut link, id).await?;
        // A claude typed into a shell has its projector opened by the pane's
        // next tick, a second after this first ask: look once more.
        if first.epoch == 0 {
            tokio::time::sleep(Duration::from_millis(2500)).await;
            first = page(&mut link, id).await?;
        }
        Some(Rows { epoch: first.epoch, count: first.rows.len() })
    } else {
        None
    };
    let facts = Facts {
        pane_mode: pane_mode(t).into(),
        state: state(t).into(),
        running_agent: t.running_agent.clone(),
        program: t.command_preset.clone(),
        title: t.title.clone(),
        role: role(t).into(),
        agent_session_id: t.agent_session_id.clone(),
        offered,
        runner_build: link.daemon_build().to_string(),
        rows,
        remote,
    };
    let (lines, reason) = assess(&facts);
    let short = crate::short_bytes(&t.id);
    if json_out {
        let checks: Vec<Value> = lines.iter().map(|l| json!({ "check": l.check, "pass": l.pass, "saw": l.saw })).collect();
        println!(
            "{}",
            json!({
                "terminal": short,
                "offered": reason.is_none(),
                "unavailable": reason.map(|r| json!({ "check": r.check(), "sentence": r.sentence(), "note": r.note() })),
                "checks": checks,
            })
        );
        return Ok(());
    }
    println!("Conversation view for {short} ({})", t.title);
    for line in &lines {
        let mark = match line.pass {
            Some(true) => "PASS",
            Some(false) => "FAIL",
            None => "    ",
        };
        println!("  {mark}  {:<15} {}", line.check, line.saw);
    }
    match reason {
        Some(r) => println!("Apps show: {}", r.note()),
        None => println!("Apps show: the conversation view, offered"),
    }
    Ok(())
}

#[cfg(test)]
#[path = "conversation_check_tests.rs"]
mod tests;
