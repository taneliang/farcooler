package com.farcooler.ceremony

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** The device that's already signed in can't paste a key, so its camera copy mustn't offer to. */
class ScanCopyTest {
    @Test
    fun `pasting is offered only where it is open`() {
        assertTrue(ScanCopy.cameraFailed(true).contains("pasting its key"))
        assertFalse(ScanCopy.cameraFailed(false).contains("past"))
        assertTrue(ScanCopy.cameraOff(true).contains("pasting its key"))
        assertFalse(ScanCopy.cameraOff(false).contains("past"))
    }
}
