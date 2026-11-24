use super::*;

pub(crate) const BOARD_ORDER_TABLE: &str = "task_board_positions";
pub(crate) const BOARD_ORDER_SCHEMA: &str = "CREATE TABLE task_board_positions (
    task_id BLOB PRIMARY KEY NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK(position >= 0)
)";

pub(crate) fn migration_board_order(tx: &rusqlite::Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch(BOARD_ORDER_SCHEMA)?;
    tx.execute(&format!("INSERT INTO task_board_positions(task_id, position) {LEGACY_BOARD_ORDER}"), [])?;
    Ok(())
}

// Seed old boards from their explicit lines. Agent and build lines take
// precedence over the plan when both ranked a card.
pub(crate) const LEGACY_BOARD_ORDER: &str = "
    SELECT id, ROW_NUMBER() OVER (PARTITION BY workspace_id ORDER BY
        CASE WHEN wait_rank IS NULL THEN 2 WHEN wait_line = 'agent' THEN 0 ELSE 1 END,
        COALESCE(wait_rank, plan_rank), created_at, filing_order) - 1
    FROM (
        SELECT tasks.id, tasks.workspace_id, tasks.created_at, tasks.rowid AS filing_order, tasks.wait_line,
            CASE WHEN tasks.wait_kind = 'in_line' AND (
                (tasks.wait_line = 'agent' AND tasks.status IN ('backlog', 'todo')) OR
                (tasks.wait_line = 'build' AND tasks.status IN ('in_progress', 'in_review'))
            ) THEN tasks.wait_rank END AS wait_rank,
            CASE WHEN ticket_lines.stage = 'waiting' THEN ticket_lines.plan_rank END AS plan_rank
        FROM tasks LEFT JOIN ticket_lines ON ticket_lines.task_id = tasks.id
    )
    WHERE wait_rank IS NOT NULL OR plan_rank IS NOT NULL";

// Existing and newly filed cards without an explicit position follow the
// ordered cards in filing order. A card keeps its position as its status changes.
pub(crate) const BOARD_ORDER_JOIN: &str = "LEFT JOIN task_board_positions ON task_board_positions.task_id = tasks.id";
pub(crate) const BOARD_ORDER: &str = "position IS NULL, position, tasks.created_at, tasks.rowid";

fn positions(conn: &rusqlite::Connection, workspace: &[u8]) -> Result<Vec<(Uuid, Option<i64>)>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT id, position FROM tasks {BOARD_ORDER_JOIN} WHERE workspace_id = ?1 ORDER BY {BOARD_ORDER}"
    )).map_err(map_err)?;
    let rows = stmt.query_map([workspace], |row| Ok((get_uuid(row, 0)?, row.get(1)?))).map_err(map_err)?;
    rows.collect::<rusqlite::Result<_>>().map_err(map_err)
}

fn write_positions(tx: &rusqlite::Transaction<'_>, order: Vec<(Uuid, Option<i64>)>, through: usize) -> Result<()> {
    let ranked = order.iter().rposition(|(_, rank)| rank.is_some()).map_or(0, |i| i + 1);
    for (index, (id, previous)) in order.into_iter().take(ranked.max(through)).enumerate() {
        let position = index as i64;
        if previous != Some(position) {
            tx.execute("INSERT INTO task_board_positions(task_id, position) VALUES (?1, ?2)
                ON CONFLICT(task_id) DO UPDATE SET position = excluded.position",
                params![uuid_blob(id), position]).map_err(map_err)?;
        }
    }
    Ok(())
}

/// Move an already validated list to the front of the board, preserving the
/// relative order of all other cards.
pub(crate) fn prioritize_board_tasks(tx: &rusqlite::Transaction<'_>, workspace: Uuid, tasks: &[Uuid]) -> Result<()> {
    if tasks.is_empty() { return Ok(()); }
    let mut order = positions(tx, &uuid_blob(workspace))?;
    let ranks: std::collections::HashMap<_, _> = tasks.iter().enumerate().map(|(index, id)| (*id, index)).collect();
    order.sort_by_key(|(id, _)| ranks.get(id).copied().unwrap_or(usize::MAX));
    write_positions(tx, order, tasks.len())
}

impl Store {
    /// The board's order by key, without loading card bodies or workflow state.
    pub fn board_task_keys(&self, workspace: Uuid) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT key FROM tasks {BOARD_ORDER_JOIN} WHERE workspace_id = ?1 ORDER BY {BOARD_ORDER}"
        )).map_err(map_err)?;
        let keys = stmt.query_map([uuid_blob(workspace)], |row| row.get(0)).map_err(map_err)?;
        keys.collect::<rusqlite::Result<_>>().map_err(map_err)
    }

    /// Place a card relative to another within the same board. Work against the
    /// committed order so concurrent moves never overwrite a stale list.
    pub fn reorder_task(&self, task: Uuid, anchor: Uuid, after: bool) -> Result<bool> {
        if task == anchor {
            return Err(DomainError::InvalidArgument { what: "anchor" });
        }
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(map_err)?;
        let workspace: Vec<u8> = tx.query_row(
            "SELECT workspace_id FROM tasks WHERE id = ?1", [uuid_blob(task)], |row| row.get(0)
        ).map_err(map_err)?;
        let mut order = positions(&tx, &workspace)?;
        let from = order.iter().position(|(id, _)| *id == task).ok_or(DomainError::NotFound)?;
        let moved = order.remove(from);
        let at = order.iter().position(|(id, _)| *id == anchor)
            .ok_or(DomainError::InvalidArgument { what: "anchor must be on the same board" })? + usize::from(after);
        if from == at { return Ok(false); }
        order.insert(at, moved);
        // Preserve the unplaced suffix. Only rows whose persisted position
        // differs need a write, and no status or run ownership changes.
        write_positions(&tx, order, at + 1)?;
        tx.commit().map_err(map_err)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ticket_lines::NewTicketLine;
    use crate::waits::TaskLine;

    #[test]
    fn board_order_survives_reopen_and_status_moves() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("board.sqlite");
        let store = Store::open(&path).unwrap();
        let repo = store.register_repository_for_test("ordered");
        let workspace = store.ensure_main_workspace(repo).unwrap().id;
        let first = store.create_task(workspace, "First", Actor::User).unwrap();
        let middle = store.create_task(workspace, "Middle", Actor::User).unwrap();
        let last = store.create_task(workspace, "Last", Actor::User).unwrap();
        let before = store.get_task(last.id).unwrap();
        assert!(store.reorder_task(last.id, first.id, false).unwrap());
        assert!(!store.reorder_task(last.id, first.id, false).unwrap());
        assert_eq!(store.get_task(last.id).unwrap(), before);
        let other = store.create_workspace(repo, "Another", "an").unwrap().id;
        let outside = store.create_task(other, "Outside", Actor::User).unwrap();
        assert!(store.reorder_task(first.id, outside.id, false).is_err());

        for id in [last.id, first.id] {
            store.set_task_status(id, TaskStatus::Todo, Actor::User).unwrap();
        }
        drop(store);
        let store = Store::open(&path).unwrap();
        let new = store.create_task(workspace, "New", Actor::User).unwrap();
        assert_eq!(store.board_task_keys(workspace).unwrap(), [last.key, first.key, middle.key, new.key]);
        let listed = |status| store.list_tasks(TaskScope::Workspace(workspace), Some(status)).unwrap()
            .into_iter().map(|task| task.id).collect::<Vec<_>>();
        assert_eq!(listed(TaskStatus::Todo), [last.id, first.id]);
        assert_eq!(listed(TaskStatus::Backlog), [middle.id, new.id]);
        assert!(store.reorder_task(middle.id, new.id, true).unwrap());
        assert_eq!(listed(TaskStatus::Backlog), [new.id, middle.id]);
    }

    #[test]
    fn line_commands_and_relative_moves_share_the_order() {
        let store = Store::open_in_memory().unwrap();
        let repo = store.register_repository_for_test("board");
        let workspace = store.ensure_main_workspace(repo).unwrap().id;
        let [a, b, c] = ["a", "b", "c"].map(|title| store.create_task(workspace, title, Actor::User).unwrap());
        for task in [&a, &b, &c] {
            store.put_on_line(task.id, &NewTicketLine::default(), Actor::User).unwrap();
        }
        store.set_line(workspace, TaskLine::Agent, &[c.id, b.id], Actor::User).unwrap();
        assert_eq!(store.board_task_keys(workspace).unwrap(), [&c.key, &b.key, &a.key].map(String::as_str));
        store.set_line_order(workspace, &[b.id, c.id], Actor::User).unwrap();
        let facts = |id| store.task_facts(&[store.get_task(id).unwrap()]).unwrap().remove(0);
        assert_eq!(facts(c.id).ahead, [b.key.clone()]);
        assert_eq!(store.ticket_line(b.id).unwrap().plan_rank, Some(1));
        store.reorder_task(c.id, b.id, false).unwrap();
        assert_eq!(facts(b.id).ahead, [c.key.clone()]);
        assert_eq!(store.ticket_line(c.id).unwrap().plan_rank, Some(1));
        store.set_line(workspace, TaskLine::Agent, &[], Actor::User).unwrap();
        store.set_line_order(workspace, &[], Actor::User).unwrap();
        assert_eq!(store.board_task_keys(workspace).unwrap(), [&c.key, &b.key, &a.key].map(String::as_str));
    }

    #[test]
    fn migration_seeds_legacy_waits_and_plan_without_changing_membership() {
        let store = Store::open_in_memory().unwrap();
        let repo = store.register_repository_for_test("board");
        let workspace = store.ensure_main_workspace(repo).unwrap().id;
        let [a, b, c, d, e] = ["a", "b", "c", "d", "e"].map(|title| store.create_task(workspace, title, Actor::User).unwrap());
        for task in [&b, &c, &d] {
            store.put_on_line(task.id, &NewTicketLine::default(), Actor::User).unwrap();
        }
        store.set_task_status(e.id, TaskStatus::InProgress, Actor::User).unwrap();
        {
            let mut conn = store.conn();
            let tx = conn.transaction().unwrap();
            tx.execute("UPDATE ticket_lines SET plan_rank = 1 WHERE task_id = ?1", [uuid_blob(b.id)]).unwrap();
            tx.execute("UPDATE ticket_lines SET plan_rank = 2 WHERE task_id = ?1", [uuid_blob(c.id)]).unwrap();
            tx.execute("UPDATE ticket_lines SET plan_rank = 3 WHERE task_id = ?1", [uuid_blob(d.id)]).unwrap();
            for (task, line, rank) in [(&c, "agent", 2), (&d, "agent", 1), (&e, "build", 1)] {
                tx.execute("UPDATE tasks SET wait_kind = 'in_line', wait_line = ?2, wait_rank = ?3, wait_since = 1 WHERE id = ?1",
                    params![uuid_blob(task.id), line, rank]).unwrap();
            }
            tx.execute_batch("DROP TABLE task_board_positions").unwrap();
            migration_board_order(&tx).unwrap();
            tx.commit().unwrap();
        }
        assert_eq!(store.board_task_keys(workspace).unwrap(), [&d.key, &c.key, &e.key, &b.key, &a.key].map(String::as_str));
        assert_eq!(store.ticket_line(d.id).unwrap().plan_rank, Some(1));
        assert_eq!(store.ticket_line(b.id).unwrap().plan_rank, Some(3));
        let facts = store.task_facts(&[store.get_task(c.id).unwrap(), store.get_task(e.id).unwrap()]).unwrap();
        assert_eq!((facts[0].position, facts[1].position), (2, 1));
    }
}
