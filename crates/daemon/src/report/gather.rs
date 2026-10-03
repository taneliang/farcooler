//! Reading the board into `Inputs`. Reads only: every call here is a SELECT.

use std::collections::HashMap;

use farcooler_core::Result;
use farcooler_store::models::{Task, TaskStatus};
use farcooler_store::{Store, TaskScope};
use uuid::Uuid;

use super::{Inputs, Period, Scope, TaskFacts, Usage};

/// Which of the runner's boards a report covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Narrowing {
    /// Every repository on the runner.
    Runner,
    Repository(Uuid),
    Workspace(Uuid),
}

/// Every task in `narrowing` that the period could have touched, with its
/// record. A repository or workspace that isn't on the runner is
/// `NotFound`.
pub fn gather(store: &Store, narrowing: Narrowing, period: Period) -> Result<Inputs> {
    let mut workspaces = store.list_workspaces(None)?;
    let scope = match narrowing {
        Narrowing::Runner => Scope::default(),
        Narrowing::Repository(id) => {
            let repository = store.get_repository(id)?;
            workspaces.retain(|w| w.repository_id == id);
            Scope { kind: "repository".into(), name: Some(repository.display_name), repository: None }
        }
        Narrowing::Workspace(id) => {
            let workspace = store.get_workspace(id)?;
            let repository = store.get_repository(workspace.repository_id)?;
            workspaces.retain(|w| w.id == id);
            Scope { kind: "workspace".into(), name: Some(workspace.name), repository: Some(repository.display_name) }
        }
    };

    let mut repository_names: HashMap<Uuid, String> = HashMap::new();
    let mut tasks = Vec::new();
    for workspace in workspaces {
        let repository = match repository_names.get(&workspace.repository_id) {
            Some(name) => name.clone(),
            None => {
                let name = store.get_repository(workspace.repository_id)?.display_name;
                repository_names.insert(workspace.repository_id, name.clone());
                name
            }
        };
        for task in store.list_tasks(TaskScope::Workspace(workspace.id), None)? {
            if !could_touch(&task, period) {
                continue;
            }
            let notes = store.notes_for(task.id, None)?;
            tasks.push(TaskFacts { task, workspace: workspace.name.clone(), repository: repository.clone(), notes });
        }
    }

    let usage = usage_in(store, &tasks, period)?;
    Ok(Inputs { tasks, usage, scope })
}

/// Whether anything about `task` could fall inside `period`: filed before it
/// ended, and either still open (so it spent time in a status) or moved or
/// written to since it began. A task finished and quiet before the period
/// is the one thing skipped, which is most of an old board.
fn could_touch(task: &Task, period: Period) -> bool {
    let open = !matches!(task.status, TaskStatus::Done | TaskStatus::Cancelled);
    task.created_at < period.until && (open || task.status_since >= period.since || task.updated_at >= period.since)
}

/// Token usage and agent time inside `period`, per task: THE SEAM for
/// ov-194.
///
/// Empty today, because the store records no usage yet. When ov-194's
/// per-turn records land, this sums each task's turns that ended inside the
/// period into a `Usage` (leaving a field `None` when no turn reported it)
/// and `compute` carries the sums into every tally and nothing else
/// changes. Read-only, like everything here.
fn usage_in(_store: &Store, _tasks: &[TaskFacts], _period: Period) -> Result<HashMap<Uuid, Usage>> {
    Ok(HashMap::new())
}
