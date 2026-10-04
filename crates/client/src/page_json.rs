//! Orchestrator pages as JSON (ov-269, ov-282): `page.list` and `page.get` in
//! the shape the apps decode, which is the shape `farcooler page show --json`
//! prints, so the Mac and the phones read one fixture, `test/fixtures/page.json`.
//!
//! EXPERIMENTAL, behind `board_pages`, and removable with the rest of the
//! feature. The phones read pages and never write them: the orchestrator is
//! their one writer.
//!
//! The document passes through unchanged: `doc` is `doc_json` parsed, not read.
//! What a block means is the renderer's, held to `test/fixtures/pages/`.

use farcooler_protocol::v1 as pb;
use serde_json::{Value, json};

use crate::session::{short, uuid_of};

/// One page. `doc` is `null` for a page read without its document.
pub fn page_json(page: &pb::BoardPage) -> Value {
    json!({
        "id": uuid_of(&page.id).to_string(),
        "short": short(&page.id),
        "slot": page.slot,
        "title": page.title,
        "summary": page.summary,
        "anchor_kind": page.anchor_kind,
        "anchor": page.anchor,
        "revision": page.revision,
        "ordinal": page.ordinal,
        "actor": page.actor,
        "updated_at_ms": page.updated_at_ms,
        "doc": if page.doc_json.is_empty() { Value::Null } else { serde_json::from_str(&page.doc_json).unwrap_or(Value::Null) },
    })
}

/// `page.list`'s answer: `{"pages": [...]}` in the order the runner lists them.
pub fn pages_json(list: &pb::BoardPageList) -> Value {
    json!({ "pages": list.pages.iter().map(page_json).collect::<Vec<_>>() })
}

#[cfg(test)]
#[path = "page_json_tests.rs"]
mod tests;
