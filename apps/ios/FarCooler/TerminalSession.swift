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
/// that says it cannot be shown. It is also what holds the screen while a
/// stream that failed is being re-attached — see `scheduleStreamRetry` — so
/// the two paths are not "the good one and the sad one" so much as the
/// picture and the stand-in that keeps the pane live until the picture is
/// back.
///
/// This never decides whether a terminal is "live" — the host does, the same
/// way `Connection` never computes a workspace's state. An unreadable screen
/// is reported exactly as the host phrased it.
@MainActor
final class TerminalSession: ObservableObject {
    enum Phase: Equatable {
        case connecting
        case notLive
        /// A sentence this app wrote, and — where it has none of its own — the
        /// host's answer to put under it. See `humanFailure(_:)`.
        case failed(String, transcript: String?)
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

    /// The pane's scrollback, as bytes ready to feed straight into an
    /// emulator: CRLF-repaired, colour-reset and terminated by the host, with
    /// the alternate-screen case already decided there. See `historyLines` for
    /// why this is held rather than re-fetched, and `render` for where it goes.
    ///
    /// Empty is the honest default and the common case — a pane that has
    /// printed less than a screenful has no history, and neither does one on
    /// the alternate screen.
    private var history: [UInt8] = []

    /// Where the view sits in the pane's history, republished with the grid.
    ///
    /// Read by `TerminalView` for the accessibility value, which is what lets a
    /// UI test assert that a swipe moved something — there is no other way to
    /// ask a `Canvas` what it is showing. It is also exactly what a scrollbar
    /// would need, if this ever grows one.
    @Published private(set) var scrollPosition = TerminalScrollPosition()

    /// An in-flight scrollback refresh, so two of them cannot race and the
    /// last one cannot outlive the pane.
    private var historyRefresh: Task<Void, Never>?

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
    /// Watches for a stream that has not painted anything yet — the short half
    /// of the same wait. See ``waitForTheFirstByte()``.
    private var firstPaint: Task<Void, Never>?
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
    /// The emulator the stream is going to fill, held aside until the stream
    /// actually says something.
    ///
    /// `prime` builds this — empty, at the pane's size — because the bytes
    /// start arriving the moment the channel opens and a chunk with no
    /// emulator to receive it is simply lost. It used to go straight into
    /// `vt`, which was right while a reopen was the only thing that ever
    /// re-attached: nothing else was painting, so an empty core on screen for
    /// one round trip was the same spinner the reopen already showed.
    ///
    /// It is wrong now that a re-attach runs WITH the poll loop still
    /// painting. `vt` is what `publish` draws and what a keystroke's
    /// `jumpToBottom` redraws, so installing an empty core into it mid-retry
    /// would blank a screen polling was keeping perfectly current — the exact
    /// thing keeping polling alive through the retries was for. So the
    /// stream's core waits here, and `consume` installs it at the instant the
    /// first byte makes the stream the painter.
    ///
    /// Its size travels with it because `render` moves `paneSize` on every
    /// poll. Without it, a pane somebody reshaped during the retry would leave
    /// `checkGeometry` comparing the pane's new size against a `paneSize` that
    /// already agrees, while the emulator just installed is still the old
    /// width — a mis-wrapped screen with nothing left to notice it.
    private var streamCore: (emulator: VTCore, columns: Int, rows: Int)?
    /// How long the next re-attach waits. Widened by `scheduleStreamRetry`,
    /// reset by the first byte through — see `consume`.
    private var streamRetryDelay: Double = TerminalSession.streamRetryFloor
    /// The re-attach that is waiting to happen, held so that an `open` from
    /// any other cause can cancel it rather than race it.
    private var streamRetry: Task<Void, Never>?
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

    /// How long a stream may stay silent before this screen gives up on it and
    /// polls instead. See ``waitForTheFirstByte()``.
    ///
    /// Measured against what the first byte is actually waiting for, which is
    /// not a round trip.
    ///
    /// **On a runner that advertises `terminal_stream` the cold process below is
    /// gone.** `open_stream` opens the second channel with `--stdio` and speaks
    /// `terminal.attach` on it, and `--stdio` finds a daemon already listening
    /// and becomes a pipe to it (`crates/daemon/src/main.rs`, `serve_stdio_session`)
    /// — no second `Service`, no second SQLite handle, no migrations, no second
    /// inventory refresh. What is left is a handshake, one round trip, and the
    /// four `tmux` captures: 67ms end to end over loopback ssh, measured in
    /// `crates/daemon/tests/a_forced_command_still_streams.rs`.
    ///
    /// The deadline stays where it is, because the paragraph below is still the
    /// worst case and this must survive the worst case: a runner too old for the
    /// method still gets the exec, and the pane with the most scrollback is
    /// still the one anybody opens.
    ///
    /// The exec, then. `farcoolerd --stream <id>` on the
    /// runner (`crates/client/src/session.rs`) is a cold process:
    /// it opens a `Service` — a second SQLite handle, migrations included — and
    /// refreshes the whole tmux inventory before it looks at this pane at all
    /// (`crates/daemon/src/main.rs:304`). Only then does the replay begin, and
    /// the replay's first act is four concurrent `tmux` subprocesses, the
    /// largest of which is `capture-pane -e -p -J -S - -E -1` — the pane's
    /// ENTIRE scrollback, escape sequences and all, up to
    /// `farcooler_vt::SCROLLBACK_LINES` of it (`crates/tmux/src/windows.rs:365`,
    /// `crates/daemon/src/runtime.rs:205`). Nothing reaches this device until
    /// all of that has finished and the result has crossed the link.
    ///
    /// Two seconds could not cover that, and the pane it could not cover is the
    /// agent that has been working all night — the one with the most scrollback
    /// to capture and the one anybody actually opens. Every one of those tripped
    /// the deadline on its FIRST open, when `phase` is still `.connecting` and
    /// the guard in `streamSaidNothing` therefore lets it through, and was
    /// handed to the poll loop while its stream was still perfectly healthy.
    ///
    /// That handover is not the cheap downgrade it reads as. A poll carries
    /// `capture-pane -e -p` — the visible screen and NO history
    /// (`crates/tmux/src/windows.rs:250`) — into a `VTCore` that `render`
    /// rebuilds from scratch on every changed capture. So a pane on the poll
    /// path has nothing for `scroll` to move through and could not hold a
    /// scrolled-back view for longer than one interval even if it had: a swipe
    /// on any program that has not claimed the mouse does nothing whatsoever,
    /// silently, forever. The daemon says the same thing from its own side of
    /// the wire, about the same missing scrollback, in
    /// `crates/daemon/src/runtime.rs:161` — "That is the bug, and it looked
    /// like a scroll bug."
    ///
    /// So the deadline is generous rather than eager, and deliberately so. It
    /// exists for a channel that is wedged, and a wedged channel is still wedged
    /// twelve seconds later; what it must never do is outrun a stream that is
    /// merely doing the work its first byte requires. The cost of firing late is
    /// a spinner held longer on a pane that was going to fail anyway. The cost
    /// of firing early is a working terminal that quietly stops scrolling.
    private static let firstByteDeadline: Duration = .seconds(12)

    /// How long a stream may stay silent before this screen paints SOMETHING —
    /// which is a different question from how long before it gives up, and
    /// used to be answered by the same number.
    ///
    /// `firstByteDeadline` was doing two jobs. One is "when do we conclude this
    /// channel is wedged and tear it down", and everything the constant above
    /// argues for that is still true: firing early swaps a working terminal for
    /// a poll loop that has no scrollback to scroll. The other is "how long does
    /// a person stare at a spinner", and twelve seconds is a catastrophic answer
    /// to it. Tying them together meant the cautious answer to the first
    /// question was charged to the second, on every pane, on every open.
    ///
    /// What made that stop being theoretical is measured in the comment on
    /// `attach`: on a device enrolled the way this product enrolls devices, the
    /// stream's first byte never comes at all. So every pane paid the full
    /// twelve seconds, every time, and then polled — which is the report this
    /// number exists to answer, "spins on Loading shell… for about fifteen
    /// seconds".
    ///
    /// Split, each answer can be right. Nothing is torn down here: the poll loop
    /// simply starts painting, exactly as it does through every re-attach, and
    /// `consume` hands the screen back the instant a byte arrives — with the
    /// stream's own replay and its scrollback intact. So a stream that is merely
    /// slow loses nothing but the capture this drew over, while a pane whose
    /// stream will never speak is live in a round trip instead of twelve
    /// seconds.
    ///
    /// 700ms is chosen against the healthy case rather than the broken one. A
    /// stream that works delivers its replay in about 60ms over loopback ssh —
    /// measured, see `attach` — so a working pane never reaches this at all and
    /// never sees the re-flow flash `open` refuses to show. It is far enough
    /// above that to leave room for a real network and a large scrollback, and
    /// short enough that a person reads it as the screen arriving rather than as
    /// waiting.
    private static let firstPaintGrace: Duration = .milliseconds(700)

    /// How long the first re-attach waits after a stream fails, and how long
    /// the longest one waits.
    ///
    /// A pane no longer settles for polling after three dead attaches — see
    /// `streamEnded` — so the interval is the only thing keeping the retries
    /// affordable, and the flat 500ms this used to sleep cannot do it alone:
    /// two attaches a second, per pane, on a link that is already failing.
    ///
    /// The floor stays at that 500ms, because the common failure is one
    /// dropped channel on a link that is otherwise fine, and half a second is
    /// the right answer to it. The ceiling is a different question, and it is
    /// deliberately NOT `slowInterval`. A poll is one `terminal.screen` RPC
    /// answered by a daemon already running. An attach on a runner too old to
    /// advertise `terminal_stream` execs `farcoolerd --stream` there: a cold
    /// process that opens a second SQLite handle with its migrations, refreshes
    /// the whole tmux inventory, and only then captures the pane's entire
    /// scrollback — the work `firstByteDeadline` allows twelve seconds for.
    /// Retrying that once a second would not be a retry, it would be a load
    /// generator aimed at a runner already having a bad time, and every attempt
    /// spends one of the ten ssh sessions a default sshd gives this whole phone.
    ///
    /// And a runner too old for the method is exactly the runner that retries
    /// forever: on one that has it, an enrolled device's attach now succeeds, so
    /// there is nothing to retry. This ceiling is what kept those doomed retries
    /// affordable while every enrolled device in the field was making them.
    ///
    /// Thirty seconds is therefore the ceiling. It is comfortably longer than
    /// one attach's own worst case, so attempts cannot overlap or queue behind
    /// each other; it is short enough that a link which comes back is
    /// streaming again — scrollback and all — within half a minute, with
    /// nobody tapping anything; and nothing is waiting on it meanwhile,
    /// because the poll loop is painting the whole time. The only thing a
    /// wider gap costs is scrollback the pane did not have a moment ago
    /// anyway.
    private static let streamRetryFloor: Double = 0.5
    private static let streamRetryCeiling: Double = 30.0

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

    /// How much scrollback a polled pane asks the host for.
    ///
    /// A poll carries `capture-pane -e -p` — the visible screen and no history
    /// (`crates/tmux/src/windows.rs:251`, against `capture_scrollback` at
    /// `:365`, which only the stream path calls). Fed into a fresh emulator
    /// that is exactly as tall as the screen, that leaves `history_size` at
    /// zero, and a core with no history is a core whose `scroll` moves
    /// nothing. Measured against the real `farcooler-vt`: a 24-line capture
    /// into an 80x24 terminal reports `history_size == 0`, and `scroll(5)`
    /// leaves `display_offset` at 0. That is the whole of "swiping the
    /// terminal does nothing".
    ///
    /// So the poll path asks for history explicitly. Not on every poll —
    /// history costs a second `capture-pane` on the runner and it is the same
    /// lines every time — but once when the pane opens, so the first swipe
    /// moves immediately rather than after a round trip, and again whenever
    /// the reader actually enters scrollback, so what they are reading is
    /// current rather than however stale the pane has grown since it opened.
    ///
    /// Two thousand rather than the emulator's full `SCROLLBACK_LINES` of
    /// 10,000: this crosses a phone's link base64'd, and at roughly 200 bytes
    /// a line the full history is a couple of megabytes to answer a swipe.
    /// Two thousand lines is far more than a thumb travels in one session and
    /// costs a few hundred kilobytes once per pane.
    private static let historyLines = 2_000

    private var interval: Double = TerminalSession.fastInterval

    init(terminalID: String, core: ClientCore) {
        self.terminalID = terminalID
        self.core = core
    }

    deinit {
        poller?.cancel()
        geometry?.cancel()
        streamRetry?.cancel()
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
        // The backoff belongs to the terminal being left, not the one
        // arriving. Carried over, a terminal whose predecessor had spent the
        // last ten minutes failing to stream would open and then wait half a
        // minute before its own first re-attach. When this was a strike count
        // instead, the same carry-over was worse: the arriving terminal
        // reached the three-strike fallback on its first hiccup and showed a
        // failure it had not earned, which is the error that flashed when
        // switching tabs.
        streamRetryDelay = Self.streamRetryFloor
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
    /// `ShellPaneTrack` mounts every visited pane and hides the ones you are not
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
        // A fresh visit deserves a fresh backoff: an interval an earlier
        // visit widened must not make this one wait half a minute for its
        // first re-attach.
        streamRetryDelay = Self.streamRetryFloor
        // AFTER `open`, which is not where the other openers put it, and has to
        // be here.
        //
        // `scheduleResize` compares `lastRequestedSize` against `lastResizeSent`
        // and does nothing when they agree — and on a return visit they always
        // agree at this instant, wrongly. `lastRequestedSize` still holds the
        // shape this phone asked for last time, `lastResizeSent` still holds the
        // shape it was told it got, and neither survived contact with what `stop`
        // did on the way out: `releasePane` handed the pane back to whatever the
        // Mac had it at. So the pane is genuinely 200×50 again, both of those
        // variables still say 45×40, and a `scheduleResize` run here decides
        // there is nothing to ask for.
        //
        // `prime` is what corrects it, by seeding `lastResizeSent` from the
        // host's own answer — and `prime` is inside `open`. Running the schedule
        // after it means the comparison is made against the pane's real shape
        // rather than against this device's memory of a shape it gave back.
        //
        // `configure`'s own `scheduleResize` cannot cover for this, because
        // `configure` only awaits `open` when IT is the one opening. `resume`
        // sets `started` synchronously, so the geometry task finds the pane
        // already open, skips the await, and schedules against the same stale
        // pair. That is why the size stuck: before `resume` existed, `configure`
        // opened the pane and its schedule therefore ran after `prime` — the
        // ordering this restores.
        Task {
            await open()
            scheduleResize()
        }
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
        streamRetryDelay = Self.streamRetryFloor
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
        // The shape this device imposed has just been handed back, so this
        // device has not reshaped the pane any more — and `prime` gates
        // recording a new `shapeBeforeUs` on exactly that. Left true, the next
        // visit reshapes the pane to phone width with nothing remembered to
        // restore, and whoever else is watching keeps the phone's column for
        // good. It only mattered once `resume` started re-asserting the size on
        // a return visit at all; before that, a returned-to pane was never
        // reshaped, so there was never anything to give back.
        hasResized = false
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
    /// sshd, after which no stream opens at all — see the awaited stop
    /// `scheduleStreamRetry` now makes before every wait.
    private func teardown() {
        poller?.cancel()
        poller = nil
        // The scrollback refresh goes with the loop it was lining up work
        // for. Left running it would answer into a pane nobody is looking at,
        // holding an ssh round trip open on a connection whose sessions are
        // the scarce thing here.
        historyRefresh?.cancel()
        historyRefresh = nil
        teardownExceptPolling()
    }

    /// Everything `teardown` stops except the poll loop.
    ///
    /// For the three callers that must not stop it: `streamEnded` and
    /// `streamSaidNothing`, which hand the screen to polling and then retry,
    /// and `open`, which every one of those retries goes through. Cancelling
    /// the poller there would freeze the pane on whatever the last capture
    /// painted for the length of the backoff — and with the retries no longer
    /// stopping at three, "the length of the backoff" has no end. A pane
    /// frozen on a stale screen that still looks alive is worse than the
    /// second-old one it replaced, which would have made this whole change a
    /// regression.
    ///
    /// The pending re-attach goes here rather than in `teardown`, for the same
    /// reason it is cancelled at all: an `open` in progress IS an attempt, and
    /// a timer that fires another one on top of it would fork the retry chain
    /// in two, then four. Every path back into `open` therefore arrives with
    /// at most one retry outstanding — its own, which it has just cancelled.
    private func teardownExceptPolling() {
        geometry?.cancel()
        geometry = nil
        silence?.cancel()
        silence = nil
        firstPaint?.cancel()
        firstPaint = nil
        streamRetry?.cancel()
        streamRetry = nil
        resizeDebounce?.cancel()
        resizeDebounce = nil
        pendingResizeSize = nil
        inbox = nil
        streamCore = nil
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
        // `teardown` minus the poll loop, which has to survive this call.
        // Every caller but one has already stopped the poller, or never
        // started it; the exception is the re-attach in `scheduleStreamRetry`,
        // which arrives here with polling actively painting the pane and needs
        // it to keep painting until a stream is genuinely carrying bytes
        // again. Handing the screen back is `consume`'s job, on the first
        // byte, so that at no instant do both paint.
        teardownExceptPolling()
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
            if case .failed = phase {
                startLoop()
                // And ask for the stream again on the widening interval. A
                // screen call that failed says nothing about whether this link
                // can carry a stream — it usually means there is no link at
                // all this second — so giving up on streaming because of one
                // would be settling for polling by a side door, which is the
                // thing this pane must never do.
                scheduleStreamRetry()
            } else {
                // And a `.notLive` pane stops being polled at all. Said out
                // loud here because this method no longer stops the poller on
                // the way in: a loop started by a retry would otherwise
                // outlive the pane it was covering for, which is the round
                // trip a second the paragraph above refuses to spend.
                poller?.cancel()
                poller = nil
            }
            return
        }
        // Re-checked here, after every await above, and not only in
        // `reattach`. `stop()` can land while this call is suspended in
        // `stopStream` or `prime` — the view disappeared, the tab changed —
        // and the guard `reattach` passed a moment ago says nothing about
        // that. Attaching anyway opens an ssh channel for a pane nobody is
        // looking at, AFTER `stop`'s own `stopStream` has already gone, so
        // nothing releases it until the pane is opened again.
        //
        // A default sshd gives the whole phone ten sessions across every pane
        // on that runner, and this file's history is largely the story of
        // running out of them. The hazard predates the retry loop, which had
        // one flat attempt and no cancellation at all; unbounded retries do
        // not create it but do make it reachable far more often, which is
        // reason enough to close it now.
        guard started else { return }
        if await attach() {
            watchGeometry()
            return waitForTheFirstByte()
        }
        // Nothing opened, so the core `prime` just built has nothing to fill
        // it, and the poll path builds its own.
        streamCore = nil
        // Painted only when nothing else is painting. On a retry a poll loop
        // is already running, and this screen and its next answer were in
        // flight together — so drawing this one now is as likely to put an
        // older capture over a newer one as it is to help, for a pane that is
        // already repainting itself several times a second.
        if poller == nil { render(screen) }
        startLoop()
        // `startStream` answers false when there is no ssh session to open a
        // second channel on — and a session comes back without anything here
        // being told. `relink` covers the case where the link is replaced
        // while this pane is on screen; this covers the rest, for the price of
        // one attempt every thirty seconds against a connection that is
        // already answering polls.
        scheduleStreamRetry()
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
    /// How long it waits is `firstByteDeadline`, and that number is the whole
    /// difference between this being a safety net and being the thing that
    /// breaks scrolling — the constant says why. It costs nothing when it does
    /// not fire, and the first byte cancels it: see `consume`.
    ///
    /// Two waits, not one, and the shorter one is the one a person feels. The
    /// deadline above decides whether this CHANNEL is dead; `firstPaintGrace`
    /// decides only whether this SCREEN has waited long enough to be shown
    /// something, and answers it by starting the poll loop rather than by
    /// tearing anything down. Both are cancelled by the first byte, and both go
    /// with `teardownExceptPolling`. See `firstPaintGrace` for why they were one
    /// number and must not be.
    private func waitForTheFirstByte() {
        silence?.cancel()
        silence = Task { [weak self] in
            try? await Task.sleep(for: Self.firstByteDeadline)
            guard !Task.isCancelled else { return }
            await self?.streamSaidNothing()
        }
        firstPaint?.cancel()
        firstPaint = Task { [weak self] in
            try? await Task.sleep(for: Self.firstPaintGrace)
            guard !Task.isCancelled else { return }
            await self?.streamHasNotPaintedYet()
        }
    }

    /// The grace passed with the screen still blank, and the stream still open.
    ///
    /// Deliberately the smallest possible response: start polling, change
    /// nothing else. The stream keeps its channel, keeps its emulator waiting in
    /// `streamCore`, and keeps its twelve seconds to prove itself — this is not
    /// a downgrade and must not read as one. It is the same arrangement every
    /// re-attach already runs in, where polling paints while a stream opens
    /// behind it and `consume` performs the handover on the first byte; the only
    /// pane that never got it was the one being opened for the first time, which
    /// is the pane a person is actually watching.
    ///
    /// `render` will take `phase` to `.live` a round trip from now, which is why
    /// `streamSaidNothing` can no longer ask `phase` whether a byte has arrived.
    private func streamHasNotPaintedYet() {
        guard streaming, streamCore != nil else { return }
        startLoop()
    }

    /// The deadline passed with the screen still blank.
    ///
    /// This used to be a permanent surrender, and said so: "this path goes to
    /// polling and never reopens." That was the second cliff, and the quieter
    /// one — `streamEnded`'s three strikes at least needed three channels to
    /// drop, while a single channel that opened and then sat there stranded
    /// the pane on the poll loop on the first try. On a wedged link it is also
    /// the likelier of the two, because a wedged channel does not drop, it
    /// hangs.
    ///
    /// So it now does exactly what `streamEnded` does: hand the screen to
    /// polling, and try the stream again on the widening interval. Its
    /// fire-and-forget stop went with the reasoning that justified it — see
    /// `scheduleStreamRetry`, which stops the channel awaited, because there
    /// is now a start for a late stop to arrive after and cancel.
    private func streamSaidNothing() {
        // Only while this STREAM has said nothing. A stream that delivered and
        // then went quiet is an idle pane, which is the ordinary state of most
        // of them, and tearing that down would swap a working stream for polling
        // every twelve seconds of quiet.
        //
        // Asked of `streamCore` rather than of `phase`, which is what this used
        // to read and can no longer: `firstPaintGrace` puts the poll loop on the
        // screen a round trip in, so by the time this fires `phase` is `.live`
        // on a pane whose stream is as silent as it ever was — and the guard
        // would bail, leaving a wedged channel open and never retried. The
        // emulator `prime` built and `consume` installs on the first byte is the
        // exact fact wanted: still waiting means no byte has arrived.
        guard streaming, streamCore != nil else { return }
        teardownExceptPolling()
        startLoop()
        scheduleStreamRetry()
    }

    /// Hand the channel back, wait, and attach again.
    ///
    /// The wait widens geometrically, in the same shape and on the same
    /// `backoffFactor` the poll loop coasts on; only the ceiling differs, and
    /// it differs by a factor of thirty for the reasons `streamRetryCeiling`
    /// gives. From `streamRetryFloor` that is nine widenings and something
    /// under a minute and a half of wall clock to reach it, which is the shape
    /// wanted: a link that hiccuped is streaming again within a second, and a
    /// link that is genuinely down is asked twice a minute instead of twice a
    /// second.
    ///
    /// The stop happens BEFORE the wait, and is awaited. Before, because a
    /// channel that just failed is the likeliest one to be wedged, and holding
    /// it for the length of a backoff that now reaches half a minute is how
    /// the next pane finds no channels left — ten is all a default sshd gives
    /// this phone for every pane it has open. Awaited, because stop and start
    /// go through the same actor in the order they are asked, and a stop left
    /// in flight arrives after the next start and cancels the stream that
    /// start just opened: a session that believes it is streaming, attached to
    /// nothing, which is a screen that stops updating and never says why.
    /// `open` awaits its own stop for precisely this reason, and this stop is
    /// no different now that both failure paths reopen.
    private func scheduleStreamRetry() {
        let id = terminalID
        let delay = streamRetryDelay
        streamRetryDelay = min(streamRetryDelay * Self.backoffFactor, Self.streamRetryCeiling)
        streamRetry?.cancel()
        streamRetry = Task { [weak self] in
            await self?.stopStream(id)
            try? await Task.sleep(for: .seconds(delay))
            guard !Task.isCancelled else { return }
            await self?.reattach()
        }
    }

    /// The scheduled re-attach, arriving. The handle is dropped first because
    /// this retry is no longer pending — it is the one running — and `open`
    /// cancels whatever is pending on its way in.
    ///
    /// `started` is checked because the cancel and the wake-up can cross: a
    /// retry that was already resuming when `stop` cancelled it would open an
    /// ssh channel for a pane nobody is looking at any more, and the stop that
    /// `stop` itself made has already been and gone. That channel is one of
    /// ten and nothing would close it until this object died. Every other
    /// caller of `open` sets `started` first, or is only reachable while it is
    /// already true, so this costs them nothing.
    private func reattach() async {
        streamRetry = nil
        guard started else { return }
        await open()
    }

    /// One screen: the pane's size, whether it is running at all, and — only
    /// if nothing better is coming — something to draw.
    private func prime() async -> ScreenResponse? {
        do {
            // History is asked for HERE and not in `poll`, which is the whole
            // cost discipline: once per pane rather than ten times a second.
            // See `historyLines`.
            let data = try await core.call(
                "terminal.screen",
                ["terminal": terminalID, "historyLines": Self.historyLines])
            let response = try JSONDecoder().decode(ScreenResponse.self, from: data)
            revision = response.revision
            history = Self.decodedHistory(response)
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
            // Installed as the screen's emulator only when nothing else is
            // painting. When a poll loop is — which is every re-attach, and
            // was never possible before the retries stopped cancelling it —
            // putting an empty core into `vt` would blank a pane that polling
            // is keeping current, the moment anything called `publish`. It
            // waits in `streamCore` instead, and `consume` installs it on the
            // first byte. See `streamCore`.
            streamCore = (emulator, response.columns, response.rows)
            if poller == nil { vt = emulator }
            return response
        } catch {
            report(error)
            return nil
        }
    }

    /// Open the byte stream, and say whether it opened.
    ///
    /// **True here still does not mean bytes are coming**, and the reason it
    /// used to mean the opposite is worth keeping. The core opened the stream by
    /// exec'ing `~/.local/bin/farcoolerd --stream <id>` on a second ssh channel,
    /// while every key this product enrolls is written by `crates/fence` as
    /// `restrict,command="~/.local/bin/farcoolerd --stdio --client … --scope …"`
    /// — and a forced command is OpenSSH's promise that the client's own command
    /// is discarded, the very promise
    /// `crates/daemon/tests/a_real_sshd_forces_the_scope.rs` exists to pin. So
    /// what ran on the stream channel was a second `--stdio` protocol relay, and
    /// a protocol server says nothing until it is spoken to: the channel opened,
    /// this returned true, and no byte ever arrived. On a loopback sshd with the
    /// same daemon and the same pane, the only difference being the
    /// `authorized_keys` line, a plain key reached its first replayed byte in
    /// 62ms and the line this product writes reached nothing in 25 seconds.
    ///
    /// The stream is a wire method now. `crates/client`'s `open_stream` opens
    /// the second channel with `--stdio` — the one command BOTH key types run —
    /// and speaks `terminal.attach` on it, so which line this device holds is no
    /// longer something the stream depends on. Measured the same way, on the
    /// same harness (`a_forced_command_still_streams.rs`): 67ms on the forced
    /// command against 64ms on a plain key.
    ///
    /// What is left is the ordinary reason a stream can open and stay quiet — a
    /// runner too old to advertise `terminal_stream`, where the client still
    /// execs `--stream` and a forced command still swallows it. That is what
    /// `firstPaintGrace` and the poll fallback below are for, and they are not
    /// dead code: an app ships ahead of the runners it talks to.
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

    /// Feed everything that has arrived, in order, and redraw once — and, on
    /// the first byte, take the screen back from the poll loop.
    private func consume() {
        guard let inbox else { return }
        let bytes = inbox.take()
        guard !bytes.isEmpty else { return }
        // The stream spoke, so the deadline on its silence is spent. Cancelled
        // rather than left to fire and find `phase` already `.live`, because a
        // task sleeping for two seconds per pane per open is a real cost on a
        // tab strip, and because the guard it would rely on is a second place
        // to get this right.
        silence?.cancel()
        silence = nil
        // And the grace, for the same reason and one more: it starts a poll
        // loop, and starting one after the stream has taken the screen is the
        // two painters this method exists to keep down to one.
        firstPaint?.cancel()
        firstPaint = nil
        // And so is the interval the failures before it had widened. A stream
        // that delivered is evidence about this link that the tally of what
        // came before it is not, so the next failure starts over at half a
        // second rather than wherever this pane's history had crept to.
        streamRetryDelay = Self.streamRetryFloor
        // The handover, and the whole reason it is here. Until this byte,
        // polling owned the screen — through every retry, which is what keeps
        // a pane live instead of frozen on whatever `prime` last painted. From
        // this byte on the stream owns it, and only ever one of them may:
        // `streamEnded` records what both at once looked like, "two painters,
        // disagreeing, one of them every second".
        //
        // It is a swap and not merely a cancel, because the two paths do not
        // share an emulator. `render` builds a fresh `VTCore` for every
        // capture on purpose, so the core `vt` holds at this instant is one a
        // poll built and the next poll would have thrown away. These bytes are
        // a continuation of the screen the stream's own replay draws, so they
        // belong in the core `prime` built for them — and both halves happen
        // here, in one turn on the main actor, with nothing published in
        // between, so no frame is ever drawn from half of each.
        //
        // The poller is cancelled first, but a cancel cannot recall a round
        // trip already asked for: `poll` checks for its own cancellation
        // before it paints, which is the other half of this.
        if let streamCore {
            poller?.cancel()
            poller = nil
            vt = streamCore.emulator
            paneSize = (streamCore.columns, streamCore.rows)
            self.streamCore = nil
        }
        guard let vt else { return }
        vt.feed(bytes)
        // From here on the emulator's own caret is the true one: these bytes
        // are a continuation of the screen they move the caret across, unlike
        // a capture. See `capturedCursor`.
        capturedCursor = nil
        publish()
    }

    /// The stream stopped. Hand the screen to polling, and try again.
    ///
    /// A stream ends for two very different reasons: the pane finished, or the
    /// channel did. Only the host can tell those apart, so this asks it — by
    /// reopening, which begins with the screen call that reports a pane that
    /// is no longer running, and which stops the retries for good when it says
    /// so. See the `.notLive` branch in `open`.
    ///
    /// It used to stop asking after three dead attaches and settle for polling
    /// permanently. The reason was channel exhaustion, and it was a real one:
    /// every attach spends one of the ten sessions a default sshd allows, and
    /// the fallback used to tear down this side while leaving the stream
    /// running on the host, so a handful of panes doing it exhausted the
    /// budget and then nothing could stream at all. That leak is fixed —
    /// `scheduleStreamRetry` releases the channel, awaited, before it waits,
    /// and `open` stops the stream again before it starts one. Retries release
    /// before they re-acquire, so they cannot accumulate.
    ///
    /// What the cliff cost outlived what it bought. A pane on the polling path
    /// has no scrollback whatsoever — a poll carries `capture-pane -e -p`, the
    /// visible screen and no history — so swiping it does nothing at all,
    /// silently, on a pane that repaints every second and looks perfectly
    /// alive. Three unlucky seconds put a pane in that state until somebody
    /// happened to switch tabs and back.
    ///
    /// The error the stream ended with is deliberately not shown. It was worth
    /// showing at the cliff, as the last thing that pane would ever say about
    /// streaming; now that polling is painting and the stream is coming back
    /// on its own, "Could not load" over a screen that is visibly working
    /// would simply be untrue. A failure polling can see for itself is still
    /// reported, by `poll`.
    private func streamEnded(_ error: String?) {
        guard streaming else { return }
        // Everything the streaming path set up goes before the polling path
        // starts, and that is the whole fix for a screen that flashed a wrong
        // layout once a second. Falling back used to start the poll loop and
        // leave the rest running, so a stream that was still delivering fed the
        // emulator correct bytes while the poll repainted a capture over the
        // top of them — two painters, disagreeing, one of them every second. A
        // fallback has to be a handover, not an addition — and now that the
        // fallback is temporary, so does the return trip: `consume` cancels
        // the poller in the same turn it installs the stream's emulator.
        teardownExceptPolling()
        startLoop()
        scheduleStreamRetry()
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
    ///
    /// The handle is cleared as well as cancelled, because `startLoop` now
    /// refuses to start a second loop over a live one and reads that handle to
    /// decide. The cadence reset moved in there with it.
    private func wake() {
        poller?.cancel()
        poller = nil
        startLoop()
    }

    /// Start the poll loop, unless one is already running.
    ///
    /// Idempotent because the callers now overlap. `open` starts a loop when
    /// it cannot stream, both stream failures start one before they retry, and
    /// every retry goes back through `open` — so on a pane that keeps failing,
    /// this is called once per attempt with a loop already painting. Two loops
    /// on one pane would be two captures per interval and two painters of the
    /// same kind, which is merely wasteful rather than wrong; what makes it
    /// unacceptable is that it would be one more loop per retry, without end.
    ///
    /// The cadence is reset here rather than at each call site, because every
    /// reason to start a loop is a reason to poll now: polling begins when
    /// something just changed, and whatever interval an earlier loop had
    /// backed off to described a screen that is no longer the one being shown.
    private func startLoop() {
        guard poller == nil else { return }
        interval = Self.fastInterval
        poller = Task { [weak self] in
            while let self, !Task.isCancelled {
                await self.poll()
                guard !Task.isCancelled else { return }
                try? await Task.sleep(for: .seconds(self.interval))
            }
        }
    }

    private func poll() async {
        // A pane the reader has scrolled back is a pane they are READING, and
        // the loop's job while they do is to leave it alone.
        //
        // `render` preserves the offset across a rebuild, so this is not what
        // keeps the view from snapping to the bottom. What it prevents is
        // subtler and worse: the offset is a count of lines from the live
        // screen, so every line the program prints while somebody is reading
        // history slides the text they are reading up by one. Ten lines of
        // output and the paragraph they were halfway through is gone. Holding
        // the capture still holds the words still.
        //
        // Nothing is lost by waiting. Returning to the bottom wakes the loop
        // (see `resumeAfterScrollback`), and the first thing it does is ask
        // for a screen — so the pane is current within one round trip of the
        // moment anybody wants it to be.
        guard (vt?.scrollPosition.offset ?? 0) == 0 else { return }
        do {
            let data = try await core.call("terminal.screen", ["terminal": terminalID])
            // The stream may have taken the screen while this call was in
            // flight. `consume` cancels this loop the instant the first byte
            // lands, but a cancel cannot recall an ssh round trip already
            // asked for, and what comes back is a capture: painting it now
            // would draw a whole re-flowed screen over the bytes the stream
            // has just started delivering. That is the two painters again, in
            // the one window left where they can still overlap. Checked here
            // rather than only in the loop above, because that check happens
            // after this method has already painted.
            guard !Task.isCancelled else { return }
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
            // A failure that arrived after the stream took over is not this
            // screen's news to report: `.failed` over a pane the stream is now
            // painting would be the same lie from the other direction.
            guard !Task.isCancelled else { return }
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
            // This app's own finding, not the host's: the base64 did not
            // decode here. There is no transcript, because nothing said
            // anything — the bytes simply were not what they claimed to be.
            phase = .failed(
                "The host sent a screen this device could not decode.", transcript: nil)
            return
        }
        // Where the reader was looking, so rebuilding does not yank them back
        // to the bottom.
        //
        // This method builds a WHOLE NEW core on every changed capture, which
        // is fine while the view is live and destroys a scrolled-back view
        // otherwise: a polled pane repaints several times a second, so without
        // this a swipe would move the screen and the next poll would move it
        // straight back. Reapplied after the feed below, once there is history
        // to apply it to.
        let wasScrolledBackTo = vt?.scrollPosition.offset ?? 0

        let emulator = VTCore(columns: response.columns, rows: response.rows)
        // A fresh core starts on the VT crate's own default palette, not the
        // theme in force. Without this the chrome would be themed and every
        // character would not.
        emulator.setPalette(Themes.shared.current.packed)
        applyModes(response.modes, to: emulator)
        // The scrollback, before the screen and separated from it by a clear.
        //
        // This is the same order, and for the same reasons, that the daemon's
        // own `replay` writes down a stream — see `replay` in
        // `crates/daemon/src/runtime.rs`. The host has already done the parts
        // that need tmux to answer for them: repairing bare line feeds,
        // resetting the colour the last history line left set, and deciding
        // that an alternate screen has no history worth sending.
        //
        // What is left here is the clear, and what it buys is ALIGNMENT, not
        // history. This comment used to say the clear is what pushes the
        // history lines up into scrollback; measured against the real
        // emulator, that is not true — feeding lines past the bottom of the
        // region scrolls them into history on its own, and a full-height
        // capture reports the same 100 lines of history and the same top row
        // either way.
        //
        // It earns its place on a capture SHORTER than the screen, which tmux
        // makes the common case by trimming trailing blanks: a quiet pane
        // answers with two lines, not twenty-four. Fed without the clear,
        // those two lines land at the bottom of whatever the history left on
        // screen — 40 lines of history collapse to 18, and the top row reads
        // `history line 18` instead of the screen. With it: 40, and the screen
        // where it belongs.
        if !history.isEmpty {
            emulator.feed(history)
            emulator.feed(Array("\u{1b}[H\u{1b}[2J".utf8))
        }
        // A capture separates its lines with a bare line feed, which to a
        // terminal means "down one row" and nothing about which column. Fed
        // straight in, every line starts where the previous one ended and the
        // screen arrives as a staircase — text broken mid-word at a different
        // place on each row, which is what a wrong screen looked like here.
        // The daemon repairs this for the replay it sends down a stream, for
        // exactly the same reason; a capture arriving by any other route needs
        // the same repair. Live pty bytes never do — a pty already emits both.
        emulator.feed(carriageReturned(withoutTrailingNewlines([UInt8](bytes))))
        if wasScrolledBackTo > 0 {
            emulator.scroll(lines: Int32(wasScrolledBackTo))
        }
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
        scrollPosition = TerminalScrollPosition(
            emulator.scrollPosition, streaming: streaming, wantsMouse: emulator.wantsMouse)
        grid = emulator.withSnapshot { snapshot in
            // The host's caret only means anything on a view that is showing
            // the host's screen. Scrolled back, the rows on screen are history
            // and the prompt is somewhere below them, so drawing the host's
            // row and column would put a caret in the middle of text nobody is
            // typing into. The emulator knows where the caret really sits
            // relative to the view; `grid.rs` reports it off-screen and this
            // clamps it, which is the honest answer.
            guard snapshot.displayOffset == 0 else { return TerminalGrid(snapshot: snapshot) }
            return TerminalGrid(
                snapshot: snapshot,
                cursorRow: response.cursorRow,
                cursorColumn: response.cursorColumn)
        }
        phase = .live
    }

    /// The scrollback out of a screen response, or nothing.
    ///
    /// Nothing is the ordinary answer and not a failure: a pane that has
    /// printed less than a screenful has no history, one on the alternate
    /// screen has none the host will send, and a runner too old to know the
    /// field at all simply omits it — which is exactly the behaviour that
    /// keeps this working against a daemon that has not been updated.
    private static func decodedHistory(_ response: ScreenResponse) -> [UInt8] {
        guard
            let encoded = response.history, !encoded.isEmpty,
            let bytes = Data(base64Encoded: encoded)
        else { return [] }
        return [UInt8](bytes)
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
        scrollPosition = TerminalScrollPosition(
            vt.scrollPosition, streaming: streaming, wantsMouse: vt.wantsMouse)
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
        phase = message == "resource not found" ? .notLive : Self.humanFailure(message)
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
    /// Everything else is KEPT, whole. A message from the host is the host's
    /// to word, and rewriting all of them into one apology would throw away
    /// the only clue a real failure carries.
    ///
    /// It is no longer kept in the sentence's place, though. Passing it through
    /// as the phase's only string put the host's fragment under a headline this
    /// app wrote, in the app's own face, with nothing to mark where Far Cooler
    /// stopped speaking — see `TerminalView.phaseContent`, which now draws the
    /// sentence as prose and the transcript in a `DetailBox`. The Mac's
    /// `ChangesPane` made the same move for the same string.
    ///
    /// The sentence says only what this side knows: the read did not finish. No
    /// cause is named, because from here the cause is unknowable and a guess
    /// sends somebody to change a setting that was never the problem — see
    /// `Enrollment.note(about:outcome:)` — and no retry is promised, because
    /// nothing here performs one.
    private static func humanFailure(_ message: String) -> Phase {
        message == "not connected"
            ? .failed("The connection to this runner dropped. Reconnecting…", transcript: nil)
            : .failed("The request that reads this pane didn’t finish.", transcript: message)
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
    /// first — the same rule the Mac's `keyDown` follows.
    ///
    /// It DOES read the offset first, where it used to say there was no point.
    /// That was true while returning to the bottom was only a redraw; it stopped
    /// being true when a scrolled-back pane started holding its poll loop
    /// still. Typing is one of the two ways back to the live screen — the other
    /// is swiping down, in `scrollLocally` — and a pane that resumed painting
    /// only when swiped would go dead under a reader who scrolled up, gave up,
    /// and typed instead.
    private func jumpToBottom(_ vt: VTCore) {
        let wasScrolledBack = vt.scrollPosition.offset > 0
        vt.scrollToBottom()
        publish()
        if wasScrolledBack { resumeAfterScrollback() }
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

        // Who gets the wheel: the program, or this device's own view.
        //
        // The old rule was "whoever will take it" — if the core could encode a
        // wheel event, the program had mouse reporting on and the bytes went
        // out. That is wrong on the primary screen, and it is the reason
        // "scroll is broken for terminals" was reported from a real phone
        // while every scroll test stayed green.
        //
        // A full-screen TUI turns mouse reporting ON and then decides for
        // itself what a wheel means. Several do nothing visible with it. The
        // swipe left, the program ignored it, and the pane sat still — with no
        // other scroll affordance on a phone, that pane simply could not be
        // scrolled. No test could see it, because the demo runner's idle bash
        // prompt has mouse reporting OFF and so only ever took the local path.
        // The symptom is intermittent by nature: it depends entirely on which
        // program happens to be running in the pane you are looking at.
        //
        // The rule is now about WHAT IS BEHIND the screen, not about who is
        // willing to take the bytes. On the alternate screen there is no
        // scrollback to reach, so the wheel can only mean something to the
        // program. On the primary screen there is history, reaching it is what
        // a swipe means on a phone, and it wins — even from a program that
        // asked for the mouse.
        guard vt.isAlternateScreen else {
            await scrollLocally(vt, lines: lines)
            return
        }

        let button: UInt32 =
            lines > 0 ? UInt32(FARCOOLER_VT_MOUSE_WHEEL_UP) : UInt32(FARCOOLER_VT_MOUSE_WHEEL_DOWN)

        var bytes: [UInt8] = []
        for _ in 0..<abs(lines) {
            guard
                let chunk = vt.encode(
                    mouse: button, action: UInt32(FARCOOLER_VT_MOUSE_PRESS),
                    column: column, row: row, modifiers: [])
            else {
                await scrollLocally(vt, lines: lines)
                return
            }
            bytes.append(contentsOf: chunk)
        }
        await write(bytes)
    }

    /// Move this device's own view of the pane, and keep the poll loop in step
    /// with where the reader now is.
    ///
    /// Reached whenever the pane is on the primary screen, which is where the
    /// scrollback lives — whether or not the program asked for the mouse. See
    /// `scroll` for why that is the rule.
    private func scrollLocally(_ vt: VTCore, lines: Int) async {
        // Read BEFORE the scroll, because "we are at the bottom" and "we have
        // just arrived at the bottom" are different questions and only the
        // second one is worth a round trip.
        //
        // A drag reports every line it crosses, and the core clamps at zero.
        // So a downward swipe on a view that is already live reports a dozen
        // times, arrives at zero a dozen times, and without this fires a
        // scrollback refresh for each of them — a dozen ssh round trips for a
        // gesture that changed nothing.
        let wasScrolledBack = vt.scrollPosition.offset > 0
        vt.scroll(lines: Int32(lines))
        publish()
        // Back at the live screen. Start painting again, and take the chance
        // to refresh the scrollback while sitting somewhere a rebuild cannot
        // be seen.
        if vt.scrollPosition.offset == 0 {
            if wasScrolledBack { resumeAfterScrollback() }
            return
        }
        // Scrolled back, so stop painting. `poll` refuses to run in this state
        // as well; this stops the loop rather than leaving it spinning through
        // a guard several times a second.
        poller?.cancel()
        poller = nil
    }

    /// Resume the poll loop, and line up fresh scrollback for the next time
    /// somebody swipes.
    ///
    /// The refresh happens HERE, at the bottom, rather than on the way INTO
    /// scrollback, and the reason is that `display_offset` counts lines from
    /// the live screen. Replacing the history under a reader who is already
    /// twenty lines up moves those twenty lines somewhere else — they would
    /// watch the paragraph they were reading jump. At the bottom there is
    /// nothing to move: the screen is the same screen, and the only difference
    /// is that there is now more above it than there was.
    ///
    /// The cost is bounded and worth naming: within one continuous visit to
    /// the scrollback, anything that scrolled off since the reader was last at
    /// the bottom is not there. Nothing scrolls off while they are up there —
    /// this pane has stopped polling — so the gap can only be as old as their
    /// last look at the live screen.
    private func resumeAfterScrollback() {
        guard started, !streaming else { return }
        startLoop()
        historyRefresh?.cancel()
        historyRefresh = Task { [weak self] in await self?.refreshHistory() }
    }

    /// Ask for the pane's scrollback again and keep it for the next swipe.
    ///
    /// Deliberately does not paint. A screen arrives with the answer and
    /// drawing it here would put a second painter beside the poll loop, racing
    /// it to show whichever capture happened to be older. The loop is running
    /// by the time this lands and will fold the new history in on its next
    /// pass, which is at most `fastInterval` away.
    private func refreshHistory() async {
        guard
            let data = try? await core.call(
                "terminal.screen",
                ["terminal": terminalID, "historyLines": Self.historyLines]),
            let response = try? JSONDecoder().decode(ScreenResponse.self, from: data),
            !Task.isCancelled
        else { return }
        // Only while the reader is still at the bottom. A swipe can land
        // during the round trip, and the paragraph above is exactly why that
        // history must not be swapped in underneath it.
        guard (vt?.scrollPosition.offset ?? 0) == 0 else { return }
        history = Self.decodedHistory(response)
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
    /// The pane's scrollback, base64, ready to feed. Present only when it was
    /// asked for — see `historyLines` — and empty for a pane that has none or
    /// is on the alternate screen, which has no history of its own.
    var history: String?
}

/// How far back the view is scrolled, and how far back it COULD be.
///
/// `history == 0` is the whole scrolling bug in one number: a pane with no
/// history has nothing a swipe can move to, however well the gesture works.
///
/// Named for the terminal rather than plain `ScrollPosition`, which is
/// SwiftUI's own type — `AgentView` uses the real one and a shadowing struct
/// here broke it at a distance.
struct TerminalScrollPosition: Equatable {
    var offset = 0
    var history = 0
    /// Which of the two painters is feeding this pane: the byte stream, or the
    /// capture poll.
    ///
    /// Carried here because it is otherwise unobservable from outside, and the
    /// two look identical on a screen — a polled pane repaints several times a
    /// second and is perfectly readable. That is exactly what let streaming be
    /// broken on every enrolled device for as long as it was: nothing anybody
    /// could see said which path a pane was on.
    var streaming = false

    /// Whether the program in this pane has asked for mouse events.
    ///
    /// Published for the same reason `streaming` is: it is invisible from
    /// outside and it is exactly the axis a regression hid along. A pane whose
    /// program wants the mouse and one whose program does not are pixel-for-
    /// pixel identical, and `scroll` used to treat them completely differently
    /// — so the suite could reach only the harmless case and stayed green while
    /// a phone could not scroll at all. A test can now ask for the other one.
    var wantsMouse = false

    init() {}

    init(_ position: (offset: Int, history: Int), streaming: Bool, wantsMouse: Bool) {
        offset = position.offset
        history = position.history
        self.streaming = streaming
        self.wantsMouse = wantsMouse
    }
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
