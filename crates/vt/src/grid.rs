//! A renderer-facing snapshot of the screen.
//!
//! Platform renderers do not reach into the emulator's internals. They ask for
//! a snapshot of plain data and draw it. That keeps the C ABI narrow and means
//! swapping the emulator underneath changes nothing above it.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;

use crate::Terminal;

/// One character cell, already resolved to concrete colours.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub ch: char,
    /// Packed 0xRRGGBB.
    pub fg: u32,
    pub bg: u32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
    /// A double-width character. The following column is its spacer.
    pub wide: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub cells: Vec<Cell>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub columns: u16,
    pub rows: Vec<Row>,
    pub cursor_row: u16,
    pub cursor_column: u16,
    pub cursor_visible: bool,
}

/// Read the current screen.
pub fn snapshot(term: &Terminal) -> Snapshot {
    let t = term.term();
    let grid = t.grid();
    let cols = grid.columns();
    let lines = grid.screen_lines();

    let mut rows = Vec::with_capacity(lines);
    for line in 0..lines {
        let mut cells = Vec::with_capacity(cols);
        for col in 0..cols {
            let c = &grid[Line(line as i32)][Column(col)];
            let flags = c.flags;
            cells.push(Cell {
                ch: c.c,
                fg: resolve(c.fg, true),
                bg: resolve(c.bg, false),
                bold: flags.contains(Flags::BOLD),
                italic: flags.contains(Flags::ITALIC),
                underline: flags.contains(Flags::UNDERLINE),
                inverse: flags.contains(Flags::INVERSE),
                wide: flags.contains(Flags::WIDE_CHAR),
            });
        }
        rows.push(Row { cells });
    }

    let cursor = grid.cursor.point;
    Snapshot {
        columns: cols as u16,
        rows,
        cursor_row: cursor.line.0.max(0) as u16,
        cursor_column: cursor.column.0 as u16,
        cursor_visible: t.mode().contains(alacritty_terminal::term::TermMode::SHOW_CURSOR),
    }
}

/// Resolve a colour to packed RGB.
///
/// Named and indexed colours are resolved HERE rather than in each renderer, so
/// Mac, iOS and Android cannot drift into three different palettes.
fn resolve(color: alacritty_terminal::vte::ansi::Color, foreground: bool) -> u32 {
    use alacritty_terminal::vte::ansi::{Color, NamedColor};

    match color {
        Color::Spec(rgb) => pack(rgb.r, rgb.g, rgb.b),
        Color::Indexed(i) => indexed(i),
        Color::Named(named) => match named {
            NamedColor::Black => indexed(0),
            NamedColor::Red => indexed(1),
            NamedColor::Green => indexed(2),
            NamedColor::Yellow => indexed(3),
            NamedColor::Blue => indexed(4),
            NamedColor::Magenta => indexed(5),
            NamedColor::Cyan => indexed(6),
            NamedColor::White => indexed(7),
            NamedColor::BrightBlack => indexed(8),
            NamedColor::BrightRed => indexed(9),
            NamedColor::BrightGreen => indexed(10),
            NamedColor::BrightYellow => indexed(11),
            NamedColor::BrightBlue => indexed(12),
            NamedColor::BrightMagenta => indexed(13),
            NamedColor::BrightCyan => indexed(14),
            NamedColor::BrightWhite => indexed(15),
            NamedColor::Foreground | NamedColor::BrightForeground => DEFAULT_FG,
            NamedColor::Background => DEFAULT_BG,
            NamedColor::Cursor => DEFAULT_FG,
            _ => {
                if foreground {
                    DEFAULT_FG
                } else {
                    DEFAULT_BG
                }
            }
        },
    }
}

pub const DEFAULT_FG: u32 = 0xDB_E0_E8;
pub const DEFAULT_BG: u32 = 0x12_14_19;

fn pack(r: u8, g: u8, b: u8) -> u32 {
    ((r as u32) << 16) | ((g as u32) << 8) | b as u32
}

/// The xterm 256-colour cube, tuned to stay legible on a dark ground.
fn indexed(i: u8) -> u32 {
    const BASE: [u32; 16] = [
        0x42_47_54, 0xF0_61_66, 0x6B_D1_82, 0xE6_C0_5C, 0x70_A9_F2, 0xCA_8C_F0, 0x5C_C9_D1,
        0xD4_D9_E0, 0x6B_72_82, 0xFF_82_85, 0x8F_EB_A1, 0xFA_DB_7D, 0x94_C2_FF, 0xE1_AE_FF,
        0x82_E5_EB, 0xF5_F7_FA,
    ];

    if i < 16 {
        return BASE[i as usize];
    }
    if i < 232 {
        const STEPS: [u32; 6] = [0, 95, 135, 175, 215, 255];
        let n = (i - 16) as usize;
        return pack(
            STEPS[(n / 36) % 6] as u8,
            STEPS[(n / 6) % 6] as u8,
            STEPS[n % 6] as u8,
        );
    }
    let g = 8 + (i as u32 - 232) * 10;
    pack(g as u8, g as u8, g as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_snapshot_matches_the_grid_size() {
        let t = Terminal::new(30, 9);
        let s = snapshot(&t);
        assert_eq!(s.columns, 30);
        assert_eq!(s.rows.len(), 9);
        assert!(s.rows.iter().all(|r| r.cells.len() == 30));
    }

    #[test]
    fn truecolour_is_preserved_exactly() {
        let mut t = Terminal::new(20, 4);
        t.feed(b"\x1b[38;2;18;52;86mX");
        assert_eq!(snapshot(&t).rows[0].cells[0].fg, 0x12_34_56);
    }

    #[test]
    fn the_256_colour_cube_is_deterministic() {
        // Every renderer must agree, so this is resolved once here.
        assert_eq!(indexed(16), 0x00_00_00);
        assert_eq!(indexed(231), 0xFF_FF_FF);
        assert_eq!(indexed(232), 0x08_08_08);
        assert_eq!(indexed(255), 0xEE_EE_EE);
    }

    #[test]
    fn inverse_is_reported_rather_than_pre_swapped() {
        // The renderer decides how to show inversion; the core just states it.
        let mut t = Terminal::new(20, 4);
        t.feed(b"\x1b[7mX");
        assert!(snapshot(&t).rows[0].cells[0].inverse);
    }
}
