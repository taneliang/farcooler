package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * What an app is allowed to assume about a runner it is newer than.
 *
 * Play review and `farcooler host install` do not tick together, so a phone
 * weeks behind talking to a daemon updated this morning is the ordinary case.
 * These are the rules that make it survivable, and they match iOS exactly.
 */
class CapabilityTest {
    @Test
    fun aRunnerThatNamesAFeatureCanDoIt() {
        val daemon = DaemonBuild(
            version = "0.1.0+abc",
            matches = true,
            platform = "linux",
            capabilities = setOf("workspaces", "terminals", "changes"),
        )
        assertTrue(daemon.can("changes"))
        assertTrue(daemon.can("workspaces"))
    }

    @Test
    fun aRunnerThatDoesNotNameAFeatureCannotDoIt() {
        // The whole point: a control whose capability is absent gets dimmed
        // with a reason rather than offered and failing.
        val daemon = DaemonBuild(
            version = "0.1.0+abc",
            matches = true,
            platform = "linux",
            capabilities = setOf("workspaces", "terminals"),
        )
        assertFalse(daemon.can("changes"))
        assertFalse(daemon.can("stack"))
    }

    @Test
    fun aDaemonTooOldToAnswerStillGetsItsOldFeatures() {
        // Silence means a daemon predating capabilities entirely, so it has
        // exactly the feature set that existed then. Reading that as "can do
        // nothing" would blank the UI against every older runner.
        val ancient = DaemonBuild(version = "0.1.0+old", matches = false, platform = "linux")
        assertTrue(ancient.can("workspaces"))
        assertTrue(ancient.can("terminals"))
        assertFalse(ancient.can("changes"))
        assertFalse(ancient.can("stack"))
    }

    @Test
    fun capabilitiesAreSeparateFromWhetherTheBuildsMatch() {
        // Two different questions. `matches` is "were these built from the same
        // source"; `can` is "what does that runner do". A runner can be a
        // different build and still do everything this app needs.
        val different = DaemonBuild(
            version = "0.9.0+other",
            matches = false,
            platform = "linux",
            capabilities = setOf("workspaces", "terminals", "changes", "stack"),
        )
        assertFalse(different.matches)
        assertTrue(different.can("stack"))
    }

    @Test
    fun aNarrowerGrantIsWhatDimsARunnerControl() {
        // The two grants that are genuinely narrower than host_admin.
        val read = DaemonBuild(
            version = "1.0.0", matches = true, platform = "linux", grantedScope = "read",
        )
        val control = DaemonBuild(
            version = "1.0.0", matches = true, platform = "linux", grantedScope = "control",
        )
        assertFalse(read.mayAdministerRunner())
        assertFalse(control.mayAdministerRunner())

        val admin = DaemonBuild(
            version = "1.0.0", matches = true, platform = "linux", grantedScope = "host_admin",
        )
        assertTrue(admin.mayAdministerRunner())
    }

    @Test
    fun anUnrecognizedGrantKeepsEveryControlOffered() {
        // "No answer", never "no permission" — the distinction the whole
        // predicate turns on, and why it is a deny-list of the two narrow
        // grants rather than `== "host_admin"`.
        //
        // `unspecified` is what a runner too old to name a scope sends, and
        // what a NEWER runner naming a scope this build has no word for looks
        // like from here. Reading either as a refusal would let a new runner
        // silently strip controls off an older app.
        val silent = DaemonBuild(version = "1.0.0", matches = true, platform = "linux")
        assertEquals("unspecified", silent.grantedScope)
        assertTrue(silent.mayAdministerRunner())

        val fromTheFuture = DaemonBuild(
            version = "9.0.0", matches = false, platform = "linux",
            grantedScope = "some_scope_this_build_has_never_heard_of",
        )
        assertTrue(fromTheFuture.mayAdministerRunner())
    }
}
