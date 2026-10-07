//! Which input a person typed, and which their terminal sent on its own.
//!
//! An input mark (`mark_input`) holds back the daemon's own typing: an
//! answer, a draft, a message from a native composer. Every client's
//! emulator (`farcooler_vt`, on the Mac, iOS and Android alike) writes to
//! the pane things nobody typed, though, down the same channel as keys:
//!
//! - mouse reports, once the program asks for the mouse. claude 2.1.292 asks
//!   for all of it (`?1000h ?1002h ?1003h ?1006h`), so a click in its pane or
//!   the wheel over it is a report: SGR `ESC [ < b ; x ; y M/m`, X10
//!   `ESC [ M` plus three bytes, urxvt `ESC [ b ; x ; y M`;
//! - focus reports, `ESC [ I` and `ESC [ O`;
//! - replies to the program's questions. claude asks `ESC [ > 0 q`,
//!   `ESC [ ? u`, `ESC [ c` and `ESC [ ? 2026 $ p` when it draws its prompt,
//!   and the emulator answers `ESC [ ? 6 c`, `ESC [ ? 5 u`, `ESC [ ? 2026 ; 2 $ y`;
//!   it also answers a cursor-position or status query, a mode query, and a
//!   size query. Color and version replies are covered too, though
//!   `farcooler_vt` sends neither today.
//!
//! When these marked too, a click or a scroll in claude's pane refused a
//! message sent mid-turn for 15 s as someone typing, its box empty (ov-407).
//! So a run of input marks unless every byte of it is one of these, whole:
//! a report beside a key is still typing.

/// Whether a run of input bytes holds anything a person typed or pasted:
/// anything but whole reports (this module's docs).
pub(crate) fn typed(bytes: &[u8]) -> bool {
    let mut rest = bytes;
    while !rest.is_empty() {
        match report(rest) {
            Some(n) => rest = &rest[n..],
            None => return true,
        }
    }
    false
}

/// `typed`, for a run as the input channel carries it: hex. A run that
/// isn't hex is typing, since doubt means don't type over it.
pub(crate) fn typed_hex(hex: &str) -> bool {
    let hex = hex.trim().as_bytes();
    if hex.len() % 2 != 0 {
        return true;
    }
    let digit = |c: u8| (c as char).to_digit(16);
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    for pair in hex.chunks(2) {
        let (Some(high), Some(low)) = (digit(pair[0]), digit(pair[1])) else { return true };
        bytes.push((high * 16 + low) as u8);
    }
    typed(&bytes)
}

const ESC: u8 = 0x1b;

/// The length of the report `bytes` starts with, if it starts with one.
fn report(bytes: &[u8]) -> Option<usize> {
    match bytes {
        [ESC, b'[', b'M', _, _, _, ..] => Some(6),
        [ESC, b'[', b'I' | b'O', ..] => Some(3),
        [ESC, b'[', ..] => csi(bytes),
        [ESC, b']', ..] => osc(bytes),
        [ESC, b'P', ..] => dcs(bytes),
        _ => None,
    }
}

/// A report shaped as a control sequence: `ESC [`, a private marker, numbers
/// separated by `;`, an intermediate, and a final byte.
fn csi(bytes: &[u8]) -> Option<usize> {
    let mut at = 2;
    let marker = bytes.get(at).copied().filter(|b| matches!(b, b'<' | b'?' | b'>'));
    at += usize::from(marker.is_some());
    let start = at;
    while bytes.get(at).is_some_and(|b| b.is_ascii_digit() || *b == b';') {
        at += 1;
    }
    let numbers: Vec<Option<u32>> =
        std::str::from_utf8(&bytes[start..at]).ok()?.split(';').map(|n| n.parse().ok()).collect();
    if numbers.iter().any(Option::is_none) {
        return None;
    }
    let numbers: Vec<u32> = numbers.into_iter().flatten().collect();
    let dollar = bytes.get(at) == Some(&b'$');
    at += usize::from(dollar);
    let last = *bytes.get(at)?;
    let is = match (marker, dollar, last, numbers.as_slice()) {
        // Mouse: SGR, and urxvt's three numbers.
        (Some(b'<'), false, b'M' | b'm', [_, _, _]) => true,
        (None, false, b'M', [_, _, _]) => true,
        // Device attributes, primary and secondary.
        (Some(b'?' | b'>'), false, b'c', [_, ..]) => true,
        // Status, and the cursor's position. `1;2R` to `1;16R` is also F3
        // with a modifier, so that much of row 1 counts as a key.
        (None, false, b'n', [0]) => true,
        (None, false, b'R', [row, column]) => !(*row == 1 && (2..=16).contains(column)),
        // A mode's state, private or not.
        (Some(b'?') | None, true, b'y', [_, _]) => true,
        // The keyboard protocol's flags.
        (Some(b'?'), false, b'u', [_]) => true,
        // The text area's size, in pixels or cells.
        (None, false, b't', [4 | 8, _, _]) => true,
        _ => false,
    };
    is.then_some(at + 1)
}

/// A color reply: `ESC ] 4 ; n ; rgb:…`, or `10` to `19` in place of
/// `4 ; n`, ended by BEL or `ESC \`.
fn osc(bytes: &[u8]) -> Option<usize> {
    let (body, len) = terminated(&bytes[2..], true)?;
    let body = std::str::from_utf8(body).ok()?;
    let (head, value) = body.rsplit_once(';')?;
    let color = match head.split_once(';') {
        Some(("4", index)) => index.parse::<u8>().is_ok(),
        None => head.parse::<u8>().is_ok_and(|n| (10..=19).contains(&n)),
        _ => false,
    };
    (color && value.starts_with("rgb:")).then_some(2 + len)
}

/// A device control string reply: the version (`>|`), a setting
/// (`1$r` or `0$r`), or the unit's id (`!|`), ended by `ESC \`.
fn dcs(bytes: &[u8]) -> Option<usize> {
    let (body, len) = terminated(&bytes[2..], false)?;
    [&b">|"[..], b"1$r", b"0$r", b"!|"].iter().any(|p| body.starts_with(p)).then_some(2 + len)
}

/// A string's body up to its terminator, and its length with it: `ESC \`,
/// or BEL when `bell` allows. `None` with no terminator, or a control
/// character inside, which no reply holds.
fn terminated(bytes: &[u8], bell: bool) -> Option<(&[u8], usize)> {
    for (at, &b) in bytes.iter().enumerate() {
        match b {
            0x07 if bell => return Some((&bytes[..at], at + 1)),
            ESC => return (bytes.get(at + 1) == Some(&b'\\')).then(|| (&bytes[..at], at + 2)),
            0x00..=0x1f | 0x7f => return None,
            _ => {}
        }
    }
    None
}

#[cfg(test)]
#[path = "typed_tests.rs"]
mod tests;
