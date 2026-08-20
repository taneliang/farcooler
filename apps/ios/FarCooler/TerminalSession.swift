import Foundation
import FarCoolerVT
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
    /// What the pane looked like before this device reshaped it, so leaving
    /// can put it back. See `releasePane`.
    private var shapeBeforeUs: (columns: Int, rows: Int)?
    /// Whether this device has asked the host to reshape the pane yet.
    ///
    /// Gating `shapeBeforeUs` on this rather than on being the first `prime`,
    /// because the two race: the debounced resize is 200ms and a screen call
    /// is a round trip to the host, so a `prime` can easily land after the
    /// resize and record the shape THIS device just imposed as the one to hand
    /// back. Which made handing back a no-op that looked like it worked.
    private var hasResized = false
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
    /// Watches for a stream that opened and then said nothing. See
    /// ``waitForTheFirstByte()``.
    private var silence: Task<Void, Never>?
    /// The size the emulator was built at, which is the pane's size as of the
    /// last time anything looked.
    private var paneSize: (columns: Int, rows: Int)?
    /// The host's caret, for as long as the emulator's own cannot be believed.
    ///
    /// Set whenever `render` builds an emulator out of a CAPTURE. A capture is
    /// text: feeding it leaves the emulator's caret wherever the last character
    /// landed — the end of the last non-empty line — rather than at the prompt
    /// someone is typing into. `render` already knew this and passed the host's
    /// cursor explicitly; `publish` did not, so every path that redraws without
    /// a fresh capture put the caret back at the end of the text.
    ///
    /// That is the jump: typing called `jumpToBottom` → `publish`, which drew
    /// the caret at the last character of the last line, and ~100ms later the
    /// poll's `render` put it back at the prompt. Once per keystroke.
    ///
    /// Cleared by `consume`, because from the first streamed byte onwards the
    /// bytes that move the emulator's caret ARE the bytes that draw the screen
    /// around it, and the host's separately-fetched cursor is the stale one.
    private var capturedCursor: (row: Int, column: Int)?
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
        let handBack = shapeBeforeUs
        shapeBeforeUs = nil
        Task {
            await stopStream(leaving)
            await releasePane(leaving, to: handBack)
        }
        terminalID = id
        vt = nil
        grid = nil
        lastScreen = nil
        revision = 0
        paneSize = nil
        capturedCursor = nil
        phase = .connecting
        started = true
        // The strike count belongs to the terminal being left, not the one
        // arriving. Carried over, a terminal whose predecessor had already
        // given up on streaming reached the three-strike fallback on its first
        // hiccup and showed a failure it had not earned — which is the error
        // that flashed when switching tabs.
        failedAttaches = 0
        hasResized = false
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

    /// The link under this session was replaced by a new one.
    ///
    /// Everything the old link set up — the stream, the emulator's idea of
    /// where the cursor is, the last screen a poll compared against — belonged
    /// to an SSH session that no longer exists, so it is dropped rather than
    /// reused. Without this the screen still recovers, because `streamEnded`
    /// falls back to polling, but it stays on the slower path until the view
    /// is rebuilt: streaming is one round trip and polling is an interval.
    ///
    /// `switchTo` minus the two things that only make sense when the terminal
    /// itself changes. The id is the same, and the outgoing pane is not handed
    /// its old size back — that pane is on the far side of a link that is
    /// gone, so asking it anything is a request into the void.
    /// Come back to a pane that was hidden, keeping what it looked like.
    ///
    /// `PaneHost` mounts every visited pane and hides the ones you are not
    /// looking at, precisely so their state survives — and the pane's state
    /// includes its screen. So coming back is not `relink`: nothing about the
    /// link changed, the terminal id is the same, and the only thing that
    /// actually stopped is the stream, which `stop` closed on the way out.
    ///
    /// Using `relink` here defeated the whole mechanism, and did it by a race.
    /// Two `.task` modifiers fire when a pane becomes visible: this one, and
    /// the geometry one that calls `configure`. `relink` bails on `!started`
    /// and `configure` sets `started` — so when `configure` won, `relink` no
    /// longer bailed, wiped `grid`, `vt` and `phase`, and the pane a person
    /// had just tapped back to went to "Loading…" and rebuilt itself from
    /// nothing. When `relink` won, it bailed and the switch was instant. Same
    /// two lines of code, two behaviors, decided by scheduling.
    ///
    /// Guarding on `started` from the OTHER side settles it whichever way the
    /// race lands: whoever arrives first opens, the second finds it open and
    /// does nothing, and neither ever throws the screen away.
    ///
    /// What stays on screen meanwhile is the pane as it was when you left it,
    /// which is what it is: the stream's replay repaints it within a round
    /// trip, and a screen one switch out of date beats a spinner that has
    /// nothing to say at all.
    func resume() {
        guard !started else { return }
        started = true
        // A fresh visit deserves fresh strikes: three dead attaches from an
        // earlier visit must not send this one straight to polling.
        failedAttaches = 0
        Task { await open() }
        scheduleResize()
    }

    func relink() {
        guard started else { return }
        teardown()
        vt = nil
        grid = nil
        lastScreen = nil
        revision = 0
        paneSize = nil
        capturedCursor = nil
        phase = .connecting
        failedAttaches = 0
        hasResized = false
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
        hasResized = true
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
        let handBack = shapeBeforeUs
        shapeBeforeUs = nil
        Task {
            await stopStream(id)
            await releasePane(id, to: handBack)
        }
    }

    /// Give a pane back the shape it had before this device reshaped it.
    ///
    /// A phone reflows a pane to its own narrow viewport, which is right while
    /// it is the thing looking at it and wrong the moment it is not: whoever
    /// else has that terminal on screen is left with a column of phone-shaped
    /// text. So switching tabs, or leaving the terminal screen, hands the pane
    /// back rather than leaving the last viewer's shape imposed on everyone.
    ///
    /// Restoring the remembered size rather than asking who else is watching,
    /// because nothing here knows that — the host would have to, and a viewer
    /// registry is a bigger thing than the one case this covers. What it
    /// cannot survive is this device disappearing without saying so; the pane
    /// then keeps the phone's shape until something else resizes it.
    private func releasePane(_ id: String, to shape: (columns: Int, rows: Int)?) async {
        guard let shape else { return }
        _ = try? await core.call(
            "terminal.resize",
            ["terminal": id, "columns": shape.columns, "rows": shape.rows])
    }

    /// Stop everything on THIS side that paints, and everything that feeds
    /// what paints. The ssh channel is not one of them.
    ///
    /// Stopping the stream is the caller's, and it has to be: the stop is
    /// async and every caller needs it sequenced differently. `switchTo` and
    /// `stop` stop a named terminal and then hand its pane back; `open` awaits
    /// the stop so it cannot land after the start it is about to make — the
    /// hazard `stopStream` documents. A fire-and-forget stop in here would put
    /// that hazard back into `open`, which is the one path that must not have
    /// it.
    ///
    /// This comment used to claim the opposite, and the two fallback paths
    /// believed it: `streamEnded` and `streamSaidNothing` tore down and went to
    /// polling while their ssh channel stayed open, delivering into an
    /// emulator nothing was showing. Ten of those is `MaxSessions` on a default
    /// sshd, after which no stream opens at all — see the stop each of them now
    /// makes for itself.
    private func teardown() {
        poller?.cancel()
        poller = nil
        geometry?.cancel()
        geometry = nil
        silence?.cancel()
        silence = nil
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
        guard let screen = await prime() else {
            // A screen call that failed leaves nothing running that would ever
            // look again, and that was the bug: this returned here, so the pane
            // kept whatever `report` had just set and kept it forever.
            //
            // Which matters far more than one unlucky call suggests, because
            // the failure is not local to this pane. An ssh hiccup on ANY call
            // anywhere in the app empties the core's session slot — see
            // `farcooler_client_call` — and every call after it is answered
            // "not connected" until something reconnects. So the pane that
            // happened to be opening during that window showed "Could not
            // load", stopped asking, and stayed there even once the link was
            // back.
            //
            // The poll loop IS the retry, and it already behaves correctly for
            // this: `poll` reports its own failures and backs off to
            // `slowInterval`, so a host that has genuinely said no is not
            // hammered, and the first answer that does arrive calls `render`,
            // which returns the pane to `.live` on its own. No tap, no
            // `relink`, and no dependence on this pane being the visible one at
            // the moment `Connection` notices the drop.
            //
            // Only for `.failed`. A `.notLive` pane is not a pane that could
            // not be reached, it is a pane the host says is not running, and
            // polling one of those every second forever would spend a round
            // trip per second to be told the same true thing.
            if case .failed = phase { startLoop() }
            return
        }
        if await attach() {
            watchGeometry()
            return waitForTheFirstByte()
        }
        render(screen)
        startLoop()
    }

    /// A stream that opened, was accepted, and then never said anything.
    ///
    /// `streamEnded` is the recovery for a channel that DROPS, and it was the
    /// only recovery there was. So a channel that opens and then goes quiet had
    /// nothing watching it at all: `attach` returning true is read as "the
    /// stream is live, stop polling", the capture is deliberately not painted
    /// because the stream's own replay is about to arrive, and `phase` only
    /// leaves `.connecting` when the first bytes reach `consume`. No bytes, no
    /// end, no timeout anywhere — a spinner reading "Loading shell…" for as
    /// long as anybody cared to watch it.
    ///
    /// Nothing in the protocol promises a first byte. The daemon's replay is
    /// what normally arrives at once, and a daemon too old to send one, an
    /// sshd with no channels left, and a pane whose replay is genuinely empty
    /// all look identical from this side: open, and silent.
    ///
    /// So silence gets a deadline, and the answer to it is the handover
    /// `streamEnded` already performs for a channel that cannot carry a stream
    /// — stop everything the streaming path set up, then poll. Polling is
    /// slower and it works, which beats a spinner that is neither.
    ///
    /// Two seconds because the normal case never reaches it: the replay is one
    /// round trip, and a stream still saying nothing after two seconds is not
    /// one worth waiting on. It costs nothing when it does not fire, and the
    /// first byte cancels it — see `consume`.
    private func waitForTheFirstByte() {
        silence?.cancel()
        silence = Task { [weak self] in
            try? await Task.sleep(for: .seconds(2))
            guard !Task.isCancelled else { return }
            await self?.streamSaidNothing()
        }
    }

    /// The deadline passed with the screen still blank.
    private func streamSaidNothing() {
        // Only while nothing has been painted. A stream that delivered and then
        // went quiet is an idle pane, which is the ordinary state of most of
        // them, and tearing that down would swap a working stream for polling
        // every two seconds of quiet.
        guard streaming, phase == .connecting else { return }
        teardown()
        // Fire and forget, safely: this path goes to polling and never reopens,
        // so there is no start for a late stop to arrive after. A channel that
        // opened and said nothing is the likeliest one to be wedged, and
        // leaving it held is how the next pane finds no channels left.
        let id = terminalID
        Task { await stopStream(id) }
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
            // The shape the pane had before this device asked for anything,
            // remembered once so it can be handed back on the way out. See
            // `releasePane`.
            if shapeBeforeUs == nil, !hasResized {
                shapeBeforeUs = (response.columns, response.rows)
            }
            // An emulator at the pane's size, empty. Built here rather than by
            // whoever paints first, because the streaming path never paints a
            // capture and still has to have something to feed: the bytes start
            // arriving the moment the channel opens, and a chunk with no
            // emulator to receive it is simply lost. It stays empty because the
            // stream's first act is to clear the screen and replay it.
            let emulator = VTCore(columns: response.columns, rows: response.rows)
            // A fresh core starts on the VT crate's own default palette, not
            // the theme in force. Without this the chrome would be themed and
            // every character would not.
            emulator.setPalette(Themes.shared.current.packed)
            applyModes(response.modes, to: emulator)
            vt = emulator
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
        // The stream spoke, so the deadline on its silence is spent. Cancelled
        // rather than left to fire and find `phase` already `.live`, because a
        // task sleeping for two seconds per pane per open is a real cost on a
        // tab strip, and because the guard it would rely on is a second place
        // to get this right.
        silence?.cancel()
        silence = nil
        failedAttaches = 0
        vt.feed(bytes)
        // From here on the emulator's own caret is the true one: these bytes
        // are a continuation of the screen they move the caret across, unlike
        // a capture. See `capturedCursor`.
        capturedCursor = nil
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
            if let error { phase = .failed(Self.humanFailure(error)) }
            // Everything the streaming path set up goes before the polling path
            // starts, and that is the whole fix for a screen that flashed a
            // wrong layout once a second. Falling back used to start the poll
            // loop and leave the rest running, so a stream that was still
            // delivering fed the emulator correct bytes while the poll repainted
            // a capture over the top of them — two painters, disagreeing, one of
            // them every second. A fallback has to be a handover, not an
            // addition.
            teardown()
            // See `teardown`: it stops nothing on the host, and this is the
            // path its comment wrongly promised it did. Three dead attaches is
            // three channels, and the pane that inherits an sshd with none left
            // gets a stream that opens and never speaks.
            let id = terminalID
            Task { await stopStream(id) }
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
        let now = (columns: response.columns, rows: response.rows)
        guard now != size else { return }
        paneSize = now

        // A pane that is the shape THIS device asked for is not news, and
        // reopening for it was a loop: the resize lands, the next check sees a
        // size that no longer matches the one `prime` recorded, reopens, and
        // the reopen's own resize starts it again. Nine ssh channels for one
        // terminal had accumulated by the time it was noticed, and every
        // reopen left a window in which the emulator was fresh — so it did not
        // yet know the pane wanted the mouse, and scrolling silently did
        // nothing until the replay arrived. That is the intermittent scroll.
        //
        // The emulator is reflowed in place instead. It holds the same screen
        // either way, and tmux is already sending the program's redraw down
        // the stream this is beside.
        if let sent = lastResizeSent, now == sent {
            vt?.resize(columns: now.columns, rows: now.rows)
            publish()
            return
        }

        // Somebody else reshaped it. That needs the replay, because only the
        // host can re-wrap a screen it wrapped at a different width.
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
    /// Tell a fresh emulator what the program has already asked for.
    ///
    /// Without this the emulator is built believing the program wants no
    /// mouse, is not on the alternate screen and sends ordinary arrow keys —
    /// wrong for every full-screen program there is. The stream's replay says
    /// the same thing, and depending on it was the bug: a stream that ends and
    /// reattaches rebuilds the emulator, and the replay carrying the modes had
    /// gone into the one that was discarded. What survived declined every
    /// wheel event, so scrolling silently stopped working — and stayed stopped,
    /// because nothing else ever mentions modes again.
    ///
    /// Arriving with the screen, they cannot be missed by whichever stream
    /// happens to win.
    private func applyModes(_ modes: String?, to emulator: VTCore) {
        guard let modes, !modes.isEmpty else { return }
        emulator.feed([UInt8](modes.utf8))
    }

    private func render(_ response: ScreenResponse) {
        revision = response.revision
        guard let bytes = Data(base64Encoded: response.contents) else {
            phase = .failed("The host sent a screen this device could not decode.")
            return
        }
        let emulator = VTCore(columns: response.columns, rows: response.rows)
        // A fresh core starts on the VT crate's own default palette, not the
        // theme in force. Without this the chrome would be themed and every
        // character would not.
        emulator.setPalette(Themes.shared.current.packed)
        applyModes(response.modes, to: emulator)
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
        // left — rather than at the prompt someone is typing into.
        //
        // Remembered rather than only used here. Every OTHER redraw of this
        // emulator — a keystroke's `jumpToBottom`, a reflow, a wheel event —
        // goes through `publish`, which has no response to read it off and
        // used the emulator's own. See `capturedCursor`.
        capturedCursor = (response.cursorRow, response.cursorColumn)
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
        // OSC 52, drained here because this is the one place every path that
        // feeds bytes ends up — the live stream, the poll, and a jump to the
        // bottom all call it, and a program's copy must not depend on which
        // route its output took.
        //
        // On this platform it is the ONLY way anything on screen reaches the
        // clipboard: there is no text selection in the renderer.
        if let copied = vt.takeClipboard() {
            UIPasteboard.general.string = copied
        }
        // Whichever caret can currently be believed. See `capturedCursor`:
        // while the emulator holds a capture, its own is at the end of the
        // text and the host's is at the prompt.
        let host = capturedCursor
        grid = vt.withSnapshot { snapshot in
            guard let host else { return TerminalGrid(snapshot: snapshot) }
            return TerminalGrid(
                snapshot: snapshot, cursorRow: host.row, cursorColumn: host.column)
        }
        phase = .live
    }

    /// The URL under a cell of the screen as currently shown, or nil.
    ///
    /// Asked of the emulator rather than of the host: it holds the same bytes,
    /// and a round trip to answer a long press would arrive after the gesture.
    func url(atRow row: Int, column: Int) -> String? {
        vt?.url(atRow: row, column: column)
    }

    /// "resource not found" is the host's answer for a terminal that is not
    /// currently a live pane — restarted, stopped, never started. Everything
    /// else is a real failure, and the two must read differently: one is
    /// "come back later", the other is "something is wrong".
    private func report(_ error: Error) {
        let message = error.localizedDescription
        phase = message == "resource not found" ? .notLive : .failed(Self.humanFailure(message))
    }

    /// What actually goes under "Could not load".
    ///
    /// The core's word for a dead link is "not connected", which is right for a
    /// log and wrong for a screen: it is lowercase, it is a fragment, and it
    /// describes the FFI's session slot rather than anything the person holding
    /// the phone did or can do about it. It is also the line they saw — one ssh
    /// hiccup anywhere empties that slot and every call afterwards is answered
    /// with it, so this is the most common failure text there is, not an edge.
    ///
    /// Matched on the string rather than on `ClientCore.CoreError`, which the
    /// call path could have offered, because the OTHER caller could not: a
    /// stream reports its end as a bare JSON string with no type left on it
    /// (see `farcooler_client_stream_start`). One rule both paths can use beats
    /// a typed check here and an untyped one six lines away that drift apart.
    ///
    /// Everything else is passed through. A message from the host is the
    /// host's to word, and rewriting all of them into one apology would throw
    /// away the only clue a real failure carries.
    private static func humanFailure(_ message: String) -> String {
        message == "not connected"
            ? "The connection to this runner dropped. Reconnecting…"
            : message
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
            lines > 0 ? UInt32(FARCOOLER_VT_MOUSE_WHEEL_UP) : UInt32(FARCOOLER_VT_MOUSE_WHEEL_DOWN)

        var bytes: [UInt8] = []
        for _ in 0..<abs(lines) {
            guard
                let chunk = vt.encode(
                    mouse: button, action: UInt32(FARCOOLER_VT_MOUSE_PRESS),
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
    /// The escape sequences that put a fresh emulator into the modes the
    /// program is in. See `applyModes`.
    var modes: String?
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
    /// Already resolved for `FARCOOLER_VT_FLAG_INVERSE` — the core reports
    /// foreground and background separately from the flag, and a renderer
    /// that forgot to swap them would draw reverse-video text invisibly on
    /// itself.
    var foreground: Color
    var background: Color

    init(_ cell: FarCoolerVtCell) {
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

/// Colors the core did not resolve, because they belong to the phone's
/// screen, not to any program running on the host: the fill behind a short
/// last row and the cursor block. Values mirror the Mac app's so the same
/// terminal looks like the same terminal on both.
///
/// Read from the theme in force rather than fixed, and read on each access
/// rather than captured: these were `static let`s, which is what made the
/// palette unchangeable without relaunching.
@MainActor
enum TerminalPalette {
    static var backgroundPacked: UInt32 { Themes.shared.current.background }
    static var background: Color { Themes.shared.current.backgroundColor }
    static var cursor: Color { Themes.shared.current.cursorColor }
}
