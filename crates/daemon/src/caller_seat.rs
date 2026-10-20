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
//! - pid 1 met with no agent on the chain is a person only on evidence that
//!   one is there (ov-649), since a command an agent's tool shell left behind
//!   with `(cmd &)` reaches pid 1 too, with nothing above it to read:
//!   - the caller's controlling terminal, when it's none of this runner's
//!     panes: a person's Terminal or iTerm shell, an ssh session with a pty.
//!     An agent's tool shell has none (Claude Code's runs in a session of
//!     its own); a caller whose terminal is a pane's is unknown, an orphan
//!     that pane's shell left behind;
//!   - root's `sshd` on the chain (`sshd-session` too, OpenSSH 9.8's): a
//!     device reaching the runner over ssh, tailcat's included, whose
//!     `farcoolerd --stdio` relays to this socket. Known by its program's
//!     name, which any user's process may read (`proc_table::name_of`), and
//!     its owner, root (`uid_of`): a real session's chain always has root's
//!     listener or `[priv]` monitor, and a name alone is a copied binary or,
//!     on Linux, a write to `/proc/self/comm`;
//!   - the Mac app this daemon ships in, on the chain: its own calls, and the
//!     CLI it runs as a child. Known by its executable's path, under the
//!     bundle's `Contents/MacOS` (`seat_with`), not by any `.app`: an agent
//!     can run an app's binary as a launcher (Python.app, an Electron app as
//!     node).
//!
//!   Anything else is unknown: launchd's job, a cron job, an agent's leftover.
//!
//! Unknown is never a person: a Prompt from it is refused `in_pane`, which the
//! CLI explains, rather than sent another way. An agent this runner doesn't
//! recognize (gemini, aider) in a shell pane reads as a person; Far Cooler has
//! no mailbox for one, so it can't be told anything either way.
//!
//! The connection the caller came on is its own evidence: `None` is the
//! in-process `--stdio` session, a device that ssh reached with no daemon
//! running, a person; a socket caller whose pid the kernel wouldn't say is
//! `UNKNOWN_PID`, never one.
//!
//! **A guardrail, not a boundary.** An agent that gives its command a
//! terminal of its own (`script`, `tmux new -d`) or reaches the runner over
//! `ssh localhost` reads as a person, and Control scope can type into any
//! pane anyway (`terminal.write`). This stops an agent's words being taken for the
//! person's by mistake; it doesn't stop an agent set on it.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

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

/// A socket caller whose pid the kernel didn't say (`RpcFactory::
/// with_socket_caller`): unknown, never a person. No process has it, and
/// nothing that reads a caller's pid signals one.
pub const UNKNOWN_PID: i32 = i32::MIN;

/// The seat of the local caller `pid`: `None` is the in-process `--stdio`
/// session, a person.
pub async fn caller_seat(svc: &crate::service::Service, pid: Option<i32>) -> Seat {
    seat_with(svc, pid, own_app().as_deref()).await
}

/// `caller_seat`, with `app` the Mac app bundle this daemon ships in, if any.
pub(crate) async fn seat_with(svc: &crate::service::Service, pid: Option<i32>, app: Option<&Path>) -> Seat {
    let Some(pid) = pid else { return Seat::Person };
    if pid <= 0 {
        return Seat::Unreadable;
    }
    let Some(mut view) = View::read(svc) else { return Seat::Unreadable };
    if view.servers.is_empty()
        && let Ok(out) = svc.tmux.run(&["display-message", "-p", "#{pid}"]).await
        && let Ok(server) = out.stdout.trim().parse()
    {
        view.servers.insert(server);
    }
    let registry = svc.registry();
    let agent = |pid: i32| crate::proc_table::command_of(pid).map(|(exe, args)| is_agent(&registry, &exe, &args));
    view.seat(pid, app, &agent)
}

/// `caller_seat` without asking tmux, for the pane walks that fail closed
/// on it (`people::pane_of` and its kin, ov-649). `Unreadable` is a caller
/// no pane walk may take for "in no pane": a command an agent left behind,
/// a pid unknown. An agent outside every pane (the owner's claude in
/// Terminal) is `Agent(None)`, placed. A pane so new the inventory hasn't
/// seen it is `Unreadable` too, until its next refresh.
pub fn walk_seat(svc: &crate::service::Service, pid: i32) -> Seat {
    if pid <= 0 {
        return Seat::Unreadable;
    }
    let Some(view) = View::read(svc) else { return Seat::Unreadable };
    let registry = svc.registry();
    let agent = |pid: i32| crate::proc_table::command_of(pid).map(|(exe, args)| is_agent(&registry, &exe, &args));
    view.seat(pid, own_app().as_deref(), &agent)
}

/// What the walk reads of the runner and the host, at one moment.
struct View {
    panes: Panes,
    servers: HashSet<i32>,
    ttys: HashSet<String>,
    procs: HashMap<i32, Proc>,
}

impl View {
    /// The runner's panes and the host's process table; `None` when the
    /// table can't be read.
    fn read(svc: &crate::service::Service) -> Option<View> {
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
        let procs = procs(&crate::proc_table::snapshot()?);
        let servers: HashSet<i32> = panes.keys().filter_map(|root| procs.get(root)).map(|row| row.ppid).filter(|&p| p > 1).collect();
        Some(View { panes, servers, ttys, procs })
    }

    /// `pid`'s seat, with `app` the bundle this daemon ships in and `agent`
    /// reading a process's command line.
    fn seat(&self, pid: i32, app: Option<&Path>, agent: &dyn Fn(i32) -> Option<bool>) -> Seat {
        let stub = crate::service::test_stub_agents();
        let vouches = |pid: i32| stub || vouches_for_a_person(pid, app);
        let runner = Runner { panes: &self.panes, servers: &self.servers, ttys: &self.ttys };
        stubbed(seat_in(pid, &self.procs, &runner, agent, &vouches), stub)
    }
}

/// The programs that serve an ssh session: `sshd`, and from OpenSSH 9.8 the
/// `sshd-session` it runs each one in.
const SSHD: [&str; 2] = ["sshd", "sshd-session"];

/// Whether process `pid` says a person is behind what runs under it: root's
/// `sshd`, or the Mac app `app` (its `Contents/MacOS`). See this module's
/// docs.
fn vouches_for_a_person(pid: i32, app: Option<&Path>) -> bool {
    if crate::proc_table::name_of(pid).is_some_and(|name| SSHD.contains(&name.as_str())) && is_roots(pid) {
        return true;
    }
    app.is_some_and(|app| crate::proc_table::path_of(pid).is_some_and(|exe| Path::new(&exe).starts_with(app.join("Contents/MacOS"))))
}

/// Whether `pid` runs as root (`proc_table::uid_of`). A test can't start a
/// root process, so it names its stand-in for root's `sshd` here
/// (`test_support::Outside::under_root`).
fn is_roots(pid: i32) -> bool {
    #[cfg(test)]
    if ROOT_IN_TESTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).contains(&pid) {
        return true;
    }
    crate::proc_table::uid_of(pid) == Some(0)
}

/// The processes a test stands in for root's (`is_roots`).
#[cfg(test)]
pub(crate) static ROOT_IN_TESTS: std::sync::Mutex<std::collections::BTreeSet<i32>> = std::sync::Mutex::new(std::collections::BTreeSet::new());

/// The `.app` bundle this daemon's executable is in, as the Mac app ships it
/// (`Far Cooler.app/Contents/Resources/farcoolerd`); `None` for one run from
/// anywhere else (Linux, a build tree), which no app vouches for.
fn own_app() -> Option<PathBuf> {
    let exe = std::fs::canonicalize(std::env::current_exe().ok()?).ok()?;
    app_of(&exe)
}

/// The nearest `.app` bundle holding `exe` in its `Contents`.
pub(crate) fn app_of(exe: &Path) -> Option<PathBuf> {
    exe.ancestors()
        .skip(1)
        .find(|dir| dir.extension().is_some_and(|e| e == "app") && exe.starts_with(dir.join("Contents")))
        .map(Path::to_path_buf)
}

/// A runner that launches only stubs (`service::test_stub_agents`, a test
/// harness's scratch daemon, which refuses a real agent) has no agent of its
/// own outside its panes to tell apart: an agent above the caller there is
/// the harness's own (a Mac test run from a terminal inside a claude pane),
/// and the caller is the test standing in for a person, with or without a
/// terminal or an `sshd` above it (`seat_with` takes its word, as a test run
/// in CI has neither). Inside its panes, where its stand-ins run, the walk
/// stands.
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
/// agent, `None` when its command line can't be read, and `vouches` whether
/// one says a person is behind it (an `sshd`, the Mac app). See this
/// module's docs.
pub fn seat_in(pid: i32, procs: &HashMap<i32, Proc>, runner: &Runner, agent: &dyn Fn(i32) -> Option<bool>, vouches: &dyn Fn(i32) -> bool) -> Seat {
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
        // Outside the runner, `unknown` isn't asked (sshd is root's), but a
        // person has to be shown (ov-649).
        (None, End::One) => {
            let tty = procs.get(&pid).map(|row| row.tty.as_str()).unwrap_or("??");
            if runner.ttys.contains(tty) {
                Seat::Unreadable
            } else if tty != "??" || chain.iter().any(|&p| vouches(p)) {
                Seat::Person
            } else {
                Seat::Unreadable
            }
        }
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
