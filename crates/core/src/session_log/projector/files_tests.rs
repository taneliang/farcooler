//! The reader against a file claude is still writing, and the session's
//! files together.

use std::io::Write;

use super::files::{Line, LineReader, MAX_LINE_BYTES};
use super::fixtures::*;
use super::rows::*;
use super::SessionProjector;

fn append(path: &std::path::Path, bytes: &[u8]) {
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
    f.write_all(bytes).unwrap();
}

fn lines(reader: &mut LineReader) -> Vec<String> {
    let mut out = Vec::new();
    reader.read(|line| match line {
        Line::Complete(b) => out.push(String::from_utf8_lossy(b).into_owned()),
        Line::TooLarge(n) => out.push(format!("<too large: {n}>")),
        Line::Restart => {}
    });
    out
}

#[test]
fn a_half_written_line_is_held_until_its_newline() {
    let dir = Scratch::new("half");
    let path = dir.path().join("s.jsonl");
    append(&path, b"{\"a\":1}\n{\"b\":");
    let mut reader = LineReader::new(path.clone());
    assert_eq!(lines(&mut reader), ["{\"a\":1}"]);
    assert_eq!(reader.held_bytes(), 5, "the half line is held, not folded");
    append(&path, b"2}\n");
    assert_eq!(lines(&mut reader), ["{\"b\":2}"], "and arrives whole");
    assert_eq!(reader.held_bytes(), 0);
}

#[test]
fn every_line_of_a_session_written_in_halves_arrives_once_and_whole() {
    // The spike's replay: each line in two writes, a read between them.
    let dir = Scratch::new("halves");
    let path = dir.path().join("s.jsonl");
    std::fs::write(&path, b"").unwrap();
    let mut reader = LineReader::new(path.clone());
    let mut got = Vec::new();
    let mut held = 0;
    for line in EDITS.lines() {
        let bytes = format!("{line}\n").into_bytes();
        let half = bytes.len() / 2;
        append(&path, &bytes[..half]);
        got.extend(lines(&mut reader));
        held += usize::from(reader.held_bytes() > 0);
        append(&path, &bytes[half..]);
        got.extend(lines(&mut reader));
    }
    assert_eq!(got, EDITS.lines().collect::<Vec<_>>());
    assert_eq!(held, EDITS.lines().count(), "every line was caught half-written once");
}

#[test]
fn a_truncated_file_is_read_again_from_its_start() {
    let dir = Scratch::new("trunc");
    let path = dir.path().join("s.jsonl");
    std::fs::write(&path, "{\"n\":1}\n{\"n\":2}\n").unwrap();
    let mut reader = LineReader::new(path.clone());
    assert_eq!(lines(&mut reader).len(), 2);
    std::fs::write(&path, "{\"n\":3}\n").unwrap();
    let mut report = None;
    let mut got = Vec::new();
    report.replace(reader.read(|l| if let Line::Complete(b) = l { got.push(b.to_vec()) }));
    assert!(report.unwrap().rewritten);
    assert_eq!(got, [b"{\"n\":3}".to_vec()]);
}

#[test]
fn a_replaced_file_is_read_again_and_the_projection_says_so() {
    let dir = Scratch::new("replaced");
    let path = dir.path().join("s.jsonl");
    std::fs::write(&path, EDITS).unwrap();
    let mut session = SessionProjector::open(path.clone());
    session.poll();
    let other = dir.path().join("other.jsonl");
    std::fs::write(&other, format!("{EDITS}{COMPACT}")).unwrap();
    std::fs::rename(&other, &path).unwrap();
    session.poll();
    assert!(
        session.projection().rows().iter().any(|r| matches!(&r.kind, RowKind::Gap(g) if g.reason == GapReason::Rewritten)),
        "a rewrite is a visible gap"
    );
}

/// The largest line seen in a real log: 1.35 MB, nearly all of it base64.
fn huge_image_prompt() -> String {
    let data = "iVBORw0KGgo".repeat(1_350_000 / 11);
    format!(
        r#"{{"type":"user","promptId":"pimg","promptSource":"typed","timestamp":"2026-10-06T16:00:00.000Z","message":{{"role":"user","content":[{{"type":"text","text":"[Image #1] what is this?"}},{{"type":"image","source":{{"type":"base64","media_type":"image/png","data":"{data}"}}}}]}}}}"#
    )
}

#[test]
fn a_one_point_three_five_megabyte_line_folds_without_its_image() {
    let line = huge_image_prompt();
    assert!(line.len() > 1_350_000);
    let dir = Scratch::new("huge");
    let path = dir.path().join("s.jsonl");
    // Written in pieces, as a slow writer would.
    std::fs::write(&path, b"").unwrap();
    let mut session = SessionProjector::open(path.clone());
    for piece in line.as_bytes().chunks(300_000) {
        append(&path, piece);
        session.poll();
    }
    append(&path, b"\n");
    session.poll();
    let p = session.projection();
    assert_eq!(turn(p, "turn:pimg").prompt, "[Image #1] what is this?");
    assert_eq!(p.stats().gaps, 0);
}

#[test]
fn a_line_over_the_cap_streams_past_as_one_gap_and_the_next_line_still_reads() {
    let dir = Scratch::new("overcap");
    let path = dir.path().join("s.jsonl");
    std::fs::write(&path, b"").unwrap();
    let mut session = SessionProjector::open(path.clone());
    append(&path, EDITS.lines().next().unwrap().as_bytes());
    append(&path, b"\n{\"type\":\"user\",\"x\":\"");
    let filler = vec![b'a'; 1024 * 1024];
    let mut peak = 0;
    for _ in 0..(MAX_LINE_BYTES / filler.len() + 2) {
        append(&path, &filler);
        session.poll();
        peak = peak.max(session.main_held_bytes());
    }
    append(&path, b"\"}\n");
    append(&path, COMPACT.lines().next().unwrap().as_bytes());
    append(&path, b"\n");
    session.poll();
    assert!(peak <= MAX_LINE_BYTES, "never held whole: {peak}");
    let p = session.projection();
    assert!(p.rows().iter().any(|r| matches!(&r.kind, RowKind::Gap(g) if g.reason == GapReason::TooLarge)));
    assert!(p.row("turn:p1").is_some(), "the line after it is read");
}

#[test]
fn a_session_joins_its_subagent_files_by_their_meta() {
    let dir = Scratch::new("bg");
    let main = write_background(dir.path());
    let mut session = SessionProjector::open(main);
    session.poll();
    let p = session.projection();
    let bg = sub(p, "sub:toolu_bg");
    assert_eq!(bg.current_action, "Bash Count README lines");
    assert_eq!(bg.tool_count, 1);
    assert_eq!(bg.last_ms, Some(ms("10:00:50.000")), "its own newest record");
    let fg = sub(p, "sub:toolu_fg");
    assert_eq!(fg.tool_count, 2, "the sidechain's two calls are the result's two, not four");
    assert_eq!(fg.current_action, "Grep TODO");
}

#[test]
fn a_foreground_agents_sidechain_joins_by_meta_before_its_result_names_it() {
    // The parent's result names the agentId only when a foreground agent ENDS;
    // until then the meta file is the only join.
    let dir = Scratch::new("fgjoin");
    let main = write_background(dir.path());
    let upto: String = BACKGROUND.lines().take_while(|l| !l.contains("\"agentId\":\"afg1\"")).map(|l| format!("{l}\n")).collect();
    std::fs::write(&main, upto).unwrap();
    let mut session = SessionProjector::open(main);
    session.poll();
    let fg = sub(session.projection(), "sub:toolu_fg");
    assert_eq!(fg.agent_id.as_deref(), Some("afg1"));
    assert_eq!(fg.status, SubagentState::Running);
    assert_eq!(fg.current_action, "Grep TODO", "live from its own file");
}

#[test]
fn a_sidechain_seen_before_its_launch_is_kept_and_given_to_the_row_later() {
    let dir = Scratch::new("orphan");
    let main = write_background(dir.path());
    let first: String = BACKGROUND.lines().take(3).map(|l| format!("{l}\n")).collect();
    std::fs::write(&main, &first).unwrap();
    let mut session = SessionProjector::open(main.clone());
    session.poll();
    assert!(session.projection().row("sub:toolu_bg").is_none());
    std::fs::write(&main, BACKGROUND).unwrap();
    session.poll();
    let bg = sub(session.projection(), "sub:toolu_bg");
    assert_eq!(bg.current_action, "Bash Count README lines", "what it did before the join");
}

#[test]
fn a_rebind_reads_the_new_session_and_keeps_the_old_rows() {
    let dir = Scratch::new("rebind");
    let before = dir.path().join("before.jsonl");
    let after = dir.path().join("after.jsonl");
    std::fs::write(&before, CLEARED_BEFORE).unwrap();
    std::fs::write(&after, CLEARED_AFTER).unwrap();
    let mut session = SessionProjector::open(before);
    session.poll();
    session.rebind(after);
    session.poll();
    let p = session.projection();
    assert!(p.row("turn:p1").is_some() && p.row("turn:p9").is_some());
    assert!(p.rows().iter().any(|r| matches!(&r.kind, RowKind::Notice(n) if n.text == "/clear")));
}
