package com.farcooler.net

import java.util.concurrent.ConcurrentLinkedQueue
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicInteger
import kotlin.concurrent.thread
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.cancel
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The network's callbacks arrive on ConnectivityManager's thread, and the
 * retry they trigger walks the fleet's plain map of connections, which the
 * main thread also writes. These drive [NetworkRecovery] from other threads,
 * with a named single thread standing in for the main one, and check the retry
 * only ever runs there.
 */
class NetworkRecoveryTest {

    @Volatile private var ownerThread: Thread? = null
    private val ownerExecutor = Executors.newSingleThreadExecutor { r ->
        Thread(r, OWNER).also { ownerThread = it }
    }
    private val ownerDispatcher = ownerExecutor.asCoroutineDispatcher()
    private val owner = CoroutineScope(SupervisorJob() + ownerDispatcher)

    @After
    fun tearDown() {
        owner.cancel()
        ownerExecutor.shutdownNow()
    }

    /** Waits until everything already handed to the owner has run. */
    private fun drain() = runBlocking { withContext(ownerDispatcher) {} }

    @Test
    fun a_retry_runs_on_the_owner_and_never_on_the_callback_thread() {
        val wrongThreads = ConcurrentLinkedQueue<String>()
        val retries = AtomicInteger()
        // The fleet's map, as plain as the real one: the owner writes it, and
        // the retry iterates it. Off the owner, that iteration races the
        // writes below and throws ConcurrentModificationException, or reads a
        // half-written map.
        val connections = mutableMapOf<String, Int>()
        val failures = ConcurrentLinkedQueue<Throwable>()
        val recovery = NetworkRecovery(owner) {
            // By identity: coroutine debug mode appends to thread names.
            val here = Thread.currentThread()
            if (here !== ownerThread) wrongThreads += here.name
            try {
                connections.values.forEach { it + 1 }
            } catch (t: Throwable) {
                failures += t
            }
            retries.incrementAndGet()
        }
        recovery.started(hasNetwork = true)

        val go = CountDownLatch(1)
        val callbacks = (1..4).map { n ->
            thread(name = "ConnectivityThread-$n") {
                go.await()
                repeat(500) {
                    recovery.lost(stillHasNetwork = false)
                    recovery.available()
                }
            }
        }
        // The owner keeps reshaping the map while the callbacks fire.
        val churn = thread(name = "churn") {
            go.await()
            repeat(2_000) { i ->
                runBlocking(ownerDispatcher) {
                    connections["r$i"] = i
                    if (i % 3 == 0) connections.remove("r${i - 1}")
                }
            }
        }
        go.countDown()
        callbacks.forEach { it.join() }
        churn.join()
        drain()

        assertTrue("retried on ${wrongThreads.toSet()}, not on $OWNER", wrongThreads.isEmpty())
        assertTrue("the map was corrupted mid-iteration: ${failures.firstOrNull()}", failures.isEmpty())
        assertTrue("never retried at all", retries.get() > 0)
    }

    @Test
    fun only_the_way_out_of_having_no_network_retries() {
        val retries = AtomicInteger()
        val recovery = NetworkRecovery(owner) { retries.incrementAndGet() }
        recovery.started(hasNetwork = true)

        thread(name = "ConnectivityThread") {
            // A second network beside a working first is not a recovery.
            recovery.available()
            // Wi-Fi drops while cell data carries on: still no recovery.
            recovery.lost(stillHasNetwork = true)
            recovery.available()
            // The last network goes, then one comes back: exactly one retry.
            recovery.lost(stillHasNetwork = false)
            recovery.available()
            recovery.available()
        }.join()
        drain()

        assertEquals(1, retries.get())
    }

    private companion object {
        const val OWNER = "owner"
    }
}
