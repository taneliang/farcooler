package com.farcooler.ui

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.ImageDecoder
import com.farcooler.model.PromptImageBudget
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import kotlin.math.max
import kotlin.math.roundToInt

/**
 * A picked photo's bytes, ready for [PromptImageBudget.fit] to re-encode, or
 * null when this device cannot decode them.
 *
 * Decoded by [ImageDecoder], which applies the EXIF orientation a camera photo
 * carries, so a re-encoded photo is the right way up. Each scale is decoded at
 * its target size rather than scaled down from a full-size bitmap, which for a
 * 4032×3024 photo would be 48 MB of pixels to throw away.
 */
fun promptImageEncoder(bytes: ByteArray): PromptImageBudget.Encoder? {
    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
    BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
    if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return null
    return DecodedPhoto(bytes, bounds.outWidth, bounds.outHeight)
}

private class DecodedPhoto(
    private val bytes: ByteArray,
    override val width: Int,
    override val height: Int,
) : PromptImageBudget.Encoder {
    private var decoded: Pair<Double, Bitmap>? = null

    override fun jpeg(scale: Double, quality: Int): ByteArray? {
        val bitmap = decoded?.takeIf { it.first == scale }?.second ?: decode(scale)?.also {
            decoded?.second?.recycle()
            decoded = scale to it
        } ?: return null
        val out = ByteArrayOutputStream()
        return if (bitmap.compress(Bitmap.CompressFormat.JPEG, quality, out)) out.toByteArray() else null
    }

    private fun decode(scale: Double): Bitmap? = runCatching {
        ImageDecoder.decodeBitmap(ImageDecoder.createSource(ByteBuffer.wrap(bytes))) { decoder, info, _ ->
            // Software, because a hardware bitmap is drawn and not re-encoded.
            decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
            if (scale < 1.0) {
                decoder.setTargetSize(
                    max(1, (info.size.width * scale).roundToInt()),
                    max(1, (info.size.height * scale).roundToInt()),
                )
            }
        }
    }.getOrNull()
}
