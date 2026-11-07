//! The durable boundaries of a cutover, named so a test can really crash a
//! process at each one. In a shipped build `reach` is empty and no environment
//! variable is read: the kill exists only under `cfg(test)` or the `testing`
//! feature, which only dev-dependencies turn on.

/// Every boundary, in the order a forward run reaches it. Recover's come last.
pub const ALL: &[&str] = &[
    "hold.before", "hold.torn", "hold.after",
    "begin.before-baseline-db", "begin.after-baseline-db", "begin.torn-baseline", "begin.before-intent", "begin.torn-intent",
    "begin.after-intent", "begin.in-upgrade", "begin.after-upgrade",
    "commit.before-copy", "commit.after-copy", "commit.torn-snapshot", "commit.after-snapshot", "commit.after-restore", "journal.torn", "commit.after-permit",
    "commit.in-mapping", "commit.after-mapping", "commit.after-journal",
    "verify.after", "release.in-transaction", "release.after", "finish.before-rename", "finish.after-rename",
    "recover.before-rename", "recover.after-rename", "recover.after-retire",
];
pub const KILL_AT: &str = "FARCOOLER_TEST_CUTOVER_KILL_AT";

#[cfg(not(any(test, feature = "testing")))]
#[inline(always)]
pub fn reach(_name: &'static str) {}

/// Die here, as a power cut or `kill -9` would, if the test named this boundary.
#[cfg(any(test, feature = "testing"))]
pub fn reach(name: &'static str) {
    assert!(ALL.contains(&name), "unlisted cutover boundary {name}");
    if std::env::var(KILL_AT).as_deref() == Ok(name) {
        // SAFETY: signaling this process; nothing is passed to the kernel but numbers.
        unsafe { libc::kill(libc::getpid(), libc::SIGKILL); }
        loop { std::thread::park(); }
    }
}
