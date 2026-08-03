//! How much of a frame the terminal core costs.
//!
//!     cargo run --release --example bench -p farcooler-vt
//!
//! Worth having because "the terminal feels laggy" is a claim someone will make
//! again, and the first question is whether the emulator is responsible. This
//! answers it without guessing: if feeding and snapshotting are a rounding
//! error against a 120 Hz frame budget of 8.3 ms, the lag is somewhere else —
//! drawing, or the transport — and optimising here would be wasted work.

use std::hint::black_box;
use std::time::Instant;

fn main() {
    // One full-screen redraw from a busy agent: an absolute cursor move and a
    // colour change per row, then a screenful of text. This is the shape of the
    // traffic that actually arrives, not a synthetic stream of plain bytes.
    let mut redraw = Vec::new();
    for row in 1..=40 {
        redraw.extend_from_slice(format!("\x1b[{row};1H\x1b[38;5;{}m", row % 256).as_bytes());
        redraw
            .extend_from_slice("the quick brown fox jumps over the lazy dog ".repeat(2).as_bytes());
    }

    let rounds = 2000u32;
    let mut term = farcooler_vt::Terminal::new(120, 40);

    let start = Instant::now();
    for _ in 0..rounds {
        term.feed(&redraw);
    }
    let feeding = start.elapsed();

    let start = Instant::now();
    for _ in 0..rounds {
        black_box(farcooler_vt::grid::snapshot(&term));
    }
    let snapshotting = start.elapsed();

    let bytes = redraw.len() * rounds as usize;
    let per_frame = feeding / rounds + snapshotting / rounds;

    println!("one redraw is {} bytes over 40 rows", redraw.len());
    println!(
        "feed       {:>10.2?} per redraw  ({:.0} MB/s)",
        feeding / rounds,
        bytes as f64 / feeding.as_secs_f64() / 1e6
    );
    println!("snapshot   {:>10.2?} per frame", snapshotting / rounds);
    println!(
        "together   {:>10.2?}             {:.2}% of an 8.3 ms frame at 120 Hz",
        per_frame,
        per_frame.as_secs_f64() / 0.00833 * 100.0
    );
}
