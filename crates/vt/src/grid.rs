//! A renderer-facing snapshot of the screen.
//!
//! Platform renderers do not reach into the emulator's internals. They ask for
//! a snapshot of plain data and draw it. That keeps the C ABI narrow and means
//! swapping the emulator underneath changes nothing above it.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;

use crate::Terminal;

/// One character cell, already resolved to concrete colors.
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
    /// How far the view is scrolled back, in lines. Zero means live.
    pub display_offset: u32,
    /// Lines available above the screen. The client draws its scrollbar from
    /// this rather than counting rows itself.
    pub history_size: u32,
}

/// The nineteen colours a theme decides for a terminal.
///
/// Held here rather than taken from `farcooler-core`, which is where a `Theme`
/// lives: this crate is a leaf, and it is compiled into a shared library that
/// ships inside the iOS and Android apps. Depending on `core` to name nineteen
/// integers would pull a protobuf runtime into both for nothing. The client
/// converts — `Theme::packed()` produces exactly this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub ansi: [u32; 16],
    pub foreground: u32,
    pub background: u32,
    pub cursor: u32,
}

impl Default for Palette {
    fn default() -> Self {
        // The palette this crate resolved with before themes existed. Kept as
        // the default so a client that never sets one renders exactly as it
        // used to, and so `vt`'s own tests do not depend on a theme table.
        Self {
            ansi: [
                0x42_47_54, 0xF0_61_66, 0x6B_D1_82, 0xE6_C0_5C, 0x70_A9_F2, 0xCA_8C_F0,
                0x5C_C9_D1, 0xD4_D9_E0, 0x6B_72_82, 0xFF_82_85, 0x8F_EB_A1, 0xFA_DB_7D,
                0x94_C2_FF, 0xE1_AE_FF, 0x82_E5_EB, 0xF5_F7_FA,
            ],
            foreground: 0xDB_E0_E8,
            background: 0x12_14_19,
            cursor: 0xDB_E0_E8,
        }
    }
}

impl Palette {
    /// Read nineteen packed values in the FFI's order: sixteen ANSI, then
    /// foreground, background, cursor.
    ///
    /// `None` for any other length, rather than filling the gap. A caller that
    /// sent eighteen has a bug, and a nineteenth colour invented here would
    /// appear on screen with nothing in any theme file to explain it.
    pub fn from_packed(values: &[u32]) -> Option<Self> {
        if values.len() != 19 {
            return None;
        }
        let mut ansi = [0u32; 16];
        ansi.copy_from_slice(&values[..16]);
        Some(Self { ansi, foreground: values[16], background: values[17], cursor: values[18] })
    }
}

/// Read the currently displayed screen, which is not always the live one: when
/// the user has scrolled back, this returns what they are looking at.
pub fn snapshot(term: &Terminal) -> Snapshot {
    let palette = term.palette();
    let t = term.term();
    let grid = t.grid();
    let cols = grid.columns();
    let lines = grid.screen_lines();
    // Indexing the grid by line ignores the scroll position — line 0 is always
    // the top of the ACTIVE region, and history is at negative lines. So the
    // offset has to be applied here, or scrolling back would silently show the
    // live screen.
    let offset = grid.display_offset() as i32;

    let mut rows = Vec::with_capacity(lines);
    for line in 0..lines {
        let mut cells = Vec::with_capacity(cols);
        for col in 0..cols {
            let c = &grid[Line(line as i32 - offset)][Column(col)];
            let flags = c.flags;
            cells.push(Cell {
                ch: c.c,
                fg: resolve(c.fg, true, palette),
                bg: resolve(c.bg, false, palette),
                bold: flags.contains(Flags::BOLD),
                italic: flags.contains(Flags::ITALIC),
                underline: flags.contains(Flags::UNDERLINE),
                inverse: flags.contains(Flags::INVERSE),
                wide: flags.contains(Flags::WIDE_CHAR),
            });
        }
        rows.push(Row { cells });
    }

    // The cursor sits at a fixed place in the active region, so scrolling back
    // moves it down the view and eventually off it. Reporting it as invisible
    // then is the honest answer: drawing it clamped to the last row would put a
    // caret somewhere the user is not typing.
    let cursor = grid.cursor.point;
    let cursor_row = cursor.line.0 + offset;
    let on_screen = cursor_row >= 0 && (cursor_row as usize) < lines;

    Snapshot {
        columns: cols as u16,
        rows,
        cursor_row: cursor_row.max(0) as u16,
        cursor_column: cursor.column.0 as u16,
        cursor_visible: on_screen
            && t.mode().contains(alacritty_terminal::term::TermMode::SHOW_CURSOR),
        display_offset: offset as u32,
        history_size: grid.total_lines().saturating_sub(lines) as u32,
    }
}

/// Resolve a color to packed RGB.
///
/// Named and indexed colors are resolved HERE rather than in each renderer, so
/// Mac, iOS and Android cannot drift into three different palettes.
fn resolve(
    color: alacritty_terminal::vte::ansi::Color,
    foreground: bool,
    palette: &Palette,
) -> u32 {
    use alacritty_terminal::vte::ansi::{Color, NamedColor};

    match color {
        Color::Spec(rgb) => pack(rgb.r, rgb.g, rgb.b),
        Color::Indexed(i) => indexed(i, palette),
        Color::Named(named) => match named {
            NamedColor::Black => indexed(0, palette),
            NamedColor::Red => indexed(1, palette),
            NamedColor::Green => indexed(2, palette),
            NamedColor::Yellow => indexed(3, palette),
            NamedColor::Blue => indexed(4, palette),
            NamedColor::Magenta => indexed(5, palette),
            NamedColor::Cyan => indexed(6, palette),
            NamedColor::White => indexed(7, palette),
            NamedColor::BrightBlack => indexed(8, palette),
            NamedColor::BrightRed => indexed(9, palette),
            NamedColor::BrightGreen => indexed(10, palette),
            NamedColor::BrightYellow => indexed(11, palette),
            NamedColor::BrightBlue => indexed(12, palette),
            NamedColor::BrightMagenta => indexed(13, palette),
            NamedColor::BrightCyan => indexed(14, palette),
            NamedColor::BrightWhite => indexed(15, palette),
            NamedColor::Foreground | NamedColor::BrightForeground => palette.foreground,
            NamedColor::Background => palette.background,
            NamedColor::Cursor => palette.cursor,
            _ => {
                if foreground {
                    palette.foreground
                } else {
                    palette.background
                }
            }
        },
    }
}

fn pack(r: u8, g: u8, b: u8) -> u32 {
    ((r as u32) << 16) | ((g as u32) << 8) | b as u32
}

/// The xterm 256-color cube. Only the first sixteen are the theme's; the rest
/// is arithmetic every terminal agrees on and no theme file specifies.
fn indexed(i: u8, palette: &Palette) -> u32 {
    if i < 16 {
        return palette.ansi[i as usize];
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
    fn truecolor_is_preserved_exactly() {
        let mut t = Terminal::new(20, 4);
        t.feed(b"\x1b[38;2;18;52;86mX");
        assert_eq!(snapshot(&t).rows[0].cells[0].fg, 0x12_34_56);
    }

    #[test]
    fn the_256_color_cube_is_deterministic_and_not_the_themes_business() {
        // Every renderer must agree, so this is resolved once here. Above 15
        // the answer is arithmetic no theme file specifies, so it must come
        // out identical whatever palette is in force — otherwise a theme would
        // silently redefine 240 colours it never mentions.
        let a = Palette::default();
        let b = Palette::from_packed(&[0x11_22_33; 19]).expect("nineteen values");
        for i in [16u8, 231, 232, 255] {
            assert_eq!(indexed(i, &a), indexed(i, &b), "index {i} moved with the palette");
        }
        assert_eq!(indexed(16, &a), 0x00_00_00);
        assert_eq!(indexed(231, &a), 0xFF_FF_FF);
        assert_eq!(indexed(232, &a), 0x08_08_08);
        assert_eq!(indexed(255, &a), 0xEE_EE_EE);
    }

    #[test]
    fn a_new_palette_recolours_indexed_cells_but_not_truecolor_ones() {
        // The distinction a theme exists to draw: it colours what the program
        // left to the terminal to decide, and never overrules a program that
        // named an exact colour.
        let mut t = Terminal::new(10, 2);
        t.feed(b"\x1b[31mA\x1b[38;2;18;52;86mB");
        let before = snapshot(&t);

        let mut recoloured = Palette::default();
        recoloured.ansi[1] = 0xAB_CD_EF;
        t.set_palette(recoloured);
        let after = snapshot(&t);

        assert_ne!(before.rows[0].cells[0].fg, after.rows[0].cells[0].fg);
        assert_eq!(after.rows[0].cells[0].fg, 0xAB_CD_EF, "ANSI red follows the theme");
        assert_eq!(after.rows[0].cells[1].fg, 0x12_34_56, "truecolor is the program's to keep");
    }

    #[test]
    fn a_palette_of_the_wrong_length_is_refused() {
        assert!(Palette::from_packed(&[0; 18]).is_none());
        assert!(Palette::from_packed(&[0; 20]).is_none());
        assert!(Palette::from_packed(&[0; 19]).is_some());
    }

    #[test]
    fn inverse_is_reported_rather_than_pre_swapped() {
        // The renderer decides how to show inversion; the core just states it.
        let mut t = Terminal::new(20, 4);
        t.feed(b"\x1b[7mX");
        assert!(snapshot(&t).rows[0].cells[0].inverse);
    }
}
