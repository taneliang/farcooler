//! Atomic engine/legacy task ownership. Standalone items have an explicit NULL
//! task mapping; a board item resolves only on its own workspace by key or UUID.
use crate::workflow::Result;
use farcooler_workflow::{Input, Command};
use rusqlite::{Transaction, OptionalExtension, params};

fn task(tx: &Transaction<'_>, input: &Input, item: &str) -> Result<Option<Vec<u8>>> {
    let Ok(workspace) = uuid::Uuid::parse_str(&input.workspace) else { return Ok(None) };
    let id = uuid::Uuid::parse_str(item).ok().map(|u| u.as_bytes().to_vec());
    let mut statement = tx.prepare("SELECT id FROM tasks WHERE workspace_id=?1
        AND (key=?2 COLLATE NOCASE OR former_key=?2 COLLATE NOCASE OR id=?3) LIMIT 2")?;
    let mut rows = statement.query(params![workspace.as_bytes().as_slice(),item,id])?;
    let found = rows.next()?.map(|row| row.get(0)).transpose()?;
    if rows.next()?.is_some() { return Err(crate::workflow::WorkflowError::AmbiguousTask); }
    Ok(found)
}
pub(crate) fn available(tx: &Transaction<'_>, input: &Input) -> Result<bool> {
    // Start fences a ticket's subagent records and blocks (a run owns them), so it waits for the orchestrator to
    // settle them: otherwise a stopped subagent could never end and a block could never clear (ov-965).
    // The tickets that follow the run are fenced with it, so the same holds for each of them (ov-970).
    if let Command::Start { item, with, .. } = &input.command {
        for ticket in std::iter::once(item).chain(with) {
            let Some(task) = task(tx,input,ticket)? else { continue };
            if tx.query_row("SELECT EXISTS(SELECT 1 FROM ticket_work WHERE task_id=?1 AND ended_at IS NULL)
                OR EXISTS(SELECT 1 FROM task_blocks WHERE task_id=?1)", [task], |r| r.get::<_,bool>(0))? { return Ok(false); }
        }
        return Ok(true);
    }
    let Command::Enqueue { item, definition, .. } = &input.command else { return Ok(true) };
    let Some(task) = task(tx,input,item)? else { return Ok(true) };
    // A migrated card must be retained-intent. Its history stays inert (a landed line, an old
    // landing in any state since ov-960, a dead pane), but anything live after release is a hand worker the engine must
    // not run beside: the same facts `preflight` quarantines at cutover, plus live panes and
    // open work, which only a post-release hand agent can have (ov-946).
    let migrated: Option<String> = tx.query_row("SELECT disposition FROM engine_cutover_items WHERE task=?1", [&task], |r|r.get(0)).optional()?;
    let live = match migrated {
        Some(d) if d != "retained-intent" => return Ok(false),
        Some(_) => "OR EXISTS(SELECT 1 FROM terminals WHERE task_id=?1 AND exit_code IS NULL AND exit_signal IS NULL)
            OR EXISTS(SELECT 1 FROM ticket_lines WHERE task_id=?1 AND stage NOT IN ('landed','dropped'))
            OR EXISTS(SELECT 1 FROM task_blocks WHERE task_id=?1)
            OR EXISTS(SELECT 1 FROM tasks WHERE id=?1 AND wait_kind IS NOT NULL)
            OR EXISTS(SELECT 1 FROM check_in_schedules WHERE task_id=?1 AND state='enabled')",
        // A ticket tracked with no run takes over an existing card and its line; only a live worker blocks it.
        // The old landings are history whatever state they were left in, so none holds a ticket (ov-960).
        None if definition.is_ticket() => "OR EXISTS(SELECT 1 FROM terminals WHERE task_id=?1 AND exit_code IS NULL AND exit_signal IS NULL)",
        None => "OR EXISTS(SELECT 1 FROM terminals WHERE task_id=?1)
            OR EXISTS(SELECT 1 FROM ticket_lines WHERE task_id=?1)
            OR EXISTS(SELECT 1 FROM landing_tickets WHERE task_id=?1)",
    };
    Ok(!tx.query_row(&format!("SELECT EXISTS(SELECT 1 FROM workflow_owned_items WHERE task_id=?1)
        OR EXISTS(SELECT 1 FROM ticket_work WHERE task_id=?1 AND ended_at IS NULL) {live}"), [task], |r| r.get::<_,bool>(0))?)
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
            // The text registry 68 recognizes and narrows for a ticket at rest (`engine_stage`).
            tx.execute_batch(&format!("{};", crate::engine_stage::excludes(table, suffix, event, false)))?;
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

#[cfg(test)]
mod alias_tests {
    use super::*;
    use crate::{Store, models::Actor};
    use farcooler_workflow::{Definition, Grant, Refusal};
    use farcooler_protocol::v1::TerminalIntent;
    fn input(workspace: uuid::Uuid, item: String) -> Input {
        let definition = Definition::tracker("alias-owner");
        Input { schema:1, id:"alias".into(), workspace:workspace.to_string(), expected_revision:0,
            epoch:1, source:"controller".into(), source_sequence:1, now_ms:0,
            grant:Grant::Controller, busy_resources:Default::default(), command:Command::Enqueue {
                item, policy_pin:definition.pin(), resource:"alias-resource".into(), dependencies:Default::default(),
                initial_candidate:Some(definition.pin()), definition } }
    }
    fn alias_order(former: bool, engine_first: bool) {
                let store = Store::open_in_memory().unwrap();
                let repo = store.register_repository_for_test("Aliases");
                let workspace = store.ensure_main_workspace(repo).unwrap();
                let task = store.create_task(workspace.id,"alias",Actor::User).unwrap();
                let alias = if former {
                    store.conn().execute("UPDATE tasks SET former_key='-old42' WHERE id=?1", [task.id.as_bytes().as_slice()]).unwrap();
                    "-OLD42".to_string()
                } else { task.key.to_uppercase() };
                assert_ne!(alias, task.key);
                let tree = store.create_worktree(repo,"topic","/isolated",false).unwrap();
                let legacy = || store.create_terminal_for_task(tree.id,"worker","claude",TerminalIntent::Running,80,24,Some(task.id));
                let input = input(workspace.id, alias);
                if engine_first {
                    assert!(store.workflow_apply(&input).unwrap().result.is_ok());
                    assert!(legacy().is_err(), "engine alias must own the actual task UUID");
                } else {
                    legacy().unwrap();
                    assert_eq!(store.workflow_apply(&input).unwrap().result, Err(Refusal::Resource));
                    assert!(store.workflow_state(&input.workspace).unwrap().is_none());
                    assert!(store.workflow_outbox(&input.workspace).unwrap().is_empty());
                }
    }
    #[test]
    fn workflow_ownership_aliases_current_legacy_first() { alias_order(false,false); }
    #[test]
    fn workflow_ownership_aliases_current_engine_first() { alias_order(false,true); }
    #[test]
    fn workflow_ownership_aliases_former_legacy_first() { alias_order(true,false); }
    #[test]
    fn workflow_ownership_aliases_former_engine_first() { alias_order(true,true); }
    #[test]
    fn workflow_ownership_aliases_refuse_ambiguity_and_keep_workspace_boundary() {
        let store = Store::open_in_memory().unwrap();
        let repo = store.register_repository_for_test("Aliases");
        let workspace = store.ensure_main_workspace(repo).unwrap();
        let a = store.create_task(workspace.id,"a",Actor::User).unwrap();
        let b = store.create_task(workspace.id,"b",Actor::User).unwrap();
        store.conn().execute("UPDATE tasks SET former_key='-collision' WHERE id IN (?1,?2)",
            params![a.id.as_bytes().as_slice(), b.id.as_bytes().as_slice()]).unwrap();
        assert!(matches!(store.workflow_apply(&input(workspace.id,"-COLLISION".into())),
            Err(crate::workflow::WorkflowError::AmbiguousTask)));
        assert!(store.workflow_state(&workspace.id.to_string()).unwrap().is_none());
        // Another workspace's exact UUID/key cannot grant ownership here.
        for item in [a.id.to_string(), a.key, "genuinely-standalone".into()] {
            let mut conn = store.conn(); let tx = conn.transaction().unwrap();
            let other = uuid::Uuid::now_v7();
            assert_eq!(task(&tx, &input(other,item.clone()), &item).unwrap(), None);
        }
        {
            let mut conn = store.conn(); let tx = conn.transaction().unwrap();
            assert_eq!(task(&tx, &input(workspace.id,a.id.to_string()), &a.id.to_string()).unwrap(), Some(a.id.as_bytes().to_vec()));
        }
        let standalone = input(workspace.id,"genuinely-standalone".into());
        assert!(store.workflow_apply(&standalone).unwrap().result.is_ok());
        let mapped: Option<Vec<u8>> = store.conn().query_row(
            "SELECT task_id FROM workflow_owned_items WHERE workspace=?1 AND item='genuinely-standalone'",
            [&standalone.workspace], |r| r.get(0)).unwrap();
        assert_eq!(mapped,None,"a genuinely unmatched item must remain standalone");
    }
}
