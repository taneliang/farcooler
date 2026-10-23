import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// What a reader sees when the lazy stack sizes its content again (ov-699).
///
/// SwiftUI's lazy stack guesses every row it hasn't built at the mean of the
/// rows it has: 148 pt from the session's last rows, where the true mean is
/// about 121. Flung up `RealSession`, it holds its content at that guess
/// (47,285 pt) until about 30,000 pt up, then sizes it again in one frame,
/// to about 38,600 pt.
///
/// Measured here two ways, through the same flings as `noRowJumps`, every
/// fifth frame late (a 25 ms stall), as on a busy Mac:
///
/// - **The rows** (`rowsMoveOnlyByTheScroll`): whether the content on screen
///   moved by exactly what the fling scrolled, added up over each fling. It
///   does, to a few points, on the re-estimate frame as on any other: the
///   stack moves the clip by what it took off above. `noRowJumps` reads
///   145 to 340 pt "jumps" there because it compares one frame's move with
///   that frame's ask; on a late frame the clip moves two or three frames'
///   worth at once. Green on main, and `theRowCheckSeesAJump` shows it can
///   go red.
/// - **The scroller** (`theScrollerHoldsStill`): the system knob's place on
///   its track is the clip's place in the content, so sizing the content
///   again moved it with no scroll: about a fifth of the track in one frame.
///   The transcript's scroller is placed from our own heights instead
///   (`TranscriptScroller`, ov-701), and moves under 1%.
///
/// Opt-in with FARCOOLER_PERF=1, as the other fling tests: a fling needs the
/// display link to itself; run in the nightly Mac perf job. The designs
/// compared against the lazy stack (an NSTableView of hosted rows, and
/// SwiftUI's List) are at the tag `proto/ov-699`.
@MainActor
@Suite(.serialized)
struct TranscriptJumpTests {
    typealias T = NativeAgentTests

    init() throws {
        _ = try ConversationScrollTests()
    }

    /// Where each row on screen is, by probe id, from the top of the window.
    static func shown(_ frames: NativePolishTests.Frames, scroll: NSScrollView) -> [String: CGRect] {
        ConversationScrollTests.shown(frames, height: scroll.window?.contentView?.bounds.height ?? 900)
    }

    /// What a fling scrolled against what the rows on screen moved, added up
    /// frame by frame (ov-699): the median move of the rows on screen in both
    /// frames. A row that jumps leaves the two apart by the jump; a late
    /// frame's scroll, applied two or three frames' worth at once, doesn't.
    struct Tally {
        private(set) var asked: CGFloat = 0
        private(set) var moved: CGFloat = 0
        private var before: [String: CGRect]

        init(_ shown: [String: CGRect]) { before = shown }

        mutating func add(asked asking: CGFloat, shown now: [String: CGRect]) {
            let moves = now.compactMap { id, frame in before[id].map { frame.minY - $0.minY } }.sorted()
            if !moves.isEmpty { moved += moves[moves.count / 2] }
            asked += asking
            before = now
        }

        /// What was scrolled less what the rows moved: scrolled up, the
        /// rows move down by it.
        var lost: CGFloat { -asked - moved }
    }

    /// One fling's record.
    struct Flung {
        let velocity: CGFloat
        /// What the fling scrolled less what the rows on screen moved, added
        /// up from its first frame through the rest after it: a row that
        /// jumped leaves it off by the jump.
        let lost: CGFloat
        /// Whether it reached the first row, where the scroll view stops the
        /// clip short of what was asked.
        let top: Bool
        /// The most the knob moved in one frame beyond the frame's scroll,
        /// as a share of its track; and the content's largest change.
        let knob: Double
        let resized: CGFloat
        /// The frames where the knob moved more than 2% beyond the scroll:
        /// what moved, for a failure's message.
        var over: [String] = []
    }

    /// Where the scroller draws its knob's top, as a share of its track:
    /// what the reader sees move.
    static func knob(_ scroll: NSScrollView) -> Double {
        guard let scroller = scroll.verticalScroller else { return 0 }
        return scroller.doubleValue * Double(1 - scroller.knobProportion)
    }

    /// `noRowJumps`'s flings, slow near the end and then fast all the way up,
    /// with every `stall`th frame held 25 ms; `jump` moves the clip on one
    /// frame with no scroll, to show the row check sees it.
    static func fling(
        _ window: NSWindow, _ scroll: NSScrollView, _ frames: NativePolishTests.Frames, stall: Int = 5,
        velocities: [CGFloat] = [-1_500, -1_500, -3_000, -6_000, -6_000] + Array(repeating: -12_000, count: 12),
        jump: (tick: Int, by: CGFloat)? = nil
    ) async -> [Flung] {
        var flung: [Flung] = []
        for velocity in velocities where scroll.contentView.bounds.origin.y > -scroll.contentInsets.top + 1 {
            let fling = Fling(scroll, in: window, velocity: velocity)
            var tally = Tally(shown(frames, scroll: scroll))
            var knob = 0.0
            var resized: CGFloat = 0
            var docWas = scroll.documentView?.frame.height ?? 0
            var placeWas = Self.knob(scroll)
            var originWas = scroll.contentView.bounds.minY
            var over: [String] = []
            func track(_ asking: CGFloat) {
                tally.add(asked: asking, shown: shown(frames, scroll: scroll))
                let doc = scroll.documentView?.frame.height ?? 0
                let place = Self.knob(scroll)
                // A frame's own scroll moves the knob by its share of the
                // track, a late frame's by up to a few frames' worth.
                let range = max(1, doc + scroll.contentInsets.top + scroll.contentInsets.bottom - scroll.contentView.bounds.height)
                let scrolled = Double((abs(asking) * 4 + 20) / range)
                let moved = abs(place - placeWas) - scrolled
                knob = max(knob, moved)
                // Only past the bar, so a pass formats nothing.
                if moved > 0.02 {
                    let driver = TranscriptScroller.driving(scroll)
                    let ours = driver?.layout?.knob(scroll)
                    over.append(String(
                        format: "tick %d asked %.0f: knob %.4f>%.4f (value %.4f, ours %.4f%@, aimed %@), doc %.0f>%.0f, origin %.0f>%.0f, live %@",
                        fling.stamps.count, asking, placeWas, place, scroll.verticalScroller?.doubleValue ?? -1, ours?.value ?? -1,
                        ours?.onScreen == false ? " off screen" : "", driver?.aimed.map { String(format: "%.4f", $0) } ?? "none",
                        docWas, doc, originWas, scroll.contentView.bounds.minY, LiveScroll.scrolls(scroll) ? "yes" : "no"))
                }
                resized = max(resized, abs(doc - docWas))
                docWas = doc
                placeWas = place
                originWas = scroll.contentView.bounds.minY
            }
            fling.onTick = { asking in
                if let jump, fling.stamps.count == jump.tick {
                    scroll.contentView.scroll(to: NSPoint(x: 0, y: scroll.contentView.bounds.origin.y + jump.by))
                    scroll.reflectScrolledClipView(scroll.contentView)
                    window.contentView?.layoutSubtreeIfNeeded()
                }
                if stall > 0, fling.stamps.count % stall == 0 { usleep(25_000) }
                track(asking)
            }
            fling.start()
            await fling.finish()
            await T.settle(window, 200)  // capture-exempt: not a capture; the clip catching up with the fling's last frames
            track(0)
            let top = scroll.contentView.bounds.origin.y < -scroll.contentInsets.top + 200
            flung.append(Flung(velocity: velocity, lost: tally.lost, top: top, knob: knob, resized: resized, over: over))
        }
        print("ov-699: " + flung.map {
            String(format: "%.0f pt/s lost %.0f knob %.1f%% resized %.0f%@", $0.velocity, $0.lost, $0.knob * 100, $0.resized, $0.top ? " (top)" : "")
        }.joined(separator: "; "))
        return flung
    }

    @Test(
        "Flung up a long session, the rows on screen move by the scroll and nothing else",
        .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_PERF"] == "1"))
    func rowsMoveOnlyByTheScroll() async throws {
        let frames = NativePolishTests.Frames()
        let (window, _, scroll) = try await ConversationScrollTests.session(probed: frames)
        defer { window.close() }
        let flung = await Self.fling(window, scroll, frames)
        #expect(flung.count > 8, "the flings stopped after \(flung.count)")
        for fling in flung where !fling.top {
            #expect(abs(fling.lost) <= 12, "a fling at \(Int(fling.velocity)) pt/s moved the rows \(Int(fling.lost)) pt less than it scrolled")
        }
    }

    /// The check above can fail: the clip moved 150 pt on one frame with no
    /// scroll is read as a fling that lost 150 pt.
    @Test(
        "A row that jumps is caught",
        .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_PERF"] == "1"))
    func theRowCheckSeesAJump() async throws {
        let frames = NativePolishTests.Frames()
        let (window, _, scroll) = try await ConversationScrollTests.session(probed: frames)
        defer { window.close() }
        let flung = await Self.fling(window, scroll, frames, velocities: [-3_000], jump: (30, -150))
        let lost = try #require(flung.first).lost
        #expect(abs(abs(lost) - 150) <= 12, "a 150 pt jump read as \(Int(lost)) pt")
    }

    @Test(
        "Flung up a long session, the scroller's knob moves only with the scroll",
        .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_PERF"] == "1"))
    func theScrollerHoldsStill() async throws {
        let frames = NativePolishTests.Frames()
        let (window, _, scroll) = try await ConversationScrollTests.session(probed: frames)
        defer { window.close() }
        // A busier main thread than every fifth frame late
        // (`FARCOOLER_FLING_STALL`).
        let stall = Int(ProcessInfo.processInfo.environment["FARCOOLER_FLING_STALL"] ?? "") ?? 5
        let flung = await Self.fling(window, scroll, frames, stall: stall)
        #expect(flung.count > 8, "the flings stopped after \(flung.count)")
        // Two percent of an 860 pt track is 17 pt: a row's guess put right.
        // The system's knob, placed from the lazy stack's content size,
        // moved about 18% on its re-estimate (ov-699), every run; ours is
        // placed from our own heights (`TranscriptLayout`, ov-701).
        #expect(flung.contains { $0.resized > 2_000 }, "the stack never sized its content again: this checked nothing")
        // How often AppKit set the knob from the content and ours was set
        // again before the frame was committed.
        let driver = TranscriptScroller.driving(scroll)
        print("ov-701: AppKit set the knob again \(driver?.overwritten ?? -1) times over the flings")
        let worst = flung.max { $0.knob < $1.knob }
        #expect(
            (worst?.knob ?? 0) <= 0.02,
            "the knob jumped \(String(format: "%.1f", (worst?.knob ?? 0) * 100))% of its track at \(Int(worst?.velocity ?? 0)) pt/s, the content resized by \(Int(worst?.resized ?? 0)) pt in a frame: \(flung.flatMap(\.over).prefix(6).joined(separator: "; "))")
    }

    /// The main thread's time a step while the window's edge is dragged,
    /// wider and back, mid-session: every step lays the rows out at a new
    /// width, and the scroller's heights are worked out again (ov-701).
    /// Printed, for comparing two builds alternately.
    @Test(
        "Resizing the window mid-session costs the main thread little a step",
        .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_PERF"] == "1"))
    func resizeKeepsUp() async throws {
        let rows = Int(ProcessInfo.processInfo.environment["FARCOOLER_FLING_ROWS"] ?? "") ?? RealSession.count
        let (window, _, scroll) = try await ConversationScrollTests.session(rows: rows, resizable: true)
        defer { window.close() }
        // Mid-session, by the scroll view's own measure.
        let insets = scroll.contentInsets
        let range = (scroll.documentView?.frame.height ?? 0) + insets.top + insets.bottom - scroll.contentView.bounds.height
        scroll.contentView.scroll(to: NSPoint(x: 0, y: range / 2 - insets.top))
        scroll.reflectScrolledClipView(scroll.contentView)
        await T.settle(window, 400)  // capture-exempt: not a capture; the rows there laid out
        var steps: [Double] = []
        let widths = Array(stride(from: CGFloat(740), through: 1_300, by: 8)) + Array(stride(from: CGFloat(1_292), through: 600, by: -8))
        for width in widths {
            let before = Fling.threadCPU()
            window.setContentSize(NSSize(width: width, height: 900))
            window.contentView?.layoutSubtreeIfNeeded()
            window.displayIfNeeded()
            steps.append(Fling.threadCPU() - before)
            try? await Task.sleep(for: .milliseconds(8))
        }
        let sorted = steps.sorted()
        let mean = steps.reduce(0, +) / Double(steps.count)
        print(String(format: "ov-701 resize: %d steps, main thread CPU mean %.2f ms, median %.2f, p95 %.2f, max %.2f a step", steps.count, mean,
                     sorted[sorted.count / 2], sorted[Int(Double(sorted.count) * 0.95)], sorted.last ?? 0))
        #expect(steps.count > 100)
    }
}
