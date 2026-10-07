//! A call's `images` argument, `[{ "mime": "image/png", "base64": "..." }]`,
//! decoded here so the protocol carries bytes and only this boundary deals
//! in text: `terminal.agent_prompt`'s and `terminal.compose`'s.

/// Each image's MIME type and bytes; an item missing either, or not base64,
/// is left out.
pub(super) fn images(args: &serde_json::Value) -> Vec<(String, Vec<u8>)> {
    let Some(items) = args.get("images").and_then(|v| v.as_array()) else { return Vec::new() };
    items
        .iter()
        .filter_map(|item| {
            let mime = item.get("mime")?.as_str()?.to_string();
            let data = farcooler_core::base64::decode(item.get("base64")?.as_str()?)?;
            Some((mime, data))
        })
        .collect()
}
