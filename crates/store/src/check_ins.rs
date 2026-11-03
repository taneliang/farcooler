//! Durable state for fixed workspace check-ins (ov-892).
//!
//! These rows are deliberately separate from the ordinary message queues.
//! They have no cascading foreign keys: audit evidence and an admitted fence
//! must survive removal of the objects whose identities they record.

use rusqlite::Transaction;

/// The only text the scheduler can submit. It is constructed by the runner
/// and stored with the occurrence; no caller-supplied prompt is accepted.
pub fn fixed_check_in_message(task_key: &str, occurrence: uuid::Uuid) -> String {
    format!("[Far Cooler] Scheduled hourly check-in for this workspace. Follow its charter, check the board and CI, continue ready work, and report on {task_key}. Check-in {occurrence}. This reminder grants no new approval.")
}

fn valid_pane_id(pane: &str) -> bool {
    pane.strip_prefix('%').is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

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
            authorizer_source TEXT NOT NULL CHECK (authorizer_source IN ('local_person', 'owner_device')),
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
            authorizer_device TEXT,
            authorizer_source TEXT NOT NULL CHECK (authorizer_source IN ('local_person', 'owner_device')),
            authorized_at INTEGER NOT NULL,
            template_version INTEGER NOT NULL,
            due_at INTEGER NOT NULL,
            state TEXT NOT NULL CHECK (state IN
                ('pending', 'claimed', 'enter_admitted', 'submitted', 'confirmed', 'cancelled', 'uncertain', 'resolved_uncertain')),
            attempt_id BLOB,
            daemon_instance BLOB,
            terminal_id BLOB,
            pane_id TEXT,
            terminal_lease INTEGER,
            terminal_epoch INTEGER,
            process_id INTEGER,
            process_started INTEGER,
            thread_id TEXT,
            message_text TEXT,
            rollout_path TEXT,
            rollout_device INTEGER,
            rollout_inode INTEGER,
            rollout_offset INTEGER,
            claimed_at INTEGER,
            admitted_at INTEGER,
            submitted_at INTEGER,
            confirmed_at INTEGER,
            observation TEXT,
            reason TEXT
        );
        CREATE UNIQUE INDEX check_in_one_active ON check_in_occurrences(workspace_id)
            WHERE state IN ('pending', 'claimed', 'enter_admitted', 'submitted', 'uncertain');
        -- Independent of a schedule: a concurrent first enable cannot race a
        -- runner-controlled terminal stop that began while check-ins were off.
        CREATE TABLE check_in_target_reservations (
            token BLOB PRIMARY KEY NOT NULL,
            workspace_id BLOB,
            terminal_id BLOB NOT NULL,
            operation TEXT NOT NULL CHECK (operation IN ('stop', 'remove')),
            phase TEXT NOT NULL CHECK (phase IN ('reserved', 'completed', 'uncertain')),
            process_id INTEGER NOT NULL,
            process_started INTEGER,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX check_in_reservations_by_workspace ON check_in_target_reservations(workspace_id);
        -- A cleanup whose server-side completion is unknown suspends only
        -- the scheduler; ordinary cleanup and registration remain available.
        CREATE TABLE check_in_instance (
            singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
            instance_id BLOB NOT NULL,
            clean INTEGER NOT NULL CHECK (clean IN (0, 1))
        );
        CREATE TABLE check_in_cleanup_fence (
            token BLOB PRIMARY KEY NOT NULL,
            started_at INTEGER NOT NULL
        );
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
        ("lead_insert", "INSERT ON workspace_leads", "NOT EXISTS (SELECT 1 FROM workspace_leads WHERE workspace_id = NEW.workspace_id AND handle IS NEW.handle)", "s.workspace_id = NEW.workspace_id"),
        ("lead_update", "UPDATE ON workspace_leads", "NOT (OLD.workspace_id IS NEW.workspace_id AND OLD.handle IS NEW.handle)", "s.workspace_id IN (OLD.workspace_id, NEW.workspace_id)"),
        ("lead_delete", "DELETE ON workspace_leads", "1", "s.workspace_id = OLD.workspace_id"),
        ("terminal_insert", "INSERT ON terminals", "NEW.role = 2", "s.workspace_id = NEW.workspace_id"),
        ("terminal_update", "UPDATE ON terminals", "NOT (OLD.workspace_id IS NEW.workspace_id AND OLD.role IS NEW.role AND OLD.command_preset IS NEW.command_preset AND OLD.intent IS NEW.intent AND OLD.runtime_confirmed IS NEW.runtime_confirmed AND OLD.exit_code IS NEW.exit_code AND OLD.exit_signal IS NEW.exit_signal AND OLD.lease_generation IS NEW.lease_generation AND OLD.epoch IS NEW.epoch AND OLD.pane_mode IS NEW.pane_mode AND OLD.agent_session_id IS NEW.agent_session_id)", "(s.workspace_id IN (OLD.workspace_id, NEW.workspace_id) AND (OLD.role = 2 OR NEW.role = 2))"),
        ("terminal_delete", "DELETE ON terminals", "OLD.role = 2", "s.workspace_id = OLD.workspace_id"),
        ("repository_delete", "DELETE ON repositories", "1", "s.workspace_id IN (SELECT id FROM workspaces WHERE repository_id = OLD.id)"),
        ("worktree_delete", "DELETE ON worktrees", "1", "s.workspace_id IN (SELECT workspace_id FROM terminals WHERE worktree_id = OLD.id AND role = 2)"),
    ];
    for (name, event, changed, affected) in rules {
        let update_affected = affected.replace("s.", "check_in_schedules.");
        tx.execute_batch(&format!(
            "CREATE TRIGGER check_in_{name} BEFORE {event} WHEN {changed} BEGIN \
             SELECT CASE WHEN EXISTS (SELECT 1 FROM check_in_schedules s JOIN check_in_enter_fences f ON f.workspace_id = s.workspace_id WHERE {affected}) \
                 THEN RAISE(ABORT, 'check_in_enter_in_flight') END; \
             UPDATE check_in_schedules SET state = 'suspended', reason = 'eligibility_changed', eligibility_epoch = eligibility_epoch + 1 \
                 WHERE state = 'enabled' AND {update_affected}; END;"
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
            let update_affected = affected.replace("s.", "check_in_schedules.");
            let name = format!("check_in_{table}_{}", op.to_lowercase());
            let value = match table {
                "owner_devices" => Some("state"),
                "device_caps" => Some("cap"),
                "member_devices" => Some("handle"),
                _ => None,
            };
            let changed = match (op, value) {
                ("UPDATE", Some(column)) => format!("NOT (OLD.client_id IS NEW.client_id AND OLD.{column} IS NEW.{column})"),
                ("UPDATE", None) => "NOT (OLD.client_id IS NEW.client_id)".to_string(),
                ("INSERT", Some(column)) => format!("NOT EXISTS (SELECT 1 FROM {table} WHERE client_id = NEW.client_id AND {column} IS NEW.{column})"),
                ("INSERT", None) => format!("NOT EXISTS (SELECT 1 FROM {table} WHERE client_id = NEW.client_id)"),
                _ => "1".to_string(),
            };
            tx.execute_batch(&format!(
                "CREATE TRIGGER {name} BEFORE {op} ON {table} WHEN {changed} BEGIN \
                 SELECT CASE WHEN EXISTS (SELECT 1 FROM check_in_schedules s JOIN check_in_enter_fences f ON f.workspace_id = s.workspace_id WHERE {affected}) \
                     THEN RAISE(ABORT, 'check_in_enter_in_flight') END; \
                 UPDATE check_in_schedules SET state = 'suspended', reason = 'authorizer_changed', eligibility_epoch = eligibility_epoch + 1 \
                     WHERE state = 'enabled' AND {update_affected}; END;"
            ))?;
        }
    }
    tx.execute_batch(
        "CREATE TRIGGER check_in_schedule_insert BEFORE INSERT ON check_in_schedules \
         WHEN EXISTS (SELECT 1 FROM check_in_enter_fences WHERE workspace_id = NEW.workspace_id) \
         BEGIN SELECT RAISE(ABORT, 'check_in_enter_in_flight'); END; \
         CREATE TRIGGER check_in_reserved_target_move BEFORE UPDATE OF workspace_id, role ON terminals \
         WHEN NOT (OLD.workspace_id IS NEW.workspace_id AND OLD.role IS NEW.role) \
          AND EXISTS (SELECT 1 FROM check_in_target_reservations WHERE terminal_id = OLD.id) \
         BEGIN SELECT RAISE(ABORT, 'check_in_target_in_flight'); END; \
         CREATE TRIGGER check_in_schedule_update BEFORE UPDATE ON check_in_schedules \
         WHEN EXISTS (SELECT 1 FROM check_in_enter_fences WHERE workspace_id = OLD.workspace_id) \
         BEGIN SELECT RAISE(ABORT, 'check_in_enter_in_flight'); END; \
         CREATE TRIGGER check_in_schedule_delete BEFORE DELETE ON check_in_schedules \
         WHEN EXISTS (SELECT 1 FROM check_in_enter_fences WHERE workspace_id = OLD.workspace_id) \
         BEGIN SELECT RAISE(ABORT, 'check_in_enter_in_flight'); END;"
    )?;
    Ok(())
}

/// Persist the additional destination proof without changing schema 59.
/// Existing attempts lack this proof and cannot be admitted or confirmed by
/// the new observer; recovery and expiry retain their existing safe paths.
pub(crate) fn migration_check_in_destination(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch("ALTER TABLE check_in_occurrences ADD COLUMN tty TEXT;
                      ALTER TABLE check_in_occurrences ADD COLUMN process_incarnation INTEGER;")
}


impl crate::Store {
    /// Persist a new unclean instance before allowing any scheduled effects.
    /// Missing history is accepted only when no old operation is unresolved.
    pub fn arm_check_in_instance(&self, instance: uuid::Uuid) -> farcooler_core::Result<bool> {
        use crate::error::map_err;
        use crate::models::uuid_blob;
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(map_err)?;
        let blocked: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM check_in_instance WHERE clean = 0)
               OR EXISTS (SELECT 1 FROM check_in_cleanup_fence)
               OR EXISTS (SELECT 1 FROM check_in_enter_fences)
               OR EXISTS (SELECT 1 FROM check_in_occurrences WHERE state IN ('claimed', 'enter_admitted', 'submitted', 'uncertain'))
               OR EXISTS (SELECT 1 FROM check_in_target_reservations)", [], |r| r.get(0),
        ).map_err(map_err)?;
        if blocked { return Ok(false) }
        tx.execute(
            "INSERT INTO check_in_instance (singleton, instance_id, clean) VALUES (1, ?1, 0)
             ON CONFLICT(singleton) DO UPDATE SET instance_id = excluded.instance_id, clean = 0",
            [uuid_blob(instance)],
        ).map_err(map_err)?;
        tx.commit().map_err(map_err)?;
        Ok(true)
    }

    /// Mark only this instance clean after its owned work is joined and no
    /// uncertainty remains. Failure leaves the unclean sentinel intact.
    pub fn close_check_in_instance(&self, instance: uuid::Uuid) -> farcooler_core::Result<bool> {
        use crate::error::map_err;
        use crate::models::uuid_blob;
        Ok(self.conn().execute(
            "UPDATE check_in_instance SET clean = 1 WHERE singleton = 1 AND instance_id = ?1 AND clean = 0
             AND NOT EXISTS (SELECT 1 FROM check_in_cleanup_fence)
             AND NOT EXISTS (SELECT 1 FROM check_in_enter_fences)
             AND NOT EXISTS (SELECT 1 FROM check_in_occurrences WHERE state IN ('claimed', 'enter_admitted', 'submitted', 'uncertain'))
             AND NOT EXISTS (SELECT 1 FROM check_in_target_reservations)", [uuid_blob(instance)],
        ).map_err(map_err)? == 1)
    }

    /// Mark one cleanup operation before a server command may be dispatched.
    /// Multiple outstanding operations are independent; no restart clears them.
    pub fn begin_check_in_cleanup(&self, token: uuid::Uuid) -> farcooler_core::Result<()> {
        use crate::error::map_err;
        use crate::models::uuid_blob;
        self.conn().execute(
            "INSERT INTO check_in_cleanup_fence (token, started_at) VALUES (?1, ?2)",
            rusqlite::params![uuid_blob(token), crate::tasks::now_millis()],
        ).map_err(map_err)?;
        Ok(())
    }

    /// Clear only the operation whose owned client concluded successfully.
    pub fn complete_check_in_cleanup(&self, token: uuid::Uuid) -> farcooler_core::Result<bool> {
        use crate::error::map_err;
        use crate::models::uuid_blob;
        Ok(self.conn().execute("DELETE FROM check_in_cleanup_fence WHERE token = ?1", [uuid_blob(token)]).map_err(map_err)? == 1)
    }

    /// Admit one already-claimed attempt immediately before its one Enter.
    /// The caller must hold the target's typing exclusion and complete all
    /// physical proofs before this call. `thread` is the proved session, not
    /// a value supplied by a remote client. No connection guard crosses I/O.
    #[allow(clippy::too_many_arguments)] // The identity tuple pins the exact admitted attempt and pane.
    pub fn admit_check_in_enter(
        &self,
        occurrence: uuid::Uuid,
        attempt: uuid::Uuid,
        daemon: uuid::Uuid,
        terminal: uuid::Uuid,
        thread: &str,
        pane: &str,
        process_id: i32,
        process_started: i64,
        process_incarnation: i64,
        now: i64,
    ) -> farcooler_core::Result<bool> {
        use crate::error::map_err;
        use crate::models::uuid_blob;
        use rusqlite::{OptionalExtension, params};

        if thread.is_empty() || !valid_pane_id(pane) || process_id <= 0 || process_started <= 0 || process_incarnation <= 0 {
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
                AND o.state = 'claimed' AND o.terminal_id = ?4 AND o.thread_id = ?5 AND o.pane_id = ?6 AND o.process_id = ?7 AND o.process_started = ?8
                AND o.message_text IS NOT NULL AND NOT EXISTS (SELECT 1 FROM check_in_target_reservations r WHERE r.workspace_id = o.workspace_id)
                AND NOT EXISTS (SELECT 1 FROM check_in_cleanup_fence)
                AND o.tty IS NOT NULL AND o.process_incarnation = ?9 AND o.rollout_path IS NOT NULL AND o.rollout_device IS NOT NULL
                AND o.rollout_inode IS NOT NULL AND o.rollout_offset IS NOT NULL
                AND s.state = 'enabled' AND s.generation = o.generation
                AND s.eligibility_epoch = o.eligibility_epoch AND s.task_id = o.task_id
                AND s.authorizer_device IS o.authorizer_device AND s.authorizer_source = o.authorizer_source AND s.authorized_at = o.authorized_at
                AND s.template_version = o.template_version
                AND t.workspace_id = o.workspace_id AND t.status NOT IN ('done', 'cancelled')
                AND w.wake_on_answer = 1 AND p.workspace_id = o.workspace_id
                AND p.lease_generation = o.terminal_lease AND p.epoch = o.terminal_epoch
                AND p.role = 2 AND p.pane_mode = 0 AND (p.command_preset = 'codex' OR p.command_preset LIKE 'codex:%') AND p.intent = 1
                AND p.runtime_confirmed = 1 AND p.exit_code IS NULL AND p.exit_signal IS NULL
                AND p.agent_session_id = ?5
                AND (s.authorizer_device IS NULL OR (
                    EXISTS (SELECT 1 FROM owner_devices d WHERE d.client_id = s.authorizer_device AND d.state = 'confirmed')
                    AND NOT EXISTS (SELECT 1 FROM device_caps d WHERE d.client_id = s.authorizer_device)
                    AND NOT EXISTS (SELECT 1 FROM member_devices d WHERE d.client_id = s.authorizer_device)
                    AND NOT EXISTS (SELECT 1 FROM released_devices d WHERE d.client_id = s.authorizer_device)))",
            params![uuid_blob(occurrence), uuid_blob(attempt), uuid_blob(daemon), uuid_blob(terminal), thread, pane, process_id, process_started, process_incarnation],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).optional().map_err(map_err)?;
        let Some((workspace, task, device)) = valid else { return Ok(false) };
        tx.execute(
            "INSERT INTO check_in_enter_fences (workspace_id, task_id, terminal_id, occurrence_id, attempt_id, daemon_instance, authorizer_device)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![workspace, task, uuid_blob(terminal), uuid_blob(occurrence), uuid_blob(attempt), uuid_blob(daemon), device],
        ).map_err(map_err)?;
        let changed = tx.execute(
            "UPDATE check_in_occurrences SET state = 'enter_admitted', admitted_at = ?3, reason = 'enter_in_flight' WHERE id = ?1 AND attempt_id = ?2 AND state = 'claimed'",
            params![uuid_blob(occurrence), uuid_blob(attempt), now],
        ).map_err(map_err)?;
        if changed != 1 { return Ok(false) }
        tx.commit().map_err(map_err)?;
        Ok(true)
    }

    /// Settle only the exact admitted attempt after its sender is joined and
    /// cannot start another send. Only a successful client completion uses
    /// this path; an error retains the fence. Submission is not a receipt.
    pub fn settle_check_in_enter(
        &self,
        occurrence: uuid::Uuid,
        attempt: uuid::Uuid,
        now: i64,
    ) -> farcooler_core::Result<bool> {
        use crate::error::map_err;
        use crate::models::uuid_blob;
        use rusqlite::params;

        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(map_err)?;
        let changed = tx.execute(
            "UPDATE check_in_occurrences SET state = 'submitted', submitted_at = ?3, reason = 'awaiting_receipt' WHERE id = ?1 AND attempt_id = ?2 AND state = 'enter_admitted'
             AND EXISTS (SELECT 1 FROM check_in_enter_fences f WHERE f.occurrence_id = ?1 AND f.attempt_id = ?2)",
            params![uuid_blob(occurrence), uuid_blob(attempt), now],
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
            "INSERT INTO check_in_occurrences (id, workspace_id, task_id, generation, eligibility_epoch, authorizer_device, authorizer_source, authorized_at, template_version, due_at, state, reason)
             SELECT ?1, s.workspace_id, s.task_id, s.generation, s.eligibility_epoch, s.authorizer_device, s.authorizer_source, s.authorized_at, s.template_version, s.next_due, 'pending', 'waiting_for_idle_codex'
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
    #[allow(clippy::too_many_arguments)] // Claim the exact terminal, pane and session as one identity.
    pub fn claim_check_in(
        &self,
        occurrence: uuid::Uuid,
        attempt: uuid::Uuid,
        daemon: uuid::Uuid,
        terminal: uuid::Uuid,
        thread: &str,
        pane: &str,
        process_id: i32,
        process_started: i64,
        now: i64,
    ) -> farcooler_core::Result<bool> {
        use crate::error::map_err;
        use crate::models::uuid_blob;
        use rusqlite::{OptionalExtension, params};

        if thread.is_empty() || !valid_pane_id(pane) || process_id <= 0 || process_started <= 0 { return Ok(false) }
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(map_err)?;
        let task_key: Option<String> = tx.query_row(
            "SELECT t.key FROM check_in_occurrences o JOIN tasks t ON t.id = o.task_id WHERE o.id = ?1 AND o.state = 'pending'",
            params![uuid_blob(occurrence)], |r| r.get(0),
        ).optional().map_err(map_err)?;
        let Some(task_key) = task_key else { return Ok(false) };
        let message = fixed_check_in_message(&task_key, occurrence);
        let changed = tx.execute(
            "UPDATE check_in_occurrences AS o SET state = 'claimed', attempt_id = ?2, daemon_instance = ?3,
                    terminal_id = ?4, thread_id = ?5, claimed_at = ?6, pane_id = ?7,
                    terminal_lease = (SELECT lease_generation FROM terminals WHERE id = ?4),
                    terminal_epoch = (SELECT epoch FROM terminals WHERE id = ?4), message_text = ?8, reason = 'delivery_in_progress',
                    process_id = ?9, process_started = ?10
              WHERE o.id = ?1 AND o.state = 'pending'
                AND NOT EXISTS (SELECT 1 FROM check_in_target_reservations r WHERE r.workspace_id = o.workspace_id)
                AND NOT EXISTS (SELECT 1 FROM check_in_cleanup_fence)
                AND EXISTS (SELECT 1 FROM check_in_schedules s JOIN tasks t ON t.id = s.task_id
                    JOIN workspaces w ON w.id = s.workspace_id JOIN terminals p ON p.id = ?4
                    WHERE s.workspace_id = o.workspace_id AND s.state = 'enabled' AND s.generation = o.generation
                      AND s.eligibility_epoch = o.eligibility_epoch AND s.task_id = o.task_id
                AND s.authorizer_device IS o.authorizer_device AND s.authorizer_source = o.authorizer_source AND s.authorized_at = o.authorized_at
                AND s.template_version = o.template_version
                      AND t.workspace_id = o.workspace_id AND t.status NOT IN ('done', 'cancelled')
                      AND w.wake_on_answer = 1 AND p.workspace_id = o.workspace_id AND p.role = 2 AND p.pane_mode = 0
                      AND (p.command_preset = 'codex' OR p.command_preset LIKE 'codex:%') AND p.intent = 1 AND p.runtime_confirmed = 1
                      AND p.exit_code IS NULL AND p.exit_signal IS NULL AND p.agent_session_id = ?5
                      AND (s.authorizer_device IS NULL OR (
                          EXISTS (SELECT 1 FROM owner_devices d WHERE d.client_id = s.authorizer_device AND d.state = 'confirmed')
                          AND NOT EXISTS (SELECT 1 FROM device_caps d WHERE d.client_id = s.authorizer_device)
                          AND NOT EXISTS (SELECT 1 FROM member_devices d WHERE d.client_id = s.authorizer_device)
                          AND NOT EXISTS (SELECT 1 FROM released_devices d WHERE d.client_id = s.authorizer_device))))",
            params![uuid_blob(occurrence), uuid_blob(attempt), uuid_blob(daemon), uuid_blob(terminal), thread, now, pane, message, process_id, process_started],
        ).map_err(map_err)?;
        tx.commit().map_err(map_err)?;
        Ok(changed == 1)
    }
}

impl crate::Store {
    /// Read the runner-built text for exactly the claimed attempt. It is
    /// stored at claim so later task edits cannot change what is confirmed.
    pub fn claimed_check_in_message(&self, occurrence: uuid::Uuid, attempt: uuid::Uuid) -> farcooler_core::Result<Option<String>> {
        use crate::error::map_err;
        use crate::models::uuid_blob;
        use rusqlite::{OptionalExtension, params};
        self.conn().query_row(
            "SELECT message_text FROM check_in_occurrences WHERE id = ?1 AND attempt_id = ?2 AND state = 'claimed' AND message_text IS NOT NULL",
            params![uuid_blob(occurrence), uuid_blob(attempt)], |r| r.get(0),
        ).optional().map_err(map_err)
    }

    /// Stop future admission for this workspace. The caller must authorize the
    /// verified owner or current own-workspace orchestrator before calling.
    /// A claimed attempt may already have pasted, so it becomes uncertain;
    /// an admitted attempt vetoes this transaction with ResourceConflict.
    pub fn disable_check_in(&self, workspace: uuid::Uuid, caller_terminal: Option<uuid::Uuid>, caller_device: Option<&str>) -> farcooler_core::Result<bool> {
        use crate::error::map_err;
        use crate::models::uuid_blob;
        use rusqlite::{OptionalExtension, params};

        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(map_err)?;
        if let Some(device) = caller_device {
            // The seat lookup in the daemon may have awaited while this
            // device's authority changed. Check the relevant branch again
            // under the same write transaction as the disable.
            let authorized: bool = tx.query_row(
                "SELECT NOT EXISTS (SELECT 1 FROM released_devices WHERE client_id = ?1)
                    AND CASE WHEN ?2 THEN
                        EXISTS (SELECT 1 FROM owner_devices WHERE client_id = ?1 AND state = 'confirmed')
                        AND NOT EXISTS (SELECT 1 FROM member_devices WHERE client_id = ?1)
                        AND NOT EXISTS (SELECT 1 FROM device_caps WHERE client_id = ?1)
                    ELSE
                        NOT EXISTS (SELECT 1 FROM device_caps WHERE client_id = ?1 AND cap = 'viewer')
                        AND NOT EXISTS (SELECT 1 FROM member_devices d JOIN member_access a ON a.handle = d.handle
                                        WHERE d.client_id = ?1 AND a.access = 'viewer')
                    END", params![device, caller_terminal.is_none()], |r| r.get(0),
            ).map_err(map_err)?;
            if !authorized { return Err(farcooler_core::DomainError::Conflict { what: "person_only" }) }
        }
        if let Some(terminal) = caller_terminal {
            let current: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM terminals WHERE id = ?1 AND workspace_id = ?2 AND role = 2 AND intent = 1)",
                params![uuid_blob(terminal), uuid_blob(workspace)], |r| r.get(0),
            ).map_err(map_err)?;
            if !current { return Err(farcooler_core::DomainError::Conflict { what: "not_orchestrator" }) }
        }
        let admitted: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM check_in_enter_fences WHERE workspace_id = ?1)",
            params![uuid_blob(workspace)], |r| r.get(0),
        ).map_err(map_err)?;
        if admitted { return Err(farcooler_core::DomainError::ResourceConflict) }
        let audit_task: Option<uuid::Uuid> = tx.query_row(
            "SELECT s.task_id FROM check_in_schedules s JOIN tasks t ON t.id = s.task_id AND t.workspace_id = s.workspace_id WHERE s.workspace_id = ?1",
            params![uuid_blob(workspace)], |r| crate::models::get_uuid(r, 0),
        ).optional().map_err(map_err)?;
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
        if changed == 1
            && let Some(task) = audit_task
        {
            crate::tasks::insert_note(
                &tx, task, crate::models::NoteKind::Progress, crate::models::Actor::Runner,
                "Workspace check-ins were disabled; an admitted or uncertain key may still arrive.",
                &serde_json::json!({ "event": "check_in_disabled", "source": if caller_terminal.is_some() { "orchestrator" } else { "owner" } }), None,
            )?;
        }
        tx.commit().map_err(map_err)?;
        Ok(changed == 1)
    }
}

/// Inputs bound together when enabling a new authorized schedule generation.
pub struct EnableCheckIn<'a> {
    pub workspace: uuid::Uuid, pub task: uuid::Uuid, pub device: Option<&'a str>,
    pub expected_generation: Option<uuid::Uuid>, pub expected_occurrence: Option<uuid::Uuid>,
    pub acknowledge_uncertain: bool, pub now: i64,
}

/// Identity of the proved rollout pinned to a claimed occurrence.
pub struct PinCheckInRollout<'a> {
    pub occurrence: uuid::Uuid, pub attempt: uuid::Uuid, pub path: &'a str, pub tty: &'a str,
    pub incarnation: i64, pub device: i64, pub inode: i64, pub offset: i64,
}

impl crate::Store {
    /// Establish a new generation after the daemon verifies the person's
    /// owner identity and caller seat. An unresolved possible delivery requires
    /// an explicit acknowledgement; the caller must disclose it to the owner.
    pub fn enable_check_in(&self, input: EnableCheckIn<'_>) -> farcooler_core::Result<uuid::Uuid> {
        let EnableCheckIn { workspace, task, device, expected_generation, expected_occurrence, acknowledge_uncertain, now } = input;
        use crate::error::map_err;
        use crate::models::uuid_blob;
        use farcooler_core::DomainError;
        use rusqlite::{OptionalExtension, params};

        let next_due = now.checked_add(3_600_000).ok_or(DomainError::InvalidArgument { what: "time" })?;
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(map_err)?;
        let existing: Option<Vec<u8>> = tx.query_row(
            "SELECT generation FROM check_in_schedules WHERE workspace_id = ?1",
            params![uuid_blob(workspace)], |r| r.get(0),
        ).optional().map_err(map_err)?;
        if existing != expected_generation.map(uuid_blob) { return Err(DomainError::ResourceConflict) }
        if let Some(expected) = expected_occurrence {
            let latest: Option<Vec<u8>> = tx.query_row(
                "SELECT id FROM check_in_occurrences WHERE workspace_id = ?1 ORDER BY rowid DESC LIMIT 1",
                params![uuid_blob(workspace)], |r| r.get(0),
            ).optional().map_err(map_err)?;
            if latest != Some(uuid_blob(expected)) { return Err(DomainError::ResourceConflict) }
        }
        let valid: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM workspaces w JOIN tasks t ON t.workspace_id = w.id
             WHERE w.id = ?1 AND t.id = ?2 AND t.status NOT IN ('done', 'cancelled') AND w.wake_on_answer = 1)
             AND (?3 IS NULL OR (EXISTS (SELECT 1 FROM owner_devices WHERE client_id = ?3 AND state = 'confirmed')
                 AND NOT EXISTS (SELECT 1 FROM device_caps WHERE client_id = ?3)
                 AND NOT EXISTS (SELECT 1 FROM member_devices WHERE client_id = ?3)
                 AND NOT EXISTS (SELECT 1 FROM released_devices WHERE client_id = ?3)))",
            params![uuid_blob(workspace), uuid_blob(task), device], |r| r.get(0),
        ).map_err(map_err)?;
        if !valid { return Err(DomainError::InvalidArgument { what: "check_in_authority_or_task" }) }
        let busy: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM check_in_enter_fences WHERE workspace_id = ?1)
             OR EXISTS (SELECT 1 FROM check_in_occurrences WHERE workspace_id = ?1 AND state IN ('claimed', 'enter_admitted', 'submitted'))",
            params![uuid_blob(workspace)], |r| r.get(0),
        ).map_err(map_err)?;
        if busy { return Err(DomainError::ResourceConflict) }
        let uncertain: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM check_in_occurrences WHERE workspace_id = ?1 AND state = 'uncertain')",
            params![uuid_blob(workspace)], |r| r.get(0),
        ).map_err(map_err)?;
        if uncertain {
            let current: Vec<u8> = tx.query_row(
                "SELECT id FROM check_in_occurrences WHERE workspace_id = ?1 AND state = 'uncertain'",
                params![uuid_blob(workspace)], |r| r.get(0),
            ).map_err(map_err)?;
            if Some(current) != expected_occurrence.map(uuid_blob) { return Err(DomainError::ResourceConflict) }
            let disabled: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM check_in_schedules WHERE workspace_id = ?1 AND state = 'disabled')",
                params![uuid_blob(workspace)], |r| r.get(0),
            ).map_err(map_err)?;
            if !disabled || !acknowledge_uncertain { return Err(DomainError::Conflict { what: "check_in_uncertain" }) }
        }
        tx.execute(
            "UPDATE check_in_occurrences SET state = CASE state WHEN 'pending' THEN 'cancelled' ELSE 'resolved_uncertain' END,
                    reason = CASE state WHEN 'pending' THEN 'superseded' ELSE 'owner_acknowledged_uncertain' END
              WHERE workspace_id = ?1 AND state IN ('pending', 'uncertain')",
            params![uuid_blob(workspace)],
        ).map_err(map_err)?;
        let generation = uuid::Uuid::now_v7();
        tx.execute(
            "INSERT INTO check_in_schedules (workspace_id, task_id, generation, authorizer_device, authorizer_source, authorized_at, template_version, next_due, state)
             VALUES (?1, ?2, ?3, ?4, CASE WHEN ?4 IS NULL THEN 'local_person' ELSE 'owner_device' END, ?5, 1, ?6, 'enabled')
             ON CONFLICT(workspace_id) DO UPDATE SET task_id = excluded.task_id, generation = excluded.generation,
                 authorizer_device = excluded.authorizer_device, authorizer_source = excluded.authorizer_source, authorized_at = excluded.authorized_at,
                 template_version = excluded.template_version, next_due = excluded.next_due,
                 state = 'enabled', eligibility_epoch = check_in_schedules.eligibility_epoch + 1, reason = NULL",
            params![uuid_blob(workspace), uuid_blob(task), uuid_blob(generation), device, now, next_due],
        ).map_err(map_err)?;
        crate::tasks::insert_note(
            &tx, task, crate::models::NoteKind::Progress, crate::models::Actor::Runner,
            "A verified owner enabled fixed hourly workspace check-ins.",
            &serde_json::json!({ "event": "check_in_enabled", "generation": generation.to_string(),
                "source": if device.is_some() { "owner_device" } else { "local_person" } }), None,
        )?;
        tx.commit().map_err(map_err)?;
        Ok(generation)
    }
}

/// A submitted check-in that has no correlated transcript receipt by this
/// deadline becomes uncertain. It never starts the next hourly interval.
pub const CHECK_IN_CONFIRMATION_TIMEOUT_MS: i64 = 5 * 60 * 1000;

impl crate::Store {
    /// Mark unanswered submissions uncertain without attempting another send.
    pub fn expire_check_in_submissions(&self, now: i64) -> farcooler_core::Result<usize> {
        use crate::error::map_err;
        use rusqlite::params;

        let cutoff = now.saturating_sub(CHECK_IN_CONFIRMATION_TIMEOUT_MS);
        self.conn().execute(
            "UPDATE check_in_occurrences SET state = 'uncertain', reason = 'receipt_timeout'
              WHERE state = 'submitted' AND submitted_at IS NOT NULL AND submitted_at <= ?1",
            params![cutoff],
        ).map_err(map_err)
    }

    /// Recover only after the caller has exclusive daemon ownership. An
    /// admitted fence remains held: a crashed daemon may have left a tmux
    /// client alive, so exclusive ownership alone does not prove it cannot
    /// initiate a late send. Store::open must never perform this recovery.
    pub fn recover_check_ins_after_exclusive_start(&self) -> farcooler_core::Result<()> {
        use crate::error::map_err;

        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(map_err)?;
        tx.execute(
            "UPDATE check_in_occurrences SET state = 'uncertain', reason = 'daemon_restarted_during_attempt'
              WHERE state IN ('claimed', 'enter_admitted', 'submitted')",
            [],
        ).map_err(map_err)?;
        tx.commit().map_err(map_err)?;
        Ok(())
    }
}

/// Durable configuration and the most recent occurrence, for authorized
/// inspection. State words are internal; the client supplies human copy.
#[derive(Debug, Clone)]
pub struct CheckInStatus {
    pub workspace: uuid::Uuid,
    pub task: uuid::Uuid,
    pub generation: uuid::Uuid,
    pub authorizer_device: Option<String>,
    pub authorizer_source: String,
    pub authorized_at: i64,
    pub next_due: i64,
    pub state: String,
    pub reason: Option<String>,
    pub occurrence: Option<CheckInOccurrence>,
}

#[derive(Debug, Clone)]
pub struct CheckInOccurrence {
    pub id: uuid::Uuid,
    pub state: String,
    pub terminal: Option<uuid::Uuid>,
    pub thread: Option<String>,
    pub due_at: i64,
    pub claimed_at: Option<i64>,
    pub admitted_at: Option<i64>,
    pub submitted_at: Option<i64>,
    pub confirmed_at: Option<i64>,
    pub observation: Option<String>,
    pub reason: Option<String>,
}

impl crate::Store {
    pub fn check_in_status(&self, workspace: uuid::Uuid) -> farcooler_core::Result<Option<CheckInStatus>> {
        use crate::error::map_err;
        use crate::models::{get_uuid, uuid_blob};
        use rusqlite::{OptionalExtension, params};

        let conn = self.conn();
        let mut status = conn.query_row(
            "SELECT s.workspace_id, s.task_id, s.generation, s.authorizer_device, s.authorizer_source, s.authorized_at, s.next_due, s.state,
                    CASE WHEN EXISTS (SELECT 1 FROM check_in_cleanup_fence) THEN 'cleanup_completion_unknown'
                         WHEN EXISTS (SELECT 1 FROM check_in_target_reservations r WHERE r.workspace_id = s.workspace_id)
                         THEN 'target_change_in_progress' ELSE s.reason END
               FROM check_in_schedules s WHERE s.workspace_id = ?1",
            params![uuid_blob(workspace)],
            |r| Ok(CheckInStatus {
                workspace: get_uuid(r, 0)?, task: get_uuid(r, 1)?, generation: get_uuid(r, 2)?,
                authorizer_device: r.get(3)?, authorizer_source: r.get(4)?, authorized_at: r.get(5)?, next_due: r.get(6)?,
                state: r.get(7)?, reason: r.get(8)?, occurrence: None,
            }),
        ).optional().map_err(map_err)?;
        if let Some(status) = &mut status {
            status.occurrence = conn.query_row(
                "SELECT id, state, terminal_id, thread_id, due_at, claimed_at, admitted_at, submitted_at, confirmed_at, observation, reason
                   FROM check_in_occurrences WHERE workspace_id = ?1 ORDER BY rowid DESC LIMIT 1",
                params![uuid_blob(workspace)],
                |r| {
                    let terminal: Option<Vec<u8>> = r.get(2)?;
                    let terminal = terminal.map(|raw| uuid::Uuid::from_slice(&raw).map_err(|err|
                        rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Blob, Box::new(err)))).transpose()?;
                    Ok(CheckInOccurrence {
                        id: get_uuid(r, 0)?, state: r.get(1)?, terminal, thread: r.get(3)?, due_at: r.get(4)?,
                        claimed_at: r.get(5)?, admitted_at: r.get(6)?, submitted_at: r.get(7)?,
                        confirmed_at: r.get(8)?, observation: r.get(9)?, reason: r.get(10)?,
                    })
                },
            ).optional().map_err(map_err)?;
        }
        Ok(status)
    }
}

impl crate::Store {
    /// Pin the exact rollout identity and starting byte before Enter. The
    /// daemon obtains these from the proved Codex process and an open file;
    /// callers must not fill them from a remote request.
    pub fn pin_check_in_rollout(&self, input: PinCheckInRollout<'_>) -> farcooler_core::Result<bool> {
        let PinCheckInRollout { occurrence, attempt, path, tty, incarnation, device, inode, offset } = input;
        use crate::error::map_err;
        use crate::models::uuid_blob;
        use rusqlite::params;

        if path.is_empty() || tty.is_empty() || incarnation <= 0 || device < 0 || inode < 0 || offset < 0 { return Ok(false) }
        let changed = self.conn().execute(
            "UPDATE check_in_occurrences SET rollout_path = ?3, rollout_device = ?4, rollout_inode = ?5, rollout_offset = ?6, tty = ?7, process_incarnation = ?8
              WHERE id = ?1 AND attempt_id = ?2 AND state = 'claimed' AND rollout_path IS NULL",
            params![uuid_blob(occurrence), uuid_blob(attempt), path, device, inode, offset, tty, incarnation],
        ).map_err(map_err)?;
        Ok(changed == 1)
    }
}

impl crate::Store {
    /// Persist a receipt after a trusted daemon observer has matched the exact
    /// runner-authored message in the pinned file and session. This method
    /// verifies the evidence belongs to the exact submitted attempt; callers
    /// must perform file identity and content verification before invoking it.
    #[allow(clippy::too_many_arguments)] // Each value binds the same receipt to its attempted session and file.
    pub fn confirm_check_in_receipt(
        &self,
        occurrence: uuid::Uuid,
        attempt: uuid::Uuid,
        terminal: uuid::Uuid,
        thread: &str,
        device: i64,
        inode: i64,
        line_offset: i64,
        message_at: i64,
        observed_at: i64,
    ) -> farcooler_core::Result<bool> {
        use crate::error::map_err;
        use crate::models::uuid_blob;
        use rusqlite::{OptionalExtension, params};

        if thread.is_empty() || device < 0 || inode < 0 || line_offset < 0 { return Ok(false) }
        let Some(next_due) = observed_at.checked_add(3_600_000) else { return Ok(false) };
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(map_err)?;
        let workspace: Option<Vec<u8>> = tx.query_row(
            "SELECT workspace_id FROM check_in_occurrences
              WHERE id = ?1 AND attempt_id = ?2 AND state = 'submitted' AND terminal_id = ?3 AND thread_id = ?4
                AND rollout_device = ?5 AND rollout_inode = ?6 AND rollout_offset <= ?7
                AND admitted_at IS NOT NULL AND admitted_at <= ?8 AND ?8 <= ?10 AND submitted_at IS NOT NULL
                AND submitted_at >= ?9 AND submitted_at <= ?10",
            params![uuid_blob(occurrence), uuid_blob(attempt), uuid_blob(terminal), thread, device, inode,
                    line_offset, message_at, observed_at.saturating_sub(CHECK_IN_CONFIRMATION_TIMEOUT_MS), observed_at],
            |r| r.get(0),
        ).optional().map_err(map_err)?;
        let Some(workspace) = workspace else { return Ok(false) };
        let evidence = format!("codex_rollout_line:{line_offset};message_at:{message_at}");
        let changed = tx.execute(
            "UPDATE check_in_occurrences SET state = 'confirmed', confirmed_at = ?3, observation = ?4, reason = NULL
              WHERE id = ?1 AND attempt_id = ?2 AND state = 'submitted'",
            params![uuid_blob(occurrence), uuid_blob(attempt), observed_at, evidence],
        ).map_err(map_err)?;
        if changed != 1 { return Ok(false) }
        tx.execute(
            "UPDATE check_in_schedules SET next_due = ?2
              WHERE workspace_id = ?1 AND state = 'enabled'
                AND EXISTS (SELECT 1 FROM check_in_occurrences o WHERE o.id = ?3
                    AND o.generation = check_in_schedules.generation
                    AND o.eligibility_epoch = check_in_schedules.eligibility_epoch)",
            params![workspace, next_due, uuid_blob(occurrence)],
        ).map_err(map_err)?;
        tx.commit().map_err(map_err)?;
        Ok(true)
    }
}

/// Internal observation coordinates captured before the Enter attempt. These
/// are not proof on their own; the observer must open this exact file and
/// verify its identity, session, and message before recording a receipt.
#[derive(Debug, Clone)]
pub struct CheckInObservation {
    pub workspace: uuid::Uuid,
    pub path: String,
    pub device: i64,
    pub inode: i64,
    pub from: i64,
    pub terminal: uuid::Uuid,
    pub pane: String,
    pub tty: String,
    pub terminal_lease: i64,
    pub terminal_epoch: i64,
    pub process_id: i32,
    pub process_started: i64,
    pub process_incarnation: i64,
    pub thread: String,
    pub message: String,
    pub admitted_at: i64,
}

impl crate::Store {
    pub fn check_in_observation(
        &self,
        occurrence: uuid::Uuid,
        attempt: uuid::Uuid,
    ) -> farcooler_core::Result<Option<CheckInObservation>> {
        use crate::error::map_err;
        use crate::models::{get_uuid, uuid_blob};
        use rusqlite::{OptionalExtension, params};

        self.conn().query_row(
            "SELECT rollout_path, rollout_device, rollout_inode, rollout_offset, terminal_id, thread_id, admitted_at,
                    pane_id, terminal_lease, terminal_epoch, message_text, process_id, process_started, workspace_id, tty, process_incarnation
               FROM check_in_occurrences WHERE id = ?1 AND attempt_id = ?2 AND state = 'submitted'
                AND rollout_path IS NOT NULL AND rollout_device IS NOT NULL AND rollout_inode IS NOT NULL
                AND rollout_offset IS NOT NULL AND terminal_id IS NOT NULL AND thread_id IS NOT NULL AND admitted_at IS NOT NULL
                AND pane_id IS NOT NULL AND terminal_lease IS NOT NULL AND terminal_epoch IS NOT NULL AND message_text IS NOT NULL
                AND process_id IS NOT NULL AND process_started IS NOT NULL AND tty IS NOT NULL AND process_incarnation > 0",
            params![uuid_blob(occurrence), uuid_blob(attempt)],
            |r| Ok(CheckInObservation {
                path: r.get(0)?, device: r.get(1)?, inode: r.get(2)?, from: r.get(3)?,
                terminal: get_uuid(r, 4)?, thread: r.get(5)?, admitted_at: r.get(6)?, pane: r.get(7)?,
                terminal_lease: r.get(8)?, terminal_epoch: r.get(9)?, message: r.get(10)?, process_id: r.get(11)?, process_started: r.get(12)?, workspace: get_uuid(r, 13)?, tty: r.get(14)?, process_incarnation: r.get(15)?,
            }),
        ).optional().map_err(map_err)
    }
}

/// Immutable identity of an unclaimed scheduler occurrence. The runner must
/// still re-read eligibility and prove the live target before claiming it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingCheckIn {
    pub id: uuid::Uuid,
    pub workspace: uuid::Uuid,
    pub task: uuid::Uuid,
    pub generation: uuid::Uuid,
}

impl crate::Store {
    pub fn due_check_in_workspaces(&self, now: i64) -> farcooler_core::Result<Vec<uuid::Uuid>> {
        use crate::error::map_err;
        use crate::models::get_uuid;
        use rusqlite::params;

        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT s.workspace_id FROM check_in_schedules s WHERE s.state = 'enabled' AND s.next_due <= ?1
              AND NOT EXISTS (SELECT 1 FROM check_in_occurrences o WHERE o.workspace_id = s.workspace_id
                  AND o.state IN ('pending', 'claimed', 'enter_admitted', 'submitted', 'uncertain'))
              ORDER BY s.next_due, s.rowid",
        ).map_err(map_err)?;
        let rows = stmt.query_map(params![now], |r| get_uuid(r, 0)).map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)
    }

    /// Attempts eligible for receipt observation. They never enter the
    /// delivery queue again, even after a process restart.
    pub fn submitted_check_ins(&self) -> farcooler_core::Result<Vec<(uuid::Uuid, uuid::Uuid)>> {
        use crate::error::map_err;
        use crate::models::get_uuid;
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, attempt_id FROM check_in_occurrences WHERE state = 'submitted' AND attempt_id IS NOT NULL ORDER BY rowid",
        ).map_err(map_err)?;
        let rows = stmt.query_map([], |r| Ok((get_uuid(r, 0)?, get_uuid(r, 1)?))).map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)
    }

    pub fn pending_check_ins(&self) -> farcooler_core::Result<Vec<PendingCheckIn>> {
        use crate::error::map_err;
        use crate::models::get_uuid;

        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT o.id, o.workspace_id, o.task_id, o.generation FROM check_in_occurrences o
               JOIN check_in_schedules s ON s.workspace_id = o.workspace_id
              WHERE o.state = 'pending' AND s.state = 'enabled' AND s.generation = o.generation
                AND s.eligibility_epoch = o.eligibility_epoch ORDER BY o.due_at, o.rowid",
        ).map_err(map_err)?;
        let rows = stmt.query_map([], |r| Ok(PendingCheckIn {
            id: get_uuid(r, 0)?, workspace: get_uuid(r, 1)?, task: get_uuid(r, 2)?, generation: get_uuid(r, 3)?,
        })).map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)
    }
}


impl crate::Store {
    /// Suspend a claimed attempt when paste or a later gate is uncertain. A
    /// subsequent admission cannot use it, and it is never retried.
    pub fn fail_claimed_check_in(&self, occurrence: uuid::Uuid, attempt: uuid::Uuid) -> farcooler_core::Result<bool> {
        use crate::error::map_err;
        use crate::models::uuid_blob;
        use rusqlite::params;
        let changed = self.conn().execute(
            "UPDATE check_in_occurrences SET state = 'uncertain', reason = 'paste_or_gate_uncertain'
              WHERE id = ?1 AND attempt_id = ?2 AND state = 'claimed'",
            params![uuid_blob(occurrence), uuid_blob(attempt)],
        ).map_err(map_err)?;
        Ok(changed == 1)
    }

    /// An admitted sender failed or timed out. Keep its durable fence: a
    /// transport timeout does not prove that tmux cannot accept the key.
    pub fn fail_admitted_check_in(&self, occurrence: uuid::Uuid, attempt: uuid::Uuid) -> farcooler_core::Result<bool> {
        use crate::error::map_err;
        use crate::models::uuid_blob;
        use rusqlite::params;
        let changed = self.conn().execute(
            "UPDATE check_in_occurrences SET state = 'uncertain', reason = 'enter_outcome_unknown'
              WHERE id = ?1 AND attempt_id = ?2 AND state = 'enter_admitted'
                AND EXISTS (SELECT 1 FROM check_in_enter_fences f WHERE f.occurrence_id = ?1 AND f.attempt_id = ?2)",
            params![uuid_blob(occurrence), uuid_blob(attempt)],
        ).map_err(map_err)?;
        Ok(changed == 1)
    }
}

impl crate::Store {
    /// Reserve a runner-controlled mutation that may stop an orchestrator
    /// before its database row changes. The token lives until the caller has
    /// observed the external operation and its row update finish. An error or
    /// cancellation must leave it in place for explicit reconciliation.
    pub fn reserve_check_in_target_change(
        &self, terminal: uuid::Uuid, token: uuid::Uuid, operation: &str, process_started: Option<i64>,
    ) -> farcooler_core::Result<bool> {
        use crate::error::map_err;
        use crate::models::uuid_blob;
        use farcooler_core::DomainError;
        use rusqlite::{OptionalExtension, params};
        if !matches!(operation, "stop" | "remove") { return Err(DomainError::InvalidArgument { what: "operation" }) }
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(map_err)?;
        let terminal_row: Option<(Option<Vec<u8>>, i64)> = tx.query_row(
            "SELECT workspace_id, role FROM terminals WHERE id = ?1",
            params![uuid_blob(terminal)], |r| Ok((r.get(0)?, r.get(1)?)),
        ).optional().map_err(map_err)?;
        let Some((workspace, role)) = terminal_row else { return Err(DomainError::NotFound) };
        // Even an unassigned shell can be promoted while a stop is in
        // flight. Reserve its identity too, so it cannot become a target
        // until that external operation finishes.
        let affected_workspace = if role == 2 { workspace } else { None };
        if let Some(workspace) = &affected_workspace {
            let admitted: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM check_in_enter_fences WHERE workspace_id = ?1)",
                params![workspace], |r| r.get(0),
            ).map_err(map_err)?;
            if admitted { return Err(DomainError::ResourceConflict) }
        }
        tx.execute(
            "INSERT INTO check_in_target_reservations (token, workspace_id, terminal_id, operation, phase, process_id, process_started, created_at)
             VALUES (?1, ?2, ?3, ?4, 'reserved', ?5, ?6, ?7)",
            params![uuid_blob(token), affected_workspace, uuid_blob(terminal), operation, i64::from(std::process::id()), process_started, crate::tasks::now_millis()],
        ).map_err(map_err)?;
        tx.commit().map_err(map_err)?;
        Ok(true)
    }

    /// Record that the owned external operation returned successfully and
    /// cannot initiate another action. A failed or cancelled operation must
    /// not use this transition.
    pub fn complete_check_in_target_change(&self, token: uuid::Uuid) -> farcooler_core::Result<bool> {
        use crate::error::map_err;
        use crate::models::uuid_blob;
        let changed = self.conn().execute(
            "UPDATE check_in_target_reservations SET phase = 'completed' WHERE token = ?1 AND phase = 'reserved'",
            [uuid_blob(token)],
        ).map_err(map_err)?;
        Ok(changed == 1)
    }

    /// Keep an ambiguous operation visible; no timeout removes its hold.
    pub fn mark_check_in_target_uncertain(&self, token: uuid::Uuid) -> farcooler_core::Result<bool> {
        use crate::error::map_err;
        use crate::models::uuid_blob;
        let changed = self.conn().execute(
            "UPDATE check_in_target_reservations SET phase = 'uncertain' WHERE token = ?1 AND phase = 'reserved'",
            [uuid_blob(token)],
        ).map_err(map_err)?;
        Ok(changed == 1)
    }

    /// Owner-facing reconciliation rechecks a remote owner's binding in
    /// the same write that removes a conclusively completed token.
    pub fn reconcile_check_in_target_change(&self, token: uuid::Uuid, device: Option<&str>) -> farcooler_core::Result<bool> {
        use crate::error::map_err;
        use crate::models::uuid_blob;
        use rusqlite::params;
        let changed = self.conn().execute(
            "DELETE FROM check_in_target_reservations WHERE token = ?1 AND phase = 'completed'
             AND (?2 IS NULL OR (EXISTS (SELECT 1 FROM owner_devices WHERE client_id = ?2 AND state = 'confirmed')
                 AND NOT EXISTS (SELECT 1 FROM device_caps WHERE client_id = ?2)
                 AND NOT EXISTS (SELECT 1 FROM member_devices WHERE client_id = ?2)
                 AND NOT EXISTS (SELECT 1 FROM released_devices WHERE client_id = ?2)))",
            params![uuid_blob(token), device],
        ).map_err(map_err)?;
        Ok(changed == 1)
    }

    /// Release only the matching reservation after the external operation can
    /// no longer initiate and its final row update succeeded.
    pub fn release_check_in_target_change(&self, token: uuid::Uuid) -> farcooler_core::Result<bool> {
        use crate::error::map_err;
        use crate::models::uuid_blob;
        let changed = self.conn().execute(
            "DELETE FROM check_in_target_reservations WHERE token = ?1 AND phase = 'completed'", [uuid_blob(token)],
        ).map_err(map_err)?;
        Ok(changed == 1)
    }
}

impl crate::Store {
    /// A non-local owner caller must have a current confirmed, uncapped owner
    /// binding. The local socket is separately verified by its caller seat.
    pub fn is_check_in_owner_device(&self, device: &str) -> farcooler_core::Result<bool> {
        use crate::error::map_err;
        self.conn().query_row(
            "SELECT EXISTS (SELECT 1 FROM owner_devices WHERE client_id = ?1 AND state = 'confirmed')
                AND NOT EXISTS (SELECT 1 FROM device_caps WHERE client_id = ?1)
                AND NOT EXISTS (SELECT 1 FROM member_devices WHERE client_id = ?1)
                AND NOT EXISTS (SELECT 1 FROM released_devices WHERE client_id = ?1)",
            [device], |r| r.get(0),
        ).map_err(map_err)
    }
}

/// An unresolved or not-yet-cleaned runner target operation, including holds
/// for terminals that have no workspace or whose row has been deleted.
#[derive(Debug, Clone)]
pub struct CheckInTargetHold {
    pub token: uuid::Uuid,
    pub workspace: Option<uuid::Uuid>,
    pub terminal: uuid::Uuid,
    pub operation: String,
    pub phase: String,
    pub process_id: i64,
    pub process_started: Option<i64>,
    pub created_at: i64,
}

impl crate::Store {
    pub fn check_in_target_holds(&self, device: Option<&str>) -> farcooler_core::Result<Vec<CheckInTargetHold>> {
        use crate::error::map_err;
        use crate::models::get_uuid;
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT token, workspace_id, terminal_id, operation, phase, process_id, process_started, created_at
               FROM check_in_target_reservations
              WHERE (?1 IS NULL OR (EXISTS (SELECT 1 FROM owner_devices WHERE client_id = ?1 AND state = 'confirmed')
                 AND NOT EXISTS (SELECT 1 FROM device_caps WHERE client_id = ?1)
                 AND NOT EXISTS (SELECT 1 FROM member_devices WHERE client_id = ?1)
                 AND NOT EXISTS (SELECT 1 FROM released_devices WHERE client_id = ?1)))
              ORDER BY created_at, rowid",
        ).map_err(map_err)?;
        let rows = stmt.query_map([device], |r| {
            let workspace: Option<Vec<u8>> = r.get(1)?;
            let workspace = workspace.map(|raw| uuid::Uuid::from_slice(&raw).map_err(|error|
                rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Blob, Box::new(error)))).transpose()?;
            Ok(CheckInTargetHold {
                token: get_uuid(r, 0)?, workspace, terminal: get_uuid(r, 2)?, operation: r.get(3)?, phase: r.get(4)?,
                process_id: r.get(5)?, process_started: r.get(6)?, created_at: r.get(7)?,
            })
        }).map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)
    }
}

#[cfg(test)]
#[path = "check_ins_tests.rs"]
mod tests;
