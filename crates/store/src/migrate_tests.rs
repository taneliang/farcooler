//! Migration tests moved out of migrate.rs, to keep it inside its size
//! ceiling (ov-169).

use rusqlite::Connection;

use super::{MIGRATIONS, migrate, migrate_only_to};

fn open() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT);").unwrap();
    conn
}

/// One Main per repository, held by the schema.
#[test]
fn a_repository_has_one_main() {
    let mut conn = open();
    migrate(&mut conn, 0).unwrap();
    conn.execute_batch(
        "INSERT INTO repository_roots VALUES (x'01', x'02', '/r', 0, 1);
         INSERT INTO repositories VALUES (x'03', x'02', x'01', 'r', '/r/.git', '', 1, '');
         INSERT INTO workspaces (id, repository_id, name, task_prefix, is_main, ordinal, created_at)
             VALUES (x'04', x'03', 'Main', 'r', 1, 0, 0);",
    )
    .unwrap();
    let second = conn.execute(
        "INSERT INTO workspaces (id, repository_id, name, task_prefix, is_main, ordinal, created_at)
             VALUES (x'05', x'03', 'Main', 'r2', 1, 1, 0)",
        [],
    );
    assert!(second.is_err(), "a second Main is refused");
}

/// A terminal from before web panes gains the column and reads as no page
/// (ov-435): NULL, which every older pane is.
#[test]
fn a_terminal_from_before_web_panes_has_no_page() {
    let before = MIGRATIONS
        .iter()
        .position(|(m, _)| std::ptr::fn_addr_eq(*m, crate::web_panes::migration_0040_web_url as super::Migration))
        .expect("the migration is registered") as u32;
    let mut conn = open();
    migrate_only_to(&mut conn, before);
    conn.execute_batch(
        "INSERT INTO repository_roots VALUES (x'01', x'02', '/r', 0, 1);
         INSERT INTO repositories VALUES (x'03', x'02', x'01', 'r', '/r/.git', '', 1, 'r');
         INSERT INTO worktrees (id, repository_id, branch, worktree_path, hidden, creation_failed, resource_version)
             VALUES (x'05', x'03', 'main', '/r', 0, 0, 1);
         INSERT INTO terminals (id, worktree_id, title, command_preset, intent, runtime_confirmed,
             lease_generation, epoch, \"columns\", \"rows\", resource_version)
             VALUES (x'06', x'05', 'old', 'shell', 1, 0, 0, 0, 80, 24, 1);",
    )
    .unwrap();

    migrate(&mut conn, before).unwrap();

    let url: Option<String> = conn
        .query_row("SELECT web_url FROM terminals WHERE id = x'06'", [], |r| r.get(0))
        .expect("the old terminal is still there, with the column");
    assert_eq!(url, None, "an old pane has no page");
}

/// A database from before a terminal recorded where it came from gains
/// the column, and its terminals read as unknown: NULL, never a guess.
#[test]
fn a_database_from_before_splits_were_recorded_reads_every_terminal_as_unknown() {
    let mut conn = open();
    migrate_only_to(&mut conn, 16);
    conn.execute_batch(
        "INSERT INTO repository_roots VALUES (x'01', x'02', '/r', 0, 1);
         INSERT INTO repositories VALUES (x'03', x'02', x'01', 'r', '/r/.git', '', 1, 'r');
         INSERT INTO worktrees (id, repository_id, branch, worktree_path, hidden, creation_failed, resource_version)
             VALUES (x'05', x'03', 'main', '/r', 0, 0, 1);
         INSERT INTO terminals (id, worktree_id, title, command_preset, intent, runtime_confirmed,
             lease_generation, epoch, \"columns\", \"rows\", resource_version)
             VALUES (x'06', x'05', 'old', 'shell', 1, 0, 0, 0, 80, 24, 1);",
    )
    .unwrap();

    migrate(&mut conn, 16).unwrap();

    let split_of: Option<Vec<u8>> = conn
        .query_row("SELECT split_of FROM terminals WHERE id = x'06'", [], |r| r.get(0))
        .expect("the old terminal is still there, with the column");
    assert_eq!(split_of, None, "an old row's origin is unknown");
}

/// A split recorded before the source's role was gains the column, and
/// reads as not known: NULL, never a guess either way.
#[test]
fn a_split_from_before_the_sources_role_was_recorded_reads_as_unknown() {
    let mut conn = open();
    migrate_only_to(&mut conn, 17);
    conn.execute_batch(
        "INSERT INTO repository_roots VALUES (x'01', x'02', '/r', 0, 1);
         INSERT INTO repositories VALUES (x'03', x'02', x'01', 'r', '/r/.git', '', 1, 'r');
         INSERT INTO worktrees (id, repository_id, branch, worktree_path, hidden, creation_failed, resource_version)
             VALUES (x'05', x'03', 'main', '/r', 0, 0, 1);
         INSERT INTO terminals (id, worktree_id, title, command_preset, intent, runtime_confirmed,
             lease_generation, epoch, \"columns\", \"rows\", resource_version, split_of)
             VALUES (x'06', x'05', 'old', 'shell', 1, 0, 0, 0, 80, 24, 1, x'07');",
    )
    .unwrap();

    migrate(&mut conn, 17).unwrap();

    let (split_of, from_orchestrator): (Option<Vec<u8>>, Option<i64>) = conn
        .query_row("SELECT split_of, split_of_orchestrator FROM terminals WHERE id = x'06'", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .expect("the old split is still there, with the column");
    assert_eq!(split_of, Some(vec![7]), "its origin stands");
    assert_eq!(from_orchestrator, None, "whether that was the orchestrator isn't known");
}
