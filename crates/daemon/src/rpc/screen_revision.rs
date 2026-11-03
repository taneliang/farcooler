/// A cheap identity for a screen.
///
/// FNV-1a over the capture and the cursor. Not a checksum anyone relies on for
/// correctness — a collision means one stale frame until the next change, which
/// is a redraw, not corruption — and it is compared only against a value this
/// same runner produced moments earlier.
///
/// The scrollback is deliberately not in it. This number answers "is the screen
/// I hold still the screen?", and history is not diffed against anything: it is
/// asked for explicitly, by a client that has none, and sent whenever it is
/// asked for. Hashing it in would change the revision every time a line scrolled
/// off — resending screens a client already holds, which is the whole cost this
/// number exists to avoid.
pub(super) fn screen_revision(contents: &str, cursor_column: u32, cursor_row: u32) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x1000_0000_01b3);
        }
    };
    eat(contents.as_bytes());
    eat(&cursor_column.to_le_bytes());
    eat(&cursor_row.to_le_bytes());
    // Zero means "I have nothing" on the wire, so it must never be a real value.
    if hash == 0 { 1 } else { hash }
}

