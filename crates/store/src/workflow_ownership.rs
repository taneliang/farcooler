//! Atomic engine/legacy task ownership. Standalone items have an explicit NULL
//! task mapping; a board item resolves only on its own workspace by key or UUID.
use crate::workflow::Result;
use farcooler_workflow::{Input, Command};
use rusqlite::{Transaction, OptionalExtension, params};

fn task(tx: &Transaction<'_>, input: &Input, item: &str) -> Result<Option<Vec<u8>>> {
    let Ok(workspace) = uuid::Uuid::parse_str(&input.workspace) else { return Ok(None) };
    let id = uuid::Uuid::parse_str(item).ok().map(|u| u.as_bytes().to_vec());
    Ok(tx.query_row("SELECT id FROM tasks WHERE workspace_id=?1 AND (key=?2 OR id=?3)",
        params![workspace.as_bytes().as_slice(),item,id], |r| r.get(0)).optional()?)
}
pub(crate) fn available(tx: &Transaction<'_>, input: &Input) -> Result<bool> {
    let Command::Enqueue { item,.. } = &input.command else { return Ok(true) };
    let Some(task) = task(tx,input,item)? else { return Ok(true) };
    Ok(!tx.query_row("SELECT EXISTS(SELECT 1 FROM workflow_owned_items WHERE task_id=?1)
        OR EXISTS(SELECT 1 FROM terminals WHERE task_id=?1)
        OR EXISTS(SELECT 1 FROM ticket_lines WHERE task_id=?1)
        OR EXISTS(SELECT 1 FROM ticket_work WHERE task_id=?1 AND ended_at IS NULL)
        OR EXISTS(SELECT 1 FROM landing_tickets WHERE task_id=?1)", [task], |r| r.get::<_,bool>(0))?)
}
pub(crate) fn reserve(tx: &Transaction<'_>, input: &Input) -> Result<()> {
    if let Command::Enqueue { item,.. } = &input.command {
        tx.execute("INSERT INTO workflow_owned_items(workspace,item,task_id) VALUES(?1,?2,?3)",
            params![input.workspace,item,task(tx,input,item)?])?;
    }
    Ok(())
}
pub(crate) fn migration(tx: &Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch("CREATE TABLE workflow_owned_items(workspace TEXT NOT NULL,item TEXT NOT NULL,task_id BLOB UNIQUE REFERENCES tasks(id),PRIMARY KEY(workspace,item));
        CREATE TRIGGER workflow_owned_no_update BEFORE UPDATE ON workflow_owned_items BEGIN SELECT RAISE(ABORT,'immutable execution ownership'); END;
        CREATE TRIGGER workflow_owned_no_delete BEFORE DELETE ON workflow_owned_items BEGIN SELECT RAISE(ABORT,'immutable execution ownership'); END;")?;
    // All normal legacy entry points converge on these durable writes, including
    // a relink/restart or worker discovered after the engine transaction commits.
    for table in ["terminals","ticket_lines","ticket_work","landing_tickets"] {
        for (suffix,event) in [("insert","INSERT"),("update","UPDATE")] {
            tx.execute_batch(&format!("CREATE TRIGGER workflow_excludes_{table}_{suffix} BEFORE {event} ON {table}
                WHEN NEW.task_id IS NOT NULL AND EXISTS(SELECT 1 FROM workflow_owned_items WHERE task_id=NEW.task_id)
                BEGIN SELECT RAISE(ABORT,'task owned by workflow engine'); END;"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Store, models::Actor};
    use farcooler_workflow::{Definition,Grant,Refusal};
    use farcooler_protocol::v1::TerminalIntent;
    #[test]
    fn workflow_ownership_concurrent_engine_and_legacy_have_one_winner() {
        for _ in 0..6 {
            let store=std::sync::Arc::new(Store::open_in_memory().unwrap());
            let repo=store.register_repository_for_test("Workflow ownership");
            let workspace=store.ensure_main_workspace(repo).unwrap();
            let task=store.create_task(workspace.id,"fresh",Actor::User).unwrap();
            let tree=store.create_worktree(repo,"topic","/isolated",false).unwrap();
            let definition=Definition::tracker("owner");
            let input=Input {schema:1,id:"e".into(),workspace:workspace.id.to_string(),expected_revision:0,epoch:1,
                source:"controller".into(),source_sequence:1,now_ms:0,grant:Grant::Controller,busy_resources:Default::default(),
                command:Command::Enqueue {item:task.key.clone(),policy_pin:definition.pin(),resource:"own".into(),
                    dependencies:Default::default(),initial_candidate:Some(definition.pin()),definition}};
            let gate=std::sync::Arc::new(std::sync::Barrier::new(2));
            let (other,ready)=(store.clone(),gate.clone());
            let legacy=std::thread::spawn(move || { ready.wait(); other.create_terminal_for_task(tree.id,"worker","claude",TerminalIntent::Running,80,24,Some(task.id)) });
            gate.wait();let engine=store.workflow_apply(&input).unwrap();let legacy=legacy.join().unwrap();
            assert_ne!(engine.result.is_ok(),legacy.is_ok(),"both paths won or neither did");
            if engine.result.is_err() {assert_eq!(engine.result,Err(Refusal::Resource));}
            else {
                assert!(store.create_terminal_for_task(tree.id,"again","claude",TerminalIntent::Running,80,24,Some(task.id)).is_err());
                assert_eq!(store.workflow_apply(&input).unwrap(),engine,"receipt retry must not reclaim ownership");
            }
        }
    }
}

#[cfg(test)]
mod legacy_first_tests {
    use super::*;
    use crate::{Store,models::Actor};
    use farcooler_workflow::{Definition,Grant,Refusal};
    use farcooler_protocol::v1::TerminalIntent;
    #[test]
    fn workflow_ownership_existing_legacy_terminal_refuses_before_any_effect() {
        let store=Store::open_in_memory().unwrap();let repo=store.register_repository_for_test("Ownership");
        let workspace=store.ensure_main_workspace(repo).unwrap();let task=store.create_task(workspace.id,"legacy",Actor::User).unwrap();
        let tree=store.create_worktree(repo,"topic","/isolated",false).unwrap();
        store.create_terminal_for_task(tree.id,"worker","claude",TerminalIntent::Running,80,24,Some(task.id)).unwrap();
        let definition=Definition::tracker("owner");let input=Input {schema:1,id:"e".into(),workspace:workspace.id.to_string(),expected_revision:0,epoch:1,
            source:"controller".into(),source_sequence:1,now_ms:0,grant:Grant::Controller,busy_resources:Default::default(),
            command:Command::Enqueue {item:task.key,policy_pin:definition.pin(),resource:"own".into(),dependencies:Default::default(),initial_candidate:Some(definition.pin()),definition}};
        let receipt=store.workflow_apply(&input).unwrap();assert_eq!(receipt.result,Err(Refusal::Resource));
        assert!(store.workflow_outbox(&input.workspace).unwrap().is_empty());
        assert!(store.workflow_state(&input.workspace).unwrap().is_none());
    }
}
