use std::ffi::c_char;
use farcooler_ffi_guard::guarded;

/// Pure local projection, not a network endpoint. The caller supplies authenticated context.
/// `request` must be a readable NUL-terminated UTF-8 string or NULL; `out` must be
/// NULL or writable for `capacity` bytes. Returns required length; a short buffer
/// is untouched. Output has no NUL terminator and uses the shared v1 response/error shape.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn farcooler_client_workflow_presentation(
    request: *const c_char, out: *mut u8, capacity: usize,
) -> usize {
    guarded(0, || {
        let Some(request) = (unsafe { super::read_str(request) }) else { return 0; };
        super::spill(&crate::workflow_presentation::json(&request), out, capacity)
    })
}
