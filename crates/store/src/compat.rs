//! Which builds may open which databases (ov-143).
//!
//! A build that finds a database at a NEWER schema than its own refuses it
//! (`DomainError::NewerData`) unless the build that wrote it vouched for it:
//! every migration says whether older code may read past it (`Older`), and
//! the oldest schema that may (`COMPATIBLE_DOWN_TO`) is stamped into `meta`
//! as `compatible_down_to` beside `schema_version`. `DatabaseSchema` and
//! `read_schema` let a CLI ask the same question of a file before it replaces
//! the daemon that owns it.

use std::path::Path;

use farcooler_core::{DomainError, Result};
use rusqlite::{Connection, OptionalExtension};

use crate::error::map_err;
use crate::migrate::{CURRENT_SCHEMA_VERSION, MIGRATIONS, read_schema_version};

/// Whether code written before a migration can run against the schema after
/// it.
///
/// `Welcome` is for a migration an older build cannot tell happened: a new
/// table it never touches, or a column it never names that has a default.
/// Anything else is `Refused`: a new trigger or constraint the old code's
/// writes would trip, a column it would leave empty that newer code relies
/// on, a table it reads that was dropped or reshaped. When in doubt it is
/// `Refused`, which costs a person an update; the other mistake costs them
/// their data.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Older {
    Refused,
    Welcome,
}

/// The oldest schema whose build may open a database at this one.
///
/// Stamped into `meta` as `compatible_down_to` beside `schema_version`, and
/// read by an OLDER build that finds a schema newer than its own: it opens the
/// database only if its own version is at least this. So a downgrade across
/// migrations that are all `Welcome` keeps working, and one across any
/// `Refused` migration is refused with `DomainError::NewerData`.
///
/// Counted from the newest migration back, stopping at the first `Refused`
/// one, rather than written as a number: a number set once for one
/// compatible migration would go on vouching for whatever came after it.
pub(crate) const COMPATIBLE_DOWN_TO: u32 = {
    let mut v = MIGRATIONS.len();
    while v > 0 && matches!(MIGRATIONS[v - 1].1, Older::Welcome) {
        v -= 1;
    }
    v as u32
};

/// The `compatible_down_to` a newer build stamped, or `None` where no build
/// did: every database written before the marker existed, which therefore
/// vouches for no older build at all.
pub(crate) fn read_compatible_down_to(conn: &Connection) -> farcooler_core::Result<Option<u32>> {
    read_meta_u32(conn, "compatible_down_to")
}

pub(crate) fn read_meta_u32(conn: &Connection, key: &str) -> farcooler_core::Result<Option<u32>> {
    let raw: Option<String> = conn
        .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
        .optional()
        .map_err(map_err)?;
    Ok(raw.and_then(|s| s.parse().ok()))
}

/// Record `COMPATIBLE_DOWN_TO` for a database at this build's schema, if it
/// doesn't say so already.
///
/// Read first so an ordinary open, which every `--stdio` and `--stream`
/// process does, writes nothing.
pub(crate) fn stamp_compatible_down_to(conn: &Connection) -> farcooler_core::Result<()> {
    if read_compatible_down_to(conn)? == Some(COMPATIBLE_DOWN_TO) {
        return Ok(());
    }
    conn.execute(
        "INSERT INTO meta (key, value) VALUES ('compatible_down_to', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [COMPATIBLE_DOWN_TO.to_string()],
    )
    .map_err(map_err)?;
    Ok(())
}


/// What a database says about which builds may open it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DatabaseSchema {
    /// The schema it is at. Zero for a file no build has written yet.
    pub version: u32,
    /// The oldest schema whose build may open it, as the build that wrote it
    /// stamped it. `None` from a build older than the marker, which vouches
    /// for no older build.
    pub compatible_down_to: Option<u32>,
}

impl DatabaseSchema {
    /// Whether THIS build opens it: anything at or below its own schema, and
    /// a newer one only where the newer build vouched for this one.
    pub fn opens_here(&self) -> bool {
        let ours = CURRENT_SCHEMA_VERSION;
        self.version <= ours || self.compatible_down_to.is_some_and(|floor| floor <= ours)
    }

    /// Whether it was written by a build newer than this one, whether or not
    /// this one may still open it. The direction a "replace that daemon with
    /// this build" must never go unasked: it is a downgrade.
    pub fn newer_than_here(&self) -> bool {
        self.version > CURRENT_SCHEMA_VERSION
    }

    /// The schema this build writes.
    pub fn here() -> u32 {
        CURRENT_SCHEMA_VERSION
    }
}

/// Read a database file's schema without opening it as a store: read-only,
/// nothing migrated, nothing stamped, and `None` where there is no file.
///
/// For deciding, BEFORE replacing a running daemon with this build, whether
/// this build could serve that daemon's data at all
/// (`daemon_link::ensure_local`). Safe beside the daemon that owns the file:
/// a read-only connection takes no write lock.
pub fn read_schema(path: impl AsRef<Path>) -> Result<Option<DatabaseSchema>> {
    let path = path.as_ref();
    if !path.exists() {
        return Ok(None);
    }
    let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(map_err)?;
    let has_meta: bool = conn
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'meta')",
            [],
            |r| r.get(0),
        )
        .map_err(map_err)?;
    if !has_meta {
        return Ok(Some(DatabaseSchema { version: 0, compatible_down_to: None }));
    }
    Ok(Some(DatabaseSchema {
        version: read_schema_version(&conn)?,
        compatible_down_to: read_compatible_down_to(&conn)?,
    }))
}

/// `Store::init` for a database whose `schema_version` is above this
/// build's: open it only where the build that wrote it vouched for this one.
pub(crate) fn open_newer(conn: &Connection, current: u32) -> Result<()> {
    // Written by a newer build. This build's code has never seen that
    // schema, and running it there is how a rollback quietly breaks
    // things: a newer trigger refusing writes this code thinks are
    // fine, a constraint it doesn't know to satisfy. Only the newer
    // build can say it's safe, and it says so in `compatible_down_to`.
    // A database that says nothing vouches for nobody.
    let schema = DatabaseSchema {
        version: current,
        compatible_down_to: read_compatible_down_to(conn)?,
    };
    if schema.opens_here() {
        return Ok(());
    }
    tracing::error!(
        database = current,
        this_build = CURRENT_SCHEMA_VERSION,
        "the database is at a newer schema than this build knows; refusing to open it"
    );
    Err(DomainError::NewerData)
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::Store;

    /// Which migrations an older build may read past, pinned (ov-143).
    ///
    /// Each was checked for what an older build's code would meet: a column
    /// it never names with a default or allowing NULL, an index, or a table it
    /// never touches whose rows cascade with their task. Nothing here adds a
    /// trigger or a constraint an old write could trip, drops anything, or
    /// rewrites data. Changing a marking is a decision about every runner's
    /// downgrade, so it has to change this list too.
    #[test]
    fn the_migrations_older_builds_may_read_past() {
        let welcome: Vec<usize> = MIGRATIONS
            .iter()
            .enumerate()
            .filter(|(_, (_, older))| *older == Older::Welcome)
            .map(|(i, _)| i + 1)
            .collect();
        assert_eq!(welcome, vec![4, 11, 13, 16, 17, 18, 20, 22, 23]);
        // 0019's runner-written notes and 0021's `wait` and `worker` notes
        // are additive, but fail an older build's note decoder (see
        // `waits::migration_0021_waits_and_workers`), so 0021 is the floor.
        // 0022's tables (ov-113) are ones no older build touches, so it
        // moves nothing, and nor do 0023's (ov-268), which only the plan
        // layer touches.
        assert_eq!(COMPATIBLE_DOWN_TO, 21);
    }

    /// The newest schema main writes is ov-194's agent-turn tables, and this
    /// build counts them as its own: a file main left at that schema opens
    /// and migrates on to this build's, and only the schema after this
    /// build's is "newer".
    ///
    /// The file is built from migrations 0001-0019 plus 0020 by name, not
    /// from `MIGRATIONS`, so it is main's schema whatever this list says. If
    /// 0020 drops out of the list, this build reruns a migration on main's
    /// file; if it moves, the 20th migration is not ov-194's.
    #[test]
    fn main_s_newest_schema_opens_here_and_the_one_after_it_is_refused() {
        type Step = fn(&rusqlite::Transaction) -> rusqlite::Result<()>;
        let ov_194 = crate::usage::migration_0020_agent_turns as Step;
        assert!(
            std::ptr::fn_addr_eq(MIGRATIONS[19].0, ov_194),
            "the 20th migration is ov-194's agent turns"
        );

        let dir = std::env::temp_dir().join(format!("farcooler-main20-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("store.db");
        {
            let mut conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "PRAGMA foreign_keys = ON; PRAGMA recursive_triggers = ON; \
                 CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);",
            )
            .unwrap();
            let tx = conn.transaction().unwrap();
            for (m, _) in &MIGRATIONS[..19] {
                m(&tx).unwrap();
            }
            ov_194(&tx).unwrap();
            // Main has no marker: it vouches for nobody.
            tx.execute("INSERT INTO meta (key, value) VALUES ('schema_version', '20')", []).unwrap();
            tx.commit().unwrap();
        }

        let store = Store::open(&path).expect("main's newest schema opens here");
        drop(store);
        let conn = Connection::open(&path).unwrap();
        assert_eq!(read_schema_version(&conn).unwrap(), 23, "migrated on past ov-212's waits");
        assert_eq!(read_compatible_down_to(&conn).unwrap(), Some(COMPATIBLE_DOWN_TO));
        assert_eq!(CURRENT_SCHEMA_VERSION, 23, "a new migration moves this test's 'next' along");
        std::fs::remove_dir_all(&dir).ok();

        let next = database_left_by_a_newer_build(24, None);
        let err = Store::open(&next).err().expect("a schema after main's must not open unvouched");
        assert!(matches!(err, DomainError::NewerData), "{err:?}");
        assert_eq!(
            err.redacted_message(),
            "This runner's data was written by a newer Far Cooler. Update Far Cooler to use it."
        );
        std::fs::remove_dir_all(next.parent().unwrap()).ok();
    }

    /// A fresh file holding a store, with its `meta` rewritten as given, the
    /// way a newer build would have left it.
    fn database_left_by_a_newer_build(schema: u32, floor: Option<u32>) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("farcooler-newer-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("store.db");
        drop(Store::open(&path).unwrap());
        let conn = Connection::open(&path).unwrap();
        conn.execute("UPDATE meta SET value = ?1 WHERE key = 'schema_version'", [schema.to_string()])
            .unwrap();
        conn.execute("DELETE FROM meta WHERE key = 'compatible_down_to'", []).unwrap();
        if let Some(floor) = floor {
            conn.execute(
                "INSERT INTO meta (key, value) VALUES ('compatible_down_to', ?1)",
                [floor.to_string()],
            )
            .unwrap();
        }
        path
    }

    /// The rollback this exists for: a build that finds a schema newer than
    /// its own used to take `>=` for "current" and run against it. It refuses,
    /// says so in words a person can act on, and leaves the file as it was.
    #[test]
    fn a_database_from_a_newer_build_is_refused() {
        let newer = CURRENT_SCHEMA_VERSION + 1;
        let path = database_left_by_a_newer_build(newer, None);

        let err = Store::open(&path).err().expect("a newer schema must not open");
        assert!(matches!(err, DomainError::NewerData), "{err:?}");
        assert_eq!(
            err.redacted_message(),
            "This runner's data was written by a newer Far Cooler. Update Far Cooler to use it."
        );

        let conn = Connection::open(&path).unwrap();
        assert_eq!(read_schema_version(&conn).unwrap(), newer, "the refusal wrote nothing");
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    /// A newer build can vouch for an older one, and when it does the older
    /// one opens the file without touching its version.
    #[test]
    fn a_newer_database_that_vouches_for_this_build_opens() {
        let current = CURRENT_SCHEMA_VERSION;
        let path = database_left_by_a_newer_build(current + 2, Some(current));

        Store::open(&path).expect("a newer schema compatible down to this build opens");
        let conn = Connection::open(&path).unwrap();
        assert_eq!(read_schema_version(&conn).unwrap(), current + 2);
        assert_eq!(read_compatible_down_to(&conn).unwrap(), Some(current));
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    /// And when the floor it names is above this build, it doesn't.
    #[test]
    fn a_newer_database_whose_floor_is_above_this_build_is_refused() {
        let current = CURRENT_SCHEMA_VERSION;
        let path = database_left_by_a_newer_build(current + 2, Some(current + 1));

        let err = Store::open(&path).err().expect("refused");
        assert!(matches!(err, DomainError::NewerData), "{err:?}");
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    /// Every database this build leaves says how far back it can be read,
    /// including one that was already current before the marker existed.
    #[test]
    fn opening_stamps_how_far_back_the_database_can_be_read() {
        let current = CURRENT_SCHEMA_VERSION;
        let path = database_left_by_a_newer_build(current, None);
        {
            let conn = Connection::open(&path).unwrap();
            assert_eq!(read_compatible_down_to(&conn).unwrap(), None);
        }

        drop(Store::open(&path).unwrap());
        let conn = Connection::open(&path).unwrap();
        assert_eq!(
            read_compatible_down_to(&conn).unwrap(),
            Some(COMPATIBLE_DOWN_TO)
        );

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    /// The pre-flight a CLI runs before replacing a daemon: read-only, so it
    /// changes nothing it reads, and `None` where there's no database yet.
    #[test]
    fn a_database_s_schema_is_read_without_touching_it() {
        let current = CURRENT_SCHEMA_VERSION;
        let dir = std::env::temp_dir().join(format!("farcooler-schema-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(read_schema(dir.join("absent.db")).unwrap(), None);
        assert!(!dir.join("absent.db").exists(), "reading made no file");

        let path = database_left_by_a_newer_build(current + 1, None);
        let schema = read_schema(&path).unwrap().unwrap();
        assert_eq!(schema, DatabaseSchema { version: current + 1, compatible_down_to: None });
        assert!(!schema.opens_here() && schema.newer_than_here());

        let vouched = DatabaseSchema { version: current + 1, compatible_down_to: Some(current) };
        assert!(vouched.opens_here() && vouched.newer_than_here());
        let ours = DatabaseSchema { version: current, compatible_down_to: None };
        assert!(ours.opens_here() && !ours.newer_than_here());

        let conn = Connection::open(&path).unwrap();
        assert_eq!(read_compatible_down_to(&conn).unwrap(), None, "nothing stamped");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }
}
