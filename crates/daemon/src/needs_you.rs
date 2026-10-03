//! The needs-you rollup: everything on this runner a person has to act on.
//!
//! One definition, computed here and nowhere else, because only the daemon
//! holds all four facts it needs: held asks with their ids and options, chat
//! permissions, agent activity, and the tasks. Each runner computes its own
//! list; a client merges runners by `rank`. See spec §2 of
//! `docs/superpowers/specs/2026-09-28-workspace-ui-design.md`.
//!
//! **Four kinds, in rank order:** an ask (a permission with an id and
//! options), a block (an agent blocked with no answerable ask, or whose last
//! turn failed), a decision (a task in Needs Decision whose latest question
//! has no later answer), and a review (a task in In Review).
//!
//! **One item per subject.** A signal whose terminal has a task is about the
//! task; an orchestrator's are about its own terminal. When a subject has
//! several signals the most urgent wins and the rest go in `also`.
//!
//! `assemble` is pure: `gather` reads the daemon's state into `Inputs`, and
//! everything after that is a function of those inputs and a clock, which is
//! what the tests here pin.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use farcooler_agent::event::PermissionOption;
use farcooler_protocol::v1::{self as pb, AgentActivity, NeedsYouKind};
use farcooler_store::models::{self, NoteKind, TaskStatus};
use farcooler_store::TaskScope;
use uuid::Uuid;

use crate::wire::{id_bytes, pane_mode, terminal_role, timestamp};

/// One tier's width on the rank scale: `Terminal.rank`'s, so an item and a
/// terminal read on one scale. `farcooler_core::feed`'s constant is private;
/// `ranks_share_the_terminal_scale` pins that the two agree.
const TIER_SPAN: u32 = 100_000_000;

/// What the watcher last decided about one terminal.
#[derive(Debug, Clone, Default)]
pub struct Observation {
    pub activity: AgentActivity,
    /// Unix milliseconds, from when `activity` began.
    pub state_since: i64,
    pub blocked_question: Option<String>,
    pub turn_failed: bool,
    /// What is running in the pane: `claude`, `codex`. What a notice calls
    /// the agent, and so what an item does.
    pub command: String,
    pub chat_capable: bool,
}

/// A worktree, and its `+N -M` when the watch loop has measured it.
#[derive(Debug, Clone)]
pub struct WorktreeFacts {
    pub worktree: models::Worktree,
    pub counts: Option<(u32, u32)>,
}

/// Everything `assemble` reads. Filled by `gather`; built by hand in tests.
#[derive(Debug, Clone, Default)]
pub struct Inputs {
    /// The watcher's view of every terminal it has sampled.
    pub observed: HashMap<Uuid, Observation>,
    /// Every terminal row. Ended ones are skipped here, not by the caller.
    pub terminals: Vec<models::Terminal>,
    /// `HookAsks::open`: every held, offered hook ask.
    pub hook_asks: Vec<(Uuid, String, SystemTime)>,
    /// `AgentSupervisor::open_permission`, per terminal that has one.
    pub permissions: HashMap<Uuid, (String, Vec<PermissionOption>, SystemTime)>,
    /// Every task in Needs Decision or In Review, and every task a live
    /// terminal was opened for.
    pub tasks: Vec<models::Task>,
    /// The QUESTION and ANSWER notes of each task in Needs Decision, in the
    /// order they were appended.
    pub notes: HashMap<Uuid, Vec<models::TaskNote>>,
    pub worktrees: HashMap<Uuid, WorktreeFacts>,
    /// Workspace names, by id.
    pub workspaces: HashMap<Uuid, String>,
}

/// Read the daemon's state into `Inputs`.
pub async fn gather(
    svc: &crate::service::Service,
    watcher: &crate::watch::Watcher,
) -> farcooler_core::Result<Inputs> {
    let mut inputs = Inputs { observed: watcher.observed_snapshot().await, ..Inputs::default() };

    for worktree in svc.store.list_all_worktrees()? {
        inputs.terminals.extend(svc.store.list_terminals_for_worktree(worktree.id)?);
        let counts = match svc.review_cache.counts(worktree.id) {
            crate::review::Counts::Known(_, ins, del) => Some((ins, del)),
            _ => None,
        };
        inputs.worktrees.insert(worktree.id, WorktreeFacts { worktree, counts });
    }

    inputs.hook_asks = svc.hooks().asks().open();
    for t in &inputs.terminals {
        if let Some(open) = svc.agents().open_permission(t.id) {
            inputs.permissions.insert(t.id, open);
        }
    }

    let mut tasks: HashMap<Uuid, models::Task> = HashMap::new();
    for workspace in svc.store.list_workspaces(None)? {
        for status in [TaskStatus::NeedsDecision, TaskStatus::InReview] {
            for task in svc.store.list_tasks(TaskScope::Workspace(workspace.id), Some(status))? {
                tasks.insert(task.id, task);
            }
        }
        inputs.workspaces.insert(workspace.id, workspace.name);
    }
    for id in inputs.terminals.iter().filter(|t| !crate::wire::has_ended(t)).filter_map(|t| t.task_id) {
        if !tasks.contains_key(&id)
            && let Ok(task) = svc.store.get_task(id)
        {
            tasks.insert(id, task);
        }
    }
    for task in tasks.values().filter(|t| t.status == TaskStatus::NeedsDecision) {
        let notes = svc
            .store
            .notes_for(task.id, None)?
            .into_iter()
            .filter(|n| matches!(n.kind, NoteKind::Question | NoteKind::Answer))
            .collect();
        inputs.notes.insert(task.id, notes);
    }
    inputs.tasks = tasks.into_values().collect();
    Ok(inputs)
}

/// What an item is about: a task, or a terminal with no task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Subject {
    Task(Uuid),
    Terminal(Uuid),
}

/// One reason a subject needs somebody, before signals are folded into items.
struct Signal {
    kind: NeedsYouKind,
    id: String,
    since_ms: i64,
    subject: Subject,
    terminal: Option<Uuid>,
    question: String,
    detail: Option<String>,
    ask_id: Option<String>,
    actions: Vec<pb::NeedsYouAction>,
}

fn tier(kind: NeedsYouKind) -> u32 {
    match kind {
        NeedsYouKind::Ask => 0,
        NeedsYouKind::Blocked => 1,
        NeedsYouKind::Decision => 2,
        NeedsYouKind::Review | NeedsYouKind::Unspecified => 3,
    }
}

/// `Terminal.rank`'s rule: the tier dominates, and within it the older sorts
/// first. A duration, never a clock reading, so two runners' items merge by
/// rank without comparing clocks.
fn rank(kind: NeedsYouKind, since_ms: i64, now_ms: i64) -> u32 {
    let age_secs = (now_ms.saturating_sub(since_ms).max(0) / 1000).min(i64::from(TIER_SPAN) - 1) as u32;
    tier(kind) * TIER_SPAN + (TIER_SPAN - 1 - age_secs)
}

fn millis(at: SystemTime) -> i64 {
    at.duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or_default()
}

fn action(id: &str, title: &str, destructive: bool, primary: bool) -> pb::NeedsYouAction {
    pb::NeedsYouAction { id: id.to_string(), title: title.to_string(), destructive, primary }
}

fn open_action() -> pb::NeedsYouAction {
    action("open", "Open", false, false)
}

/// What a row calls a terminal: the agent running in it (`claude`), as a
/// notice does, else its title.
fn label(t: &models::Terminal, observed: Option<&Observation>) -> String {
    observed
        .map(|o| o.command.trim().to_string())
        .filter(|c| !c.is_empty())
        .or_else(|| Some(t.title.clone()).filter(|t| !t.is_empty()))
        .unwrap_or_else(|| t.command_preset.clone())
}

/// A question's options, from its note's `extra_json.options`
/// (`farcooler task ask --option`).
fn decision_options(note: &models::TaskNote) -> Vec<String> {
    note.extra["options"]
        .as_array()
        .map(|a| a.iter().filter_map(|o| o.as_str()).map(str::to_string).collect())
        .unwrap_or_default()
}

/// The terminal's signal, if it has one: an ask, else a block, else a failed
/// turn.
fn terminal_signal(t: &models::Terminal, inputs: &Inputs, subject: Subject) -> Option<Signal> {
    let observed = inputs.observed.get(&t.id);
    let who = label(t, observed);
    if let Some((id, options, at)) = inputs.permissions.get(&t.id) {
        // A hook ask counts only while the hook is still holding it: its
        // `Permission` can outlive the hold in the ring, and an answer to it
        // would be refused.
        let held = if id.starts_with(crate::hook_asks::HOOK_ASK_PREFIX) {
            inputs.hook_asks.iter().find(|(terminal, held, _)| *terminal == t.id && held == id).map(|h| h.2)
        } else {
            // A chat's ask ends with its `Resolved` (recorded by
            // `terminal.agent_answer`), its turn ending, or the shim starting
            // over: `open_permission` already stops there. Not by the agent's
            // activity, which a sibling subagent's work folds to Working while
            // this ask still waits.
            Some(*at)
        };
        if let Some(since) = held {
            let question = options
                .iter()
                .find(|o| o.kind.starts_with("allow"))
                .map(|o| o.name.clone())
                .unwrap_or_else(|| format!("{who} is asking to use a tool"));
            return Some(Signal {
                kind: NeedsYouKind::Ask,
                id: format!("ask:{id}"),
                since_ms: millis(since),
                subject,
                terminal: Some(t.id),
                question,
                detail: None,
                ask_id: Some(id.clone()),
                actions: options
                    .iter()
                    .map(|o| action(&o.id, &o.name, o.kind.starts_with("reject"), o.kind.starts_with("allow")))
                    .collect(),
            });
        }
    }
    let observed = observed?;
    let question = match observed.activity {
        AgentActivity::Blocked => observed.blocked_question.clone().unwrap_or_else(|| format!("{who} needs you")),
        // A finished turn is not an item, for anybody; a failed one is.
        AgentActivity::Done if observed.turn_failed => "Its last turn didn’t finish".to_string(),
        _ => return None,
    };
    Some(Signal {
        kind: NeedsYouKind::Blocked,
        id: format!("blocked:{}", t.id),
        since_ms: observed.state_since,
        subject,
        terminal: Some(t.id),
        question,
        detail: None,
        ask_id: None,
        actions: vec![open_action()],
    })
}

/// Whether `task`, in Needs Decision, is still waiting on its decision, and
/// on which question: `Some(Some(q))` for its latest QUESTION, `Some(None)`
/// for none asked, `None` once answered. `notes` are its QUESTION and ANSWER
/// notes, oldest first.
///
/// Answered: an ANSWER after the latest QUESTION, or, with no question, one
/// since the task last moved. An answer only appends a note; the task stays
/// where the orchestrator left it. The task notice composer asks the same
/// question, so a notice and the Needs You row can't disagree (ov-94).
pub(crate) fn waiting_question<'a>(
    task: &models::Task,
    notes: &'a [models::TaskNote],
) -> Option<Option<&'a models::TaskNote>> {
    let asked = notes.iter().rposition(|n| n.kind == NoteKind::Question);
    let answered = match asked {
        Some(q) => notes[q + 1..].iter().any(|n| n.kind == NoteKind::Answer),
        None => notes.iter().any(|n| n.kind == NoteKind::Answer && n.at >= task.status_since),
    };
    (!answered).then(|| asked.map(|q| &notes[q]))
}

/// A task's signal from the board, if it has one.
fn task_signal(task: &models::Task, inputs: &Inputs) -> Option<Signal> {
    match task.status {
        TaskStatus::NeedsDecision => {
            let notes = inputs.notes.get(&task.id).map(Vec::as_slice).unwrap_or_default();
            let question = waiting_question(task, notes)?;
            Some(Signal {
                kind: NeedsYouKind::Decision,
                id: format!("decision:{}", task.id),
                since_ms: question.map_or(task.status_since, |q| q.at),
                subject: Subject::Task(task.id),
                terminal: None,
                question: question
                    .map(|q| q.body.clone())
                    .filter(|b| !b.trim().is_empty())
                    .unwrap_or_else(|| "Needs a decision".to_string()),
                detail: None,
                ask_id: None,
                actions: question
                    .map(decision_options)
                    .unwrap_or_default()
                    .iter()
                    .map(|o| action(o, o, false, false))
                    .collect(),
            })
        }
        TaskStatus::InReview => Some(Signal {
            kind: NeedsYouKind::Review,
            id: format!("review:{}", task.id),
            since_ms: task.status_since,
            subject: Subject::Task(task.id),
            terminal: None,
            question: "Ready for review".to_string(),
            detail: task
                .worktree_id
                .and_then(|w| inputs.worktrees.get(&w))
                .and_then(|w| w.counts)
                .map(|(ins, del)| format!("+{ins} −{del}")),
            ask_id: None,
            actions: vec![open_action()],
        }),
        _ => None,
    }
}

/// Every item on this runner, ranked: smaller `rank` first.
pub fn assemble(inputs: &Inputs, now: SystemTime) -> Vec<pb::NeedsYouItem> {
    let now_ms = millis(now);
    let tasks: HashMap<Uuid, &models::Task> = inputs.tasks.iter().map(|t| (t.id, t)).collect();
    let terminals: HashMap<Uuid, &models::Terminal> = inputs.terminals.iter().map(|t| (t.id, t)).collect();

    let mut signals: Vec<Signal> = Vec::new();
    for t in inputs.terminals.iter().filter(|t| !crate::wire::has_ended(t)) {
        // An orchestrator is never a task's agent, so its signals are about
        // its own terminal whatever its row says.
        let subject = match crate::task_link::bound_task(t).filter(|id| tasks.contains_key(id)) {
            Some(task) => Subject::Task(task),
            None => Subject::Terminal(t.id),
        };
        signals.extend(terminal_signal(t, inputs, subject));
    }
    for task in &inputs.tasks {
        signals.extend(task_signal(task, inputs));
    }

    // One item per subject: the most urgent signal wins, the oldest within a
    // kind, and the others' kinds go in `also`.
    let mut by_subject: HashMap<Subject, Vec<Signal>> = HashMap::new();
    for s in signals {
        by_subject.entry(s.subject).or_default().push(s);
    }
    let mut items: Vec<pb::NeedsYouItem> = by_subject
        .into_values()
        .map(|mut group| {
            group.sort_by_key(|s| (tier(s.kind), s.since_ms));
            let mut rest = group.split_off(1);
            let winner = group.pop().expect("a group has a signal");
            let mut also: Vec<NeedsYouKind> = Vec::new();
            for s in &rest {
                if s.kind != winner.kind && !also.contains(&s.kind) {
                    also.push(s.kind);
                }
            }
            let terminal = winner.terminal.or_else(|| rest.iter_mut().find_map(|s| s.terminal));
            item(winner, also, terminal, &tasks, &terminals, inputs, now_ms)
        })
        .collect();
    items.sort_by(|a, b| a.rank.cmp(&b.rank).then_with(|| a.id.cmp(&b.id)));
    items
}

fn item(
    winner: Signal,
    also: Vec<NeedsYouKind>,
    terminal: Option<Uuid>,
    tasks: &HashMap<Uuid, &models::Task>,
    terminals: &HashMap<Uuid, &models::Terminal>,
    inputs: &Inputs,
    now_ms: i64,
) -> pb::NeedsYouItem {
    let task = match winner.subject {
        Subject::Task(id) => tasks.get(&id).copied(),
        Subject::Terminal(_) => None,
    };
    let terminal = terminal.and_then(|id| terminals.get(&id).copied());
    let worktree = terminal
        .map(|t| t.worktree_id)
        .or_else(|| task.and_then(|t| t.worktree_id))
        .and_then(|id| inputs.worktrees.get(&id));
    // The task's board, then the terminal's workspace, then the worktree's
    // owner. None of the three is an Unclaimed item.
    let workspace = task
        .map(|t| t.workspace_id)
        .filter(|id| !id.is_nil())
        .or_else(|| terminal.and_then(|t| t.workspace_id))
        .or_else(|| worktree.and_then(|w| w.worktree.workspace_id));
    let repository = task.map(|t| t.repository_id).or_else(|| worktree.map(|w| w.worktree.repository_id));

    pb::NeedsYouItem {
        id: winner.id,
        kind: winner.kind as i32,
        also: also.into_iter().map(|k| k as i32).collect(),
        rank: rank(winner.kind, winner.since_ms, now_ms),
        since: Some(timestamp(winner.since_ms)),
        workspace_id: workspace.map(id_bytes).unwrap_or_default(),
        workspace_name: workspace.and_then(|w| inputs.workspaces.get(&w).cloned()).unwrap_or_default(),
        repository_id: repository.map(id_bytes).unwrap_or_default(),
        task: task.map(crate::wire::task_ref),
        terminal: terminal.map(|t| {
            let observed = inputs.observed.get(&t.id);
            pb::TerminalRef {
                id: id_bytes(t.id),
                worktree_id: id_bytes(t.worktree_id),
                label: label(t, observed),
                role: terminal_role(t.role),
                pane_mode: pane_mode(t.pane_mode),
                chat_capable: observed.is_some_and(|o| o.chat_capable),
            }
        }),
        worktree: worktree.map(|w| pb::WorktreeRef {
            id: id_bytes(w.worktree.id),
            name: w.worktree.name(),
            branch: w.worktree.branch.clone(),
            insertions: w.counts.map_or(0, |c| c.0),
            deletions: w.counts.map_or(0, |c| c.1),
        }),
        question: winner.question,
        detail: winner.detail,
        ask_id: winner.ask_id,
        actions: winner.actions,
    }
}

/// The item a peer below Control scope may see.
///
/// An ask's option names carry the raw command or file path, which travels
/// otherwise only on the agent channel (Control). So below Control an item
/// keeps its id, kind, `also`, rank, `since`, workspace, task and terminal;
/// its question becomes a fixed sentence for its kind; and its detail, ask
/// id, actions and worktree go. One rule for every kind, though a decision's
/// question is a board note Read may already see.
///
/// The only place the Read shape is made.
pub fn redact_below_control(item: pb::NeedsYouItem) -> pb::NeedsYouItem {
    let agent = item
        .terminal
        .as_ref()
        .map(|t| t.label.trim().to_string())
        .filter(|l| !l.is_empty() && !l.contains('/'))
        .unwrap_or_else(|| "The agent".to_string());
    let question = match NeedsYouKind::try_from(item.kind).unwrap_or(NeedsYouKind::Unspecified) {
        NeedsYouKind::Ask => format!("{agent} is asking to use a tool"),
        NeedsYouKind::Blocked => format!("{agent} needs you"),
        NeedsYouKind::Decision => "Needs a decision".to_string(),
        NeedsYouKind::Review | NeedsYouKind::Unspecified => "Ready for review".to_string(),
    };
    pb::NeedsYouItem {
        question,
        detail: None,
        ask_id: None,
        actions: Vec::new(),
        worktree: None,
        ..item
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_protocol::v1::TerminalIntent;
    use farcooler_store::models::{Actor, PaneMode, TerminalRole};
    use std::time::Duration;

    const MINUTE: i64 = 60_000;

    /// A fixed "now", so every age below is exact.
    fn now() -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(2_000_000_000)
    }

    fn ago(ms: i64) -> i64 {
        millis(now()) - ms
    }

    fn ago_time(ms: i64) -> SystemTime {
        now() - Duration::from_millis(ms as u64)
    }

    struct Fleet {
        inputs: Inputs,
        worktree: Uuid,
        workspace: Uuid,
        repository: Uuid,
    }

    impl Fleet {
        fn new() -> Self {
            let repository = Uuid::now_v7();
            let workspace = Uuid::now_v7();
            let mut inputs = Inputs::default();
            inputs.workspaces.insert(workspace, "Billing".into());
            let mut fleet = Fleet { inputs, worktree: Uuid::nil(), workspace, repository };
            fleet.worktree = fleet.a_worktree(Some(workspace), false);
            fleet
        }

        fn a_worktree(&mut self, owner: Option<Uuid>, hidden: bool) -> Uuid {
            let id = Uuid::now_v7();
            self.inputs.worktrees.insert(
                id,
                WorktreeFacts {
                    worktree: models::Worktree {
                        id,
                        repository_id: self.repository,
                        branch: "fc-3-webhooks".into(),
                        worktree_path: "/tmp/probe/fc-3-webhooks".into(),
                        hidden,
                        creation_failed: false,
                        is_main_checkout: false,
                        worktree_missing: false,
                        ordinal: 0,
                        resource_version: 1,
                        workspace_id: owner,
                        claim_source: None,
                    },
                    counts: Some((18, 40)),
                },
            );
            id
        }

        fn a_terminal(&mut self, worktree: Uuid, role: TerminalRole, task: Option<Uuid>) -> Uuid {
            let id = Uuid::now_v7();
            self.inputs.terminals.push(models::Terminal {
                id,
                worktree_id: worktree,
                title: "Terminal 3".into(),
                command_preset: "claude".into(),
                intent: TerminalIntent::Running,
                runtime_confirmed: true,
                exit_code: None,
                exit_signal: None,
                lease_generation: 0,
                epoch: 0,
                columns: 80,
                rows: 24,
                resource_version: 1,
                pane_mode: PaneMode::Terminal,
                agent_session_id: None,
                task_id: task,
                workspace_id: None,
                role,
                split_of: None,
                split_of_orchestrator: None,
            });
            self.observe(id, AgentActivity::Working, 0);
            id
        }

        fn agent(&mut self, task: Option<Uuid>) -> Uuid {
            self.a_terminal(self.worktree, TerminalRole::Agent, task)
        }

        fn observe(&mut self, terminal: Uuid, activity: AgentActivity, since_ago: i64) {
            let o = self.inputs.observed.entry(terminal).or_default();
            o.activity = activity;
            o.state_since = ago(since_ago);
            o.command = "claude".into();
        }

        fn task(&mut self, status: TaskStatus, since_ago: i64) -> Uuid {
            let id = Uuid::now_v7();
            self.inputs.tasks.push(models::Task {
                id,
                key: format!("bil-{}", self.inputs.tasks.len() + 1),
                repository_id: self.repository,
                workspace_id: self.workspace,
                title: "Invoice PDF export".into(),
                status,
                status_since: ago(since_ago),
                intent: String::new(),
                acceptance: vec![],
                constraints: vec![],
                worktree_id: Some(self.worktree),
                labels: vec![],
                resource_version: 1,
                created_at: 0,
                updated_at: 0,
            });
            id
        }

        fn note(&mut self, task: Uuid, kind: NoteKind, body: &str, extra: serde_json::Value, at_ago: i64) {
            self.inputs.notes.entry(task).or_default().push(models::TaskNote {
                id: Uuid::now_v7(),
                task_id: task,
                kind,
                actor: Actor::Manager,
                at: ago(at_ago),
                body: body.into(),
                extra,
                supersedes: None,
            });
        }

        /// A claude hook ask held and offered on `terminal`.
        fn hook_ask(&mut self, terminal: Uuid, allow: &str, since_ago: i64) -> String {
            let id = format!("{}{}", crate::hook_asks::HOOK_ASK_PREFIX, Uuid::now_v7());
            let at = ago_time(since_ago);
            self.inputs.hook_asks.retain(|(t, _, _)| *t != terminal);
            self.inputs.hook_asks.push((terminal, id.clone(), at));
            self.inputs.permissions.insert(
                terminal,
                (
                    id.clone(),
                    vec![
                        PermissionOption { id: "allow".into(), name: allow.into(), kind: "allow_once".into() },
                        PermissionOption { id: "deny".into(), name: "Deny".into(), kind: "reject_once".into() },
                    ],
                    at,
                ),
            );
            self.observe(terminal, AgentActivity::Blocked, since_ago);
            id
        }

        fn items(&self) -> Vec<pb::NeedsYouItem> {
            assemble(&self.inputs, now())
        }
    }

    fn kinds(items: &[pb::NeedsYouItem]) -> Vec<NeedsYouKind> {
        items.iter().map(|i| i.kind()).collect()
    }

    #[test]
    fn a_task_with_a_held_ask_and_a_decision_is_one_ask_item_with_decision_in_also() {
        let mut fleet = Fleet::new();
        let task = fleet.task(TaskStatus::NeedsDecision, 5 * MINUTE);
        fleet.note(task, NoteKind::Question, "Which PDF library?", serde_json::json!({}), 5 * MINUTE);
        let pane = fleet.agent(Some(task));
        fleet.hook_ask(pane, "Allow touch x", MINUTE);
        let items = fleet.items();
        assert_eq!(items.len(), 1, "{items:#?}");
        assert_eq!(items[0].kind(), NeedsYouKind::Ask);
        assert_eq!(items[0].also, vec![NeedsYouKind::Decision as i32]);
        assert_eq!(items[0].task.as_ref().map(|t| t.key.as_str()), Some("bil-1"));
        assert_eq!(items[0].terminal.as_ref().map(|t| t.id.clone()), Some(id_bytes(pane)));
    }

    #[test]
    fn an_ask_outranks_a_block_outranks_a_decision_outranks_a_review() {
        let mut fleet = Fleet::new();
        // Each younger than the one below it, so age can't be what orders them.
        let review = fleet.task(TaskStatus::InReview, 40 * MINUTE);
        let _ = review;
        let decision = fleet.task(TaskStatus::NeedsDecision, 30 * MINUTE);
        fleet.note(decision, NoteKind::Question, "Which?", serde_json::json!({}), 30 * MINUTE);
        let blocked = fleet.agent(None);
        fleet.observe(blocked, AgentActivity::Blocked, 20 * MINUTE);
        let asking = fleet.agent(None);
        fleet.hook_ask(asking, "Allow touch x", MINUTE);
        assert_eq!(
            kinds(&fleet.items()),
            [NeedsYouKind::Ask, NeedsYouKind::Blocked, NeedsYouKind::Decision, NeedsYouKind::Review]
        );
    }

    #[test]
    fn within_a_kind_the_oldest_comes_first() {
        let mut fleet = Fleet::new();
        let young = fleet.agent(None);
        fleet.observe(young, AgentActivity::Blocked, MINUTE);
        let old = fleet.agent(None);
        fleet.observe(old, AgentActivity::Blocked, 9 * MINUTE);
        let ids: Vec<_> = fleet.items().into_iter().map(|i| i.id).collect();
        assert_eq!(ids, [format!("blocked:{old}"), format!("blocked:{young}")]);
    }

    #[test]
    fn a_done_agent_is_not_an_item() {
        let mut fleet = Fleet::new();
        let pane = fleet.agent(None);
        fleet.observe(pane, AgentActivity::Done, MINUTE);
        assert_eq!(fleet.items(), vec![]);
    }

    #[test]
    fn an_orchestrators_finished_turn_is_not_an_item() {
        let mut fleet = Fleet::new();
        let worktree = fleet.worktree;
        let lead = fleet.a_terminal(worktree, TerminalRole::Orchestrator, None);
        fleet.observe(lead, AgentActivity::Done, MINUTE);
        assert_eq!(fleet.items(), vec![]);
    }

    #[test]
    fn an_orchestrators_ask_is_an_item_about_its_own_terminal() {
        let mut fleet = Fleet::new();
        // Even a row that names a task: an orchestrator is never its agent.
        let task = fleet.task(TaskStatus::InProgress, MINUTE);
        let worktree = fleet.worktree;
        let lead = fleet.a_terminal(worktree, TerminalRole::Orchestrator, Some(task));
        let ask = fleet.hook_ask(lead, "Allow touch x", MINUTE);
        let items = fleet.items();
        assert_eq!(items.len(), 1, "{items:#?}");
        assert_eq!(items[0].id, format!("ask:{ask}"));
        assert_eq!(items[0].task, None, "an orchestrator's item named a task");
        assert_eq!(items[0].terminal.as_ref().map(|t| t.role), Some(terminal_role(TerminalRole::Orchestrator)));
    }

    #[test]
    fn a_blocked_codex_with_no_open_ask_is_a_blocked_item() {
        let mut fleet = Fleet::new();
        let pane = fleet.agent(None);
        fleet.observe(pane, AgentActivity::Blocked, MINUTE);
        fleet.inputs.observed.get_mut(&pane).unwrap().command = "codex".into();
        fleet.inputs.observed.get_mut(&pane).unwrap().blocked_question = Some("Allow codex to run tests?".into());
        let items = fleet.items();
        assert_eq!(kinds(&items), [NeedsYouKind::Blocked]);
        assert_eq!(items[0].question, "Allow codex to run tests?");
        assert_eq!(items[0].ask_id, None);
        assert_eq!(items[0].terminal.as_ref().map(|t| t.label.as_str()), Some("codex"));
    }

    #[test]
    fn a_failed_turn_is_a_blocked_item_and_a_seen_one_is_not() {
        let mut fleet = Fleet::new();
        let pane = fleet.agent(None);
        fleet.observe(pane, AgentActivity::Done, MINUTE);
        fleet.inputs.observed.get_mut(&pane).unwrap().turn_failed = true;
        assert_eq!(kinds(&fleet.items()), [NeedsYouKind::Blocked]);
        // `terminal.seen` turns Done into Idle; the verdict stays on the row.
        fleet.inputs.observed.get_mut(&pane).unwrap().activity = AgentActivity::Idle;
        assert_eq!(fleet.items(), vec![]);
    }

    #[test]
    fn a_bad_exit_is_not_an_item() {
        let mut fleet = Fleet::new();
        let pane = fleet.agent(None);
        fleet.observe(pane, AgentActivity::Idle, MINUTE);
        let row = fleet.inputs.terminals.iter_mut().find(|t| t.id == pane).unwrap();
        row.exit_code = Some(101);
        assert_eq!(fleet.items(), vec![]);
        // Nor is one whose last sample still said Blocked.
        fleet.observe(pane, AgentActivity::Blocked, MINUTE);
        assert_eq!(fleet.items(), vec![]);
    }

    #[test]
    fn an_answered_decision_is_not_an_item() {
        let mut fleet = Fleet::new();
        let task = fleet.task(TaskStatus::NeedsDecision, 10 * MINUTE);
        fleet.note(task, NoteKind::Question, "Which PDF library?", serde_json::json!({}), 10 * MINUTE);
        assert_eq!(kinds(&fleet.items()), [NeedsYouKind::Decision]);
        fleet.note(task, NoteKind::Answer, "printpdf", serde_json::json!({}), MINUTE);
        assert_eq!(fleet.items(), vec![], "the task is still in Needs Decision, and answered");
        // A new question reopens it.
        fleet.note(task, NoteKind::Question, "And the font?", serde_json::json!({}), 0);
        assert_eq!(kinds(&fleet.items()), [NeedsYouKind::Decision]);
    }

    #[test]
    fn a_task_in_review_is_a_review_item_with_only_an_open_action() {
        let mut fleet = Fleet::new();
        fleet.task(TaskStatus::InReview, MINUTE);
        let items = fleet.items();
        assert_eq!(kinds(&items), [NeedsYouKind::Review]);
        assert_eq!(items[0].question, "Ready for review");
        assert_eq!(items[0].detail.as_deref(), Some("+18 −40"));
        let actions: Vec<_> = items[0].actions.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(actions, ["open"], "the inbox opens a review; it never approves one");
    }

    #[test]
    fn an_item_takes_the_tasks_workspace_then_the_terminals_then_the_worktree_owners() {
        let mut fleet = Fleet::new();
        let (ops, lead_ws) = (Uuid::now_v7(), Uuid::now_v7());
        fleet.inputs.workspaces.insert(ops, "Ops".into());
        fleet.inputs.workspaces.insert(lead_ws, "Growth".into());

        // The task's board wins over the terminal's and the worktree's.
        let task = fleet.task(TaskStatus::InProgress, MINUTE);
        let on_task = fleet.agent(Some(task));
        fleet.inputs.terminals.iter_mut().find(|t| t.id == on_task).unwrap().workspace_id = Some(ops);
        fleet.observe(on_task, AgentActivity::Blocked, 3 * MINUTE);

        // No task: the terminal's workspace wins over the worktree's owner.
        let own = fleet.agent(None);
        fleet.inputs.terminals.iter_mut().find(|t| t.id == own).unwrap().workspace_id = Some(lead_ws);
        fleet.observe(own, AgentActivity::Blocked, 2 * MINUTE);

        // Neither: the worktree's owner.
        let bare = fleet.agent(None);
        fleet.observe(bare, AgentActivity::Blocked, MINUTE);

        let names: Vec<_> = fleet.items().into_iter().map(|i| i.workspace_name).collect();
        assert_eq!(names, ["Billing", "Growth", "Billing"]);
        let items = fleet.items();
        assert_eq!(items[0].workspace_id, id_bytes(fleet.workspace));
        assert_eq!(items[1].workspace_id, id_bytes(lead_ws));
        assert_eq!(items[2].workspace_id, id_bytes(fleet.workspace));
    }

    #[test]
    fn an_item_in_a_hidden_worktree_still_counts() {
        let mut fleet = Fleet::new();
        let hidden = fleet.a_worktree(Some(fleet.workspace), true);
        let lead = fleet.a_terminal(hidden, TerminalRole::Orchestrator, None);
        fleet.observe(lead, AgentActivity::Blocked, MINUTE);
        assert_eq!(kinds(&fleet.items()), [NeedsYouKind::Blocked]);
    }

    #[test]
    fn an_unclaimed_worktrees_item_has_an_empty_workspace() {
        let mut fleet = Fleet::new();
        let unclaimed = fleet.a_worktree(None, false);
        let pane = fleet.a_terminal(unclaimed, TerminalRole::Agent, None);
        fleet.observe(pane, AgentActivity::Blocked, MINUTE);
        let items = fleet.items();
        assert_eq!(items.len(), 1);
        assert!(items[0].workspace_id.is_empty(), "{:?}", items[0].workspace_id);
        assert_eq!(items[0].workspace_name, "");
        assert_eq!(items[0].repository_id, id_bytes(fleet.repository), "still counted under its repository");
    }

    #[test]
    fn a_decisions_options_become_its_actions() {
        let mut fleet = Fleet::new();
        let task = fleet.task(TaskStatus::NeedsDecision, MINUTE);
        fleet.note(
            task,
            NoteKind::Question,
            "Which PDF library?",
            serde_json::json!({ "options": ["printpdf", "typst", "wkhtmltopdf"] }),
            MINUTE,
        );
        let items = fleet.items();
        assert_eq!(items[0].question, "Which PDF library?");
        let actions: Vec<_> = items[0].actions.iter().map(|a| (a.id.as_str(), a.title.as_str())).collect();
        assert_eq!(actions, [("printpdf", "printpdf"), ("typst", "typst"), ("wkhtmltopdf", "wkhtmltopdf")]);
    }

    #[test]
    fn an_items_id_survives_its_rank_changing() {
        let mut fleet = Fleet::new();
        let pane = fleet.agent(None);
        fleet.hook_ask(pane, "Allow touch x", MINUTE);
        let task = fleet.task(TaskStatus::NeedsDecision, MINUTE);
        fleet.note(task, NoteKind::Question, "Which?", serde_json::json!({}), MINUTE);
        fleet.task(TaskStatus::InReview, MINUTE);
        let blocked = fleet.agent(None);
        fleet.observe(blocked, AgentActivity::Blocked, MINUTE);

        let before = assemble(&fleet.inputs, now());
        let after = assemble(&fleet.inputs, now() + Duration::from_secs(600));
        assert_eq!(before.len(), 4);
        for (b, a) in before.iter().zip(&after) {
            assert_eq!(b.id, a.id);
            assert_ne!(b.rank, a.rank, "{} did not age", b.id);
            assert!(a.rank < b.rank, "an older item sorts earlier");
        }
    }

    #[test]
    fn a_superseding_ask_changes_the_items_id() {
        let mut fleet = Fleet::new();
        let pane = fleet.agent(None);
        let first = fleet.hook_ask(pane, "Allow touch x", MINUTE);
        let before = fleet.items();
        let second = fleet.hook_ask(pane, "Allow touch y", 0);
        let after = fleet.items();
        assert_eq!(before[0].id, format!("ask:{first}"));
        assert_eq!(after[0].id, format!("ask:{second}"));
        assert_eq!(after[0].ask_id.as_deref(), Some(second.as_str()));
    }

    /// An open chat ask counts whatever its agent's activity says: a sibling
    /// subagent working folds the pane to Working while the ask still waits.
    /// What ends one is `open_permission`'s business (its `Resolved`, its turn
    /// ending, the shim starting over).
    #[test]
    fn a_chat_ask_counts_while_its_agent_works_on() {
        let mut fleet = Fleet::new();
        let pane = fleet.agent(None);
        let option = PermissionOption { id: "allow".into(), name: "Allow touch x".into(), kind: "allow_once".into() };
        fleet.inputs.permissions.insert(pane, ("chat-1".into(), vec![option], ago_time(MINUTE)));
        fleet.observe(pane, AgentActivity::Blocked, MINUTE);
        assert_eq!(kinds(&fleet.items()), [NeedsYouKind::Ask]);
        fleet.observe(pane, AgentActivity::Working, 0);
        assert_eq!(kinds(&fleet.items()), [NeedsYouKind::Ask], "a pending ask hid behind a working subagent");
    }

    #[test]
    fn a_read_scoped_item_carries_no_ask_command_path_or_actions() {
        let mut fleet = Fleet::new();
        let pane = fleet.agent(None);
        fleet.hook_ask(pane, "Allow /tmp/probe/x.txt", MINUTE);
        let control = fleet.items().remove(0);
        assert!(format!("{control:?}").contains("/tmp/probe"), "the probe never reached the item");
        let read = redact_below_control(control.clone());
        assert!(!format!("{read:?}").contains("/tmp/probe"), "{read:#?}");
        assert_eq!(read.question, "claude is asking to use a tool");
        assert_eq!((read.detail, read.ask_id, read.actions.len(), read.worktree), (None, None, 0, None));
        // And what a Read client is owed stays.
        assert_eq!((read.id, read.kind, read.rank), (control.id, control.kind, control.rank));
        assert_eq!(read.terminal, control.terminal);
        assert_eq!(read.workspace_name, "Billing");
    }

    #[test]
    fn ranks_share_the_terminal_scale() {
        use farcooler_core::feed::{rank as terminal_rank, AgentState, Subject as Rung};
        let rung = |state| Rung::Agent {
            name: "claude".into(),
            state,
            turn_elapsed: None,
            state_age: Duration::from_secs(90),
            question: None,
            signal: None,
        };
        let blocked = terminal_rank(&rung(AgentState::Blocked));
        let working = terminal_rank(&rung(AgentState::Working));
        // Tier 0 on the terminal scale is Blocked; tier 1 here is Blocked. One
        // tier apart, same age: the scale and the age rule are the same.
        assert_eq!(rank(NeedsYouKind::Ask, 0, 90_000), blocked);
        assert_eq!(rank(NeedsYouKind::Decision, 0, 90_000), working);
    }
}
