//! Naming a terminal (ov-234): `terminal.rename`.
//!
//! The name is the stored `title`, the one column a client already shows when
//! it is not the automatic "Terminal 12". Writing it through the store is what
//! makes it survive a daemon restart; nothing else holds it.

use super::*;

/// The longest name a terminal takes, in characters. A row, a tab and a
/// jumpbar entry all draw it, and none of them has room for a paragraph.
pub(crate) const MAX_TERMINAL_NAME: usize = 80;

/// What an empty name puts back: the placeholder every terminal is created
/// with, which clients read as "name it for what runs in it".
const AUTOMATIC: &str = "Terminal";

impl Service {
    /// Give a terminal a name, or clear it with an empty one.
    ///
    /// Trimmed. Refused as `name` when longer than [`MAX_TERMINAL_NAME`] or
    /// when it holds a control character: a newline in a tab title is a way to
    /// forge a second row, and the name reaches notifications and tmux.
    ///
    /// Any terminal may be renamed, including a lost or exited one; the row
    /// keeps its name until it is removed. The agent's own title for a
    /// conversation never overwrites a name a person chose
    /// (`remember_agent_title`).
    pub fn rename_terminal(&self, id: Uuid, name: &str) -> Result<models::Terminal> {
        let name = name.trim();
        if name.chars().count() > MAX_TERMINAL_NAME || name.chars().any(char::is_control) {
            return Err(DomainError::InvalidArgument { what: "name" });
        }
        let term = self.store.get_terminal(id)?;
        let title = if name.is_empty() { AUTOMATIC } else { name };
        if term.title == title {
            return Ok(term);
        }
        self.store.update_terminal(
            id,
            term.resource_version,
            terminal_update(&term, |u| u.title = title.to_string()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::fixture;

    /// A terminal row in the repository's main checkout, with no tmux behind it:
    /// naming is a fact about the record.
    fn a_terminal(svc: &Service) -> models::Terminal {
        let ws = svc.list_worktrees().unwrap().remove(0);
        svc.store.create_terminal(ws.id, "Terminal 3", "shell", TerminalIntent::Running, 80, 24).unwrap()
    }

    /// **The name survives a restart.** The daemon is opened again on the same
    /// state directory, which is everything a restart keeps.
    #[tokio::test]
    async fn a_renamed_terminal_keeps_its_name_across_a_daemon_restart() {
        let (dir, svc, _repo) = fixture().await;
        let term = a_terminal(&svc);

        let renamed = svc.rename_terminal(term.id, "  gcp proxy ").expect("rename");
        assert_eq!(renamed.title, "gcp proxy", "trimmed");
        assert_eq!(renamed.resource_version, term.resource_version + 1);

        let reopened = Store::open(dir.path().join("state").join("farcooler.db")).expect("reopen");
        assert_eq!(reopened.get_terminal(term.id).unwrap().title, "gcp proxy");
    }

    /// An empty name clears it back to the automatic one, which the clients
    /// read as "name it for what is running".
    #[tokio::test]
    async fn an_empty_name_puts_the_automatic_one_back() {
        let (_dir, svc, _repo) = fixture().await;
        let term = a_terminal(&svc);
        svc.rename_terminal(term.id, "proxy").unwrap();
        assert_eq!(svc.rename_terminal(term.id, "   ").unwrap().title, "Terminal");
    }

    /// A name that could forge a row, or that no row has room for, is refused
    /// and leaves the old one standing.
    #[tokio::test]
    async fn a_name_with_a_newline_or_a_paragraph_is_refused() {
        let (_dir, svc, _repo) = fixture().await;
        let term = a_terminal(&svc);
        svc.rename_terminal(term.id, "proxy").unwrap();
        for bad in ["a\nb", "a\u{1b}[31mb", &"x".repeat(MAX_TERMINAL_NAME + 1)] {
            let err = svc.rename_terminal(term.id, bad).unwrap_err();
            assert!(matches!(err, DomainError::InvalidArgument { what: "name" }), "{bad:?}: {err:?}");
        }
        assert_eq!(svc.store.get_terminal(term.id).unwrap().title, "proxy");
        svc.rename_terminal(term.id, &"x".repeat(MAX_TERMINAL_NAME)).expect("the limit itself is allowed");
    }

    /// A terminal that isn't there is not found, not renamed into existence.
    #[tokio::test]
    async fn renaming_a_missing_terminal_is_not_found() {
        let (_dir, svc, _repo) = fixture().await;
        let err = svc.rename_terminal(Uuid::now_v7(), "proxy").unwrap_err();
        assert!(matches!(err, DomainError::NotFound), "{err:?}");
    }
}
