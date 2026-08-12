package com.farcooler.model

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * What an app is allowed to assume about a machine it is newer than.
 *
 * Play review and `farcooler host install` do not tick together, so a phone
 * weeks behind talking to a daemon updated this morning is the ordinary case.
 * These are the rules that make it survivable, and they match iOS exactly.
 */
class CapabilityTest {
    @Test
    fun aMachineThatNamesAFeatureCanDoIt() {
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
    fun aMachineThatDoesNotNameAFeatureCannotDoIt() {
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
        // nothing" would blank the UI against every older machine.
        val ancient = DaemonBuild(version = "0.1.0+old", matches = false, platform = "linux")
        assertTrue(ancient.can("workspaces"))
        assertTrue(ancient.can("terminals"))
        assertFalse(ancient.can("changes"))
        assertFalse(ancient.can("stack"))
    }

    @Test
    fun capabilitiesAreSeparateFromWhetherTheBuildsMatch() {
        // Two different questions. `matches` is "were these built from the same
        // source"; `can` is "what does that machine do". A machine can be a
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
}
