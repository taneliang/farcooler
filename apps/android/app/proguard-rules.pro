# What R8 may not take away from a release build.
#
# This file exists because `build.gradle.kts` has named it since the Android app
# was created and it was never written, so `assembleRelease` has never once
# succeeded — R8 fails the build outright on a configuration file that is not
# there. The debug build never reads it, which is why nothing noticed.
#
# It is deliberately short. Most of what a client like this needs is already
# shipped BY the libraries that need it, as consumer rules R8 merges in
# automatically, and a rule copied from a template is a rule nobody can later
# tell from a rule that is load-bearing:
#
#   * kotlinx-serialization-core carries `kotlinx-serialization-common.pro`,
#     which keeps every `Companion`, every `serializer()` and the generated
#     `$$serializer` descriptors, and disables the one optimization that is
#     known to miscompile them (Kotlin/kotlinx.serialization#2719).
#   * lifecycle-viewmodel-savedstate carries the rule that keeps
#     `<init>(Application, SavedStateHandle)`, which the activity's default
#     factory finds reflectively — see the note on the dependency itself.
#   * Firebase, CameraX and Compose each carry their own.
#   * Classes named in `AndroidManifest.xml` — `FarCoolerApp`, `MainActivity`,
#     `FarCoolerMessagingService` — are kept by a rule AGP generates from the
#     merged manifest, so they are not listed here either.
#
# So what follows is only the part no library can know about: this app's own
# boundary with the Rust core.

# The JNI boundary, by name, because the linker has nothing but the name.
#
# `System.loadLibrary("farcooler_jni")` binds `NativeClient.nativeNew` to the
# symbol `Java_com_farcooler_core_NativeClient_nativeNew` in `crates/android`,
# and that symbol is spelled out of the PACKAGE, the CLASS and the METHOD name.
# Rename any of the three on this side and the Rust side is unreachable: the
# APK still builds, still installs, and dies at the first screen with an
# `UnsatisfiedLinkError` — the same failure the `abiFilters` comment in
# `build.gradle.kts` describes arriving from the other direction.
#
# `proguard-android-optimize.txt` does already carry a global
# `-keepclasseswithmembernames class * { native <methods>; }` that would cover
# these two. Naming them anyway is four lines to make the app's entire transport
# — SSH, the protocol, the terminal emulator — stop depending on a wildcard in a
# file this project does not own and does not pin.
#
# Both are `internal object`s, so `-keepclasseswithmembernames` rather than
# `-keep`: R8 may still discard either one if nothing calls it, which is the
# honest state to be in, but may not rename what it keeps. Every signature here
# is primitives, `String`, `ByteArray` and `IntArray`, so there are no parameter
# types to pin alongside them.
-keepclasseswithmembernames class com.farcooler.core.NativeClient {
    native <methods>;
}
-keepclasseswithmembernames class com.farcooler.core.NativeVt {
    native <methods>;
}

# Recorded, because the DEX will not match this file and that is correct.
#
# `-keepclasseswithmembernames` pins the NAMES of what survives; it does not
# make anything survive. Five of the thirty-six bindings declared in `Native.kt`
# are reachable from no Kotlin at all, so R8 drops them and a release APK has
# thirty-one:
#
#   NativeVt.nativeRevision, nativeTakeBell, nativeAltScreen
#       Declared and never called from anywhere in the app.
#   NativeClient.nativeConnected, NativeVt.nativeTitle
#       Called, but only by `ClientCore.isConnected()` and `VtCore.title()`,
#       which nothing calls in turn — so the whole chain is dead.
#
# All five are live exports on the Rust side (`crates/android/src/lib.rs`), so
# this is the Kotlin binding layer having drifted ahead of its callers rather
# than a rule that is too weak. Left alone deliberately: they are the cheap half
# of a two-sided ABI, and deleting them makes the next screen that wants a
# window title re-derive it. What is worth knowing is that a native method
# MISSING from a release DEX is not by itself evidence of a broken keep rule —
# check whether Kotlin calls it before changing anything here.
