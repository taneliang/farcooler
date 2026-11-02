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
    )
}
