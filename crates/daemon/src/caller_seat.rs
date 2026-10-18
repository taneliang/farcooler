//! Who a local caller is, from its process (ov-620): a person, an agent, or
//! unknown. A person's words are typed as their own turn
//! (`watch::answer_wake::route`), and an agent's go by the mailbox, so this is
//! read where a request arrives, never from what the request claims: a claude
//! in a shell pane has no `FARCOOLER_ACTOR`, so its CLI claims `user`.
//!
//! **The walk.** From the caller up its ancestors, each one's command line
//! read now (`proc_table::command_of`; the snapshot has one only for a
//! terminal's foreground group, and a caller's ancestors seldom are), until it
//! meets one of this runner's panes, this runner's tmux server, or pid 1:
//! - an agent process anywhere on that chain (claude, codex or cursor-agent,
//!   by its program or the script node runs, as the watcher recognizes one:
//!   `running_agent`, `agent_of_process`) makes it an agent's, whatever the
//!   walk met. So a claude outside Far Cooler, and an agent in a pane the
//!   inventory hasn't seen yet, are agents (review 1, B5);
//! - a pane launched as an agent is an agent's too;
//! - in any other pane (a shell, an adopted orchestrator), only shells and the
//!   CLI on the chain make it a person's, typing in their own shell pane
//!   (review 1, B3); a link whose command line can't be read makes it unknown;
//! - this runner's tmux server met with no pane below it (a pane just made, an
//!   agent just respawned, tmux inside tmux) is unknown (N2). The server is
//!   known by its pid, the parent of the panes' own processes, or tmux's word;
//! - pid 1 met with no agent on the chain is a person (a terminal outside Far
//!   Cooler, an ssh session, whose root-owned ancestors can't be read and
//!   aren't asked), unless the caller's terminal is one of this runner's panes:
//!   an orphan the pane's shell left behind is unknown.
//!
//! Unknown is never a person: a Prompt from it is refused `in_pane`, which the
//! CLI explains, rather than sent another way. An agent this runner doesn't
//! recognize (gemini, aider) in a shell pane reads as a person; Far Cooler has
//! no mailbox for one, so it can't be told anything either way.
//!
//! **A guardrail, not a boundary.** An agent that detaches from its pane
//! (`setsid`, then orphaned) or reaches the runner over `ssh localhost` reads
//! as a person, and Control scope can type into any pane anyway
//! (`terminal.write`). This stops an agent's words being taken for the
//! person's by mistake; it doesn't stop an agent set on it.

use std::collections::{HashMap, HashSet};

use uuid::Uuid;

/// Who a caller is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seat {
    /// A person: in no pane, or in their own shell pane with no agent above.
    Person,
    /// An agent, in this pane when it's one of the runner's.
    Agent(Option<Uuid>),
    /// Not known: never taken for a person.
    Unreadable,
}

/// One row of the process table: its parent and its terminal.
#[derive(Debug, Clone)]
pub struct Proc {
    pub ppid: i32,
    pub tty: String,
}

/// A pane by its root process: its terminal, and whether it was launched as
/// an agent.
pub type Panes = HashMap<i32, (Uuid, bool)>;

/// What the walk knows of the runner.
pub struct Runner<'a> {
    pub panes: &'a Panes,
    /// The tmux server's pid.
    pub servers: &'a HashSet<i32>,
    /// The panes' terminals, `ttys012`.
    pub ttys: &'a HashSet<String>,
}

/// The seat of the local caller `pid` (`None`: a remote device, a person).
pub async fn caller_seat(svc: &crate::service::Service, pid: Option<i32>) -> Seat {
    let Some(pid) = pid else { return Seat::Person };
    let store = &svc.store;
    let snapshot = svc.inventory_snapshot();
    let panes: Panes = snapshot
        .panes
        .iter()
        .map(|p| {
            let launched = store.get_terminal(p.terminal_id).is_ok_and(|t| crate::watch::answer_wake::is_an_agent_preset(&t.command_preset));
            (p.stamp.pid as i32, (p.terminal_id, launched))
        })
        .collect();
    let ttys: HashSet<String> = snapshot.panes.iter().map(|p| p.tty.trim_start_matches("/dev/").to_string()).collect();
    let Some(table) = crate::proc_table::snapshot() else { return Seat::Unreadable };
    let procs = procs(&table);
    let mut servers: HashSet<i32> = panes.keys().filter_map(|root| procs.get(root)).map(|row| row.ppid).filter(|&p| p > 1).collect();
    if servers.is_empty()
        && let Ok(out) = svc.tmux.run(&["display-message", "-p", "#{pid}"]).await
        && let Ok(server) = out.stdout.trim().parse()
    {
        servers.insert(server);
    }
    let registry = svc.registry();
    let agent = |pid: i32| crate::proc_table::command_of(pid).map(|(exe, args)| is_agent(&registry, &exe, &args));
    stubbed(seat_in(pid, &procs, &Runner { panes: &panes, servers: &servers, ttys: &ttys }, &agent), crate::service::test_stub_agents())
}

/// A runner that launches only stubs (`service::test_stub_agents`, a test
/// harness's scratch daemon, which refuses a real agent) has no agent of its
/// own outside its panes to tell apart: an agent above the caller there is
/// the harness's own (a Mac test run from a terminal inside a claude pane),
/// and the caller is the test standing in for a person. Inside its panes,
/// where its stand-ins run, the walk stands.
pub fn stubbed(seat: Seat, stub_agents: bool) -> Seat {
    match seat {
        Seat::Agent(None) if stub_agents => Seat::Person,
        seat => seat,
    }
}

/// `proc_table::snapshot`'s rows (`pid ppid pgid tty stat args…`), by pid.
pub fn procs(table: &str) -> HashMap<i32, Proc> {
    table
        .lines()
        .filter_map(|line| {
            let mut f = line.split_whitespace();
            let pid = f.next()?.parse().ok()?;
            let ppid = f.next()?.parse().ok()?;
            let tty = f.nth(1)?.to_string();
            Some((pid, Proc { ppid, tty }))
        })
        .collect()
}

/// The walk, on a table already read. `agent` says whether a process is an
/// agent, `None` when its command line can't be read. See this module's docs.
pub fn seat_in(pid: i32, procs: &HashMap<i32, Proc>, runner: &Runner, agent: &dyn Fn(i32) -> Option<bool>) -> Seat {
    let mut chain = Vec::new();
    let mut pane = None;
    let mut ended = End::One;
    let mut at = pid;
    loop {
        if chain.len() >= 64 {
            ended = End::Hole;
            break;
        }
        if let Some(&found) = runner.panes.get(&at) {
            chain.push(at);
            pane = Some(found);
            break;
        }
        if runner.servers.contains(&at) {
            ended = End::Server;
            break;
        }
        let Some(row) = procs.get(&at) else {
            ended = End::Hole;
            break;
        };
        chain.push(at);
        if row.ppid <= 1 || row.ppid == at {
            break;
        }
        at = row.ppid;
    }
    // An agent anywhere on the chain, however the walk ended.
    let mut unknown = false;
    for p in &chain {
        match agent(*p) {
            Some(true) => return Seat::Agent(pane.map(|(id, _)| id)),
            Some(false) => {}
            None => unknown = true,
        }
    }
    match (pane, ended) {
        (Some((id, true)), _) => Seat::Agent(Some(id)),
        (Some(_), _) if unknown => Seat::Unreadable,
        (Some(_), _) => Seat::Person,
        (None, End::Server | End::Hole) => Seat::Unreadable,
        // Not asked of the unread links: outside the runner (sshd is root's).
        (None, End::One) if procs.get(&pid).is_some_and(|row| runner.ttys.contains(&row.tty)) => Seat::Unreadable,
        (None, End::One) => Seat::Person,
    }
}

/// How a walk with no pane ended.
#[derive(Clone, Copy)]
enum End {
    /// At pid 1.
    One,
    /// At this runner's tmux server.
    Server,
    /// At a process the table doesn't have, or too deep.
    Hole,
}

/// Whether a process is an agent's, by its executable and command line, as
/// the watcher reads one.
fn is_agent(registry: &farcooler_core::activity::Registry, exe: &str, args: &str) -> bool {
    let program = if exe.is_empty() { args.split_whitespace().next().unwrap_or_default() } else { exe };
    registry.rules_for_command(args).is_some() || crate::watch::answer_wake::agent_of_process(program, args).is_some()
}

#[cfg(test)]
#[path = "caller_seat_tests.rs"]
mod tests;
