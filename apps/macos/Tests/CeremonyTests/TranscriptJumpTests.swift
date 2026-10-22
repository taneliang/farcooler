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
/// - **The scroller** (`theScrollerHoldsStill`): the knob's place on its
///   track is the clip's place in the content, so sizing the content again
///   moves the knob with no scroll: about a fifth of the track in one frame.
///   Red on main, every run, so held as a known issue until ov-701.
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
            var originWas = scroll.contentView.bounds.origin.y
            func track(_ asking: CGFloat) {
                tally.add(asked: asking, shown: shown(frames, scroll: scroll))
                let doc = scroll.documentView?.frame.height ?? 0
                let origin = scroll.contentView.bounds.origin.y
                // The clip's travel: from the top inset to the content's end.
                let inset = scroll.contentInsets.top
                let range = max(1, doc + inset - scroll.contentView.bounds.height)
                let rangeWas = max(1, docWas + inset - scroll.contentView.bounds.height)
                let place = Double((origin + inset) / range)
                let placeWas = Double((originWas + inset) / rangeWas)
                // A frame's own scroll moves the knob by its share of the
                // track, a late frame's by up to a few frames' worth.
                let scrolled = Double((abs(asking) * 4 + 20) / range)
                knob = max(knob, abs(place - placeWas) - scrolled)
                resized = max(resized, abs(doc - docWas))
                docWas = doc
                originWas = origin
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
            flung.append(Flung(velocity: velocity, lost: tally.lost, top: top, knob: knob, resized: resized))
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
        let flung = await Self.fling(window, scroll, frames)
        #expect(flung.count > 8, "the flings stopped after \(flung.count)")
        // Two percent of an 860 pt track is 17 pt: a row's guess put right.
        // The lazy stack's re-estimate moves it about 18% (ov-699), every
        // run: held here until its card lands, so the nightly stays green.
        let worst = flung.max { $0.knob < $1.knob }
        withKnownIssue("ov-701: the lazy stack sizes its content again in one frame, and the scroller's knob jumps with it") {
            #expect(
                (worst?.knob ?? 0) <= 0.02,
                "the knob jumped \(String(format: "%.1f", (worst?.knob ?? 0) * 100))% of its track at \(Int(worst?.velocity ?? 0)) pt/s, the content resized by \(Int(worst?.resized ?? 0)) pt in a frame")
        }
    }
}
