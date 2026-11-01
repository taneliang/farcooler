package com.farcooler.ceremony

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.concurrent.ExecutorService
import java.util.concurrent.TimeUnit

/**
 * The analyzer's thread outlives the provider arriving (ov-876).
 *
 * The scan screen's camera effect is keyed on the provider, so it runs first
 * with none and is disposed when one arrives. The thread used to be shut down
 * in that first disposal, and every frame after it was refused. This replays
 * that sequence exactly as `CameraPreview`'s `DisposableEffect` drives it.
 */
class AnalysisThreadTest {
    private val camera = Any()

    @Test
    fun `the thread bound once the provider arrives runs frames`() {
        var bound: ExecutorService? = null
        val unbound = mutableListOf<Any>()

        // First composition: no provider yet. Then it arrives and the effect
        // is disposed and run again.
        AnalysisThread.attach<Any>(null, { _, _ -> error("bound without a camera") }, { unbound += it })()
        val release = AnalysisThread.attach(camera, { _, executor -> bound = executor }, { unbound += it })

        val executor = bound!!
        val ran = executor.submit<Int> { 1 }
        assertEquals(1, ran.get(1, TimeUnit.SECONDS))
        assertTrue("nothing to unbind before a camera was bound", unbound.isEmpty())

        // Leaving the screen: unbound, and the thread stopped with it.
        release()
        assertEquals(listOf(camera), unbound)
        assertTrue(executor.isShutdown)
    }
}
