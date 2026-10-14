package com.farcooler.ceremony

import org.junit.Assert.assertEquals
import org.junit.Test

/** The fingerprint two people compare in person, in the form every platform shows (ov-525). */
class KeyFingerprintTest {
    @Test
    fun `sixteen characters of the key, eight from each end`() {
        assertEquals(
            "SHA256:iDqoxayS…j0Rg7vz4",
            CeremonyStore.abbreviated("SHA256:iDqoxaySm9gzxtvLrNXXpM5PimPLeBknaaNj0Rg7vz4"),
        )
        assertEquals("SHA256:short", CeremonyStore.abbreviated("SHA256:short"))
        assertEquals("not a fingerprint", CeremonyStore.abbreviated("not a fingerprint"))
    }
}
