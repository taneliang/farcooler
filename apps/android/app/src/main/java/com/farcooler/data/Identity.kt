package com.farcooler.data

import android.content.Context
import android.content.SharedPreferences
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import com.farcooler.core.ClientCore
import com.farcooler.model.Trouble
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
     *
     * A [Trouble] rather than a string, because one of the three ways this is
     * set has the platform's own words to add and the other two do not. Those
     * words used to be spliced onto the end of the sentence with a colon, which
     * made a Java exception read as Far Cooler's account of the Keystore.
     */
    @Volatile
    var lastError: Trouble? = null
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
            lastError = Trouble("This device could not generate an SSH key.")
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
            lastError =
                Trouble("The stored SSH key could not be read; this device needs authorizing again.")
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
        // The sentence says what this side knows — the write did not take — and
        // the Keystore's own words go under it rather than onto the end of it.
        // No cause named and no retry promised: from here a `KeyStoreException`
        // could be a locked key, a strongbox that refused, or a full store, and
        // guessing sends somebody to change a setting that was never the
        // problem. See `Enrollment.note(about:outcome:)` in the Mac app.
        lastError = Trouble("This device’s SSH key couldn’t be stored.", it.message)
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

/**
 * This device's tailcat node key pair — the identity a tunneled runner admits.
 *
 * A node key is not an SSH key and does not replace one. [Identity] above is how
 * a runner decides this device may log in; this is how the TUNNEL decides this
 * device may reach the runner at all. A runner with no address on any network
 * this phone can see is unreachable without one, which is why nothing could ever
 * grant a phone a tunneled runner before this existed.
 *
 * **Minted here and never received.** The pair comes out of
 * [ClientCore.mintNodeKey] by value, not out of a file and not off the wire: a
 * private key that exists in two places is not an identity, and the runner only
 * ever learns the public half. The native entry point deliberately takes no path
 * so that this decision cannot be reversed quietly.
 *
 * **Once per device, not once per runner.** Ten tunneled runners are ten tokens
 * and one node key. `RunnerStore` in `crates/cli/src/runner_pipe.rs` holds it the
 * same way for the desktop, and for the same reason: it is a fact about this
 * device, not about any runner.
 *
 * **Both halves are stored, as one value.** [Identity.publicKey] derives its
 * public half every time and stores nothing, because storing it separately made
 * two facts that diverge. There is no entry point that derives a node public key
 * from a node private key, so the pair is kept together, under one preference
 * key, encrypted with one Keystore key, with one lifetime. Two facts that cannot
 * outlive each other cannot disagree.
 *
 * The storage split is [Identity]'s exactly: ciphertext in ordinary preferences,
 * the key that opens it in the Keystore where the app can use it but never read
 * it. A preferences file lifted off a rooted device, or out of a backup, is bytes
 * nobody can decrypt.
 */
object NodeIdentity {
    private const val PREFS = "farcooler.identity"
    private const val CIPHERTEXT = "nodeKey.ciphertext"
    private const val IV = "nodeKey.iv"
    private const val KEY_ALIAS = "farcooler.device.node"
    private const val TRANSFORMATION = "AES/GCM/NoPadding"
    private const val TAG_BITS = 128

    /**
     * One mint at a time, for the reason [Identity]'s lock exists: two callers
     * finding nothing and both minting would leave the device offering one
     * public half while holding the other's private one, and a tunnel ignores a
     * client it does not recognize in silence.
     */
    private val lock = Any()

    private lateinit var preferences: SharedPreferences

    fun initialize(context: Context) {
        preferences = context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
    }

    /**
     * The pair this device offers, minting one the first time it is needed.
     *
     * Null when this build cannot mint — an APK with no `libtailcat.so`, or a
     * core that answered `no_tailcat`. **That is not a failure to report.** The
     * answer to it is an offer carrying no node key, which is the `v=1` shape
     * the core has always accepted.
     */
    fun mintIfNeeded(): Pair<String, String>? = synchronized(lock) {
        // Re-read inside the lock: whoever held it may have just minted one.
        read()?.let { return it }
        val pair = ClientCore.mintNodeKey() ?: return null
        if (!write(pair)) return null
        pair
    }

    /** The public half to put in an offer, or null when this device has none. */
    val offeredPublicKey: String? get() = mintIfNeeded()?.second

    /**
     * The private half a dial needs — read, never minted.
     *
     * Dialing must not mint. A tunneled runner was granted against ONE public
     * half, which is now a line in that runner's allowlist; minting a second
     * pair here would produce a key nobody has authorized, and tailcat ignores
     * an unrecognized client without answering, so the symptom would be a
     * connection that hangs and then times out. Null is the honest answer and
     * `Connection` turns it into a sentence.
     */
    val storedPrivateKey: String? get() = synchronized(lock) { read()?.first }

    private fun read(): Pair<String, String>? {
        val ciphertext = preferences.getString(CIPHERTEXT, null) ?: return null
        val iv = preferences.getString(IV, null) ?: return null
        val plain = runCatching {
            val cipher = Cipher.getInstance(TRANSFORMATION)
            cipher.init(
                Cipher.DECRYPT_MODE,
                secretKey(),
                GCMParameterSpec(TAG_BITS, Base64.decode(iv, Base64.NO_WRAP)),
            )
            String(cipher.doFinal(Base64.decode(ciphertext, Base64.NO_WRAP)), Charsets.UTF_8)
        }.getOrElse {
            // A ciphertext the Keystore can no longer open — the same recovery
            // [Identity] takes, and for the same reason. Forgetting it does NOT
            // silently mint a replacement: the next dial to a tunneled runner
            // says so, because a fresh pair would be a key that runner's
            // allowlist has never heard of.
            preferences.edit().remove(CIPHERTEXT).remove(IV).apply()
            return null
        }
        return split(plain)
    }

    private fun write(pair: Pair<String, String>): Boolean = runCatching {
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, secretKey())
        val ciphertext = cipher.doFinal("${pair.first}\n${pair.second}".toByteArray(Charsets.UTF_8))
        preferences.edit()
            .putString(CIPHERTEXT, Base64.encodeToString(ciphertext, Base64.NO_WRAP))
            .putString(IV, Base64.encodeToString(cipher.iv, Base64.NO_WRAP))
            .commit()
    }.getOrElse { false } == true

    /**
     * Private first, public second — the order `mintNodeKey` writes them in and
     * the order `fc_tailcat_mint_node_key` writes them in underneath that.
     *
     * Refuses a half-written value rather than returning one: a blank public
     * half would be offered and admit nobody, and a blank private half would
     * dial as a client the tunnel has never heard of. Both fail silently, which
     * is the failure this whole mechanism exists to end.
     */
    internal fun split(stored: String): Pair<String, String>? {
        val lines = stored.split("\n")
        if (lines.size != 2) return null
        if (lines[0].isEmpty() || lines[1].isEmpty()) return null
        return lines[0] to lines[1]
    }

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
            // agent can be acted on without unlocking first — the same choice
            // [Identity] makes, and the same reason.
            .setUserAuthenticationRequired(false)
            .setRandomizedEncryptionRequired(true)
            .build()
        generator.init(spec)
        return generator.generateKey()
    }
}
