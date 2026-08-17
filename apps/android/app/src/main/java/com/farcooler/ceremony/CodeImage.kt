package com.farcooler.ceremony

import android.graphics.Bitmap
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.FilterQuality
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.dp
import com.google.zxing.BarcodeFormat
import com.google.zxing.EncodeHintType
import com.google.zxing.qrcode.QRCodeWriter
import com.google.zxing.qrcode.decoder.ErrorCorrectionLevel

/**
 * A QR code, drawn large enough to scan off a screen.
 *
 * Android has no `CIQRCodeGenerator`, so this is ZXing — the same library the
 * scanner uses, which is most of why it is ZXing rather than ML Kit. See the
 * dependency note in `build.gradle.kts`.
 *
 * Rendered at ONE PIXEL PER MODULE and scaled up when it is drawn. A bitmap
 * scaled with [FilterQuality.None] is nearest-neighbour, so the modules stay
 * square-edged at any size; encoding straight into a 1000-pixel bitmap would
 * cost a megabyte of heap for the same picture, and encoding at some middle
 * size then filtering it up is how a code comes out blurred and unreadable.
 */
fun qrBitmap(payload: String): Bitmap? {
    if (payload.isEmpty()) return null
    val hints = mapOf(
        // Medium correction: the code is read off a lit screen at close range,
        // not off a printed label, so capacity is worth more than redundancy —
        // and a manifest can be 1800 bytes.
        EncodeHintType.ERROR_CORRECTION to ErrorCorrectionLevel.M,
        // Two modules of quiet zone in the bitmap itself, with the rest coming
        // from the white padding around it. A code flush to the edge of its
        // image is one a decoder cannot find the boundary of.
        EncodeHintType.MARGIN to 2,
        EncodeHintType.CHARACTER_SET to "UTF-8",
    )
    // A requested size of 1 means "no scaling": ZXing enlarges the output to
    // the smallest that fits, which is one pixel per module.
    val matrix = runCatching {
        QRCodeWriter().encode(payload, BarcodeFormat.QR_CODE, 1, 1, hints)
    }.getOrNull() ?: return null

    val width = matrix.width
    val height = matrix.height
    val pixels = IntArray(width * height)
    for (y in 0 until height) {
        val row = y * width
        for (x in 0 until width) {
            pixels[row + x] = if (matrix[x, y]) BLACK else WHITE
        }
    }
    return Bitmap.createBitmap(pixels, width, height, Bitmap.Config.ARGB_8888)
}

/**
 * The code, on a white card whatever the app's theme is.
 *
 * Not themed. A dark-mode code is an inverted code, which some decoders read
 * and some do not, and the one thing this picture has to do is be read by
 * whatever the other device is running.
 */
@Composable
fun CodeImage(payload: String, modifier: Modifier = Modifier) {
    val bitmap = remember(payload) { qrBitmap(payload) }
    Box(
        modifier
            .fillMaxWidth()
            .aspectRatio(1f)
            .clip(RoundedCornerShape(12.dp))
            .background(Color.White)
            .padding(16.dp),
    ) {
        if (bitmap != null) {
            Image(
                bitmap = bitmap.asImageBitmap(),
                contentDescription = "A code for another device to scan",
                modifier = Modifier.fillMaxWidth(),
                contentScale = ContentScale.Fit,
                filterQuality = FilterQuality.None,
            )
        }
    }
}

private const val BLACK = 0xFF000000.toInt()
private const val WHITE = 0xFFFFFFFF.toInt()
