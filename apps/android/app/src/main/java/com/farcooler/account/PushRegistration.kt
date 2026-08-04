package com.farcooler.account

import android.os.Build
import com.google.firebase.messaging.FirebaseMessaging
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

/**
 * Where FCM delivers this device's address, and where it goes next.
 *
 * The local `Notifier` covers the case where the phone is awake and the app is
 * running. This covers the case the product exists for: the app is not running,
 * the device is asleep, and an agent on a machine three time zones away is
 * stuck. Nothing local can know that — only the daemon can, and the only route
 * to a sleeping device is FCM.
 *
 * The relay already speaks both platforms: `services/relay/src/index.ts`
 * accepts `apns` and `fcm`, and `push.ts` sends through the FCM v1 API. So
 * nothing on the server had to change for Android — this is the client half
 * that was missing.
 */
class PushRegistration(private val context: android.content.Context, private val account: Account) {

    /**
     * Whether the relay has this device's address.
     *
     * Published rather than swallowed, because the failure is the product's
     * central promise going quietly missing — the device is never filed, the
     * daemon's pushes reach nobody, and the settings screen goes on saying
     * notifications can reach this device.
     */
    private val _registered = MutableStateFlow(false)
    val registered: StateFlow<Boolean> = _registered.asStateFlow()

    private val _lastError = MutableStateFlow<String?>(null)
    val lastError: StateFlow<String?> = _lastError.asStateFlow()

    /**
     * The last token FCM gave us, held until there is an account to file it
     * under. Registration and sign-in finish in either order, and whichever is
     * second completes the pair.
     */
    private var token: String? = null

    private var scope: CoroutineScope? = null

    fun attach(scope: CoroutineScope) {
        this.scope = scope
        // A token FCM rotated while the app was closed.
        //
        // `FarCoolerMessagingService` runs in a process that may have no
        // signed-in account and no coroutine scope to file one with, so it
        // records the new address and this picks it up. Without it a rotation
        // means every later push reaches an address nobody is listening on, and
        // nothing anywhere says so.
        val preferences = context.getSharedPreferences(
            com.farcooler.notify.FarCoolerMessagingService.PREFS,
            android.content.Context.MODE_PRIVATE,
        )
        preferences
            .getString(com.farcooler.notify.FarCoolerMessagingService.KEY_PENDING_TOKEN, null)
            ?.let { pending ->
                token = pending
                preferences.edit()
                    .remove(com.farcooler.notify.FarCoolerMessagingService.KEY_PENDING_TOKEN)
                    .apply()
            }
        requestToken()
    }

    /**
     * Ask FCM for an address.
     *
     * Wrapped, because a build with no `google-services.json` has no Firebase
     * project and this throws. That is not worth telling anyone about: it is
     * the ordinary state of a checkout somebody just cloned, local
     * notifications keep working, and the settings screen says plainly that
     * pushes cannot reach this device rather than claiming they can.
     */
    private fun requestToken() {
        runCatching {
            FirebaseMessaging.getInstance().token
                .addOnSuccessListener { received(it) }
                .addOnFailureListener { unavailable(it.message) }
        }.onFailure { unavailable(it.message) }
    }

    fun received(deviceToken: String) {
        token = deviceToken
        scope?.launch { sendIfPossible() }
    }

    /** Called after signing in, for the case where the token arrived first. */
    suspend fun sendIfPossible() {
        val token = token ?: return
        if (!account.isSignedIn) return
        val ok = account.registerDevice(pushToken = token, platform = "fcm", label = label())
        _registered.value = ok
        _lastError.value =
            if (ok) null else "Could not tell the relay how to reach this device."
    }

    fun unavailable(reason: String?) {
        _registered.value = false
        _lastError.value =
            "This build has no Firebase project, so notifications cannot reach this device " +
                "while the app is closed."
    }

    /**
     * How this device names itself on the account's device list.
     *
     * The model, because Android stopped handing out user-set device names
     * without a permission and asking for one to label a row would be a poor
     * trade.
     */
    private fun label(): String = Build.MODEL.ifBlank { "Android device" }
}
