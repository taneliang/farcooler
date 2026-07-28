//! The shared terminal core.
//!
//! One VT emulator, in Rust, behind an Overnight-owned C ABI, consumed by the
//! Mac, iOS and Android clients. Each platform writes only a renderer: it asks
//! for the grid and draws cells. Parsing, the screen model, the cursor, scroll
//! regions, the alternate screen and character attributes live here, once.
//!
//! The design's original choice for this slot was `libghostty-vt`. This fills
//! the same slot with `alacritty_terminal`, which is published and semver
//! stable, is the language the daemon already uses so there is one toolchain,
//! and is proven in Alacritty. Should libghostty stabilise, only this crate
//! changes: the C ABI and every renderer stay put. That is the point of owning
//! the boundary.

pub mod ffi;
pub mod grid;
pub mod input;

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};

/// Events the emulator raises that a client may care about.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Signals {
    /// The program rang the bell.
    pub bell: bool,
    /// The program asked to change the window title.
    pub title: Option<String>,
    /// The program wrote to the host (a device-status reply, a mouse report).
    /// These bytes must be delivered back to the pty in order.
    pub pty_writes: Vec<u8>,
}

#[derive(Clone, Default)]
struct Collector {
    inner: std::sync::Arc<std::sync::Mutex<Signals>>,
}

impl EventListener for Collector {
    fn send_event(&self, event: Event) {
        let mut s = self.inner.lock().expect("signal lock");
        match event {
            Event::Bell => s.bell = true,
            Event::Title(t) => s.title = Some(t),
            Event::PtyWrite(text) => s.pty_writes.extend_from_slice(text.as_bytes()),
            _ => {}
        }
    }
}

/// One terminal: a parser plus the screen it maintains.
pub struct Terminal {
    term: Term<Collector>,
    parser: Processor<StdSyncHandler>,
    collector: Collector,
}

impl Terminal {
    pub fn new(columns: u16, rows: u16) -> Self {
        let (columns, rows) = clamp(columns, rows);
        let collector = Collector::default();
        let size = TermSize::new(columns as usize, rows as usize);
        Self {
            term: Term::new(Config::default(), &size, collector.clone()),
            parser: Processor::new(),
            collector,
        }
    }

    /// Feed output bytes from the program.
    ///
    /// Byte-exact and resumable: a sequence split across two calls parses the
    /// same as one call, which matters because the transport chunks arbitrarily.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    /// Resize the grid. The emulator reflows its own content.
    pub fn resize(&mut self, columns: u16, rows: u16) {
        let (columns, rows) = clamp(columns, rows);
        self.term.resize(TermSize::new(columns as usize, rows as usize));
    }

    pub fn columns(&self) -> u16 {
        use alacritty_terminal::grid::Dimensions;
        self.term.grid().columns() as u16
    }

    pub fn rows(&self) -> u16 {
        use alacritty_terminal::grid::Dimensions;
        self.term.grid().screen_lines() as u16
    }

    /// Take whatever the program signalled since the last call.
    pub fn take_signals(&mut self) -> Signals {
        let mut guard = self.collector.inner.lock().expect("signal lock");
        std::mem::take(&mut *guard)
    }

    /// The modes the program has set. Input encoding depends on these, which is
    /// why key encoding lives beside the emulator rather than in a renderer.
    pub fn mode(&self) -> alacritty_terminal::term::TermMode {
        *self.term.mode()
    }

    /// Encode a keystroke for the program currently running.
    pub fn encode_key(&self, key: input::Key, mods: input::Modifiers) -> Vec<u8> {
        input::encode_key(self.mode(), key, mods)
    }

    /// Encode a mouse event, or None if the program does not want it.
    pub fn encode_mouse(
        &self,
        button: input::MouseButton,
        action: input::MouseAction,
        column: u16,
        row: u16,
        mods: input::Modifiers,
    ) -> Option<Vec<u8>> {
        input::encode_mouse(self.mode(), button, action, column, row, mods)
    }

    /// Wrap pasted text so the program can tell it apart from typing.
    pub fn encode_paste(&self, text: &str) -> Vec<u8> {
        input::encode_paste(self.mode(), text)
    }

    pub(crate) fn term(&self) -> &Term<Collector> {
        &self.term
    }
}

/// Terminal dimensions are clamped to the same range the protocol enforces.
fn clamp(columns: u16, rows: u16) -> (u16, u16) {
    (columns.clamp(20, 500), rows.clamp(5, 200))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::snapshot;

    fn render(t: &Terminal) -> Vec<String> {
        snapshot(t)
            .rows
            .iter()
            .map(|r| r.cells.iter().map(|c| c.ch).collect::<String>().trim_end().to_string())
            .collect()
    }

    #[test]
    fn plain_text_lands_on_the_first_row() {
        let mut t = Terminal::new(40, 6);
        t.feed(b"hello");
        assert_eq!(render(&t)[0], "hello");
    }

    #[test]
    fn carriage_return_and_line_feed_behave_separately() {
        // LF alone moves down without returning, which is why a bare \n from a
        // captured screen produces a staircase.
        let mut t = Terminal::new(40, 6);
        t.feed(b"abc\ndef");
        let r = render(&t);
        assert_eq!(r[0], "abc");
        assert_eq!(r[1], "   def", "LF must not return the carriage");

        let mut t2 = Terminal::new(40, 6);
        t2.feed(b"abc\r\ndef");
        assert_eq!(render(&t2)[1], "def");
    }

    #[test]
    fn absolute_cursor_positioning() {
        let mut t = Terminal::new(40, 6);
        t.feed(b"\x1b[3;10HMOVED");
        assert_eq!(render(&t)[2], "         MOVED");
    }

    #[test]
    fn erase_display_clears_the_screen() {
        let mut t = Terminal::new(40, 6);
        t.feed(b"junk everywhere");
        t.feed(b"\x1b[2J\x1b[H");
        assert!(render(&t).iter().all(|r| r.is_empty()));
    }

    #[test]
    fn colour_and_bold_survive_into_the_grid() {
        let mut t = Terminal::new(40, 6);
        t.feed(b"\x1b[1;31mR\x1b[0mp");
        let snap = snapshot(&t);
        let red = &snap.rows[0].cells[0];
        let plain = &snap.rows[0].cells[1];

        assert_eq!(red.ch, 'R');
        assert!(red.bold, "bold attribute must reach the grid");
        assert_ne!(red.fg, plain.fg, "the styled cell must differ in colour");
        assert!(!plain.bold, "reset must clear bold");
    }

    #[test]
    fn a_sequence_split_across_feeds_parses_identically() {
        // The transport chunks wherever it likes, so this must hold.
        let mut whole = Terminal::new(40, 6);
        whole.feed(b"\x1b[1;32mgreen\x1b[0m");

        let mut split = Terminal::new(40, 6);
        for chunk in [&b"\x1b[1;3"[..], &b"2mgre"[..], &b"en\x1b["[..], &b"0m"[..]] {
            split.feed(chunk);
        }

        assert_eq!(render(&whole), render(&split));
        assert_eq!(snapshot(&whole).rows[0].cells[0].fg, snapshot(&split).rows[0].cells[0].fg);
    }

    #[test]
    fn wide_characters_occupy_two_columns() {
        let mut t = Terminal::new(40, 6);
        t.feed("字a".as_bytes());
        let snap = snapshot(&t);
        assert_eq!(snap.rows[0].cells[0].ch, '字');
        assert!(snap.rows[0].cells[0].wide, "CJK must be marked wide");
        // The spacer column belongs to the wide char, so 'a' lands at column 2.
        assert_eq!(snap.rows[0].cells[2].ch, 'a');
    }

    #[test]
    fn resize_is_clamped_to_the_protocol_range() {
        let mut t = Terminal::new(1, 1);
        assert_eq!((t.columns(), t.rows()), (20, 5));
        t.resize(9999, 9999);
        assert_eq!((t.columns(), t.rows()), (500, 200));
    }

    #[test]
    fn resize_reflows_rather_than_leaving_stale_wraps() {
        let mut t = Terminal::new(20, 6);
        t.feed(b"0123456789012345678901234"); // 25 chars into 20 columns
        assert_eq!(render(&t)[1].trim_end(), "01234");

        t.resize(40, 6);
        // The emulator owns reflow. This is the bug a snapshot client cannot fix.
        assert_eq!(t.columns(), 40);
    }

    #[test]
    fn cursor_position_is_reported() {
        let mut t = Terminal::new(40, 6);
        t.feed(b"\x1b[4;7H");
        let snap = snapshot(&t);
        assert_eq!((snap.cursor_row, snap.cursor_column), (3, 6));
    }

    #[test]
    fn the_bell_is_signalled_once_and_then_cleared() {
        let mut t = Terminal::new(40, 6);
        t.feed(b"\x07");
        assert!(t.take_signals().bell);
        assert!(!t.take_signals().bell, "signals are drained when taken");
    }

    #[test]
    fn a_device_status_request_produces_bytes_for_the_pty() {
        let mut t = Terminal::new(40, 6);
        t.feed(b"\x1b[6n"); // report cursor position
        let s = t.take_signals();
        assert!(!s.pty_writes.is_empty(), "the reply must be sent back to the program");
        assert_eq!(s.pty_writes[0], 0x1b);
    }

    #[test]
    fn the_alternate_screen_does_not_destroy_the_primary() {
        let mut t = Terminal::new(40, 6);
        t.feed(b"primary");
        t.feed(b"\x1b[?1049h"); // enter alt screen
        t.feed(b"\x1b[2J\x1b[Halt");
        assert_eq!(render(&t)[0], "alt");
        t.feed(b"\x1b[?1049l"); // leave
        assert_eq!(render(&t)[0], "primary", "the primary screen must be restored");
    }
}
