//! Reading the board into `Inputs`. Reads only: every call here is a SELECT.

use std::collections::HashMap;

use farcooler_core::Result;
use farcooler_store::models::{Task, TaskStatus};
use farcooler_store::{Store, TaskScope};
use uuid::Uuid;

use super::{Inputs, Period, Scope, TaskFacts, spend};

/// Which of the runner's boards a report covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Narrowing {
    /// Every repository on the runner.
    Runner,
    Repository(Uuid),
    Workspace(Uuid),
}

/// Every task in `narrowing` that the period could have touched, with its
/// record, and what agents spent in it. A repository or workspace that isn't
/// on the runner is `NotFound`. `utc_offset_minutes` is where the client's
/// days begin, for the spend's by-period lines.
pub fn gather(store: &Store, narrowing: Narrowing, period: Period, utc_offset_minutes: i32) -> Result<Inputs> {
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

    let (spend, usage) = spend::read(store, narrowing, period, utc_offset_minutes)?;

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
            // An agent working a task the board didn't move still touched it.
            if !could_touch(&task, period) && !usage.contains_key(&task.id) {
                continue;
            }
            let notes = store.notes_for(task.id, None)?;
            tasks.push(TaskFacts { task, workspace: workspace.name.clone(), repository: repository.clone(), notes });
        }
    }

    Ok(Inputs { tasks, usage, spend, scope })
}

/// Whether anything about `task` could fall inside `period`: filed before it
/// ended, and either still open (so it spent time in a status) or moved or
/// written to since it began. A task finished and quiet before the period
/// is the one thing skipped, which is most of an old board.
fn could_touch(task: &Task, period: Period) -> bool {
    let open = !matches!(task.status, TaskStatus::Done | TaskStatus::Cancelled);
    task.created_at < period.until && (open || task.status_since >= period.since || task.updated_at >= period.since)
}
