//! Ticket-linked review through the existing scoped runner methods.
use super::{FilesPlace, Session, SessionError, task_note_append, web_board::on_board};
use crate::{
    web_board,
    web_review::{self, Candidate, Edit, Position, Request},
    workflow_presentation::{Action, ActionKind, Style},
};
use farcooler_protocol::{
    capability,
    v1::{self as pb, ErrorCode},
};
use serde_json::Value;
use uuid::Uuid;

impl Session {
    pub async fn web_review(
        &self,
        workspace: Uuid,
        ticket: Uuid,
        request: Request,
    ) -> Result<Value, SessionError> {
        // Runner file and diff reads expose source, so both require control.
        let scope = self.granted_scope();
        if !web_board::can_write(scope) {
            return Err(refused(ErrorCode::ScopeDenied, ""));
        }
        let (detail, terminals, trees) =
            tokio::try_join!(self.task_record(ticket), self.terminals(), self.worktrees(),)?;
        let task = on_board(&detail, workspace)?;
        let (_, tree) = web_board::destinations(task, &terminals, &trees);
        let tree = tree.ok_or_else(|| refused(ErrorCode::NotFound, "review_worktree"))?;
        let key = task.key.clone();
        let place = FilesPlace::Worktree(tree);
        match request {
            Request::Directory { path } => Ok(web_review::files(
                &self.list_dir(&place, &path).await?,
                true,
            )),
            Request::File { path } => Ok(web_review::files(
                &self.read_file(&place, &path).await?,
                false,
            )),
            Request::Verify { candidate } => {
                if candidate.is_empty() {
                    return Err(stale());
                }
                let changes = self.change_set_record(tree, true).await?;
                check_candidate(&Candidate::new(
                    tree,
                    &changes,
                    Position {
                        candidate,
                        ..Position::default()
                    },
                ))?;
                Ok(Value::Null)
            }
            Request::Changes {
                position,
                mut marks,
                edit,
            } => {
                let changes = self.change_set_record(tree, true).await?;
                let view = Candidate::new(tree, &changes, position.clone());
                if edit.is_some() {
                    if position.candidate.is_empty() || position.file != view.position.file {
                        return Err(stale());
                    }
                    check_candidate(&view)?;
                }
                let patch = if view.position.file.is_empty() {
                    None
                } else {
                    let patch = self
                        .file_diff_record(
                            tree,
                            &view.position.file,
                            view.position.comparison.scope(),
                            view.position.from,
                            0,
                        )
                        .await?;
                    if !view.matches_patch(&patch) {
                        return Err(stale());
                    }
                    // Local diffs read mutable files. A committed diff is pinned
                    // on current runners, but a fresh read also rejects movement
                    // between the two reads on older runners.
                    let after = self.change_set_record(tree, true).await?;
                    check_candidate(&Candidate::new(tree, &after, view.position.clone()))?;
                    Some(patch)
                };
                let mut notes = detail.notes;
                match edit {
                    Some(Edit::Review { reviewed }) => {
                        let patch = patch.as_ref().ok_or_else(stale)?;
                        if reviewed && !view.can_mark_reviewed(patch) {
                            return Err(refused(ErrorCode::InvalidArgument, "review_incomplete"));
                        }
                        view.update_marks(&mut marks, reviewed);
                    }
                    Some(Edit::Comment { body }) => {
                        if !patch.as_ref().is_some_and(|p| view.selected_line(p)) {
                            return Err(stale());
                        }
                        if body.trim().is_empty() {
                            return Err(refused(ErrorCode::InvalidArgument, "review_comment"));
                        }
                        let mut note = task_note_append(ticket, pb::TaskNoteKind::Comment, &body);
                        note.extra_json = view.comment_extra();
                        notes.push(self.task_note_record(note).await?);
                    }
                    None => {}
                }
                let approval = if self.can(capability::WORKFLOW_COMMANDS)
                    && view.position.comparison.scope() == "branch"
                    && !changes.base_commit.is_empty()
                {
                    self.web_review_approval(workspace, &key, &changes.head_commit)
                        .await
                } else {
                    None
                };
                Ok(view.view(
                    patch.as_ref(),
                    &notes,
                    scope,
                    &marks,
                    approval.as_ref().map(|a| (key.as_str(), a)),
                ))
            }
        }
    }

    async fn web_review_approval(&self, workspace: Uuid, key: &str, head: &str) -> Option<Action> {
        let snapshot = self.workflow_snapshot(workspace, Style::Web).await.ok()?;
        review_action(snapshot.presentation.runs, key, head)
    }
}

fn review_action(
    runs: Vec<crate::workflow_presentation::RunCard>,
    key: &str,
    head: &str,
) -> Option<Action> {
    let run = runs
        .into_iter()
        .find(|r| r.item == key && !head.is_empty() && r.candidate.as_deref() == Some(head))?;
    run.actions
        .into_iter()
        .find(|a| matches!(a.kind, ActionKind::Approve | ActionKind::AcceptReview))
}

fn check_candidate(view: &Candidate<'_>) -> Result<(), SessionError> {
    if view.stale { Err(stale()) } else { Ok(()) }
}

fn stale() -> SessionError {
    refused(ErrorCode::ResourceConflict, "review_stale")
}

fn refused(code: ErrorCode, what: &str) -> SessionError {
    SessionError::Refused {
        code: code as i32,
        retryable: false,
        message: what.into(),
        what: what.into(),
    }
}
