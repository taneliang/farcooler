import Foundation
import OvernightVT
import SwiftUI

/// One terminal's screen, kept live while a `TerminalView` is on screen.
///
/// Streams, and polls only when it cannot stream. The two are genuinely
/// different pictures of the same pane, not two speeds of the same one: a
/// stream carries the bytes tmux wrote, in order, so the emulator here sees
/// exactly what one on the host would — cursor motion, partial redraws, a
/// spinner actually spinning. A poll carries the screen after it settled,
/// which is the same thing a photograph is to a film. Everything that made
/// this app feel remote came from the second one, and the fix was not to
/// photograph faster.
///
/// The polling path below is kept, and is not dead code: `startStream`
/// answers false when there is no ssh session to open a second channel on,
/// and a screen that updates once a second is enormously better than a screen
/// that says it cannot be shown.
///
/// This never decides whether a terminal is "live" — the host does, the same
/// way `Connection` never computes a workspace's state. An unreadable screen
/// is reported exactly as the host phrased it.
@MainActor
final class TerminalSession: ObservableObject {
    enum Phase: Equatable {
        case connecting
        case notLive
        case failed(String)
        case live
    }

    @Published private(set) var phase: Phase = .connecting
    @Published private(set) var grid: TerminalGrid?

    private var terminalID: String
    private let core: ClientCore
    private var vt: VTCore?
    private var poller: Task<Void, Never>?
    /// The size this screen would like the pane to be, kept even while
    /// nothing is happening about it so a later trigger — the app returning
    /// to the foreground, say — has something to re-assert.
    private var lastRequestedSize: (columns: Int, rows: Int)?
    /// The size the host was last actually asked to become. Doubles as "the
    /// pane's size as far as this screen currently knows it", since `prime`
    /// seeds it from the host's own answer — which is what lets a resize
    /// request be skipped entirely when the pane already happens to be the
    /// right shape.
    private var lastResizeSent: (columns: Int, rows: Int)?
    /// The size a pending debounce is going to ask for, distinct from
    /// `lastResizeSent`. Without this, a `configure` that keeps re-arriving
    /// with the same unchanged size — sub-pixel jitter in a `GeometryReader`
    /// during layout is enough — would cancel and restart the debounce timer
    /// forever and the resize would never actually fire.
    private var pendingResizeSize: (columns: Int, rows: Int)?
    private var resizeDebounce: Task<Void, Never>?
    /// How long to wait for a burst of size changes to settle before asking
    /// the host to reflow. A keyboard sliding up and a rotation each produce
    /// several `configure` calls within a couple of hundred milliseconds;
    /// this is long enough to coalesce a whole one of those into a single
    /// request.
    private static let resizeDebounceDelay: Duration = .milliseconds(200)

    /// The last screen decoded, kept so the next poll can tell whether
    /// anything actually changed. See `poll()` for what that buys.
    private var lastScreen: ScreenResponse?

    // MARK: - Streaming

    /// True once a second ssh channel is carrying this pane's bytes.
    private var streaming = false
    /// Bytes waiting to be fed, filled from off the main actor. See `Inbox`.
    private var inbox: Inbox?
    /// Watches for someone else resizing this pane. See `checkGeometry`.
    private var geometry: Task<Void, Never>?
    /// The size the emulator was built at, which is the pane's size as of the
    /// last time anything looked.
    private var paneSize: (columns: Int, rows: Int)?
    /// The revision of the last screen seen, so the geometry check can be
    /// answered in a hundred bytes when nothing moved.
    private var revision: UInt64 = 0
    /// Consecutive stream attaches that produced nothing before ending.
    /// Reset by the first byte through. See `streamEnded`.
    private var failedAttaches = 0
    private var started = false

    /// How often to ask the host how big this pane is now.
    ///
    /// The stream carries content and says nothing about geometry, which is
    /// correct — it is a byte stream, and inventing a frame format to carry a
    /// column count would make it something else. But the pane belongs to
    /// whoever else is looking at it, and someone splitting a window on the
    /// Mac resizes it out from under this screen. So geometry is asked for on
    /// its own slow schedule, and because the ask carries the revision this
    /// device already holds, the usual answer is a hundred bytes saying
    /// "still 80×24, still unchanged".
    private static let geometryInterval: Double = 2.0

    // MARK: - Adaptive polling (fallback only)
    //
    // Reached when there is no ssh session to stream over. "Adaptive" means
    // choosing how often to ask, cheaply, rather than changing what is asked
    // for. A capture costs the host about 16ms; at a fixed 1000ms interval
    // that 16ms was rounding error next to 984ms of the phone doing nothing
    // but waiting for its own clock. Polling faster while the screen is
    // actually moving, and backing off once it stops, spends that budget
    // where a human would notice it — mid-keystroke — instead of evenly
    // wherever a plain interval happens to land.

    /// The busiest a poll loop gets: about as fast as a human can perceive a
    /// screen redrawing, and comfortably above the host's own ~16ms capture
    /// cost — polling faster than the capture itself would just queue up SSH
    /// round trips the host cannot answer any quicker.
    private static let fastInterval: Double = 0.1
    /// The idle cadence a quiet terminal settles into. Still fast enough that
    /// "nothing happened in the last second" reads as current, not stale.
    private static let slowInterval: Double = 1.0
    /// How quickly an unchanging screen backs off. Geometric rather than a
    /// fixed step, so a screen that goes quiet coasts to the slow interval in
    /// a handful of polls rather than taking as long to slow down as it took
    /// to speed up.
    private static let backoffFactor: Double = 1.6

    private var interval: Double = TerminalSession.fastInterval

    init(terminalID: String, core: ClientCore) {
        self.terminalID = terminalID
        self.core = core
    }

    deinit {
        poller?.cancel()
        geometry?.cancel()
        resizeDebounce?.cancel()
        // The core's stream task outlives this object — it belongs to the ssh
        // session, not to whoever was watching — so it has to be told, not
        // just dropped. `nonisolated` capture of the id and the core is the
        // whole reason both are stored rather than passed around.
        let id = terminalID
        let core = self.core
        Task.detached { await core.stopStream(id) }
    }

    /// Point this same session at a different terminal — what the tab strip
    /// calls when you tap a sibling.
    ///
    /// A new `TerminalSession` per tap would work too, but this screen keeps
    /// one for its lifetime, so retargeting is what a tap actually means.
    /// Everything describing the outgoing terminal goes at once — its
    /// emulator, its screen, its stream — because every one of them would
    /// otherwise be read as belonging to the incoming one: a stale `grid`
    /// makes the tap look like it did nothing, and a stale `lastScreen` makes
    /// the new terminal's first capture compare equal by coincidence.
    func switchTo(_ id: String) {
        guard id != terminalID else { return }
        teardown()
        // The outgoing terminal, named before it stops being the current one.
        let leaving = terminalID
        Task { await stopStream(leaving) }
        terminalID = id
        vt = nil
        grid = nil
        lastScreen = nil
        revision = 0
        paneSize = nil
        phase = .connecting
        started = true
        // The size this device would like has not changed — the screen did
        // not resize, only what it is showing did — but `lastResizeSent`
        // described the OUTGOING terminal's pane, and comparing the new
        // terminal's shape against it would just be wrong. Clearing it makes
        // `scheduleResize` re-evaluate from scratch once `open` has asked the
        // incoming terminal what shape it actually is.
        lastResizeSent = nil
        Task { await open() }
        scheduleResize()
    }

    /// Debounce and dedupe a request to reflow the pane to `lastRequestedSize`.
    ///
    /// Every trigger that thinks the pane might need refitting — the screen
    /// appearing, a rotation, the keyboard showing, the tab strip switching
    /// terminals, the app returning to the foreground — funnels through here
    /// rather than calling the host directly, for two reasons. First, several
    /// of those triggers fire in a burst: a keyboard animation alone produces
    /// a `configure` call on close to every frame while it slides, and each
    /// one is a request the host would otherwise have to answer. Second, most
    /// calls into this method ask for a size the pane is already at, and
    /// asking again would just be a round trip that reflows a pane other
    /// people may be looking at for no visible change.
    ///
    /// `pendingResizeSize` exists because `lastRequestedSize` re-arriving
    /// unchanged is common — a `GeometryReader` can report the same size
    /// several times during one layout pass — and restarting the sleep below
    /// on every one of those would mean the debounce never actually elapses.
    /// Only a genuine change to the target restarts the clock.
    private func scheduleResize() {
        // Tuples are not `Equatable`, so the comparisons below unwrap by hand
        // rather than comparing the optionals directly.
        guard let size = lastRequestedSize else { return }
        if let sent = lastResizeSent, sent == size { return }
        if let pending = pendingResizeSize, pending == size { return }
        pendingResizeSize = size
        resizeDebounce?.cancel()
        resizeDebounce = Task { [weak self] in
            try? await Task.sleep(for: Self.resizeDebounceDelay)
            guard !Task.isCancelled else { return }
            await self?.resizePane()
        }
    }

    /// Actually ask the host to reflow the pane to `lastRequestedSize`, once
    /// `scheduleResize`'s debounce has settled on a value nothing has since
    /// superseded.
    ///
    /// A tmux pane is shared: resizing it reflows it for every other client
    /// attached to that window, the Mac included, not just for this phone.
    /// Doing this automatically, rather than behind the explicit button this
    /// method used to sit behind, is a deliberate trade the user asked for —
    /// unreadably tiny text on a pane sized for a laptop was judged worse than
    /// occasionally reflowing someone else's terminal. It is still a real
    /// cost, just one that is now paid by default instead of on request.
    private func resizePane() async {
        guard let size = lastRequestedSize else { return }
        if let sent = lastResizeSent, sent == size { return }
        pendingResizeSize = nil
        lastResizeSent = size
        _ = try? await core.call(
            "terminal.resize",
            ["terminal": terminalID, "columns": size.columns, "rows": size.rows])
        // The resize itself changes the screen, but not in a way this
        // terminal has captured yet — without clearing it, a resize to
        // exactly the content already on screen (rare, but possible) would
        // compare equal and the redraw would wait for the next backed-off
        // tick instead of showing the new dimensions immediately.
        lastScreen = nil
        // Reopened rather than waiting for the geometry check to notice what
        // this method just did: the emulator is still the old size, and the
        // redraw tmux is sending right now would be wrapped to it.
        if streaming {
            await open()
        } else {
            wake()
        }
    }

    /// Learn the size this screen would like the pane to be, open the
    /// terminal on the first call, and — after `scheduleResize`'s debounce —
    /// ask the host to reflow the pane to that size.
    ///
    /// This used to only record the size and leave resizing to an explicit
    /// tap on a toolbar button: a pane belongs to whoever else is looking at
    /// it too, and reflowing the shared window just because a phone glanced
    /// at it once squeezed every pane in that layout down to phone width for
    /// everyone, Mac included. The trade-off has not gone away — see
    /// `resizePane` — but the user's verdict was that unreadably tiny text on
    /// a pane sized for a laptop is the worse failure, so this now asks for
    /// real rather than waiting to be asked.
    func configure(columns: Int, rows: Int) async {
        guard columns > 0, rows > 0 else { return }
        lastRequestedSize = (columns, rows)
        if !started {
            started = true
            await open()
        }
        scheduleResize()
    }

    /// Re-assert the size this screen already wants, without anything having
    /// told it a new one.
    ///
    /// For triggers where the risk is not that this screen's own size
    /// changed but that the pane drifted while nothing here was watching it —
    /// chiefly the app returning to the foreground, where someone on the Mac
    /// could have resized the shared window while this device was
    /// backgrounded.
    func reassertSize() {
        scheduleResize()
    }

    /// Stop watching. Called from `.onDisappear` — a backgrounded terminal
    /// view has no business holding an ssh channel open, or spending this
    /// phone's battery on a screen nobody is reading.
    func stop() {
        teardown()
        started = false
        let id = terminalID
        Task { await stopStream(id) }
    }

    /// Stop everything that paints, and everything that feeds what paints.
    ///
    /// The stream is stopped whether or not this object believes it is
    /// streaming. It used to be conditional, which read as an optimisation and
    /// was a leak: `streamEnded` sets the flag false before anything tears
    /// down, so the one path that most needed the stream stopped was the one
    /// path that skipped it, and an ssh channel went on delivering into an
    /// emulator nothing was showing. `stopStream` is documented as safe when
    /// nothing is running, which is what makes the unconditional version the
    /// simpler one as well as the correct one.
    private func teardown() {
        poller?.cancel()
        poller = nil
        geometry?.cancel()
        geometry = nil
        resizeDebounce?.cancel()
        resizeDebounce = nil
        pendingResizeSize = nil
        inbox = nil
        streaming = false
    }

    /// Stop the stream and wait for the stop to land.
    ///
    /// Awaited wherever anything follows it, because stop and start go through
    /// the same actor in the order they are asked: a stop left in flight
    /// arrives after the next start and cancels the stream that start just
    /// opened. What that looks like is a session that believes it is streaming,
    /// attached to nothing — a screen that stops updating and never says why.
    private func stopStream(_ id: String) async {
        await core.stopStream(id)
    }

    // MARK: - Opening

    /// Show this terminal and keep it live, by whichever means the connection
    /// supports.
    ///
    /// The screen call first is not for something to look at: the stream
    /// carries no geometry, and an emulator has to be built at some size before
    /// a single byte can be fed to it. It is also the one call that reports a
    /// pane that is not running, which a stream can only express by failing to
    /// open.
    ///
    /// Which is why it does not paint when a stream is coming. A capture and a
    /// stream disagree about the same screen — tmux hands out captures with
    /// bare line feeds, so `render` has to repair them, and even repaired they
    /// are a screen re-flowed rather than the bytes that drew it. Painting one
    /// and then being repainted by the stream's own replay a moment later is
    /// visible, and it was: every reopen flashed a differently-wrapped screen
    /// before settling. A spinner for that moment is honest; a wrong screen is
    /// not.
    private func open() async {
        teardown()
        // Awaited, unlike the fire-and-forget stop `teardown` does for callers
        // that cannot wait. Both stop and start go through the same actor, and
        // a stop that had not landed yet would arrive after the start below and
        // cancel the stream this call just opened — leaving a session that
        // believes it is streaming attached to nothing, which is a screen that
        // stops updating and never says why.
        await stopStream(terminalID)
        guard let screen = await prime() else { return }
        if await attach() { return watchGeometry() }
        render(screen)
        startLoop()
    }

    /// One screen: the pane's size, whether it is running at all, and — only
    /// if nothing better is coming — something to draw.
    private func prime() async -> ScreenResponse? {
        do {
            let data = try await core.call("terminal.screen", ["terminal": terminalID])
            let response = try JSONDecoder().decode(ScreenResponse.self, from: data)
            revision = response.revision
            paneSize = (response.columns, response.rows)
            // The pane's actual shape, straight from the host, seeds the
            // baseline `scheduleResize` compares against — so a `configure`
            // that asks for the size the pane already happens to be does not
            // spend a round trip saying so.
            lastResizeSent = (response.columns, response.rows)
            // An emulator at the pane's size, empty. Built here rather than by
            // whoever paints first, because the streaming path never paints a
            // capture and still has to have something to feed: the bytes start
            // arriving the moment the channel opens, and a chunk with no
            // emulator to receive it is simply lost. It stays empty because the
            // stream's first act is to clear the screen and replay it.
            vt = VTCore(columns: response.columns, rows: response.rows)
            return response
        } catch {
            report(error)
            return nil
        }
    }

    /// Open the byte stream, and say whether it opened.
    private func attach() async -> Bool {
        let inbox = Inbox()
        self.inbox = inbox

        let opened = await core.startStream(
            terminalID,
            onChunk: { [weak self] bytes in
                // Buffered here, on whatever thread the core drained on, and
                // consumed on the main actor. Feeding directly from a hop per
                // chunk would put the emulator's input at the mercy of the
                // order unstructured tasks happen to run in, and bytes that
                // arrive out of order are not a slightly wrong screen — they
                // are an escape sequence cut in half.
                inbox.append(bytes)
                Task { @MainActor in self?.consume() }
            },
            onEnd: { [weak self] error in
                Task { @MainActor in self?.streamEnded(error) }
            })

        streaming = opened
        if !opened { self.inbox = nil }
        return opened
    }

    /// Feed everything that has arrived, in order, and redraw once.
    private func consume() {
        guard let inbox, let vt else { return }
        let bytes = inbox.take()
        guard !bytes.isEmpty else { return }
        failedAttaches = 0
        vt.feed(bytes)
        publish()
    }

    /// The stream stopped. Try again, then settle for polling.
    ///
    /// A stream ends for two very different reasons: the pane finished, or the
    /// channel did. Only the host can tell those apart, so this asks it — by
    /// reopening, which begins with the screen call that reports a pane that
    /// is no longer running. A channel that drops repeatedly without ever
    /// delivering a byte is a connection that cannot carry a stream, and
    /// retrying it forever would be a worse screen than the polling that
    /// definitely works.
    private func streamEnded(_ error: String?) {
        guard streaming else { return }
        streaming = false
        failedAttaches += 1
        guard failedAttaches < 3 else {
            if let error { phase = .failed(error) }
            // Everything the streaming path set up goes before the polling path
            // starts, and that is the whole fix for a screen that flashed a
            // wrong layout once a second. Falling back used to start the poll
            // loop and leave the rest running, so a stream that was still
            // delivering fed the emulator correct bytes while the poll repainted
            // a capture over the top of them — two painters, disagreeing, one of
            // them every second. A fallback has to be a handover, not an
            // addition.
            teardown()
            startLoop()
            return
        }
        Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(500))
            await self?.open()
        }
    }

    // MARK: - Geometry

    private func watchGeometry() {
        geometry?.cancel()
        geometry = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(Self.geometryInterval))
                guard !Task.isCancelled else { return }
                await self?.checkGeometry()
            }
        }
    }

    /// Notice someone else resizing this pane, and rebuild if they did.
    ///
    /// Rebuilding rather than calling `VTCore.resize`: tmux answers a resize by
    /// making the program redraw itself, and those bytes are already on their
    /// way down a stream whose emulator is still the old width. Reopening
    /// replays the screen at the size it is actually being drawn at now, which
    /// is the same recovery the stream performs when it first attaches.
    private func checkGeometry() async {
        guard streaming, let size = paneSize else { return }
        guard
            let data = try? await core.call(
                "terminal.screen", ["terminal": terminalID, "knownRevision": revision]),
            let response = try? JSONDecoder().decode(ScreenResponse.self, from: data)
        else { return }
        revision = response.revision
        guard response.columns != size.columns || response.rows != size.rows else { return }
        await open()
    }

    /// Cancel and restart the loop at the fast interval — "poll right now,
    /// then resume at full speed" — without ending up with two loops running.
    ///
    /// A plain `Task { await poll() }` fired alongside the existing loop would
    /// work for the one poll, but the existing loop's own `Task.sleep` would
    /// still be counting down on the OLD interval underneath it, so the next
    /// scheduled tick would still land late. Restarting the loop is what
    /// actually changes the cadence rather than just sneaking in one extra
    /// poll ahead of it.
    private func wake() {
        interval = Self.fastInterval
        poller?.cancel()
        startLoop()
    }

    private func startLoop() {
        poller = Task { [weak self] in
            while let self, !Task.isCancelled {
                await self.poll()
                guard !Task.isCancelled else { return }
                try? await Task.sleep(for: .seconds(self.interval))
            }
        }
    }

    private func poll() async {
        do {
            let data = try await core.call("terminal.screen", ["terminal": terminalID])
            let response = try JSONDecoder().decode(ScreenResponse.self, from: data)

            // The cheap compare that makes backing off free: `ScreenResponse`
            // carries the still-base64 payload, so this is a byte compare of
            // exactly what the host sent, before any of the more expensive
            // work — base64 decoding, feeding a fresh `VTCore`, copying a
            // snapshot into cells SwiftUI will diff — runs on a screen that
            // has not moved. Most polls of an idle terminal end right here.
            guard response != lastScreen else {
                interval = min(interval * Self.backoffFactor, Self.slowInterval)
                return
            }
            lastScreen = response
            interval = Self.fastInterval
            render(response)
        } catch {
            report(error)
            // Back off on a persistent error too — a terminal that keeps
            // failing to load has no "changing" to detect, and retrying it
            // every 100ms would hammer a host that has already said no.
            interval = min(interval * Self.backoffFactor, Self.slowInterval)
        }
    }

    // MARK: - Drawing

    /// Build an emulator from a captured screen and show it.
    ///
    /// A fresh `VTCore` per capture rather than feeding into the last one: a
    /// capture is a whole screen, not a continuation of one, and a persistent
    /// core would need the dump to be self-clearing to stay correct. "Assume
    /// every capture starts by wiping the grid" is a fact about the host's
    /// tmux version, not something this file should have to trust. The
    /// streaming path is the opposite by nature — its bytes ARE a
    /// continuation — which is why it keeps whatever core this built.
    private func render(_ response: ScreenResponse) {
        revision = response.revision
        guard let bytes = Data(base64Encoded: response.contents) else {
            phase = .failed("The host sent a screen this device could not decode.")
            return
        }
        let emulator = VTCore(columns: response.columns, rows: response.rows)
        // A capture separates its lines with a bare line feed, which to a
        // terminal means "down one row" and nothing about which column. Fed
        // straight in, every line starts where the previous one ended and the
        // screen arrives as a staircase — text broken mid-word at a different
        // place on each row, which is what a wrong screen looked like here.
        // The daemon repairs this for the replay it sends down a stream, for
        // exactly the same reason; a capture arriving by any other route needs
        // the same repair. Live pty bytes never do — a pty already emits both.
        emulator.feed(carriageReturned(withoutTrailingNewlines([UInt8](bytes))))
        vt = emulator
        paneSize = (response.columns, response.rows)
        // The host's cursor, not the emulator's: a capture is text, so feeding
        // it leaves the caret wherever the last character landed — the bottom
        // left — rather than at the prompt someone is typing into. The stream
        // has no such gap, and `publish` uses the emulator's own.
        grid = emulator.withSnapshot {
            TerminalGrid(
                snapshot: $0,
                cursorRow: response.cursorRow,
                cursorColumn: response.cursorColumn)
        }
        phase = .live
    }

    /// Drop the newline a captured screen ends with.
    ///
    /// A capture is as many lines as the screen is tall, so feeding the last
    /// one's line feed moves the caret off the bottom row and scrolls the whole
    /// screen up by one: the top line goes into history and every remaining row
    /// is drawn one higher than it belongs. The caret then comes from the host,
    /// which knows nothing about that scroll, so it lands a row below the text
    /// it should be sitting in — which is exactly what a caret one row off
    /// looks like. The daemon drops this same newline before replaying a screen
    /// down a stream, and says why; a capture arriving by any other route needs
    /// the same treatment.
    private func withoutTrailingNewlines(_ bytes: [UInt8]) -> [UInt8] {
        var end = bytes.count
        while end > 0, bytes[end - 1] == 0x0A || bytes[end - 1] == 0x0D { end -= 1 }
        return Array(bytes[..<end])
    }

    /// Give every bare line feed the carriage return a captured screen left
    /// out. One that already has one is left alone, so this is safe to apply
    /// to anything.
    private func carriageReturned(_ bytes: [UInt8]) -> [UInt8] {
        var out: [UInt8] = []
        out.reserveCapacity(bytes.count + bytes.count / 40)
        var previous: UInt8 = 0
        for byte in bytes {
            if byte == 0x0A, previous != 0x0D { out.append(0x0D) }
            out.append(byte)
            previous = byte
        }
        return out
    }

    /// Show what the emulator currently holds, cursor included.
    private func publish() {
        guard let vt else { return }
        grid = vt.withSnapshot { TerminalGrid(snapshot: $0) }
        phase = .live
    }

    /// "resource not found" is the host's answer for a terminal that is not
    /// currently a live pane — restarted, stopped, never started. Everything
    /// else is a real failure, and the two must read differently: one is
    /// "come back later", the other is "something is wrong".
    private func report(_ error: Error) {
        let message = error.localizedDescription
        phase = message == "resource not found" ? .notLive : .failed(message)
    }

    /// Send typed text. Each scalar is encoded on its own so a Ctrl modifier —
    /// which only ever applies to the next key — transforms exactly one of
    /// them, matching how a physical Ctrl key behaves.
    func send(text: String, modifiers: VTModifiers) async {
        guard let vt else { return }
        jumpToBottom(vt)
        var bytes: [UInt8] = []
        for (index, scalar) in text.unicodeScalars.enumerated() {
            bytes += vt.encode(scalar: scalar, modifiers: index == 0 ? modifiers : [])
        }
        await write(bytes)
    }

    func send(key: UInt32, modifiers: VTModifiers = []) async {
        guard let vt else { return }
        jumpToBottom(vt)
        await write(vt.encode(key: key, modifiers: modifiers))
    }

    /// Typing means "act on the live screen", so it always returns there
    /// first — the same rule the Mac's `keyDown` follows. Unlike the Mac,
    /// this does not guard on whether the view was actually scrolled: there
    /// is no per-frame redraw loop here to spare the cost of an unnecessary
    /// one, so the guard would only add a branch without saving anything.
    private func jumpToBottom(_ vt: VTCore) {
        vt.scrollToBottom()
        publish()
    }

    /// Scroll by `lines`, positive back into history. Mirrors the Mac's
    /// `scrollWheel(with:)` exactly, because that policy is the correct one
    /// on any platform: ask the core to encode a wheel event for the program
    /// running in this pane first. A full-screen program — `less`, an
    /// agent's TUI — gets mouse reports instead, because from inside an
    /// alternate screen a wheel means something to the program that this
    /// device's own scrollback cannot express. Only once the core says the
    /// program does not want the event does this fall back to scrolling the
    /// emulator's own history and redrawing locally, which sends nothing to
    /// the host at all.
    func scroll(lines: Int, column: Int, row: Int) async {
        guard let vt, lines != 0 else { return }
        let button: UInt32 =
            lines > 0 ? UInt32(OVERNIGHT_VT_MOUSE_WHEEL_UP) : UInt32(OVERNIGHT_VT_MOUSE_WHEEL_DOWN)

        var bytes: [UInt8] = []
        for _ in 0..<abs(lines) {
            guard
                let chunk = vt.encode(
                    mouse: button, action: UInt32(OVERNIGHT_VT_MOUSE_PRESS),
                    column: column, row: row, modifiers: [])
            else {
                vt.scroll(lines: Int32(lines))
                publish()
                return
            }
            bytes.append(contentsOf: chunk)
        }
        await write(bytes)
    }

    private func write(_ bytes: [UInt8]) async {
        guard !bytes.isEmpty else { return }
        let hex = bytes.map { String(format: "%02x", $0) }.joined()
        _ = try? await core.call("terminal.write", ["terminal": terminalID, "hex": hex])
        // Nothing to prompt when streaming: the echo is already on its way
        // back down the same channel, and asking for a screen would only race
        // it. Polling has no such luxury — the moment a key is sent is the
        // moment staleness is least acceptable, so snap back to the fast
        // interval rather than waiting out however long a quiet screen had
        // backed off to.
        if !streaming { wake() }
    }
}

/// Bytes handed over from off the main actor, in the order they arrived.
///
/// The core delivers chunks in order — it drains its queue in one place — but
/// the emulator lives on the main actor, and hopping per chunk hands the
/// ordering to the scheduler, which makes no promise about it. So chunks are
/// appended here under a lock and taken in one piece, and however many hops
/// end up racing each other, the first one to arrive carries everything and
/// the rest find nothing to do.
private final class Inbox: @unchecked Sendable {
    private let lock = NSLock()
    private var bytes: [UInt8] = []

    func append(_ chunk: [UInt8]) {
        lock.lock()
        bytes += chunk
        lock.unlock()
    }

    func take() -> [UInt8] {
        lock.lock()
        defer { lock.unlock() }
        let out = bytes
        bytes.removeAll(keepingCapacity: true)
        return out
    }
}

private struct ScreenResponse: Decodable, Equatable {
    var contents: String
    var columns: Int
    var rows: Int
    var cursorColumn: Int
    var cursorRow: Int
    /// A hash of the screen and cursor, handed back on the next ask so the
    /// host can answer "unchanged" in a hundred bytes instead of resending a
    /// capture nothing did anything with.
    var revision: UInt64
    var unchanged: Bool
}

/// A plain-data copy of one screen, safe to publish and hold past the moment
/// it was read.
///
/// `VTSnapshot` borrows a buffer that dies on the next core call; a `@Published`
/// property lives until the next poll overwrites it and is read by SwiftUI on
/// its own schedule, not this file's. Those two lifetimes cannot share a
/// pointer, so this copies once, here, and nowhere downstream has to worry
/// about it again.
struct TerminalGrid {
    var columns: Int
    var rows: Int
    var cells: [TerminalCell]
    var cursorRow: Int
    var cursorColumn: Int

    /// The emulator's own caret, which is the right one whenever the bytes
    /// that moved it are the same bytes that drew the screen around it.
    init(snapshot: VTSnapshot) {
        self.init(
            snapshot: snapshot,
            cursorRow: snapshot.cursorRow,
            cursorColumn: snapshot.cursorColumn)
    }

    init(snapshot: VTSnapshot, cursorRow: Int, cursorColumn: Int) {
        columns = snapshot.columns
        rows = snapshot.rows
        cells = (0..<(snapshot.rows * snapshot.columns)).map { i in
            TerminalCell(snapshot.cells[i])
        }
        // Clamp rather than trust: the cursor position comes from a separate
        // call (`terminal.cursor`, folded into this response by the host) than
        // the screen dump, and the two can race a resize by one poll interval.
        self.cursorRow = min(max(cursorRow, 0), max(rows - 1, 0))
        self.cursorColumn = min(max(cursorColumn, 0), max(columns - 1, 0))
    }

    subscript(row: Int, column: Int) -> TerminalCell {
        cells[row * columns + column]
    }
}

struct TerminalCell {
    var character: Character?
    var bold: Bool
    var wide: Bool
    /// Already resolved for `OVERNIGHT_VT_FLAG_INVERSE` — the core reports
    /// foreground and background separately from the flag, and a renderer
    /// that forgot to swap them would draw reverse-video text invisibly on
    /// itself.
    var foreground: Color
    var background: Color

    init(_ cell: OvernightVtCell) {
        character = cell.character
        bold = cell.isBold
        wide = cell.isWide
        foreground = Color(packed: cell.isInverse ? cell.bg : cell.fg)
        background = Color(packed: cell.isInverse ? cell.fg : cell.bg)
    }
}

extension Color {
    init(packed: UInt32) {
        self.init(
            red: Double((packed >> 16) & 0xFF) / 255,
            green: Double((packed >> 8) & 0xFF) / 255,
            blue: Double(packed & 0xFF) / 255)
    }
}

/// Colours the core did not resolve, because they belong to the phone's
/// screen, not to any program running on the host: the fill behind a short
/// last row and the cursor block. Values mirror the Mac app's so the same
/// terminal looks like the same terminal on both.
enum TerminalPalette {
    static let backgroundPacked: UInt32 = 0x12_14_19
    static let background = Color(packed: backgroundPacked)
    static let cursor = Color(red: 0.44, green: 0.66, blue: 1.0, opacity: 0.9)
}
