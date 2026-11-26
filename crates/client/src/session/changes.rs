//! Worktree change sets, diffs, and the Changes inbox.
//! The JSON entry points share projections with the CLI. Web review uses
//! the typed records to resolve candidate, file, and line anchors.

use farcooler_protocol::v1::{request, result};
use serde_json::json;
use uuid::Uuid;

use super::{Session, SessionError, wrong};
use crate::changes_json::{change_set_json, file_change_json, file_diff_json, inbox_json};

impl Session {
    /// What a worktree changed, against its base.
    pub async fn change_set(
        &self,
        worktree: Uuid,
        fresh: bool,
    ) -> Result<serde_json::Value, SessionError> {
        Ok(change_set_json(&self.change_set_record(worktree, fresh).await?))
    }

    pub(super) async fn change_set_record(
        &self,
        worktree: Uuid,
        fresh: bool,
    ) -> Result<farcooler_protocol::v1::ChangeSet, SessionError> {
        let payload =
            request::Payload::ChangeSetRequest(farcooler_protocol::v1::ChangeSetRequest {
                worktree_id: bytes::Bytes::copy_from_slice(worktree.as_bytes()),
                selector: None,
                fresh,
            });
        match self.value("changes.change_set", None, Some(payload)).await? {
            result::Value::ChangeSet(cs) => Ok(cs),
            other => Err(wrong("change_set", &other)),
        }
    }

    /// One file's patch, in the comparison `scope` names.
    ///
    /// `context` of zero means git's own three, which is what a first read
    /// wants; a caller opening one of the gaps between hunks asks for enough to
    /// swallow it. See the Mac's `ChangesStore.open(gap:of:in:)` for why that is
    /// per-gap rather than per-file.
    pub async fn file_diff(
        &self,
        worktree: Uuid,
        path: &str,
        scope: &str,
        context: u32,
    ) -> Result<serde_json::Value, SessionError> {
        Ok(file_diff_json(&self.file_diff_record(worktree, path, scope, 0, context).await?))
    }

    pub(super) async fn file_diff_record(
        &self,
        worktree: Uuid,
        path: &str,
        scope: &str,
        from_hunk: u32,
        context: u32,
    ) -> Result<farcooler_protocol::v1::FileDiff, SessionError> {
        use farcooler_protocol::v1::diff_selector::Kind;
        let kind = match scope {
            "local" => Kind::Local(farcooler_protocol::v1::Empty {}),
            "staged" => Kind::Staged(farcooler_protocol::v1::Empty {}),
            "unstaged" => Kind::Unstaged(farcooler_protocol::v1::Empty {}),
            // Anything else is a commit sha, and the empty string is the branch
            // range — the default every caller that names no scope gets.
            "" | "branch" => Kind::Range(farcooler_protocol::v1::Empty {}),
            sha => Kind::Commit(sha.to_string()),
        };
        let payload = request::Payload::FileDiffRequest(farcooler_protocol::v1::FileDiffRequest {
            worktree_id: bytes::Bytes::copy_from_slice(worktree.as_bytes()),
            selector: Some(farcooler_protocol::v1::DiffSelector { kind: Some(kind) }),
            path: path.to_string(),
            from_hunk,
            context,
        });
        match self.value("changes.file_diff", None, Some(payload)).await? {
            result::Value::FileDiff(d) => Ok(d),
            other => Err(wrong("file_diff", &other)),
        }
    }

    /// The files one commit touched.
    pub async fn commit_files(
        &self,
        worktree: Uuid,
        sha: &str,
    ) -> Result<serde_json::Value, SessionError> {
        let payload =
            request::Payload::CommitFilesRequest(farcooler_protocol::v1::CommitFilesRequest {
                worktree_id: bytes::Bytes::copy_from_slice(worktree.as_bytes()),
                sha: sha.to_string(),
            });
        match self.value("changes.commit_files", None, Some(payload)).await? {
            result::Value::FileChangeList(l) => Ok(json!({
                "files": l.items.iter().map(file_change_json).collect::<Vec<_>>()
            })),
            other => Err(wrong("file_change_list", &other)),
        }
    }

    /// Every worktree with something to say, and whether anybody has looked.
    ///
    /// Not "every worktree that changed since it was last looked at", which is
    /// what this said and is narrower than what `review_ops::inbox` returns: a
    /// worktree with a nonzero diff is listed even after you mark it read, and
    /// `changed_since_reviewed` is how a row says which it is. A front door
    /// built on the old sentence would have dropped every branch you had
    /// already seen out of a list whose whole job is "is this worth opening".
    ///
    /// Shared with `farcooler changes inbox --json` since the two shapes became
    /// one — see `changes_json::inbox_json` for what they used to be and why
    /// this half won.
    pub async fn changes_inbox(&self) -> Result<serde_json::Value, SessionError> {
        let payload =
            request::Payload::ChangesInbox(farcooler_protocol::v1::ChangesInboxRequest {});
        match self.value("changes.inbox", None, Some(payload)).await? {
            result::Value::ChangesInbox(inbox) => Ok(inbox_json(&inbox)),
            other => Err(wrong("changes_inbox", &other)),
        }
    }

    /// Mark a worktree as read, which is what clears its inbox badge.
    pub async fn changes_mark_read(&self, worktree: Uuid) -> Result<(), SessionError> {
        let payload =
            request::Payload::ChangesMarkRead(farcooler_protocol::v1::ChangesMarkRead {
                worktree_id: bytes::Bytes::copy_from_slice(worktree.as_bytes()),
                branch: String::new(),
            });
        self.value("changes.mark_read", None, Some(payload)).await?;
        Ok(())
    }

    /// Pin what this worktree is compared against.
    ///
    /// The affordance that exists because a GUESSED base produces a wrong diff
    /// that looks exactly like a right one — see `BaseSource` in the protocol.
    pub async fn changes_set_base(
        &self,
        worktree: Uuid,
        base_ref: &str,
    ) -> Result<serde_json::Value, SessionError> {
        let payload = request::Payload::ChangesSetBase(farcooler_protocol::v1::ChangesSetBase {
            worktree_id: bytes::Bytes::copy_from_slice(worktree.as_bytes()),
            base_ref: base_ref.to_string(),
        });
        match self.value("changes.set_base", None, Some(payload)).await? {
            result::Value::ChangeSet(cs) => Ok(change_set_json(&cs)),
            other => Err(wrong("change_set", &other)),
        }
    }
}
