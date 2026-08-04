package com.farcooler.account

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * Where the session tokens live.
 *
 * Encrypted with a Keystore key, for the reason the Apple apps put theirs in
 * the Keychain: the refresh token is the durable credential — `/v1/auth/refresh`
 * will trade it for a fresh session indefinitely — so a preferences file
 * readable off a rooted device or out of a backup would be account takeover, no
 * exploit required.
 *
 * A separate Keystore alias from the SSH identity's, deliberately. They are
 * different credentials with different lifetimes: signing out must not be able
 * to reach the device key, and losing the device key must not sign you out.
 *
 * The rest of the account — the user id, the email — stays in plain
 * preferences. Those are labels, not credentials, and they are what the sign-in
 * row shows before anything asks the Keystore for anything.
 */
class TokenStore(context: Context) {
    private val preferences =
        context.applicationContext.getSharedPreferences("farcooler.tokens", Context.MODE_PRIVATE)

    fun read(key: String): String? {
        val ciphertext = preferences.getString("$key.ciphertext", null) ?: return null
        val iv = preferences.getString("$key.iv", null) ?: return null
        return runCatching {
            val cipher = Cipher.getInstance(TRANSFORMATION)
            cipher.init(
                Cipher.DECRYPT_MODE,
                secretKey(),
                GCMParameterSpec(TAG_BITS, Base64.decode(iv, Base64.NO_WRAP)),
            )
            String(cipher.doFinal(Base64.decode(ciphertext, Base64.NO_WRAP)), Charsets.UTF_8)
        }.getOrElse {
            // Unreadable means the key is gone, which means the session is
            // over. Clearing it is what turns "every call fails silently" into
            // "the settings screen shows a sign-in button".
            delete(key)
            null
        }
    }

    fun write(key: String, value: String): Boolean = runCatching {
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, secretKey())
        val ciphertext = cipher.doFinal(value.toByteArray(Charsets.UTF_8))
        preferences.edit()
            .putString("$key.ciphertext", Base64.encodeToString(ciphertext, Base64.NO_WRAP))
            .putString("$key.iv", Base64.encodeToString(cipher.iv, Base64.NO_WRAP))
            .commit()
    }.getOrDefault(false)

    fun delete(key: String) {
        preferences.edit().remove("$key.ciphertext").remove("$key.iv").apply()
    }

    private fun secretKey(): SecretKey {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (store.getEntry(ALIAS, null) as? KeyStore.SecretKeyEntry)?.let { return it.secretKey }

        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        generator.init(
            KeyGenParameterSpec.Builder(
                ALIAS,
                KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
            )
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                // Usable while the screen is locked: a push about a blocked
                // agent arrives while the phone is in a pocket, and the tap
                // that opens the app has to be able to refresh a session
                // without a biometric prompt in front of it.
                .setUserAuthenticationRequired(false)
                .setRandomizedEncryptionRequired(true)
                .build()
        )
        return generator.generateKey()
    }

    private companion object {
        const val ALIAS = "farcooler.account"
        const val TRANSFORMATION = "AES/GCM/NoPadding"
        const val TAG_BITS = 128
    }
}
