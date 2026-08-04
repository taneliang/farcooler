package com.farcooler.net

import kotlinx.coroutines.CancellationException

/**
 * Cancellation is not a failure, and catching it is always a bug.
 *
 * Kotlin signals cancellation by throwing [CancellationException] out of the
 * suspend call that was waiting — and that class extends `IllegalStateException`,
 * so it is caught by `catch (e: Exception)` and by `runCatching`, both of which
 * look like ordinary defensive code. Two things go wrong when it is:
 *
 * 1. **It is reported as an error.** The terminal cancels its own poll every
 *    time a key is pressed, to restart it at the fast interval — so typing
 *    produced "Could not load: StandaloneCoroutine was cancelled" over the
 *    screen, flashing when the next poll won the race and sticking when it did
 *    not. That is the defect these helpers exist to make unrepresentable.
 * 2. **The coroutine keeps running.** Swallowing the exception means a job that
 *    was told to stop carries on to its next suspension point and is cancelled
 *    there instead, so teardown happens later than it was asked to and in a
 *    place nobody chose.
 *
 * The Apple apps do not have this bug and it is worth saying why, because the
 * reason is not that they are more careful: Swift's
 * `withCheckedThrowingContinuation` is not cancellation-aware, so a cancelled
 * `core.call` there simply never resolves and never throws. The hazard is
 * specific to this platform, which is exactly the sort of thing a straight port
 * carries over without noticing.
 */
fun Throwable.rethrowIfCancellation() {
    if (this is CancellationException) throw this
}

/**
 * [runCatching], minus the one throwable that must never be caught.
 *
 * Use this anywhere a failure is genuinely ignorable — a fire-and-forget call
 * to the host, a refresh that can wait for the next tick. Where the error is
 * shown to someone, catch it explicitly and call [rethrowIfCancellation] first,
 * so the message that reaches the screen is one the host actually sent.
 */
inline fun <T> attempt(block: () -> T): Result<T> = try {
    Result.success(block())
} catch (cancellation: CancellationException) {
    throw cancellation
} catch (e: Exception) {
    Result.failure(e)
}
