//! Whose a Needs You item is (ov-633, `.claude/agent/reports/ov-633/design.md`).
//!
//! On a runner with no members nothing here runs: no item carries a concern,
//! and every push goes to the owner as it always has. Once there is a member,
//! each item concerns one person, or nobody:
//!
//! - an ask or a block on a rep: its member;
//! - on an orchestrator: its lead, who with the owner is the only one `gate`
//!   lets answer it (a member, or the owner while the owner leads);
//! - on any other pane: the member whose login it was launched on
//!   (`terminal_logins`, so an orchestrator's lanes are its lead's); no row is
//!   the runner's own login, the owner's; a removed member's login is nobody's;
//! - a task's item with no agent on it: the member whose desk it is, else
//!   nobody's;
//! - a pull request review request: the owner's, whose `gh` login it asks;
//! - a member's login: that member's (`login`).
//!
//! **Order, grouping and pushes, never visibility.** Every reader gets every
//! item. Theirs and nobody's come first, then everyone else's; and a push
//! about a member's item goes to that member, with the owner's copy moving
//! the card silently (`push_to`).

use std::collections::HashMap;

use farcooler_protocol::v1 as pb;
use farcooler_store::models::{self, TerminalRole};
use uuid::Uuid;

use crate::people::Person;

/// The one person an item concerns.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Whose {
    /// The runner's owner.
    Owner,
    /// A member, by handle.
    Member(String),
}

/// What decides one pane's person, read from the store (`pane_facts`) or
/// built by hand in a test.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PaneFacts {
    /// The member the runner bound it to as their rep.
    pub rep: Option<String>,
    /// For an orchestrator of a workspace: its lead, `None` while the owner
    /// leads.
    pub orchestrator_lead: Option<Option<String>>,
    /// The member login it was launched on, and whether that login is still
    /// theirs (`false`: they were removed, or the handle is someone else's).
    pub login: Option<(String, bool)>,
}

/// The rule for a pane: rep, then orchestrator, then its recorded login.
/// `None` is nobody's.
pub fn whose_pane(facts: &PaneFacts) -> Option<Whose> {
    if let Some(handle) = &facts.rep {
        return Some(Whose::Member(handle.clone()));
    }
    if let Some(lead) = &facts.orchestrator_lead {
        return Some(lead.clone().map_or(Whose::Owner, Whose::Member));
    }
    match &facts.login {
        Some((handle, true)) => Some(Whose::Member(handle.clone())),
        Some((_, false)) => None,
        None => Some(Whose::Owner),
    }
}

/// Who gets pushed about an item, and whether the owner's own notice still
/// alerts. The owner's notice always goes (it moves the live card and the
/// task's thread); it alerts unless the item is a member's, whose own phone
/// is told instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushTo {
    pub owner_alerts: bool,
    pub member: Option<String>,
}

/// `whose` of an item to `PushTo`. Nobody's and the owner's go to the owner,
/// as every push did before; a member's to the member alone.
pub fn push_to(whose: Option<&Whose>) -> PushTo {
    match whose {
        Some(Whose::Member(handle)) => PushTo { owner_alerts: false, member: Some(handle.clone()) },
        Some(Whose::Owner) | None => PushTo { owner_alerts: true, member: None },
    }
}

/// The wire's concern for `whose`, as `viewer` reads it. `names` holds each
/// member's name by handle.
pub fn concern(whose: Option<&Whose>, viewer: Option<&Person>, names: &HashMap<String, String>) -> Option<pb::NeedsYouConcern> {
    let whose = whose?;
    let yours = match (whose, viewer) {
        (Whose::Owner, Some(Person::Owner)) => true,
        (Whose::Member(handle), Some(Person::Member(me))) => handle == me,
        _ => false,
    };
    Some(match whose {
        Whose::Owner => pb::NeedsYouConcern { handle: String::new(), name: String::new(), yours },
        Whose::Member(handle) => pb::NeedsYouConcern {
            handle: handle.clone(),
            name: names.get(handle).cloned().unwrap_or_else(|| handle.clone()),
            yours,
        },
    })
}

/// Whether an item is someone else's: it sorts, and is filed, after the
/// reader's own and nobody's.
pub fn is_others(item: &pb::NeedsYouItem) -> bool {
    item.concern.as_ref().is_some_and(|c| !c.yours)
}

/// What the runner knows of its people around a set of items: read once per
/// list (`read`), or for one push (`about`), and asked by the one rule both
/// use (`whose`), so a list and a push can't disagree.
#[derive(Debug, Clone, Default)]
pub struct People {
    /// Whether the runner has a member. Without one no item carries a concern
    /// and every push is the owner's, as before.
    pub team: bool,
    /// Each pane's facts.
    pub panes: HashMap<Uuid, PaneFacts>,
    /// Each task's lane: the live pane that works for it (`task_link::task_of`),
    /// never an orchestrator, the oldest when there are several.
    pub lanes: HashMap<Uuid, Uuid>,
    /// Each desk card's member.
    pub desks: HashMap<Uuid, String>,
    /// Each member's name, by handle.
    pub names: HashMap<String, String>,
}

impl People {
    /// The person an item about `terminal` or `task` concerns: the pane's,
    /// else the task's lane's, else the task's desk's member, else nobody.
    pub fn whose(&self, terminal: Option<Uuid>, task: Option<Uuid>) -> Option<Whose> {
        if !self.team {
            return None;
        }
        if let Some(pane) = terminal.or_else(|| task.and_then(|t| self.lanes.get(&t).copied())) {
            return self.panes.get(&pane).map_or(Some(Whose::Owner), whose_pane);
        }
        task.and_then(|t| self.desks.get(&t)).map(|h| Whose::Member(h.clone()))
    }

    /// Whether `viewer` may answer on `terminal`: `people::gate`'s rule, from
    /// the same facts. A pane not read here is not one the gate limits.
    pub fn may_act(&self, viewer: &Person, terminal: Uuid) -> bool {
        self.panes.get(&terminal).is_none_or(|f| {
            crate::people::may_act_on(viewer, f.rep.as_deref(), f.orchestrator_lead.as_ref().map(|l| l.as_deref()))
        })
    }

    /// For a list: `terminals` (the listed ones) and `task_of` (each live
    /// agent's task, from `gather`).
    pub fn read(
        store: &farcooler_store::Store,
        terminals: &[models::Terminal],
        task_of: &HashMap<Uuid, Uuid>,
    ) -> farcooler_core::Result<People> {
        let mut people = People { team: store.any_members()?, ..People::default() };
        for t in terminals {
            people.panes.insert(t.id, pane_facts(store, t)?);
            if let Some(task) = task_of.get(&t.id) {
                people.note_lane(*task, t);
            }
        }
        if people.team {
            let workspaces = store.list_workspaces(None)?;
            for member in store.members()? {
                for ws in &workspaces {
                    if let Some(task) = store.rep_desk(ws.id, &member.handle)? {
                        people.desks.insert(task, member.handle.clone());
                    }
                }
                people.names.insert(member.handle, member.name);
            }
        }
        Ok(people)
    }

    /// For one push: `terminal`, or `task` with its lane and desk, by point
    /// lookups, so a push never reads whole tables.
    pub fn about(
        store: &farcooler_store::Store,
        terminal: Option<&models::Terminal>,
        task: Option<&models::Task>,
    ) -> farcooler_core::Result<People> {
        let mut people = People { team: store.any_members()?, ..People::default() };
        if !people.team {
            return Ok(people);
        }
        if let Some(t) = terminal {
            people.panes.insert(t.id, pane_facts(store, t)?);
        }
        if let Some(task) = task {
            let mut candidates = store.terminals_for_task(task.id)?;
            if let Some(lane) = task.worktree_id {
                candidates.extend(store.list_terminals_for_worktree(lane)?);
            }
            for t in candidates {
                if crate::task_link::task_of(store, &t).is_some_and(|of| of.id == task.id) {
                    people.panes.insert(t.id, pane_facts(store, &t)?);
                    people.note_lane(task.id, &t);
                }
            }
            for member in store.members()? {
                if store.rep_desk(task.workspace_id, &member.handle)? == Some(task.id) {
                    people.desks.insert(task.id, member.handle);
                }
            }
        }
        Ok(people)
    }

    /// `t` works for `task`: its lane, if it is live, not an orchestrator,
    /// and older than the lane noted so far.
    fn note_lane(&mut self, task: Uuid, t: &models::Terminal) {
        if t.role == TerminalRole::Orchestrator || crate::wire::has_ended(t) {
            return;
        }
        let lane = self.lanes.entry(task).or_insert(t.id);
        *lane = (*lane).min(t.id);
    }
}

/// One pane's facts by point lookups.
pub fn pane_facts(store: &farcooler_store::Store, term: &models::Terminal) -> farcooler_core::Result<PaneFacts> {
    let rep = store.rep_of_terminal(term.id)?.map(|r| r.handle);
    let orchestrator_lead = match (term.role, term.workspace_id) {
        (TerminalRole::Orchestrator, Some(ws)) => Some(store.workspace_lead(ws)?),
        _ => None,
    };
    let login = match store.terminal_login(term.id)? {
        Some(row) => {
            let current = store.member_login(&row.handle)?.as_deref() == Some(row.login_id.as_str());
            Some((row.handle, current))
        }
        None => None,
    };
    Ok(PaneFacts { rep, orchestrator_lead, login })
}

/// The person a push about `terminal` concerns: `People::whose`, as a list
/// reads it. `None` on a runner with no members, or for nobody's.
pub fn whose_terminal(store: &farcooler_store::Store, terminal: Uuid) -> Option<Whose> {
    let term = store.get_terminal(terminal).ok()?;
    People::about(store, Some(&term), None).ok()?.whose(Some(terminal), None)
}

/// The person a push about `task` concerns: `People::whose`, as a list reads
/// an item about the task with no pane of its own.
pub fn whose_task(store: &farcooler_store::Store, task: &models::Task) -> Option<Whose> {
    People::about(store, None, Some(task)).ok()?.whose(None, Some(task.id))
}

#[cfg(test)]
#[path = "concern_tests.rs"]
mod tests;
