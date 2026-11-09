package com.farcooler.ceremony

import android.content.Context
import android.hardware.biometrics.BiometricManager
import android.hardware.biometrics.BiometricPrompt
import android.os.CancellationSignal
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlin.coroutines.resume

// MARK: - The gate

/**
 * A fingerprint, at the moment of the tap.
 *
 * This is what someone standing at an unlocked phone runs into, and it is the
 * only thing between them and an enrolled device. An account check does not help
 * there: that phone is signed in, and they would be using its session.
 *
 * The platform's own `BiometricPrompt` rather than `androidx.biometric`, because
 * `minSdk = 37` makes the compatibility wrapper weight for API levels this app
 * does not build for. `BIOMETRIC_STRONG or DEVICE_CREDENTIAL` is what makes the
 * fallback the device's PIN, pattern or password and never nothing — a phone
 * with no biometrics enrolled still has to answer something. A cancelled or
 * failed authentication enrolls nothing.
 */
object ConfirmingTap {
    enum class Outcome {
        CONFIRMED,

        /** Someone changed their mind. Not a failure, and not worth a screen. */
        CANCELLED,

        /** It asked and did not get an answer it accepted, or it could not ask. */
        REFUSED,
    }

    private const val ALLOWED = BiometricManager.Authenticators.BIOMETRIC_STRONG or
        BiometricManager.Authenticators.DEVICE_CREDENTIAL

    suspend fun ask(context: Context, title: String, subtitle: String): Outcome =
        suspendCancellableCoroutine { continuation ->
            val manager = context.getSystemService(BiometricManager::class.java)
            if (manager == null ||
                manager.canAuthenticate(ALLOWED) != BiometricManager.BIOMETRIC_SUCCESS
            ) {
                // No biometric and no screen lock at all. Nothing to fall back
                // to, so nothing is enrolled — the alternative is an unguarded
                // tap, which is the whole thing this gate exists to prevent.
                continuation.resume(Outcome.REFUSED)
                return@suspendCancellableCoroutine
            }

            val cancel = CancellationSignal()
            continuation.invokeOnCancellation { cancel.cancel() }

            val prompt = BiometricPrompt.Builder(context)
                .setTitle(title)
                .setSubtitle(subtitle)
                .setAllowedAuthenticators(ALLOWED)
                // The explicit confirmation after a passive match. A face
                // unlocking as the phone is raised is not somebody agreeing to
                // enroll a device.
                .setConfirmationRequired(true)
                .build()

            prompt.authenticate(
                cancel,
                context.mainExecutor,
                object : BiometricPrompt.AuthenticationCallback() {
                    override fun onAuthenticationSucceeded(
                        result: BiometricPrompt.AuthenticationResult
                    ) {
                        if (continuation.isActive) continuation.resume(Outcome.CONFIRMED)
                    }

                    override fun onAuthenticationError(code: Int, message: CharSequence) {
                        // There is no negative-button case to consider: allowing
                        // DEVICE_CREDENTIAL means the prompt has no negative
                        // button, and `setNegativeButton` alongside it throws.
                        val changedTheirMind =
                            code == BiometricPrompt.BIOMETRIC_ERROR_USER_CANCELED ||
                                code == BiometricPrompt.BIOMETRIC_ERROR_CANCELED
                        if (continuation.isActive) {
                            continuation.resume(
                                if (changedTheirMind) Outcome.CANCELLED else Outcome.REFUSED
                            )
                        }
                    }

                    // `onAuthenticationFailed` is a finger that did not match,
                    // not an outcome: the prompt is still up, and it will either
                    // succeed or error.
                },
            )
        }
}
