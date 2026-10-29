//! Themes in the shape every client decodes.

use serde_json::{Value, json};

/// Themes as every client already decodes them, so a write's answer and
/// `themes`'s answer are the same shape.
pub(super) fn theme_json(items: &[farcooler_protocol::v1::Theme]) -> Value {
    json!({
        "themes": items.iter().map(|t| json!({
            "name": t.name,
            "dark": t.dark,
            "background": t.background,
            "foreground": t.foreground,
            "cursor": t.cursor,
            "ansi": t.ansi,
        })).collect::<Vec<_>>()
    })
}
