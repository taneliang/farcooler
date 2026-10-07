package com.farcooler.model

/**
 * An image waiting in a phone's conversation composer (ov-404): its bytes as the
 * runner takes them (PNG, JPEG, GIF or WebP). AgentKit's `OutgoingImage`, in the
 * same words and with the same limits, which the Mac's `ComposeImage` follows.
 *
 * The runner sniffs the bytes and refuses anything else, so an image in another
 * format (a HEIC photo) is converted as it's added: a JPEG at 0.9 when it's
 * opaque, as a photo is, a PNG when it has transparency; either at most
 * [LONGEST_EDGE] pixels on its long side. A kept format past [LARGEST_KEPT] is
 * converted the same way. The decoding is the platform's (`convertForRunner`);
 * the rule for when to use it is here, so the JVM tests hold it.
 */
class OutgoingImage(val id: Int, val mime: String, val data: ByteArray) {
    /** An image the platform re-encoded: its type and bytes. */
    class Converted(val mime: String, val data: ByteArray)

    companion object {
        /** The long side, in pixels, of an image that is converted: what claude reads an image at. */
        const val LONGEST_EDGE = 2576

        /** The largest file the runner takes a piece of (`MAX_PASTE_FILE_BYTES`). */
        const val LARGEST_KEPT = 16 * 1024 * 1024

        /** The type of [bytes] if it's a format the runner reads as it is, from the file's own first bytes. */
        fun sniff(bytes: ByteArray): String? {
            fun starts(vararg head: Int) = bytes.size >= head.size && head.indices.all { (bytes[it].toInt() and 0xFF) == head[it] }
            return when {
                starts(0x89, 0x50, 0x4E, 0x47) -> "image/png"
                starts(0xFF, 0xD8, 0xFF) -> "image/jpeg"
                starts(0x47, 0x49, 0x46, 0x38) -> "image/gif"
                // RIFF, four bytes of length, WEBP.
                starts(0x52, 0x49, 0x46, 0x46) && bytes.size >= 12 &&
                    String(bytes, 8, 4, Charsets.US_ASCII) == "WEBP" -> "image/webp"
                else -> null
            }
        }

        /**
         * [bytes] as the runner takes them: as they are when they're a format the
         * runner reads and fit, converted by [convert] when they aren't. Null when
         * they aren't an image.
         */
        fun make(id: Int, bytes: ByteArray, convert: (ByteArray) -> Converted?): OutgoingImage? {
            val kept = sniff(bytes)
            if (kept != null && bytes.size <= LARGEST_KEPT) return OutgoingImage(id, kept, bytes)
            return convert(bytes)?.let { OutgoingImage(id, it.mime, it.data) }
        }
    }
}
