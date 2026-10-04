package com.farcooler.ceremony

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** A camera that won't start says so instead of leaving a black preview (ov-180). */
class CodeScannerTest {
    @Test
    fun `a bind that throws marks the camera failed until the scanner restarts`() {
        val scanner = CodeScanner()
        assertFalse(scanner.cameraFailed.value)
        scanner.bindCamera { throw IllegalStateException("Camera in use") }
        assertTrue(scanner.cameraFailed.value)
        scanner.start()
        assertFalse(scanner.cameraFailed.value)
    }

    @Test
    fun `a bind that works leaves the camera well`() {
        val scanner = CodeScanner()
        scanner.bindCamera { }
        assertFalse(scanner.cameraFailed.value)
    }
}
