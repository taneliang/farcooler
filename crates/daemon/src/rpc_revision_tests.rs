//! `screen_revision`: what tells a client its screen is unchanged.

use super::screen_revision;

#[test]
fn the_same_screen_has_the_same_revision() {
    assert_eq!(screen_revision("hello", 1, 2), screen_revision("hello", 1, 2));
}

#[test]
fn a_moved_cursor_is_a_different_screen() {
    // Nothing else changed, and a client told "unchanged" would leave the
    // caret in the wrong cell.
    assert_ne!(screen_revision("hello", 1, 2), screen_revision("hello", 2, 2));
    assert_ne!(screen_revision("hello", 1, 2), screen_revision("hello", 1, 3));
}

#[test]
fn different_contents_differ() {
    assert_ne!(screen_revision("hello", 0, 0), screen_revision("hellp", 0, 0));
}

#[test]
fn zero_is_never_a_real_revision() {
    // The wire uses it to mean "I hold nothing", so a screen that hashed to
    // it would be resent forever.
    for text in ["", "a", "the quick brown fox"] {
        assert_ne!(screen_revision(text, 0, 0), 0);
    }
}
