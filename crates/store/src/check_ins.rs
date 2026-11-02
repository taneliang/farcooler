//! Durable state for fixed workspace check-ins (ov-892).
//!
//! These rows are deliberately separate from the ordinary message queues.
//! They have no cascading foreign keys: audit evidence and an admitted fence
//! must survive removal of the objects whose identities they record.

use rusqlite::Transaction;

/// Older runners cannot honour the admission fence and must not open this
/// schema. Registration uses `Older::Refused`, regardless of its additive DDL.
pub(crate) fn migration_check_ins(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
        CREATE TABLE check_in_schedules (
            workspace_id BLOB PRIMARY KEY NOT NULL,
            task_id BLOB NOT NULL,
            generation BLOB NOT NULL,
            authorizer_device TEXT,
            authorized_at INTEGER NOT NULL,
            template_version INTEGER NOT NULL,
            next_due INTEGER NOT NULL,
            state TEXT NOT NULL CHECK (state IN ('enabled', 'disabled', 'suspended')),
            eligibility_epoch INTEGER NOT NULL DEFAULT 0,
            reason TEXT
        );
        CREATE TABLE check_in_occurrences (
            id BLOB PRIMARY KEY NOT NULL,
            workspace_id BLOB NOT NULL,
            task_id BLOB NOT NULL,
            generation BLOB NOT NULL,
            eligibility_epoch INTEGER NOT NULL,
            template_version INTEGER NOT NULL,
            due_at INTEGER NOT NULL,
            state TEXT NOT NULL CHECK (state IN
                ('pending', 'claimed', 'enter_admitted', 'submitted', 'confirmed', 'cancelled', 'uncertain')),
            attempt_id BLOB,
            daemon_instance BLOB,
            terminal_id BLOB,
            thread_id TEXT,
            claimed_at INTEGER,
            admitted_at INTEGER,
            submitted_at INTEGER,
            confirmed_at INTEGER,
            observation TEXT,
            reason TEXT
        );
        CREATE UNIQUE INDEX check_in_one_active ON check_in_occurrences(workspace_id)
            WHERE state IN ('pending', 'claimed', 'enter_admitted', 'submitted', 'uncertain');
        CREATE TABLE check_in_enter_fences (
            workspace_id BLOB PRIMARY KEY NOT NULL,
            task_id BLOB NOT NULL,
            terminal_id BLOB NOT NULL,
            occurrence_id BLOB NOT NULL UNIQUE,
            attempt_id BLOB NOT NULL UNIQUE,
            daemon_instance BLOB NOT NULL,
            authorizer_device TEXT
        );
        "#,
    )?;

    // The veto and epoch change happen in the writer's own transaction. In
    // particular they catch indirect and bulk writes, not only RPC entrypoints.
    let rules = [
        ("task_update", "UPDATE OF status, workspace_id ON tasks", "NOT (OLD.workspace_id IS NEW.workspace_id) OR ((OLD.status IN ('done', 'cancelled')) != (NEW.status IN ('done', 'cancelled')))", "s.task_id = OLD.id"),
        ("task_delete", "DELETE ON tasks", "1", "s.task_id = OLD.id"),
        ("workspace_update", "UPDATE OF wake_on_answer ON workspaces", "NOT (OLD.wake_on_answer IS NEW.wake_on_answer)", "s.workspace_id = OLD.id"),
        ("workspace_delete", "DELETE ON workspaces", "1", "s.workspace_id = OLD.id"),
        ("lead_insert", "INSERT ON workspace_leads", "1", "s.workspace_id = NEW.workspace_id"),
        ("lead_update", "UPDATE ON workspace_leads", "NOT (OLD.workspace_id IS NEW.workspace_id AND OLD.handle IS NEW.handle)", "s.workspace_id IN (OLD.workspace_id, NEW.workspace_id)"),
        ("lead_delete", "DELETE ON workspace_leads", "1", "s.workspace_id = OLD.workspace_id"),
        ("terminal_insert", "INSERT ON terminals", "NEW.role = 2", "s.workspace_id = NEW.workspace_id"),
        ("terminal_update", "UPDATE ON terminals", "NOT (OLD.workspace_id IS NEW.workspace_id AND OLD.role IS NEW.role AND OLD.command_preset IS NEW.command_preset AND OLD.intent IS NEW.intent AND OLD.runtime_confirmed IS NEW.runtime_confirmed AND OLD.exit_code IS NEW.exit_code AND OLD.exit_signal IS NEW.exit_signal AND OLD.lease_generation IS NEW.lease_generation AND OLD.epoch IS NEW.epoch AND OLD.pane_mode IS NEW.pane_mode AND OLD.agent_session_id IS NEW.agent_session_id)", "(s.workspace_id IN (OLD.workspace_id, NEW.workspace_id) AND (OLD.role = 2 OR NEW.role = 2))"),
        ("terminal_delete", "DELETE ON terminals", "OLD.role = 2", "s.workspace_id = OLD.workspace_id"),
        ("repository_delete", "DELETE ON repositories", "1", "s.workspace_id IN (SELECT id FROM workspaces WHERE repository_id = OLD.id)"),
        ("worktree_delete", "DELETE ON worktrees", "1", "s.workspace_id IN (SELECT workspace_id FROM terminals WHERE worktree_id = OLD.id AND role = 2)"),
    ];
    for (name, event, changed, affected) in rules {
        tx.execute_batch(&format!(
            "CREATE TRIGGER check_in_{name} BEFORE {event} WHEN {changed} BEGIN \
             SELECT CASE WHEN EXISTS (SELECT 1 FROM check_in_schedules s JOIN check_in_enter_fences f ON f.workspace_id = s.workspace_id WHERE {affected}) \
                 THEN RAISE(ABORT, 'check_in_enter_in_flight') END; \
             UPDATE check_in_schedules AS s SET state = 'suspended', reason = 'eligibility_changed', eligibility_epoch = eligibility_epoch + 1 \
                 WHERE state = 'enabled' AND {affected}; END;"
        ))?;
    }
    for table in ["owner_devices", "device_caps", "released_devices", "member_devices"] {
        for (op, row) in [("INSERT", "NEW"), ("UPDATE", "OLD"), ("DELETE", "OLD")] {
            // A replacement or a rebinding must invalidate even if the same
            // device later returns to its former state.
            let affected = if op == "UPDATE" {
                "s.authorizer_device IN (OLD.client_id, NEW.client_id)".to_string()
            } else {
                format!("s.authorizer_device = {row}.client_id")
            };
            let name = format!("check_in_{table}_{}", op.to_lowercase());
            tx.execute_batch(&format!(
                "CREATE TRIGGER {name} BEFORE {op} ON {table} BEGIN \
                 SELECT CASE WHEN EXISTS (SELECT 1 FROM check_in_schedules s JOIN check_in_enter_fences f ON f.workspace_id = s.workspace_id WHERE {affected}) \
                     THEN RAISE(ABORT, 'check_in_enter_in_flight') END; \
                 UPDATE check_in_schedules AS s SET state = 'suspended', reason = 'authorizer_changed', eligibility_epoch = eligibility_epoch + 1 \
                     WHERE state = 'enabled' AND {affected}; END;"
            ))?;
        }
    }
    tx.execute_batch(
        "CREATE TRIGGER check_in_schedule_update BEFORE UPDATE ON check_in_schedules \
         WHEN EXISTS (SELECT 1 FROM check_in_enter_fences WHERE workspace_id = OLD.workspace_id) \
         BEGIN SELECT RAISE(ABORT, 'check_in_enter_in_flight'); END; \
         CREATE TRIGGER check_in_schedule_delete BEFORE DELETE ON check_in_schedules \
         WHEN EXISTS (SELECT 1 FROM check_in_enter_fences WHERE workspace_id = OLD.workspace_id) \
         BEGIN SELECT RAISE(ABORT, 'check_in_enter_in_flight'); END;"
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use farcooler_core::DomainError;
    use rusqlite::params;
    use uuid::Uuid;

    use crate::models::{Actor, TaskStatus, uuid_blob};
    use crate::Store;

    fn board() -> (Store, Uuid, Uuid) {
        let store = Store::open_in_memory().unwrap();
        let repo = store.register_repository_for_test("check-in");
        let ws = store.ensure_main_workspace(repo).unwrap().id;
        let task = store.create_task(ws, "audit", Actor::Manager).unwrap().id;
        store.conn().execute(
            "INSERT INTO check_in_schedules (workspace_id, task_id, generation, authorized_at, template_version, next_due, state) VALUES (?1, ?2, ?3, 1, 1, 3600001, 'enabled')",
            params![uuid_blob(ws), uuid_blob(task), uuid_blob(Uuid::now_v7())],
        ).unwrap();
        (store, ws, task)
    }

    #[test]
    fn status_change_invalidates_and_does_not_revive_on_return() {
        let (store, ws, task) = board();
        store.set_task_status(task, TaskStatus::Done, Actor::Manager).unwrap();
        store.set_task_status(task, TaskStatus::Backlog, Actor::Manager).unwrap();
        let state: (String, i64) = store.conn().query_row(
            "SELECT state, eligibility_epoch FROM check_in_schedules WHERE workspace_id = ?1",
            [uuid_blob(ws)], |r| Ok((r.get(0)?, r.get(1)?)),
        ).unwrap();
        assert_eq!(state, ("suspended".into(), 1));
    }

    #[test]
    fn admitted_enter_vetoes_status_until_fence_is_released() {
        let (store, ws, task) = board();
        store.conn().execute(
            "INSERT INTO check_in_enter_fences VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
            params![uuid_blob(ws), uuid_blob(task), uuid_blob(Uuid::now_v7()), uuid_blob(Uuid::now_v7()), uuid_blob(Uuid::now_v7()), uuid_blob(Uuid::now_v7())],
        ).unwrap();
        assert!(matches!(store.set_task_status(task, TaskStatus::Done, Actor::Manager), Err(DomainError::ResourceConflict)));
        assert_eq!(store.get_task(task).unwrap().status, TaskStatus::Backlog);
        let state: String = store.conn().query_row(
            "SELECT state FROM check_in_schedules WHERE workspace_id = ?1", [uuid_blob(ws)], |r| r.get(0),
        ).unwrap();
        assert_eq!(state, "enabled");
        store.conn().execute("DELETE FROM check_in_enter_fences WHERE workspace_id = ?1", [uuid_blob(ws)]).unwrap();
        store.set_task_status(task, TaskStatus::Done, Actor::Manager).unwrap();
    }
}
