package com.farcooler.ui

import android.graphics.Bitmap
import android.graphics.ImageDecoder
import com.farcooler.model.OutgoingImage
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import kotlin.math.max
import kotlin.math.roundToInt

/**
 * [bytes] decoded, scaled down to [OutgoingImage.LONGEST_EDGE], and written as a
 * JPEG at 0.9 when opaque or a PNG when not, or null when this device can't
 * decode them.
 *
 * Decoded by [ImageDecoder], which applies the EXIF orientation a camera photo
 * carries, at its target size rather than scaled down from a full-size bitmap
 * (a 48 MP photo is 190 MB of pixels to throw away).
 */
fun convertForRunner(bytes: ByteArray): OutgoingImage.Converted? = runCatching {
    val bitmap = ImageDecoder.decodeBitmap(ImageDecoder.createSource(ByteBuffer.wrap(bytes))) { decoder, info, _ ->
        // Software, because a hardware bitmap is drawn and not re-encoded.
        decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
        val longest = max(info.size.width, info.size.height)
        if (longest > OutgoingImage.LONGEST_EDGE) {
            val scale = OutgoingImage.LONGEST_EDGE.toDouble() / longest
            decoder.setTargetSize(
                max(1, (info.size.width * scale).roundToInt()),
                max(1, (info.size.height * scale).roundToInt()),
            )
        }
    }
    val opaque = !bitmap.hasAlpha()
    val out = ByteArrayOutputStream()
    val format = if (opaque) Bitmap.CompressFormat.JPEG else Bitmap.CompressFormat.PNG
    if (!bitmap.compress(format, 90, out)) null
    else OutgoingImage.Converted(if (opaque) "image/jpeg" else "image/png", out.toByteArray())
}.getOrNull()

/** A chip's picture: the image decoded small, once, not at full size on every redraw. */
fun chipPicture(bytes: ByteArray, side: Int = 96): Bitmap? = runCatching {
    ImageDecoder.decodeBitmap(ImageDecoder.createSource(ByteBuffer.wrap(bytes))) { decoder, info, _ ->
        decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
        val longest = max(info.size.width, info.size.height)
        if (longest > side) {
            val scale = side.toDouble() / longest
            decoder.setTargetSize(
                max(1, (info.size.width * scale).roundToInt()),
                max(1, (info.size.height * scale).roundToInt()),
            )
        }
    }
}.getOrNull()
