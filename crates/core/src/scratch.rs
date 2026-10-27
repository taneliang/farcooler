//! Scratch directories for tests, removed when the test process ends (ov-541).
//!
//! A test used to make its directory with
//! `std::env::temp_dir().join(format!("farcooler-x-{}", std::process::id()))`
//! and never remove it, so every run left one more entry per test in `$TMPDIR`
//! (685,000 of them by 14 Oct). Every directory made here lives under one root
//! per test process, and that root is deleted by an `atexit` handler, so a test
//! that panics or forgets to clean up leaves nothing either. Code that is not a
//! test has no use for this; it is public only so the crates' own tests can share
//! it.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

static ROOT: OnceLock<PathBuf> = OnceLock::new();
static NEXT: AtomicU64 = AtomicU64::new(0);

/// Registered with `atexit`, where a panic must not unwind into C.
#[cfg(not(target_arch = "wasm32"))]
extern "C" fn remove_root() {
    farcooler_ffi_guard::guarded((), || {
        if let Some(root) = ROOT.get() {
            let _ = std::fs::remove_dir_all(root);
        }
    })
}

/// The directory every scratch directory of this process lives in.
pub fn root() -> &'static Path {
    ROOT.get_or_init(|| {
        // Short, because a test binds a Unix socket in one of these, and a path
        // over about 100 bytes is refused (keep tags short where one is bound).
        // The suffix is the clock's low digits, so a root left by a killed
        // process whose pid has come round again is never the one handed out.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let root = std::env::temp_dir().join(format!("fc-t{}-{nanos:x}", std::process::id()));
        std::fs::create_dir_all(&root).expect("a scratch root");
        // SAFETY: `remove_root` is an `extern "C"` function that takes nothing
        // and only removes a directory. No tests run in a browser (ov-768),
        // so there is nothing to remove there and no `atexit` to ask.
        #[cfg(not(target_arch = "wasm32"))]
        unsafe {
            libc::atexit(remove_root);
        }
        root
    })
}

/// A new, empty directory named for `tag`, unique in this process.
pub fn dir(tag: &str) -> PathBuf {
    let dir = root().join(format!("{tag}-{}", NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_directories_are_distinct_and_empty() {
        let a = dir("same");
        let b = dir("same");
        assert_ne!(a, b);
        assert!(a.starts_with(root()) && a.is_dir());
        assert_eq!(std::fs::read_dir(&a).unwrap().count(), 0);
    }

    #[test]
    fn the_root_is_removed_when_the_process_ends() {
        // A child process makes a directory and exits normally; the root it
        // made must be gone afterwards.
        let exe = std::env::current_exe().unwrap();
        let out = std::process::Command::new(exe)
            .args(["scratch::tests::child_makes_a_directory", "--exact", "--nocapture", "--ignored"])
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        let made = text.lines().find_map(|l| l.strip_prefix("SCRATCH=")).expect("the child printed its directory");
        assert!(!Path::new(made.trim()).exists(), "{made} outlived its process");
    }

    #[test]
    #[ignore = "run by the test above, as a child process"]
    fn child_makes_a_directory() {
        println!("SCRATCH={}", dir("child").display());
    }
}
