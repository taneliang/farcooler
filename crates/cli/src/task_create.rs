use farcooler_protocol::capability;
use farcooler_protocol::v1::{self as pb, request::Payload, result};

use super::{Board, DispatchLink, actor_for, board_for, new_acceptance, now_millis, refused, task_json};
use crate::workspaces::{WORKSPACE_ENV, needs_workstreams};
use crate::{Fallible, expect_value, id_bytes, req_for, with};

#[derive(clap::Args)]
pub struct CreateArgs {
    /// One line, for the board. At most 200 characters.
    #[arg(long)]
    title: String,
    /// What this task is for, in prose. Revisable at any time.
    #[arg(long)]
    intent: Option<String>,
    /// One checkable thing. Repeatable.
    #[arg(long = "accept")]
    accept: Vec<String>,
    /// One thing this task may not do. Repeatable.
    #[arg(long = "constraint")]
    constraint: Vec<String>,
    /// A word to file this task under. Repeatable.
    #[arg(long = "label")]
    label: Vec<String>,
    /// The theme to file this card under, by name or id.
    #[arg(long)]
    theme: Option<String>,
    /// The worktree this task will use, if it already has one.
    #[arg(long)]
    worktree: Option<String>,
    /// Which workspace's board, by name, task prefix or id. Defaults to
    /// the pane's own workspace (FARCOOLER_WORKSPACE), then to the
    /// repository's Main.
    #[arg(long)]
    workspace: Option<String>,
    /// Which repository. Defaults to the pane's own, then to the only one
    /// there is.
    #[arg(long)]
    repo: Option<String>,
    /// Who this write is from: `user`, `manager`, or `agent:<terminal id>`.
    /// Read from FARCOOLER_ACTOR when not given, and `user` when neither
    /// says — always sent, never left for the runner to assume.
    #[arg(long)]
    actor: Option<String>,
}

pub(super) async fn run<L: DispatchLink>(link: &mut L, args: CreateArgs, json: bool) -> Fallible {
    let CreateArgs { title, intent, accept, constraint, label, theme, worktree, workspace, repo, actor } = args;
    let actor = actor_for(actor.as_deref())?;
    let board =
        board_for(link, repo.as_deref(), workspace.as_deref(), std::env::var(WORKSPACE_ENV).ok())
            .await?;
    let worktree_id = match worktree.as_deref() {
        Some(name) => Some(crate::resolve_worktree_id(link, name).await?),
        None => None,
    };
    let (board, theme_id) = if let Some(name) = theme {
        let (board, id) = crate::plan::board_and_theme_for_create(link, board, &name).await?;
        (board, Some(id))
    } else {
        (board, None)
    };
    let create = request(
        &board,
        pb::TaskCreate {
            repository_id: bytes::Bytes::new(),
            title: title.trim().to_string(),
            intent: intent.unwrap_or_default(),
            acceptance: accept.iter().map(|t| new_acceptance(t)).collect(),
            constraints: constraint,
            labels: label,
            worktree_id: worktree_id.map(id_bytes),
            actor: actor.to_string(),
            workspace_id: None,
            theme_id,
        },
    );
    let r = link
        .call(create)
        .await
        .map_err(|e| refused(e, "a task needs a title of at most 200 characters"))?;
    let result::Value::Task(created) = expect_value(r.value)? else {
        return Err(crate::daemon_link::UNREADABLE.into());
    };

    if json {
        println!("{}", task_json(&created, now_millis()));
        return Ok(());
    }
    println!("{}  {}", created.key, created.title);
    Ok(())
}

/// File on the selected board and require the capabilities that preserve
/// the requested workspace and theme on the runner.
pub(super) fn request(board: &Board, create: pb::TaskCreate) -> pb::Request {
    let workspace = board.workspace.as_ref().map(|w| w.id.clone());
    let filing_on_a_workspace = workspace.is_some();
    let filing_in_a_theme = create.theme_id.is_some();
    let request = with(
        req_for("task.create", board.repository),
        Payload::TaskCreate(pb::TaskCreate {
            repository_id: id_bytes(board.repository),
            workspace_id: workspace,
            ..create
        }),
    );
    let mut request = if filing_on_a_workspace { needs_workstreams(request) } else { request };
    if filing_in_a_theme {
        request.required_capabilities.push(capability::TASK_CREATE_THEME.into());
    }
    request
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn a_themed_create_requires_runner_support() {
        let board = Board::repository(Uuid::from_u128(1));
        let theme = id_bytes(Uuid::from_u128(2));
        let request = request(&board, pb::TaskCreate { theme_id: Some(theme.clone()), ..Default::default() });
        assert!(request.required_capabilities.iter().any(|c| c == capability::TASK_CREATE_THEME));
        assert!(matches!(request.payload, Some(Payload::TaskCreate(p)) if p.theme_id == Some(theme)));
    }
}
