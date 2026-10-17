//! A count of whole-table reads, so a test can say a hot path does not make
//! one (ov-598).
//!
//! The reads that return a table's every row (`members`, `terminal_logins`,
//! `member_login_ids`) call `note_whole_table_read`. A release build keeps
//! nothing; with the `testing` feature each thread counts its own, so tests
//! running side by side never see each other's.

/// Record that the calling thread read a whole table. Nothing without the
/// `testing` feature.
#[inline]
pub(crate) fn note_whole_table_read() {
    #[cfg(any(test, feature = "testing"))]
    COUNT.with(|c| c.set(c.get() + 1));
}

#[cfg(any(test, feature = "testing"))]
thread_local! {
    static COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many whole-table reads this thread has made so far.
#[cfg(any(test, feature = "testing"))]
pub fn whole_table_reads() -> usize {
    COUNT.with(|c| c.get())
}
