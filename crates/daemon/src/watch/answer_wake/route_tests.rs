//! The delivery routes (ov-620): a person's words reach claude only typed, as
//! their own turn, and never by its mailbox, even with a live waiter; agent
//! and runner text keeps the mailbox; each live request goes by its route.
//! The test plays the mailbox's waiter, as `mailbox_tests` does. The real
//! claude never runs. On the board and tmux server of `tests`.

use farcooler_agent_hooks::mailbox::{Mailbox, pid_alive};

use super::route::{Author, Delivered, Delivery, Kind, Part, Press};
use super::*;

const SESSION: &str = "5a1e0620-31c6-4c8e-9704-b2d582039d04";

/// `pane`'s session set to `SESSION`, and its mailbox with this process as
/// its live waiter.
fn armed(b: &Board, pane: &Terminal) -> Mailbox {
    let pane = b.svc.store.get_terminal(pane.id).unwrap();
    b.svc.store.set_pane_mode(pane.id, pane.resource_version, PaneMode::Terminal, Some(SESSION.into()), false).unwrap();
    let mailbox = Mailbox::of_session(&crate::claude_mailbox::root_of(b.svc.root_dir()), SESSION).unwrap();
    assert!(mailbox.claim_waiter(std::process::id(), &pid_alive));
    mailbox
}

/// The lead's `message.send` to a claude lane is typed and submitted, as the
/// person's own turn, though the pane's mailbox has a live waiter. On main it
/// went by the mailbox, where claude 2.1.301 wraps it "NOT USER INPUT".
#[tokio::test]
async fn a_persons_message_is_typed_never_mailboxed() {
    let b = board().await;
    let pane = b.agent("Agent 2", "claude").await;
    let mailbox = armed(&b, &pane);
    let si = b.stand_in(&pane, "claude", "claude").await;
    b.doing(pane.id, AgentActivity::Idle).await;
    b.send("user", &b.task.key, "Rename foo to bar").await.unwrap();
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), ["[from the terminal] Rename foo to bar"], "{}", si.log());
    assert_eq!(mailbox.take_oldest().unwrap(), None, "never dropped in the mailbox");
    assert!(b.messages().is_empty(), "settled: {:?}", b.messages());
}

/// A member's device speaks as that person: typed too, tagged with their
/// name, never mailboxed.
#[tokio::test]
async fn a_members_message_is_typed_never_mailboxed() {
    let b = board().await;
    let orchestrator = b.orchestrator().await;
    let mailbox = armed(&b, &orchestrator);
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    b.svc.store.set_member("bob", "Bob", &["phone-b".to_string()]).unwrap();
    b.svc.store.set_workspace_lead(b.task.workspace_id, Some("bob")).unwrap();
    b.send_as(crate::people::Person::Member("bob".into()), "user", "orchestrator", "Ship Friday", &b.task.key).await.unwrap();
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), ["[from Bob] Ship Friday"], "{}", si.log());
    assert_eq!(mailbox.take_oldest().unwrap(), None, "never dropped in the mailbox");
}

/// A draft in the box holds a person's message, says why on its card, and
/// never sends it another way: nothing typed, nothing in the mailbox.
#[tokio::test]
async fn a_persons_message_behind_a_draft_is_held_never_mailboxed() {
    let b = board().await;
    let pane = b.agent("Agent 2", "claude").await;
    let mailbox = armed(&b, &pane);
    let si = b.stand_in(&pane, "claude", "claude").await;
    si.show("draft:half a thought").await;
    b.screen_with(pane.id, "half a thought").await;
    b.doing(pane.id, AgentActivity::Idle).await;
    b.send("user", &b.task.key, "Rename foo to bar").await.unwrap();
    b.pump().await;
    b.pump().await;
    assert_eq!(mailbox.take_oldest().unwrap(), None, "never dropped in the mailbox");
    let waiting = b.messages();
    assert_eq!(waiting.len(), 1, "held");
    assert_eq!(waiting[0].claimed_at, None, "nothing typed");
    assert!(!si.log().contains("PASTE"), "{}", si.log());
    assert!(
        b.progress().iter().any(|n| n.starts_with("Waiting to tell") && n.ends_with("there's a draft in its box.")),
        "{:?}",
        b.progress()
    );
}

/// The orchestrator's and the runner's text keeps the mailbox: unchanged.
#[tokio::test]
async fn agent_and_runner_messages_keep_the_mailbox() {
    let b = board().await;
    let pane = b.agent("Agent 2", "claude").await;
    let mailbox = armed(&b, &pane);
    let si = b.stand_in(&pane, "claude", "claude").await;
    b.doing(pane.id, AgentActivity::Working).await;
    b.send("manager", &b.task.key, "Rebase on main first").await.unwrap();
    b.pump().await;
    assert_eq!(mailbox.take_oldest().unwrap().as_deref(), Some("[from the orchestrator] Rebase on main first"));
    b.pump().await;
    assert!(b.messages().is_empty(), "settled once taken");
    assert!(!si.log().contains("PASTE"), "{}", si.log());
}

/// Each queued wake's route: a person's answer and message are Prompts; a
/// hold's end, the orchestrator's, a lane's, the runner's and an unknown
/// sender's are agent messages.
#[test]
fn a_wake_routes_by_its_author() {
    let wake = |kind, actor| PendingWake {
        kind,
        note: Uuid::nil(),
        task: Uuid::nil(),
        body: String::new(),
        actor,
        enqueued_at: 0,
        claimed_at: None,
        pasted_at: None,
        to: None,
    };
    let lane = Actor::Agent { terminal: Uuid::nil() };
    assert_eq!(Kind::of_wake(&wake(WakeKind::Answer, Actor::User)), Kind::Prompt);
    assert_eq!(Kind::of_wake(&wake(WakeKind::Message, Actor::User)), Kind::Prompt);
    for actor in [Actor::Manager, lane, Actor::Runner, Actor::Unknown] {
        assert_eq!(Kind::of_wake(&wake(WakeKind::Message, actor)), Kind::AgentMessage, "{actor:?}");
    }
    assert_eq!(Kind::of_wake(&wake(WakeKind::HoldEnded, Actor::Runner)), Kind::AgentMessage);
}

/// An agent's live delivery is held to its rules: no command first in a
/// Prompt, no trust Enter, and no agent message sent from here at all (it
/// enters by the queue). Nothing is typed.
#[tokio::test]
async fn an_agents_live_deliveries_are_held_to_its_rules() {
    let b = board().await;
    let (agent, si) = super::compose_tests::idle_claude(&b).await;
    let as_agent = |kind: Kind, body: &str| Delivery { kind, author: Author::Agent { from: Some(agent.id) }, body: body.into(), parts: Vec::new() };
    let trust = Press::Trust { ask: "trust-prompt-x".into(), option: "trust".into() };
    // A file the request staged: removed when the send is refused (N3).
    let staged = b.dir.path().join("staged.txt");
    std::fs::write(&staged, "x").unwrap();
    for (d, word) in [
        (as_agent(Kind::Prompt, "/ship it"), "command"),
        (as_agent(Kind::Key(trust), ""), "not_a_person"),
        (as_agent(Kind::AgentMessage, "hi"), "kind"),
        (Delivery { parts: vec![Part::Image("image/png".into(), vec![1])], ..as_agent(Kind::Prompt, "look") }, "images"),
        (Delivery { parts: vec![Part::File(staged.clone())], ..as_agent(Kind::Prompt, "read") }, "files"),
    ] {
        let said = b.watcher.deliver(agent.id, d.clone()).await;
        assert_eq!(said.as_ref().err().map(|e| e.what()), Some(word), "{d:?}: {said:?}");
    }
    assert!(!staged.exists(), "the staged file was left behind");
    assert!(b.messages().is_empty(), "nothing filed");
    assert!(!si.log().contains("PASTE") && !si.log().contains("ENTER"), "{}", si.log());
}

/// A person's Prompt goes through compose: typed, submitted, and Sent once
/// claude's hook names it.
#[tokio::test]
async fn a_prompt_is_composed() {
    let b = board().await;
    let (agent, si) = super::compose_tests::idle_claude(&b).await;
    let _hook = super::compose_tests::hook_on_submit(&b, &si);
    let said = b.watcher.deliver(agent.id, Delivery::prompt("fix the flaky test")).await;
    assert!(matches!(said, Ok(Delivered::Sent)), "{said:?}");
    assert_eq!(si.submitted(), ["fix the flaky test"], "{}", si.log());
}

/// A person's Prompt into a box holding a draft is refused as `draft`, and
/// nothing is typed or sent any other way.
#[tokio::test]
async fn a_prompt_behind_a_draft_is_refused() {
    let b = board().await;
    let (agent, si) = super::compose_tests::idle_claude(&b).await;
    si.show("draft:half a thought").await;
    b.screen_with(agent.id, "half a thought").await;
    let said = b.watcher.deliver(agent.id, Delivery::prompt("fix the flaky test")).await;
    assert!(matches!(said, Err(DomainError::Conflict { what: "draft" })), "{said:?}");
    assert!(!si.log().contains("PASTE") && !si.log().contains("ENTER"), "{}", si.log());
}

/// A Draft is pasted with no Enter.
#[tokio::test]
async fn a_draft_is_pasted_never_entered() {
    let b = board().await;
    let orchestrator = b.orchestrator().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    let said = b.watcher.deliver(orchestrator.id, Delivery::draft("About ov-1: ", false)).await;
    assert!(matches!(said, Ok(Delivered::Drafted(_))), "{said:?}");
    assert!(si.log().contains("PASTE"), "{}", si.log());
    assert!(!si.log().contains("ENTER"), "{}", si.log());
}

/// A Key goes through its gate: Stop on an idle claude is refused as `idle`,
/// and no key is pressed.
#[tokio::test]
async fn a_key_is_gated() {
    let b = board().await;
    let (agent, si) = super::compose_tests::idle_claude(&b).await;
    let said = b.watcher.deliver(agent.id, Delivery::key(Press::Interrupt)).await;
    assert!(matches!(said, Err(DomainError::Conflict { what: "idle" })), "{said:?}");
    assert!(!si.log().contains("ESC"), "{}", si.log());
}

/// An answer, a Prompt, gets compose's last check before an Enter with no
/// fence (`still_between`): a turn claude began during the read-back, here
/// one only its registry shows, gets no Enter, and the answer waits, pasted,
/// for the fenced one (`finish`). Before ov-620 that Enter went in unfenced.
#[tokio::test]
async fn an_answer_gets_composes_check_before_an_unfenced_enter() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    let (control, log) = (si.control.clone(), si.log.clone());
    *b.watcher.before_paste.lock().unwrap() = Some(Box::new(move || {
        std::fs::write(&control, "working-hidden").unwrap();
        for _ in 0..750 {
            if std::fs::read_to_string(&log).unwrap_or_default().contains("MODE working-hidden\n") {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("the stand-in never began its turn");
    }));
    b.answer("Drill in");
    b.pump().await;
    assert!(si.log().contains("PASTE "), "{}", si.log());
    assert!(!si.log().contains("ENTER"), "an Enter with a turn running: {}", si.log());
    let pending = b.pending();
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert!(pending[0].pasted_at.is_some(), "waits, pasted, for the fenced Enter: {pending:?}");
}

/// A person's message held by a draft never stalls the agents' text behind
/// it: the two are lines of their own (`messages::in_line`).
#[tokio::test]
async fn a_held_persons_message_never_stalls_the_agents_behind_it() {
    let b = board().await;
    let pane = b.agent("Agent 2", "claude").await;
    let mailbox = armed(&b, &pane);
    let si = b.stand_in(&pane, "claude", "claude").await;
    si.show("draft:half a thought").await;
    b.screen_with(pane.id, "half a thought").await;
    b.doing(pane.id, AgentActivity::Idle).await;
    b.send("user", &b.task.key, "Rename foo to bar").await.unwrap();
    b.send("manager", &b.task.key, "Rebase on main first").await.unwrap();
    b.pump().await;
    assert_eq!(mailbox.take_oldest().unwrap().as_deref(), Some("[from the orchestrator] Rebase on main first"));
    b.pump().await;
    let waiting = b.messages();
    assert_eq!(waiting.len(), 1, "the person's still held: {waiting:?}");
    assert_eq!(waiting[0].actor, Actor::User);
}

// ---- review 1: who is a person is worked out from the caller's process ----

/// A request through `Rpc`, from a local caller whose process is `pid`, as
/// the runner's owner: `None` when answered, else `<code word>:<what>`.
async fn call_from(b: &Board, pid: i32, req: farcooler_protocol::v1::Request) -> Option<String> {
    use farcooler_transport::Handler as _;
    let peer = farcooler_transport::Peer { client_id: None, scope: pb::Scope::HostAdmin };
    let factory = crate::rpc::RpcFactory::new(b.svc.clone(), b.watcher.clone(), std::sync::Arc::new(tokio::sync::Notify::new()), peer)
        .with_caller_pid(Some(pid));
    match factory.handle(req).await.outcome {
        Some(pb::response::Outcome::Result(_)) => None,
        Some(pb::response::Outcome::Error(e)) => Some(format!("{}:{}", farcooler_core::error::word_for(e.code), e.what)),
        other => panic!("no outcome: {other:?}"),
    }
}

/// The pid of `pane`'s own process, as the inventory has it.
async fn pid_of(b: &Board, pane: &Terminal) -> i32 {
    b.svc.inventory.refresh().await.claimants(pane.id).into_iter().next().expect("a pane").stamp.pid as i32
}

fn message_to(b: &Board, to: &str, text: &str) -> farcooler_protocol::v1::Request {
    farcooler_protocol::v1::Request {
        method: "message.send".into(),
        payload: Some(pb::request::Payload::MessageSend(pb::MessageSend {
            to: to.into(),
            text: text.into(),
            actor: String::new(),
            task: String::new(),
            workspace_id: b.task.workspace_id.as_bytes().to_vec().into(),
        })),
        ..Default::default()
    }
}

fn tell(pane: &Terminal, text: &str) -> farcooler_protocol::v1::Request {
    let blocks = vec![pb::AgentPromptBlock { content: Some(pb::agent_prompt_block::Content::Text(text.into())) }];
    farcooler_protocol::v1::Request {
        method: "terminal.tell".into(),
        target_resource_id: Some(crate::wire::id_bytes(pane.id)),
        payload: Some(pb::request::Payload::AgentPrompt(pb::AgentPrompt {
            terminal_id: crate::wire::id_bytes(pane.id),
            blocks,
            hold_behind_dialog: false,
        })),
        ..Default::default()
    }
}

/// The process in `pane` that runs `program` (its command line's first word,
/// by name), found under the pane's own process in the process table, within
/// 30 s.
async fn pid_in(b: &Board, pane: &Terminal, program: &str) -> i32 {
    let root = pid_of(b, pane).await;
    let since = std::time::Instant::now();
    loop {
        if let Some(pid) = running_under(&crate::proc_table::snapshot().expect("a process table"), root, program) {
            return pid;
        }
        assert!(since.elapsed() < Duration::from_secs(30), "nothing in the pane runs {program}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// In a process table (`proc_table::snapshot`'s rows), the process under
/// `root` whose program is `program`, the deepest if more than one is. Not one
/// whose command line only names it: the shell a pane runs a command through
/// (`sh -c "env … /x/claude …"`) names the program in its own arguments, and
/// where that shell forks the command rather than exec it, it stays, the
/// pane's own process, beside the stand-in. Taken for the stand-in, that shell
/// is a person's, and the test read Person: Train 183's red Linux run, which
/// matched `stand_in.pl` anywhere in a command line and took whichever
/// process the table listed first. On the Mac the shell execs the command,
/// so only the stand-in ever matched.
fn running_under(table: &str, root: i32, program: &str) -> Option<i32> {
    let rows: std::collections::HashMap<i32, (i32, String)> = table
        .lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let pid = f.next()?.parse().ok()?;
            let ppid = f.next()?.parse().ok()?;
            Some((pid, (ppid, f.skip(3).collect::<Vec<_>>().join(" "))))
        })
        .collect();
    let depth = |mut at: i32| {
        for n in 0..64 {
            if at == root {
                return Some(n);
            }
            match rows.get(&at) {
                Some(&(ppid, _)) if ppid > 1 => at = ppid,
                _ => return None,
            }
        }
        None
    };
    let runs = |args: &str| args.split_whitespace().next().is_some_and(|first| first.rsplit('/').next() == Some(program));
    rows.iter().filter(|(_, (_, args))| runs(args)).filter_map(|(&pid, _)| depth(pid).map(|d| (d, pid))).max().map(|(_, pid)| pid)
}

/// The shell a pane runs a command through names the program in its own
/// arguments; only the process that runs it is taken, and the deepest of two.
#[test]
fn the_process_that_runs_a_program_is_found_not_the_shell_that_names_it() {
    let table = "\
100 1 100 ?? S tmux -L farcooler-x new-session
200 100 200 pts/1 S+ sh -c env A=1 '/x/si/claude' '/x/si/stand_in.pl' claude /x/c /x/l
300 200 200 pts/1 S+ /x/si/claude /x/si/stand_in.pl claude /x/c /x/l
400 300 200 pts/1 S+ /x/si/claude -e 1
500 1 500 pts/9 S+ /y/claude stand_in.pl
";
    assert_eq!(running_under(table, 200, "claude"), Some(400), "the deepest under the pane");
    assert_eq!(running_under(&table.replace("400 300", "400 999"), 200, "claude"), Some(300));
    assert_eq!(running_under(table, 200, "stand_in.pl"), None, "a program's argument is not its program");
    assert_eq!(running_under(table, 600, "claude"), None);
}

/// Who a caller is, by its process: a person in a plain shell pane, in none,
/// or remote; an agent in a pane launched as one; unknown never a person
/// (review 1, B3).
#[tokio::test]
async fn a_callers_seat_is_read_from_its_process() {
    use crate::caller_seat::{Seat, caller_seat};
    let b = board().await;
    let shell = b.shell_pane().await;
    assert_eq!(caller_seat(&b.svc, Some(pid_of(&b, &shell).await)).await, Seat::Person, "a person's own shell pane");
    let lane = b.agent("Agent 2", "claude").await;
    assert_eq!(caller_seat(&b.svc, Some(pid_of(&b, &lane).await)).await, Seat::Agent(Some(lane.id)));
    let outside = super::seat_tests::Orphan::spawn(6051);
    assert_eq!(caller_seat(&b.svc, Some(outside.0)).await, Seat::Person);
    assert_eq!(caller_seat(&b.svc, None).await, Seat::Person);
    assert_eq!(Author::of(Seat::Unreadable), Author::Unknown, "never taken for a person");
}

/// A claude in an adopted shell pane has no `FARCOOLER_ACTOR`, so its CLI
/// claims `user`, but the claude above it in the pane makes it an agent. Its
/// message to a lane goes by the lane's mailbox as the orchestrator's,
/// never typed as a person's turn (review 1, B1, B3 and N1). Round 1 typed it.
#[tokio::test]
async fn a_claude_in_a_shell_pane_messaging_a_lane_goes_by_the_mailbox() {
    let b = board().await;
    b.watcher.with_timing(|t| t.told_spacing = crate::watch::Timing::SHORT);
    let lane = b.agent("Agent 2", "claude").await;
    let mailbox = armed(&b, &lane);
    let si = b.stand_in(&lane, "claude", "claude").await;
    b.doing(lane.id, AgentActivity::Idle).await;
    let shell = b.adopted_shell().await;
    let _claude = b.stand_in(&shell, "claude", "claude").await;
    let pid = pid_in(&b, &shell, "claude").await;
    assert_eq!(crate::caller_seat::caller_seat(&b.svc, Some(pid)).await, crate::caller_seat::Seat::Agent(Some(shell.id)));
    assert_eq!(call_from(&b, pid, message_to(&b, &b.task.key, "Approve the merge")).await, None);
    b.pump().await;
    assert_eq!(mailbox.take_oldest().unwrap().as_deref(), Some("[from the orchestrator] Approve the merge"));
    assert!(!si.log().contains("PASTE"), "typed as a person's turn: {}", si.log());

    // The same CLI from outside every pane is a person's: typed.
    let outside = super::seat_tests::Orphan::spawn(6052);
    assert_eq!(call_from(&b, outside.0, message_to(&b, &b.task.key, "Ship it")).await, None);
    // Past the spacing after the first was told.
    for _ in 0..300 {
        b.pump().await;
        if si.log().contains("PASTE") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    si.submits(1).await;
    assert_eq!(si.submitted(), ["[from the terminal] Ship it"], "{}", si.log());
}

/// A person typing the CLI in their own plain shell pane, with no agent
/// above it, is a person: their tell and their message are typed as their
/// own turn, never sent by a mailbox (review 1, B3). Round 3 took them for
/// an agent's: the tell was refused `no_task`, the message mailboxed.
#[tokio::test]
async fn a_person_in_a_plain_shell_pane_is_typed_as_their_own_turn() {
    let b = board().await;
    b.watcher.with_timing(|t| t.told_spacing = crate::watch::Timing::SHORT);
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    let _hook = super::compose_tests::hook_on_submit(&b, &si);
    let lane = b.agent("Agent 2", "claude").await;
    let mailbox = armed(&b, &lane);
    let other = b.stand_in(&lane, "claude", "claude").await;
    b.doing(lane.id, AgentActivity::Idle).await;
    // A command the person typed in their own shell pane, as the CLI is.
    let mine = b.shell_pane().await;
    super::seat_tests::type_into(&b, &mine, "sleep 6061").await;
    let me = super::seat_tests::pid_running("sleep 6061").await;
    b.svc.inventory.refresh().await;
    assert_eq!(call_from(&b, me, tell(&orchestrator, "land it")).await, None);
    assert_eq!(si.submitted(), ["land it"], "{}", si.log());
    assert_eq!(call_from(&b, me, message_to(&b, &b.task.key, "Ship it")).await, None);
    for _ in 0..300 {
        b.pump().await;
        if other.log().contains("PASTE") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    other.submits(1).await;
    assert_eq!(other.submitted(), ["[from the terminal] Ship it"], "{}", other.log());
    assert_eq!(mailbox.take_oldest().unwrap(), None, "never mailboxed");
}

/// A caller not known to be a person is refused `in_pane`, with nothing
/// typed or filed, never sent another way (review 1, B3).
#[tokio::test]
async fn an_unknown_caller_is_refused_in_pane() {
    let b = board().await;
    let (agent, si) = super::compose_tests::idle_claude(&b).await;
    let said = b.watcher.deliver(agent.id, Delivery { author: Author::Unknown, ..Delivery::prompt("land it") }).await;
    assert!(matches!(said, Err(DomainError::Conflict { what: "in_pane" })), "{said:?}");
    let req = message_to(&b, &b.task.key, "land it");
    let said = b.watcher.message_send_from(req, crate::people::Person::Owner, pb::Scope::HostAdmin, crate::caller_seat::Seat::Unreadable).await;
    assert!(matches!(said, Err(DomainError::Conflict { what: "in_pane" })), "{said:?}");
    assert!(b.messages().is_empty(), "nothing filed");
    assert!(!si.log().contains("PASTE"), "{}", si.log());
}

/// An agent's tell with no card on either side (ad hoc orchestration on
/// Main) isn't refused: it goes straight into the orchestrator's mailbox,
/// tagged with its sender (the owner's ruling, review 1).
#[tokio::test]
async fn an_agents_tell_with_no_card_goes_unfiled_to_the_mailbox() {
    let b = board().await;
    let orchestrator = b.orchestrator().await;
    let mailbox = armed(&b, &orchestrator);
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    let main = b.svc.store.list_worktrees_for_repository(b.lane.repository_id).unwrap().into_iter().find(|w| w.is_main_checkout).expect("the main checkout");
    let adhoc = b.svc.create_terminal_with_prompt(main.id, "Scout", "claude", None, None).await.expect("an agent");
    assert!(crate::task_link::task_of(&b.svc.store, &adhoc).is_none(), "works no card");
    let pid = pid_of(&b, &adhoc).await;
    assert_eq!(call_from(&b, pid, tell(&orchestrator, "found it")).await, None);
    assert_eq!(mailbox.take_oldest().unwrap().as_deref(), Some("[from Scout] found it"));
    assert!(b.messages().is_empty(), "unfiled");
    assert!(!si.log().contains("PASTE"), "{}", si.log());
}

/// A tell, or a composed message, from inside a pane may be an agent's, and
/// an agent's words are never the person's turn (R-78): it goes by the
/// orchestrator's mailbox as an agent message, never typed (review 1, the
/// owner's ruling). And as a second guard nothing a box reads as a command
/// or a mode comes first, so no skill or custom command runs (B2): refused
/// at the boundary, nothing filed. Round 2 typed the plain text.
#[tokio::test]
async fn an_agents_tell_goes_by_the_mailbox_and_runs_no_command() {
    let b = board().await;
    let orchestrator = b.orchestrator().await;
    let mailbox = armed(&b, &orchestrator);
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    let lane = b.agent("Agent 2", "claude").await;
    let pid = pid_of(&b, &lane).await;
    for text in ["/ship it", " /loop", "!ls", "#remember", "@src/main.rs", "&x", "$x", "?", "\\x"] {
        assert_eq!(call_from(&b, pid, tell(&orchestrator, text)).await.as_deref(), Some("resource-conflict:command"), "{text:?}");
    }
    assert!(b.messages().is_empty(), "nothing filed: {:?}", b.messages());
    assert_eq!(call_from(&b, pid, tell(&orchestrator, "land it, then /review")).await, None);
    let mut composed = tell(&orchestrator, "and the docs");
    composed.method = "terminal.compose".into();
    assert_eq!(call_from(&b, pid, composed).await, None);
    b.pump().await;
    // As the lane's own, a peer's by the inbox socket (review 1, N1).
    let key = &b.task.key;
    let sender = farcooler_agent_hooks::mailbox::Sender { name: key.clone(), reply_to: key.clone() };
    let said = |text: &str| Some(farcooler_agent_hooks::mailbox::with_sender(&sender, &format!("[from {key}] {text}")));
    assert_eq!(mailbox.take_oldest().unwrap(), said("land it, then /review"));
    b.pump().await;
    assert_eq!(mailbox.take_oldest().unwrap(), said("and the docs"));
    assert!(!si.log().contains("PASTE"), "typed as the person's turn: {}", si.log());
}

/// A person's tell keeps compose's rules: `#` and `@` are typed as text,
/// and claude's own commands hand off to the terminal.
#[tokio::test]
async fn a_persons_tell_keeps_composes_rules() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    let _hook = super::compose_tests::hook_on_submit(&b, &si);
    let outside = super::seat_tests::Orphan::spawn(6053);
    let me = outside.0;
    assert_eq!(call_from(&b, me, tell(&orchestrator, "/cost")).await.as_deref(), Some("resource-conflict:handoff"));
    assert_eq!(call_from(&b, me, tell(&orchestrator, "#remember this")).await, None);
    assert_eq!(si.submitted(), ["#remember this"], "{}", si.log());
}

/// The trust Enter from inside a pane is refused `not_a_person` at the
/// boundary, whatever actor it claims (review 1, S1).
#[tokio::test]
async fn a_trust_enter_from_a_pane_is_not_a_persons() {
    let b = board().await;
    let lane = b.agent("Agent 2", "claude").await;
    let pid = pid_of(&b, &lane).await;
    let answer = |actor: &str| farcooler_protocol::v1::Request {
        method: "terminal.agent_answer".into(),
        target_resource_id: Some(crate::wire::id_bytes(lane.id)),
        payload: Some(pb::request::Payload::AgentAnswer(pb::AgentAnswer {
            terminal_id: crate::wire::id_bytes(lane.id),
            request_id: format!("{}x", crate::needs_you::TRUST_ASK_PREFIX),
            option_id: "trust".into(),
            actor: actor.into(),
            ..Default::default()
        })),
        ..Default::default()
    };
    for actor in ["", "user"] {
        assert_eq!(call_from(&b, pid, answer(actor)).await.as_deref(), Some("resource-conflict:not_a_person"), "{actor:?}");
    }
    assert_ne!(
        call_from(&b, super::seat_tests::Orphan::spawn(6054).0, answer("user")).await.as_deref(),
        Some("resource-conflict:not_a_person"),
        "a person outside every pane passes this check"
    );
}

/// A message, `name`d in `session`'s mailbox, dropped there by a daemon from
/// before ov-620, which recorded its route.
fn dropped_before_the_upgrade(b: &Board, pane: &Terminal, mailbox: &Mailbox, text: &str) -> String {
    let wake = b.messages().remove(0);
    let name = format!("{:013}-{}", wake.enqueued_at, wake.note.simple());
    mailbox.drop_message(&name, text).unwrap();
    let routes = crate::claude_mailbox::routes_of(b.svc.root_dir());
    std::fs::create_dir_all(&routes).unwrap();
    std::fs::write(routes.join(wake.note.simple().to_string()), format!("{} {SESSION}", pane.id)).unwrap();
    name
}

/// A person's message the old daemon dropped and the waiter took is settled
/// as told, never typed a second time (review 1, S3).
#[tokio::test]
async fn a_persons_message_taken_before_the_upgrade_is_not_typed_again() {
    let b = board().await;
    let pane = b.agent("Agent 2", "claude").await;
    let mailbox = armed(&b, &pane);
    let si = b.stand_in(&pane, "claude", "claude").await;
    b.doing(pane.id, AgentActivity::Idle).await;
    b.send("user", &b.task.key, "Rename foo to bar").await.unwrap();
    dropped_before_the_upgrade(&b, &pane, &mailbox, "[from the terminal] Rename foo to bar");
    assert!(mailbox.take_oldest().unwrap().is_some(), "the waiter took it");
    b.pump().await;
    assert!(b.messages().is_empty(), "settled: {:?}", b.messages());
    assert!(!si.log().contains("PASTE"), "typed twice: {}", si.log());
}

/// One the waiter hasn't taken yet is taken back, then typed: once.
#[tokio::test]
async fn a_persons_message_waiting_from_before_the_upgrade_is_taken_back_and_typed() {
    let b = board().await;
    let pane = b.agent("Agent 2", "claude").await;
    let mailbox = armed(&b, &pane);
    let si = b.stand_in(&pane, "claude", "claude").await;
    b.doing(pane.id, AgentActivity::Idle).await;
    b.send("user", &b.task.key, "Rename foo to bar").await.unwrap();
    let name = dropped_before_the_upgrade(&b, &pane, &mailbox, "[from the terminal] Rename foo to bar");
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), ["[from the terminal] Rename foo to bar"], "{}", si.log());
    assert_eq!(mailbox.slot(&name), farcooler_agent_hooks::mailbox::Slot::Absent, "taken back");
    assert_eq!(mailbox.take_oldest().unwrap(), None, "so the waiter can't deliver it too");
}
