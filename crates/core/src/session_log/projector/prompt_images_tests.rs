//! A prompt's images on its turn row, and read back from its line (ov-454).

use super::fixtures::Scratch;
use super::prompt_images::read;
use super::{LineReader, RowKind, SessionProjector};

/// A transcript with a prompt carrying two images, a line before it so
/// it doesn't start at 0, and the projector's own read of it.
fn session(dir: &std::path::Path) -> (std::path::PathBuf, SessionProjector) {
    let path = dir.join("0199aaaa-0000-7000-8000-00000000abcd.jsonl");
    let png = crate::base64::encode(b"\x89PNG\r\n\x1a\nfirst");
    let jpeg = crate::base64::encode(b"\xff\xd8\xffsecond");
    let lines = [
        r#"{"type":"system","subtype":"init","uuid":"u0","timestamp":"2026-10-10T17:00:00.000Z"}"#.to_string(),
        format!(
            r#"{{"type":"user","uuid":"u1","promptId":"p1","promptSource":"typed","timestamp":"2026-10-10T17:00:01.000Z","imagePasteIds":[60,61],"message":{{"role":"user","content":[{{"type":"text","text":"look [Image #60] [Image #61]"}},{{"type":"image","source":{{"type":"base64","media_type":"image/png","data":"{png}"}}}},{{"type":"image","source":{{"type":"base64","media_type":"image/jpeg","data":"{jpeg}"}}}}]}}}}"#
        ),
        r#"{"type":"user","uuid":"u2","promptId":"p2","promptSource":"typed","timestamp":"2026-10-10T17:00:02.000Z","message":{"role":"user","content":"no pictures"}}"#.to_string(),
    ];
    std::fs::write(&path, lines.join("\n") + "\n").expect("written");
    let mut projector = SessionProjector::open(path.clone());
    projector.poll();
    (path, projector)
}

fn turn<'p>(projector: &'p SessionProjector, id: &str) -> &'p super::Turn {
    match &projector.projection().row(id).expect("the turn").kind {
        RowKind::Turn(turn) => turn,
        other => panic!("not a turn: {other:?}"),
    }
}

#[test]
fn a_prompts_images_are_on_its_row_and_read_back_from_its_line() {
    let dir = Scratch::new("images");
    let (path, projector) = session(dir.path());
    let first = turn(&projector, "turn:p1");
    assert_eq!(first.images.iter().map(|i| i.mime.as_str()).collect::<Vec<_>>(), ["image/png", "image/jpeg"]);
    let source = first.source.clone().expect("where the prompt is");
    assert_eq!(&*source.path, path.as_path());
    assert!(source.at > 0, "past the first line");
    assert_eq!(read(&source, "p1", 0), Some(("image/png".into(), b"\x89PNG\r\n\x1a\nfirst".to_vec())));
    assert_eq!(read(&source, "p1", 1), Some(("image/jpeg".into(), b"\xff\xd8\xffsecond".to_vec())));
    assert_eq!(read(&source, "p1", 2), None, "no third image");
    assert_eq!(read(&source, "p2", 0), None, "another prompt's id");

    let second = turn(&projector, "turn:p2");
    assert!(second.images.is_empty() && second.source.is_none());
    let json = serde_json::to_value(&projector.projection().row("turn:p1").expect("the row").kind).expect("json");
    assert_eq!(json["Turn"]["images"], serde_json::json!([{ "mime": "image/png" }, { "mime": "image/jpeg" }]));
    assert!(json["Turn"].get("source").is_none(), "where the file is stays on the runner");
}

/// A line folded in two reads, its first half held: its start is where
/// the held half began, not where the second read did.
#[test]
fn a_line_read_in_two_halves_starts_where_its_first_half_did() {
    let dir = Scratch::new("images");
    let path = dir.path().join("0199aaaa-0000-7000-8000-00000000abce.jsonl");
    std::fs::write(&path, "{\"a\":1}\n{\"b\":").expect("written");
    let mut reader = LineReader::new(path.clone());
    let mut starts = Vec::new();
    reader.read_at(|at, _| starts.push(at));
    let mut file = std::fs::OpenOptions::new().append(true).open(&path).expect("open");
    std::io::Write::write_all(&mut file, b"2}\n{\"c\":3}\n").expect("appended");
    reader.read_at(|at, _| starts.push(at));
    assert_eq!(starts, [0, 8, 16]);
}
