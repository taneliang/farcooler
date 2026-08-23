plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.kotlin.serialization)
}

// Firebase only when a project has actually been configured.
//
// `google-services.json` is per-project and carries a real Firebase project id,
// so it is not in the repository. The plugin fails the build outright when it
// is missing, which would mean nobody could build Far Cooler for Android
// without first creating a Firebase project — for a feature (a push reaching a
// sleeping phone) that is not needed to run the app at all.
//
// So the plugin is applied only when the file is there. Without it the Firebase
// SDK logs that it could not initialise, `PushRegistration` reports itself
// unavailable, and every local notification keeps working: exactly the shape
// the iOS app already has on the simulator.
val hasFirebase = file("google-services.json").exists()
if (hasFirebase) {
    apply(plugin = libs.plugins.google.services.get().pluginId)
}

/// The version of the whole system, from `scripts/version.sh`.
///
/// Read at configure time rather than hard-coded, for the reason the script
/// itself states: the CLI, the daemon, the Mac app, the phone apps and the
/// relay must agree, and a hand-stamped number is one somebody forgets on the
/// release that mattered. `build-app.sh` and `generate-project.py` stamp the
/// Apple targets from the same script.
fun versionScript(vararg args: String): String {
    val script = rootProject.file("../../scripts/version.sh")
    if (!script.exists()) return ""
    return try {
        providers.exec {
            commandLine(listOf(script.absolutePath) + args)
            workingDir = script.parentFile.parentFile
        }.standardOutput.asText.get().trim()
    } catch (e: Exception) {
        // A version that could not be read is a build problem, not a reason to
        // fail: a checkout without git history still has to compile.
        ""
    }
}

val marketingVersion = versionScript().ifEmpty { "0.0.0" }
val buildNumber = versionScript("build").toIntOrNull() ?: 1
val releaseChannel = versionScript("channel").ifEmpty { "local" }

/// The URL scheme AuthKit comes back to, asked for rather than derived here.
///
/// The stable-keeps-the-bare-name rule lives in `scripts/version.sh` alongside
/// the same rule for bundle identifiers, and a second copy in Kotlin is a
/// second copy to disagree. Falls back to the bare scheme only when the script
/// cannot be run at all, which is the same fallback every other value here has.
val authScheme = versionScript("scheme").ifEmpty { "farcooler" }
val displayVersion = versionScript("display").ifEmpty { marketingVersion }

// The terminal's typeface, staged rather than copied into this app.
//
// `apps/ios/FarCooler/Fonts` already has these two files in git, and they are
// 13 MB each. A second copy here would double that in history forever and give
// the two apps a way to drift on to different builds of the same font — the
// same argument `.gitignore` makes about the staged VT header, which is the
// single source of truth for the ABI.
//
// Renamed on the way in because Android resource names may only be lowercase
// with underscores, which is the only reason this is a copy task rather than an
// extra source directory.
val stageFonts by tasks.registering(Copy::class) {
    val fonts = rootProject.file("../ios/FarCooler/Fonts")
    from(fonts) {
        include("IosevkaNerdFontMono-Regular.ttf")
        rename { "iosevka_regular.ttf" }
    }
    from(fonts) {
        include("IosevkaNerdFontMono-Bold.ttf")
        rename { "iosevka_bold.ttf" }
    }
    into(file("src/main/res/font"))
}

android {
    namespace = "com.farcooler"
    compileSdk = 37

    defaultConfig {
        // One application id per channel, so all four install side by side and
        // none can see another's data. Stable keeps the bare id: it is what any
        // existing install already has, and changing it would orphan those
        // rather than update them.
        applicationId = when (releaseChannel) {
            "stable" -> "com.farcooler.android"
            else -> "com.farcooler.android.$releaseChannel"
        }

        // Android 17. Stated by the product, not inferred: Far Cooler's Android
        // client exists for a phone running the current OS, and every API below
        // this would buy compatibility with devices nobody is asking about at
        // the cost of shims on the paths that matter — predictive back, the
        // per-app language and notification permissions, and the 16 KB page
        // size the native cores are aligned for.
        minSdk = 37
        targetSdk = 37

        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"

        versionCode = buildNumber
        versionName = displayVersion

        // Read back by `AppVersion`, which reports it to the relay so the
        // devices screen can say which of your runners is behind.
        buildConfigField("String", "CHANNEL", "\"$releaseChannel\"")
        buildConfigField("String", "MARKETING_VERSION", "\"$marketingVersion\"")

        // Both, and they must agree: the manifest placeholder decides which
        // scheme this app REGISTERS with Android, and the BuildConfig field
        // decides which one it asks WorkOS to redirect to. One without the
        // other is a sign-in that leaves and never comes back.
        buildConfigField("String", "AUTH_SCHEME", "\"$authScheme\"")
        manifestPlaceholders["authScheme"] = authScheme

        // The AuthKit client id. Public by design — it names the app, not the
        // bearer — and overridable so a fork can point at its own WorkOS
        // project without editing source, matching the iOS Info.plist key.
        buildConfigField(
            "String",
            "WORKOS_CLIENT_ID",
            "\"${project.findProperty("farcooler.workosClientId") ?: ""}\"",
        )

        // Only what a core has actually been built for — DISCOVERED, not
        // listed.
        //
        // `build-android-libs.sh` builds arm64 always and x86_64 only with
        // `--emulator`, so which ABIs exist is a fact about what someone ran,
        // not something a build file can assert. A hard-coded list is worse
        // than none: it makes Gradle package `lib/x86_64/` — other AndroidX
        // libraries have slices for it — with no `libfarcooler_jni.so` beside
        // them, so the app installs on an emulator and dies at the first screen
        // with an `UnsatisfiedLinkError`.
        val builtAbis = file("src/main/jniLibs")
            .listFiles { file -> file.isDirectory && file.resolve("libfarcooler_jni.so").exists() }
            ?.map { it.name }
            ?.sorted()
            .orEmpty()
        if (builtAbis.isEmpty()) {
            logger.warn(
                "No Rust core found in app/src/main/jniLibs. " +
                    "Run ./scripts/build-android-libs.sh before building the app."
            )
        }
        ndk {
            abiFilters += builtAbis
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
        debug {
            applicationIdSuffix = ".debug"
        }

        // Instrumented tests get their own application id, and their own
        // install.
        //
        // AGP uninstalls the app under test when `connectedAndroidTest`
        // finishes. Run against `debug` that is the build someone is actually
        // using — and the uninstall takes its data with it, which here means
        // the runner list AND the device's SSH identity, since the Keystore
        // key goes when the app does. The phone then has a new key the runner
        // has never authorized, which looks exactly like a rejected connection.
        //
        // A separate id means the tests install, run and uninstall something
        // nobody was relying on:
        //
        //     ./gradlew connectedInstrumentedAndroidTest
        create("instrumented") {
            initWith(getByName("debug"))
            applicationIdSuffix = ".test"
            isDebuggable = true
        }
    }

    // Both test variants build against `instrumented`, not `debug`.
    //
    // AGP generates test variants for exactly one build type, so this renames
    // the unit-test task as well — `testInstrumentedUnitTest`, not
    // `testDebugUnitTest`. That is a fair price: what it buys is that a
    // connected run installs and uninstalls `…android.test` and never touches
    // the `…android.debug` someone is actually using. Which build type the JVM
    // unit tests compile against is immaterial — they are pure logic, and
    // `instrumented` is `initWith(debug)`.
    testBuildType = "instrumented"

    buildFeatures {
        compose = true
        buildConfig = true
    }

    packaging {
        jniLibs {
            // The `.so` stays a real file in the APK rather than being extracted
            // at install time. Required from Android 6 onwards for anything the
            // linker loads directly, and it is what lets the 16 KB page
            // alignment the NDK produces survive into the installed app — a
            // Pixel with 16 KB pages refuses to map a library that lost it.
            useLegacyPackaging = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

kotlin {
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
    }
}

// Every task that reads resources needs the fonts staged first.
//
// Matched by name rather than listed, because AGP has several that read the res
// directory — generate, merge, process, and the navigation and source-set-path
// passes beside them — and naming them individually means the build breaks the
// next time it grows one.
tasks.matching {
    it.name != "stageFonts" &&
        (it.name.contains("Resources") || it.name.contains("SourceSetPaths"))
}.configureEach { dependsOn(stageFonts) }

dependencies {
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.lifecycle.runtime.ktx)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    // Declared rather than taken transitively. `AppModel` names
    // `SavedStateHandle` in its constructor — it is where the navigation stack
    // lives across a process death — and a direct dependency that arrives only
    // because something else happens to pull it in is one an unrelated upgrade
    // can take away.
    //
    // It also carries the consumer ProGuard rule that keeps
    // `<init>(Application, SavedStateHandle)` on an `AndroidViewModel`. That
    // constructor is found REFLECTIVELY by the activity's default factory, so
    // without the rule the release build strips it and `by viewModels()`
    // crashes on a screen the debug build opens fine.
    implementation(libs.androidx.lifecycle.viewmodel.savedstate)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.browser)
    implementation(libs.kotlinx.serialization.json)

    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.ui.graphics)
    implementation(libs.androidx.compose.ui.tooling.preview)
    implementation(libs.androidx.compose.material3)
    implementation(libs.androidx.compose.material.icons.extended)
    debugImplementation(libs.androidx.compose.ui.tooling)

    implementation(platform(libs.firebase.bom))
    implementation(libs.firebase.messaging)

    // The enrollment ceremony needs a camera and a QR encoder, and neither is
    // free on this platform. Both halves are a decision:
    //
    // CameraX, because Android has no system QR scanner to hand a result back
    // from — unlike iOS, where `AVCaptureMetadataOutput` does the whole job.
    // Measured, not guessed: about 4.6 MB of AARs once Gradle resolves it, since
    // 1.6.x routes `camera-camera2` through `camera-camera2-pipe` (1.5 MB) and
    // `camera-view` brings `camera-video` and `viewfinder-core` with it. There is
    // no cheaper road to a preview that gets rotation and scaling right on every
    // OEM's camera2 implementation, and hand-rolling one over `SurfaceView` is
    // exactly where camera bugs live.
    //
    // ZXing's `core` for BOTH directions — 600 KB, no native code, no model file.
    // The plan named ML Kit for scanning, and ML Kit's BUNDLED barcode model is
    // another 3-4 MB of model and native code ON TOP of the same CameraX, plus
    // `play-services-basement`. What it buys is a decoder that is better in poor
    // light and at an angle, which is worth paying for when reading a crumpled
    // label and is not the case here: this code is drawn black-on-white on a lit
    // screen and held about 15 cm from the lens, at medium error correction for
    // exactly that reason. And ZXing has to be here regardless, because Android
    // has no `CIQRCodeGenerator` and the device being added has to DRAW the first
    // code. One library, both legs.
    //
    // Nothing here is a biometric library: `android.hardware.biometrics
    // .BiometricPrompt` is the platform's own, and `minSdk = 37` means the
    // back-compat `androidx.biometric` wrapper would be dependency weight for
    // API levels this app does not build for.
    implementation(libs.androidx.camera.camera2)
    implementation(libs.androidx.camera.lifecycle)
    implementation(libs.androidx.camera.view)
    implementation(libs.zxing.core)

    testImplementation(libs.junit)
    testImplementation(libs.kotlinx.coroutines.test)
    androidTestImplementation(libs.junit)
    androidTestImplementation(libs.androidx.test.ext.junit)
    androidTestImplementation(libs.androidx.test.runner)
}
