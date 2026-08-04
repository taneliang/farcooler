package com.farcooler.account

import com.farcooler.BuildConfig

/**
 * What this build is, in the one form everything else quotes.
 *
 * Three audiences, one source. A person reading Settings wants `0.2.0 (beta 3)`.
 * The relay wants something it can compare across a fleet and show back as
 * "this Mac is two versions behind". A bug report wants both, unambiguously —
 * which is the whole reason the channel exists at all: a beta and a release
 * that call themselves `0.2.0` make a report impossible to act on.
 *
 * Read from `BuildConfig` rather than hard-coded, because `app/build.gradle.kts`
 * stamps it from `scripts/version.sh` at build time and a constant here would be
 * a second place to forget.
 */
object AppVersion {
    /** `0.2.0` — the semver, matching the daemon's and the CLI's. */
    val marketing: String get() = BuildConfig.MARKETING_VERSION

    /** The commit count. Monotonic, and it names the commit it came from. */
    val build: String get() = BuildConfig.VERSION_CODE.toString()

    /** `dev`, `beta`, or `release`. */
    val channel: String get() = BuildConfig.CHANNEL.ifEmpty { "dev" }

    val isRelease: Boolean get() = channel == "release"

    /** What a person is shown: `0.2.0`, `0.2.0 (beta 3)`, `0.2.0 (dev a1b2c3)`. */
    val display: String get() = BuildConfig.VERSION_NAME.ifEmpty { marketing }

    /**
     * What the relay is told, and what it shows back on the devices screen.
     *
     * One field rather than three, because the relay's job here is to let
     * someone glance at a list and see which machine is behind — not to hold a
     * version model of its own.
     */
    val reported: String get() = "$display · $build"
}
