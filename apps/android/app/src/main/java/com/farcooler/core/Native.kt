package com.farcooler.core

/**
 * The Rust cores, as Kotlin sees them.
 *
 * Declarations only. Everything these call lives in `crates/android`, which is
 * itself only a translation layer over the same two C ABIs the Mac and iOS
 * apps use — so the SSH transport, the protocol, the emulator and the key
 * encoder are one implementation shared by three platforms rather than three
 * that agree today.
 *
 * Nothing in this file is safe to call from two threads at once: both cores
 * document that a handle is confined to one thread, and that contract is kept
 * by [ClientCore] and [VtCore], which own the handles. Nothing else should hold
 * one.
 */
internal object NativeLibrary {
    /**
     * Whether the tunnel loaded.
     *
     * The Go half, `libtailcat.so`, which is how this app reaches a runner
     * whose reach is Tailcat. It is loaded FIRST and on purpose:
     * `libfarcooler_jni.so` carries a `DT_NEEDED` on it, and while Android's
     * linker will usually resolve that out of the app's own library directory
     * unaided, naming it here makes the order certain and makes a failure to
     * find it say so on its own line rather than as a core that mysteriously
     * would not load.
     *
     * Recorded, never thrown, for the same reason [loaded] is.
     */
    val tunnelLoaded: Boolean = runCatching { System.loadLibrary("tailcat") }.isSuccess

    /**
     * Whether the shared object loaded.
     *
     * Recorded rather than thrown, because the honest failure here is a build
     * problem — an APK for an ABI nobody cross-compiled a core for — and an
     * `UnsatisfiedLinkError` escaping a static initialiser takes the process
     * down before any screen exists to say so. A flag lets the app start and
     * report it in words.
     */
    val loaded: Boolean = runCatching { System.loadLibrary("farcooler_jni") }.isSuccess
}

/**
 * Whether the Rust cores are available to this build at all.
 *
 * False only when the APK carries no `libfarcooler_jni.so` for this device's
 * ABI, which is a packaging mistake rather than a runtime condition — but it is
 * one worth being able to state in a diagnostics block rather than leaving to
 * be inferred from every screen failing at once.
 */
val coreIsAvailable: Boolean get() = NativeLibrary.loaded

/**
 * Whether the tunnel is available to this build.
 *
 * Separate from [coreIsAvailable] because the two fail for different reasons
 * and a diagnostics block that conflates them sends someone looking in the
 * wrong place: the core missing is an ABI nobody built, the tunnel missing is
 * an APK packaged without `libtailcat.so`. When this is false, a runner whose
 * reach is Tailcat cannot be connected to at all — there is no fallback path,
 * the reach is chosen once per runner where this app cannot see it — while
 * every Direct runner still works.
 */
val tunnelIsAvailable: Boolean get() = NativeLibrary.tunnelLoaded

internal object NativeClient {
    external fun nativeNew(): Long

    external fun nativeFree(handle: Long)

    external fun nativeConnect(handle: Long, config: String): Long

    external fun nativeCall(handle: Long, method: String, args: String): Long

    external fun nativePasteFile(
        handle: Long,
        terminal: String,
        name: String,
        mime: String,
        data: ByteArray,
    ): Long

    external fun nativePoll(handle: Long): String?

    external fun nativeConnected(handle: Long): Boolean

    external fun nativeStreamStart(handle: Long, terminal: String): Boolean

    external fun nativeStreamStop(handle: Long, terminal: String)

    external fun nativeGenerateKey(comment: String): String?

    external fun nativePublicKey(privateKey: String): String?

    /**
     * The SHA256 fingerprint of an OpenSSH public key, as a person reads it.
     *
     * Not computed here. Far Cooler implements no cryptography of its own, and
     * this particular string has to be the one a reply is addressed to —
     * `ceremony_reply` writes the same value into `target`, and a second way to
     * derive it is a second answer the day one of them changes.
     */
    external fun nativeFingerprint(publicKey: String): String?

    /**
     * The client id this device is enrolled under, derived from its own key.
     *
     * Not derived here, and that matters: the daemon's "already enrolled" check
     * compares client ids, so a format invented per platform means one device
     * enrols twice under two names and the daemon can no longer say which
     * session arrived on which key.
     */
    external fun nativeClientId(publicKey: String): String?

    // MARK: - The enrollment ceremony
    //
    // Four moments, and every rule about whether a scan is acceptable is behind
    // them in `crates/client/src/ceremony.rs`. Each answers the payload it was
    // asked for, or `{"error":"stale"}` — a stable word this app owns the
    // sentence for. Kotlin decides nothing.
    //
    // **These declarations are checked by nothing.** JNI binds by name and never
    // compares an argument list, so a shim whose Rust signature does not match
    // one of these is a crash at the first call rather than a build failure —
    // there is no compiler on either side of this line that can see both. Two of
    // them are newer than the shims that shipped in `156937d`:
    // `nativeCeremonyScan` takes the account, because the account rule moved into
    // `ceremony::accept_offer` where it belongs, and `nativeFingerprint` and
    // `nativeClientId` are new.
    // Both need `crates/android` rebuilt through
    // `scripts/build-android-libs.sh`; a stale `libfarcooler_jni.so` in
    // `app/src/main/jniLibs` will not do.

    /** The code a new device shows. [keyB] is null on a phone: no Zed, no key B. */
    external fun nativeCeremonyOffer(
        name: String,
        account: String,
        keyA: String,
        keyB: String?,
    ): String?

    /**
     * Read a code, on behalf of the account doing the reading.
     *
     * [expectingAccount] is passed so RUST refuses a stranger's device, with
     * `wrong_account`, rather than three apps each comparing two strings and
     * one of them forgetting to. [heldMs] is this device's own elapsed time,
     * which is the only clock that counts — a code carries no timestamp,
     * because the device showing it would control that number.
     */
    external fun nativeCeremonyScan(
        encoded: String,
        expectingAccount: String,
        heldMs: Long,
    ): String?

    /** The reply a trusted device shows back, capped by measured bytes. */
    external fun nativeCeremonyReply(
        offerJson: String,
        runnersJson: String,
        budgetBytes: Int,
    ): String?

    /** A scanned reply, taken or refused. */
    external fun nativeCeremonyAccept(
        encoded: String,
        expectingJson: String,
        alreadyTaken: Boolean,
        heldMs: Long,
    ): String?

    /** The themes compiled in, as JSON. No session needed. */
    external fun nativeBuiltinThemes(): String?
}

internal object NativeVt {
    external fun nativeNew(columns: Int, rows: Int): Long

    external fun nativeFree(handle: Long)

    external fun nativeFeed(handle: Long, bytes: ByteArray)

    external fun nativeResize(handle: Long, columns: Int, rows: Int)

    external fun nativeRevision(handle: Long): Long

    /** See `Java_com_farcooler_core_NativeVt_nativeSnapshot` for the layout. */
    external fun nativeSnapshot(handle: Long): IntArray?

    external fun nativeScroll(handle: Long, lines: Int)

    external fun nativeScrollToBottom(handle: Long)

    /**
     * Recolour the terminal. Nineteen packed 0xRRGGBB values: sixteen ANSI,
     * then foreground, background, cursor.
     */
    external fun nativeSetPalette(handle: Long, colors: IntArray): Boolean

    external fun nativeTakeWrites(handle: Long): ByteArray?

    external fun nativeTakeBell(handle: Long): Boolean

    external fun nativeTitle(handle: Long): String?

    /**
     * Text the program asked to put on the clipboard (OSC 52), or null.
     *
     * There is no read counterpart: a program asking for the clipboard's
     * contents is refused inside the core.
     */
    external fun nativeTakeClipboard(handle: Long): String?

    /**
     * The URL under a cell, or null. The core decides what counts as one and
     * which schemes may be opened — terminal output is not trusted input.
     */
    external fun nativeUrlAt(handle: Long, row: Int, column: Int): String?

    external fun nativeAltScreen(handle: Long): Boolean

    external fun nativeEncodeKey(handle: Long, key: Int, modifiers: Int): ByteArray?

    /** Null means the program does not want the event — handle it locally. */
    external fun nativeEncodeMouse(
        handle: Long,
        button: Int,
        action: Int,
        column: Int,
        row: Int,
        modifiers: Int,
    ): ByteArray?

    external fun nativeEncodePaste(handle: Long, text: String): ByteArray?
}

/**
 * Key codes, modifiers and mouse constants, mirroring `farcooler_vt.h`.
 *
 * A printable key is its own Unicode scalar value: pass through whatever the
 * platform's keyboard produced. Special keys live in the Unicode private use
 * area, which no keyboard layout can generate, so the two spaces cannot
 * collide.
 */
object Vt {
    const val KEY_ENTER = 0xE000
    const val KEY_TAB = 0xE001
    const val KEY_BACKSPACE = 0xE002
    const val KEY_ESCAPE = 0xE003
    const val KEY_UP = 0xE004
    const val KEY_DOWN = 0xE005
    const val KEY_RIGHT = 0xE006
    const val KEY_LEFT = 0xE007
    const val KEY_HOME = 0xE008
    const val KEY_END = 0xE009
    const val KEY_PAGE_UP = 0xE00A
    const val KEY_PAGE_DOWN = 0xE00B
    const val KEY_INSERT = 0xE00C
    const val KEY_DELETE = 0xE00D

    /** F1..F12 are `KEY_F1 + n - 1`. */
    const val KEY_F1 = 0xE010

    const val MOD_SHIFT = 1 shl 0
    const val MOD_ALT = 1 shl 1
    const val MOD_CTRL = 1 shl 2

    const val MOUSE_LEFT = 0
    const val MOUSE_MIDDLE = 1
    const val MOUSE_RIGHT = 2
    const val MOUSE_WHEEL_UP = 3
    const val MOUSE_WHEEL_DOWN = 4

    const val MOUSE_PRESS = 0
    const val MOUSE_RELEASE = 1
    const val MOUSE_MOVE = 2

    const val FLAG_BOLD = 1 shl 0
    const val FLAG_ITALIC = 1 shl 1
    const val FLAG_UNDERLINE = 1 shl 2
    const val FLAG_INVERSE = 1 shl 3

    /** A double-width character. The next column is its spacer; skip it. */
    const val FLAG_WIDE = 1 shl 4
}
