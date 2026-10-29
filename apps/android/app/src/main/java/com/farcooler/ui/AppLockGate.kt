package com.farcooler.ui

import android.content.Context
import android.hardware.biometrics.BiometricManager
import android.hardware.biometrics.BiometricPrompt
import android.os.CancellationSignal
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import com.farcooler.model.AppLock
import kotlin.coroutines.resume
import kotlinx.coroutines.launch
import kotlinx.coroutines.suspendCancellableCoroutine

/**
 * Fingerprint, face or screen lock, asked once, with nothing remembered between two (ov-856, §3a). The platform's own
 * `BiometricPrompt`, as [com.farcooler.ceremony.ConfirmingTap].
 */
object AppLockAuth {
    private const val ALLOWED = BiometricManager.Authenticators.BIOMETRIC_STRONG or BiometricManager.Authenticators.DEVICE_CREDENTIAL

    /**
     * Whether this device can ask at all: with no screen lock set there is nothing to ask with, and a lock that can't
     * open would only lock the person out of their own runners.
     */
    fun canAsk(context: Context): Boolean =
        context.getSystemService(BiometricManager::class.java)?.canAuthenticate(ALLOWED) == BiometricManager.BIOMETRIC_SUCCESS

    suspend fun ask(context: Context, reason: String = AppLock.PROMPT): Boolean = suspendCancellableCoroutine { continuation ->
        val cancel = CancellationSignal()
        continuation.invokeOnCancellation { cancel.cancel() }
        BiometricPrompt.Builder(context)
            .setTitle("Far Cooler is locked")
            .setSubtitle(reason)
            .setAllowedAuthenticators(ALLOWED)
            .setConfirmationRequired(false)
            .build()
            .authenticate(
                cancel,
                context.mainExecutor,
                object : BiometricPrompt.AuthenticationCallback() {
                    override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                        if (continuation.isActive) continuation.resume(true)
                    }

                    override fun onAuthenticationError(code: Int, message: CharSequence) {
                        if (continuation.isActive) continuation.resume(false)
                    }
                },
            )
    }
}

/**
 * The lock: a prompt when the app resumes after five minutes away, and at launch. [on] is the Settings switch;
 * [canAsk] and [ask] are the device's, given so a test can answer them.
 */
@Composable
fun AppLockGate(
    on: Boolean,
    now: () -> Long = System::currentTimeMillis,
    canAsk: (Context) -> Boolean = AppLockAuth::canAsk,
    ask: suspend (Context) -> Boolean = AppLockAuth::ask,
    content: @Composable () -> Unit,
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val lock = remember { AppLock.atLaunch(on) }
    var locked by remember { mutableStateOf(lock.isLocked) }
    var asking by remember { mutableStateOf(false) }
    var declined by remember { mutableStateOf(false) }
    lock.isOn = on

    fun attempt() {
        if (asking) return
        if (!canAsk(context)) {
            lock.unlock()
            locked = false
            return
        }
        asking = true
        scope.launch {
            val opened = ask(context)
            asking = false
            declined = !opened
            if (opened) {
                lock.unlock()
                locked = false
            }
        }
    }

    val owner = LocalLifecycleOwner.current
    DisposableEffect(owner) {
        val observer = LifecycleEventObserver { _, event ->
            when (event) {
                Lifecycle.Event.ON_STOP -> lock.didLeave(now())
                Lifecycle.Event.ON_START -> {
                    lock.didReturn(now())
                    locked = lock.isLocked
                    if (locked) attempt()
                }
                else -> {}
            }
        }
        owner.lifecycle.addObserver(observer)
        onDispose { owner.lifecycle.removeObserver(observer) }
    }
    LaunchedEffect(Unit) { if (lock.isLocked) attempt() }

    // Recents shows no terminal while locked.
    val activity = context as? android.app.Activity
    val covered = locked && on
    DisposableEffect(covered) {
        if (covered) activity?.window?.addFlags(android.view.WindowManager.LayoutParams.FLAG_SECURE)
        onDispose { if (covered) activity?.window?.clearFlags(android.view.WindowManager.LayoutParams.FLAG_SECURE) }
    }

    Box(Modifier.fillMaxSize()) {
        // Not composed at all while locked: a dialog, a bottom sheet or a focused terminal takes a window or the
        // keyboard of its own, which no Surface over the content would cover. What was asked to open waits in the
        // model and opens once this does.
        if (!covered) content()
        if (covered) {
            Surface(Modifier.fillMaxSize().testTag("app-lock"), color = MaterialTheme.colorScheme.background) {
                Column(
                    Modifier.padding(32.dp),
                    verticalArrangement = Arrangement.spacedBy(16.dp, Alignment.CenterVertically),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    Text("Far Cooler is locked", style = MaterialTheme.typography.titleLarge)
                    if (declined) {
                        Text(
                            "Your fingerprint, face or screen lock didn’t work. Try again.",
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            textAlign = TextAlign.Center,
                        )
                    }
                    Button(onClick = { attempt() }, modifier = Modifier.testTag("app-lock-unlock")) { Text(AppLock.UNLOCK_BUTTON) }
                }
            }
        }
    }
}
