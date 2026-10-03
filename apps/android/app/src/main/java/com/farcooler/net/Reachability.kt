package com.farcooler.net

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkRequest
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch

/**
 * The moment when waiting out a backoff is the wrong thing to do.
 *
 * The Mac's `Reachability`, on a phone, minus the half that does not apply: a
 * laptop's lid is a wake notification, and a phone's equivalent is the process
 * being resumed, which the activity already reports (see `MainActivity`). What
 * is left is the network, and it matters more here than it does on a desk — a
 * phone changes networks by being carried through a door.
 *
 * Deliberately one callback rather than a listener per connection: a connection
 * does not need to know why now is a better moment than the one its timer
 * picked, only that it is.
 */
class Reachability(
    context: Context,
    owner: CoroutineScope,
    onShouldRetry: () -> Unit,
) {

    private val manager =
        context.applicationContext.getSystemService(ConnectivityManager::class.java)

    private val recovery = NetworkRecovery(owner, onShouldRetry)

    /**
     * Runs on ConnectivityManager's own thread, never the main one, so it only
     * reads the network and forwards; [NetworkRecovery] hops to the owner
     * before anything is decided or retried. `activeNetwork` is read here, at
     * the moment of the loss, rather than after the hop: by the time the main
     * thread gets to it, a second network may have come up and the loss would
     * read as nothing having happened.
     */
    private val callback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) = recovery.available()

        override fun onLost(network: Network) {
            // `activeNetwork` rather than a counter: a phone dropping Wi-Fi
            // while cell data carries on has lost a network and not the
            // network, and treating those alike would arm a recovery that
            // fires on the next handoff.
            recovery.lost(stillHasNetwork = manager?.activeNetwork != null)
        }
    }

    fun start() {
        recovery.started(hasNetwork = manager?.activeNetwork != null)
        manager?.registerNetworkCallback(NetworkRequest.Builder().build(), callback)
    }

    fun stop() {
        runCatching { manager?.unregisterNetworkCallback(callback) }
    }
}

/**
 * When the network coming back is worth a retry, decided on one thread.
 *
 * ConnectivityManager calls back on a thread of its own, and the retry walks
 * [FleetRepository]'s plain map of connections and writes each one's phase,
 * both of which the main thread also writes. So every event hops onto [owner]
 * (the view model's scope, which is the main thread) before it reads or writes
 * anything, and [hadNetwork] and the retry are confined to it. Apart from the
 * framework so a JVM test can drive it from other threads.
 */
class NetworkRecovery(
    private val owner: CoroutineScope,
    private val onShouldRetry: () -> Unit,
) {
    /**
     * Whether a network was available last time this was told anything.
     *
     * Android reports `onAvailable` for every network that appears, including
     * a second one alongside a working first — walking into Wi-Fi while cell
     * data is fine is not a recovery, and reconnecting every runner for it
     * would be a burst of SSH handshakes for nothing. Only the transition out
     * of having none counts. Read and written only on [owner].
     */
    private var hadNetwork = true

    /** What the network was when listening began. */
    fun started(hasNetwork: Boolean) {
        owner.launch { hadNetwork = hasNetwork }
    }

    /** A network appeared; a retry only if there was none before it. */
    fun available() {
        owner.launch {
            val was = hadNetwork
            hadNetwork = true
            if (!was) onShouldRetry()
        }
    }

    /** A network went away; [stillHasNetwork] is whether another carries on. */
    fun lost(stillHasNetwork: Boolean) {
        owner.launch { hadNetwork = stillHasNetwork }
    }
}
