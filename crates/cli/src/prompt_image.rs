//! `terminal prompt-image` (ov-454): one image a prompt carried, as the
//! conversation view's thumbnail fetches it (`agent.image`), written to a
//! file: what a turn row's `images` name, read back from the agent's
//! transcript on the runner, a piece at a time.

use farcooler_protocol::v1::{self as pb, request};

use super::{id_bytes, req, terminal_by_record, with};

/// Image `index` of turn row `row` in `terminal`, written to `out`; prints
/// its MIME type and size, or under `json`, `{"mime":…,"bytes":…}`.
pub(crate) async fn run(runner: Option<&str>, terminal: &str, row: &str, index: u32, out: &std::path::Path, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let (mut link, id) = terminal_by_record(runner, terminal).await?;
    let mut bytes = Vec::new();
    let mime = loop {
        let ask = pb::AgentImageRequest { terminal_id: id_bytes(id), row_id: row.to_string(), index, offset: bytes.len() as u64 };
        let answer = link.call(with(req("agent.image"), request::Payload::AgentImage(ask))).await?;
        let Some(pb::result::Value::AgentImage(piece)) = answer.value else {
            return Err("the runner answered something other than an image".into());
        };
        if piece.offset != bytes.len() as u64 || (piece.chunk.is_empty() && (bytes.len() as u64) < piece.total_size) {
            return Err("the runner's image changed while it was read".into());
        }
        bytes.extend_from_slice(&piece.chunk);
        if bytes.len() as u64 >= piece.total_size {
            break piece.mime_type;
        }
    };
    std::fs::write(out, &bytes)?;
    if json {
        println!("{}", serde_json::json!({ "mime": mime, "bytes": bytes.len() }));
    } else {
        println!("{mime}, {} bytes, written to {}", bytes.len(), out.display());
    }
    Ok(())
}
