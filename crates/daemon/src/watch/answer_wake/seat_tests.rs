//! Who a caller is, on real processes (ov-620 re-review 2): a person's
//! command in their own shell pane, an agent's child in a pane, a pane the
//! inventory hasn't seen, and processes outside the runner, each found in the
//! process table and walked as a request's caller would be. On the board and
//! tmux server of `tests`. No real agent runs: a copy of `sh` named `claude`
//! stands in for one.

use std::io::BufRead;

use crate::caller_seat::{Seat, caller_seat};

use super::*;

/// A `sleep` orphaned to pid 1, so no agent this test may itself run under
/// is its ancestor: a person's process outside the runner. Killed when
/// dropped.
pub(super) struct Orphan(pub(super) i32);

impl Orphan {
    pub(super) fn spawn(seconds: u32) -> Orphan {
        let out = std::process::Command::new("sh")
            .args(["-c", &format!("sleep {seconds} >/dev/null 2>&1 </dev/null & echo $!")])
            .output()
            .expect("sh");
        Orphan(String::from_utf8_lossy(&out.stdout).trim().parse().expect("a pid"))
    }
}

impl Drop for Orphan {
    fn drop(&mut self) {
        // SAFETY: a signal to the pid this test started.
        unsafe { libc::kill(self.0, libc::SIGTERM) };
    }
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
/// process with no agent above it is a person's.
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
    let person = Orphan::spawn(6042);
    assert_eq!(caller_seat(&b.svc, Some(person.0)).await, Seat::Person);
}
