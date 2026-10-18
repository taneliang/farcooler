//! The seat walk's logic on a process table written out by hand (`seat_in`).
//! What real processes read as is `answer_wake::tests::seat_tests`.

use super::*;

const PANE: Uuid = Uuid::from_u128(7);
const SERVER: i32 = 10;
const ROOT: i32 = 20;

/// `rows` as `(pid, ppid, tty)`.
fn table(rows: &[(i32, i32, &str)]) -> HashMap<i32, Proc> {
    rows.iter().map(|&(pid, ppid, tty)| (pid, Proc { ppid, tty: tty.into() })).collect()
}

/// The walk, with `agents` the pids that are agents and `unread` the ones
/// whose command line can't be read.
fn walk(pid: i32, rows: &[(i32, i32, &str)], panes: &Panes, agents: &[i32], unread: &[i32]) -> Seat {
    let servers = HashSet::from([SERVER]);
    let ttys = HashSet::from(["ttys001".to_string()]);
    let agent = |p: i32| if unread.contains(&p) { None } else { Some(agents.contains(&p)) };
    seat_in(pid, &table(rows), &Runner { panes, servers: &servers, ttys: &ttys }, &agent)
}

fn pane(launched_as_agent: bool) -> Panes {
    HashMap::from([(ROOT, (PANE, launched_as_agent))])
}

/// The server, the pane's root shell, an interactive shell, and the caller.
const ROWS: [(i32, i32, &str); 4] = [(SERVER, 1, "??"), (ROOT, SERVER, "ttys001"), (21, ROOT, "ttys001"), (30, 21, "ttys001")];

#[test]
fn shells_alone_above_the_caller_are_a_persons() {
    assert_eq!(walk(30, &ROWS, &pane(false), &[], &[]), Seat::Person);
}

#[test]
fn an_agent_on_the_chain_makes_it_an_agents() {
    assert_eq!(walk(30, &ROWS, &pane(false), &[21], &[]), Seat::Agent(Some(PANE)));
    assert_eq!(walk(30, &ROWS, &pane(true), &[], &[]), Seat::Agent(Some(PANE)), "a pane launched as an agent");
}

#[test]
fn an_unread_link_in_a_pane_is_unknown_unless_an_agent_is_read() {
    assert_eq!(walk(30, &ROWS, &pane(false), &[], &[21]), Seat::Unreadable);
    assert_eq!(walk(30, &ROWS, &pane(false), &[ROOT], &[21]), Seat::Agent(Some(PANE)));
}

/// No pane below the server: unknown, or an agent when one is on the chain.
#[test]
fn the_server_with_no_pane_below_it_is_never_a_person() {
    assert_eq!(walk(30, &ROWS, &Panes::new(), &[], &[]), Seat::Unreadable);
    assert_eq!(walk(30, &ROWS, &Panes::new(), &[21], &[]), Seat::Agent(None));
}

/// To pid 1 with no pane and no server: a person, unless an agent is on the
/// chain, or the caller's terminal is one of the runner's panes (an orphan).
#[test]
fn outside_the_runner() {
    let outside = [(5, 1, "??"), (6, 5, "ttys009"), (7, 6, "ttys009")];
    assert_eq!(walk(7, &outside, &pane(false), &[], &[5]), Seat::Person, "an sshd that can't be read isn't asked");
    assert_eq!(walk(7, &outside, &pane(false), &[6], &[]), Seat::Agent(None), "a claude outside Far Cooler");
    let orphan = [(40, 1, "ttys001")];
    assert_eq!(walk(40, &orphan, &pane(false), &[], &[]), Seat::Unreadable, "left behind in a pane's terminal");
    assert_eq!(walk(99, &outside, &pane(false), &[], &[]), Seat::Unreadable, "the caller gone");
}

/// A stub-only runner (a test harness's) doesn't count an agent outside its
/// panes, which is the harness's own; one inside them still counts, and so
/// does unknown.
#[test]
fn a_stub_runner_counts_only_agents_in_its_panes() {
    assert_eq!(stubbed(Seat::Agent(None), true), Seat::Person);
    assert_eq!(stubbed(Seat::Agent(None), false), Seat::Agent(None));
    assert_eq!(stubbed(Seat::Agent(Some(PANE)), true), Seat::Agent(Some(PANE)));
    assert_eq!(stubbed(Seat::Unreadable, true), Seat::Unreadable);
}
