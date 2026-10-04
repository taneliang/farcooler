//! A worktree's files, and the runner's extra read-only folders, as a phone
//! reads them (ov-259): `worktree.list_dir` and `worktree.read_file` as the
//! JSON `files_json` shapes for every client.

use farcooler_protocol::capability::{READ_ONLY_FOLDERS, WORKTREE_FILES};
use farcooler_protocol::v1 as pb;
use serde_json::Value;
use uuid::Uuid;

use super::{Session, SessionError, request, require, result, wrong};
use crate::files_json::{dir_json, file_json};

/// Where a path is read from: a worktree, or one of the runner's extra
/// read-only folders by the name `Host.read_only_folders` gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilesPlace {
    Worktree(Uuid),
    Folder(String),
}

impl FilesPlace {
    /// The request's `(worktree_id, folder)` pair: exactly one is set.
    fn ids(&self) -> (bytes::Bytes, String) {
        match self {
            FilesPlace::Worktree(id) => (bytes::Bytes::copy_from_slice(id.as_bytes()), String::new()),
            FilesPlace::Folder(name) => (bytes::Bytes::new(), name.clone()),
        }
    }

    /// What the request depends on that an older runner would silently drop:
    /// the `folder` field is one, so that runner must refuse instead of
    /// listing a worktree it was never asked about.
    fn required(&self) -> Vec<String> {
        match self {
            FilesPlace::Worktree(_) => Vec::new(),
            FilesPlace::Folder(_) => vec![READ_ONLY_FOLDERS.to_string()],
        }
    }

    /// Refused here, without a round trip, from a runner that lacks what the
    /// place needs.
    fn check(&self, advertised: &[String], method: &str) -> Result<(), SessionError> {
        require(advertised, WORKTREE_FILES, method)?;
        if matches!(self, FilesPlace::Folder(_)) {
            require(advertised, READ_ONLY_FOLDERS, method)?;
        }
        Ok(())
    }
}

impl Session {
    /// One directory's entries, as `dir_json` shapes them.
    pub async fn list_dir(&self, place: &FilesPlace, path: &str) -> Result<Value, SessionError> {
        place.check(self.capabilities(), "worktree.list_dir")?;
        let (worktree_id, folder) = place.ids();
        let payload =
            request::Payload::WorktreeDir(pb::WorktreeDirRequest { worktree_id, folder, path: path.to_string() });
        match self.value_requiring("worktree.list_dir", None, Some(payload), place.required()).await? {
            result::Value::WorktreeDir(d) => Ok(dir_json(&d)),
            other => Err(wrong("worktree_dir", &other)),
        }
    }

    /// One file, as `file_json` shapes it.
    pub async fn read_file(&self, place: &FilesPlace, path: &str) -> Result<Value, SessionError> {
        place.check(self.capabilities(), "worktree.read_file")?;
        let (worktree_id, folder) = place.ids();
        let payload =
            request::Payload::WorktreeFile(pb::WorktreeFileRequest { worktree_id, path: path.to_string(), folder });
        match self.value_requiring("worktree.read_file", None, Some(payload), place.required()).await? {
            result::Value::WorktreeFile(f) => Ok(file_json(&f)),
            other => Err(wrong("worktree_file", &other)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_names_its_capability_and_a_worktree_does_not() {
        assert_eq!(FilesPlace::Folder("logs".into()).required(), ["read_only_folders"]);
        assert!(FilesPlace::Worktree(Uuid::now_v7()).required().is_empty());
        let (id, folder) = FilesPlace::Folder("logs".into()).ids();
        assert!(id.is_empty() && folder == "logs");
        let wt = Uuid::now_v7();
        let (id, folder) = FilesPlace::Worktree(wt).ids();
        assert_eq!(&id[..], wt.as_bytes());
        assert!(folder.is_empty());
    }

    #[test]
    fn an_older_runner_is_refused_before_the_wire() {
        let files_only = vec!["worktree_files".to_string()];
        let wt = FilesPlace::Worktree(Uuid::now_v7());
        let folder = FilesPlace::Folder("logs".into());
        assert!(wt.check(&files_only, "worktree.list_dir").is_ok());
        assert!(matches!(folder.check(&files_only, "worktree.list_dir"), Err(SessionError::Refused { .. })));
        assert!(matches!(wt.check(&["tasks".to_string()], "worktree.list_dir"), Err(SessionError::Refused { .. })));
    }
}
