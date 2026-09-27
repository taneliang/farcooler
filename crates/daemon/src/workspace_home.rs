//! Each workspace's home: `<runtime dir>/workspaces/<workspace id>/`.
//!
//! Keyed by id, so renaming a workspace never moves a file. It holds
//! `charter.md`, the user's instructions for that workstream, and it's where
//! a Claude Code or Cursor orchestrator runs. Far Cooler puts nothing else
//! there.
//!
//! **The charter lives here, not in the repository.** It's personal:
//! coworkers aren't running this orchestrator. A `.farcooler/manager.md` in a
//! repository, which is where the charter lived before workspaces, is copied
//! into Main's home once (`adopt_repository_charters`) and left where it was;
//! removing it is the user's commit to make.
//!
//! **A charter is never overwritten.** Every write here creates the file or
//! does nothing, with `create_new`, so a charter the user has edited, or one
//! a second daemon process wrote a moment earlier, stays exactly as it is.
//!
//! **A charter can be absent.** A home with no `charter.md` means nobody has
//! written one yet, which is the manager skill's cue to interview for it.
//! Nothing here writes an empty one: an empty file would read as a charter
//! somebody chose to leave blank, and would stop a repository's
//! `.farcooler/manager.md` from ever being adopted.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use uuid::Uuid;

/// The charter's file name inside a home.
pub const CHARTER_FILE: &str = "charter.md";

/// Where a pre-workspaces repository kept its charter, relative to the main
/// checkout.
pub const REPOSITORY_CHARTER: &str = ".farcooler/manager.md";

/// A workspace's home: `<root>/workspaces/<workspace id>`. `root` is the
/// runtime directory (`FARCOOLER_HOME` when set). Only computed; see
/// `make_home`.
pub fn home(root: &Path, workspace: Uuid) -> PathBuf {
    root.join("workspaces").join(workspace.to_string())
}

/// A workspace's charter: `home/charter.md`. It may not exist yet.
pub fn charter_path(root: &Path, workspace: Uuid) -> PathBuf {
    home(root, workspace).join(CHARTER_FILE)
}

/// Make `workspace`'s home if it isn't there, and give it a charter copied
/// from `seed_from` if it has none and the seed exists.
///
/// An existing charter is never overwritten. A missing seed (Main has no
/// charter yet) leaves the charter missing too, rather than writing an empty
/// one; see the module doc. Returns the home.
pub fn make_home(root: &Path, workspace: Uuid, seed_from: Option<&Path>) -> io::Result<PathBuf> {
    seed(root, workspace, seed_from)?;
    Ok(home(root, workspace))
}

/// `make_home`, saying whether it wrote the charter.
fn seed(root: &Path, workspace: Uuid, seed_from: Option<&Path>) -> io::Result<bool> {
    let home = home(root, workspace);
    std::fs::create_dir_all(&home)?;
    let charter = home.join(CHARTER_FILE);
    let Some(seed) = seed_from else { return Ok(false) };
    if charter.exists() {
        return Ok(false);
    }
    let text = match std::fs::read(seed) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    // `create_new`, not the `exists` check above, is what keeps an existing
    // charter: the check only saves reading the seed.
    let mut file = match std::fs::OpenOptions::new().write(true).create_new(true).open(&charter) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Ok(false),
        Err(e) => return Err(e),
    };
    file.write_all(&text)?;
    Ok(true)
}

/// Copy each repository's `.farcooler/manager.md` into its Main's charter,
/// where Main has none. `mains` pairs each Main with its repository's main
/// checkout. Returns how many charters were written.
///
/// The file in the repository is left alone. A Main that already has a
/// charter keeps it, so this is safe to run at every start: after the first
/// run it copies nothing.
///
/// One repository failing doesn't stop the rest; it's logged and skipped,
/// because an unmounted volume shouldn't cost every other repository its
/// charter.
pub fn adopt_repository_charters(root: &Path, mains: &[(Uuid, PathBuf)]) -> usize {
    let mut adopted = 0;
    for (main, checkout) in mains {
        let source = checkout.join(REPOSITORY_CHARTER);
        match seed(root, *main, Some(&source)) {
            Ok(true) => {
                tracing::info!(workspace = %main, from = %source.display(), "adopted a repository's charter");
                adopted += 1;
            }
            Ok(false) => {}
            Err(e) => {
                tracing::warn!(workspace = %main, from = %source.display(), error = %e, "could not adopt a repository's charter")
            }
        }
    }
    adopted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repository_with_charter(text: &str) -> tempfile::TempDir {
        let repo = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(repo.path().join(".farcooler")).unwrap();
        std::fs::write(repo.path().join(REPOSITORY_CHARTER), text).unwrap();
        repo
    }

    #[test]
    fn a_home_is_keyed_by_id_under_the_runtime_directory() {
        let root = Path::new("/r");
        let ws = Uuid::now_v7();
        assert_eq!(home(root, ws), PathBuf::from(format!("/r/workspaces/{ws}")));
        assert_eq!(charter_path(root, ws), PathBuf::from(format!("/r/workspaces/{ws}/charter.md")));
    }

    #[test]
    fn a_repository_charter_is_copied_into_main_and_left_where_it_was() {
        let root = tempfile::tempdir().unwrap();
        let repo = repository_with_charter("ours");
        let main = Uuid::now_v7();
        assert_eq!(adopt_repository_charters(root.path(), &[(main, repo.path().into())]), 1);
        assert_eq!(std::fs::read_to_string(charter_path(root.path(), main)).unwrap(), "ours");
        assert_eq!(std::fs::read_to_string(repo.path().join(REPOSITORY_CHARTER)).unwrap(), "ours");
        // A second start copies nothing.
        assert_eq!(adopt_repository_charters(root.path(), &[(main, repo.path().into())]), 0);
    }

    #[test]
    fn adoption_never_overwrites_a_charter_main_already_has() {
        let root = tempfile::tempdir().unwrap();
        let repo = repository_with_charter("the repository's");
        let main = Uuid::now_v7();
        make_home(root.path(), main, None).unwrap();
        std::fs::write(charter_path(root.path(), main), "edited").unwrap();
        assert_eq!(adopt_repository_charters(root.path(), &[(main, repo.path().into())]), 0);
        assert_eq!(std::fs::read_to_string(charter_path(root.path(), main)).unwrap(), "edited");
    }

    /// A repository without a charter gives Main a home and no charter, and
    /// doesn't stop the repository after it.
    #[test]
    fn a_repository_with_no_charter_adopts_nothing_and_stops_nothing() {
        let root = tempfile::tempdir().unwrap();
        let bare = tempfile::tempdir().unwrap();
        let repo = repository_with_charter("second");
        let (first, second) = (Uuid::now_v7(), Uuid::now_v7());
        let mains = [(first, bare.path().into()), (second, repo.path().into())];
        assert_eq!(adopt_repository_charters(root.path(), &mains), 1);
        assert!(home(root.path(), first).is_dir());
        assert!(!charter_path(root.path(), first).exists(), "no empty charter");
        assert_eq!(std::fs::read_to_string(charter_path(root.path(), second)).unwrap(), "second");
    }

    #[test]
    fn an_existing_charter_is_never_overwritten() {
        let root = tempfile::tempdir().unwrap();
        let ws = Uuid::now_v7();
        make_home(root.path(), ws, None).unwrap();
        std::fs::write(charter_path(root.path(), ws), "edited").unwrap();
        let seed = root.path().join("seed.md");
        std::fs::write(&seed, "seed").unwrap();
        make_home(root.path(), ws, Some(&seed)).unwrap();
        assert_eq!(std::fs::read_to_string(charter_path(root.path(), ws)).unwrap(), "edited");
    }

    #[test]
    fn a_home_starts_with_a_copy_of_its_seed() {
        let root = tempfile::tempdir().unwrap();
        let seed = root.path().join("seed.md");
        std::fs::write(&seed, "Main's charter").unwrap();
        let ws = Uuid::now_v7();
        assert_eq!(make_home(root.path(), ws, Some(&seed)).unwrap(), home(root.path(), ws));
        assert_eq!(std::fs::read_to_string(charter_path(root.path(), ws)).unwrap(), "Main's charter");
        // A copy, not a link: editing one leaves the other.
        std::fs::write(charter_path(root.path(), ws), "Billing's").unwrap();
        assert_eq!(std::fs::read_to_string(&seed).unwrap(), "Main's charter");
    }

    #[test]
    fn a_missing_seed_leaves_the_charter_missing() {
        let root = tempfile::tempdir().unwrap();
        let ws = Uuid::now_v7();
        make_home(root.path(), ws, Some(&root.path().join("nothing.md"))).unwrap();
        assert!(home(root.path(), ws).is_dir());
        assert!(!charter_path(root.path(), ws).exists());
    }
}

/// The homes as the daemon makes them: at registration, at each of its
/// starts, and when a workspace is created.
#[cfg(test)]
mod service_tests {
    use farcooler_protocol::v1::{self as pb, Scope};

    use super::*;
    use crate::git;
    use crate::service::Service;
    use crate::test_support::fixture;

    /// A git repository at `path`, with `.farcooler/manager.md` saying `charter`.
    async fn repository_at(path: &Path, charter: &str) {
        std::fs::create_dir_all(path.join(".farcooler")).unwrap();
        std::fs::write(path.join(REPOSITORY_CHARTER), charter).unwrap();
        for args in [
            vec!["init", "-q", "-b", "main", "."],
            vec!["config", "user.email", "t@example.com"],
            vec!["config", "commit.gpgsign", "false"],
            vec!["config", "user.name", "t"],
            vec!["commit", "-q", "--allow-empty", "-m", "base"],
        ] {
            git::git(path, &args).await.unwrap();
        }
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    #[tokio::test]
    async fn registering_a_repository_adopts_its_charter_into_main() {
        let (dir, svc, _) = fixture().await;
        let path = dir.path().join("charted");
        repository_at(&path, "ship small").await;

        let repo = svc.register_repository(&path).await.unwrap();
        let main = svc.store.main_workspace(repo.id).unwrap();
        assert_eq!(read(&charter_path(svc.root_dir(), main.id)), "ship small");
        assert_eq!(read(&path.join(REPOSITORY_CHARTER)), "ship small", "left in the repository");
    }

    /// A board from before workspaces: Mains with no home, a repository
    /// with a `.farcooler/manager.md`, and a workspace made by a daemon that
    /// didn't make homes. The next start gives Main the repository's charter
    /// and the other workspace a copy of it.
    #[tokio::test]
    async fn a_start_adopts_charters_and_gives_every_workspace_a_home() {
        let (dir, svc, repo) = fixture().await;
        std::fs::create_dir_all(dir.path().join("repo/.farcooler")).unwrap();
        std::fs::write(dir.path().join("repo").join(REPOSITORY_CHARTER), "from the repo").unwrap();
        let main = svc.store.main_workspace(repo).unwrap();
        let billing = svc.store.create_workspace(repo, "Billing", "bil").unwrap();
        assert!(!charter_path(svc.root_dir(), main.id).exists(), "registration found no charter");
        assert!(!home(svc.root_dir(), billing.id).exists(), "made past the daemon");

        let restarted = Service::open_in(svc.root_dir().to_path_buf()).await.unwrap();
        restarted.prepare_workspace_homes();
        assert_eq!(read(&charter_path(restarted.root_dir(), main.id)), "from the repo");
        assert_eq!(read(&charter_path(restarted.root_dir(), billing.id)), "from the repo", "seeded from Main");

        // Seeded once. A charter the user took out of a home stays out.
        std::fs::remove_file(charter_path(restarted.root_dir(), billing.id)).unwrap();
        drop(restarted);
        let again = Service::open_in(svc.root_dir().to_path_buf()).await.unwrap();
        again.prepare_workspace_homes();
        assert!(!charter_path(again.root_dir(), billing.id).exists());
    }

    #[tokio::test]
    async fn a_new_workspace_starts_with_a_copy_of_mains_charter() {
        let (_dir, svc, repo) = fixture().await;
        let main = svc.store.main_workspace(repo).unwrap();
        std::fs::write(charter_path(svc.root_dir(), main.id), "Main's words").unwrap();
        let watcher = crate::watch::Watcher::new(svc.clone());

        let req = pb::WorkspaceCreate { name: "Billing".into(), task_prefix: "bil".into() };
        let made = crate::workspace_ops::create(&svc, &watcher, repo, &req, Scope::HostAdmin).unwrap();
        let charter = made.charter_path.expect("host_admin sees the path");
        assert_eq!(read(Path::new(&charter)), "Main's words");
        std::fs::write(&charter, "Billing's words").unwrap();
        assert_eq!(read(&charter_path(svc.root_dir(), main.id)), "Main's words", "a copy");
    }
}
