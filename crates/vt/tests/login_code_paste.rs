//! What a phone sends when someone pastes a Claude sign-in code into a pane
//! (ov-604). The code is `code#state`, about a hundred characters, and it has
//! to arrive as one bracketed paste: typed a character at a time it is a
//! hundred separate keystrokes that the program may act on one by one.

use farcooler_vt::Terminal;

const CODE: &str = "Zk3_pQ9xLmN2vB7cD4fG8hJ1kR5tY6uW0aS-eXiOoPqRsTuVwXyZ0123456789abc#9f8e7d6c5b4a39281706f5e4d3c2b1a0";

fn terminal(bracketed: bool) -> Terminal {
    let mut t = Terminal::new(80, 24);
    if bracketed {
        t.feed(b"\x1b[?2004h");
    }
    t
}

#[test]
fn a_pasted_code_goes_out_as_one_bracketed_write_with_nothing_changed() {
    let out = terminal(true).encode_paste(CODE);
    let mut want = b"\x1b[200~".to_vec();
    want.extend_from_slice(CODE.as_bytes());
    want.extend_from_slice(b"\x1b[201~");
    assert_eq!(out, want);
}

#[test]
fn a_trailing_newline_stays_inside_the_paste() {
    let text = format!("{CODE}\n");
    let out = terminal(true).encode_paste(&text);
    assert!(out.starts_with(b"\x1b[200~"));
    assert!(out.ends_with(b"\n\x1b[201~"), "the newline is part of the paste, not a Return after it");
}

#[test]
fn a_program_that_has_not_asked_for_brackets_gets_the_code_raw() {
    assert_eq!(terminal(false).encode_paste(CODE), CODE.as_bytes());
}
