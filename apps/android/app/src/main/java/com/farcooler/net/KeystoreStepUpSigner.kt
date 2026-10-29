package com.farcooler.net

import android.content.Context
import android.hardware.biometrics.BiometricManager
import android.hardware.biometrics.BiometricPrompt
import android.os.CancellationSignal
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import com.farcooler.model.StepUpFailure
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.PrivateKey
import java.security.Signature
import java.security.spec.ECGenParameterSpec
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlinx.coroutines.suspendCancellableCoroutine

/**
 * A Keystore P-256 key that asks the person each time it signs (ov-856, §3 of the design).
 *
 * - **Device only, in hardware where there is any.** StrongBox when the device has it, the TEE otherwise, so the key
 *   never leaves the device.
 * - **Verified for each signature.** `setUserAuthenticationParameters(0, BIOMETRIC_STRONG or DEVICE_CREDENTIAL)`
 *   makes the key unusable without an authentication tied to this very signature: the platform `BiometricPrompt`
 *   takes a `CryptoObject` over the [Signature], so one prompt is one action and the screen-lock credential is the
 *   fallback.
 * - **The platform's own prompt**, as [com.farcooler.ceremony.ConfirmingTap]: `minSdk` is 37, so the compatibility
 *   wrapper would be weight for levels this app does not build for.
 */
class KeystoreStepUpSigner(private val context: Context, private val alias: String = "farcooler.step-up") : StepUpSigning {
    private val store: KeyStore get() = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }

    override val publicKey: ByteArray?
        get() = store.getCertificate(alias)?.publicKey?.let(P256::uncompressed)

    override fun makeKey(): ByteArray {
        publicKey?.let { return it }
        val manager = context.getSystemService(BiometricManager::class.java)
        if (manager == null || manager.canAuthenticate(ALLOWED) != BiometricManager.BIOMETRIC_SUCCESS) {
            throw StepUpFailure.NoScreenLock
        }
        fun spec(strongBox: Boolean) = KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_SIGN)
            .setAlgorithmParameterSpec(ECGenParameterSpec("secp256r1"))
            .setDigests(KeyProperties.DIGEST_SHA256)
            .setUserAuthenticationRequired(true)
            .setUserAuthenticationParameters(0, KeyProperties.AUTH_BIOMETRIC_STRONG or KeyProperties.AUTH_DEVICE_CREDENTIAL)
            .setIsStrongBoxBacked(strongBox)
            .build()
        val generator = KeyPairGenerator.getInstance(KeyProperties.KEY_ALGORITHM_EC, "AndroidKeyStore")
        try {
            generator.initialize(spec(strongBox = true))
            generator.generateKeyPair()
        } catch (e: android.security.keystore.StrongBoxUnavailableException) {
            // Said plainly in the code and nowhere else: the TEE is the fallback, and the key is still device-only.
            generator.initialize(spec(strongBox = false))
            generator.generateKeyPair()
        }
        return publicKey ?: throw StepUpFailure.KeyUnavailable
    }

    override suspend fun sign(statement: ByteArray, reason: String): ByteArray {
        val key = store.getKey(alias, null) as? PrivateKey ?: throw StepUpFailure.KeyUnavailable
        val signature = Signature.getInstance("SHA256withECDSA").apply { initSign(key) }
        return suspendCancellableCoroutine { continuation ->
            val cancel = CancellationSignal()
            continuation.invokeOnCancellation { cancel.cancel() }
            val prompt = BiometricPrompt.Builder(context)
                .setTitle("Confirm")
                .setSubtitle(reason)
                .setAllowedAuthenticators(ALLOWED)
                .setConfirmationRequired(true)
                .build()
            prompt.authenticate(
                BiometricPrompt.CryptoObject(signature),
                cancel,
                context.mainExecutor,
                object : BiometricPrompt.AuthenticationCallback() {
                    override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                        if (!continuation.isActive) return
                        val signed = result.cryptoObject?.signature
                        try {
                            signed!!.update(statement)
                            continuation.resume(signed.sign())
                        } catch (e: Exception) {
                            continuation.resumeWithException(StepUpFailure.KeyUnavailable)
                        }
                    }

                    override fun onAuthenticationError(code: Int, message: CharSequence) {
                        if (!continuation.isActive) return
                        val closed = code == BiometricPrompt.BIOMETRIC_ERROR_USER_CANCELED ||
                            code == BiometricPrompt.BIOMETRIC_ERROR_CANCELED
                        continuation.resumeWithException(if (closed) StepUpFailure.Cancelled else StepUpFailure.KeyUnavailable)
                    }
                },
            )
        }
    }

    /** Forget the key (a device removed from its runner, or a reset). */
    fun deleteKey() {
        runCatching { store.deleteEntry(alias) }
    }
}

private const val ALLOWED = BiometricManager.Authenticators.BIOMETRIC_STRONG or BiometricManager.Authenticators.DEVICE_CREDENTIAL
