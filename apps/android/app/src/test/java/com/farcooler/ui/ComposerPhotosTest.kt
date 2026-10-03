package com.farcooler.ui

import com.farcooler.net.AgentStream
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** A photo still being fitted holds the send, so it can't slip to the next prompt. See [ComposerPhotos]. */
class ComposerPhotosTest {
    private val photo = AgentStream.Attachment("image/jpeg", byteArrayOf(1, 2, 3))

    /** The bug: picked, sent before the fit landed, and attached to the next prompt instead. */
    @Test
    fun `a photo being prepared holds the send`() {
        val picked = ComposerPhotos().began()
        assertFalse(picked.canSend("look at this"))

        val ready = picked.finished(photo)
        assertTrue(ready.canSend("look at this"))
        assertEquals(listOf(photo), ready.ready)
    }

    @Test
    fun `a photo that couldn't be prepared releases the send`() {
        val failed = ComposerPhotos().began().finished(null)
        assertTrue(failed.canSend("hello"))
        assertFalse(failed.canSend(""))
    }

    @Test
    fun `two photos in flight hold the send until both land`() {
        val two = ComposerPhotos().began().began().finished(photo)
        assertFalse(two.canSend("x"))
        assertTrue(two.finished(photo).canSend(""))
    }

    @Test
    fun `sending empties the composer`() {
        val sent = ComposerPhotos().began().finished(photo).sent()
        assertEquals(ComposerPhotos(), sent)
    }
}
