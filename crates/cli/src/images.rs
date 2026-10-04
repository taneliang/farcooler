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
