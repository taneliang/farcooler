//! Who a caller is, on real processes (ov-620 re-review 2): a person's
//! command in their own shell pane, an agent's child in a pane, a pane the
//! inventory hasn't seen, and processes outside the runner, each found in the
//! process table and walked as a request's caller would be. On the board and
//! tmux server of `tests`. No real agent runs: a copy of `sh` named `claude`
//! stands in for one.

use std::io::BufRead;

use crate::caller_seat::{Seat, caller_seat};
use crate::test_support::Outside;

use super::*;

/// A person's process outside the runner, as a device's ssh session is: a
/// `sleep` under a stand-in for root's `sshd`, orphaned to pid 1, so no agent this test
/// may itself run under is its ancestor. No terminal: the `sshd` is what
/// says a person is there (ov-649). Ended when dropped.
pub(super) fn a_persons_outside(b: &Board, seconds: u32) -> Outside {
    Outside::a_person(b.dir.path(), seconds)
}

/// The pid whose command line, as the process table has it, holds `needle`,
/// within 30 s. Only a terminal's foreground group has its command line
/// there, which is where these run.
pub(super) async fn pid_running(needle: &str) -> i32 {
    let since = std::time::Instant::now();
    loop {
        let table = crate::proc_table::snapshot().expect("a process table");
        let found = table.lines().find(|l| l.split_whitespace().skip(5).collect::<Vec<_>>().join(" ").contains(needle));
        if let Some(pid) = found.and_then(|l| l.split_whitespace().next()?.parse().ok()) {
            return pid;
        }
        assert!(since.elapsed() < Duration::from_secs(30), "nothing runs {needle}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A copy of perl named `claude`, in the board's directory, as the stand-in
/// is (a copied `/bin/sh` is killed on macOS for its signature).
fn fake_claude(b: &Board) -> std::path::PathBuf {
    let dir = b.dir.path().join("fake-agent");
    std::fs::create_dir_all(&dir).unwrap();
    let program = dir.join("claude");
    copy_program(&farcooler_core::programs::find("perl").expect("perl"), &program);
    program
}

/// The fake claude's script: run `sleep <seconds>` as a child, say its pid,
/// and wait for it, as an agent's tool shell runs a command.
fn runs_a_child(seconds: u32) -> String {
    format!("$| = 1; my $p = fork(); if (!$p) {{ exec 'sleep', '{seconds}'; }} print \"$p\\n\"; waitpid($p, 0);")
}

/// `keys` typed into `pane`'s shell, then Enter.
pub(super) async fn type_into(b: &Board, pane: &Terminal, keys: &str) {
    b.svc.inventory.refresh().await;
    let id = b.svc.inventory_snapshot().claimants(pane.id).into_iter().next().expect("a pane").pane_id.clone();
    b.svc.tmux.run(&["send-keys", "-t", &id, keys, "Enter"]).await.unwrap();
}

/// A command a person typed in their own plain shell pane: the shell, and the
/// pane's own process, aren't in the foreground, so the process table has no
/// command line for them, and they're read now. A person. Round 4 read them
/// as `-` and refused the person `in_pane`.
#[tokio::test]
async fn a_persons_command_in_their_own_shell_pane_is_a_person() {
    let b = board().await;
    let shell = b.shell_pane().await;
    b.screen_with(shell.id, "").await;
    type_into(&b, &shell, "sleep 6017").await;
    let pid = pid_running("sleep 6017").await;
    b.svc.inventory.refresh().await;
    assert_eq!(caller_seat(&b.svc, Some(pid)).await, Seat::Person);
}

/// An agent's child in a shell pane (a command its tool shell runs) is the
/// agent's, though the pane wasn't launched as one.
#[tokio::test]
async fn an_agents_child_in_a_shell_pane_is_an_agent() {
    let b = board().await;
    let shell = b.shell_pane().await;
    let claude = fake_claude(&b);
    let script = b.dir.path().join("fake-agent").join("child.pl");
    std::fs::write(&script, runs_a_child(6023)).unwrap();
    b.run_unfollowed(&shell, &format!("'{}' '{}'", claude.display(), script.display())).await;
    let pid = pid_running("sleep 6023").await;
    b.svc.inventory.refresh().await;
    assert_eq!(caller_seat(&b.svc, Some(pid)).await, Seat::Agent(Some(shell.id)));
}

/// A pane on this runner's tmux server that the inventory doesn't know (one
/// just made, here one Far Cooler didn't make) is never a person's: the walk
/// meets the server, known by its pid, with no pane below it.
#[tokio::test]
async fn a_pane_the_inventory_hasnt_seen_is_never_a_person() {
    let b = board().await;
    b.shell_pane().await;
    b.svc.tmux.run(&["new-window", "-d", "sh -c 'sleep 6031; true'"]).await.unwrap();
    let pid = pid_running("sleep 6031").await;
    assert_eq!(caller_seat(&b.svc, Some(pid)).await, Seat::Unreadable);
}

/// Outside the runner: a claude's child is an agent's (in no pane), and a
/// process under an `sshd` with no agent above it is a person's.
#[tokio::test]
async fn outside_the_runner_an_agent_is_still_an_agent() {
    let b = board().await;
    b.shell_pane().await;
    let claude = fake_claude(&b);
    let mut agent = std::process::Command::new(&claude)
        .args(["-e", &runs_a_child(6041)])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("the fake claude");
    let mut line = String::new();
    std::io::BufReader::new(agent.stdout.as_mut().unwrap()).read_line(&mut line).unwrap();
    let child: i32 = line.trim().parse().expect("its child's pid");
    let seat = caller_seat(&b.svc, Some(child)).await;
    // SAFETY: signals to the processes this test started.
    unsafe {
        libc::kill(child, libc::SIGTERM);
    }
    let _ = agent.wait();
    assert_eq!(seat, Seat::Agent(None));
    let person = a_persons_outside(&b, 6042);
    assert_eq!(caller_seat(&b.svc, Some(person.pid)).await, Seat::Person);
}

/// A command an agent's tool shell left behind with `(cmd &)`: orphaned to
/// pid 1, with no terminal, and nothing above it now says who runs it. Never
/// a person (ov-649): before, any walk that reached pid 1 was one.
#[tokio::test]
async fn a_command_an_agent_left_behind_is_never_a_person() {
    let b = board().await;
    b.shell_pane().await;
    let claude = Outside::stand_in(&b.dir.path().join("fake-agent").join("claude"));
    let orphan = Outside::left_by(&claude, 6043);
    b.svc.inventory.refresh().await;
    assert_eq!(caller_seat(&b.svc, Some(orphan.pid)).await, Seat::Unreadable);
}

/// With no agent above it, a person's process outside the runner is one on
/// evidence: a terminal of its own that's no pane's (their Terminal shell),
/// with nothing else to say so; or, with no terminal, the Mac app this
/// daemon ships in, as its own call or its CLI's. An app this daemon doesn't
/// ship in, or none, says nothing.
#[tokio::test]
async fn a_persons_terminal_or_the_mac_app_is_a_person() {
    let b = board().await;
    b.shell_pane().await;
    let shell = Outside::in_a_terminal(&Outside::stand_in(&b.dir.path().join("term").join("zsh")), 6045);
    b.svc.inventory.refresh().await;
    assert_eq!(caller_seat(&b.svc, Some(shell.pid)).await, Seat::Person, "a person's Terminal shell");

    let bundle = b.dir.path().join("Far Cooler.app");
    let app = Outside::under(&Outside::stand_in(&bundle.join("Contents").join("MacOS").join("FarCooler")), 6046);
    let bundle = std::fs::canonicalize(&bundle).unwrap();
    use crate::caller_seat::seat_with;
    let other = b.dir.path().join("Other.app");
    assert_eq!(seat_with(&b.svc, Some(app.pid), Some(&bundle)).await, Seat::Person, "the CLI the app runs");
    assert_eq!(seat_with(&b.svc, app.parent, Some(&bundle)).await, Seat::Person, "the app's own call");
    assert_eq!(seat_with(&b.svc, Some(app.pid), Some(&other)).await, Seat::Unreadable, "an app this daemon isn't in");
    assert_eq!(seat_with(&b.svc, Some(app.pid), None).await, Seat::Unreadable, "a daemon in no app");
    assert_eq!(caller_seat(&b.svc, Some(app.pid)).await, Seat::Unreadable, "this test's daemon is in no app");
}

/// An `sshd` that isn't root's says nothing (ov-649 review): a copied
/// interpreter named `sshd`, or on Linux a write to `/proc/self/comm`, is
/// this user's, where a real session's chain has root's listener or monitor.
#[tokio::test]
async fn a_users_own_sshd_isnt_a_person() {
    let b = board().await;
    b.shell_pane().await;
    let sshd = Outside::stand_in(&b.dir.path().join("fake-ssh").join("sshd"));
    let users = Outside::under(&sshd, 6047);
    b.svc.inventory.refresh().await;
    assert_eq!(caller_seat(&b.svc, Some(users.pid)).await, Seat::Unreadable, "this user's sshd");
    let roots = Outside::under_root(&sshd, 6048);
    assert_eq!(caller_seat(&b.svc, Some(roots.pid)).await, Seat::Person, "root's");
}
