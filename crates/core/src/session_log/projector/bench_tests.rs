//! What a fold costs per line, on a large replayed corpus.
//!
//! The spike measured 23-26 µs per line in Python on the 1,562-line recorded
//! session with 8 background subagents. The bound here is the release-build
//! budget the design sets, 25 µs, and a looser one for the debug build that
//! `cargo test` runs, which is several times slower at JSON.

use std::io::Write;
use std::time::Instant;

use super::fixtures::*;
use super::SessionProjector;

/// Every fixture's lines, renamed per copy so ids stay unique, with an
/// attachment of the size real sessions carry between them (2 KB is the
/// recorded corpus's mean line).
fn corpus(copies: usize) -> String {
    let attachment = format!(
        r#"{{"type":"attachment","attachment":{{"type":"skill_listing","content":"{}"}},"timestamp":"2026-10-06T10:00:00.000Z"}}"#,
        "x".repeat(8 * 1024)
    );
    let mut out = String::new();
    for n in 0..copies {
        for text in [BACKGROUND, EDITS, COMPACT, RECORDED] {
            for line in text.lines() {
                let line = line
                    .replace("\"p1\"", &format!("\"p1-{n}\""))
                    .replace("\"p2\"", &format!("\"p2-{n}\""))
                    .replace("toolu_", &format!("toolu_{n}_"))
                    .replace("abg1", &format!("abg1x{n}"))
                    .replace("afg1", &format!("afg1x{n}"));
                out.push_str(&line);
                out.push('\n');
            }
            out.push_str(&attachment);
            out.push('\n');
        }
    }
    out
}

fn budget_us() -> f64 {
    if cfg!(debug_assertions) { 400.0 } else { 25.0 }
}

#[test]
fn folding_a_large_corpus_costs_a_few_microseconds_a_line() {
    let text = corpus(80);
    let lines = text.lines().count();
    assert!(lines > 5_000, "{lines}");
    let mut p = super::Projection::new();
    let start = Instant::now();
    for line in text.lines() {
        p.fold_line(line.as_bytes());
    }
    let per_line = start.elapsed().as_secs_f64() * 1e6 / lines as f64;
    eprintln!("fold: {lines} lines, {} MB, {per_line:.1} µs/line", text.len() / 1_000_000);
    assert!(per_line < budget_us(), "{per_line:.1} µs per line");
    assert_eq!(p.stats().gaps, 0);
}

#[test]
fn a_replay_written_in_halves_folds_every_line_once_inside_the_budget() {
    let text = corpus(20);
    let dir = Scratch::new("bench-replay");
    let path = dir.path().join("s.jsonl");
    std::fs::write(&path, b"").unwrap();
    let mut session = SessionProjector::open(path.clone());
    let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
    let mut folded = 0;
    let mut spent = std::time::Duration::ZERO;
    for line in text.lines() {
        let bytes = format!("{line}\n").into_bytes();
        let half = bytes.len() / 2;
        file.write_all(&bytes[..half]).unwrap();
        let start = Instant::now();
        folded += session.poll();
        file.write_all(&bytes[half..]).unwrap();
        folded += session.poll();
        spent += start.elapsed();
    }
    let lines = text.lines().count() as u64;
    assert_eq!(folded, lines, "every line once");
    assert!(session.held_back >= lines, "each line was caught half-written");
    // Two reads a line here (a stat, an open, a read), which the daemon does
    // once per filesystem event rather than per half-line.
    let per_line = spent.as_secs_f64() * 1e6 / lines as f64;
    eprintln!("replay: {lines} lines in halves, {per_line:.1} µs/line including two reads");
    assert!(per_line < budget_us() * 4.0, "{per_line:.1} µs per line");
}

/// A recorded session, when one is named: `FARCOOLER_PROJECTOR_SESSION` is a
/// main transcript with its `subagents/` beside it. Ignored by default; run
/// in release for the number the report gives.
#[test]
#[ignore]
fn folding_a_recorded_session() {
    let Some(path) = std::env::var_os("FARCOOLER_PROJECTOR_SESSION") else { return };
    let start = Instant::now();
    let mut session = SessionProjector::open(path.into());
    session.poll();
    let elapsed = start.elapsed();
    let stats = session.projection().stats();
    eprintln!(
        "recorded: {} lines, {:.1} MB, {:.1} ms, {:.1} µs/line, {} gaps, {} rows",
        stats.lines,
        stats.bytes as f64 / 1e6,
        elapsed.as_secs_f64() * 1e3,
        elapsed.as_secs_f64() * 1e6 / stats.lines.max(1) as f64,
        stats.gaps,
        session.projection().rows().len()
    );
    for row in session.projection().rows() {
        if let super::RowKind::Subagent(sub) = &row.kind {
            let run = sub.ended_ms.zip(sub.started_ms).map(|(e, s)| (e - s) / 1000);
            eprintln!("  {:?} bg={} {:?}s tools={} {} :: {}", sub.status, sub.background, run, sub.tool_count, sub.description, sub.current_action);
        }
    }
}
