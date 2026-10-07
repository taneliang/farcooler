import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The chat scrolls like Messages (ov-383): the real `AgentSurface` in an
/// offscreen window, fed by a stand-in CLI, read through `AgentScrollProbe`.
///
/// - Pinned, the tail stays in view above the composer as content arrives,
///   with the composer's height and a row's spacing between them.
/// - Content runs under the composer.
/// - Scrolled up, nothing moves, and Jump to Latest is offered.
/// - A sent message comes into view wherever the reader was.
/// - The composer growing keeps the tail in view.
/// - Jump to Latest goes back.
@MainActor
@Suite(.serialized)
struct AgentScrollTests {
    /// A stand-in `farcooler`: `agent-subscribe` prints `first`, then, with
    /// `--follow`, every line later appended to `more`; anything else, such
    /// as `agent-prompt`, answers `{}`.
    struct StandIn {
        let binary: String
        let more: URL

        init(first: Data) throws {
            let dir = FileManager.default.temporaryDirectory.appendingPathComponent("agent-scroll-\(UUID().uuidString)")
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            let firstFile = dir.appendingPathComponent("first.jsonl")
            try (first + Data("\n".utf8)).write(to: firstFile)
            more = dir.appendingPathComponent("more.jsonl")
            try Data().write(to: more)
            let script = dir.appendingPathComponent("farcooler")
            try """
                #!/bin/sh
                case "$*" in
                  *agent-subscribe*)
                    cat '\(firstFile.path)'
                    case "$*" in *--follow*) exec tail -n +1 -f '\(more.path)';; esac;;
                  *) echo '{}';;
                esac
                """.write(to: script, atomically: true, encoding: .utf8)
            try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: script.path)
            binary = script.path
        }

        func stream(_ lines: [Data]) throws {
            let handle = try FileHandle(forWritingTo: more)
            defer { try? handle.close() }
            try handle.seekToEnd()
            for line in lines { try handle.write(contentsOf: line + Data("\n".utf8)) }
        }
    }

    static func payload(_ event: [String: Any]) -> String {
        String(decoding: try! JSONSerialization.data(withJSONObject: event), as: UTF8.self)
    }

    static func message(_ role: String, _ text: String) -> String {
        payload(["Message": ["role": role, "text": text, "parent": NSNull()]])
    }

    static func batch(_ payloads: [String], from seq: Int) -> Data {
        let events = payloads.enumerated().map { ["seq": seq + $0.offset, "payloadJson": $0.element] as [String: Any] }
        return try! JSONSerialization.data(withJSONObject: ["epoch": 1, "events": events])
    }

    static let sentence = "The agent explains what it did and why, at some length, so a turn takes a few lines. "

    /// A terminal of its own, so the composer starts from no draft: drafts
    /// are kept per pane across runs (`PaneDraftStore`).
    static func terminal(_ id: String, activity: String = "idle") throws -> Terminal {
        let json = #"""
            {"id":"\#(id)","short":"\#(id)","title":"Terminal 1","preset":"claude","state":"running",
             "epoch":1,"paneMode":"agent","activity":"\#(activity)"}
            """#
        return try JSONDecoder().decode(Terminal.self, from: Data(json.utf8))
    }

    /// The transcript's scroll view: the one with the tallest document.
    static func scrollView(in view: NSView) -> NSScrollView? {
        var best: NSScrollView?
        func walk(_ v: NSView) {
            if let s = v as? NSScrollView,
                (s.documentView?.frame.height ?? 0) > (best?.documentView?.frame.height ?? 0)
            {
                best = s
            }
            v.subviews.forEach(walk)
        }
        walk(view)
        return best
    }

    /// Waits until `done` holds, up to `seconds`; whether it did. Generous,
    /// since a passing run returns as soon as it holds, and the full suite
    /// runs this beside two hundred others on a loaded machine.
    static func until(_ seconds: Double = 30, _ done: () -> Bool) async -> Bool {
        let deadline = ContinuousClock.now + .seconds(seconds)
        while ContinuousClock.now < deadline {
            if done() { return true }
            try? await Task.sleep(for: .milliseconds(20))
        }
        return done()
    }

    /// Puts the scroll view at `y` and keeps putting it there until `done`:
    /// a reader who has scrolled. Once, it can be undone before the chat has
    /// seen it: a jump's landing re-targets the end, and SwiftUI applies that
    /// at its next update, which a loaded machine reaches late, after the
    /// scroll and with no geometry event that shows the step. The reader's
    /// scroll isn't what's under test there, the chat's reading of it is.
    static func scroll(_ scroll: NSScrollView, to y: CGFloat, until done: () -> Bool) async -> Bool {
        await until {
            if done() { return true }
            scroll.contentView.scroll(to: NSPoint(x: 0, y: y))
            scroll.reflectScrolledClipView(scroll.contentView)
            return false
        }
    }

    /// SwiftUI's pending updates, applied now rather than whenever a loaded
    /// machine gets to them.
    static func flush(_ host: NSView, _ window: NSWindow) {
        CATransaction.flush()
        host.layoutSubtreeIfNeeded()
        window.displayIfNeeded()
    }

    /// The window as the window server composites it, glass included, to
    /// `FARCOOLER_CAPTURE_OUT/agent-scroll-<name>.png` when that's set:
    /// captures for a review, off in CI.
    static func capture(_ window: NSWindow, _ name: String) {
        guard let out = ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] else { return }
        let file = URL(fileURLWithPath: out).appendingPathComponent("agent-scroll-\(name).png")
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
        process.arguments = ["-x", "-o", "-l", String(window.windowNumber), file.path]
        try? process.run()
        process.waitUntilExit()
    }

    /// The pane's terminal, which a test changes as the fleet would.
    @MainActor
    final class Mount: ObservableObject {
        @Published var terminal: Terminal
        init(_ terminal: Terminal) { self.terminal = terminal }
    }

    struct Hosted: View {
        @ObservedObject var mount: Mount
        let binary: String
        let probe: AgentScrollProbe
        var body: some View {
            AgentSurface(
                terminal: mount.terminal, binary: binary, environment: ProcessInfo.processInfo.environment,
                hostArguments: [], linkGeneration: 0, refusal: { nil }, isFocused: false,
                searchFiles: { _ in [] }, onResize: { _, _ in }
            )
            .environment(\.agentScrollProbe, probe)
            .frame(width: 700, height: 600)
        }
    }

    /// A real click at `point` in `host`'s own coordinates, delivered as
    /// the window delivers one: an event, not the button's action called.
    static func click(_ host: NSView, at point: NSPoint, in window: NSWindow) {
        let location = host.convert(point, to: nil)
        for type in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
            guard
                let event = NSEvent.mouseEvent(
                    with: type, location: location, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                    windowNumber: window.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 1)
            else { continue }
            window.sendEvent(event)
        }
    }

    /// Lets layout and any scroll it asks for land.
    static func settle() async { try? await Task.sleep(for: .milliseconds(400)) }

    @Test func theChatScrollsLikeMessages() async throws {
        var history: [String] = []
        for turn in 0..<24 {
            history.append(Self.message("User", "Question \(turn)?"))
            history.append(Self.message("Agent", String(repeating: Self.sentence, count: 4)))
            history.append(Self.payload(["TurnEnded": ["reason": "EndTurn"]]))
        }
        let standIn = try StandIn(first: Self.batch(history, from: 0))
        var seq = history.count

        let probe = AgentScrollProbe()
        let pane = "scroll-\(UUID().uuidString.prefix(8))"
        defer { PaneDraftStore.record("", forPane: pane) }
        let mount = Mount(try Self.terminal(pane))
        let host = NSHostingView(rootView: Hosted(mount: mount, binary: standIn.binary, probe: probe))
        let window = NSWindow(
            contentRect: NSRect(x: -7000, y: -7000, width: 700, height: 600), styleMask: [.titled],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        if let look = ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_APPEARANCE"] {
            window.appearance = NSAppearance(named: look == "dark" ? .darkAqua : .aqua)
        }
        window.contentView = host
        window.orderFrontRegardless()
        defer { window.close() }

        // Opens at the tail, in view above the composer, with content
        // running under it.
        // Up to a minute: alone it lays out in under a second, and inside the
        // full suite, with the main actor shared, it has taken 21 s.
        #expect(await Self.until(60) { (probe.geometry?.contentSize.height ?? 0) > 2_000 }, "the history never laid out")
        await Self.settle()
        let opened = try #require(probe.geometry)
        #expect((probe.tailHiddenBy ?? .infinity) <= 0.5, "opened with the tail \(probe.tailHiddenBy ?? -1) pt under the composer")
        #expect(opened.contentInsets.bottom > 40, "the composer makes no inset: \(opened.contentInsets.bottom)")
        #expect(opened.visibleRect.height > opened.containerSize.height + 40, "content doesn't run under the composer")
        #expect(probe.following && !probe.showsJump)
        // A row's spacing between the last row and the glass, not the 1 pt
        // anchor under it: the tail can be "in view" with no room at all.
        #expect(
            (probe.clearance ?? 0) >= Spacing.inset - 1,
            "the last row ends \(probe.clearance ?? -1) pt above the composer")
        Self.capture(window, "1-opened")

        // Pinned: a streamed reply is followed.
        try standIn.stream([Self.batch([Self.message("User", "And now?")], from: seq)])
        seq += 1
        for k in 0..<6 {
            try standIn.stream([Self.batch([Self.message("Agent", Self.sentence + "\n\n")], from: seq + k)])
            try await Task.sleep(for: .milliseconds(150))
        }
        seq += 6
        let before = opened.contentSize.height
        #expect(await Self.until { (probe.geometry?.contentSize.height ?? 0) > before + 100 })
        await Self.settle()
        #expect((probe.tailHiddenBy ?? .infinity) <= 0.5, "streaming left the tail \(probe.tailHiddenBy ?? -1) pt under the composer")

        // Scrolled up: following stops, the way back is offered, and new
        // content moves nothing.
        let scroll = try #require(Self.scrollView(in: host))
        scroll.contentView.scroll(to: NSPoint(x: 0, y: 200))
        scroll.reflectScrolledClipView(scroll.contentView)
        #expect(await Self.until { !probe.following && probe.showsJump }, "scrolling up didn't stop following")
        let offset = try #require(probe.geometry?.contentOffset.y)
        let height = probe.geometry?.contentSize.height
        try standIn.stream([Self.batch([Self.message("Agent", String(repeating: Self.sentence, count: 3))], from: seq)])
        seq += 1
        _ = height
        try await Task.sleep(for: .milliseconds(1_000))
        #expect(abs((probe.geometry?.contentOffset.y ?? 0) - offset) < 1, "content arriving moved a reader who'd scrolled up")
        #expect(!probe.following && probe.showsJump)
        Self.capture(window, "2-scrolled-up")

        // Nor does the composer growing, or Working… appearing: both re-anchor
        // a pinned transcript, and only a pinned one.
        probe.prefill?((1...6).map { "Draft line \($0)" }.joined(separator: "\n"))
        try await Task.sleep(for: .milliseconds(1_000))
        #expect(abs((probe.geometry?.contentOffset.y ?? 0) - offset) < 1, "the composer growing moved a reader who'd scrolled up")
        #expect(!probe.following && probe.showsJump)
        mount.terminal = try Self.terminal(pane, activity: "working")
        try await Task.sleep(for: .milliseconds(1_000))
        #expect(abs((probe.geometry?.contentOffset.y ?? 0) - offset) < 1, "Working… appearing moved a reader who'd scrolled up")
        #expect(!probe.following && probe.showsJump)
        // One line again, for the growth below.
        probe.prefill?("x")

        // A sent message comes into view, from up there.
        probe.send?("Here's what I want next.")
        #expect(await Self.until { probe.following && (probe.tailHiddenBy ?? .infinity) <= 0.5 }, "the sent message stayed out of view: \(probe.tailHiddenBy ?? -1) pt")
        #expect(!probe.showsJump)
        await Self.settle()
        #expect(
            (probe.clearance ?? 0) >= Spacing.inset - 1,
            "the sent message ends \(probe.clearance ?? -1) pt above the composer")
        Self.capture(window, "3-sent")

        // The composer growing keeps the tail in view.
        let inset = try #require(probe.geometry?.contentInsets.bottom)
        probe.prefill?((1...6).map { "Line \($0) of a longer message" }.joined(separator: "\n"))
        #expect(await Self.until { (probe.geometry?.contentInsets.bottom ?? 0) > inset + 30 }, "the composer didn't grow")
        await Self.settle()
        #expect((probe.tailHiddenBy ?? .infinity) <= 0.5, "the composer grew over the tail by \(probe.tailHiddenBy ?? -1) pt")
        Self.capture(window, "4-composer-grown")

        // Jump to Latest goes back.
        scroll.contentView.scroll(to: NSPoint(x: 0, y: 100))
        scroll.reflectScrolledClipView(scroll.contentView)
        #expect(await Self.until(10) { probe.showsJump })
        await Self.settle()
        // Clicked where it's drawn: centered, its bottom 8 pt over the
        // composer group, whose top is where the scroll view's container
        // ends. It's drawn outside that group's frame, which is where a
        // click can miss.
        let container = try #require(probe.geometry?.containerSize.height)
        Self.click(host, at: NSPoint(x: 350, y: container - Spacing.group - 15), in: window)
        #expect(await Self.until { probe.following && (probe.tailHiddenBy ?? .infinity) <= 0.5 }, "Jump to Latest didn't return to the tail")
        #expect(!probe.showsJump)

        // A tail scrolled just under the composer isn't the tail: the part
        // of the viewport behind the glass isn't seen.
        // After Jump to Latest's animation has finished, or what it moves
        // next undoes the scroll below. Waited for, not slept through: a
        // loaded machine overruns any delay, and until it lands the chat
        // reads a step up as a height correction, not the reader.
        #expect(await Self.until { !probe.jumping }, "Jump to Latest never landed")
        let tailOffset = try #require(probe.geometry?.contentOffset.y)
        Self.flush(host, window)
        #expect(await Self.scroll(scroll, to: tailOffset - 70) { probe.showsJump }, "a tail 70 pt under the composer counted as seen")
    }

    /// A jump in flight with one step back made on the first geometry event
    /// after the click that finds the end still far off, so it can't land
    /// early on a fast runner without the step having been taken.
    @MainActor
    final class Stepper {
        var steps = 0
        /// Armed at least once: the jump has begun.
        var began = false
        var armed = false
        func arm(
            _ probe: AgentScrollProbe, _ scroll: NSScrollView, back points: CGFloat, after moved: CGFloat = 20,
            before: @escaping @MainActor () -> Void = {}
        ) {
            armed = true
            began = true
            // Where the jump starts from: the first events after the click
            // report that, before the animation has moved. A height
            // correction is taken once it has moved `moved` points, since a
            // step before that interrupts the animation instead of landing
            // inside it.
            let start = scroll.contentView.bounds.origin.y
            probe.onGeometry = { [self] geometry in
                guard armed, AgentSurface.tailHiddenBy(geometry) > 400, geometry.contentOffset.y > start + moved else { return }
                armed = false
                // A later turn: not from inside the chat's own observer.
                DispatchQueue.main.async { [self] in
                    before()
                    let origin = scroll.contentView.bounds.origin
                    scroll.contentView.scroll(to: NSPoint(x: origin.x, y: origin.y - points))
                    scroll.reflectScrolledClipView(scroll.contentView)
                    steps += 1
                }
            }
        }
    }

    /// Scrolled up and Jump to Latest clicked, in a chat that's laid out.
    private func jumping(_ body: (AgentScrollProbe, NSScrollView, NSView, NSWindow) async throws -> Void) async throws {
        var history: [String] = []
        for turn in 0..<24 {
            history.append(Self.message("User", "Question \(turn)?"))
            history.append(Self.message("Agent", String(repeating: Self.sentence, count: 4)))
            history.append(Self.payload(["TurnEnded": ["reason": "EndTurn"]]))
        }
        let standIn = try StandIn(first: Self.batch(history, from: 0))
        let probe = AgentScrollProbe()
        let pane = "jump-\(UUID().uuidString.prefix(8))"
        defer { PaneDraftStore.record("", forPane: pane) }
        let mount = Mount(try Self.terminal(pane))
        let host = NSHostingView(rootView: Hosted(mount: mount, binary: standIn.binary, probe: probe))
        let window = NSWindow(
            contentRect: NSRect(x: -7000, y: -7000, width: 700, height: 600), styleMask: [.titled],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.orderFrontRegardless()
        defer { window.close() }

        #expect(await Self.until(60) { (probe.geometry?.contentSize.height ?? 0) > 2_000 }, "the history never laid out")
        await Self.settle()
        let scroll = try #require(Self.scrollView(in: host))
        #expect(await Self.scroll(scroll, to: 100) { probe.showsJump }, "scrolling up didn't offer Jump to Latest")
        await Self.settle()
        try await body(probe, scroll, host, window)
    }

    /// A jump's animation, stretched for the tests that step the scroll
    /// mid-flight: the step is then mid-flight however long a loaded machine
    /// takes to reach it, since the animation runs on the clock and not on
    /// the test. The landing is waited for (`probe.jumping`), never slept
    /// through.
    private static let longFlight = 6.0

    private static func clickJump(_ probe: AgentScrollProbe, _ host: NSView, _ window: NSWindow) {
        let container = probe.geometry?.containerSize.height ?? 0
        click(host, at: NSPoint(x: 350, y: container - Spacing.group - 15), in: window)
    }

    /// Runs a scenario that needs a jump to animate, in a fresh chat each
    /// try. Under load a jump sometimes lands with no animation at all (its
    /// completion fires with the offset where it began, then the offset is
    /// at the end), and there's no mid-flight to step in. `body` returns
    /// whether the step was taken; when it wasn't, nothing about the
    /// outcome was asserted and the scenario is run again.
    private func animatedJump(
        _ body: (AgentScrollProbe, NSScrollView, NSView, NSWindow) async throws -> Bool
    ) async throws {
        let original = AgentSurface.scheduleBackstop
        let duration = AgentSurface.jumpDuration
        defer {
            AgentSurface.scheduleBackstop = original
            AgentSurface.jumpDuration = duration
        }
        for _ in 0..<6 {
            var stepped = false
            try await jumping { probe, scroll, host, window in
                stepped = try await body(probe, scroll, host, window)
            }
            if stepped { return }
            AgentSurface.scheduleBackstop = original
            AgentSurface.jumpDuration = duration
        }
        Issue.record("the jump never animated in 6 tries, so there was no mid-flight to step in")
    }

    /// Waits for the step to be taken, or for the jump to end without it.
    private static func stepTaken(_ stepper: Stepper, _ probe: AgentScrollProbe) async -> Bool {
        // The click is delivered as an event, so the jump begins a turn
        // later: its backstop is what arms the stepper.
        #expect(await until { stepper.began }, "the click never started a jump")
        _ = await until { stepper.steps == 1 || !probe.jumping }
        return stepper.steps == 1
    }

    /// Jump to Latest doesn't flicker (ov-386). A lazy stack correcting its
    /// height mid-flight walks the offset back, which the chat read as the
    /// reader scrolling up: following stopped and the button came back until
    /// the scroll landed.
    @Test func jumpToLatestDoesNotFlickerAgainstAHeightCorrection() async throws {
        try await animatedJump { probe, scroll, host, window in
            let detached = probe.detaches
            let stepper = Stepper()
            // Armed as the jump begins, not before the click. The backstop
            // is held, not run: a second of the clock would end the jump
            // mid-flight on a slow runner, and only the animation's own
            // completion should.
            AgentSurface.jumpDuration = Self.longFlight
            AgentSurface.scheduleBackstop = { _ in stepper.arm(probe, scroll, back: 100) }
            Self.clickJump(probe, host, window)
            guard await Self.stepTaken(stepper, probe) else { return false }
            #expect(await Self.until { probe.following && (probe.tailHiddenBy ?? .infinity) <= 0.5 }, "Jump to Latest didn't return to the tail")
            #expect(await Self.until { !probe.jumping }, "Jump to Latest never landed")
            #expect(probe.detaches == detached, "the height correction stopped following \(probe.detaches - detached) time(s) mid-jump")
            #expect(probe.following && !probe.showsJump)
            return true
        }
    }

    /// A second Jump to Latest isn't ended by the first one's backstop
    /// (the jump generation, ov-386). Backstops are held, and the first's
    /// is run inside the second jump's flight, just before the step back
    /// that only the second jump's own guard absorbs.
    @Test func aSecondJumpIsNotEndedByTheFirstOnesBackstop() async throws {
        try await animatedJump { probe, scroll, host, window in
            let held = Held()
            let stepper = Stepper()
            // The second jump arms the step back as it begins, not before the click.
            AgentSurface.scheduleBackstop = { body in
                held.bodies.append(body)
                if held.bodies.count == 2 {
                    stepper.arm(probe, scroll, back: 100) {
                        // Jump 1's backstop; jump 2's own stays held.
                        #expect(held.bodies.count == 2, "expected one backstop per jump, got \(held.bodies.count)")
                        held.bodies.first?()
                    }
                }
            }
            Self.clickJump(probe, host, window)
            #expect(await Self.until { probe.following && (probe.tailHiddenBy ?? .infinity) <= 0.5 })
            // Landed, or the scroll up below reads as a height correction.
            #expect(await Self.until { !probe.jumping }, "the first jump never landed")
            Self.flush(host, window)
            #expect(await Self.scroll(scroll, to: 100) { probe.showsJump }, "scrolling up after the jump didn't offer Jump to Latest")
            await Self.settle()
            Self.flush(host, window)
            let detached = probe.detaches
            AgentSurface.jumpDuration = Self.longFlight
            Self.clickJump(probe, host, window)
            guard await Self.stepTaken(stepper, probe) else { return false }
            #expect(await Self.until { probe.following && (probe.tailHiddenBy ?? .infinity) <= 0.5 })
            #expect(await Self.until { !probe.jumping }, "the second jump never landed")
            #expect(probe.detaches == detached, "the second jump flickered: \(probe.detaches - detached)")
            return true
        }
    }

    @MainActor final class Held {
        var bodies: [@MainActor () -> Void] = []
    }

    /// A reader who flicks away mid-flight by more than a correction is not
    /// pulled back when the animation ends.
    ///
    /// The flick is armed from inside the jump (its backstop is scheduled
    /// as the jump begins), not before the click: armed earlier, a late
    /// geometry report from the scrolled-up chat could take the flick before
    /// the jump had started, and the jump then re-followed.
    @Test func aReaderWhoFlicksAwayMidJumpIsNotPulledBack() async throws {
        try await animatedJump { probe, scroll, host, window in
            let stepper = Stepper()
            AgentSurface.jumpDuration = Self.longFlight
            // The flick is on the first event, before the animation has
            // moved: it interrupts the animation, whose completion then runs
            // with the reader away. Taken later, the animation, still going,
            // carries the reader to the end on its own (the 0.25 s a reader
            // is overridden for), and that isn't what's under test.
            AgentSurface.scheduleBackstop = { _ in stepper.arm(probe, scroll, back: 1_200, after: -1) }
            Self.clickJump(probe, host, window)
            guard await Self.stepTaken(stepper, probe) else { return false }
            #expect(await Self.until { !probe.following && probe.showsJump }, "a flick of 1,200 pt didn't stop following")
            // The animation's completion runs, with the reader away. If the
            // animation carried them back first, there's nothing to check.
            let ended = await Self.until { !probe.jumping || probe.following }
            guard ended, !probe.following else { return false }
            // What the completion would do is applied by the next update, so
            // flush it, and give it a moment to show.
            Self.flush(host, window)
            await Self.settle()
            #expect(!probe.following && probe.showsJump, "the reader was pulled back to the tail")
            return true
        }
    }
}
