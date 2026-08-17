package com.farcooler.net

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkRequest

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
class Reachability(context: Context, private val onShouldRetry: () -> Unit) {

    private val manager =
        context.applicationContext.getSystemService(ConnectivityManager::class.java)

    /**
     * Whether a network was available last time this was told anything.
     *
     * Android reports `onAvailable` for every network that appears, including
     * a second one alongside a working first — walking into Wi-Fi while cell
     * data is fine is not a recovery, and reconnecting every runner for it
     * would be a burst of SSH handshakes for nothing. Only the transition out
     * of having none counts.
     */
    private var hadNetwork = true

    private val callback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) {
            val was = hadNetwork
            hadNetwork = true
            if (!was) onShouldRetry()
        }

        override fun onLost(network: Network) {
            // `activeNetwork` rather than a counter: a phone dropping Wi-Fi
            // while cell data carries on has lost a network and not the
            // network, and treating those alike would arm a recovery that
            // fires on the next handoff.
            hadNetwork = manager?.activeNetwork != null
        }
    }

    fun start() {
        hadNetwork = manager?.activeNetwork != null
        manager?.registerNetworkCallback(NetworkRequest.Builder().build(), callback)
    }

    fun stop() {
        runCatching { manager?.unregisterNetworkCallback(callback) }
    }
}
