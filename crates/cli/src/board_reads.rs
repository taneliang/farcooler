//! `farcooler board mark-read`: raise what is read on one board (ov-113).
//!
//! What has been read on a board lives on the runner, so every device sees the
//! same Unread. The Mac reaches it through this command, as it reaches the
//! rest of the board. Each mark and the floor only rise, so running it twice,
//! or from two devices out of order, ends in the same state.
//!
//! Every time is the runner's clock in Unix milliseconds, read from what the
//! caller was told (a ticket's `last_moved`, a note's `at`). There is no
//! "now" here, on purpose: a device's own clock is the one thing a shared
//! mark must never carry.

use clap::Subcommand;
use farcooler_client::session::reads_json;
use farcooler_protocol::v1::{self as pb, request, result};
use farcooler_transport::ClientError;
use uuid::Uuid;

use crate::tasks::{Board, DispatchLink, Refused, board_for, refusal};
use crate::{Fallible, connect_to, expect_value, id_bytes, req_for, with};

/// What a runner without `board_reads` is told.
const NO_READS: &str = "this runner's Far Cooler is older than shared read state. update it and try again";

/// `farcooler board`'s subcommands.
#[derive(Debug, Clone, Subcommand)]
pub enum BoardCmd {
    /// Raise what is read on a board, and print the board's read state.
    ///
    /// `--task ID:MS` says a ticket was opened, with the runner-clock time
    /// it was seen through. `--floor MS` is Mark All as Read: everything at or
    /// before it counts as read. `--seed` marks a floor as this device's
    /// pre-sync one, which replaces the runner's first-look default once. Every
    /// time only ever rises, so a repeat is harmless.
    MarkRead {
        /// Which repository's board, when two have the workspace's name.
        #[arg(long)]
        repo: Option<String>,
        /// The board, by name or task prefix.
        #[arg(long)]
        workspace: Option<String>,
        /// A ticket opened, as `<task id>:<milliseconds>`. Repeat for more.
        #[arg(long = "task", value_name = "ID:MS")]
        tasks: Vec<String>,
        /// Everything at or before this time counts as read.
        #[arg(long, value_name = "MS")]
        floor: Option<i64>,
        /// The floor is this device's pre-sync one.
        #[arg(long, requires = "floor")]
        seed: bool,
    },
}

pub async fn board(runner: Option<&str>, cmd: BoardCmd, json: bool) -> Fallible {
    let BoardCmd::MarkRead { repo, workspace, tasks, floor, seed } = cmd;
    let opened = tasks.iter().map(|t| parse_mark(t)).collect::<Result<Vec<_>, _>>()?;
    if opened.is_empty() && floor.is_none() {
        return Err("name what was read: a ticket with --task, or everything before a time with --floor".into());
    }
    let mut link = connect_to(runner).await?;
    let board = board_for(&mut link, repo.as_deref(), workspace.as_deref(), std::env::var(crate::workspaces::WORKSPACE_ENV).ok())
        .await?;
    let reads = mark_read(&mut link, &board, floor, opened, seed).await?;
    if json {
        println!("{}", reads_json(&reads));
    } else {
        println!("marked read: {} ticket(s) above the floor on this board", reads.opened.len());
    }
    Ok(())
}

/// `<task id>:<milliseconds>`, as a mark.
fn parse_mark(text: &str) -> Result<(Uuid, i64), Box<dyn std::error::Error>> {
    let bad = || format!("{text:?} isn't a mark. write it as <task id>:<milliseconds>");
    let (task, ms) = text.split_once(':').ok_or_else(bad)?;
    let task = task.parse::<Uuid>().map_err(|_| bad())?;
    let ms = ms.parse::<i64>().ok().filter(|ms| *ms >= 0).ok_or_else(bad)?;
    Ok((task, ms))
}

/// `workspace.mark_read` on `board`, answering with the board's state after
/// the merge.
///
/// Refused here, without a round trip, for a runner without `board_reads` and
/// for a board that is no workspace's.
async fn mark_read<L: DispatchLink>(
    link: &mut L,
    board: &Board,
    floor_ms: Option<i64>,
    opened: Vec<(Uuid, i64)>,
    seeds_floor: bool,
) -> Result<pb::BoardReads, Box<dyn std::error::Error>> {
    if !link.capabilities().iter().any(|c| c == farcooler_protocol::capability::BOARD_READS) {
        return Err(Box::new(Refused::new(NO_READS.to_string(), Some(pb::ErrorCode::CapabilityUnsupported as i32))));
    }
    let Some(workspace) = &board.workspace else {
        return Err("name a board with --workspace".into());
    };
    let mut r = with(
        req_for("workspace.mark_read", crate::uuid_of(&workspace.id)),
        request::Payload::WorkspaceMarkRead(pb::WorkspaceMarkRead {
            workspace_id: workspace.id.clone(),
            floor_ms,
            opened: opened
                .into_iter()
                .map(|(task, opened_ms)| pb::TaskRead { task_id: id_bytes(task), opened_ms })
                .collect(),
            seeds_floor,
        }),
    );
    r.required_capabilities.push(farcooler_protocol::capability::BOARD_READS.to_string());
    let answer = link.call(r).await.map_err(|e| match e {
        // Not `other_board`'s sentence for a line: that one is about lines.
        ClientError::Daemon { what, .. } if what == "other_board" => {
            Box::new(Refused::new("those tickets aren't all on this board".to_string(), None)) as Box<dyn std::error::Error>
        }
        other => Box::new(refusal(other, "the runner couldn't record that. try again")),
    })?;
    match expect_value(answer.value)? {
        result::Value::BoardReads(reads) => Ok(reads),
        _ => Err(crate::daemon_link::UNREADABLE.into()),
    }
}

#[cfg(test)]
#[path = "board_reads_tests.rs"]
mod tests;
