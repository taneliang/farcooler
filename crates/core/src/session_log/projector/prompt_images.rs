//! A prompt's images, read back from its transcript record (ov-454).
//!
//! The fold keeps only each image's type on the `Turn` row and where the
//! record is (`Turn::source`), so a long session's screenshots never sit in
//! memory or ride a page of rows. A view asks for one image, and this reads
//! that one line again and decodes the image it names.

use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};

use super::files::MAX_LINE_BYTES;
use super::rows::RecordAt;

/// Image `index` of the prompt record at `source`, as its MIME type and
/// bytes. `None` when the line there isn't that prompt's (`prompt_id`, the
/// file rewritten since), has no such image, or can't be read.
pub fn read(source: &RecordAt, prompt_id: &str, index: usize) -> Option<(String, Vec<u8>)> {
    let mut file = std::fs::File::open(&source.path).ok()?;
    file.seek(SeekFrom::Start(source.at)).ok()?;
    let mut line = Vec::new();
    BufReader::new(file).take(MAX_LINE_BYTES as u64 + 1).read_until(b'\n', &mut line).ok()?;
    let record: serde_json::Value = serde_json::from_slice(&line).ok()?;
    if record.get("promptId").and_then(|p| p.as_str()) != Some(prompt_id) {
        return None;
    }
    let blocks = record.get("message")?.get("content")?.as_array()?;
    let image = blocks.iter().filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("image")).nth(index)?;
    let source = image.get("source")?;
    let mime = source.get("media_type").and_then(|m| m.as_str()).unwrap_or_default().to_string();
    let bytes = crate::base64::decode(source.get("data")?.as_str()?)?;
    Some((mime, bytes))
}
