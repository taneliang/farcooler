//! What counts as typing, on the bytes the clients' emulator really sends:
//! `farcooler_vt` fed what claude 2.1.292 writes, then asked for the mouse
//! reports and keys a person's gestures make, and for its replies.

use farcooler_vt::Terminal;
use farcooler_vt::input::{Key, Modifiers, MouseAction, MouseButton};

use super::{typed, typed_hex};

/// The modes claude 2.1.292 turns on as its prompt comes up, as a sandbox
/// claude wrote them: every kind of mouse report, SGR-encoded, and the
/// keyboard protocol's flags 5.
const CLAUDE_MODES: &[u8] = b"\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h\x1b[>5u";
/// What it asks the terminal as its prompt comes up, verbatim.
const CLAUDE_QUESTIONS: &[u8] = b"\x1b[>0q\x1b[?u\x1b[c\x1b[?2026$p\x1b[c";

fn claude() -> Terminal {
    let mut t = Terminal::new(160, 45);
    t.feed(CLAUDE_MODES);
    t
}

fn mouse(t: &Terminal, button: MouseButton, action: MouseAction) -> Vec<u8> {
    t.encode_mouse(button, action, 40, 12, Modifiers::default()).expect("claude asked for the mouse")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A click, a drag and the wheel over claude's pane, SGR as claude asks,
/// and X10 as a program asking for the mouse alone gets: no typing.
#[test]
fn mouse_reports_are_not_typing() {
    let t = claude();
    let mut reports = Vec::new();
    for (button, action) in [
        (MouseButton::Left, MouseAction::Press),
        (MouseButton::Left, MouseAction::Move),
        (MouseButton::Left, MouseAction::Release),
        (MouseButton::WheelUp, MouseAction::Press),
        (MouseButton::WheelDown, MouseAction::Press),
    ] {
        reports.push(mouse(&t, button, action));
    }
    assert_eq!(reports[0], b"\x1b[<0;41;13M");
    let wheel = [reports[3].clone(), reports[3].clone(), reports[3].clone()].concat();
    for report in reports.iter().chain([&wheel]) {
        assert!(!typed(report), "{report:?}");
        assert!(!typed_hex(&hex(report)), "{report:?} as hex");
    }
    let mut x10 = Terminal::new(160, 45);
    x10.feed(b"\x1b[?1000h");
    let click = mouse(&x10, MouseButton::Left, MouseAction::Press);
    assert_eq!(&click[..3], b"\x1b[M");
    assert!(!typed(&click));
    assert!(!typed(&mouse(&x10, MouseButton::Left, MouseAction::Release)));
    assert!(!typed(b"\x1b[32;41;13M"), "urxvt");
}

/// The emulator's answers to what claude asks: no typing.
#[test]
fn replies_are_not_typing() {
    let mut t = claude();
    t.take_signals();
    t.feed(CLAUDE_QUESTIONS);
    let replies = t.take_signals().pty_writes;
    assert!(replies.starts_with(b"\x1b[?5u\x1b[?6c"), "{:?}", String::from_utf8_lossy(&replies));
    assert!(!typed(&replies), "{:?}", String::from_utf8_lossy(&replies));
    for question in [&b"\x1b[5n"[..], b"\x1b[6n", b"\x1b[>c", b"\x1b[18t", b"\x1b[?2004$p", b"\x1b[4$p"] {
        t.feed(question);
        let reply = t.take_signals().pty_writes;
        assert!(!reply.is_empty(), "{question:?} answered");
        assert!(!typed(&reply), "{:?}", String::from_utf8_lossy(&reply));
    }
    for other in [
        &b"\x1b[I"[..],
        b"\x1b[O",
        b"\x1b]11;rgb:1e1e/1e1e/1e1e\x07",
        b"\x1b]4;1;rgb:cc/00/00\x1b\\",
        b"\x1bP>|farcooler 1.0\x1b\\",
        b"\x1bP1$r0m\x1b\\",
    ] {
        assert!(!typed(other), "{other:?}");
    }
}

/// Every key, a paste, and a report beside a key, are typing; so is
/// anything the channel can't decode.
#[test]
fn keys_and_pastes_are_typing() {
    let t = claude();
    let plain = Terminal::new(160, 45);
    let shift = Modifiers { shift: true, ..Modifiers::default() };
    let ctrl = Modifiers { ctrl: true, ..Modifiers::default() };
    for term in [&t, &plain] {
        for (key, mods) in [
            (Key::Char('a'), Modifiers::default()),
            (Key::Char('M'), shift),
            (Key::Char('c'), ctrl),
            (Key::Enter, Modifiers::default()),
            (Key::Escape, Modifiers::default()),
            (Key::Backspace, Modifiers::default()),
            (Key::Up, Modifiers::default()),
            (Key::Tab, shift),
            (Key::Function(3), shift),
            (Key::Function(3), ctrl),
        ] {
            let bytes = term.encode_key(key, mods);
            assert!(typed(&bytes), "{key:?} {mods:?}: {bytes:?}");
        }
        assert!(typed(&term.encode_paste("fix the flaky test")));
    }
    let click = mouse(&t, MouseButton::Left, MouseAction::Press);
    assert!(typed(&[&click[..], b"a"].concat()), "a report then a key");
    assert!(typed(&[&b"a"[..], &click[..]].concat()), "a key then a report");
    assert!(typed(b"\x1b[<0;41;13"), "a report cut short");
    assert!(typed(b"\x1b]11;rgb:1e1e/1e1e/1e1e"), "a reply with no end");
    assert!(typed(b"\x1bPq\x1b\\"), "a string no reply starts with");
    assert!(typed_hex("6"), "odd hex");
    assert!(typed_hex("zz"), "not hex");
    assert!(typed_hex(&hex(b"y")));
    assert!(!typed_hex(""), "nothing at all");
}
