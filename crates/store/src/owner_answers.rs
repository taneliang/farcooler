//! The owner's answers on a board, newest first (ov-974, M5 of the second
//! review): how the daemon finds a cutover grant. A plain query on the
//! notes, not the full-text index, so whether a grant admits never rests on
//! how a tokenizer splits "Allow cutover once".

use rusqlite::params;
use uuid::Uuid;

use crate::error::map_err;
use crate::models::{TaskNote, row_to_task_note, uuid_blob};
use crate::store::Store;
use farcooler_core::Result;

impl Store {
    /// Every answer on a card of `workspace` that claims the person (`user`),
    /// written at or after `since_ms` and corrected by no later note, newest
    /// first.
    pub fn owner_answers_since(&self, workspace: Uuid, since_ms: i64) -> Result<Vec<TaskNote>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT n.id, n.task_id, n.kind, n.actor, n.at, n.body, n.extra, n.supersedes
                   FROM task_notes n JOIN tasks t ON t.id = n.task_id
                  WHERE t.workspace_id = ?1 AND n.kind = 'answer' AND n.actor = 'user' AND n.at >= ?2
                    AND NOT EXISTS (SELECT 1 FROM task_notes s WHERE s.supersedes = n.id)
                  ORDER BY n.at DESC, n.rowid DESC",
            )
            .map_err(map_err)?;
        let rows = stmt.query_map(params![uuid_blob(workspace), since_ms], row_to_task_note).map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)
    }
}

#[cfg(test)]
mod tests {
    use crate::models::{Actor, NoteKind};
    use crate::store::Store;

    /// Newest first, on this board only, a person's only, and a corrected
    /// answer gives way to its correction.
    #[test]
    fn the_owners_answers_newest_first() {
        let store = Store::open_in_memory().unwrap();
        let repo = store.register_repository_for_test("demo");
        let workspace = store.ensure_main_workspace(repo).unwrap().id;
        let other = store.create_workspace(repo, "Other", "oth").unwrap().id;
        let task = store.create_task(workspace, "Data: a card", Actor::Manager).unwrap().id;
        let elsewhere = store.create_task(other, "Data: elsewhere", Actor::Manager).unwrap().id;
        let first = store.add_note(task, NoteKind::Answer, Actor::User, "Allow cutover once", serde_json::json!({})).unwrap();
        store.add_note(task, NoteKind::Answer, Actor::Manager, "Allow cutover once", serde_json::json!({})).unwrap();
        store.add_note(elsewhere, NoteKind::Answer, Actor::User, "Allow cutover once", serde_json::json!({})).unwrap();
        store.add_note(task, NoteKind::Comment, Actor::User, "Allow cutover once", serde_json::json!({})).unwrap();
        let second = store.add_note(task, NoteKind::Answer, Actor::User, "Wait for me", serde_json::json!({})).unwrap();
        let ids: Vec<_> = store.owner_answers_since(workspace, 0).unwrap().into_iter().map(|n| n.id).collect();
        assert_eq!(ids, [second.id, first.id]);
        let fixed = store.add_note_superseding(task, NoteKind::Answer, Actor::User, "Allow cutover once", serde_json::json!({}), second.id).unwrap();
        let ids: Vec<_> = store.owner_answers_since(workspace, 0).unwrap().into_iter().map(|n| n.id).collect();
        assert_eq!(ids, [fixed.id, first.id]);
        assert!(store.owner_answers_since(workspace, i64::MAX).unwrap().is_empty());
    }
}
