//! Messages a native composer sent that the runner holds until claude can
//! take them (ov-468), kept across a restart as answer wakes are.
//!
//! A row is one held message, as the request carried it: its text, its
//! images, and the paths of the files already written on the runner. The
//! daemon's teller (`answer_wake::compose::held`) tells a pane's rows oldest
//! first, claiming one before typing it, so a restart in the middle finds it
//! claimed and never types it twice; it deletes a row once it's told or
//! given up. A pane's rows go with it by cascade.

use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

use farcooler_core::Result;

use crate::error::map_err;
use crate::models::{get_uuid, uuid_blob};
use crate::store::Store;
use crate::tasks::now_millis;

/// Two new tables, so a build from before them reads every table it knows
/// exactly as it did; one that runs meanwhile holds nothing, as before.
pub(crate) fn migration_0045_compose_holds(tx: &rusqlite::Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
        CREATE TABLE compose_holds (
            seq INTEGER PRIMARY KEY AUTOINCREMENT,
            id BLOB NOT NULL UNIQUE,
            terminal_id BLOB NOT NULL REFERENCES terminals(id) ON DELETE CASCADE,
            text TEXT NOT NULL,
            files TEXT NOT NULL,
            enqueued_at INTEGER NOT NULL,
            claimed_at INTEGER
        );
        CREATE INDEX compose_holds_by_terminal ON compose_holds (terminal_id, seq);
        CREATE TABLE compose_hold_images (
            hold_seq INTEGER NOT NULL REFERENCES compose_holds(seq) ON DELETE CASCADE,
            ord INTEGER NOT NULL,
            mime TEXT NOT NULL,
            bytes BLOB NOT NULL,
            PRIMARY KEY (hold_seq, ord)
        );
        "#,
    )
}

/// One held message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeHold {
    pub id: Uuid,
    pub terminal: Uuid,
    pub text: String,
    /// Each image's claimed MIME type and bytes.
    pub images: Vec<(String, Vec<u8>)>,
    /// The files written on the runner, by path.
    pub files: Vec<String>,
    /// Unix milliseconds.
    pub enqueued_at: i64,
    /// When typing it began, if it did: it may have reached the agent.
    pub claimed_at: Option<i64>,
}

impl Store {
    /// Hold a message for `terminal`, behind any held already.
    pub fn hold_compose(&self, terminal: Uuid, text: &str, images: &[(String, Vec<u8>)], files: &[String]) -> Result<ComposeHold> {
        let id = Uuid::now_v7();
        let at = now_millis();
        let files_json = serde_json::to_string(files).unwrap_or_else(|_| "[]".into());
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        tx.execute(
            "INSERT INTO compose_holds (id, terminal_id, text, files, enqueued_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![uuid_blob(id), uuid_blob(terminal), text, files_json, at],
        )
        .map_err(map_err)?;
        let seq = tx.last_insert_rowid();
        for (ord, (mime, bytes)) in images.iter().enumerate() {
            tx.execute(
                "INSERT INTO compose_hold_images (hold_seq, ord, mime, bytes) VALUES (?1, ?2, ?3, ?4)",
                params![seq, ord as i64, mime, bytes],
            )
            .map_err(map_err)?;
        }
        tx.commit().map_err(map_err)?;
        Ok(ComposeHold {
            id,
            terminal,
            text: text.to_string(),
            images: images.to_vec(),
            files: files.to_vec(),
            enqueued_at: at,
            claimed_at: None,
        })
    }

    /// The oldest message held for `terminal`.
    pub fn first_compose_hold(&self, terminal: Uuid) -> Result<Option<ComposeHold>> {
        let conn = self.conn();
        let head = conn
            .query_row(
                "SELECT seq, id, text, files, enqueued_at, claimed_at FROM compose_holds
                 WHERE terminal_id = ?1 ORDER BY seq LIMIT 1",
                params![uuid_blob(terminal)],
                |r| {
                    let files: String = r.get(3)?;
                    Ok((r.get::<_, i64>(0)?, get_uuid(r, 1)?, r.get::<_, String>(2)?, files, r.get(4)?, r.get(5)?))
                },
            )
            .optional()
            .map_err(map_err)?;
        let Some((seq, id, text, files, enqueued_at, claimed_at)) = head else { return Ok(None) };
        let mut stmt = conn.prepare("SELECT mime, bytes FROM compose_hold_images WHERE hold_seq = ?1 ORDER BY ord").map_err(map_err)?;
        let images = stmt
            .query_map(params![seq], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?)))
            .map_err(map_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(map_err)?;
        let files = serde_json::from_str(&files).unwrap_or_default();
        Ok(Some(ComposeHold { id, terminal, text, images, files, enqueued_at, claimed_at }))
    }

    /// Whether any message is held for `terminal`.
    pub fn holds_compose(&self, terminal: Uuid) -> Result<bool> {
        self.conn()
            .query_row("SELECT EXISTS (SELECT 1 FROM compose_holds WHERE terminal_id = ?1)", params![uuid_blob(terminal)], |r| r.get(0))
            .map_err(map_err)
    }

    /// Every terminal with a message held, for a teller to resume after a restart.
    pub fn compose_hold_terminals(&self) -> Result<Vec<Uuid>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT DISTINCT terminal_id FROM compose_holds").map_err(map_err)?;
        let ids = stmt.query_map([], |r| get_uuid(r, 0)).map_err(map_err)?.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)?;
        Ok(ids)
    }

    /// Mark `id` as being typed now, or (`false`) not any more: it waits again.
    pub fn claim_compose_hold(&self, id: Uuid, claimed: bool) -> Result<()> {
        let at = claimed.then(now_millis);
        self.conn()
            .execute("UPDATE compose_holds SET claimed_at = ?2 WHERE id = ?1", params![uuid_blob(id), at])
            .map(|_| ())
            .map_err(map_err)
    }

    /// `id` told or given up: gone, its images with it.
    pub fn drop_compose_hold(&self, id: Uuid) -> Result<()> {
        self.conn().execute("DELETE FROM compose_holds WHERE id = ?1", params![uuid_blob(id)]).map(|_| ()).map_err(map_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Held in order with their images and files, claimed and unclaimed, and
    /// gone with their pane.
    #[test]
    fn holds_keep_their_order_and_their_parts() {
        let store = Store::open_in_memory().unwrap();
        let host = Uuid::now_v7();
        let root = store.create_repository_root(host, "/r", 1_000).unwrap();
        let repo = store.create_repository(host, root.id, "name", "/gitdir", "origin").unwrap();
        let ws = store.create_worktree(repo.id, "feature/x", "/r", false).unwrap();
        let pane = store.create_terminal(ws.id, "Agent", "claude", farcooler_protocol::v1::TerminalIntent::Running, 120, 40).unwrap();
        let terminal = pane.id;
        assert_eq!(store.first_compose_hold(terminal).unwrap(), None);
        let png = ("image/png".to_string(), vec![1u8, 2, 3]);
        let first = store.hold_compose(terminal, "first", std::slice::from_ref(&png), &["/r/a.txt".into()]).unwrap();
        store.hold_compose(terminal, "second", &[], &[]).unwrap();
        assert!(store.holds_compose(terminal).unwrap());
        assert_eq!(store.compose_hold_terminals().unwrap(), [terminal]);
        let head = store.first_compose_hold(terminal).unwrap().unwrap();
        assert_eq!((head.text.as_str(), head.images, head.files), ("first", vec![png], vec!["/r/a.txt".to_string()]));
        store.claim_compose_hold(first.id, true).unwrap();
        assert!(store.first_compose_hold(terminal).unwrap().unwrap().claimed_at.is_some());
        store.claim_compose_hold(first.id, false).unwrap();
        assert!(store.first_compose_hold(terminal).unwrap().unwrap().claimed_at.is_none());
        store.drop_compose_hold(first.id).unwrap();
        assert_eq!(store.first_compose_hold(terminal).unwrap().unwrap().text, "second");
        store.delete_terminal(terminal, pane.resource_version).unwrap();
        assert!(!store.holds_compose(terminal).unwrap());
        let images: i64 = store.conn().query_row("SELECT count(*) FROM compose_hold_images", [], |r| r.get(0)).unwrap();
        assert_eq!(images, 0);
    }
}
