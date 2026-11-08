//! Whether a running process is a given signed program, by the kernel's word
//! on its code signature (ov-974): the process satisfies the designated
//! requirement of the code at a path. Not a path, a name or a parent: a copy of
//! another program under the right name fails, and so does any other program
//! in the same bundle (the CLI and the daemon have identifiers of their own).
//! macOS only; elsewhere nothing is.

/// Whether process `pid` satisfies the designated requirement of the code at
/// `code` (a bundle or an executable). False when either can't be read.
#[cfg(target_os = "macos")]
pub fn satisfies(pid: i32, code: &std::path::Path) -> bool {
    if pid <= 0 {
        return false;
    }
    let before = crate::claude_registry::process_incarnation(pid);
    // SAFETY: each call is given live CF objects it only reads, and every object
    // made here is released once, by `Owned`.
    let valid = unsafe { ffi::satisfies(pid, code) };
    // The process checked is the one asked about, not a pid reused meanwhile.
    valid && before.is_some() && crate::claude_registry::process_incarnation(pid) == before
}

#[cfg(not(target_os = "macos"))]
pub fn satisfies(_pid: i32, _code: &std::path::Path) -> bool {
    false
}

#[cfg(target_os = "macos")]
mod ffi {
    use std::ffi::c_void;
    use std::os::unix::ffi::OsStrExt;

    type CFTypeRef = *const c_void;
    const NUMBER_SINT32: isize = 3;
    const DEFAULT_FLAGS: u32 = 0;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFTypeDictionaryKeyCallBacks: u8;
        static kCFTypeDictionaryValueCallBacks: u8;
        fn CFNumberCreate(allocator: CFTypeRef, kind: isize, value: *const c_void) -> CFTypeRef;
        fn CFDictionaryCreate(allocator: CFTypeRef, keys: *const CFTypeRef, values: *const CFTypeRef, count: isize, key_callbacks: *const u8, value_callbacks: *const u8) -> CFTypeRef;
        fn CFURLCreateFromFileSystemRepresentation(allocator: CFTypeRef, path: *const u8, length: isize, is_directory: u8) -> CFTypeRef;
        fn CFRelease(object: CFTypeRef);
    }
    #[link(name = "Security", kind = "framework")]
    unsafe extern "C" {
        static kSecGuestAttributePid: CFTypeRef;
        fn SecCodeCopyGuestWithAttributes(host: CFTypeRef, attributes: CFTypeRef, flags: u32, guest: *mut CFTypeRef) -> i32;
        fn SecStaticCodeCreateWithPath(path: CFTypeRef, flags: u32, code: *mut CFTypeRef) -> i32;
        fn SecCodeCopyDesignatedRequirement(code: CFTypeRef, flags: u32, requirement: *mut CFTypeRef) -> i32;
        fn SecCodeCheckValidity(code: CFTypeRef, flags: u32, requirement: CFTypeRef) -> i32;
    }

    /// A CF object this module made, released once.
    struct Owned(CFTypeRef);
    impl Drop for Owned {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: `self.0` is a CF object created here and released only here.
                unsafe { CFRelease(self.0) }
            }
        }
    }
    fn made(object: CFTypeRef) -> Option<Owned> {
        (!object.is_null()).then_some(Owned(object))
    }

    pub(super) unsafe fn satisfies(pid: i32, code: &std::path::Path) -> bool {
        unsafe {
            let bytes = code.as_os_str().as_bytes();
            let Some(url) = made(CFURLCreateFromFileSystemRepresentation(std::ptr::null(), bytes.as_ptr(), bytes.len() as isize, u8::from(code.is_dir()))) else { return false };
            let mut static_code = std::ptr::null();
            if SecStaticCodeCreateWithPath(url.0, DEFAULT_FLAGS, &mut static_code) != 0 { return false; }
            let Some(static_code) = made(static_code) else { return false };
            let mut requirement = std::ptr::null();
            if SecCodeCopyDesignatedRequirement(static_code.0, DEFAULT_FLAGS, &mut requirement) != 0 { return false; }
            let Some(requirement) = made(requirement) else { return false };
            let Some(number) = made(CFNumberCreate(std::ptr::null(), NUMBER_SINT32, (&raw const pid).cast())) else { return false };
            let keys = [kSecGuestAttributePid];
            let values = [number.0];
            let Some(attributes) = made(CFDictionaryCreate(std::ptr::null(), keys.as_ptr(), values.as_ptr(), 1,
                &raw const kCFTypeDictionaryKeyCallBacks, &raw const kCFTypeDictionaryValueCallBacks)) else { return false };
            let mut guest = std::ptr::null();
            if SecCodeCopyGuestWithAttributes(std::ptr::null(), attributes.0, DEFAULT_FLAGS, &mut guest) != 0 { return false; }
            let Some(guest) = made(guest) else { return false };
            SecCodeCheckValidity(guest.0, DEFAULT_FLAGS, requirement.0) == 0
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    /// A running `/bin/sleep` is `/bin/sleep`, by its signature, and not
    /// `/bin/cat`; a pid nobody has is nothing.
    #[test]
    fn a_process_is_the_program_its_signature_says() {
        let mut child = std::process::Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let pid = child.id() as i32;
        let sleep = std::path::Path::new("/bin/sleep");
        let mut seen = false;
        for _ in 0..50 {
            if super::satisfies(pid, sleep) { seen = true; break; }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(seen, "sleep is sleep");
        assert!(!super::satisfies(pid, std::path::Path::new("/bin/cat")), "and not cat");
        assert!(!super::satisfies(std::process::id() as i32, sleep), "this test is not sleep");
        assert!(!super::satisfies(pid, std::path::Path::new("/nowhere")), "no code at that path");
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(!super::satisfies(pid, sleep), "a process gone");
        assert!(!super::satisfies(0, sleep));
    }
}
