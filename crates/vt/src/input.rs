//! Turning keystrokes and mouse events into the bytes a program expects.
//!
//! This lives in the core rather than in each renderer for one reason: the
//! correct bytes depend on modes the *emulator* holds. Arrow keys change shape
//! when the program sets application cursor mode. The wheel becomes arrow keys
//! in the alternate screen. Mouse reports have four encodings and the program
//! picks. A renderer cannot know any of that, so a renderer that encodes keys
//! itself is guessing — and guessing is why arrows insert `[A` into a prompt.
//!
//! Owning it here also means Mac, iOS and Android cannot disagree about what
//! Ctrl-C is.

use alacritty_terminal::term::TermMode;

/// Held modifiers. `meta` is Command on macOS; terminals do not report it, so
/// it is accepted and ignored rather than silently encoded as something else.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

impl Modifiers {
    /// The xterm modifier parameter: 1 plus a bitmask.
    fn param(self) -> u8 {
        1 + (self.shift as u8) + 2 * (self.alt as u8) + 4 * (self.ctrl as u8)
    }

    fn any(self) -> bool {
        self.shift || self.alt || self.ctrl
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// A printable character, already resolved by the platform's keyboard
    /// layout. The core never maps scancodes: that is the OS's job and it is
    /// the only party that knows the user's layout.
    Char(char),
    Enter,
    Tab,
    Backspace,
    Escape,
    Up,
    Down,
    Right,
    Left,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    /// F1 through F12.
    Function(u8),
}

/// Encode a keystroke for the program currently running.
pub fn encode_key(mode: TermMode, key: Key, mods: Modifiers) -> Vec<u8> {
    match key {
        Key::Char(c) => encode_char(c, mods),

        // CR, unless the program asked for CRLF on Enter.
        Key::Enter => {
            let bytes: &[u8] = if mode.contains(TermMode::LINE_FEED_NEW_LINE) {
                b"\r\n"
            } else {
                b"\r"
            };
            with_alt(bytes, mods)
        }

        // Shift-Tab is a distinct sequence, not a modified Tab.
        Key::Tab if mods.shift => b"\x1b[Z".to_vec(),
        Key::Tab => with_alt(b"\t", mods),

        // DEL, not BS. Ctrl-Backspace is the one that sends BS, which is what
        // shells bind to delete-word.
        Key::Backspace if mods.ctrl => with_alt(b"\x08", mods),
        Key::Backspace => with_alt(b"\x7f", mods),

        Key::Escape => with_alt(b"\x1b", mods),

        // Cursor and Home/End keys change their introducer under application
        // cursor mode, which every full-screen program turns on.
        Key::Up => cursor(mode, b'A', mods),
        Key::Down => cursor(mode, b'B', mods),
        Key::Right => cursor(mode, b'C', mods),
        Key::Left => cursor(mode, b'D', mods),
        Key::Home => cursor(mode, b'H', mods),
        Key::End => cursor(mode, b'F', mods),

        Key::Insert => tilde(2, mods),
        Key::Delete => tilde(3, mods),
        Key::PageUp => tilde(5, mods),
        Key::PageDown => tilde(6, mods),

        // F1-F4 are SS3-introduced; F5 up are tilde sequences, and the numbers
        // skip 16 and 22 for historical reasons.
        Key::Function(n) => match n {
            1..=4 => {
                let final_byte = b'P' + (n - 1);
                if mods.any() {
                    format!("\x1b[1;{}{}", mods.param(), final_byte as char).into_bytes()
                } else {
                    vec![0x1b, b'O', final_byte]
                }
            }
            5 => tilde(15, mods),
            6..=10 => tilde(17 + (n - 6), mods),
            11..=12 => tilde(23 + (n - 11), mods),
            _ => Vec::new(),
        },
    }
}

fn encode_char(c: char, mods: Modifiers) -> Vec<u8> {
    if mods.ctrl {
        if let Some(byte) = control(c) {
            return with_alt(&[byte], mods);
        }
    }
    let mut buf = [0u8; 4];
    with_alt(c.encode_utf8(&mut buf).as_bytes(), mods)
}

/// The C0 control a Ctrl-chord produces, or None if the chord has no control.
fn control(c: char) -> Option<u8> {
    let c = c.to_ascii_lowercase();
    match c {
        'a'..='z' => Some(c as u8 - b'a' + 1),
        // The classic aliases. Ctrl-@ and Ctrl-Space are both NUL, which is how
        // set-mark works in readline and Emacs.
        '@' | ' ' => Some(0),
        '[' => Some(27),
        '\\' => Some(28),
        ']' => Some(29),
        '^' => Some(30),
        '_' | '/' => Some(31),
        '?' => Some(127),
        _ => None,
    }
}

/// Alt is an ESC prefix, the convention every shell expects.
fn with_alt(bytes: &[u8], mods: Modifiers) -> Vec<u8> {
    if mods.alt {
        let mut out = Vec::with_capacity(bytes.len() + 1);
        out.push(0x1b);
        out.extend_from_slice(bytes);
        out
    } else {
        bytes.to_vec()
    }
}

fn cursor(mode: TermMode, final_byte: u8, mods: Modifiers) -> Vec<u8> {
    if mods.any() {
        // A modified cursor key is always CSI, never SS3, even in application
        // mode — there is nowhere to put the parameter otherwise.
        format!("\x1b[1;{}{}", mods.param(), final_byte as char).into_bytes()
    } else if mode.contains(TermMode::APP_CURSOR) {
        vec![0x1b, b'O', final_byte]
    } else {
        vec![0x1b, b'[', final_byte]
    }
}

fn tilde(number: u8, mods: Modifiers) -> Vec<u8> {
    if mods.any() {
        format!("\x1b[{};{}~", number, mods.param()).into_bytes()
    } else {
        format!("\x1b[{}~", number).into_bytes()
    }
}

/// Wrap pasted text so the program can tell it apart from typing.
///
/// Without this an editor auto-indents every pasted line and a shell runs each
/// newline as a command. The program opts in; if it has not, the text goes
/// through raw.
pub fn encode_paste(mode: TermMode, text: &str) -> Vec<u8> {
    // Strip the terminator from the payload, or pasted content could end the
    // bracket early and the remainder would execute as typed input.
    let cleaned = text.replace("\x1b[201~", "");
    if mode.contains(TermMode::BRACKETED_PASTE) {
        let mut out = Vec::with_capacity(cleaned.len() + 12);
        out.extend_from_slice(b"\x1b[200~");
        out.extend_from_slice(cleaned.as_bytes());
        out.extend_from_slice(b"\x1b[201~");
        out
    } else {
        cleaned.into_bytes()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseAction {
    Press,
    Release,
    /// The pointer moved. Reported only if the program asked for motion.
    Move,
}

/// Encode a mouse event, or return None if the program does not want it.
///
/// `column` and `row` are zero-based cell coordinates; the wire format is
/// one-based, and this converts.
pub fn encode_mouse(
    mode: TermMode,
    button: MouseButton,
    action: MouseAction,
    column: u16,
    row: u16,
    mods: Modifiers,
) -> Option<Vec<u8>> {
    let wheel = matches!(button, MouseButton::WheelUp | MouseButton::WheelDown);

    // Scrolling the alternate screen with no mouse reporting: the program is
    // full-screen and has no scrollback, so the wheel means arrow keys. This is
    // what makes the wheel scroll `less` and a full-screen agent's view.
    if wheel && !mode.intersects(TermMode::MOUSE_MODE) {
        if mode.contains(TermMode::ALT_SCREEN) && mode.contains(TermMode::ALTERNATE_SCROLL) {
            let key = if button == MouseButton::WheelUp { Key::Up } else { Key::Down };
            let one = encode_key(mode, key, Modifiers::default());
            return Some(one.repeat(3));
        }
        return None;
    }

    if !mode.intersects(TermMode::MOUSE_MODE) {
        return None;
    }
    // Motion is reported only when asked for: MOUSE_DRAG while a button is
    // held, MOUSE_MOTION always. Under plain click reporting, motion is noise.
    if action == MouseAction::Move
        && !mode.intersects(TermMode::MOUSE_MOTION | TermMode::MOUSE_DRAG)
    {
        return None;
    }

    let mut code: u8 = match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
        MouseButton::WheelUp => 64,
        MouseButton::WheelDown => 65,
    };
    if action == MouseAction::Move {
        code += 32;
    }
    if mods.shift {
        code += 4;
    }
    if mods.alt {
        code += 8;
    }
    if mods.ctrl {
        code += 16;
    }

    let (col, row) = (column as u32 + 1, row as u32 + 1);

    if mode.contains(TermMode::SGR_MOUSE) {
        // The only encoding without a coordinate limit, and the only one that
        // distinguishes which button was released.
        let final_byte = if action == MouseAction::Release { 'm' } else { 'M' };
        return Some(format!("\x1b[<{};{};{}{}", code, col, row, final_byte).into_bytes());
    }

    // X10: a release is an untyped code 3, and coordinates beyond 223 cannot be
    // expressed at all, so they are dropped rather than reported wrongly.
    let code = if action == MouseAction::Release { 3 + (code & !0b11) } else { code };
    if col > 223 || row > 223 {
        return None;
    }
    Some(vec![0x1b, b'[', b'M', 32 + code, 32 + col as u8, 32 + row as u8])
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAIN: TermMode = TermMode::empty();

    fn ctrl() -> Modifiers {
        Modifiers { ctrl: true, ..Default::default() }
    }
    fn alt() -> Modifiers {
        Modifiers { alt: true, ..Default::default() }
    }
    fn shift() -> Modifiers {
        Modifiers { shift: true, ..Default::default() }
    }

    #[test]
    fn ctrl_c_is_the_interrupt_byte() {
        // If this is wrong, a runaway agent cannot be stopped.
        assert_eq!(encode_key(PLAIN, Key::Char('c'), ctrl()), vec![0x03]);
        assert_eq!(encode_key(PLAIN, Key::Char('C'), ctrl()), vec![0x03]);
        assert_eq!(encode_key(PLAIN, Key::Char('d'), ctrl()), vec![0x04]);
    }

    #[test]
    fn the_classic_control_aliases_hold() {
        assert_eq!(encode_key(PLAIN, Key::Char(' '), ctrl()), vec![0x00]);
        assert_eq!(encode_key(PLAIN, Key::Char('['), ctrl()), vec![0x1b]);
        assert_eq!(encode_key(PLAIN, Key::Char('?'), ctrl()), vec![0x7f]);
    }

    #[test]
    fn a_ctrl_chord_with_no_control_falls_back_to_the_character() {
        assert_eq!(encode_key(PLAIN, Key::Char('1'), ctrl()), b"1".to_vec());
    }

    #[test]
    fn alt_prefixes_with_escape() {
        assert_eq!(encode_key(PLAIN, Key::Char('b'), alt()), vec![0x1b, b'b']);
        assert_eq!(encode_key(PLAIN, Key::Backspace, alt()), vec![0x1b, 0x7f]);
    }

    #[test]
    fn backspace_sends_del_and_ctrl_backspace_sends_bs() {
        // Backwards here means backspace deletes forwards in half of everything.
        assert_eq!(encode_key(PLAIN, Key::Backspace, Modifiers::default()), vec![0x7f]);
        assert_eq!(encode_key(PLAIN, Key::Backspace, ctrl()), vec![0x08]);
    }

    #[test]
    fn arrows_follow_application_cursor_mode() {
        assert_eq!(encode_key(PLAIN, Key::Up, Modifiers::default()), b"\x1b[A".to_vec());
        assert_eq!(
            encode_key(TermMode::APP_CURSOR, Key::Up, Modifiers::default()),
            b"\x1bOA".to_vec()
        );
    }

    #[test]
    fn a_modified_arrow_is_csi_even_in_application_mode() {
        // There is nowhere to put the modifier parameter in an SS3 sequence.
        assert_eq!(encode_key(TermMode::APP_CURSOR, Key::Right, ctrl()), b"\x1b[1;5C".to_vec());
        assert_eq!(encode_key(PLAIN, Key::Left, alt()), b"\x1b[1;3D".to_vec());
        assert_eq!(
            encode_key(PLAIN, Key::Up, Modifiers { shift: true, ctrl: true, alt: false }),
            b"\x1b[1;6A".to_vec()
        );
    }

    #[test]
    fn shift_tab_is_its_own_sequence() {
        assert_eq!(encode_key(PLAIN, Key::Tab, shift()), b"\x1b[Z".to_vec());
        assert_eq!(encode_key(PLAIN, Key::Tab, Modifiers::default()), vec![0x09]);
    }

    #[test]
    fn enter_is_carriage_return_unless_the_program_asked_otherwise() {
        assert_eq!(encode_key(PLAIN, Key::Enter, Modifiers::default()), vec![0x0d]);
        assert_eq!(
            encode_key(TermMode::LINE_FEED_NEW_LINE, Key::Enter, Modifiers::default()),
            vec![0x0d, 0x0a]
        );
    }

    #[test]
    fn navigation_and_function_keys_match_xterm() {
        assert_eq!(encode_key(PLAIN, Key::Delete, Modifiers::default()), b"\x1b[3~".to_vec());
        assert_eq!(encode_key(PLAIN, Key::PageUp, Modifiers::default()), b"\x1b[5~".to_vec());
        assert_eq!(encode_key(PLAIN, Key::Function(1), Modifiers::default()), b"\x1bOP".to_vec());
        assert_eq!(encode_key(PLAIN, Key::Function(5), Modifiers::default()), b"\x1b[15~".to_vec());
        // The numbering skips 16 and 22.
        assert_eq!(encode_key(PLAIN, Key::Function(6), Modifiers::default()), b"\x1b[17~".to_vec());
        assert_eq!(encode_key(PLAIN, Key::Function(11), Modifiers::default()), b"\x1b[23~".to_vec());
        assert_eq!(encode_key(PLAIN, Key::Function(12), Modifiers::default()), b"\x1b[24~".to_vec());
    }

    #[test]
    fn non_ascii_typing_goes_through_as_utf8() {
        assert_eq!(encode_key(PLAIN, Key::Char('é'), Modifiers::default()), "é".as_bytes().to_vec());
        assert_eq!(encode_key(PLAIN, Key::Char('日'), Modifiers::default()), "日".as_bytes().to_vec());
    }

    #[test]
    fn paste_is_bracketed_only_when_the_program_asked() {
        assert_eq!(encode_paste(PLAIN, "hi"), b"hi".to_vec());
        assert_eq!(
            encode_paste(TermMode::BRACKETED_PASTE, "hi"),
            b"\x1b[200~hi\x1b[201~".to_vec()
        );
    }

    #[test]
    fn pasted_text_cannot_close_its_own_bracket() {
        // Otherwise pasted content after the terminator would execute as typing.
        let evil = "safe\x1b[201~rm -rf /\r";
        let out = encode_paste(TermMode::BRACKETED_PASTE, evil);
        let s = String::from_utf8(out).unwrap();
        assert_eq!(s.matches("\x1b[201~").count(), 1);
        assert!(s.ends_with("\x1b[201~"));
    }

    #[test]
    fn mouse_events_are_silent_unless_the_program_asked() {
        assert_eq!(
            encode_mouse(PLAIN, MouseButton::Left, MouseAction::Press, 0, 0, Modifiers::default()),
            None
        );
    }

    #[test]
    fn sgr_reports_press_and_release_distinctly() {
        let mode = TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE;
        let press =
            encode_mouse(mode, MouseButton::Left, MouseAction::Press, 9, 4, Modifiers::default());
        let release =
            encode_mouse(mode, MouseButton::Left, MouseAction::Release, 9, 4, Modifiers::default());
        // Coordinates are one-based on the wire.
        assert_eq!(press.unwrap(), b"\x1b[<0;10;5M".to_vec());
        assert_eq!(release.unwrap(), b"\x1b[<0;10;5m".to_vec());
    }

    #[test]
    fn sgr_has_no_coordinate_limit() {
        let mode = TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE;
        let out =
            encode_mouse(mode, MouseButton::Left, MouseAction::Press, 400, 300, Modifiers::default())
                .unwrap();
        assert_eq!(out, b"\x1b[<0;401;301M".to_vec());
    }

    #[test]
    fn x10_drops_coordinates_it_cannot_express() {
        // Reporting the wrong cell is worse than reporting nothing.
        let mode = TermMode::MOUSE_REPORT_CLICK;
        assert!(encode_mouse(
            mode,
            MouseButton::Left,
            MouseAction::Press,
            300,
            1,
            Modifiers::default()
        )
        .is_none());
        assert_eq!(
            encode_mouse(mode, MouseButton::Left, MouseAction::Press, 9, 4, Modifiers::default())
                .unwrap(),
            vec![0x1b, b'[', b'M', 32, 42, 37]
        );
    }

    #[test]
    fn motion_is_reported_only_when_requested() {
        let click = TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE;
        assert_eq!(
            encode_mouse(click, MouseButton::Left, MouseAction::Move, 1, 1, Modifiers::default()),
            None
        );

        let drag = click | TermMode::MOUSE_DRAG;
        let out =
            encode_mouse(drag, MouseButton::Left, MouseAction::Move, 1, 1, Modifiers::default())
                .unwrap();
        assert_eq!(out, b"\x1b[<32;2;2M".to_vec());
    }

    #[test]
    fn the_wheel_becomes_arrows_in_a_full_screen_program() {
        // This is what makes scrolling work in `less` and in a coding agent that
        // never asked for mouse reporting.
        let mode = TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL;
        let up = encode_mouse(
            mode,
            MouseButton::WheelUp,
            MouseAction::Press,
            0,
            0,
            Modifiers::default(),
        )
        .unwrap();
        assert_eq!(up, b"\x1b[A\x1b[A\x1b[A".to_vec());

        let app = mode | TermMode::APP_CURSOR;
        let down = encode_mouse(
            app,
            MouseButton::WheelDown,
            MouseAction::Press,
            0,
            0,
            Modifiers::default(),
        )
        .unwrap();
        assert_eq!(down, b"\x1bOB\x1bOB\x1bOB".to_vec(), "must respect application cursor mode");
    }

    #[test]
    fn a_program_that_wants_the_wheel_gets_the_wheel_not_arrows() {
        let mode = TermMode::ALT_SCREEN
            | TermMode::ALTERNATE_SCROLL
            | TermMode::MOUSE_REPORT_CLICK
            | TermMode::SGR_MOUSE;
        let out = encode_mouse(
            mode,
            MouseButton::WheelUp,
            MouseAction::Press,
            0,
            0,
            Modifiers::default(),
        )
        .unwrap();
        assert_eq!(out, b"\x1b[<64;1;1M".to_vec());
    }

    #[test]
    fn wheel_scrolling_the_primary_screen_is_the_hosts_business() {
        // Scrollback belongs to the client's own view, not to the program.
        assert_eq!(
            encode_mouse(
                TermMode::ALTERNATE_SCROLL,
                MouseButton::WheelUp,
                MouseAction::Press,
                0,
                0,
                Modifiers::default()
            ),
            None
        );
    }

    #[test]
    fn mouse_modifiers_are_folded_into_the_button_code() {
        let mode = TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE;
        let out = encode_mouse(
            mode,
            MouseButton::Left,
            MouseAction::Press,
            0,
            0,
            Modifiers { shift: true, alt: false, ctrl: true },
        )
        .unwrap();
        assert_eq!(out, b"\x1b[<20;1;1M".to_vec());
    }
}
