use std::ffi::c_char;
use farcooler_ffi_guard::guarded;

/// The plan's view model (ov-1016): pure, no transport or authority.
/// `request` must be a readable NUL-terminated UTF-8 string or NULL; `out` must be
/// NULL or writable for `capacity` bytes. Returns the answer's length; a short
/// buffer is untouched. The answer has no NUL terminator. NULL request returns 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn farcooler_client_plan_view(request: *const c_char, out: *mut u8, capacity: usize) -> usize {
    guarded(0, || {
        let Some(request) = (unsafe { super::read_str(request) }) else { return 0; };
        super::spill(&crate::plan_view::json(&request), out, capacity)
    })
}
