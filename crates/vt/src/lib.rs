//! The shared terminal core.
//!
//! One VT emulator, in Rust, behind a Far Cooler-owned C ABI, consumed by the
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
pub mod url;

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{ClipboardType, Config, Term};
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
    /// Text the program asked to put on the clipboard (OSC 52).
    ///
    /// The WRITE half only, and deliberately. `Config::osc52` defaults to
    /// `Osc52::OnlyCopy`, so a program asking to READ the clipboard is refused
    /// by the parser and never reaches here — there is a test that asserts it.
    ///
    /// That default stands because of what this product is: agents running
    /// unattended on machines nobody is sitting at. One of them being able to
    /// read the clipboard of the Mac watching it is a data path in the wrong
    /// direction, over a link that exists to carry terminal output. Copy is a
    /// program handing you something; paste is a program taking something.
    pub clipboard: Option<String>,
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
            // Last writer wins within one drain: a program that copies twice
            // before the client looks meant the second one.
            //
            // `ClipboardType::Selection` falls through to the arm below rather
            // than being treated as a copy. It is X11's PRIMARY selection,
            // which has no analogue on any of the three platforms, and quietly
            // mapping it onto the clipboard would let a program overwrite the
            // clipboard through a channel nobody expects one on.
            Event::ClipboardStore(ClipboardType::Clipboard, text) => s.clipboard = Some(text),
            _ => {}
        }
    }
}

/// One terminal: a parser plus the screen it maintains.
pub struct Terminal {
    term: Term<Collector>,
    parser: Processor<StdSyncHandler>,
    collector: Collector,
    /// What indexed and named colours resolve to. Runtime state rather than
    /// the constants this used to be, because a theme is a thing the person
    /// looking at the screen chooses. See `grid::Palette`.
    palette: grid::Palette,
}

impl Terminal {
    pub fn new(columns: u16, rows: u16) -> Self {
        let (columns, rows) = clamp(columns, rows);
        let collector = Collector::default();
        let size = TermSize::new(columns as usize, rows as usize);
        // `kitty_keyboard` is what lets a program negotiate the keyboard
        // protocol at all: without it the parser drops `CSI > flags u` on the
        // floor and every modified Enter stays indistinguishable from Enter.
        // The emulator owns the mode stack, including the separate one the
        // alternate screen gets, so a full-screen program that dies without
        // popping cannot strand the shell underneath it. See `input::kitty_key`.
        let config = Config {
            scrolling_history: SCROLLBACK_LINES,
            kitty_keyboard: true,
            ..Config::default()
        };
        Self {
            term: Term::new(config, &size, collector.clone()),
            parser: Processor::new(),
            collector,
            palette: grid::Palette::default(),
        }
    }

    pub fn palette(&self) -> &grid::Palette {
        &self.palette
    }

    /// Recolour every cell the next snapshot reads.
    ///
    /// Nothing is repainted here and nothing needs to be: colours are resolved
    /// when a snapshot is taken, not when bytes arrive, so the scrollback a
    /// program wrote an hour ago comes back in the new theme too. That is the
    /// property that makes switching themes instant rather than a redraw of
    /// history the terminal no longer has.
    pub fn set_palette(&mut self, palette: grid::Palette) {
        self.palette = palette;
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

    /// Scroll the view. Positive goes back into history, negative returns
    /// toward the live screen; the core clamps at both ends.
    ///
    /// Scrollback is the client's own view, not something the program is told
    /// about, so this never sends bytes anywhere.
    pub fn scroll(&mut self, lines: i32) {
        use alacritty_terminal::grid::Scroll;
        self.term.scroll_display(Scroll::Delta(lines));
    }

    /// Jump back to the live screen. Typing should always do this, or the user
    /// types into a view that is not showing them what they typed.
    pub fn scroll_to_bottom(&mut self) {
        use alacritty_terminal::grid::Scroll;
        self.term.scroll_display(Scroll::Bottom);
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

/// How much history a terminal keeps.
///
/// Stated rather than inherited, because it is a product decision: an agent
/// working overnight produces a lot of output, and the user's first question in
/// the morning is what it did an hour ago. At ~200 bytes a line this is a few
/// megabytes per terminal, which is the same order as the design's 8 MiB
/// replay buffer, so the two stay in proportion.
pub const SCROLLBACK_LINES: usize = 10_000;

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
    fn color_and_bold_survive_into_the_grid() {
        let mut t = Terminal::new(40, 6);
        t.feed(b"\x1b[1;31mR\x1b[0mp");
        let snap = snapshot(&t);
        let red = &snap.rows[0].cells[0];
        let plain = &snap.rows[0].cells[1];

        assert_eq!(red.ch, 'R');
        assert!(red.bold, "bold attribute must reach the grid");
        assert_ne!(red.fg, plain.fg, "the styled cell must differ in color");
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

    /// Write `count` numbered lines, so scrolled-back content is identifiable.
    fn fill(t: &mut Terminal, count: usize) {
        for i in 0..count {
            t.feed(format!("line{i}\r\n").as_bytes());
        }
    }

    #[test]
    fn replaying_a_full_screen_must_not_end_with_a_newline() {
        // The rule the daemon's replay has to respect, stated where the
        // behavior actually lives.
        //
        // A captured screen is exactly as many lines as the screen is tall
        // whenever the program fills it, which a full-screen agent always does.
        // One more line feed at the bottom row scrolls everything up by one:
        // the top line goes to history, every row appears one too high, and the
        // caret is stranded on a blank bottom row. That is one newline
        // producing two different-looking bugs.
        let lines = ["ROW1", "ROW2", "ROW3", "ROW4", "LAST"];
        let replay = lines.join("\r\n");

        let mut good = Terminal::new(40, 5);
        good.feed(b"\x1b[H\x1b[2J");
        good.feed(replay.as_bytes());
        let rendered = render(&good);
        assert_eq!(rendered[0], "ROW1", "the top line must survive");
        assert_eq!(rendered[4], "LAST", "the last line must sit on the last row");

        let mut bad = Terminal::new(40, 5);
        bad.feed(b"\x1b[H\x1b[2J");
        bad.feed(replay.as_bytes());
        bad.feed(b"\r\n");
        let shifted = render(&bad);
        assert_eq!(shifted[0], "ROW2", "the trailing newline scrolls the top line away");
        assert_eq!(shifted[4], "", "and leaves a blank row where the caret lands");
    }

    #[test]
    fn a_replay_can_restore_the_cursor_it_could_not_carry() {
        // Captured text has no cursor in it, so the position is sent
        // separately. One-based on the wire, zero-based in the snapshot.
        let mut t = Terminal::new(40, 5);
        t.feed(b"\x1b[H\x1b[2Jabc\r\ndef");
        t.feed(b"\x1b[2;3H");
        let snap = snapshot(&t);
        assert_eq!((snap.cursor_row, snap.cursor_column), (1, 2));
        assert!(snap.cursor_visible);
    }

    #[test]
    fn scrolling_back_shows_history_and_returning_shows_the_live_screen() {
        let mut t = Terminal::new(40, 6);
        fill(&mut t, 30);
        let live = render(&t);
        assert_eq!(live[4], "line29");

        t.scroll(10);
        let back = render(&t);
        assert_ne!(back, live);
        assert_eq!(back[4], "line19");

        t.scroll_to_bottom();
        assert_eq!(render(&t), live);
    }

    #[test]
    fn scrolling_is_clamped_at_both_ends() {
        let mut t = Terminal::new(40, 6);
        fill(&mut t, 30);

        t.scroll(100_000);
        let top = render(&t);
        assert_eq!(top[0], "line0", "cannot scroll past the oldest line");

        t.scroll(-100_000);
        assert_eq!(snapshot(&t).display_offset, 0, "cannot scroll past the live screen");
    }

    #[test]
    fn the_cursor_is_not_drawn_where_the_user_is_not_typing() {
        // Scrolled back, the cursor is off-screen. Clamping it to the last row
        // would put a caret somewhere nothing is being typed.
        let mut t = Terminal::new(40, 6);
        fill(&mut t, 30);
        assert!(snapshot(&t).cursor_visible);

        t.scroll(20);
        assert!(!snapshot(&t).cursor_visible);

        t.scroll_to_bottom();
        assert!(snapshot(&t).cursor_visible);
    }

    #[test]
    fn history_size_reports_what_is_available_above() {
        let mut t = Terminal::new(40, 6);
        assert_eq!(snapshot(&t).history_size, 0);
        fill(&mut t, 30);
        // 30 lines written into a 6-row screen leaves 25 above it.
        assert_eq!(snapshot(&t).history_size, 25);
    }

    #[test]
    fn new_output_does_not_yank_a_scrolled_back_view_to_the_bottom() {
        // An agent that keeps printing must not drag the screen out from under
        // someone reading what it did ten minutes ago.
        let mut t = Terminal::new(40, 6);
        fill(&mut t, 30);
        t.scroll(10);
        let reading = render(&t);

        fill(&mut t, 5);
        assert_eq!(render(&t), reading, "the view must stay where the user put it");
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

    #[test]
    fn a_replayed_mouse_mode_makes_the_wheel_the_program_s() {
        // The other half of the daemon's mode replay: tmux reports that a pane
        // wants any-event tracking with SGR encoding, the replay states it, and
        // a fresh emulator has to come out of that believing the program wants
        // the wheel. Without it the emulator declines every mouse event and the
        // client scrolls a scrollback that a full-screen program does not have,
        // which is what a dead scroll area inside a TUI actually is.
        let mut t = Terminal::new(80, 24);
        assert!(
            t.encode_mouse(input::MouseButton::WheelUp, input::MouseAction::Press, 3, 4, input::Modifiers::default())
                .is_none(),
            "a program that asked for nothing gets nothing"
        );
        t.feed(b"\x1b[?1049h\x1b[?1003h\x1b[?1006h");
        let report = t
            .encode_mouse(input::MouseButton::WheelUp, input::MouseAction::Press, 3, 4, input::Modifiers::default())
            .expect("the program asked for the mouse, so it gets the event");
        assert!(report.starts_with(b"\x1b[<"), "SGR encoding was requested: {report:?}");
    }

    /// Base64 of "hello", as a program sending OSC 52 would encode it.
    const HELLO_B64: &str = "aGVsbG8=";

    #[test]
    fn an_osc52_copy_reaches_the_signals_once() {
        let mut t = Terminal::new(40, 6);
        t.feed(format!("\x1b]52;c;{HELLO_B64}\x07").as_bytes());
        assert_eq!(t.take_signals().clipboard.as_deref(), Some("hello"));
        assert_eq!(t.take_signals().clipboard, None, "signals are drained when taken");
    }

    #[test]
    fn an_osc52_copy_terminated_by_st_works_too() {
        // BEL and ESC-backslash are both legal terminators and real programs
        // use both — tmux sends ST. Supporting only one would make this work
        // from a shell and not from inside tmux, which is where the agents run.
        let mut t = Terminal::new(40, 6);
        t.feed(format!("\x1b]52;c;{HELLO_B64}\x1b\\").as_bytes());
        assert_eq!(t.take_signals().clipboard.as_deref(), Some("hello"));
    }

    #[test]
    fn a_program_asking_to_read_the_clipboard_gets_no_reply() {
        // The security property, so it is a test rather than a comment. This
        // app exists to run agents on machines nobody is sitting at; one of
        // them being able to read the watching Mac's clipboard is a data path
        // in the wrong direction over a link that carries terminal output.
        let mut t = Terminal::new(40, 6);
        t.feed(b"\x1b]52;c;?\x07");
        let s = t.take_signals();
        assert!(s.pty_writes.is_empty(), "nothing may go back to the program");
        assert_eq!(s.clipboard, None);
    }

    #[test]
    fn a_selection_clipboard_write_is_ignored() {
        // X11's PRIMARY has no analogue on any of the three platforms, and
        // treating it as a copy would let a program overwrite the clipboard
        // through a channel nobody expects one on.
        let mut t = Terminal::new(40, 6);
        t.feed(format!("\x1b]52;p;{HELLO_B64}\x07").as_bytes());
        assert_eq!(t.take_signals().clipboard, None);
    }

    /// Shift-Enter, the chord the kitty protocol exists to make expressible.
    fn shift_enter(t: &Terminal) -> Vec<u8> {
        t.encode_key(input::Key::Enter, input::Modifiers { shift: true, ..Default::default() })
    }

    #[test]
    fn the_kitty_keyboard_protocol_is_off_until_a_program_asks_for_it() {
        // A terminal that volunteered CSI u would break every program that never
        // heard of it, so the mode is opt-in and starts empty.
        let t = Terminal::new(40, 6);
        assert_eq!(shift_enter(&t), vec![0x0d]);
    }

    #[test]
    fn a_program_can_push_and_pop_the_kitty_keyboard_protocol() {
        // This is the handshake Claude Code, neovim and helix perform at startup
        // and undo on exit. Without the pop, the shell that outlives the program
        // would keep receiving CSI u for chords it cannot read.
        let mut t = Terminal::new(40, 6);
        t.feed(b"\x1b[>1u");
        assert_eq!(shift_enter(&t), b"\x1b[13;2u".to_vec(), "the program asked to disambiguate");

        t.feed(b"\x1b[<u");
        assert_eq!(shift_enter(&t), vec![0x0d], "popping must restore the legacy encoding");
    }

    #[test]
    fn a_program_can_set_the_flags_without_pushing_them() {
        // `CSI = flags ; mode u` is the other way in, used by programs that
        // manage the mode themselves rather than nesting it.
        let mut t = Terminal::new(40, 6);
        t.feed(b"\x1b[=1;1u");
        assert_eq!(shift_enter(&t), b"\x1b[13;2u".to_vec());
    }

    #[test]
    fn a_program_can_ask_what_the_terminal_supports() {
        // The detection handshake: push, ask, and read the reply to find out
        // whether anything is listening. A terminal that stayed silent here
        // would leave the program assuming the legacy encoding forever.
        let mut t = Terminal::new(40, 6);
        t.feed(b"\x1b[>1u\x1b[?u");
        assert_eq!(t.take_signals().pty_writes, b"\x1b[?1u".to_vec());
    }

    #[test]
    fn a_full_screen_program_cannot_strand_the_protocol_on_the_shell() {
        // The alternate screen keeps its own keyboard stack, so an agent that
        // turns the protocol on and then dies without popping it leaves the
        // shell underneath encoding keys the way it always did. This is the
        // same safety property as Enter staying a carriage return: after a
        // crash, the user has to be able to type.
        let mut t = Terminal::new(40, 6);
        t.feed(b"\x1b[?1049h\x1b[>1u");
        assert_eq!(shift_enter(&t), b"\x1b[13;2u".to_vec());

        t.feed(b"\x1b[?1049l");
        assert_eq!(shift_enter(&t), vec![0x0d], "the primary screen never asked for it");
    }

    #[test]
    fn a_line_exactly_as_wide_as_the_screen_does_not_consume_two_rows() {
        // Writing the last column leaves the caret pending at the edge rather
        // than on the next row; the line feed that follows is what moves it.
        // An emulator that advanced eagerly would spend two rows on one line,
        // and a screen replayed into it would arrive scrolled — which is how a
        // caret ends up a row away from the text it belongs to.
        let mut t = Terminal::new(10, 4);
        t.feed(b"0123456789\r\nsecond");
        assert_eq!(snapshot(&t).cursor_row, 1);
    }
}
