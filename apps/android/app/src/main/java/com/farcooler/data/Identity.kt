package com.farcooler.data

import android.content.Context
import android.content.SharedPreferences
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import com.farcooler.core.ClientCore
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * The device's SSH identity.
 *
 * The private key is encrypted with a key that lives in the Android Keystore
 * and is stored as ciphertext in ordinary preferences. That split is the whole
 * design: the Keystore holds a key the app can use but never read — on a Pixel
 * it is in the Titan M security chip — so a preferences file lifted off a
 * rooted device, or out of a backup, is bytes nobody can decrypt. Putting the
 * SSH key itself in preferences would have been account access for anyone who
 * could read one file.
 *
 * `setUserAuthenticationRequired(false)` is deliberate and matches the iOS
 * app's `kSecAttrAccessibleAfterFirstUnlock`: a push about a blocked agent
 * arrives while the phone is in a pocket, and the tap that opens the app has to
 * be able to connect without a biometric prompt first. `allowBackup=false` in
 * the manifest is the other half — a Keystore key cannot leave the device, so a
 * restored backup would carry a ciphertext nothing can open, and a device
 * restored from another phone has to be authorised separately. Which is the
 * behaviour you want the day a phone is lost.
 */
object Identity {
    private const val PREFS = "farcooler.identity"
    private const val CIPHERTEXT = "sshKey.ciphertext"
    private const val IV = "sshKey.iv"
    private const val KEY_ALIAS = "farcooler.device"
    private const val TRANSFORMATION = "AES/GCM/NoPadding"
    private const val TAG_BITS = 128

    /**
     * Why the key could not be produced, when it could not be.
     *
     * Recorded rather than swallowed. The iOS app learned this the hard way: a
     * discarded keychain status meant every call generated a fresh key, failed
     * to store it, and generated another next time — so the device
     * authenticated with one key while displaying a different one to authorise,
     * which looks exactly like a host rejecting a correct key.
     */
    @Volatile
    var lastError: String? = null
        private set

    /**
     * One generation at a time.
     *
     * [privateKey] reads, and generates only if it found nothing — which is safe
     * exactly once. At launch two callers ask at the same moment: the root
     * screen, to show the key you paste into a host, and a connection, to
     * authenticate with it. Both could find nothing, both generate, and the
     * second write replaces the first.
     */
    private val lock = Any()

    private lateinit var preferences: SharedPreferences

    fun initialize(context: Context) {
        preferences = context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
    }

    /** The device's private key, generating one on first use. */
    fun privateKey(): String? = synchronized(lock) {
        // Re-read inside the lock: whoever held it may have just created one.
        read()?.let { return it }
        val name = deviceName()
        val pair = ClientCore.generateKey(name) ?: run {
            lastError = "This device could not generate an SSH key."
            return null
        }
        if (!write(pair.first)) return null
        pair.first
    }

    /**
     * The public key to paste into a host's `authorized_keys`.
     *
     * Derived from the private key every time, never stored. Caching it
     * alongside made two sources for one fact, and they diverge — a reinstall
     * keeps one store and takes the other, so the app went on authenticating
     * with one key while showing a human a different one to authorise. Every
     * connection was then refused with a correct-looking key on screen.
     */
    val publicKey: String?
        get() = privateKey()?.let { ClientCore.publicKey(it) }

    /**
     * How this device names itself in a host's `authorized_keys`.
     *
     * The comment is what makes it possible to revoke one device without
     * guessing which line is which. [Build.MODEL] rather than a user-set device
     * name: Android stopped handing those out without a permission, and a model
     * name plus the app's prefix is enough to tell a phone from a tablet in a
     * file of three lines.
     */
    fun deviceName(): String {
        val model = Build.MODEL.ifBlank { "android" }.replace(' ', '-')
        return "farcooler-$model"
    }

    // MARK: - Storage

    private fun read(): String? {
        val ciphertext = preferences.getString(CIPHERTEXT, null) ?: return null
        val iv = preferences.getString(IV, null) ?: return null
        return runCatching {
            val cipher = Cipher.getInstance(TRANSFORMATION)
            cipher.init(
                Cipher.DECRYPT_MODE,
                secretKey(),
                GCMParameterSpec(TAG_BITS, Base64.decode(iv, Base64.NO_WRAP)),
            )
            String(cipher.doFinal(Base64.decode(ciphertext, Base64.NO_WRAP)), Charsets.UTF_8)
        }.getOrElse {
            // A ciphertext the Keystore can no longer open. That happens when
            // the Keystore key is gone — a factory reset restoring preferences
            // from a backup, or the user removing their screen lock on some
            // devices — and the only honest recovery is to forget it and
            // generate a new identity, which the runner will then refuse until
            // this device is authorized again. Saying so is what stops that
            // reading as a mysterious rejection.
            lastError = "The stored SSH key could not be read; this device needs authorizing again."
            preferences.edit().remove(CIPHERTEXT).remove(IV).apply()
            null
        }
    }

    private fun write(key: String): Boolean = runCatching {
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, secretKey())
        val ciphertext = cipher.doFinal(key.toByteArray(Charsets.UTF_8))
        preferences.edit()
            .putString(CIPHERTEXT, Base64.encodeToString(ciphertext, Base64.NO_WRAP))
            .putString(IV, Base64.encodeToString(cipher.iv, Base64.NO_WRAP))
            .commit()
    }.getOrElse {
        lastError = "This device’s SSH key could not be stored: ${it.message}"
        false
    } == true

    private fun secretKey(): SecretKey {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (store.getEntry(KEY_ALIAS, null) as? KeyStore.SecretKeyEntry)?.let { return it.secretKey }

        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        val spec = KeyGenParameterSpec.Builder(
            KEY_ALIAS,
            KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
        )
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            // Usable while the screen is locked, so a push about a blocked
            // agent can be acted on without unlocking first.
            .setUserAuthenticationRequired(false)
            .setRandomizedEncryptionRequired(true)
            .build()
        generator.init(spec)
        return generator.generateKey()
    }
}
