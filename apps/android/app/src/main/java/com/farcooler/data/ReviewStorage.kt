package com.farcooler.data

import android.content.Context
import android.content.SharedPreferences

/**
 * The small, non-secret strings a review leaves behind.
 *
 * Two things outlive the process and for the same reason: where the reader was
 * in a diff, and what they typed and have not sent. `ChangesStore` and
 * everything under it hangs off a [com.farcooler.net.Connection], which goes
 * when the runner list is edited and when the view model is cleared — and the
 * situation this review surface is built for is a dozen ninety-second windows
 * across an hour, on a phone Android is free to kill between any two of them.
 * Anything held only in memory is therefore held only until the next set.
 *
 * So a bookmark and an unsent note live in preferences, the same place
 * [RunnerStore] keeps the runners: none of it is secret, none of it is large,
 * and the one thing on that screen that IS secret — the diff itself — is
 * deliberately not written down anywhere. What is stored is a path, a sha, and
 * whatever the reader typed, per worktree.
 *
 * ## Why this is an interface at all
 *
 * Because the alternative is that none of it can be tested. [Settings] and
 * [RunnerStore] take a `Context` and reach `getSharedPreferences` directly,
 * which is right for them — they are read once at startup by an app that has a
 * `Context` — and it puts them out of reach of a JVM unit test, which this
 * module has no Robolectric to escape. The bookmark's round trip and the
 * outbox's persist-on-every-write are precisely the behaviors that must be
 * proven and that no emulator was available to prove by hand. iOS injects a
 * `UserDefaults` into `ReviewCommentQueue` for exactly this reason and says so;
 * this is that seam, spelled for a platform whose preferences API is not a
 * value type.
 *
 * Deliberately three methods over strings. Anything richer would be a second
 * serialization format sitting under the one the callers already use.
 */
interface ReviewStorage {
    fun read(key: String): String?
    fun write(key: String, value: String)
    fun remove(key: String)
}

/** The real one. */
class PreferenceReviewStorage(context: Context) : ReviewStorage {
    private val preferences: SharedPreferences =
        context.applicationContext.getSharedPreferences("farcooler.review", Context.MODE_PRIVATE)

    override fun read(key: String): String? = preferences.getString(key, null)

    /**
     * `apply`, not `commit`. Every write here is on whichever thread moved the
     * scroll, and the record is three short strings — blocking a frame on a
     * disk write to remember which file is at the top of the screen would be
     * paying a frame for a fact nobody reads until the next launch.
     */
    override fun write(key: String, value: String) {
        preferences.edit().putString(key, value).apply()
    }

    override fun remove(key: String) {
        preferences.edit().remove(key).apply()
    }
}

/** For tests, and for a store built before anything has a `Context`. */
class InMemoryReviewStorage(
    private val values: MutableMap<String, String> = mutableMapOf(),
) : ReviewStorage {
    /** How many times anything was written, which is what "persists on write" means. */
    var writes: Int = 0
        private set

    override fun read(key: String): String? = values[key]

    override fun write(key: String, value: String) {
        values[key] = value
        writes += 1
    }

    override fun remove(key: String) {
        values.remove(key)
    }
}
