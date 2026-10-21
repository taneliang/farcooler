package com.farcooler.ui

import android.app.UiModeManager
import android.content.Context
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.core.content.ContextCompat

/**
 * The system's contrast level (ov-624): 0 is standard, above 0 is the person
 * asking for more. `UiModeManager.getContrast()` and its change listener are
 * API 34; this app's `minSdk` is 37, so there is no older floor to read as 0.
 *
 * Read when the theme is first drawn and again whenever the setting changes
 * while the app runs, so flipping it in Settings repaints without a restart.
 */
@Composable
fun rememberSystemContrast(): Float {
    val context = LocalContext.current
    val manager = remember(context) { context.getSystemService(Context.UI_MODE_SERVICE) as? UiModeManager }
    var contrast by remember(manager) { mutableFloatStateOf(manager?.contrast ?: 0f) }
    DisposableEffect(manager) {
        val listener = UiModeManager.ContrastChangeListener { contrast = it }
        manager?.addContrastChangeListener(ContextCompat.getMainExecutor(context), listener)
        // The setting may have moved between the first read and the listener.
        manager?.let { contrast = it.contrast }
        onDispose { manager?.removeContrastChangeListener(listener) }
    }
    return contrast
}
