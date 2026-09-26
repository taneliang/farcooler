package com.farcooler.net

import com.farcooler.model.RunnerLink
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * Which phases answer. Connected and nothing weaker: a runner that stays down
 * spends most of its outage reconnecting, between attempts, and that is the
 * phase whose last fleet the screens went on counting.
 */
class RunnerLinkTest {
    /** Mutation: `Reconnecting` mapped to `ANSWERING`. Red. */
    @Test
    fun `only a connected runner is answering`() {
        assertEquals(RunnerLink.ANSWERING, Connection.Phase.Connected.link)
        assertEquals(RunnerLink.CONNECTING, Connection.Phase.Connecting.link)
        assertEquals(RunnerLink.AWAY, Connection.Phase.Reconnecting(attempt = 3).link)
        assertEquals(RunnerLink.AWAY, Connection.Phase.Failed("The runner didn’t answer.").link)
        assertEquals(RunnerLink.AWAY, Connection.Phase.NeedsApproval("SHA256:abc").link)
    }
}
