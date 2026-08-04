//! The C ABI.
//!
//! This is Far Cooler's own boundary, not a passthrough of the emulator's API.
//! Every platform renderer — Swift today, Kotlin later — speaks only these
//! functions, so the emulator underneath can be replaced without touching a
//! single line of UI code.
//!
//! Two rules make it fast enough to drive a 60 Hz redraw:
//!
//! 1. **No per-cell calls.** Crossing the FFI boundary once per cell would cost
//!    more than the emulation. `snapshot` fills one flat, `#[repr(C)]` array the
//!    renderer walks directly.
//! 2. **No allocation per frame.** The cell buffer lives on the handle and is
//!    reused, so a steady-state redraw allocates nothing.
//!
//! Every pointer parameter is null-checked. A null handle is a no-op or a
//! zero/false return, never a crash: a renderer bug must not take down the app.

//! ## Safety, once rather than 23 times
//!
//! Every function here is `unsafe` for the same reason and under the same
//! contract, so `clippy::missing_safety_doc` is silenced at the module level
//! rather than answered per function — 23 copies of one paragraph is 23 places
//! for it to drift out of date.
//!
//! The contract: pointers are either null or valid for the call's duration, a
//! handle came from this module's constructor and has not been freed, and no
//! handle is used concurrently from two threads without the caller's own
//! synchronization. Null is checked everywhere and is never a crash — a
//! renderer bug must not take down the app.
#![allow(clippy::missing_safety_doc)]

use std::ffi::{c_char, c_void};

use crate::grid::{snapshot, Cell};
use crate::Terminal;

pub const FLAG_BOLD: u16 = 1 << 0;
pub const FLAG_ITALIC: u16 = 1 << 1;
pub const FLAG_UNDERLINE: u16 = 1 << 2;
pub const FLAG_INVERSE: u16 = 1 << 3;
pub const FLAG_WIDE: u16 = 1 << 4;

/// One cell, laid out for direct reading by the renderer.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VtCell {
    /// Unicode scalar value.
    pub ch: u32,
    /// Packed 0xRRGGBB.
    pub fg: u32,
    pub bg: u32,
    pub flags: u16,
    _pad: u16,
}

/// The screen, as a flat row-major array plus the cursor.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VtSnapshot {
    /// Borrowed. Valid until the next call on this handle.
    pub cells: *const VtCell,
    pub columns: u16,
    pub rows: u16,
    pub cursor_row: u16,
    pub cursor_column: u16,
    pub cursor_visible: bool,
    /// How far the view is scrolled back, in lines. Zero means live.
    pub display_offset: u32,
    /// Lines available above the screen, for drawing a scrollbar.
    pub history_size: u32,
}

/// Opaque to callers.
pub struct VtHandle {
    terminal: Terminal,
    cells: Vec<VtCell>,
    /// Bytes the program wants written back to the pty, drained by the caller.
    pending_writes: Vec<u8>,
    title: Option<std::ffi::CString>,
    /// Bumped on every feed, so a renderer can skip a redraw that would paint
    /// exactly what is already on screen. This is what keeps an idle terminal
    /// from burning a frame every tick.
    revision: u64,
    bell: bool,
}

/// Create a terminal. Free with `farcooler_vt_free`.
#[unsafe(no_mangle)]
pub extern "C" fn farcooler_vt_new(columns: u16, rows: u16) -> *mut c_void {
    let handle = Box::new(VtHandle {
        terminal: Terminal::new(columns, rows),
        cells: Vec::new(),
        pending_writes: Vec::new(),
        title: None,
        revision: 0,
        bell: false,
    });
    Box::into_raw(handle) as *mut c_void
}

/// Destroy a terminal. Safe to call with null; never call twice.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn farcooler_vt_free(handle: *mut c_void) {
    if handle.is_null() {
        return;
    }
    drop(unsafe { Box::from_raw(handle as *mut VtHandle) });
}

/// Feed program output.
///
/// Chunk boundaries are irrelevant: a sequence split across calls parses the
/// same as one call, which is what makes this safe to drive from a socket.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn farcooler_vt_feed(handle: *mut c_void, bytes: *const u8, len: usize) {
    let Some(h) = (unsafe { as_handle(handle) }) else { return };
    if bytes.is_null() || len == 0 {
        return;
    }
    let slice = unsafe { std::slice::from_raw_parts(bytes, len) };
    h.terminal.feed(slice);
    h.revision = h.revision.wrapping_add(1);

    let signals = h.terminal.take_signals();
    h.bell |= signals.bell;
    h.pending_writes.extend_from_slice(&signals.pty_writes);
    if let Some(t) = signals.title {
        h.title = std::ffi::CString::new(t).ok();
    }
}

/// Resize the grid. Dimensions are clamped to the protocol's range.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn farcooler_vt_resize(handle: *mut c_void, columns: u16, rows: u16) {
    let Some(h) = (unsafe { as_handle(handle) }) else { return };
    h.terminal.resize(columns, rows);
    h.revision = h.revision.wrapping_add(1);
}

/// A counter that changes whenever the screen may have changed.
///
/// A renderer that caches this value and finds it unchanged can skip the frame
/// entirely.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn farcooler_vt_revision(handle: *mut c_void) -> u64 {
    match unsafe { as_handle(handle) } {
        Some(h) => h.revision,
        None => 0,
    }
}

/// Read the screen into `out`.
///
/// `out.cells` borrows a buffer owned by the handle. It stays valid until the
/// next call on this handle, which is all a synchronous draw needs, and is why
/// a steady redraw allocates nothing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn farcooler_vt_snapshot(handle: *mut c_void, out: *mut VtSnapshot) -> bool {
    let Some(h) = (unsafe { as_handle(handle) }) else { return false };
    if out.is_null() {
        return false;
    }

    let snap = snapshot(&h.terminal);
    let count = snap.rows.len() * snap.columns as usize;
    h.cells.clear();
    h.cells.reserve(count);
    for row in &snap.rows {
        for cell in &row.cells {
            h.cells.push(pack(cell));
        }
    }

    unsafe {
        *out = VtSnapshot {
            cells: h.cells.as_ptr(),
            columns: snap.columns,
            rows: snap.rows.len() as u16,
            cursor_row: snap.cursor_row,
            cursor_column: snap.cursor_column,
            cursor_visible: snap.cursor_visible,
            display_offset: snap.display_offset,
            history_size: snap.history_size,
        };
    }
    true
}

/// Scroll the view. Positive goes back into history, negative returns toward
/// the live screen. Clamped at both ends.
///
/// Scrollback is the client's own view; the program is never told about it, so
/// this produces no bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn farcooler_vt_scroll(handle: *mut c_void, lines: i32) {
    let Some(h) = (unsafe { as_handle(handle) }) else { return };
    h.terminal.scroll(lines);
    h.revision = h.revision.wrapping_add(1);
}

/// Jump back to the live screen.
///
/// Call this on input: typing into a scrolled-back view would show the user
/// nothing of what they typed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn farcooler_vt_scroll_to_bottom(handle: *mut c_void) {
    let Some(h) = (unsafe { as_handle(handle) }) else { return };
    h.terminal.scroll_to_bottom();
    h.revision = h.revision.wrapping_add(1);
}

/// Take bytes the program wants written back to the pty.
///
/// Cursor-position reports and mouse replies must reach the program or a
/// full-screen agent will hang waiting for an answer. Returns the number
/// copied; call again while it equals `capacity`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn farcooler_vt_take_writes(
    handle: *mut c_void,
    out: *mut u8,
    capacity: usize,
) -> usize {
    let Some(h) = (unsafe { as_handle(handle) }) else { return 0 };
    if out.is_null() || capacity == 0 || h.pending_writes.is_empty() {
        return 0;
    }
    let n = capacity.min(h.pending_writes.len());
    unsafe { std::ptr::copy_nonoverlapping(h.pending_writes.as_ptr(), out, n) };
    h.pending_writes.drain(..n);
    n
}

/// Take the bell flag, clearing it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn farcooler_vt_take_bell(handle: *mut c_void) -> bool {
    match unsafe { as_handle(handle) } {
        Some(h) => std::mem::take(&mut h.bell),
        None => false,
    }
}

/// The current window title, or null if the program never set one.
///
/// Borrowed; valid until the next `feed`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn farcooler_vt_title(handle: *mut c_void) -> *const c_char {
    match unsafe { as_handle(handle) } {
        Some(h) => h.title.as_ref().map_or(std::ptr::null(), |t| t.as_ptr()),
        None => std::ptr::null(),
    }
}

// MARK: - Input
//
// Key codes are Unicode scalar values for printable keys, so a renderer passes
// through whatever the platform's keyboard layout produced and never maps
// scancodes itself. Special keys live in the Unicode private use area, which
// no layout can generate, so the two spaces cannot collide.

pub const KEY_ENTER: u32 = 0xE000;
pub const KEY_TAB: u32 = 0xE001;
pub const KEY_BACKSPACE: u32 = 0xE002;
pub const KEY_ESCAPE: u32 = 0xE003;
pub const KEY_UP: u32 = 0xE004;
pub const KEY_DOWN: u32 = 0xE005;
pub const KEY_RIGHT: u32 = 0xE006;
pub const KEY_LEFT: u32 = 0xE007;
pub const KEY_HOME: u32 = 0xE008;
pub const KEY_END: u32 = 0xE009;
pub const KEY_PAGE_UP: u32 = 0xE00A;
pub const KEY_PAGE_DOWN: u32 = 0xE00B;
pub const KEY_INSERT: u32 = 0xE00C;
pub const KEY_DELETE: u32 = 0xE00D;
/// F1 through F12 are `KEY_F1 + n - 1`.
pub const KEY_F1: u32 = 0xE010;

pub const MOD_SHIFT: u32 = 1 << 0;
pub const MOD_ALT: u32 = 1 << 1;
pub const MOD_CTRL: u32 = 1 << 2;

pub const MOUSE_LEFT: u32 = 0;
pub const MOUSE_MIDDLE: u32 = 1;
pub const MOUSE_RIGHT: u32 = 2;
pub const MOUSE_WHEEL_UP: u32 = 3;
pub const MOUSE_WHEEL_DOWN: u32 = 4;

pub const MOUSE_PRESS: u32 = 0;
pub const MOUSE_RELEASE: u32 = 1;
pub const MOUSE_MOVE: u32 = 2;

/// Encode a keystroke into `out`, returning the number of bytes written.
///
/// The result depends on modes the emulator holds, so this must be called on
/// the handle rather than computed by the renderer. 16 bytes is ample for every
/// sequence this produces.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn farcooler_vt_encode_key(
    handle: *mut c_void,
    key: u32,
    modifiers: u32,
    out: *mut u8,
    capacity: usize,
) -> usize {
    let Some(h) = (unsafe { as_handle(handle) }) else { return 0 };
    let Some(key) = decode_key(key) else { return 0 };
    let bytes = h.terminal.encode_key(key, decode_mods(modifiers));
    unsafe { write_out(&bytes, out, capacity) }
}

/// Encode a mouse event. Returns 0 when the program does not want the event,
/// which is not an error: it means the client should handle it locally instead.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn farcooler_vt_encode_mouse(
    handle: *mut c_void,
    button: u32,
    action: u32,
    column: u16,
    row: u16,
    modifiers: u32,
    out: *mut u8,
    capacity: usize,
) -> usize {
    use crate::input::{MouseAction, MouseButton};

    let Some(h) = (unsafe { as_handle(handle) }) else { return 0 };
    let button = match button {
        MOUSE_LEFT => MouseButton::Left,
        MOUSE_MIDDLE => MouseButton::Middle,
        MOUSE_RIGHT => MouseButton::Right,
        MOUSE_WHEEL_UP => MouseButton::WheelUp,
        MOUSE_WHEEL_DOWN => MouseButton::WheelDown,
        _ => return 0,
    };
    let action = match action {
        MOUSE_PRESS => MouseAction::Press,
        MOUSE_RELEASE => MouseAction::Release,
        MOUSE_MOVE => MouseAction::Move,
        _ => return 0,
    };
    match h.terminal.encode_mouse(button, action, column, row, decode_mods(modifiers)) {
        Some(bytes) => unsafe { write_out(&bytes, out, capacity) },
        None => 0,
    }
}

/// Encode pasted text, bracketing it if the program asked for that.
///
/// Returns the number of bytes the encoding needs. If that exceeds `capacity`,
/// nothing is written — call again with a buffer at least that large. A paste
/// is arbitrarily long, so a silent truncation here would corrupt it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn farcooler_vt_encode_paste(
    handle: *mut c_void,
    text: *const u8,
    len: usize,
    out: *mut u8,
    capacity: usize,
) -> usize {
    let Some(h) = (unsafe { as_handle(handle) }) else { return 0 };
    if text.is_null() {
        return 0;
    }
    let slice = unsafe { std::slice::from_raw_parts(text, len) };
    let Ok(s) = std::str::from_utf8(slice) else { return 0 };
    let bytes = h.terminal.encode_paste(s);
    if bytes.len() > capacity || out.is_null() {
        return bytes.len();
    }
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), out, bytes.len()) };
    bytes.len()
}

/// True when the program has taken over the whole screen.
///
/// The client uses this to decide whether the wheel scrolls its own scrollback
/// or belongs to the program.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn farcooler_vt_alt_screen(handle: *mut c_void) -> bool {
    match unsafe { as_handle(handle) } {
        Some(h) => h.terminal.mode().contains(alacritty_terminal::term::TermMode::ALT_SCREEN),
        None => false,
    }
}

fn decode_key(key: u32) -> Option<crate::input::Key> {
    use crate::input::Key;
    Some(match key {
        KEY_ENTER => Key::Enter,
        KEY_TAB => Key::Tab,
        KEY_BACKSPACE => Key::Backspace,
        KEY_ESCAPE => Key::Escape,
        KEY_UP => Key::Up,
        KEY_DOWN => Key::Down,
        KEY_RIGHT => Key::Right,
        KEY_LEFT => Key::Left,
        KEY_HOME => Key::Home,
        KEY_END => Key::End,
        KEY_PAGE_UP => Key::PageUp,
        KEY_PAGE_DOWN => Key::PageDown,
        KEY_INSERT => Key::Insert,
        KEY_DELETE => Key::Delete,
        k if (KEY_F1..KEY_F1 + 12).contains(&k) => Key::Function((k - KEY_F1 + 1) as u8),
        k => Key::Char(char::from_u32(k)?),
    })
}

fn decode_mods(bits: u32) -> crate::input::Modifiers {
    crate::input::Modifiers {
        shift: bits & MOD_SHIFT != 0,
        alt: bits & MOD_ALT != 0,
        ctrl: bits & MOD_CTRL != 0,
    }
}

unsafe fn write_out(bytes: &[u8], out: *mut u8, capacity: usize) -> usize {
    if out.is_null() || bytes.len() > capacity {
        return 0;
    }
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), out, bytes.len()) };
    bytes.len()
}

fn pack(cell: &Cell) -> VtCell {
    let mut flags = 0;
    if cell.bold {
        flags |= FLAG_BOLD;
    }
    if cell.italic {
        flags |= FLAG_ITALIC;
    }
    if cell.underline {
        flags |= FLAG_UNDERLINE;
    }
    if cell.inverse {
        flags |= FLAG_INVERSE;
    }
    if cell.wide {
        flags |= FLAG_WIDE;
    }
    VtCell { ch: cell.ch as u32, fg: cell.fg, bg: cell.bg, flags, _pad: 0 }
}

unsafe fn as_handle<'a>(handle: *mut c_void) -> Option<&'a mut VtHandle> {
    if handle.is_null() {
        return None;
    }
    Some(unsafe { &mut *(handle as *mut VtHandle) })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive the ABI exactly as a renderer would.
    fn read(h: *mut c_void) -> (VtSnapshot, Vec<VtCell>) {
        let mut snap = VtSnapshot {
            cells: std::ptr::null(),
            columns: 0,
            rows: 0,
            cursor_row: 0,
            cursor_column: 0,
            cursor_visible: false,
            display_offset: 0,
            history_size: 0,
        };
        assert!(unsafe { farcooler_vt_snapshot(h, &mut snap) });
        let cells = unsafe {
            std::slice::from_raw_parts(snap.cells, snap.rows as usize * snap.columns as usize)
        }
        .to_vec();
        (snap, cells)
    }

    fn feed(h: *mut c_void, bytes: &[u8]) {
        unsafe { farcooler_vt_feed(h, bytes.as_ptr(), bytes.len()) };
    }

    #[test]
    fn a_full_round_trip_through_the_abi() {
        let h = farcooler_vt_new(40, 6);
        feed(h, b"\x1b[1;31mR\x1b[0mp");

        let (snap, cells) = read(h);
        assert_eq!((snap.columns, snap.rows), (40, 6));
        assert_eq!(cells.len(), 240);
        assert_eq!(cells[0].ch, 'R' as u32);
        assert_eq!(cells[0].flags & FLAG_BOLD, FLAG_BOLD);
        assert_ne!(cells[0].fg, cells[1].fg);

        unsafe { farcooler_vt_free(h) };
    }

    #[test]
    fn the_revision_moves_only_when_the_screen_might_have() {
        let h = farcooler_vt_new(40, 6);
        let start = unsafe { farcooler_vt_revision(h) };

        // Reading must not count as a change, or every frame would redraw.
        let _ = read(h);
        assert_eq!(unsafe { farcooler_vt_revision(h) }, start);

        feed(h, b"x");
        assert_ne!(unsafe { farcooler_vt_revision(h) }, start);

        unsafe { farcooler_vt_free(h) };
    }

    #[test]
    fn the_snapshot_buffer_is_reused_across_frames() {
        // This is the no-allocation-per-frame promise, stated as a test.
        let h = farcooler_vt_new(40, 6);
        feed(h, b"one");
        let first = read(h).0.cells;
        feed(h, b"two");
        let second = read(h).0.cells;
        assert_eq!(first, second, "the cell buffer must not be reallocated");
        unsafe { farcooler_vt_free(h) };
    }

    #[test]
    fn resize_is_visible_through_the_abi() {
        let h = farcooler_vt_new(40, 6);
        unsafe { farcooler_vt_resize(h, 80, 24) };
        let (snap, cells) = read(h);
        assert_eq!((snap.columns, snap.rows), (80, 24));
        assert_eq!(cells.len(), 80 * 24);
        unsafe { farcooler_vt_free(h) };
    }

    #[test]
    fn pty_replies_are_drained_in_order_and_only_once() {
        let h = farcooler_vt_new(40, 6);
        feed(h, b"\x1b[6n");

        let mut buf = [0u8; 64];
        let n = unsafe { farcooler_vt_take_writes(h, buf.as_mut_ptr(), buf.len()) };
        assert!(n > 0);
        assert_eq!(buf[0], 0x1b);

        let again = unsafe { farcooler_vt_take_writes(h, buf.as_mut_ptr(), buf.len()) };
        assert_eq!(again, 0, "taking must drain");

        unsafe { farcooler_vt_free(h) };
    }

    #[test]
    fn a_short_buffer_takes_a_prefix_and_leaves_the_rest() {
        let h = farcooler_vt_new(40, 6);
        feed(h, b"\x1b[6n");

        let mut one = [0u8; 1];
        assert_eq!(unsafe { farcooler_vt_take_writes(h, one.as_mut_ptr(), 1) }, 1);
        assert_eq!(one[0], 0x1b);

        let mut rest = [0u8; 64];
        assert!(unsafe { farcooler_vt_take_writes(h, rest.as_mut_ptr(), rest.len()) } > 0);

        unsafe { farcooler_vt_free(h) };
    }

    #[test]
    fn the_title_crosses_the_boundary() {
        let h = farcooler_vt_new(40, 6);
        assert!(unsafe { farcooler_vt_title(h) }.is_null());

        feed(h, b"\x1b]0;claude\x07");
        let ptr = unsafe { farcooler_vt_title(h) };
        assert!(!ptr.is_null());
        let s = unsafe { std::ffi::CStr::from_ptr(ptr) }.to_str().unwrap();
        assert_eq!(s, "claude");

        unsafe { farcooler_vt_free(h) };
    }

    #[test]
    fn the_bell_is_taken_once() {
        let h = farcooler_vt_new(40, 6);
        feed(h, b"\x07");
        assert!(unsafe { farcooler_vt_take_bell(h) });
        assert!(!unsafe { farcooler_vt_take_bell(h) });
        unsafe { farcooler_vt_free(h) };
    }

    #[test]
    fn null_and_empty_arguments_are_survivable() {
        // A renderer bug must not crash the app.
        let null = std::ptr::null_mut();
        unsafe {
            farcooler_vt_feed(null, b"x".as_ptr(), 1);
            farcooler_vt_resize(null, 10, 10);
            assert_eq!(farcooler_vt_revision(null), 0);
            assert!(!farcooler_vt_snapshot(null, std::ptr::null_mut()));
            assert_eq!(farcooler_vt_take_writes(null, std::ptr::null_mut(), 0), 0);
            assert!(!farcooler_vt_take_bell(null));
            assert!(farcooler_vt_title(null).is_null());
            farcooler_vt_free(null);
        }

        let h = farcooler_vt_new(40, 6);
        unsafe {
            farcooler_vt_feed(h, std::ptr::null(), 4);
            farcooler_vt_feed(h, b"x".as_ptr(), 0);
            assert!(!farcooler_vt_snapshot(h, std::ptr::null_mut()));
            farcooler_vt_free(h);
        }
    }

    fn key(h: *mut c_void, code: u32, mods: u32) -> Vec<u8> {
        let mut buf = [0u8; 16];
        let n = unsafe { farcooler_vt_encode_key(h, code, mods, buf.as_mut_ptr(), buf.len()) };
        buf[..n].to_vec()
    }

    #[test]
    fn keys_encode_against_the_live_terminal_modes() {
        let h = farcooler_vt_new(40, 6);

        assert_eq!(key(h, 'c' as u32, MOD_CTRL), vec![0x03]);
        assert_eq!(key(h, KEY_UP, 0), b"\x1b[A".to_vec());

        // The program turns on application cursor mode; the same key must now
        // produce different bytes. This is the whole reason encoding lives here.
        feed(h, b"\x1b[?1h");
        assert_eq!(key(h, KEY_UP, 0), b"\x1bOA".to_vec());

        assert_eq!(key(h, KEY_F1, 0), b"\x1bOP".to_vec());
        assert_eq!(key(h, KEY_F1 + 11, 0), b"\x1b[24~".to_vec());

        unsafe { farcooler_vt_free(h) };
    }

    #[test]
    fn mouse_encoding_follows_what_the_program_enabled() {
        let h = farcooler_vt_new(40, 6);
        let mut buf = [0u8; 32];

        let n = unsafe {
            farcooler_vt_encode_mouse(h, MOUSE_LEFT, MOUSE_PRESS, 0, 0, 0, buf.as_mut_ptr(), 32)
        };
        assert_eq!(n, 0, "silent until the program asks");

        feed(h, b"\x1b[?1000h\x1b[?1006h"); // click reporting, SGR encoding
        let n = unsafe {
            farcooler_vt_encode_mouse(h, MOUSE_LEFT, MOUSE_PRESS, 9, 4, 0, buf.as_mut_ptr(), 32)
        };
        assert_eq!(&buf[..n], b"\x1b[<0;10;5M");

        unsafe { farcooler_vt_free(h) };
    }

    #[test]
    fn paste_reports_the_size_it_needs_rather_than_truncating() {
        let h = farcooler_vt_new(40, 6);
        feed(h, b"\x1b[?2004h"); // bracketed paste

        let text = b"hello";
        let needed = unsafe {
            farcooler_vt_encode_paste(h, text.as_ptr(), text.len(), std::ptr::null_mut(), 0)
        };
        assert_eq!(needed, 5 + 12);

        let mut buf = vec![0u8; needed];
        let n =
            unsafe { farcooler_vt_encode_paste(h, text.as_ptr(), text.len(), buf.as_mut_ptr(), needed) };
        assert_eq!(n, needed);
        assert_eq!(buf, b"\x1b[200~hello\x1b[201~".to_vec());

        unsafe { farcooler_vt_free(h) };
    }

    #[test]
    fn scrolling_crosses_the_boundary_with_its_position() {
        let h = farcooler_vt_new(40, 6);
        for i in 0..30 {
            feed(h, format!("line{i}\r\n").as_bytes());
        }

        let live = read(h).0;
        assert_eq!(live.display_offset, 0);
        assert_eq!(live.history_size, 25);

        unsafe { farcooler_vt_scroll(h, 10) };
        let back = read(h).0;
        assert_eq!(back.display_offset, 10);
        assert!(!back.cursor_visible, "the caret is off-screen when scrolled back");

        unsafe { farcooler_vt_scroll_to_bottom(h) };
        assert_eq!(read(h).0.display_offset, 0);

        unsafe { farcooler_vt_free(h) };
    }

    #[test]
    fn the_alt_screen_flag_tracks_the_program() {
        let h = farcooler_vt_new(40, 6);
        assert!(!unsafe { farcooler_vt_alt_screen(h) });
        feed(h, b"\x1b[?1049h");
        assert!(unsafe { farcooler_vt_alt_screen(h) });
        feed(h, b"\x1b[?1049l");
        assert!(!unsafe { farcooler_vt_alt_screen(h) });
        unsafe { farcooler_vt_free(h) };
    }

    #[test]
    fn an_undersized_key_buffer_writes_nothing() {
        // A partial escape sequence would be interpreted as garbage input.
        let h = farcooler_vt_new(40, 6);
        let mut one = [0u8; 1];
        let n = unsafe { farcooler_vt_encode_key(h, KEY_UP, 0, one.as_mut_ptr(), 1) };
        assert_eq!(n, 0);
        assert_eq!(one[0], 0);
        unsafe { farcooler_vt_free(h) };
    }

    #[test]
    fn the_cell_layout_is_what_the_renderer_expects() {
        // Renderers index this array by raw offset. If the layout shifts without
        // the header changing, every glyph lands in the wrong place.
        assert_eq!(std::mem::size_of::<VtCell>(), 16);
        assert_eq!(std::mem::align_of::<VtCell>(), 4);
    }
}
