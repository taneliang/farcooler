package com.farcooler.ceremony

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import javax.imageio.ImageIO
import kotlin.math.exp
import kotlin.math.roundToInt

/**
 * The code the Mac shows, as the Pixel's analysis frame sees it (ov-876).
 *
 * The fixtures are the Mac's own render, not a re-encode: `qrImage` in
 * `apps/macos/Sources/FarCooler/Ceremony/CodeImage.swift` run on a real
 * reply — `CIQRCodeGenerator` at correction M, one pixel per module and a
 * one-module white border, which is all the quiet zone the Mac draws before
 * its sheet's own background. The payloads beside them are what
 * `encode_manifest` writes for one and three tunneled runners that kept a LAN
 * address (ov-485): 660 bytes, a version-19 code, and 1604, a version-31 one.
 *
 * Each frame puts the code at [SHARE] of the frame's height, centered on the
 * sheet's background, area-sampled down to camera pixels (the camera
 * integrates light over each pixel; it does not pick the nearest module), then
 * softened by a small Gaussian for a lens that is not quite in focus, and
 * mapped to a lit screen's range rather than pure black and white.
 */
class QrDensityTest {
    private fun fixture(name: String): Pair<Array<BooleanArray>, String> {
        val loader = javaClass.classLoader!!
        val image = loader.getResourceAsStream("ceremony/$name.png")!!.use { ImageIO.read(it) }
        val payload = loader.getResourceAsStream("ceremony/$name.txt")!!.use { it.readBytes().decodeToString() }
        val dark = Array(image.height) { y ->
            BooleanArray(image.width) { x -> (image.getRGB(x, y) and 0xFF) < 128 }
        }
        return dark to payload
    }

    /** One luminance frame, [width] by [height], rows padded to [stride]. */
    private fun frame(
        code: Array<BooleanArray>,
        width: Int,
        height: Int,
        stride: Int,
        surround: Int,
    ): ByteArray {
        val modules = code.size
        val side = SHARE * height
        val left = (width - side) / 2
        val top = (height - side) / 2
        val raw = Array(height) { DoubleArray(width) }
        val sub = 4
        for (y in 0 until height) for (x in 0 until width) {
            var sum = 0.0
            for (sy in 0 until sub) for (sx in 0 until sub) {
                val u = (x + (sx + 0.5) / sub - left) / side * modules
                val v = (y + (sy + 0.5) / sub - top) / side * modules
                sum += if (u < 0 || v < 0 || u >= modules || v >= modules) {
                    surround.toDouble()
                } else if (code[v.toInt()][u.toInt()]) BLACK else WHITE
            }
            raw[y][x] = sum / (sub * sub)
        }
        val blurred = blur(raw, BLUR_SIGMA)
        val out = ByteArray(stride * height)
        for (y in 0 until height) for (x in 0 until width) {
            out[y * stride + x] = blurred[y][x].roundToInt().coerceIn(0, 255).toByte()
        }
        return out
    }

    private fun blur(src: Array<DoubleArray>, sigma: Double): Array<DoubleArray> {
        val r = (sigma * 3).toInt().coerceAtLeast(1)
        val k = DoubleArray(2 * r + 1) { exp(-((it - r) * (it - r)) / (2 * sigma * sigma)) }
        val norm = k.sum()
        val h = src.size
        val w = src[0].size
        val pass = Array(h) { y ->
            DoubleArray(w) { x -> (-r..r).sumOf { d -> src[y][(x + d).coerceIn(0, w - 1)] * k[d + r] } / norm }
        }
        return Array(h) { y ->
            DoubleArray(w) { x -> (-r..r).sumOf { d -> pass[(y + d).coerceIn(0, h - 1)][x] * k[d + r] } / norm }
        }
    }

    private fun read(name: String, width: Int, height: Int, surround: Int): Pair<String?, String> {
        val (code, payload) = fixture(name)
        // A padded row, as a real camera's Y plane has: the reader must use
        // the stride, not the width, to find the next row.
        val stride = width + 64
        return QrReader().read(frame(code, width, height, stride, surround), stride, width, height) to payload
    }

    @Test
    fun `the Mac's one-runner reply reads at the new analysis size on a light sheet`() {
        val (read, payload) = read("mac-reply-1-runner", 1280, 960, LIGHT_SHEET)
        assertEquals(payload, read)
    }

    @Test
    fun `the Mac's one-runner reply reads at the new analysis size on a dark sheet`() {
        val (read, payload) = read("mac-reply-1-runner", 1280, 960, DARK_SHEET)
        assertEquals(payload, read)
    }

    @Test
    fun `the Mac's three-runner reply reads at the new analysis size`() {
        val (read, payload) = read("mac-reply-3-runners", 1280, 960, DARK_SHEET)
        assertEquals(payload, read)
    }

    /**
     * Why the analysis size moved: at CameraX's default 640×480 the same code,
     * at the same share of the frame, is under two pixels a module and does
     * not read. If this starts passing, the default got better or the code
     * got sparser — measure again before trusting either.
     */
    @Test
    fun `the same reply at CameraX's default 640x480 does not read`() {
        val (read, _) = read("mac-reply-1-runner", 640, 480, DARK_SHEET)
        assertNull(read)
    }

    private companion object {
        /** The code's share of the frame's height. */
        const val SHARE = 0.4
        const val BLUR_SIGMA = 0.8
        /** A lit screen through a camera: neither end of the range. */
        const val WHITE = 215.0
        const val BLACK = 40.0
        /** The Mac's sheet background behind the code, light and dark appearance. */
        const val LIGHT_SHEET = 225
        const val DARK_SHEET = 45
    }
}
