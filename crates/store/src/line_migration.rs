//! The one-way move from lanes and trains to agents, the line and landings
//! (ov-872, slice 2).
//!
//! # Not registered yet
//!
//! `MIGRATIONS` doesn't list it. Running it drops the tables that `plan.rs`,
//! `trains.rs`, `open_train.rs`, `train_phase.rs` and `workers.rs` read and
//! write, and the daemon still calls them until slice 3 moves it onto
//! `agents.rs`, `ticket_lines.rs`, `ticket_work.rs` and `landings.rs`. So slice
//! 3 appends `LINE_MIGRATION` to `MIGRATIONS` in the same change that switches
//! the daemon, and deletes this note. Registered before then, the runner
//! would migrate, and every plan read would fail on a missing table.
//!
//! # What it does
//!
//! `Older::Refused`, by the owner's choice (Oct 31): a build from before it
//! reads tables this drops.
//!
//! - **Tickets.** Each card a lane works becomes one row on the line, keyed by
//!   the card. A card in several lanes takes the lane it is still open in, or
//!   else the one that moved last. The tickets of a lane of several cards keep
//!   that lane's id in `same_branch`, so they still land together. A lane's
//!   stage word maps over (`stage_of`), its branch, worktree, reason, landed
//!   SHA and hold carry over, and the plan's lane order becomes a ticket order
//!   numbered 1, 2, 3 on each board. A lane with no card has no ticket.
//! - **Agents.** One row per agent: each lane agent, train agent and worker
//!   record, merged by board, harness and id, plus each orchestrator pane. A
//!   `pane:<terminal>` id is a pane; any other is a subagent.
//! - **Who worked what.** Each lane agent works each of its lane's cards in its
//!   role (build, review, fix); each worker record its card; a train's agent
//!   lands the train's card.
//! - **Landings.** Each train becomes a landing with the same id, on the push
//!   route, numbered from its name (`train-72` is 72), or after the board's
//!   last when its name has no number. Its tickets are the cards of the lanes
//!   labelled with it or put on it and never taken off.
//! - **History.** `plan_events` and `train_events` become `line_events`: a
//!   theme's as they were, a lane's once for each of its cards, a train's on
//!   its landing.
//! - **Budgets.** A lane's budget goes on each of its tickets.

use std::collections::{BTreeMap, HashMap};

use rusqlite::{Transaction, params};
use uuid::Uuid;

use crate::migrate::Older;
use crate::tasks::now_millis;

type Id = Vec<u8>;

/// What slice 3 appends to `MIGRATIONS`.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const LINE_MIGRATION: (fn(&Transaction) -> rusqlite::Result<()>, Older) =
    (migration_0067_agents_and_the_line, Older::Refused);

const TABLES: &str = r#"
    CREATE TABLE agents (
        id BLOB PRIMARY KEY NOT NULL,
        workspace_id BLOB NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
        role TEXT NOT NULL CHECK (role IN ('orchestrator', 'builder', 'reviewer', 'lander')),
        kind TEXT NOT NULL CHECK (kind IN ('pane', 'subagent')),
        harness TEXT NOT NULL,
        -- A pane's terminal. No key into terminals: an agent is history, and
        -- outlives its pane.
        terminal_id BLOB,
        -- A subagent's own id (Claude's agentId, codex's agent path), and the
        -- pane it runs under.
        subagent_id TEXT,
        parent_terminal_id BLOB,
        session_id TEXT,
        session_cwd TEXT,
        model TEXT NOT NULL DEFAULT '',
        title TEXT NOT NULL DEFAULT '',
        started_at INTEGER NOT NULL,
        ended_at INTEGER,
        end_reason TEXT,
        stop_asked_by TEXT,
        stop_asked_at INTEGER,
        CHECK ((kind = 'pane') = (terminal_id IS NOT NULL)),
        CHECK ((kind = 'subagent') = (subagent_id IS NOT NULL))
    );
    CREATE UNIQUE INDEX agents_by_subagent ON agents (workspace_id, harness, subagent_id) WHERE subagent_id IS NOT NULL;
    CREATE INDEX agents_by_terminal ON agents (terminal_id) WHERE terminal_id IS NOT NULL;
    CREATE INDEX agents_open ON agents (workspace_id) WHERE ended_at IS NULL;

    -- The line: one row per ticket being worked. Its board is its task's.
    CREATE TABLE ticket_lines (
        task_id BLOB PRIMARY KEY NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
        stage TEXT NOT NULL,
        reason TEXT NOT NULL DEFAULT '',
        branch TEXT NOT NULL DEFAULT '',
        worktree_id BLOB REFERENCES worktrees(id) ON DELETE SET NULL,
        worktree_path TEXT NOT NULL DEFAULT '',
        stack_parent BLOB REFERENCES ticket_lines(task_id) ON DELETE SET NULL,
        -- The lane a carried-over ticket shares a branch from; only the
        -- migration writes it.
        same_branch BLOB,
        pr_number INTEGER,
        landed_sha TEXT,
        -- Set only on a waiting ticket.
        plan_rank INTEGER,
        stage_since INTEGER NOT NULL,
        held_off INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER NOT NULL,
        resource_version INTEGER NOT NULL
    );
    CREATE INDEX ticket_lines_by_group ON ticket_lines (same_branch) WHERE same_branch IS NOT NULL;
    CREATE INDEX ticket_lines_by_parent ON ticket_lines (stack_parent) WHERE stack_parent IS NOT NULL;

    CREATE TABLE ticket_work (
        agent_id BLOB NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
        task_id BLOB NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
        role TEXT NOT NULL CHECK (role IN ('build', 'fix', 'review', 'land')),
        started_at INTEGER NOT NULL,
        ended_at INTEGER,
        verdict TEXT,
        linked_by TEXT NOT NULL DEFAULT 'orchestrator',
        PRIMARY KEY (agent_id, task_id, role)
    );
    CREATE INDEX ticket_work_by_task ON ticket_work (task_id);

    CREATE TABLE landings (
        id BLOB PRIMARY KEY NOT NULL,
        workspace_id BLOB NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
        number INTEGER NOT NULL,
        route TEXT NOT NULL,
        base TEXT NOT NULL DEFAULT '',
        pushed_sha TEXT,
        state TEXT NOT NULL,
        state_since INTEGER NOT NULL,
        lander_id BLOB REFERENCES agents(id) ON DELETE SET NULL,
        title TEXT NOT NULL DEFAULT '',
        collecting INTEGER NOT NULL DEFAULT 0,
        actor TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        landed_at INTEGER,
        resource_version INTEGER NOT NULL,
        UNIQUE (workspace_id, number)
    );
    CREATE UNIQUE INDEX landings_collecting ON landings (workspace_id)
        WHERE collecting = 1 AND state NOT IN ('landed', 'dropped');

    CREATE TABLE landing_tickets (
        landing_id BLOB NOT NULL REFERENCES landings(id) ON DELETE CASCADE,
        task_id BLOB NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
        added_at INTEGER NOT NULL,
        PRIMARY KEY (landing_id, task_id)
    );
    CREATE INDEX landing_tickets_by_task ON landing_tickets (task_id);

    -- A theme's, a ticket's or a landing's history. A ticket's move on a
    -- landing names both.
    CREATE TABLE line_events (
        id BLOB PRIMARY KEY NOT NULL,
        at INTEGER NOT NULL,
        actor TEXT NOT NULL,
        theme_id BLOB REFERENCES board_themes(id) ON DELETE CASCADE,
        task_id BLOB REFERENCES tasks(id) ON DELETE CASCADE,
        landing_id BLOB REFERENCES landings(id) ON DELETE CASCADE,
        kind TEXT NOT NULL,
        body TEXT NOT NULL DEFAULT '',
        extra TEXT NOT NULL DEFAULT '{}',
        CHECK ((theme_id IS NULL) <> (task_id IS NULL AND landing_id IS NULL))
    );
    CREATE INDEX line_events_by_theme ON line_events (theme_id, at) WHERE theme_id IS NOT NULL;
    CREATE INDEX line_events_by_task ON line_events (task_id, at) WHERE task_id IS NOT NULL;
    CREATE INDEX line_events_by_landing ON line_events (landing_id, at) WHERE landing_id IS NOT NULL;
"#;

/// A lane, as far as the move needs it.
struct OldLane {
    id: Id,
    workspace: Id,
    name: String,
    state: String,
    reason: String,
    plan_rank: Option<i64>,
    worktree_id: Option<Id>,
    worktree_path: String,
    branch: String,
    landed_sha: Option<String>,
    state_since: i64,
    created_at: i64,
    held_off: i64,
}

impl OldLane {
    fn is_open(&self) -> bool {
        !matches!(self.state.as_str(), "landed" | "dropped")
    }
}

/// A lane's state word as a ticket's stage. A fix round is building again; the
/// fix shows in who works the ticket, as `fix`.
fn stage_of(lane_state: &str) -> &'static str {
    match lane_state {
        "building" | "fixing" => "building",
        "review" => "reviewing",
        "landing" => "landing",
        "landed" => "landed",
        "dropped" => "dropped",
        _ => "waiting",
    }
}

/// The number in a train's name, `train-72` or `integ-72`; `None` for any
/// other name. A copy of `trains::train_number`, so the migration outlives it.
fn train_number(name: &str) -> Option<u32> {
    let lower = name.trim().to_ascii_lowercase();
    let digits = lower.strip_prefix("train-").or_else(|| lower.strip_prefix("integ-"))?;
    if digits.is_empty() || digits.len() > 9 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// A train's state word as a landing's.
fn landing_state_of(train_state: &str) -> &str {
    match train_state {
        "integrating" => "open",
        other => other,
    }
}

/// One agent being gathered from the old records.
#[derive(Default)]
struct Gathered {
    harness: String,
    key: String,
    /// The earliest lane role, with when it started.
    lane_role: Option<(i64, String)>,
    orchestrator: bool,
    lander: bool,
    session_id: Option<String>,
    session_cwd: Option<String>,
    parent: Option<Id>,
    model: String,
    title: String,
    started: Option<i64>,
    ended: Option<i64>,
    open: bool,
    end_reason: Option<String>,
}

impl Gathered {
    fn saw(&mut self, started: i64, ended: Option<i64>, model: &str) {
        self.started = Some(self.started.map_or(started, |s| s.min(started)));
        match ended {
            None => self.open = true,
            Some(e) => self.ended = Some(self.ended.map_or(e, |x| x.max(e))),
        }
        if !model.is_empty() {
            self.model = model.to_string();
        }
    }

    fn role(&self) -> &'static str {
        if self.orchestrator {
            return "orchestrator";
        }
        match self.lane_role.as_ref().map(|(_, r)| r.as_str()) {
            Some("review") => "reviewer",
            Some(_) => "builder",
            None if self.lander => "lander",
            None => "builder",
        }
    }
}

type AgentKey = (Id, String, String);

fn gather<'a>(agents: &'a mut BTreeMap<AgentKey, Gathered>, workspace: &Id, harness: &str, key: &str) -> &'a mut Gathered {
    agents.entry((workspace.clone(), harness.to_string(), key.to_string())).or_insert_with(|| Gathered {
        harness: harness.to_string(),
        key: key.to_string(),
        ..Default::default()
    })
}

/// Agents, the line, who worked what, landings and their history, out of
/// lanes, trains and worker records; then the old tables go. See the module.
///
/// The old rows are read as tuples, by position, each once and next to its
/// query.
#[allow(clippy::type_complexity)]
pub(crate) fn migration_0067_agents_and_the_line(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(TABLES)?;
    let now = now_millis();

    // Lanes, and the cards each works that are still on its board, oldest
    // card first. Slices of one card are one ticket.
    let lanes: Vec<OldLane> = {
        let mut stmt = tx.prepare(
            "SELECT id, workspace_id, name, state, reason, plan_rank, worktree_id, worktree_path, branch, landed_sha,
                    state_since, created_at, held_off
               FROM lanes ORDER BY created_at, id",
        )?;
        stmt.query_map([], |r| {
            Ok(OldLane {
                id: r.get(0)?,
                workspace: r.get(1)?,
                name: r.get(2)?,
                state: r.get(3)?,
                reason: r.get(4)?,
                plan_rank: r.get(5)?,
                worktree_id: r.get(6)?,
                worktree_path: r.get(7)?,
                branch: r.get(8)?,
                landed_sha: r.get(9)?,
                state_since: r.get(10)?,
                created_at: r.get(11)?,
                held_off: r.get(12)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?
    };
    let lane_by_id: HashMap<&Id, &OldLane> = lanes.iter().map(|l| (&l.id, l)).collect();
    tx.execute_batch(
        "CREATE TEMP TABLE lane_cards AS
           SELECT DISTINCT lt.lane_id, lt.task_id, t.created_at AS task_created
             FROM lane_tasks lt JOIN lanes l ON l.id = lt.lane_id
             JOIN tasks t ON t.id = lt.task_id AND t.workspace_id = l.workspace_id;
         CREATE TEMP TABLE ticket_lane (task_id BLOB PRIMARY KEY, lane_id BLOB NOT NULL);",
    )?;
    let cards: Vec<(Id, Id)> = {
        let mut stmt = tx.prepare("SELECT lane_id, task_id FROM lane_cards ORDER BY task_created, task_id")?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?
    };

    // Each card's lane: the one it is open in, else the one that moved last.
    let mut chosen: BTreeMap<Id, &OldLane> = BTreeMap::new();
    let mut card_order: Vec<Id> = Vec::new();
    for (lane, task) in &cards {
        let lane = lane_by_id[lane];
        let rank = |l: &OldLane| (l.is_open(), l.state_since, l.created_at, l.id.clone());
        match chosen.get(task) {
            Some(held) if rank(held) >= rank(lane) => {}
            Some(_) => {
                chosen.insert(task.clone(), lane);
            }
            None => {
                card_order.push(task.clone());
                chosen.insert(task.clone(), lane);
            }
        }
    }
    let mut tickets_of_lane: HashMap<&Id, usize> = HashMap::new();
    for lane in chosen.values() {
        *tickets_of_lane.entry(&lane.id).or_default() += 1;
    }

    // The plan: queued lanes in their order, each lane's cards oldest first,
    // numbered from 1 on each board.
    let mut planned: Vec<(i64, usize, &Id)> = card_order
        .iter()
        .enumerate()
        .filter_map(|(i, task)| {
            let lane = chosen[task];
            (stage_of(&lane.state) == "waiting").then_some(())?;
            lane.plan_rank.map(|rank| (rank, i, task))
        })
        .collect();
    planned.sort();
    let mut next_rank: HashMap<&Id, i64> = HashMap::new();
    let mut rank_of: HashMap<&Id, i64> = HashMap::new();
    for (_, _, task) in planned {
        let n = next_rank.entry(&chosen[task].workspace).or_insert(0);
        *n += 1;
        rank_of.insert(task, *n);
    }

    for task in &card_order {
        let lane = chosen[task];
        let shared = tickets_of_lane[&lane.id] > 1;
        tx.execute(
            "INSERT INTO ticket_lines (task_id, stage, reason, branch, worktree_id, worktree_path, same_branch, landed_sha,
                                       plan_rank, stage_since, held_off, created_at, resource_version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 1)",
            params![
                task,
                stage_of(&lane.state),
                lane.reason,
                lane.branch,
                lane.worktree_id,
                lane.worktree_path,
                shared.then_some(&lane.id),
                lane.landed_sha,
                rank_of.get(task),
                lane.state_since,
                lane.held_off,
                lane.created_at,
            ],
        )?;
        tx.execute("INSERT INTO ticket_lane (task_id, lane_id) VALUES (?1, ?2)", params![task, lane.id])?;
        tx.execute(
            "INSERT INTO line_events (id, at, actor, task_id, kind, body, extra)
             VALUES (?1, ?2, 'runner', ?3, 'moved', ?4, json_object('lane', ?5))",
            params![Uuid::now_v7().as_bytes().to_vec(), now, task, format!("Carried over from lane {}.", lane.name), lane.name],
        )?;
    }

    // Agents, gathered from every record that names one.
    let mut agents: BTreeMap<AgentKey, Gathered> = BTreeMap::new();
    {
        let mut stmt = tx.prepare("SELECT id, workspace_id, command_preset FROM terminals WHERE role = 2 AND workspace_id IS NOT NULL")?;
        let rows: Vec<(Id, Id, String)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
        for (terminal, workspace, preset) in rows {
            let Ok(terminal) = Uuid::from_slice(&terminal) else { continue };
            let g = gather(&mut agents, &workspace, &preset, &format!("pane:{terminal}"));
            g.orchestrator = true;
            g.saw(now, None, "");
        }
    }
    let lane_agents: Vec<(Id, Id, String, String, String, String, i64, Option<i64>)> = {
        let mut stmt = tx.prepare(
            "SELECT a.lane_id, l.workspace_id, a.harness, a.agent_id, a.role, a.model, a.started_at, a.ended_at
               FROM lane_agents a JOIN lanes l ON l.id = a.lane_id ORDER BY a.started_at",
        )?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?)))?
            .collect::<rusqlite::Result<_>>()?
    };
    for (_, workspace, harness, key, role, model, started, ended) in &lane_agents {
        let g = gather(&mut agents, workspace, harness, key);
        if g.lane_role.as_ref().is_none_or(|(at, _)| started < at) {
            g.lane_role = Some((*started, role.clone()));
        }
        g.saw(*started, *ended, model);
    }
    let train_agents: Vec<(Id, Id, String, String, String, i64, Option<i64>, Option<Id>)> = {
        let mut stmt = tx.prepare(
            "SELECT id, workspace_id, agent_harness, agent_id, agent_model, coalesce(agent_started_at, created_at),
                    agent_ended_at, card_id
               FROM board_trains WHERE agent_id <> ''",
        )?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?)))?
            .collect::<rusqlite::Result<_>>()?
    };
    for (_, workspace, harness, key, model, started, ended, _) in &train_agents {
        let g = gather(&mut agents, workspace, harness, key);
        g.lander = true;
        g.saw(*started, *ended, model);
    }
    let workers: Vec<(Id, Id, String, String, Option<String>, Option<String>, Option<Id>, String, String, i64, Option<i64>, Option<String>, String)> = {
        let mut stmt = tx.prepare(
            "SELECT w.task_id, t.workspace_id, w.harness, w.agent_id, w.session_id, w.session_cwd, w.orchestrator_terminal,
                    coalesce(nullif(w.label, ''), d.description, ''), w.model, w.started_at, w.ended_at, w.end_reason,
                    w.linked_by
               FROM task_workers w JOIN tasks t ON t.id = w.task_id
               LEFT JOIN subagent_descriptions d ON d.agent_id = w.agent_id
              ORDER BY w.started_at",
        )?;
        stmt.query_map([], |r| {
            Ok((
                r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?,
                r.get(9)?, r.get(10)?, r.get(11)?, r.get(12)?,
            ))
        })?
        .collect::<rusqlite::Result<_>>()?
    };
    for (_, workspace, harness, key, session, cwd, parent, label, model, started, ended, reason, _) in &workers {
        let g = gather(&mut agents, workspace, harness, key);
        g.saw(*started, *ended, model);
        g.session_id = session.clone().or(g.session_id.take());
        g.session_cwd = cwd.clone().or(g.session_cwd.take());
        g.parent = parent.clone().or(g.parent.take());
        if !label.is_empty() {
            g.title = label.clone();
        }
        if reason.is_some() {
            g.end_reason = reason.clone();
        }
    }
    // One row each. A subagent no worker record titled takes what its parent
    // called it.
    let mut ids: HashMap<AgentKey, Id> = HashMap::new();
    for ((workspace, _, _), g) in &agents {
        let id = Uuid::now_v7().as_bytes().to_vec();
        let pane = g.key.strip_prefix("pane:").and_then(|t| Uuid::parse_str(t).ok());
        let (kind, terminal, subagent) = match pane {
            Some(t) => ("pane", Some(t.as_bytes().to_vec()), None),
            None => ("subagent", None, Some(g.key.clone())),
        };
        let title = if g.title.is_empty() && subagent.is_some() {
            tx.query_row("SELECT description FROM subagent_descriptions WHERE agent_id = ?1", params![g.key], |r| r.get(0))
                .unwrap_or_default()
        } else {
            g.title.clone()
        };
        tx.execute(
            "INSERT INTO agents (id, workspace_id, role, kind, harness, terminal_id, subagent_id, parent_terminal_id,
                                 session_id, session_cwd, model, title, started_at, ended_at, end_reason)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                id,
                workspace,
                g.role(),
                kind,
                g.harness,
                terminal,
                subagent,
                g.parent,
                g.session_id,
                g.session_cwd,
                g.model,
                title,
                g.started.unwrap_or(now),
                if g.open { None } else { g.ended },
                if g.open { None } else { g.end_reason.clone() },
            ],
        )?;
        ids.insert((workspace.clone(), g.harness.clone(), g.key.clone()), id);
    }
    let id_of = |workspace: &Id, harness: &str, key: &str| ids[&(workspace.clone(), harness.to_string(), key.to_string())].clone();

    // Who worked what. The same agent, ticket and role from two records is one
    // row: the earliest start, open if either is.
    let work = |agent: &Id, task: &Id, role: &str, started: i64, ended: Option<i64>, verdict: Option<&str>, linked: &str| {
        tx.execute(
            "INSERT INTO ticket_work (agent_id, task_id, role, started_at, ended_at, verdict, linked_by)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(agent_id, task_id, role) DO UPDATE SET
                started_at = min(started_at, excluded.started_at),
                ended_at = CASE WHEN ended_at IS NULL OR excluded.ended_at IS NULL THEN NULL
                                ELSE max(ended_at, excluded.ended_at) END,
                verdict = coalesce(excluded.verdict, verdict)",
            params![agent, task, role, started, ended, verdict, linked],
        )
    };
    let mut cards_of_lane: HashMap<&Id, Vec<&Id>> = HashMap::new();
    for (lane, task) in &cards {
        cards_of_lane.entry(lane).or_default().push(task);
    }
    for (lane, workspace, harness, key, role, _, started, ended) in &lane_agents {
        let agent = id_of(workspace, harness, key);
        let role = if matches!(role.as_str(), "build" | "fix" | "review") { role.as_str() } else { "build" };
        for task in cards_of_lane.get(lane).into_iter().flatten() {
            work(&agent, task, role, *started, *ended, None, "orchestrator")?;
        }
    }
    for (_, workspace, harness, key, _, started, ended, card) in &train_agents {
        if let Some(card) = card {
            work(&id_of(workspace, harness, key), card, "land", *started, *ended, None, "orchestrator")?;
        }
    }
    for (task, workspace, harness, key, _, _, _, _, _, started, ended, reason, linked) in &workers {
        let agent_key = (workspace.clone(), harness.clone(), key.clone());
        let role = match agents[&agent_key].role() {
            "reviewer" => "review",
            "lander" => "land",
            _ => "build",
        };
        let linked = if linked == "description" { "description" } else { "orchestrator" };
        work(&ids[&agent_key], task, role, *started, *ended, reason.as_deref(), linked)?;
    }

    // Landings, one per train and with its id, numbered from its name.
    let trains: Vec<(Id, Id, String, String, Option<String>, String, i64, String, i64, Option<i64>, i64, String, i64, String, String)> = {
        let mut stmt = tx.prepare(
            "SELECT id, workspace_id, name, base, pushed_sha, state, state_since, actor, created_at, landed_at,
                    resource_version, title, collecting, agent_harness, agent_id
               FROM board_trains ORDER BY workspace_id, created_at, id",
        )?;
        stmt.query_map([], |r| {
            Ok((
                r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?,
                r.get(9)?, r.get(10)?, r.get(11)?, r.get(12)?, r.get(13)?, r.get(14)?,
            ))
        })?
        .collect::<rusqlite::Result<_>>()?
    };
    let mut taken: HashMap<Id, std::collections::HashSet<u32>> = HashMap::new();
    let mut number_of: HashMap<Id, u32> = HashMap::new();
    for (id, workspace, name, ..) in &trains {
        if let Some(n) = train_number(name)
            && taken.entry(workspace.clone()).or_default().insert(n)
        {
            number_of.insert(id.clone(), n);
        }
    }
    let mut collecting_seen: std::collections::HashSet<Id> = std::collections::HashSet::new();
    for (id, workspace, name, base, sha, state, since, actor, created, landed, version, title, collecting, harness, agent) in &trains {
        let numbered = number_of.get(id).copied();
        let number = numbered.unwrap_or_else(|| {
            let used = taken.entry(workspace.clone()).or_default();
            let n = used.iter().max().copied().unwrap_or(0) + 1;
            used.insert(n);
            n
        });
        let title = if title.is_empty() && numbered.is_none() { name.clone() } else { title.clone() };
        let state = landing_state_of(state);
        let settled = matches!(state, "landed" | "dropped");
        let collecting = *collecting != 0 && !settled && collecting_seen.insert(workspace.clone());
        let lander = (!agent.is_empty()).then(|| id_of(workspace, harness, agent));
        tx.execute(
            "INSERT INTO landings (id, workspace_id, number, route, base, pushed_sha, state, state_since, lander_id, title,
                                   collecting, actor, created_at, landed_at, resource_version)
             VALUES (?1, ?2, ?3, 'push', ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![id, workspace, number, base, sha, state, since, lander, title, collecting, actor, created, landed, version],
        )?;
    }
    // A landing's tickets: the cards of each lane labelled with its train, or
    // put on it and not taken off since.
    tx.execute_batch(
        "INSERT OR IGNORE INTO landing_tickets (landing_id, task_id, added_at)
           SELECT t.id, c.task_id, t.created_at
             FROM lanes l JOIN board_trains t ON t.workspace_id = l.workspace_id AND l.train = t.name COLLATE NOCASE
             JOIN lane_cards c ON c.lane_id = l.id;
         INSERT OR IGNORE INTO landing_tickets (landing_id, task_id, added_at)
           SELECT e.train_id, c.task_id, e.at
             FROM train_events e JOIN lane_cards c ON c.lane_id = e.lane_id
            WHERE e.kind = 'lane_on'
              AND NOT EXISTS (SELECT 1 FROM train_events o WHERE o.train_id = e.train_id AND o.lane_id = e.lane_id
                                 AND o.kind = 'lane_off' AND o.at >= e.at);",
    )?;

    // History. A lane's entries go to each of its cards, naming the lane.
    tx.execute_batch(
        "INSERT INTO line_events (id, at, actor, theme_id, kind, body, extra)
           SELECT id, at, actor, theme_id, kind, body, extra FROM plan_events WHERE theme_id IS NOT NULL;
         INSERT INTO line_events (id, at, actor, task_id, kind, body, extra)
           SELECT randomblob(16), e.at, e.actor, c.task_id, e.kind, e.body,
                  CASE WHEN json_valid(e.extra) THEN json_set(e.extra, '$.lane', l.name) ELSE json_object('lane', l.name) END
             FROM plan_events e JOIN lanes l ON l.id = e.lane_id JOIN lane_cards c ON c.lane_id = e.lane_id;
         INSERT INTO line_events (id, at, actor, landing_id, kind, body, extra)
           SELECT id, at, actor, train_id, kind, '', extra FROM train_events
            WHERE kind NOT IN ('lane_on', 'lane_off') OR lane_id IS NULL;
         INSERT INTO line_events (id, at, actor, task_id, landing_id, kind, body, extra)
           SELECT randomblob(16), e.at, e.actor, c.task_id, e.train_id,
                  CASE e.kind WHEN 'lane_on' THEN 'ticket_on' ELSE 'ticket_off' END, '',
                  CASE WHEN json_valid(e.extra) THEN json_set(e.extra, '$.lane', l.name) ELSE json_object('lane', l.name) END
             FROM train_events e JOIN lane_cards c ON c.lane_id = e.lane_id JOIN lanes l ON l.id = e.lane_id
            WHERE e.kind IN ('lane_on', 'lane_off');",
    )?;

    // Budgets: a lane's goes on each of its tickets.
    tx.execute_batch(
        "CREATE TABLE plan_budgets_by_ticket (
             id BLOB PRIMARY KEY NOT NULL,
             workspace_id BLOB NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
             theme_id BLOB REFERENCES board_themes(id) ON DELETE CASCADE,
             task_id BLOB REFERENCES tasks(id) ON DELETE CASCADE,
             tokens INTEGER NOT NULL CHECK (tokens > 0),
             set_at INTEGER NOT NULL,
             CHECK ((theme_id IS NULL) <> (task_id IS NULL))
         );
         INSERT INTO plan_budgets_by_ticket (id, workspace_id, theme_id, tokens, set_at)
           SELECT id, workspace_id, theme_id, tokens, set_at FROM plan_budgets WHERE theme_id IS NOT NULL;
         INSERT INTO plan_budgets_by_ticket (id, workspace_id, task_id, tokens, set_at)
           SELECT randomblob(16), b.workspace_id, tl.task_id, b.tokens, b.set_at
             FROM plan_budgets b JOIN ticket_lane tl ON tl.lane_id = b.lane_id;
         DROP TABLE plan_budgets;
         ALTER TABLE plan_budgets_by_ticket RENAME TO plan_budgets;
         CREATE UNIQUE INDEX plan_budgets_by_theme ON plan_budgets (theme_id) WHERE theme_id IS NOT NULL;
         CREATE UNIQUE INDEX plan_budgets_by_task ON plan_budgets (task_id) WHERE task_id IS NOT NULL;",
    )?;

    // The old tables, children first.
    tx.execute_batch(
        "DROP TABLE train_events;
         DROP TABLE plan_events;
         DROP TABLE lane_agents;
         DROP TABLE lane_tasks;
         DROP TABLE lanes;
         DROP TABLE board_trains;
         DROP TABLE task_workers;
         DROP TABLE temp.lane_cards;
         DROP TABLE temp.ticket_lane;",
    )
}

#[cfg(test)]
#[path = "line_migration_tests.rs"]
mod tests;

/// A fresh store with the line's tables, as slice 3's registration will leave
/// one: what the tests of the line's own files open.
#[cfg(test)]
pub(crate) fn store_with_the_line() -> crate::Store {
    let store = crate::Store::open_in_memory().unwrap();
    apply(&store);
    store
}

/// Run the move on `store`, as `MIGRATIONS` will once it is registered.
#[cfg(test)]
pub(crate) fn apply(store: &crate::Store) {
    let conn = store.conn();
    let tx = conn.unchecked_transaction().unwrap();
    (LINE_MIGRATION.0)(&tx).unwrap();
    tx.commit().unwrap();
}
