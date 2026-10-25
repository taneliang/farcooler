//! Only a person answers a held hook ask (ov-647, R-78): an agent above the
//! caller, in a pane or in none, can't allow, deny, approve a plan or answer
//! a question, and neither can a caller the runner can't read: a command an
//! agent left behind (ov-649), a socket caller whose pid the kernel didn't
//! say. The person's own shell pane, and a remote device's
//! `farcoolerd --stdio` (under `sshd`), still answer. A copy of perl named
//! `claude` stands in for the agent, forking a `sleep` that is the caller,
//! and one named `sshd` for a device's session.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use farcooler_agent_hooks::wire::LONGEST_HOLD;
use farcooler_protocol::v1::{PrThreadReply, Scope};

use super::*;
use crate::hook_asks::AskShape;
use crate::test_support::{Outside, ScratchDir};

/// The stand-in: forks `sleep`, writes the sleep's pid to its one argument,
/// and waits on it.
const STAND_IN: &str = r#"$| = 1;
my $p = fork();
if (!$p) { exec 'sleep', '6647'; }
open(my $f, '>', "$ARGV[0].part") or die; print $f "$p\n"; close $f;
rename("$ARGV[0].part", $ARGV[0]) or die;
waitpid($p, 0);
"#;

struct Seats {
    dir: ScratchDir,
    svc: Arc<Service>,
    checkout: (Uuid, String),
    /// Processes this test started, ended on drop.
    started: Vec<i32>,
    /// The stand-ins it spawned, reaped on drop.
    children: Vec<std::process::Child>,
    /// Processes outside the runner it started, ended on drop.
    outside: Vec<Outside>,
}

impl Drop for Seats {
    fn drop(&mut self) {
        for pid in &self.started {
            // SAFETY: a signal to a process this test started.
            unsafe {
                libc::kill(*pid, libc::SIGTERM);
            }
        }
        for child in &mut self.children {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

async fn seats() -> Seats {
    let (dir, svc, repo) = crate::test_support::fixture().await;
    let checkout = svc.store.list_worktrees_for_repository(repo).unwrap().into_iter().find(|w| w.is_main_checkout).unwrap();
    Seats { dir, svc, checkout: (checkout.id, checkout.worktree_path.clone()), started: Vec::new(), children: Vec::new(), outside: Vec::new() }
}

impl Seats {
    /// The stand-in claude and its script, and where it writes the pid.
    fn claude(&self, name: &str) -> (PathBuf, PathBuf, PathBuf) {
        let at = self.dir.path().join(name);
        std::fs::create_dir_all(&at).unwrap();
        let claude = at.join("claude");
        let copied = std::process::Command::new("cp").arg(farcooler_core::programs::find("perl").expect("perl")).arg(&claude).status().unwrap();
        assert!(copied.success());
        std::fs::write(at.join("stand_in.pl"), STAND_IN).unwrap();
        (claude, at.join("stand_in.pl"), at.join("pid"))
    }

    /// A caller with a claude above it, in no pane of this runner.
    fn under_claude_outside(&mut self) -> i32 {
        let (claude, script, pid) = self.claude("outside");
        let agent = std::process::Command::new(&claude).arg(&script).arg(&pid).spawn().expect("the stand-in claude");
        self.children.push(agent);
        let caller = read_pid(&pid);
        self.started.push(caller);
        caller
    }

    /// A caller with a claude above it, in a shell pane of this runner.
    async fn under_claude_in_a_pane(&mut self) -> (Uuid, i32) {
        let shell = self.shell().await;
        let (claude, script, pid) = self.claude("in-pane");
        let pane = self.svc.inventory.refresh().await.claimants(shell).into_iter().next().expect("a pane").pane_id.clone();
        let q = |p: &Path| format!("'{}'", p.display());
        self.svc.tmux.respawn_pane(&pane, &self.checkout.1, &format!("{} {} {}", q(&claude), q(&script), q(&pid))).await.unwrap();
        let caller = read_pid(&pid);
        self.started.push(caller);
        self.svc.inventory.refresh().await;
        (shell, caller)
    }

    /// A person's own shell pane, and its shell's pid.
    async fn shell_pane(&mut self) -> (Uuid, i32) {
        let shell = self.shell().await;
        let pid = self.svc.inventory.refresh().await.claimants(shell).into_iter().next().expect("a pane").stamp.pid as i32;
        assert!(pid > 1, "tmux named the pane's process");
        (shell, pid)
    }

    async fn shell(&self) -> Uuid {
        let t = self.svc.create_terminal(self.checkout.0, "Terminal 1", "shell").await.expect("a shell pane");
        crate::test_support::shell_ready(&self.svc, t.id).await;
        t.id
    }

    /// A remote device's `farcoolerd --stdio`, as one is reached: under a
    /// stand-in `sshd`, orphaned to pid 1, with no agent above it.
    fn remote(&mut self) -> i32 {
        self.keep(Outside::under_root(&Outside::stand_in(&self.dir.path().join("ssh").join("sshd")), 6648))
    }

    /// A command a claude's tool shell left behind with `(cmd &)`: orphaned
    /// to pid 1, with no terminal and nothing above it.
    fn left_behind(&mut self) -> i32 {
        let (claude, _, _) = self.claude("left");
        self.keep(Outside::left_by(&claude, 6649))
    }

    fn keep(&mut self, outside: Outside) -> i32 {
        let pid = outside.pid;
        self.outside.push(outside);
        pid
    }
}

fn read_pid(file: &Path) -> i32 {
    let since = Instant::now();
    loop {
        if let Some(pid) = std::fs::read_to_string(file).ok().and_then(|s| s.trim().parse().ok()) {
            return pid;
        }
        assert!(since.elapsed() < crate::test_support::PANE_PATIENCE, "the stand-in never started");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A pid no process has: the walk can't read it.
const NOBODY: i32 = i32::MAX - 7;

fn from(svc: &Arc<Service>, pid: Option<i32>) -> RpcFactory {
    bare(svc).with_caller_pid(pid)
}

fn bare(svc: &Arc<Service>) -> RpcFactory {
    let peer = Peer { client_id: None, scope: Scope::Control };
    RpcFactory::new(svc.clone(), crate::watch::Watcher::new(svc.clone()), Arc::new(tokio::sync::Notify::new()), peer)
}

/// Each kind of ask a hook holds, and the answer a person would give it.
fn asks() -> Vec<(AskShape, &'static str, HashMap<String, String>)> {
    let plan = serde_json::json!({ "plan": "Ship it" });
    let question = serde_json::json!({ "questions": [{ "question": "Which color?", "header": "Color", "options": [{ "label": "Red" }], "multiSelect": false }] });
    let answers = HashMap::from([("Which color?".to_string(), "Red".to_string())]);
    vec![
        (AskShape::Permission, "allow", HashMap::new()),
        (AskShape::Permission, "deny", HashMap::new()),
        (AskShape::Plan { input: plan.clone() }, "allow", HashMap::new()),
        (AskShape::Plan { input: plan }, "deny", HashMap::new()),
        (AskShape::Question { input: question }, "answer", answers),
    ]
}

/// Hold `shape` on `terminal`, answer it from `caller` claiming `actor`, and
/// say what came of it: the refusal's word, or `None` when the hook was told.
async fn answered(svc: &Arc<Service>, terminal: Uuid, caller: Option<i32>, actor: &str, ask: &(AskShape, &str, HashMap<String, String>)) -> Option<String> {
    answered_by(svc, &from(svc, caller), terminal, actor, ask).await
}

/// `answered`, from the connection `rpc`.
async fn answered_by(svc: &Arc<Service>, rpc: &RpcFactory, terminal: Uuid, actor: &str, ask: &(AskShape, &str, HashMap<String, String>)) -> Option<String> {
    let (id, rx) = svc.hooks().asks().hold_shaped(terminal, None, ask.0.clone(), LONGEST_HOLD);
    let hook = tokio::spawn(async move {
        let settled = rx.await.ok()?;
        settled.ack.map(|ack| ack.send(()))?.ok()?;
        settled.decision
    });
    let req = Request {
        method: "terminal.agent_answer".into(),
        payload: Some(request::Payload::AgentAnswer(AgentAnswer {
            terminal_id: crate::wire::id_bytes(terminal),
            request_id: id.clone(),
            option_id: ask.1.into(),
            answers: ask.2.clone(),
            actor: actor.into(),
        })),
        ..Default::default()
    };
    let refused = refusal(rpc, req).await.map(|(_, what)| what);
    if refused.is_some() {
        assert!(svc.hooks().asks().is_holding(terminal), "a refused answer leaves the ask held");
        svc.hooks().asks().withdraw(terminal, &id);
        hook.abort();
    } else {
        assert!(hook.await.unwrap().is_some(), "the hook was told a decision");
    }
    refused
}

/// A claude above the caller, outside every pane, allows, denies, approves
/// and answers nothing; nor does one in a shell pane. Removing the check in
/// `rpc_agent_answer::answer` turns both red.
#[tokio::test]
async fn an_agent_above_the_caller_answers_no_held_ask() {
    let mut s = seats().await;
    let outside = s.under_claude_outside();
    let (pane, inside) = s.under_claude_in_a_pane().await;
    use crate::caller_seat::{Seat, caller_seat};
    assert_eq!(caller_seat(&s.svc, Some(outside)).await, Seat::Agent(None));
    assert_eq!(caller_seat(&s.svc, Some(inside)).await, Seat::Agent(Some(pane)));
    for ask in &asks() {
        for (caller, said) in [(outside, "outside every pane"), (inside, "in a shell pane")] {
            for actor in ["", "user"] {
                let got = answered(&s.svc, pane, Some(caller), actor, ask).await;
                assert_eq!(got.as_deref(), Some("not_a_person"), "{said}, claiming {actor:?}: {:?} {}", ask.0, ask.1);
            }
        }
    }
}

/// The person's own shell pane, and a remote device's process, answer every
/// kind of ask, and a caller the walk can't read answers none: a pid no
/// process has, a command an agent left behind (ov-649), and a socket
/// caller whose pid the kernel didn't say (ov-649).
#[tokio::test]
async fn a_person_answers_and_an_unreadable_caller_does_not() {
    let mut s = seats().await;
    let (pane, shell) = s.shell_pane().await;
    let remote = s.remote();
    let left = s.left_behind();
    let unsaid = bare(&s.svc).with_socket_caller(None);
    for ask in &asks() {
        assert_eq!(answered(&s.svc, pane, Some(shell), "", ask).await, None, "their own shell: {:?} {}", ask.0, ask.1);
        assert_eq!(answered(&s.svc, pane, Some(remote), "user", ask).await, None, "a remote device: {:?} {}", ask.0, ask.1);
        assert_eq!(answered(&s.svc, pane, None, "", ask).await, None, "an in-process stdio session: {:?} {}", ask.0, ask.1);
        for (caller, said) in [(NOBODY, "unreadable"), (left, "left behind by an agent")] {
            let got = answered(&s.svc, pane, Some(caller), "user", ask).await;
            assert_eq!(got.as_deref(), Some("not_a_person"), "{said}: {:?} {}", ask.0, ask.1);
        }
        let got = answered_by(&s.svc, &unsaid, pane, "user", ask).await;
        assert_eq!(got.as_deref(), Some("not_a_person"), "a socket caller with no pid: {:?} {}", ask.0, ask.1);
    }
}

/// An actor that says it isn't the person is taken at its word, even from
/// the person's own shell: the request can only make the check stricter.
#[tokio::test]
async fn an_actor_that_isnt_the_person_is_refused_from_any_seat() {
    let mut s = seats().await;
    let (pane, shell) = s.shell_pane().await;
    let ask = &asks()[0];
    for actor in ["pane", "manager", "agent:0199a1b2-7c3d-7e4f-8a9b-0c1d2e3f4a5b"] {
        assert_eq!(answered(&s.svc, pane, Some(shell), actor, ask).await.as_deref(), Some("not_a_person"), "{actor}");
    }
}

/// `pr.thread_reply`'s send posts as the person's login and writes their
/// Answer, so an agent's, or an unreadable caller's, is refused before
/// anything runs. A person's goes on to the card (here, one that doesn't
/// exist). A preview is anyone's.
#[tokio::test]
async fn only_a_person_sends_a_thread_reply() {
    let mut s = seats().await;
    let outside = s.under_claude_outside();
    let (_, inside) = s.under_claude_in_a_pane().await;
    let remote = s.remote();
    let left = s.left_behind();
    let reply = |send: bool| Request {
        method: "pr.thread_reply".into(),
        payload: Some(request::Payload::PrThreadReply(PrThreadReply {
            task_id: crate::wire::id_bytes(Uuid::now_v7()),
            send,
            body: "Fixed in the next push.".into(),
            ..Default::default()
        })),
        ..Default::default()
    };
    for (caller, said) in [(Some(outside), "outside"), (Some(inside), "in a pane"), (Some(NOBODY), "unreadable"), (Some(left), "left behind")] {
        let got = refusal(&from(&s.svc, caller), reply(true)).await.map(|(_, what)| what);
        assert_eq!(got.as_deref(), Some("not_a_person"), "{said}");
        let preview = refusal(&from(&s.svc, caller), reply(false)).await.map(|(_, what)| what);
        assert_ne!(preview.as_deref(), Some("not_a_person"), "{said}'s preview");
    }
    let unsaid = refusal(&bare(&s.svc).with_socket_caller(None), reply(true)).await.map(|(_, what)| what);
    assert_eq!(unsaid.as_deref(), Some("not_a_person"), "a socket caller with no pid");
    for caller in [Some(remote), None] {
        let got = refusal(&from(&s.svc, caller), reply(true)).await.map(|(_, what)| what);
        assert_ne!(got.as_deref(), Some("not_a_person"), "{caller:?}");
    }
}

/// ov-747 (review 743 finding 4): who answered a held ask from the local
/// socket is whose pane the caller runs in, by the one rule
/// (`caller_person`): Bob, in his own shell pane on his login, answering
/// the owner's ask, is Bob; the owner, in a shell pane on the runner's own
/// login, answering Bob's, is the owner. Red if `answerer` asks the device
/// alone, which reads both as the owner.
#[tokio::test]
async fn an_answer_from_a_members_own_pane_is_theirs() {
    use crate::hook_asks::whole_notes_for;
    let mut s = seats().await;
    s.svc.store.set_member("bob", "Bob Okafor", &[]).unwrap();
    let (bobs, bobs_shell) = s.shell_pane().await;
    let login_id = s.svc.store.ensure_member_login("bob").unwrap();
    let login = farcooler_store::claude_logins::TerminalLogin { handle: "bob".into(), login_id, config_dir: "/r/claude/bob".into(), shared: false };
    s.svc.store.set_terminal_login(bobs, &login).unwrap();
    let (owners, owners_shell) = s.shell_pane().await;
    let note = |terminal: Uuid| whole_notes_for(terminal).into_iter().last().expect("the row was told");
    let ask = &asks()[0];

    assert_eq!(answered(&s.svc, owners, Some(bobs_shell), "", ask).await, None, "Bob answers");
    let said = note(owners);
    assert_eq!(said["person"], serde_json::json!({ "handle": "bob", "name": "Bob Okafor" }), "Bob, from his own pane: {said}");
    assert_eq!(said["for"], serde_json::json!({ "handle": "", "name": "" }), "for the owner: {said}");

    assert_eq!(answered(&s.svc, bobs, Some(owners_shell), "", ask).await, None, "the owner answers");
    let said = note(bobs);
    assert_eq!(said["person"], serde_json::json!({ "handle": "", "name": "" }), "the owner, from the runner's login: {said}");
    assert_eq!(said["for"]["handle"], "bob", "for Bob: {said}");
}
