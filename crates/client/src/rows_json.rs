//! A terminal's agent rows as JSON, the shape every client reads (ov-366,
//! ov-776): the phones and the Mac through `ffi/rows_args.rs`, and the browser
//! through crates/web, which has no FFI. One place, so a page and a follow
//! can't read differently in the browser than on a phone.
//!
//! Each row is the object `projector::Row` serializes to, not a string of
//! JSON, so a client decodes rows in the same pass as the page.

use serde_json::{Value, json};

fn row_json(row: &farcooler_protocol::v1::AgentRow) -> Value {
    serde_json::from_str(&row.row_json).unwrap_or_else(|_| json!({ "id": row.id, "ord": row.ord, "rev": row.rev }))
}

/// A page as an app reads it: `{epoch, rev, moreBefore, rows: [row]}`.
pub fn rows_page_json(page: &farcooler_protocol::v1::AgentRowPage) -> Value {
    json!({
        "epoch": page.epoch,
        "rev": page.rev,
        "moreBefore": page.more_before,
        "rows": page.rows.iter().map(row_json).collect::<Vec<_>>(),
    })
}

/// A follow as an app reads it: `{epoch, rev, reset, changes: [{kind, id, rev, row?}]}`,
/// `kind` one of `insert`, `update` and `remove` (which carries no row).
pub fn changes_json(follow: &farcooler_protocol::v1::AgentRowChanges) -> Value {
    use farcooler_protocol::v1::AgentRowChangeKind as Kind;
    let changes: Vec<Value> = follow
        .changes
        .iter()
        .map(|change| {
            let kind = match Kind::try_from(change.kind) {
                Ok(Kind::Insert) => "insert",
                Ok(Kind::Remove) => "remove",
                _ => "update",
            };
            let mut out = json!({ "kind": kind, "id": change.id, "rev": change.rev });
            if let Some(row) = &change.row {
                out["row"] = row_json(row);
            }
            out
        })
        .collect();
    json!({ "epoch": follow.epoch, "rev": follow.rev, "reset": follow.reset, "changes": changes })
}

