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
        "CREATE TRIGGER check_in_schedule_insert BEFORE INSERT ON check_in_schedules \
         WHEN EXISTS (SELECT 1 FROM check_in_enter_fences WHERE workspace_id = NEW.workspace_id) \
         BEGIN SELECT RAISE(ABORT, 'check_in_enter_in_flight'); END; \
         CREATE TRIGGER check_in_schedule_update BEFORE UPDATE ON check_in_schedules \
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
    fn missed_hours_coalesce_without_using_message_wakes() {
        let (store, ws, _) = board();
        assert!(!store.tick_check_in(ws, 3_600_000).unwrap());
        assert!(store.tick_check_in(ws, 3_600_001).unwrap());
        assert!(!store.tick_check_in(ws, 20_000_000).unwrap());
        let conn = store.conn();
        let count: i64 = conn.query_row("SELECT count(*) FROM check_in_occurrences", [], |r| r.get(0)).unwrap();
        let messages: i64 = conn.query_row("SELECT count(*) FROM message_wakes", [], |r| r.get(0)).unwrap();
        assert_eq!((count, messages), (1, 0));
    }

    #[test]
    fn admission_vetoes_mutation_and_settles_only_its_attempt() {
        use farcooler_protocol::v1::TerminalIntent;
        let (store, ws, task) = board();
        let repo = store.get_workspace(ws).unwrap().repository_id;
        let worktree = store.create_worktree(repo, "test/check-in", "/check-in", false).unwrap();
        let terminal = store.create_terminal(worktree.id, "manager", "codex", TerminalIntent::Running, 80, 24).unwrap();
        store.conn().execute(
            "UPDATE terminals SET workspace_id = ?2, role = 2, runtime_confirmed = 1, agent_session_id = 'thread' WHERE id = ?1",
            params![uuid_blob(terminal.id), uuid_blob(ws)],
        ).unwrap();
        store.conn().execute("UPDATE check_in_schedules SET state = 'enabled' WHERE workspace_id = ?1", [uuid_blob(ws)]).unwrap();
        let (occ, attempt, daemon) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        store.conn().execute(
            "INSERT INTO check_in_occurrences (id, workspace_id, task_id, generation, eligibility_epoch, template_version, due_at, state, attempt_id, daemon_instance, terminal_id, thread_id)
             SELECT ?1, workspace_id, task_id, generation, eligibility_epoch, template_version, next_due, 'claimed', ?2, ?3, ?4, 'thread' FROM check_in_schedules WHERE workspace_id = ?5",
            params![uuid_blob(occ), uuid_blob(attempt), uuid_blob(daemon), uuid_blob(terminal.id), uuid_blob(ws)],
        ).unwrap();
        assert!(!store.admit_check_in_enter(occ, attempt, daemon, terminal.id, "other", 5).unwrap());
        assert!(store.admit_check_in_enter(occ, attempt, daemon, terminal.id, "thread", 5).unwrap());
        assert!(matches!(store.set_task_status(task, TaskStatus::Done, Actor::Manager), Err(DomainError::ResourceConflict)));
        assert!(matches!(store.disable_check_in(ws), Err(DomainError::ResourceConflict)));
        assert!(!store.settle_check_in_enter(occ, Uuid::now_v7(), false, 6).unwrap());
        assert!(store.settle_check_in_enter(occ, attempt, false, 6).unwrap());
        assert!(store.disable_check_in(ws).unwrap());
        store.set_task_status(task, TaskStatus::Done, Actor::Manager).unwrap();
        let state: String = store.conn().query_row("SELECT state FROM check_in_occurrences WHERE id = ?1", [uuid_blob(occ)], |r| r.get(0)).unwrap();
        assert_eq!(state, "uncertain");
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

impl crate::Store {
    /// Admit one already-claimed attempt immediately before its one Enter.
    /// The caller must hold the target's typing exclusion and complete all
    /// physical proofs before this call. `thread` is the proved session, not
    /// a value supplied by a remote client. No connection guard crosses I/O.
    pub fn admit_check_in_enter(
        &self,
        occurrence: uuid::Uuid,
        attempt: uuid::Uuid,
        daemon: uuid::Uuid,
        terminal: uuid::Uuid,
        thread: &str,
        now: i64,
    ) -> farcooler_core::Result<bool> {
        use crate::error::map_err;
        use crate::models::uuid_blob;
        use rusqlite::{OptionalExtension, params};

        if thread.is_empty() {
            return Ok(false);
        }
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(map_err)?;
        let valid: Option<(Vec<u8>, Vec<u8>, Option<String>)> = tx.query_row(
            "SELECT o.workspace_id, o.task_id, s.authorizer_device
               FROM check_in_occurrences o
               JOIN check_in_schedules s ON s.workspace_id = o.workspace_id
               JOIN tasks t ON t.id = o.task_id
               JOIN workspaces w ON w.id = o.workspace_id
               JOIN terminals p ON p.id = ?4
              WHERE o.id = ?1 AND o.attempt_id = ?2 AND o.daemon_instance = ?3
                AND o.state = 'claimed' AND o.terminal_id = ?4 AND o.thread_id = ?5
                AND s.state = 'enabled' AND s.generation = o.generation
                AND s.eligibility_epoch = o.eligibility_epoch AND s.task_id = o.task_id
                AND t.workspace_id = o.workspace_id AND t.status NOT IN ('done', 'cancelled')
                AND w.wake_on_answer = 1 AND p.workspace_id = o.workspace_id
                AND p.role = 2 AND p.command_preset = 'codex' AND p.intent = 1
                AND p.runtime_confirmed = 1 AND p.exit_code IS NULL AND p.exit_signal IS NULL
                AND p.agent_session_id = ?5
                AND (s.authorizer_device IS NULL OR (
                    EXISTS (SELECT 1 FROM owner_devices d WHERE d.client_id = s.authorizer_device AND d.state = 'confirmed')
                    AND NOT EXISTS (SELECT 1 FROM device_caps d WHERE d.client_id = s.authorizer_device)
                    AND NOT EXISTS (SELECT 1 FROM member_devices d WHERE d.client_id = s.authorizer_device)
                    AND NOT EXISTS (SELECT 1 FROM released_devices d WHERE d.client_id = s.authorizer_device)))",
            params![uuid_blob(occurrence), uuid_blob(attempt), uuid_blob(daemon), uuid_blob(terminal), thread],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).optional().map_err(map_err)?;
        let Some((workspace, task, device)) = valid else { return Ok(false) };
        tx.execute(
            "INSERT INTO check_in_enter_fences (workspace_id, task_id, terminal_id, occurrence_id, attempt_id, daemon_instance, authorizer_device)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![workspace, task, uuid_blob(terminal), uuid_blob(occurrence), uuid_blob(attempt), uuid_blob(daemon), device],
        ).map_err(map_err)?;
        let changed = tx.execute(
            "UPDATE check_in_occurrences SET state = 'enter_admitted', admitted_at = ?3 WHERE id = ?1 AND attempt_id = ?2 AND state = 'claimed'",
            params![uuid_blob(occurrence), uuid_blob(attempt), now],
        ).map_err(map_err)?;
        if changed != 1 { return Ok(false) }
        tx.commit().map_err(map_err)?;
        Ok(true)
    }

    /// Settle only the exact admitted attempt after its sender is joined and
    /// cannot start another send. A handed-off key may still arrive: submitted
    /// is not a receipt and uncertain is never retried automatically.
    pub fn settle_check_in_enter(
        &self,
        occurrence: uuid::Uuid,
        attempt: uuid::Uuid,
        submitted: bool,
        now: i64,
    ) -> farcooler_core::Result<bool> {
        use crate::error::map_err;
        use crate::models::uuid_blob;
        use rusqlite::params;

        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(map_err)?;
        let state = if submitted { "submitted" } else { "uncertain" };
        let changed = tx.execute(
            "UPDATE check_in_occurrences SET state = ?3, submitted_at = ?4 WHERE id = ?1 AND attempt_id = ?2 AND state = 'enter_admitted'
             AND EXISTS (SELECT 1 FROM check_in_enter_fences f WHERE f.occurrence_id = ?1 AND f.attempt_id = ?2)",
            params![uuid_blob(occurrence), uuid_blob(attempt), state, now],
        ).map_err(map_err)?;
        if changed != 1 { return Ok(false) }
        tx.execute("DELETE FROM check_in_enter_fences WHERE occurrence_id = ?1 AND attempt_id = ?2", params![uuid_blob(occurrence), uuid_blob(attempt)]).map_err(map_err)?;
        tx.commit().map_err(map_err)?;
        Ok(true)
    }
}

impl crate::Store {
    /// Coalesce all missed deadlines into one dedicated pending occurrence.
    /// A claimed, submitted or uncertain occurrence prevents another tick.
    pub fn tick_check_in(&self, workspace: uuid::Uuid, now: i64) -> farcooler_core::Result<bool> {
        use crate::error::map_err;
        use crate::models::uuid_blob;
        use rusqlite::params;

        let id = uuid::Uuid::now_v7();
        let changed = self.conn().execute(
            "INSERT INTO check_in_occurrences (id, workspace_id, task_id, generation, eligibility_epoch, template_version, due_at, state)
             SELECT ?1, s.workspace_id, s.task_id, s.generation, s.eligibility_epoch, s.template_version, s.next_due, 'pending'
               FROM check_in_schedules s
              WHERE s.workspace_id = ?2 AND s.state = 'enabled' AND s.next_due <= ?3
                AND NOT EXISTS (SELECT 1 FROM check_in_occurrences o WHERE o.workspace_id = s.workspace_id
                    AND o.state IN ('pending', 'claimed', 'enter_admitted', 'submitted', 'uncertain'))",
            params![uuid_blob(id), uuid_blob(workspace), now],
        ).map_err(map_err)?;
        Ok(changed == 1)
    }

    /// Persist an attempt before its first paste. The daemon supplies its own
    /// instance and the independently proved current terminal and thread.
    pub fn claim_check_in(
        &self,
        occurrence: uuid::Uuid,
        attempt: uuid::Uuid,
        daemon: uuid::Uuid,
        terminal: uuid::Uuid,
        thread: &str,
        now: i64,
    ) -> farcooler_core::Result<bool> {
        use crate::error::map_err;
        use crate::models::uuid_blob;
        use rusqlite::params;

        if thread.is_empty() { return Ok(false) }
        let changed = self.conn().execute(
            "UPDATE check_in_occurrences AS o SET state = 'claimed', attempt_id = ?2, daemon_instance = ?3,
                    terminal_id = ?4, thread_id = ?5, claimed_at = ?6
              WHERE o.id = ?1 AND o.state = 'pending'
                AND EXISTS (SELECT 1 FROM check_in_schedules s JOIN tasks t ON t.id = s.task_id
                    JOIN workspaces w ON w.id = s.workspace_id JOIN terminals p ON p.id = ?4
                    WHERE s.workspace_id = o.workspace_id AND s.state = 'enabled' AND s.generation = o.generation
                      AND s.eligibility_epoch = o.eligibility_epoch AND s.task_id = o.task_id
                      AND t.workspace_id = o.workspace_id AND t.status NOT IN ('done', 'cancelled')
                      AND w.wake_on_answer = 1 AND p.workspace_id = o.workspace_id AND p.role = 2
                      AND p.command_preset = 'codex' AND p.intent = 1 AND p.runtime_confirmed = 1
                      AND p.exit_code IS NULL AND p.exit_signal IS NULL AND p.agent_session_id = ?5
                      AND (s.authorizer_device IS NULL OR (
                          EXISTS (SELECT 1 FROM owner_devices d WHERE d.client_id = s.authorizer_device AND d.state = 'confirmed')
                          AND NOT EXISTS (SELECT 1 FROM device_caps d WHERE d.client_id = s.authorizer_device)
                          AND NOT EXISTS (SELECT 1 FROM member_devices d WHERE d.client_id = s.authorizer_device)
                          AND NOT EXISTS (SELECT 1 FROM released_devices d WHERE d.client_id = s.authorizer_device))))",
            params![uuid_blob(occurrence), uuid_blob(attempt), uuid_blob(daemon), uuid_blob(terminal), thread, now],
        ).map_err(map_err)?;
        Ok(changed == 1)
    }
}

impl crate::Store {
    /// Stop future admission for this workspace. The caller must authorize the
    /// verified owner or current own-workspace orchestrator before calling.
    /// A claimed attempt may already have pasted, so it becomes uncertain;
    /// an admitted attempt vetoes this transaction with ResourceConflict.
    pub fn disable_check_in(&self, workspace: uuid::Uuid) -> farcooler_core::Result<bool> {
        use crate::error::map_err;
        use crate::models::uuid_blob;
        use rusqlite::params;

        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(map_err)?;
        let admitted: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM check_in_enter_fences WHERE workspace_id = ?1)",
            params![uuid_blob(workspace)], |r| r.get(0),
        ).map_err(map_err)?;
        if admitted { return Err(farcooler_core::DomainError::ResourceConflict) }
        let changed = tx.execute(
            "UPDATE check_in_schedules SET state = 'disabled', eligibility_epoch = eligibility_epoch + 1, reason = 'disabled'
             WHERE workspace_id = ?1 AND state != 'disabled'",
            params![uuid_blob(workspace)],
        ).map_err(map_err)?;
        tx.execute(
            "UPDATE check_in_occurrences SET state = CASE state WHEN 'pending' THEN 'cancelled' ELSE 'uncertain' END,
                    reason = CASE state WHEN 'pending' THEN 'disabled_before_claim' ELSE 'disabled_after_claim' END
              WHERE workspace_id = ?1 AND state IN ('pending', 'claimed')",
            params![uuid_blob(workspace)],
        ).map_err(map_err)?;
        tx.commit().map_err(map_err)?;
        Ok(changed == 1)
    }
}
