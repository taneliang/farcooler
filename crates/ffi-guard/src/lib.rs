//! The panic guard every entry point the apps call goes through.
//!
//! A panic unwinding out of an `extern "C"` or `extern "system"` function
//! aborts the process — that is the defined behavior, not an accident — so
//! without this, any panic anywhere under the boundary is `SIGABRT` in the
//! Mac, iOS or Android app with nothing in the report but the signal. The app
//! is a phone in someone's hand, and much of what crosses the boundary is
//! terminal output from a runner nobody here controls; a recoverable fault
//! must not be able to close it.
//!
//! Two rules, and a test in `tests/every_export_is_guarded.rs` that holds the
//! workspace to the first:
//!
//! 1. **Every exported function's whole body is one guarded call.** Not the
//!    risky half of it: argument conversion panics too (a JNI array length
//!    turned into a capacity, a slice index), and an unguarded line before the
//!    guard is exactly as fatal as one inside the parser.
//! 2. **No lock is taken with `.expect(…)`.** A thread that panics while
//!    holding a `std::sync::Mutex` poisons it, and `.expect` then turns every
//!    later call into a panic of its own. See `locked`.

use std::sync::{Mutex, MutexGuard};

// Under `panic = "abort"`, `catch_unwind` catches nothing and every guard in
// the workspace is silently a no-op: the app aborts exactly as it did before
// any of them existed. No test can notice, because Cargo builds test targets
// and their dependencies with unwind whatever a profile says. So the build that
// ships is the one that refuses: any app library built under abort fails here.
#[cfg(not(panic = "unwind"))]
compile_error!("farcooler-ffi-guard needs panic = \"unwind\": catch_unwind is a no-op under abort");

/// Run an entry point's body so that a panic cannot leave it.
///
/// The fallback is each function's own "this did not happen" value: 0 for a
/// ticket or a length, false for a predicate, null for a pointer. Every one of
/// those is a value the apps already handle, because they are the same values
/// these functions return when they are simply asked for something impossible
/// — a null handle, a buffer that is too short.
///
/// `AssertUnwindSafe` because the arguments are raw pointers and handles from
/// C or the JVM, which carry no `UnwindSafe` claim and could not: their safety
/// contract is each module's, stated once at its top, and it is the caller's to
/// keep. A body whose state may be torn by a panic halfway through — the
/// terminal emulator's — uses `caught` and repairs it.
pub fn guarded<T>(fallback: T, body: impl FnOnce() -> T) -> T {
    caught(body).unwrap_or(fallback)
}

/// Run a body, and say whether it panicked rather than letting it unwind.
///
/// `None` means it panicked; the panic has been handed to `tracing`, which
/// records it only where a subscriber is installed — none of the apps install
/// one yet, so in a shipped app the record is the default panic hook's line on
/// stderr. For a caller
/// that has state to repair before it answers — `guarded` is this plus a
/// fallback.
pub fn caught<T>(body: impl FnOnce() -> T) -> Option<T> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)) {
        Ok(value) => Some(value),
        Err(payload) => {
            let what = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "a panic with no message".to_string());
            tracing::error!(panic = %what, "a Rust core panicked at the app boundary");
            None
        }
    }
}

/// Take a lock without caring whether a previous holder panicked.
///
/// `Mutex::lock` returns `Err` once any thread has panicked while holding it,
/// and an `.expect(…)` on that turns it into a panic of its own — one that
/// then unwound through `extern "C"` and aborted the app. So a single
/// recoverable fault permanently poisoned the lock and made every later call
/// from Swift or Kotlin fatal.
///
/// Ignoring the poison is right for the locks this guards rather than merely
/// convenient: queues of finished results, counters, maps of running streams,
/// signals a terminal raised. None of them has an invariant that a panic
/// elsewhere could have broken mid-update, and refusing to read them ever again
/// is strictly worse than reading them.
pub fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// A panic under the boundary must come back as a value, not a signal.
    ///
    /// This cannot see a profile set to `panic = "abort"` — Cargo builds tests
    /// with unwind regardless — which is what the `compile_error!` at the top
    /// of this file is for.
    #[test]
    fn a_panic_inside_an_entry_point_becomes_its_fallback() {
        assert_eq!(guarded(0u64, || panic!("the core fell over")), 0);
        assert!(!guarded(false, || -> bool { panic!("still no") }));
        assert!(guarded(std::ptr::null::<u8>(), || panic!("nor here")).is_null());
        assert_eq!(caught(|| -> u8 { panic!("{}", String::from("owned")) }), None);
    }

    /// And it only does that when something actually panicked.
    #[test]
    fn an_entry_point_that_returns_normally_is_untouched() {
        assert_eq!(guarded(0u64, || 42), 42);
        assert_eq!(caught(|| 7), Some(7));
    }

    /// One panic must not make every later call fatal.
    #[test]
    fn a_poisoned_lock_is_still_readable() {
        let queue: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        locked(&queue).push("survived".into());

        let poisoner = Arc::clone(&queue);
        let _ = std::thread::spawn(move || {
            let _held = poisoner.lock().unwrap();
            panic!("poison it");
        })
        .join();
        assert!(queue.lock().is_err(), "the lock really is poisoned");

        assert_eq!(locked(&queue).pop().as_deref(), Some("survived"));
    }
}
