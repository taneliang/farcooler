//! A picture attached to an agent prompt, by its file name.

/// The image type, from the file's extension.
///
/// Not sniffed from the bytes: the adapter needs a MIME type to decode with,
/// every real attachment comes from a picker that named it, and a wrong guess
/// here fails loudly at the far end rather than corrupting anything.
pub(crate) fn mime_for(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_lowercase().as_str() {
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" => "image/heic",
        _ => "image/jpeg",
    }
}

/// Each file at `paths` as a prompt's image block, typed by `mime_for`.
pub(crate) fn image_blocks(paths: &[std::path::PathBuf]) -> std::io::Result<Vec<farcooler_protocol::v1::AgentPromptBlock>> {
    use farcooler_protocol::v1::{AgentPromptBlock, ImageBlock, agent_prompt_block::Content};
    let mut blocks = Vec::new();
    for path in paths {
        let data = std::fs::read(path)?;
        let image = ImageBlock { mime_type: mime_for(path).to_string(), data: bytes::Bytes::from(data) };
        blocks.push(AgentPromptBlock { content: Some(Content::Image(image)) });
    }
    Ok(blocks)
}
