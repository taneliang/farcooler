//! Following the subagents a task's orchestrator recorded, by session.
//!
//! A subagent is recorded with the orchestrator's `session_id` and `cwd`
//! (`task worker` reads them from `CLAUDE_CODE_SESSION_ID` and `$PWD`), and
//! those name its files outright:
//!
//! - `~/.claude/projects/<slug of cwd>/<session>.jsonl`, where the spawn, the
//!   resume and the `<task-notification>` that says it stopped are written;
//! - `<session>/subagents/agent-<agentId>.jsonl`, where its own work is.
//!
//! This does not go through `log_join`, which finds the file of a PANE by its
//! cwd and title among every session in the project, and cannot find the
//! orchestrator's in a folder of hundreds. Nothing here needs a pane.
//!
//! Pure file reading, no clock and no store: the caller says which sessions
//! and agents it wants and what time it is, and gets back what was seen.
//! Time stamps on what was seen are the transcript's own, so a daemon that
//! was asleep for an hour reports the hour-old time a subagent last moved,
//! not the moment it woke.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use super::tail::Tail;
use super::usage::{LogUsage, LoggedTurn};
use super::{SubagentStatus, TurnEvent, claude, claude_slug};

/// A spawn this much older than the moment a session was first followed is
/// history, not news: it isn't linked from its description. Wide enough for
/// an orchestrator that launches three lanes and records the first.
const SPAWN_NEWS_MS: i64 = 2 * 60 * 1_000;
/// How much of the end of a session file is read when it is first followed,
/// to catch a stop or a resume the runner missed while it wasn't running.
/// `Tail` itself starts at the end of a file over a mebibyte.
const CATCH_UP_BYTES: u64 = 8 * 1024 * 1024;
/// How long to wait before looking again for a session file not found.
const LOOK_AGAIN_MS: i64 = 30_000;
/// Spawns remembered per session, for joining a launch result to the
/// description its call carried.
const REMEMBERED_SPAWNS: usize = 256;
/// Agents whose last stop is remembered per session.
const REMEMBERED_STOPS: usize = 4096;

/// One session to follow, and the subagents in it whose own files to read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wanted {
    pub session_id: String,
    /// Where the orchestrator was when it recorded the first one; names the
    /// project directory when that is where the session is.
    pub cwd: String,
    /// `agentId`s recorded in this session.
    pub agents: Vec<String>,
}

/// One thing seen in a followed session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seen {
    /// The session launched a subagent, new since it was first followed.
    Spawned { session: String, agent_id: String, description: String, at_ms: Option<i64> },
    /// A wanted subagent stopped.
    Ended { session: String, agent_id: String, status: SubagentStatus },
    /// A wanted subagent was sent a message and is working again.
    Resumed { session: String, agent_id: String },
    /// A wanted subagent's own transcript grew.
    Active {
        session: String,
        agent_id: String,
        /// The latest time any new line carries.
        at_ms: Option<i64>,
        /// The last tool call among them, as the verb and object it arrived as.
        doing: Option<(String, String)>,
        model: Option<String>,
    },
}

/// What one pass read.
#[derive(Debug, Default)]
pub struct Followed {
    pub seen: Vec<Seen>,
    /// Each subagent whose transcript grew, as it stands (`LogUsage::take`),
    /// with the session it's in. Keyed `claude-log:agent:<agentId>`.
    pub spend: Vec<(String, LoggedTurn)>,
    /// The sessions whose file was found: the ones the runner can say
    /// anything about. A wanted session not here is unobserved.
    pub located: Vec<String>,
}

/// Every session being followed.
pub struct WorkerFollow {
    home: PathBuf,
    sessions: BTreeMap<String, Session>,
}

struct Session {
    cwd: String,
    /// The session file, once found.
    file: Option<PathBuf>,
    looked_at_ms: Option<i64>,
    /// First seen at this time: what's older than it isn't news.
    attached_at_ms: i64,
    tail: Option<Tail>,
    /// The `tool_use` id of each spawn, to its description, newest last.
    spawns: VecDeque<(String, String)>,
    agents: BTreeMap<String, Tail>,
    usage: LogUsage,
    /// Each agent's last stop or resume in this session, wanted or not, so one
    /// recorded after it stopped (a foreground run writes its launch and its
    /// end in one line) is told where it stands the pass it becomes wanted.
    last_stop: HashMap<String, Seen>,
    /// The agents wanted on the last pass.
    known: Vec<String>,
}

impl WorkerFollow {
    /// Follow under `home` (`$HOME`, or a scratch directory for a test).
    pub fn new(home: PathBuf) -> WorkerFollow {
        WorkerFollow { home, sessions: BTreeMap::new() }
    }

    /// Read what each wanted session and subagent wrote since the last pass.
    /// A session no longer wanted is let go.
    pub fn follow(&mut self, wanted: &[Wanted], now_ms: i64) -> Followed {
        self.sessions.retain(|id, _| wanted.iter().any(|w| &w.session_id == id));
        let mut out = Followed::default();
        for want in wanted {
            let home = &self.home;
            let session = self.sessions.entry(want.session_id.clone()).or_insert_with(|| Session {
                cwd: want.cwd.clone(),
                file: None,
                looked_at_ms: None,
                attached_at_ms: now_ms,
                tail: None,
                spawns: VecDeque::new(),
                agents: BTreeMap::new(),
                usage: LogUsage::default(),
                last_stop: HashMap::new(),
                known: Vec::new(),
            });
            session.pass(home, want, now_ms, &mut out);
            if session.file.is_some() {
                out.located.push(want.session_id.clone());
            }
        }
        out
    }
}

impl Session {
    fn pass(&mut self, home: &Path, want: &Wanted, now_ms: i64, out: &mut Followed) {
        if self.file.is_none() && self.looked_at_ms.is_none_or(|at| now_ms - at >= LOOK_AGAIN_MS) {
            self.looked_at_ms = Some(now_ms);
            self.file = locate(home, &want.session_id, &self.cwd);
        }
        let Some(file) = self.file.clone() else { return };
        let first = self.tail.is_none();
        let mut lines = Vec::new();
        if first {
            let tail = Tail::wide(file.clone());
            lines = catch_up(&file);
            self.tail = Some(tail);
        }
        if let Some(tail) = self.tail.as_mut() {
            lines.extend(tail.read_new_lines());
        }
        self.read_session(&want.session_id, &lines, first, want, out);
        self.read_agents(&file, want, out);
    }

    fn read_session(&mut self, session: &str, lines: &[String], first: bool, want: &Wanted, out: &mut Followed) {
        let mut stops: Vec<Seen> = Vec::new();
        for line in lines {
            // Most lines of a session are neither a subagent's spawn, a
            // result nor a notification, and some are very large.
            if !["agentId", "resumedAgentId", "task-notification", "\"Agent\""].iter().any(|k| line.contains(k)) {
                continue;
            }
            let events = claude::parse_line(line);
            let is_result = events.iter().any(|e| matches!(e, TurnEvent::Answered { .. }));
            // A launch or a foreground end is believed only on the result of
            // an `Agent` call this follower saw made: any other tool result
            // with an `agentId` in it is not a subagent's. A notification or
            // a resume carries no such call and is read as it comes.
            let mut trusted = true;
            for event in events {
                match event {
                    TurnEvent::Subagent { id, description, .. } if !is_result => {
                        self.spawns.retain(|(known, _)| known != &id);
                        self.spawns.push_back((id, description));
                        if self.spawns.len() > REMEMBERED_SPAWNS {
                            self.spawns.pop_front();
                        }
                    }
                    TurnEvent::Subagent { id, description, .. } => {
                        trusted = self.spawns.iter().any(|(known, _)| known == &id);
                        if let (true, false, Some(spawn)) =
                            (trusted, description.is_empty(), self.spawns.iter_mut().find(|(known, _)| known == &id))
                        {
                            spawn.1 = description;
                        }
                    }
                    TurnEvent::SubagentLaunched { id, agent_id } if trusted => {
                        let at_ms = claude::timestamp_ms(line);
                        let news = at_ms.is_some_and(|at| at >= self.attached_at_ms - SPAWN_NEWS_MS);
                        let description = self
                            .spawns
                            .iter()
                            .find(|(known, d)| known == &id && !d.is_empty())
                            .map(|(_, d)| d.clone());
                        if let (true, Some(description)) = (news, description) {
                            out.seen.push(Seen::Spawned {
                                session: session.to_string(),
                                agent_id,
                                description,
                                at_ms,
                            });
                        }
                    }
                    TurnEvent::SubagentEnded { agent_id, status } if trusted => {
                        let seen = Seen::Ended { session: session.to_string(), agent_id: agent_id.clone(), status };
                        self.remember(&agent_id, &seen);
                        if want.agents.contains(&agent_id) {
                            push_stop(&mut stops, seen);
                        }
                    }
                    TurnEvent::SubagentResumed { agent_id } => {
                        let seen = Seen::Resumed { session: session.to_string(), agent_id: agent_id.clone() };
                        self.remember(&agent_id, &seen);
                        if want.agents.contains(&agent_id) {
                            push_stop(&mut stops, seen);
                        }
                    }
                    _ => {}
                }
            }
        }
        // An agent wanted only now: where it stood already.
        for agent in want.agents.iter().filter(|a| !self.known.contains(a)) {
            let said = stops.iter().any(|s| matches!(s, Seen::Ended { agent_id, .. } | Seen::Resumed { agent_id, .. } if agent_id == agent));
            if let (false, Some(seen)) = (said, self.last_stop.get(agent)) {
                stops.push(seen.clone());
            }
        }
        self.known = want.agents.clone();
        if first {
            // The first read is everything the file holds, or its last
            // mebibytes: a subagent that stopped, was resumed and stopped
            // again is one that stopped. Only where it stands now.
            let mut last: HashMap<String, Seen> = HashMap::new();
            for seen in stops.drain(..) {
                let agent = match &seen {
                    Seen::Ended { agent_id, .. } | Seen::Resumed { agent_id, .. } => agent_id.clone(),
                    _ => continue,
                };
                last.insert(agent, seen);
            }
            stops = last.into_values().collect();
        }
        out.seen.extend(stops);
    }

    fn remember(&mut self, agent: &str, seen: &Seen) {
        if self.last_stop.len() >= REMEMBERED_STOPS && !self.last_stop.contains_key(agent) {
            self.last_stop.clear();
        }
        self.last_stop.insert(agent.to_string(), seen.clone());
    }

    fn read_agents(&mut self, file: &Path, want: &Wanted, out: &mut Followed) {
        self.agents.retain(|id, _| want.agents.contains(id));
        let dir = file.with_extension("").join("subagents");
        for agent in &want.agents {
            let tail = self
                .agents
                .entry(agent.clone())
                .or_insert_with(|| Tail::wide(dir.join(format!("agent-{agent}.jsonl"))));
            let lines = tail.read_new_lines();
            if lines.is_empty() {
                continue;
            }
            let mut at_ms: Option<i64> = None;
            let mut doing = None;
            let mut model = None;
            for line in &lines {
                self.usage.subagent_line(agent, line);
                at_ms = at_ms.max(claude::timestamp_ms(line));
                model = claude::model_of(line).or(model);
                for event in claude::parse_line(line) {
                    if let TurnEvent::Did { verb, object } = event {
                        doing = Some((verb, object));
                    }
                }
            }
            out.seen.push(Seen::Active {
                session: want.session_id.clone(),
                agent_id: agent.clone(),
                at_ms,
                doing,
                model,
            });
        }
        let session = want.session_id.clone();
        out.spend.extend(self.usage.take().into_iter().map(|turn| (session.clone(), turn)));
    }
}

/// A stop or a resume, unless it repeats the last one said about the same
/// agent: a notification is written three times.
fn push_stop(stops: &mut Vec<Seen>, seen: Seen) {
    let agent = |s: &Seen| match s {
        Seen::Ended { agent_id, .. } | Seen::Resumed { agent_id, .. } => Some(agent_id.clone()),
        _ => None,
    };
    let latest = stops.iter().rev().find(|s| agent(s) == agent(&seen));
    if latest != Some(&seen) {
        stops.push(seen);
    }
}

/// The last `CATCH_UP_BYTES` of a file, as whole lines, when it's larger than
/// `Tail` reads from the start; nothing for a small file, which `Tail` reads
/// whole itself.
fn catch_up(file: &Path) -> Vec<String> {
    let Ok(mut open) = std::fs::File::open(file) else { return Vec::new() };
    let Ok(len) = open.metadata().map(|m| m.len()) else { return Vec::new() };
    if len <= 1024 * 1024 {
        return Vec::new();
    }
    let from = len.saturating_sub(CATCH_UP_BYTES);
    let mut bytes = Vec::new();
    if open.seek(SeekFrom::Start(from)).is_err() || open.take(len - from).read_to_end(&mut bytes).is_err() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut lines = text.lines();
    // Where the read began is probably mid-line.
    if from > 0 {
        lines.next();
    }
    lines.map(str::to_string).collect()
}

/// The session's file: under the project its `cwd` slugs to (as given, then
/// resolved), else under whichever project holds a file of that name. A
/// session keeps the project it began in, and `$PWD` is wherever the
/// orchestrator had moved to when it recorded the subagent.
fn locate(home: &Path, session: &str, cwd: &str) -> Option<PathBuf> {
    let root = home.join(".claude/projects");
    let name = format!("{session}.jsonl");
    let resolved = std::fs::canonicalize(cwd).map(|p| p.to_string_lossy().into_owned());
    let direct = std::iter::once(cwd.to_string()).chain(resolved).map(|dir| root.join(claude_slug(&dir)).join(&name));
    if let Some(found) = direct.into_iter().find(|p| p.is_file()) {
        return Some(found);
    }
    std::fs::read_dir(&root)
        .ok()?
        .flatten()
        .map(|project| project.path().join(&name))
        .find(|p| p.is_file())
}
