//! Render a byte stream to text, for checking the core against real output.
//!
//!     farcooler terminal stream <id> | farcooler-render 120 40
//!
//! Prints what the emulator believes is on screen. This is the same path the
//! app draws, minus the pixels, so a screen that comes out right here comes out
//! right there — and it can be checked without looking at a window.

use std::io::Read;

use farcooler_vt::grid::snapshot;
use farcooler_vt::Terminal;

fn main() {
    let mut args = std::env::args().skip(1);
    let columns: u16 = args.next().and_then(|a| a.parse().ok()).unwrap_or(120);
    let rows: u16 = args.next().and_then(|a| a.parse().ok()).unwrap_or(40);

    let mut term = Terminal::new(columns, rows);
    let mut stdin = std::io::stdin().lock();
    let mut buffer = [0u8; 8192];
    let mut total = 0usize;

    // Read to EOF: the caller decides how long to stream.
    loop {
        match stdin.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                total += n;
                term.feed(&buffer[..n]);
            }
        }
    }

    let snap = snapshot(&term);
    println!("── {}×{}, {total} bytes fed ──", snap.columns, snap.rows.len());
    for (i, row) in snap.rows.iter().enumerate() {
        let text: String = row.cells.iter().map(|c| c.ch).collect();
        println!("{i:>3} │{}│", text.trim_end());
    }
    println!(
        "cursor {},{} visible={} · history={} · alt={}",
        snap.cursor_row,
        snap.cursor_column,
        snap.cursor_visible,
        snap.history_size,
        term.mode().contains(alacritty_terminal::term::TermMode::ALT_SCREEN),
    );

    // Color is the other half of correctness, and a text dump hides it.
    let colored = snap
        .rows
        .iter()
        .flat_map(|r| r.cells.iter())
        .filter(|c| c.fg != farcooler_vt::grid::DEFAULT_FG)
        .count();
    let styled = snap.rows.iter().flat_map(|r| r.cells.iter()).filter(|c| c.bold).count();
    println!("{colored} cells carry a non-default color, {styled} are bold");
}
