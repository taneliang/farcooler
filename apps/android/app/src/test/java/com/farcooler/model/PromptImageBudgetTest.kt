package com.farcooler.model

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * A picked photo fits one control envelope (1 MiB) and is a type an agent can
 * read. See [PromptImageBudget].
 */
class PromptImageBudgetTest {
    private val png = byteArrayOf(0x89.toByte(), 'P'.code.toByte(), 'N'.code.toByte(), 'G'.code.toByte())
    private val jpeg = byteArrayOf(0xFF.toByte(), 0xD8.toByte(), 0xFF.toByte())
    private val heic = byteArrayOf(0, 0, 0, 0x18, 'f'.code.toByte(), 't'.code.toByte(), 'y'.code.toByte(), 'p'.code.toByte())

    private fun file(magic: ByteArray, size: Int) = magic + ByteArray(size - magic.size)

    /**
     * Pretends to be a camera photo whose JPEG size grows with pixels and
     * quality, and records what it was asked for.
     */
    private class Photo(override val width: Int, override val height: Int, val bytesPerPixelAt80: Double) :
        PromptImageBudget.Encoder {
        val asked = mutableListOf<Pair<Double, Int>>()
        override fun jpeg(scale: Double, quality: Int): ByteArray {
            asked += scale to quality
            val pixels = width * scale * height * scale
            return ByteArray((pixels * bytesPerPixelAt80 * quality / 80).toInt())
        }
    }

    /** The bug: a 4032×3024 camera photo went out as its 4 MB original and was refused every time. */
    @Test
    fun `a camera photo is shrunk to fit the envelope`() {
        val photo = Photo(4032, 3024, bytesPerPixelAt80 = 0.3)
        val (data, mime) = assertNotNull(PromptImageBudget.fit(file(jpeg, 4_000_000)) { photo })
        assertTrue("${data.size} bytes", data.size <= PromptImageBudget.MAX_BYTES)
        assertEquals("image/jpeg", mime)
        assertEquals(1568.0 / 4032, photo.asked.first().first, 1e-9)
    }

    @Test
    fun `a screenshot inside the budget goes untouched`() {
        val shot = file(png, 200_000)
        val (data, mime) = PromptImageBudget.fit(shot) { error("must not re-encode") }!!
        assertArrayEquals(shot, data)
        assertEquals("image/png", mime)
    }

    /** As on the iPhone, a small GIF or WebP goes untouched, so a GIF keeps its animation. */
    @Test
    fun `a small GIF or WebP goes untouched`() {
        val gif = file("GIF89a".toByteArray(), 100_000)
        assertEquals("image/gif", PromptImageBudget.fit(gif) { error("must not re-encode") }!!.second)
        val webp = file("RIFF\u0000\u0000\u0000\u0000WEBP".toByteArray(), 100_000)
        val (data, mime) = PromptImageBudget.fit(webp) { error("must not re-encode") }!!
        assertEquals("image/webp", mime)
        assertArrayEquals(webp, data)
    }

    /** Both agents refuse HEIC, however small; the iPhone sends a small one labeled JPEG. */
    @Test
    fun `a small HEIC is re-encoded as JPEG, not relabeled`() {
        val photo = Photo(1000, 750, bytesPerPixelAt80 = 0.3)
        val (data, mime) = PromptImageBudget.fit(file(heic, 200_000)) { photo }!!
        assertEquals("image/jpeg", mime)
        assertEquals(1.0, photo.asked.first().first, 0.0)
        assertTrue(data.size <= PromptImageBudget.MAX_BYTES)
    }

    @Test
    fun `quality steps down until it fits`() {
        val photo = Photo(1568, 1176, bytesPerPixelAt80 = 0.5)
        PromptImageBudget.fit(file(jpeg, 2_000_000)) { photo }!!
        assertEquals(listOf(80, 60, 45, 30), photo.asked.map { it.second })
    }

    @Test
    fun `a stubborn picture halves its edge once more`() {
        val photo = Photo(1568, 1176, bytesPerPixelAt80 = 3.0)
        PromptImageBudget.fit(file(jpeg, 9_000_000)) { photo }!!
        assertEquals(0.5 to 50, photo.asked.last())
    }

    @Test
    fun `a picture that cannot be decoded is refused`() {
        assertEquals(null, PromptImageBudget.fit(file(heic, 2_000_000)) { null })
    }

    private fun <T : Any> assertNotNull(value: T?): T {
        assertNotNull("expected a value", value as Any?)
        return value!!
    }
}
