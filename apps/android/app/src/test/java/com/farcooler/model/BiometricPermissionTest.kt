package com.farcooler.model

import java.io.File
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The platform's `BiometricManager` and `BiometricPrompt` throw a `SecurityException` for an app that does not hold
 * `USE_BIOMETRIC`, and Robolectric never checks, so only this reads it (ov-864: the app lock asked at launch and the
 * canary crashed before its first screen).
 */
class BiometricPermissionTest {
    @Test
    fun theManifestDeclaresUseBiometricWhileTheAppAsksForOne() {
        val manifest = repositoryFile("apps/android/app/src/main/AndroidManifest.xml")
        val source = repositoryFile("apps/android/app/src/main/java/com/farcooler/ui/AppLockGate.kt")
        assertTrue("The app lock no longer uses BiometricManager; delete this test.", source.contains("BiometricManager"))
        assertTrue(
            "AppLockGate asks BiometricManager at launch, which throws without USE_BIOMETRIC in the manifest.",
            manifest.contains("android.permission.USE_BIOMETRIC"),
        )
    }

    private fun repositoryFile(relative: String): String {
        var directory: File? = File(System.getProperty("user.dir") ?: ".").absoluteFile
        while (directory != null) {
            val candidate = File(directory, relative)
            if (candidate.isFile) return candidate.readText()
            directory = directory.parentFile
        }
        throw AssertionError("Could not find $relative above ${System.getProperty("user.dir")}")
    }
}
