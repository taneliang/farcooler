package com.farcooler.net

import android.util.Base64
import com.farcooler.core.ClientCore
import com.farcooler.core.TerminalGrid
import com.farcooler.core.Vt
import com.farcooler.core.VtCore
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.cancel
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import java.util.concurrent.Executors
import kotlin.math.abs

/**
 * One terminal's screen, kept live while a screen is showing it.
 *
 * Streams, and polls only when it cannot stream. The two are genuinely
 * different pictures of the same pane, not two speeds of the same one: a stream
 * carries the bytes tmux wrote, in order, so the emulator here sees exactly
 * what one on the host would — cursor motion, partial redraws, a spinner
 * actually spinning. A poll carries the screen after it settled, which is the
 * same thing a photograph is to a film. Everything that made a remote terminal
 * feel remote came from the second one, and the fix was not to photograph
 * faster.
 *
 * The polling path is kept, and is not dead code: `startStream` answers false
 * when there is no SSH session to open a second channel on, and a screen that
 * updates once a second is enormously better than one that says it cannot be
 * shown.
 *
 * This never decides whether a terminal is "live" — the host does, the same way
 * [Connection] never computes a workspace's state. An unreadable screen is
 * reported exactly as the host phrased it.
 *
 * ## Where the work happens
 *
 * The emulator is confined to its own thread rather than the main one, unlike
 * the Apple apps' main-actor confinement. A reattach replays a whole screen —
 * on a busy agent that is tens of kilobytes of escape sequences — and parsing
 * that on the thread Compose draws from is a visible stutter at exactly the
 * moment someone is watching. Grids are immutable once built, so publishing one
 * across threads costs nothing.
 */
class TerminalSession(
    terminalId: String,
    private val core: ClientCore,
) {
    sealed interface Phase {
        data object Connecting : Phase
        data object NotLive : Phase
        /**
         * A sentence this app wrote, and — where it has none of its own — the
         * host's answer to put under it. See [humanFailure].
         */
        data class Failed(val message: String, val transcript: String? = null) : Phase
        data object Live : Phase
    }

    private val emulatorThread =
        Executors.newSingleThreadExecutor { runnable ->
            Thread(runnable, "farcooler-vt").apply { isDaemon = true }
        }.asCoroutineDispatcher()

    /**
     * This session's own scope, not a child of the screen's.
     *
     * A composable's scope is cancelled when it leaves composition, and the
     * order in which that happens relative to `onDispose` is not specified — so
     * a scope inherited from the screen could already be dead by the time
     * [dispose] tried to use it to hand the pane back its shape, close the SSH
     * channel and free the emulator. Owning the scope means teardown is this
     * object's to complete, and [dispose] is the one thing that ends it.
     */
    private val scope = CoroutineScope(SupervisorJob() + emulatorThread)

    private val _phase = MutableStateFlow<Phase>(Phase.Connecting)
    val phase: StateFlow<Phase> = _phase.asStateFlow()

    private val _grid = MutableStateFlow<TerminalGrid?>(null)
    val grid: StateFlow<TerminalGrid?> = _grid.asStateFlow()

    /**
     * Text a program asked to put on the clipboard (OSC 52).
     *
     * A flow of events rather than a state, because a copy is something that
     * HAPPENED: two identical copies in a row are two copies, and a StateFlow
     * would collapse them into one. The screen owns the clipboard — this layer
     * has no Context — so it collects these and writes them.
     *
     * `extraBufferCapacity` so an emit from the emulator thread never suspends
     * waiting for the UI to collect; `DROP_OLDEST` because if several copies
     * queue up behind a stalled collector, the newest is the one the user meant.
     */
    private val _copied =
        MutableSharedFlow<String>(
            extraBufferCapacity = 8, onBufferOverflow = BufferOverflow.DROP_OLDEST)
    val copied: SharedFlow<String> = _copied.asSharedFlow()

    private var terminalId: String = terminalId
    private var vt: VtCore? = null
    private var poller: Job? = null

    /**
     * The size this screen would like the pane to be, kept even while nothing
     * is happening about it so a later trigger — the app returning to the
     * foreground, say — has something to re-assert.
     */
    private var lastRequestedSize: Pair<Int, Int>? = null

    /**
     * The size the host was last actually asked to become. Doubles as "the
     * pane's size as far as this screen currently knows it", since [prime]
     * seeds it from the host's own answer — which is what lets a resize request
     * be skipped entirely when the pane already happens to be the right shape.
     */
    private var lastResizeSent: Pair<Int, Int>? = null

    /**
     * The size a pending debounce is going to ask for, distinct from
     * [lastResizeSent]. Without this, a `configure` that keeps re-arriving with
     * the same unchanged size — sub-pixel jitter during layout is enough —
     * would cancel and restart the debounce forever and the resize would never
     * actually fire.
     */
    private var pendingResizeSize: Pair<Int, Int>? = null

    /** What the pane looked like before this device reshaped it. */
    private var shapeBeforeUs: Pair<Int, Int>? = null

    /**
     * Whether this device has asked the host to reshape the pane yet.
     *
     * Gating [shapeBeforeUs] on this rather than on being the first [prime],
     * because the two race: the debounced resize is 200 ms and a screen call is
     * a round trip, so a `prime` can easily land after the resize and record
     * the shape THIS device just imposed as the one to hand back.
     */
    private var hasResized = false
    private var resizeDebounce: Job? = null

    /** Whether reshaping the shared pane is allowed at all. See [Settings]. */
    @Volatile
    var reshapeAllowed: Boolean = true

    /** The last screen decoded, so the next poll can tell whether anything changed. */
    private var lastScreen: ScreenResponse? = null

    private var streaming = false
    private val inbox = Inbox()
    private var geometry: Job? = null

    /** The pane's size as of the last time anything looked. */
    private var paneSize: Pair<Int, Int>? = null
    private var revision: ULong = 0uL

    /** Consecutive stream attaches that produced nothing before ending. */
    private var failedAttaches = 0
    private var started = false

    private var interval = FAST_INTERVAL_MS

    private val json = Json { ignoreUnknownKeys = true }

    /**
     * Show this terminal, if nothing has yet.
     *
     * Separate from [configure] because the two answer different questions and
     * the first has to be answerable without the second: `configure` is driven
     * by the canvas reporting its size, and the canvas is only composed once
     * there is a screen to draw — so waiting for it to open the session would
     * be waiting for a screen that opening the session is what produces.
     */
    fun start() {
        scope.launch {
            if (started) return@launch
            started = true
            open()
        }
    }

    /**
     * Learn the size this screen would like the pane to be, open the terminal
     * on the first call, and — after the debounce — ask the host to reflow.
     */
    fun configure(columns: Int, rows: Int) {
        if (columns <= 0 || rows <= 0) return
        scope.launch {
            lastRequestedSize = columns to rows
            if (!started) {
                started = true
                open()
            }
            scheduleResize()
        }
    }

    /**
     * Re-assert the size this screen already wants, without anything having
     * told it a new one.
     *
     * For triggers where the risk is not that this screen's own size changed
     * but that the pane drifted while nothing here was watching it — chiefly
     * the app returning to the foreground, where someone on a Mac could have
     * resized the shared window while this device was backgrounded.
     */
    fun reassertSize() {
        scope.launch { scheduleResize() }
    }

    /**
     * Point this same session at a different terminal — what the tab strip
     * calls when you tap a sibling.
     *
     * Everything describing the outgoing terminal goes at once — its emulator,
     * its screen, its stream — because every one of them would otherwise be
     * read as belonging to the incoming one: a stale grid makes the tap look
     * like it did nothing, and a stale [lastScreen] makes the new terminal's
     * first capture compare equal by coincidence.
     */
    fun switchTo(id: String) {
        scope.launch {
            if (id == terminalId) return@launch
            teardown()
            val leaving = terminalId
            val handBack = shapeBeforeUs
            shapeBeforeUs = null
            core.stopStream(leaving)
            releasePane(leaving, handBack)

            terminalId = id
            vt?.free()
            vt = null
            _grid.value = null
            lastScreen = null
            revision = 0uL
            paneSize = null
            _phase.value = Phase.Connecting
            started = true
            // The strike count belongs to the terminal being left, not the one
            // arriving. Carried over, a terminal whose predecessor had already
            // given up on streaming reached the three-strike fallback on its
            // first hiccup and showed a failure it had not earned.
            failedAttaches = 0
            hasResized = false
            // The size this device would like has not changed — the screen did
            // not resize, only what it is showing did — but `lastResizeSent`
            // described the OUTGOING terminal's pane.
            lastResizeSent = null
            open()
            scheduleResize()
        }
    }

    /**
     * The link under this session was replaced by a new one.
     *
     * Everything the old link set up — the stream, the emulator's idea of
     * where the cursor is, the last screen a poll compared against — belonged
     * to an SSH session that no longer exists, so it is dropped rather than
     * reused. Without this the screen still recovers, because [streamEnded]
     * falls back to polling, but it stays on the slower path until the screen
     * is rebuilt: streaming is one round trip and polling is an interval.
     *
     * [switchTo] minus the two things that only make sense when the terminal
     * itself changes. The id is the same, and the outgoing pane is not handed
     * its old size back — that pane is on the far side of a link that is gone,
     * so asking it anything is a request into the void.
     */
    fun relink() {
        scope.launch {
            if (!started) return@launch
            teardown()
            vt?.free()
            vt = null
            _grid.value = null
            lastScreen = null
            revision = 0uL
            paneSize = null
            _phase.value = Phase.Connecting
            failedAttaches = 0
            hasResized = false
            lastResizeSent = null
            open()
            scheduleResize()
        }
    }

    /**
     * Stop watching. A screen nobody is looking at has no business holding an
     * SSH channel open, or spending this phone's battery.
     */
    fun stop() {
        scope.launch {
            teardown()
            started = false
            val id = terminalId
            val handBack = shapeBeforeUs
            shapeBeforeUs = null
            core.stopStream(id)
            releasePane(id, handBack)
        }
    }

    /** Release everything. Called when the screen is gone for good. */
    fun dispose() {
        stop()
        scope.launch {
            vt?.free()
            vt = null
        }.invokeOnCompletion {
            scope.cancel()
            emulatorThread.close()
        }
    }

    // MARK: - Sizing

    /**
     * Debounce and dedupe a request to reflow the pane.
     *
     * Every trigger that thinks the pane might need refitting — the screen
     * appearing, a rotation, the keyboard showing, the tab strip switching
     * terminals, the app returning to the foreground — funnels through here
     * rather than calling the host directly, for two reasons. First, several of
     * those fire in a burst: a keyboard animation alone produces a size change
     * on close to every frame while it slides. Second, most calls ask for a
     * size the pane is already at, and asking again would be a round trip that
     * reflows a pane other people may be looking at for no visible change.
     */
    private fun scheduleResize() {
        if (!reshapeAllowed) return
        val size = lastRequestedSize ?: return
        if (lastResizeSent == size) return
        if (pendingResizeSize == size) return
        pendingResizeSize = size
        resizeDebounce?.cancel()
        resizeDebounce = scope.launch {
            delay(RESIZE_DEBOUNCE_MS)
            resizePane()
        }
    }

    /**
     * Actually ask the host to reflow the pane.
     *
     * A tmux pane is shared: resizing it reflows it for every other client
     * attached to that window, not just for this phone. Doing this
     * automatically is a deliberate trade — unreadably tiny text on a pane
     * sized for a laptop was judged worse than occasionally reflowing someone
     * else's terminal — and it is a trade the user can decline in settings.
     */
    private suspend fun resizePane() {
        val size = lastRequestedSize ?: return
        if (lastResizeSent == size) return
        pendingResizeSize = null
        lastResizeSent = size
        hasResized = true
        attempt {
            core.call(
                "terminal.resize",
                Connection.args("terminal" to terminalId, "columns" to size.first, "rows" to size.second),
            )
        }
        // The resize itself changes the screen, but not in a way this terminal
        // has captured yet — without clearing it, a resize to exactly the
        // content already on screen would compare equal and the redraw would
        // wait for the next backed-off tick.
        lastScreen = null
        // Reopened rather than waiting for the geometry check to notice what
        // this method just did: the emulator is still the old size, and the
        // redraw tmux is sending right now would be wrapped to it.
        if (streaming) open() else wake()
    }

    /**
     * Give a pane back the shape it had before this device reshaped it.
     *
     * A phone reflows a pane to its own narrow viewport, which is right while
     * it is the thing looking at it and wrong the moment it is not: whoever
     * else has that terminal on screen is left with a column of phone-shaped
     * text.
     */
    private suspend fun releasePane(id: String, shape: Pair<Int, Int>?) {
        val target = shape ?: return
        attempt {
            core.call(
                "terminal.resize",
                Connection.args("terminal" to id, "columns" to target.first, "rows" to target.second),
            )
        }
    }

    // MARK: - Opening

    /**
     * Show this terminal and keep it live, by whichever means the connection
     * supports.
     *
     * The screen call first is not for something to look at: the stream carries
     * no geometry, and an emulator has to be built at some size before a single
     * byte can be fed to it. It is also the one call that reports a pane that is
     * not running, which a stream can only express by failing to open.
     *
     * Which is why it does not paint when a stream is coming. A capture and a
     * stream disagree about the same screen — tmux hands out captures with bare
     * line feeds, so [render] has to repair them, and even repaired they are a
     * screen re-flowed rather than the bytes that drew it. Painting one and
     * then being repainted by the stream's own replay a moment later is
     * visible. A spinner for that moment is honest; a wrong screen is not.
     */
    private suspend fun open() {
        teardown()
        // Awaited: stop and start go through the same core in the order they
        // are asked, and a stop still in flight would arrive after the start
        // below and cancel the stream this call just opened — leaving a session
        // that believes it is streaming attached to nothing, which is a screen
        // that stops updating and never says why.
        core.stopStream(terminalId)
        val screen = prime() ?: return
        if (attach()) {
            watchGeometry()
            return
        }
        render(screen)
        startLoop()
    }

    /**
     * One screen: the pane's size, whether it is running at all, and — only if
     * nothing better is coming — something to draw.
     */
    private suspend fun prime(): ScreenResponse? = try {
        val data = core.call("terminal.screen", Connection.args("terminal" to terminalId))
        val response = json.decodeFromJsonElement(ScreenResponse.serializer(), data)
        revision = response.revision
        paneSize = response.columns to response.rows
        // The pane's actual shape, straight from the host, seeds the baseline
        // `scheduleResize` compares against — so a `configure` that asks for
        // the size the pane already happens to be does not spend a round trip
        // saying so.
        lastResizeSent = response.columns to response.rows
        if (shapeBeforeUs == null && !hasResized) {
            shapeBeforeUs = response.columns to response.rows
        }
        // An emulator at the pane's size, empty. Built here rather than by
        // whoever paints first, because the streaming path never paints a
        // capture and still has to have something to feed: the bytes start
        // arriving the moment the channel opens, and a chunk with no emulator
        // to receive it is simply lost.
        vt?.free()
        val emulator = VtCore(response.columns, response.rows)
        // A fresh core starts on the VT crate's own default palette, not the
        // theme in force. Without this the chrome would be themed and every
        // character would not.
        emulator.setPalette(com.farcooler.data.Themes.current.packed())
        applyModes(response.modes, emulator)
        vt = emulator
        response
    } catch (e: Exception) {
        // A cancelled prime is a screen nobody is waiting for any more — the
        // tab strip moved on, or a resize reopened this session. Reporting it
        // would paint a failure over the pane that replaced it.
        e.rethrowIfCancellation()
        report(e)
        null
    }

    /** Open the byte stream, and say whether it opened. */
    private suspend fun attach(): Boolean {
        inbox.clear()
        // Recorded BEFORE the channel is asked for, not after.
        //
        // `startStream` suspends, and everything on this session shares one
        // thread — so while it is suspended, a stream end the core has already
        // queued can run. `streamEnded` returns early when this flag is still
        // false, which is right for an end that arrives after a deliberate
        // stop and catastrophic here: the failure is swallowed, nothing
        // retries, nothing falls back to polling, and the screen shows its
        // spinner for ever with no way to find out why.
        streaming = true
        val opened = core.startStream(
            terminalId,
            onChunk = { bytes ->
                // Buffered here, on whatever thread the core drained on, and
                // consumed on the emulator's own. Feeding directly from a hop
                // per chunk would put the emulator's input at the mercy of the
                // order tasks happen to run in, and bytes that arrive out of
                // order are not a slightly wrong screen — they are an escape
                // sequence cut in half.
                inbox.append(bytes)
                scope.launch { consume() }
            },
            onEnd = { error -> scope.launch { streamEnded(error) } },
        )
        if (!opened) streaming = false
        return opened
    }

    /** Feed everything that has arrived, in order, and redraw once. */
    private suspend fun consume() {
        val emulator = vt ?: return
        val bytes = inbox.take()
        if (bytes.isEmpty()) return
        failedAttaches = 0
        emulator.feed(bytes)
        // Whatever the program asked to be written back — a cursor-position
        // report, a mouse reply — goes home, or a full-screen agent sits
        // waiting for an answer that is stuck in this buffer.
        val replies = emulator.takeWrites()
        if (replies.isNotEmpty()) write(replies)
        publish()
    }

    /**
     * The stream stopped. Try again, then settle for polling.
     *
     * A stream ends for two very different reasons: the pane finished, or the
     * channel did. Only the host can tell those apart, so this asks it — by
     * reopening, which begins with the screen call that reports a pane that is
     * no longer running. A channel that drops repeatedly without ever
     * delivering a byte is a connection that cannot carry a stream, and
     * retrying it forever would be a worse screen than the polling that
     * definitely works.
     */
    private suspend fun streamEnded(error: String?) {
        if (!streaming) return
        streaming = false
        failedAttaches += 1
        if (failedAttaches >= MAX_ATTACH_ATTEMPTS) {
            if (error != null) _phase.value = humanFailure(error)
            // Everything the streaming path set up goes before the polling path
            // starts. Falling back used to start the poll loop and leave the
            // rest running, so a stream that was still delivering fed the
            // emulator correct bytes while the poll repainted a capture over
            // the top of them — two painters, disagreeing, one of them every
            // second. A fallback has to be a handover, not an addition.
            teardown()
            startLoop()
            return
        }
        delay(STREAM_RETRY_MS)
        open()
    }

    // MARK: - Geometry

    private fun watchGeometry() {
        geometry?.cancel()
        geometry = scope.launch {
            while (isActive) {
                delay(GEOMETRY_INTERVAL_MS)
                checkGeometry()
            }
        }
    }

    /**
     * Notice someone else resizing this pane, and rebuild if they did.
     *
     * The stream carries content and says nothing about geometry, which is
     * correct — it is a byte stream, and inventing a frame format to carry a
     * column count would make it something else. But the pane belongs to
     * whoever else is looking at it, and someone splitting a window on a Mac
     * resizes it out from under this screen. Because the ask carries the
     * revision this device already holds, the usual answer is a hundred bytes
     * saying "still 80×24, still unchanged".
     */
    private suspend fun checkGeometry() {
        if (!streaming) return
        val size = paneSize ?: return
        val data = attempt {
            core.call(
                "terminal.screen",
                Connection.args("terminal" to terminalId, "knownRevision" to revision),
            )
        }.getOrNull() ?: return
        val response = attempt {
            json.decodeFromJsonElement(ScreenResponse.serializer(), data)
        }.getOrNull() ?: return

        revision = response.revision
        val now = response.columns to response.rows
        if (now == size) return
        paneSize = now

        // A pane that is the shape THIS device asked for is not news, and
        // reopening for it was a loop: the resize lands, the next check sees a
        // size that no longer matches the one `prime` recorded, reopens, and
        // the reopen's own resize starts it again. Nine SSH channels for one
        // terminal had accumulated by the time it was noticed on iOS, and every
        // reopen left a window in which the emulator was fresh — so it did not
        // yet know the pane wanted the mouse, and scrolling silently did
        // nothing until the replay arrived.
        if (lastResizeSent == now) {
            vt?.resize(now.first, now.second)
            publish()
            return
        }

        // Somebody else reshaped it. That needs the replay, because only the
        // host can re-wrap a screen it wrapped at a different width.
        open()
    }

    // MARK: - Polling fallback

    /**
     * Cancel and restart the loop at the fast interval — "poll right now, then
     * resume at full speed" — without ending up with two loops running.
     */
    private fun wake() {
        interval = FAST_INTERVAL_MS
        poller?.cancel()
        startLoop()
    }

    private fun startLoop() {
        poller = scope.launch {
            while (isActive) {
                poll()
                delay(interval)
            }
        }
    }

    private suspend fun poll() {
        try {
            val data = core.call("terminal.screen", Connection.args("terminal" to terminalId))
            val response = json.decodeFromJsonElement(ScreenResponse.serializer(), data)

            // The cheap compare that makes backing off free: `ScreenResponse`
            // carries the still-base64 payload, so this is a comparison of
            // exactly what the host sent, before any of the more expensive work
            // — decoding, feeding a fresh emulator, copying a snapshot — runs
            // on a screen that has not moved. Most polls of an idle terminal
            // end right here.
            if (response == lastScreen) {
                interval = minOf((interval * BACKOFF).toLong(), SLOW_INTERVAL_MS)
                return
            }
            lastScreen = response
            interval = FAST_INTERVAL_MS
            render(response)
        } catch (e: Exception) {
            // Every keystroke cancels this poll on purpose — `write` calls
            // `wake`, which restarts the loop at the fast interval. Treating
            // that as a host failure is what put "Could not load" over the
            // terminal on every key pressed.
            e.rethrowIfCancellation()
            report(e)
            // Back off on a persistent error too — a terminal that keeps
            // failing to load has no "changing" to detect, and retrying it
            // every 100 ms would hammer a host that has already said no.
            interval = minOf((interval * BACKOFF).toLong(), SLOW_INTERVAL_MS)
        }
    }

    // MARK: - Drawing

    /**
     * Tell a fresh emulator what the program has already asked for.
     *
     * Without this the emulator is built believing the program wants no mouse,
     * is not on the alternate screen and sends ordinary arrow keys — wrong for
     * every full-screen program there is. The stream's replay says the same
     * thing, and depending on it was the bug: a stream that ends and reattaches
     * rebuilds the emulator, and the replay carrying the modes had gone into
     * the one that was discarded. What survived declined every wheel event, so
     * scrolling silently stopped working — and stayed stopped.
     */
    private fun applyModes(modes: String?, emulator: VtCore) {
        if (modes.isNullOrEmpty()) return
        emulator.feed(modes.toByteArray(Charsets.UTF_8))
    }

    /**
     * Build an emulator from a captured screen and show it.
     *
     * A fresh emulator per capture rather than feeding into the last one: a
     * capture is a whole screen, not a continuation of one, and a persistent
     * core would need the dump to be self-clearing to stay correct. "Assume
     * every capture starts by wiping the grid" is a fact about the host's tmux
     * version, not something this file should have to trust.
     */
    private fun render(response: ScreenResponse) {
        revision = response.revision
        val bytes = runCatching { Base64.decode(response.contents, Base64.DEFAULT) }.getOrNull()
        if (bytes == null) {
            _phase.value = Phase.Failed("The host sent a screen this device could not decode.")
            return
        }
        vt?.free()
        val emulator = VtCore(response.columns, response.rows)
        // A fresh core starts on the VT crate's own default palette, not the
        // theme in force. Without this the chrome would be themed and every
        // character would not.
        emulator.setPalette(com.farcooler.data.Themes.current.packed())
        applyModes(response.modes, emulator)
        // A capture separates its lines with a bare line feed, which to a
        // terminal means "down one row" and nothing about which column. Fed
        // straight in, every line starts where the previous one ended and the
        // screen arrives as a staircase — text broken mid-word at a different
        // place on each row. The daemon repairs this for the replay it sends
        // down a stream, for exactly the same reason; a capture arriving by any
        // other route needs the same repair. Live pty bytes never do.
        emulator.feed(carriageReturned(withoutTrailingNewlines(bytes)))
        vt = emulator
        paneSize = response.columns to response.rows
        // The host's cursor, not the emulator's: a capture is text, so feeding
        // it leaves the caret wherever the last character landed — the bottom
        // left — rather than at the prompt someone is typing into.
        _grid.value = emulator.snapshot()?.withCursor(response.cursorRow, response.cursorColumn)
        _phase.value = Phase.Live
    }

    /**
     * Drop the newline a captured screen ends with.
     *
     * A capture is as many lines as the screen is tall, so feeding the last
     * one's line feed moves the caret off the bottom row and scrolls the whole
     * screen up by one: the top line goes into history and every remaining row
     * is drawn one higher than it belongs. The caret then comes from the host,
     * which knows nothing about that scroll, so it lands a row below the text
     * it should be sitting in.
     */
    private fun withoutTrailingNewlines(bytes: ByteArray): ByteArray {
        var end = bytes.size
        while (end > 0 && (bytes[end - 1] == 0x0A.toByte() || bytes[end - 1] == 0x0D.toByte())) {
            end -= 1
        }
        return bytes.copyOf(end)
    }

    /**
     * Give every bare line feed the carriage return a captured screen left out.
     * One that already has one is left alone, so this is safe to apply to
     * anything.
     */
    private fun carriageReturned(bytes: ByteArray): ByteArray {
        val out = ArrayList<Byte>(bytes.size + bytes.size / 40)
        var previous: Byte = 0
        for (byte in bytes) {
            if (byte == 0x0A.toByte() && previous != 0x0D.toByte()) out.add(0x0D)
            out.add(byte)
            previous = byte
        }
        return out.toByteArray()
    }

    /** Show what the emulator currently holds, cursor included. */
    private fun publish() {
        val emulator = vt ?: return
        // OSC 52, drained here because this is the one place every path that
        // feeds bytes ends up — the live stream, the poll, and a jump to the
        // bottom all call it, so a program's copy does not depend on which
        // route its output took to get here.
        emulator.takeClipboard()?.let { _copied.tryEmit(it) }
        _grid.value = emulator.snapshot()
        _phase.value = Phase.Live
    }

    /**
     * The URL under a cell of the screen as currently shown, or null.
     *
     * Asked of the emulator rather than of the host: it holds the same bytes,
     * and a round trip to answer a long press would arrive after the gesture.
     */
    fun urlAt(row: Int, column: Int): String? = vt?.urlAt(row, column)

    /**
     * "resource not found" is the host's answer for a terminal that is not
     * currently a live pane — restarted, stopped, never started. Everything
     * else is a real failure, and the two must read differently: one is "come
     * back later", the other is "something is wrong".
     */
    private fun report(error: Exception) {
        val message = error.message.orEmpty()
        _phase.value = if (message == "resource not found") Phase.NotLive else humanFailure(message)
    }

    /**
     * What actually goes under "Could not load".
     *
     * The core's word for a dead link is "not connected", which is right for a
     * log and wrong for a screen: it is lowercase, it is a fragment, and it
     * describes the FFI's session slot rather than anything the person holding
     * the phone did or can do about it. It is also the line they saw — one SSH
     * hiccup anywhere empties that slot and every call afterwards is answered
     * with it, so this is the most common failure text there is, not an edge.
     *
     * Matched on the string rather than on `DisconnectedException`, which
     * [report] could have offered, because the other caller could not: a stream
     * reports its end as a bare JSON string with no type left on it (see
     * `farcooler_client_stream_start`). One rule both paths can use beats a
     * typed check here and an untyped one two hundred lines away that drift
     * apart.
     *
     * Everything else is KEPT, whole. A message from the host is the host's to
     * word, and rewriting all of them into one apology would throw away the
     * only clue a real failure carries. It is no longer kept in the sentence's
     * slot, though — passing it through as the phase's only string put an SSH
     * fragment under a headline this app wrote, in the app's own face, with
     * nothing to mark where Far Cooler stopped speaking.
     *
     * The sentence says only what this side knows: the read did not finish. No
     * cause named, because from here the cause is unknowable and a guess sends
     * somebody to change a setting that was never the problem — see
     * `Enrollment.note(about:outcome:)` in the Mac app — and no retry promised,
     * because nothing here performs one.
     */
    private fun humanFailure(message: String): Phase =
        if (message == "not connected") {
            Phase.Failed("The connection to this runner dropped. Reconnecting…")
        } else {
            Phase.Failed("The request that reads this pane didn’t finish.", message)
        }

    // MARK: - Input

    /**
     * Send typed text. Each scalar is encoded on its own so a Ctrl modifier —
     * which only ever applies to the next key — transforms exactly one of them,
     * matching how a physical Ctrl key behaves.
     */
    fun send(text: String, modifiers: Int) {
        scope.launch {
            val emulator = vt ?: return@launch
            jumpToBottom(emulator)
            val bytes = ArrayList<Byte>()
            var first = true
            var index = 0
            while (index < text.length) {
                val point = text.codePointAt(index)
                index += Character.charCount(point)
                bytes.addAll(emulator.encodeKey(point, if (first) modifiers else 0).toList())
                first = false
            }
            write(bytes.toByteArray())
        }
    }

    fun sendKey(key: Int, modifiers: Int = 0) {
        scope.launch {
            val emulator = vt ?: return@launch
            jumpToBottom(emulator)
            write(emulator.encodeKey(key, modifiers))
        }
    }

    /**
     * Paste, bracketed if the program asked for it — without which an editor
     * auto-indents every pasted line and a shell runs each newline as a
     * command. A phone has no other way to get a long command into a terminal,
     * so this is not a nicety here the way it is on a desk with a keyboard.
     */
    fun paste(text: String) {
        scope.launch {
            val emulator = vt ?: return@launch
            jumpToBottom(emulator)
            write(emulator.encodePaste(text))
        }
    }

    /**
     * Typing means "act on the live screen", so it always returns there first.
     */
    private fun jumpToBottom(emulator: VtCore) {
        emulator.scrollToBottom()
        publish()
    }

    /**
     * Scroll by [lines], positive back into history.
     *
     * Asks the core to encode a wheel event for the program running in this
     * pane first. A full-screen program — `less`, an agent's TUI — gets mouse
     * reports instead, because from inside an alternate screen a wheel means
     * something to the program that this device's own scrollback cannot
     * express. Only once the core says the program does not want the event does
     * this fall back to scrolling the emulator's own history and redrawing
     * locally, which sends nothing to the host at all.
     */
    fun scroll(lines: Int, column: Int, row: Int) {
        if (lines == 0) return
        scope.launch {
            val emulator = vt ?: return@launch
            val button = if (lines > 0) Vt.MOUSE_WHEEL_UP else Vt.MOUSE_WHEEL_DOWN
            val bytes = ArrayList<Byte>()
            repeat(abs(lines)) {
                val chunk = emulator.encodeMouse(button, Vt.MOUSE_PRESS, column, row, 0)
                if (chunk == null) {
                    emulator.scroll(lines)
                    publish()
                    return@launch
                }
                bytes.addAll(chunk.toList())
            }
            write(bytes.toByteArray())
        }
    }

    private suspend fun write(bytes: ByteArray) {
        if (bytes.isEmpty()) return
        val hex = buildString(bytes.size * 2) {
            for (byte in bytes) {
                append(HEX[(byte.toInt() shr 4) and 0xF])
                append(HEX[byte.toInt() and 0xF])
            }
        }
        attempt {
            core.call("terminal.write", Connection.args("terminal" to terminalId, "hex" to hex))
        }
        // Nothing to prompt when streaming: the echo is already on its way back
        // down the same channel, and asking for a screen would only race it.
        // Polling has no such luxury — the moment a key is sent is the moment
        // staleness is least acceptable, so snap back to the fast interval
        // rather than waiting out however long a quiet screen had backed off to.
        if (!streaming) wake()
    }

    /**
     * Stop everything that paints, and everything that feeds what paints.
     *
     * The stream is stopped whether or not this object believes it is
     * streaming. It used to be conditional on iOS, which read as an
     * optimisation and was a leak: `streamEnded` sets the flag false before
     * anything tears down, so the one path that most needed the stream stopped
     * was the one path that skipped it.
     */
    private fun teardown() {
        poller?.cancel()
        poller = null
        geometry?.cancel()
        geometry = null
        resizeDebounce?.cancel()
        resizeDebounce = null
        pendingResizeSize = null
        inbox.clear()
        streaming = false
    }

    /**
     * Bytes handed over from off the emulator's thread, in the order they
     * arrived.
     *
     * The core delivers chunks in order — it drains its queue in one place —
     * but the emulator lives on its own thread, and hopping per chunk hands the
     * ordering to the scheduler, which makes no promise about it. So chunks are
     * appended here under a lock and taken in one piece, and however many hops
     * end up racing each other, the first to arrive carries everything and the
     * rest find nothing to do.
     */
    private class Inbox {
        private val lock = Any()
        private var buffer = java.io.ByteArrayOutputStream()

        fun append(chunk: ByteArray) = synchronized(lock) { buffer.write(chunk) }

        fun take(): ByteArray = synchronized(lock) {
            val out = buffer.toByteArray()
            buffer.reset()
            out
        }

        fun clear() = synchronized(lock) { buffer.reset() }
    }

    private companion object {
        /**
         * The busiest a poll loop gets: about as fast as a human can perceive a
         * screen redrawing, and comfortably above the host's own ~16 ms capture
         * cost — polling faster than the capture itself would just queue up SSH
         * round trips the host cannot answer any quicker.
         */
        const val FAST_INTERVAL_MS = 100L

        /** The idle cadence a quiet terminal settles into. */
        const val SLOW_INTERVAL_MS = 1_000L

        /**
         * How quickly an unchanging screen backs off. Geometric rather than a
         * fixed step, so a screen that goes quiet coasts to the slow interval
         * in a handful of polls rather than taking as long to slow down as it
         * took to speed up.
         */
        const val BACKOFF = 1.6

        /**
         * How long to wait for a burst of size changes to settle. A keyboard
         * sliding up and a rotation each produce several calls within a couple
         * of hundred milliseconds.
         */
        const val RESIZE_DEBOUNCE_MS = 200L

        /**
         * How often to ask the host how big this pane is now, while streaming.
         */
        const val GEOMETRY_INTERVAL_MS = 2_000L

        const val STREAM_RETRY_MS = 500L
        const val MAX_ATTACH_ATTEMPTS = 3

        val HEX = "0123456789abcdef".toCharArray()
    }
}
