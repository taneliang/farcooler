//! `terminal rows`: a terminal's agent rows (ov-366), a page and then, with
//! `--follow`, every change, as JSON lines on one open link.
//!
//! The page is one line, `{"page": {...}}`; each follow that changed
//! something is one line, `{"changes": {...}}`, in the shapes the FFI gives
//! the apps (`ffi/rows_args.rs`). A reset (the runner restarted, or the pane
//! moved to another conversation) prints a fresh page. Served only by a
//! runner with `FARCOOLER_PROJECTOR=1` until a client reads it (ov-372).

use std::io::Write;

use farcooler_protocol::capability;
use farcooler_protocol::v1::{AgentRow, AgentRowChangeKind, AgentRowChanges, AgentRowPage, request, result};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::daemon_link::Link;
use crate::{expect_value, id_bytes, req, with};

/// How long the runner holds each follow while nothing changes.
const WAIT_MS: u32 = 20_000;

fn row(row: &AgentRow) -> Value {
    serde_json::from_str(&row.row_json).unwrap_or_else(|_| json!({ "id": row.id, "ord": row.ord, "rev": row.rev }))
}

/// The line for a page.
pub(crate) fn page_line(page: &AgentRowPage) -> String {
    json!({ "page": {
        "epoch": page.epoch,
        "rev": page.rev,
        "moreBefore": page.more_before,
        "rows": page.rows.iter().map(row).collect::<Vec<_>>(),
    }})
    .to_string()
}

/// The line for a follow.
pub(crate) fn changes_line(follow: &AgentRowChanges) -> String {
    let changes: Vec<Value> = follow
        .changes
        .iter()
        .map(|c| {
            let kind = match AgentRowChangeKind::try_from(c.kind) {
                Ok(AgentRowChangeKind::Insert) => "insert",
                Ok(AgentRowChangeKind::Remove) => "remove",
                _ => "update",
            };
            let mut out = json!({ "kind": kind, "id": c.id, "rev": c.rev });
            if let Some(r) = &c.row {
                out["row"] = row(r);
            }
            out
        })
        .collect();
    json!({ "changes": { "epoch": follow.epoch, "rev": follow.rev, "reset": follow.reset, "changes": changes } }).to_string()
}

async fn page(link: &mut Link, id: Uuid, before: Option<u64>, limit: u32) -> Result<AgentRowPage, Box<dyn std::error::Error>> {
    let mut ask = with(req("agent.rows"), request::Payload::AgentRowsPage(farcooler_protocol::v1::AgentRowsPage { terminal_id: id_bytes(id), before, limit }));
    ask.required_capabilities.push(capability::AGENT_ROWS.to_string());
    match expect_value(link.call(ask).await?.value)? {
        result::Value::AgentRowPage(page) => Ok(page),
        _ => Err(crate::daemon_link::UNREADABLE.into()),
    }
}

/// Print a page, then follow until the link fails or the reader goes away.
pub(crate) async fn run(mut link: Link, id: Uuid, before: Option<u64>, limit: u32, follow: bool) -> Result<(), Box<dyn std::error::Error>> {
    let mut out = std::io::stdout().lock();
    let first = page(&mut link, id, before, limit).await?;
    writeln!(out, "{}", page_line(&first))?;
    if !follow {
        return Ok(());
    }
    let (mut epoch, mut rev) = (first.epoch, first.rev);
    loop {
        let mut ask = with(
            req("agent.rows_follow"),
            request::Payload::AgentRowsFollow(farcooler_protocol::v1::AgentRowsFollow { terminal_id: id_bytes(id), epoch, after_rev: rev, wait_ms: WAIT_MS }),
        );
        ask.required_capabilities.push(capability::AGENT_ROWS.to_string());
        let result::Value::AgentRowChanges(changes) = expect_value(link.call(ask).await?.value)? else {
            return Err(crate::daemon_link::UNREADABLE.into());
        };
        let line = if changes.reset {
            let fresh = page(&mut link, id, None, limit).await?;
            (epoch, rev) = (fresh.epoch, fresh.rev);
            page_line(&fresh)
        } else if changes.changes.is_empty() {
            continue;
        } else {
            rev = changes.rev;
            changes_line(&changes)
        };
        if writeln!(out, "{line}").is_err() || out.flush().is_err() {
            // Nobody is reading any more.
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_protocol::v1::AgentRowChange;

    #[test]
    fn a_change_line_names_each_row_by_id_and_kind() {
        let follow = AgentRowChanges {
            epoch: 3,
            rev: 9,
            changes: vec![
                AgentRowChange { kind: AgentRowChangeKind::Insert as i32, id: "prose:a:0".into(), rev: 9, row: Some(AgentRow { id: "prose:a:0".into(), ord: 4, rev: 9, row_json: r#"{"id":"prose:a:0","ord":4}"#.into() }) },
                AgentRowChange { kind: AgentRowChangeKind::Remove as i32, id: "hprose:m".into(), rev: 8, row: None },
            ],
            ..Default::default()
        };
        let line: Value = serde_json::from_str(&changes_line(&follow)).unwrap();
        assert_eq!(line["changes"]["changes"][0]["kind"], "insert");
        assert_eq!(line["changes"]["changes"][0]["row"]["ord"], 4, "the row as an object, not a string");
        assert_eq!(line["changes"]["changes"][1]["kind"], "remove");
        assert!(line["changes"]["changes"][1].get("row").is_none());
        assert_eq!(line["changes"]["rev"], 9);
    }
}
