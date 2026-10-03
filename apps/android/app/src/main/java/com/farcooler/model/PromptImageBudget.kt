package com.farcooler.model

import kotlin.math.max

/**
 * Making a picked photo small enough to ride WITH a prompt.
 *
 * A prompt's image is a content block inside ONE control envelope, and the
 * protocol caps an envelope at `MAX_CONTROL_ENVELOPE_BYTES` (1 MiB, in
 * `crates/protocol/src/lib.rs`). The composer used to send a picked photo's
 * bytes untouched, so a 4032×3024 camera photo, three to eight megabytes, could
 * never be sent and failed every time; and a HEIC went out as HEIC, which both
 * Claude Code and Codex refuse. The iPhone's `PromptImageBudget` is the same
 * rule, with one difference: it sends a small HEIC untouched labeled
 * `image/jpeg`, and this does not.
 *
 * Shrinking rather than raising the cap: the cap is a deliberate guard on the
 * control channel, and models downscale on receipt anyway.
 *
 * The decisions live here, away from `Bitmap`, so a JVM test can drive them
 * with a fake [Encoder].
 */
object PromptImageBudget {
    /**
     * Comfortably inside the 1 MiB envelope with the prompt, the terminal id and
     * framing alongside it, and several images can ride one prompt.
     */
    const val MAX_BYTES = 500 * 1024

    /** The long edge to fit inside. Past this a model downsamples anyway. */
    const val MAX_DIMENSION = 1568

    /**
     * Stepped down rather than guessed: the same pixel count compresses to
     * wildly different sizes depending on what is in the picture.
     */
    val QUALITIES = listOf(80, 60, 45, 30)

    /** Re-encodes a decoded picture. Null when it cannot. */
    interface Encoder {
        val width: Int
        val height: Int

        /** JPEG at [scale] of the original size, at [quality] (0–100). */
        fun jpeg(scale: Double, quality: Int): ByteArray?
    }

    /** What the bytes are, read from their magic numbers rather than trusted from a label. */
    fun sniff(bytes: ByteArray): String? = when {
        bytes.startsWith(0x89, 'P'.code, 'N'.code, 'G'.code) -> "image/png"
        bytes.startsWith(0xFF, 0xD8, 0xFF) -> "image/jpeg"
        bytes.startsWith('G'.code, 'I'.code, 'F'.code, '8'.code) -> "image/gif"
        bytes.startsWith('R'.code, 'I'.code, 'F'.code, 'F'.code) &&
            bytes.copyOfRange(8, minOf(12, bytes.size)).contentEquals("WEBP".toByteArray()) -> "image/webp"
        else -> null
    }

    /**
     * The bytes to send and their type, or null when the picture cannot be
     * prepared.
     *
     * A PNG, JPEG, GIF or WebP already inside the budget goes untouched, as
     * on the iPhone: a screenshot is usually small and full of small text a
     * re-encode would smear, and a GIF keeps its animation. These four are the
     * types the agents read. Anything else — too big, or a format an agent
     * refuses, like HEIC — is resized to [MAX_DIMENSION] and sent as JPEG at
     * the first quality that fits.
     */
    fun fit(original: ByteArray, encoder: () -> Encoder?): Pair<ByteArray, String>? {
        val kind = sniff(original)
        if (kind != null && original.size <= MAX_BYTES) return original to kind

        val image = encoder() ?: return null
        val longEdge = max(image.width, image.height)
        val scale = if (longEdge > MAX_DIMENSION) MAX_DIMENSION.toDouble() / longEdge else 1.0
        for (quality in QUALITIES) {
            val data = image.jpeg(scale, quality) ?: continue
            if (data.size <= MAX_BYTES) return data to "image/jpeg"
        }
        // Still too big at the lowest quality worth sending: halve the edge
        // once more and take what that gives. Something legible beats nothing.
        return image.jpeg(scale * 0.5, 50)?.let { it to "image/jpeg" }
    }

    private fun ByteArray.startsWith(vararg prefix: Int): Boolean =
        size >= prefix.size && prefix.indices.all { this[it] == prefix[it].toByte() }
}
