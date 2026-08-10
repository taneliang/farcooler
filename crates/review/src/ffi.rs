//! The C ABI.
//!
//! Small on purpose. The daemon does the git work and ships hunks over the
//! protocol, so a client never parses a patch. What a client DOES need locally
//! is the other kind of diff: an agent's tool call carries the file before and
//! after, with no git involved, and the transcript renders it as it streams.
//!
//! That computation used to live in Swift, which meant the same app held two
//! diff implementations — the transcript's and review's — with two line models
//! and two ideas of what a hunk is, sometimes on screen at once. It lives here
//! now for the same reason colour resolution lives in `farcooler-vt`: three
//! renderers cannot be trusted to agree, and these two describe the same file.
//!
//! ## Safety, once rather than per function
//!
//! Every function here takes NUL-terminated UTF-8 and returns either NULL or a
//! heap string this module owns until `farcooler_review_string_free`. Pointers
//! are either null or valid for the call. Null in means null or empty out, never
//! a crash: a renderer bug must not take down the app.
#![allow(clippy::missing_safety_doc)]

use std::ffi::{CStr, CString, c_char};

use crate::diff::diff_texts;

/// Diff two whole texts, as JSON.
///
/// JSON rather than a `#[repr(C)]` array, and deliberately: unlike a terminal
/// grid this is not redrawn at 60 Hz. It is computed once when a tool call
/// arrives and then scrolled, so one allocation and one parse per edit is
/// nothing, and the shape stays free to grow a field without every renderer
/// having to agree on a struct layout the same day.
///
/// Returns `[{"kind":"context|added|removed","old_no":N|null,"new_no":N|null,
/// "text":"..."}]`, or NULL if either input is not valid UTF-8.
///
/// The caller owns the result and must pass it to
/// `farcooler_review_string_free`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn farcooler_review_diff_texts(
    old: *const c_char,
    new: *const c_char,
) -> *mut c_char {
    let old = unsafe { cstr(old) };
    let new = unsafe { cstr(new) };
    let (Some(old), Some(new)) = (old, new) else {
        return std::ptr::null_mut();
    };

    let lines = diff_texts(old, new);
    match serde_json::to_string(&lines) {
        Ok(s) => to_c(s),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Free a string returned by this module.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn farcooler_review_string_free(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    // Reconstituting the CString drops it, which is the only correct way to
    // free memory Rust allocated.
    unsafe {
        let _ = CString::from_raw(s);
    }
}

/// How many lines changed between two texts, without allocating a JSON string.
///
/// The transcript shows a `+N −M` summary above a collapsed diff and only builds
/// the full line list when someone expands it. Counting through the JSON path
/// would allocate a whole document to throw it away.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn farcooler_review_count_changes(
    old: *const c_char,
    new: *const c_char,
    added_out: *mut u32,
    removed_out: *mut u32,
) -> bool {
    let old = unsafe { cstr(old) };
    let new = unsafe { cstr(new) };
    let (Some(old), Some(new)) = (old, new) else {
        return false;
    };

    let lines = diff_texts(old, new);
    let added = lines.iter().filter(|l| l.kind == crate::diff::LineKind::Added).count() as u32;
    let removed = lines.iter().filter(|l| l.kind == crate::diff::LineKind::Removed).count() as u32;

    if !added_out.is_null() {
        unsafe { *added_out = added };
    }
    if !removed_out.is_null() {
        unsafe { *removed_out = removed };
    }
    true
}

unsafe fn cstr<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        return Some("");
    }
    unsafe { CStr::from_ptr(p) }.to_str().ok()
}

fn to_c(s: String) -> *mut c_char {
    match CString::new(s) {
        Ok(c) => c.into_raw(),
        // A NUL inside the JSON would mean a NUL inside a source line, which is
        // possible in a binary-ish file. NULL rather than a truncated document.
        Err(_) => std::ptr::null_mut(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(s: &str) -> CString {
        CString::new(s).unwrap()
    }

    #[test]
    fn diffing_two_texts_returns_json_the_caller_then_frees() {
        let old = c("a\nb\n");
        let new = c("a\nB\n");
        let out = unsafe { farcooler_review_diff_texts(old.as_ptr(), new.as_ptr()) };
        assert!(!out.is_null());
        let json = unsafe { CStr::from_ptr(out) }.to_str().unwrap().to_string();
        unsafe { farcooler_review_string_free(out) };

        assert!(json.contains("\"removed\""));
        assert!(json.contains("\"added\""));
        assert!(json.contains("\"text\":\"B\""));
    }

    #[test]
    fn a_null_input_is_treated_as_empty_and_never_crashes() {
        let new = c("a\n");
        let out = unsafe { farcooler_review_diff_texts(std::ptr::null(), new.as_ptr()) };
        assert!(!out.is_null(), "null old text means the file is new, not an error");
        let json = unsafe { CStr::from_ptr(out) }.to_str().unwrap().to_string();
        unsafe { farcooler_review_string_free(out) };
        assert!(json.contains("\"added\""));
    }

    #[test]
    fn freeing_null_is_a_no_op() {
        unsafe { farcooler_review_string_free(std::ptr::null_mut()) };
    }

    #[test]
    fn counting_changes_avoids_building_the_document() {
        let old = c("a\nb\nc\n");
        let new = c("a\nB\nc\nd\n");
        let mut added = 0u32;
        let mut removed = 0u32;
        let ok = unsafe {
            farcooler_review_count_changes(old.as_ptr(), new.as_ptr(), &mut added, &mut removed)
        };
        assert!(ok);
        assert_eq!((added, removed), (2, 1), "B and d added, b removed");
    }

    #[test]
    fn counting_tolerates_null_out_pointers() {
        let old = c("a\n");
        let new = c("b\n");
        let ok = unsafe {
            farcooler_review_count_changes(
                old.as_ptr(),
                new.as_ptr(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        assert!(ok, "a caller that wants neither number is not an error");
    }
}
