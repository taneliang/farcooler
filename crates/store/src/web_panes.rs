//! The page a web pane opened on (ov-435).
//!
//! One nullable column on `terminals`, because the URL is intent, like the
//! preset: what the pane is for, written once when it is made. A relaunched
//! app reads it back with the fleet, since the daemon and tmux outlive it.
//! Where the owner navigated after that is the Mac's to remember, never this
//! row's: R-41 lets an agent open a page, and nothing it can read says what
//! the owner browsed.
//!
//! The URL is checked before it reaches here (`farcooler_daemon::web_pane`):
//! this layer stores what it is given.

use rusqlite::params;
use uuid::Uuid;

use farcooler_core::{DomainError, Result};

use crate::error::map_err;
use crate::models::{Terminal, uuid_blob};
use crate::store::Store;

/// The column. Nullable, and NULL is already "not a web pane", so an older
/// build that never names it reads every row the way it always did. It reads
/// a web pane's mode (3) as a terminal, `PaneMode::from_i64`'s fallback: the
/// pane shows its host's one line, which is honest.
pub(crate) fn migration_0040_web_url(tx: &rusqlite::Transaction) -> rusqlite::Result<()> {
    tx.execute_batch("ALTER TABLE terminals ADD COLUMN web_url TEXT;")
}

impl Store {
    /// Record the page web pane `id` opened on, and hand back the record.
    ///
    /// Moves the resource version, as every write to a terminal row does, but
    /// not the epoch: the program in the pane is the same one.
    pub fn set_web_url(&self, id: Uuid, url: &str) -> Result<Terminal> {
        let changed = self
            .conn()
            .execute(
                "UPDATE terminals SET web_url = ?1, resource_version = resource_version + 1 WHERE id = ?2",
                params![url, uuid_blob(id)],
            )
            .map_err(map_err)?;
        if changed == 0 {
            return Err(DomainError::NotFound);
        }
        self.get_terminal(id)
    }
}

#[cfg(test)]
mod tests {
    use crate::models::PaneMode;
    use farcooler_protocol::v1::TerminalIntent;
    use crate::store::Store;
    use uuid::Uuid;

    fn terminal(s: &Store, path: &str) -> crate::models::Terminal {
        let host = Uuid::now_v7();
        let root = s.create_repository_root(host, path, 1_000).unwrap();
        let repo = s.create_repository(host, root.id, "name", "/gitdir", "origin").unwrap();
        let ws = s.create_worktree(repo.id, "feature/x", path, false).unwrap();
        s.create_terminal(ws.id, "Web", "web", TerminalIntent::Running, 120, 40).unwrap()
    }

    /// The page survives the daemon closing its database and opening it
    /// again, which is what "kept across relaunch" rests on.
    #[test]
    fn a_web_panes_page_survives_a_reopen() {
        let dir = std::env::temp_dir().join(format!("fc-web-url-{}", Uuid::now_v7().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.db");
        let (id, version) = {
            let s = Store::open(&path).unwrap();
            let t = terminal(&s, "/wt/web");
            assert_eq!(t.web_url, None, "a new pane has no page until one is recorded");
            let set = s.set_web_url(t.id, "https://github.com/").unwrap();
            assert_eq!(set.web_url.as_deref(), Some("https://github.com/"));
            assert!(set.resource_version > t.resource_version, "a write moves the version");
            assert_eq!(set.epoch, t.epoch, "the program in the pane is the same one");
            (t.id, set.resource_version)
        };
        let s = Store::open(&path).unwrap();
        let back = s.get_terminal(id).unwrap();
        assert_eq!(back.web_url.as_deref(), Some("https://github.com/"));
        assert_eq!(back.resource_version, version);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Mode 3 reads back as Web, and an unknown number still reads as the
    /// mode that always works.
    #[test]
    fn web_mode_round_trips_through_its_number() {
        assert_eq!(PaneMode::from_i64(PaneMode::Web.as_i64()), PaneMode::Web);
        assert_eq!(PaneMode::Web.as_i64(), 3);
        assert_eq!(PaneMode::from_i64(4), PaneMode::Terminal);
    }

    #[test]
    fn a_missing_terminal_is_not_found() {
        let s = Store::open_in_memory().unwrap();
        let refused = s.set_web_url(Uuid::now_v7(), "https://github.com/");
        assert!(matches!(refused, Err(farcooler_core::DomainError::NotFound)), "{refused:?}");
    }
}
