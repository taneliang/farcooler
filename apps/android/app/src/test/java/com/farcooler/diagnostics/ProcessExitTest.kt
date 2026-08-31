package com.farcooler.diagnostics

import android.app.ApplicationExitInfo
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The exit-reason report, pinned.
 *
 * **This test is the reason the mapping is a separate function.**
 * `ApplicationExitInfo` cannot be constructed outside the system server, so a
 * `when` written inline where the records are read would be a `when` nothing
 * could check — and the branch that matters, `REASON_LOW_MEMORY`, is the one
 * that fires least often. A mapping that was silently wrong there would be
 * discovered by somebody concluding from a clean log that
 * `PaneDeck.MOUNT_LIMIT = 5` was safe, which is precisely the wrong conclusion
 * to reach from a broken instrument.
 *
 * These run on the JVM against android.jar's stubs. Only compile-time constants
 * are touched — `REASON_*` are `static final int` and are inlined — so nothing
 * here needs a device.
 */
class ProcessExitTest {

    /**
     * The whole point of the file: a low-memory kill is named, in full, in the
     * words somebody will grep a bug report for.
     */
    @Test
    fun `a low-memory kill is reported by name`() {
        val word = ProcessExit.word(ApplicationExitInfo.REASON_LOW_MEMORY)
        assertTrue("says the constant: $word", word.contains("REASON_LOW_MEMORY"))
        assertTrue("and says it in English: $word", word.contains("reclaimed"))
    }

    /**
     * A history that could only report low memory would leave a reader unable to
     * tell "no kills happened" from "this code reports nothing else", which is
     * the same unobservability the file exists to end, one level up. So every
     * reason the platform defines gets its own words.
     */
    @Test
    fun `every reason the platform defines says something different`() {
        val reasons = listOf(
            ApplicationExitInfo.REASON_UNKNOWN,
            ApplicationExitInfo.REASON_EXIT_SELF,
            ApplicationExitInfo.REASON_SIGNALED,
            ApplicationExitInfo.REASON_LOW_MEMORY,
            ApplicationExitInfo.REASON_CRASH,
            ApplicationExitInfo.REASON_CRASH_NATIVE,
            ApplicationExitInfo.REASON_ANR,
            ApplicationExitInfo.REASON_INITIALIZATION_FAILURE,
            ApplicationExitInfo.REASON_PERMISSION_CHANGE,
            ApplicationExitInfo.REASON_EXCESSIVE_RESOURCE_USAGE,
            ApplicationExitInfo.REASON_USER_REQUESTED,
            ApplicationExitInfo.REASON_USER_STOPPED,
            ApplicationExitInfo.REASON_DEPENDENCY_DIED,
            ApplicationExitInfo.REASON_OTHER,
            ApplicationExitInfo.REASON_FREEZER,
            ApplicationExitInfo.REASON_PACKAGE_STATE_CHANGE,
            ApplicationExitInfo.REASON_PACKAGE_UPDATED,
        )
        val words = reasons.map { ProcessExit.word(it) }
        assertEquals("no two reasons may share a phrase", reasons.size, words.toSet().size)
        assertTrue("none may be blank", words.none { it.isBlank() })
    }

    /**
     * A constant a future Android adds keeps the only part of itself that
     * identifies it. Flattening it into "other" would throw away a fact we
     * actually have.
     */
    @Test
    fun `an unrecognised reason prints its number`() {
        assertEquals("reason 99", ProcessExit.word(99))
        assertNotEquals(ProcessExit.word(99), ProcessExit.word(ApplicationExitInfo.REASON_OTHER))
    }

    /**
     * The RSS is the number `PaneDeck`'s budget is argued in, and it is printed
     * in megabytes because that is the unit the argument uses — five mounted
     * panes against "312 MB" is a comparison a person can make, and against
     * "319488" it is not.
     */
    @Test
    fun `the line carries the resident size in megabytes`() {
        val line = ProcessExit.describe(
            reason = ApplicationExitInfo.REASON_LOW_MEMORY,
            status = 0,
            rssKb = 319_488,
            timestamp = 1_000_000L,
            now = 1_000_000L + 7_200_000L,
        )
        assertTrue(line, line.contains("312 MB resident"))
        assertTrue(line, line.contains("REASON_LOW_MEMORY"))
        assertTrue("and how long ago: $line", line.contains("2h ago"))
    }

    /** A record with no RSS says nothing about memory rather than saying zero. */
    @Test
    fun `a record with no resident size omits it`() {
        val line = ProcessExit.describe(
            reason = ApplicationExitInfo.REASON_USER_REQUESTED,
            status = 0,
            rssKb = 0,
            timestamp = 0L,
            now = 0L,
        )
        assertTrue(line, !line.contains("MB"))
        assertTrue(line, !line.contains("0 MB"))
    }

    @Test
    fun `age is coarse and never negative`() {
        fun at(delta: Long) = ProcessExit.describe(
            reason = ApplicationExitInfo.REASON_CRASH,
            status = 0, rssKb = 0, timestamp = 0L, now = delta,
        )
        assertTrue(at(5_000).startsWith("just now"))
        assertTrue(at(120_000).startsWith("2m ago"))
        assertTrue(at(7_200_000).startsWith("2h ago"))
        assertTrue(at(3 * 86_400_000L).startsWith("3d ago"))
        // A clock that went backwards between the record and now must not print
        // a negative age; it is a diagnostic and it should degrade, not confuse.
        assertTrue(at(-5_000).startsWith("just now"))
    }

    /**
     * The exit status is only meaningful for the two reasons that carry one, and
     * printing `(0)` after "the person closed it" would be noise that reads like
     * information.
     */
    @Test
    fun `the exit status is printed only where it means something`() {
        val signalled = ProcessExit.describe(
            reason = ApplicationExitInfo.REASON_SIGNALED,
            status = 9, rssKb = 0, timestamp = 0L, now = 0L,
        )
        assertTrue(signalled, signalled.contains("(9)"))

        val closed = ProcessExit.describe(
            reason = ApplicationExitInfo.REASON_USER_REQUESTED,
            status = 0, rssKb = 0, timestamp = 0L, now = 0L,
        )
        assertTrue(closed, !closed.contains("("))
    }

    @Test
    fun `the platform's own description is appended when there is one`() {
        val line = ProcessExit.describe(
            reason = ApplicationExitInfo.REASON_ANR,
            status = 0, rssKb = 0, timestamp = 0L, now = 0L,
            description = "Input dispatching timed out",
        )
        assertTrue(line, line.endsWith("— Input dispatching timed out"))

        val blank = ProcessExit.describe(
            reason = ApplicationExitInfo.REASON_ANR,
            status = 0, rssKb = 0, timestamp = 0L, now = 0L, description = "   ",
        )
        assertTrue(blank, !blank.contains("—"))
    }
}
