package com.farcooler.data

import com.farcooler.model.Trouble
import java.security.UnrecoverableKeyException
import javax.crypto.AEADBadTagException

/** What opening a stored key found (ov-572). */
sealed interface KeyRead {
    /** Nothing is stored in this slot. */
    data object Absent : KeyRead

    /** The key, decrypted. */
    data class Found(val value: String) : KeyRead

    /**
     * The Keystore key that sealed this ciphertext is gone or invalidated, so
     * it can never be opened again. The only outcome that justifies forgetting
     * the stored bytes.
     */
    data object Unusable : KeyRead

    /**
     * The Keystore failed in a way that may pass (a busy or restarting
     * Keystore daemon, a provider hiccup). Nothing is forgotten and nothing is
     * generated over the top: the stored key may be perfectly good.
     */
    data class Failed(val cause: Throwable) : KeyRead
}

/** Thrown by a reader when the Keystore has no entry under the alias. */
class KeyEntryMissing : Exception("no key under the alias")

/**
 * Tells a key that can never be opened from a read that merely failed.
 *
 * Before this, any exception while decrypting deleted the phone's SSH key, so
 * one transient Keystore failure locked the phone out of every runner. Now
 * only the cases below are [KeyRead.Unusable]; everything else is retried
 * briefly and then reported as [KeyRead.Failed], never as a reason to delete.
 */
object KeyReader {
    private const val ATTEMPTS = 3
    private const val PAUSE_MS = 150L

    /** Class names, not types: the Android classes cannot be loaded in a JVM test. */
    private val UNUSABLE_NAMES = setOf("android.security.keystore.KeyPermanentlyInvalidatedException")

    fun isUnusable(error: Throwable): Boolean = generateSequence(error) { it.cause }.take(8).any {
        it is KeyEntryMissing ||
            it is UnrecoverableKeyException ||
            // The alias was regenerated after a reset: the new key cannot open the old bytes.
            it is AEADBadTagException ||
            // Corrupt base64 or a malformed IV: retrying reads the same bytes.
            it is IllegalArgumentException ||
            it.javaClass.name in UNUSABLE_NAMES
    }

    /** Runs [open], retrying a failure that is not [isUnusable] up to [ATTEMPTS] times. */
    fun read(pause: (Long) -> Unit = { Thread.sleep(it) }, open: () -> String): KeyRead {
        var last: Throwable? = null
        repeat(ATTEMPTS) { attempt ->
            try {
                return KeyRead.Found(open())
            } catch (e: Exception) {
                if (isUnusable(e)) return KeyRead.Unusable
                last = e
                if (attempt < ATTEMPTS - 1) pause(PAUSE_MS)
            }
        }
        return KeyRead.Failed(last ?: IllegalStateException("no attempt ran"))
    }

    /**
     * Acts on a finished read: reports what the person needs to know and, only
     * for the key in use that is [KeyRead.Unusable], calls [forget] so a new
     * identity can be made. A [KeyRead.Failed] never calls [forget].
     */
    fun settle(result: KeyRead, current: Boolean, forget: () -> Unit, report: (Trouble) -> Unit) {
        when (result) {
            is KeyRead.Unusable -> if (current) {
                report(Trouble("This device’s SSH key couldn’t be read. Connect this device again."))
                forget()
            }
            is KeyRead.Failed -> if (current) {
                report(Trouble("This device’s SSH key couldn’t be read right now. Try again in a moment.", result.cause.message))
            }
            else -> Unit
        }
    }
}
