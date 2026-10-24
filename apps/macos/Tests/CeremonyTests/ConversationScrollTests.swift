import AgentKit
import AppKit
import SwiftUI
import TestWindows
import Testing

@testable import Far_Cooler

/// Scrolling a long conversation (ov-691): flung through four hundred real
/// rows a frame at a time, in a real window on a real display, the list keeps
/// up with the display, and no row moves on screen except by the scroll.
@MainActor
@Suite(.serialized)
struct ConversationScrollTests {
    typealias T = NativeAgentTests

    /// This suite's own pane, so no other suite's remembered view is touched.
    static let terminal = "0199aaaa-0000-7000-8000-000000000691"

    init() throws {
        PromptImageStore.disk = PromptImageDisk(
            files: PaneImageFiles(
                root: try ScratchDirectory.make("fc-scroll-images"), paneCap: 50_000_000, totalCap: 100_000_000,
                whenFull: .evictOldest, maxAge: 3600))
    }

    /// The session in a 720 by 900 window, or `width` wide, `rows` long, at
    /// its end, photos coming late. `resizable`, the view takes the window's
    /// size, for a test that resizes it.
    static func session(
        probed frames: NativePolishTests.Frames? = nil, width: CGFloat = 720, rows: Int = RealSession.count, resizable: Bool = false
    ) async throws -> (NSWindow, NativePaneModel, NSScrollView) {
        let model = T.model(try T.terminal(id: terminal))
        model.promptImages.source = RealSession.Photos()
        model.keys = RealSession.Keys()
        let view = NativeAgentView(model: model, isFocused: true, showTerminal: {})
        let window: NSWindow
        if resizable {
            window = NavigatorFilterTests.KeyWindow(
                contentRect: NSRect(x: -9000, y: -9000, width: width, height: 900), styleMask: [.titled, .resizable], backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.contentView = NSHostingView(rootView: view)
            TestWindow.show(window)
        } else {
            window = frames.map { NativePolishTests.window(width: width, height: 900, NativePolishTests.Framed(frames: $0, content: view)) }
                ?? NativePolishTests.window(width: width, height: 900, view)
        }
        model.store.apply(try await model.store.ledger.page(T.page(RealSession.rows(rows, of: rows))))
        await T.settle(window, 600)  // capture-exempt: not a capture; the session drawn before a fling, which measures its own frames
        let content = try #require(window.contentView)
        let scroll = try #require(Fling.scrollView(in: content), "no scroll view")
        return (window, model, scroll)
    }

    /// The streaming answer at the end grows by a few words every tenth tick,
    /// as a reply does while the reader scrolls back.
    static func streams(_ model: NativePaneModel) -> (Int) -> Void {
        var rev = 2
        var text = "Streaming answer."
        var words = 0
        return { tick in
            guard tick % 10 == 0 else { return }
            rev += 1
            RealSession.more(&text, words: &words)
            let page = RealSession.streaming(text, rev: rev)
            let at = rev
            Task { @MainActor in
                guard let data = try? JSONSerialization.data(withJSONObject: ["epoch": 1, "rev": at, "reset": false, "changes": [["id": "real:stream", "rev": at, "kind": "update", "row": page]]]),
                      case .delta(let delta)? = try? await model.store.ledger.follow(data)
                else { return }
                model.store.apply(delta)
            }
        }
    }

    /// The most of a fling's time, in ms a second, its ticks may come late
    /// while the main thread runs a frame or longer without a break: the
    /// median of four flings. Before ov-691, every streamed word drew the
    /// whole transcript and the composer again, 9 to 20 ms a time: 47 ms/s.
    /// After it, 0 to 15 ms/s here (a debug build, the Mac busy with other
    /// work); a regression of that kind lands well over this. On the
    /// nightly's runner 75 (`PerfBudget`): it measured 31 to 50 there on
    /// Oct 22, and before ov-691 this Mac measured 47 to 72.
    static let hitchBudget = PerfBudget.limit(local: 40, runner: 75)

    /// Opt-in, as `NativeStreamingStallTests` is: a timing gate on a shared
    /// machine, with every other suite drawing on the same main thread, can
    /// starve the display link and fail on nothing of its own. Runs with
    /// FARCOOLER_PERF=1. What it caught is gated in every run by AgentKit's
    /// `theNewestTurnIgnoresTheRowsAfterIt`.
    @Test(
        "Flinging up through a long session keeps up with the display, while a reply streams",
        .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_PERF"] == "1"))
    func flingKeepsUp() async throws {
        // A longer session for a cost that grows with it (`FARCOOLER_FLING_ROWS`).
        let rows = Int(ProcessInfo.processInfo.environment["FARCOOLER_FLING_ROWS"] ?? "") ?? RealSession.count
        let (window, model, scroll) = try await Self.session(rows: rows)
        defer { window.close() }
        let stream = Self.streams(model)
        var ratios: [Double] = []
        var cpu: [Double] = []
        var afterCPU: [Double] = []
        var afterLongest: [Double] = []
        var lines: [String] = []
        // More rounds for a profiler to watch (`FARCOOLER_FLING_ROUNDS`).
        let rounds = Int(ProcessInfo.processInfo.environment["FARCOOLER_FLING_ROUNDS"] ?? "") ?? 1
        let velocities = Array(repeating: [CGFloat(-3_000), -6_000, -6_000, -9_000], count: max(1, rounds)).joined()
        for velocity in velocities {
            let fling = Fling(scroll, in: window, velocity: velocity)
            fling.beforeTick = stream
            fling.start()
            await fling.finish()
            // A fling that didn't scroll, or got no frames, measured nothing.
            #expect(fling.stamps.count > 60, "\(Int(velocity)) pt/s: \(fling.stamps.count) frames")
            #expect(abs(fling.moved) > abs(velocity) * 0.4, "\(Int(velocity)) pt/s moved \(Int(fling.moved)) pt")
            // One live scroll a fling, begun and ended, as a trackpad's: the
            // end is where rows drawn mid-scroll turn selection on.
            #expect(fling.liveStarts == 1 && fling.liveEnds == 1, "\(Int(velocity)) pt/s: \(fling.liveStarts) live scrolls began, \(fling.liveEnds) ended")
            ratios.append(fling.mainThreadHitchRatio)
            cpu.append(fling.cpu / Double(max(1, fling.stamps.count)))
            afterCPU.append(fling.afterCPU)
            afterLongest.append(fling.afterLongest)
            lines.append("\(Int(velocity)) pt/s: \(fling.summary)")
            await T.settle(window, 200)  // capture-exempt: not a capture; a rest between flings
        }
        let photos = model.promptImages.states.values.filter { if case .shown = $0 { true } else { false } }.count
        print("ov-691 fling: \(photos) photos shown\n  " + lines.joined(separator: "\n  "))
        let sorted = ratios.sorted()
        let median = (sorted[(sorted.count - 1) / 2] + sorted[sorted.count / 2]) / 2
        // The main thread's CPU time a tick: steadier than the hitch ratio
        // on a busy machine, for comparing two builds (ov-693).
        let cpuMean = cpu.reduce(0, +) / Double(max(1, cpu.count))
        print(String(format: "ov-691 fling: median main-thread hitch %.1f ms/s, main thread CPU %.2f ms a tick; ", median, cpuMean)
            + PerfBudget.describe(Self.hitchBudget, unit: "ms/s"))
        // And the 150 ms after each fling's live scroll ends, where rows
        // drawn mid-scroll turn text selection on (ov-693).
        let afterMean = afterCPU.reduce(0, +) / Double(max(1, afterCPU.count))
        print(String(format: "ov-693 after each fling: main thread CPU %.1f ms, longest run %.1f ms (worst %.1f)", afterMean,
                     afterLongest.reduce(0, +) / Double(max(1, afterLongest.count)), afterLongest.max() ?? 0))
        #expect(median <= Self.hitchBudget, "scrolling hitched \(median) ms/s:\n\(lines.joined(separator: "\n"))")
    }

    /// Where each row was drawn on screen, by id, that the window shows.
    static func shown(_ frames: NativePolishTests.Frames, height: CGFloat) -> [String: CGRect] {
        frames.views.filter { $0.key.hasPrefix("native-row-") && $0.value.maxY > 0 && $0.value.minY < height }
    }

    /// One tick of `noRowJumps`, for a failure's log (ov-708): how late it
    /// came, what it asked and what the clip moved, how many rows on screen
    /// it shared with the tick before, the content's height, and the rows
    /// that came, went or changed height.
    struct Tick {
        let n: Int
        let gap: Double
        let asked: CGFloat
        let clip: CGFloat
        /// The rows shared with the tick before: their median move.
        let rows: CGFloat?
        let shared: Int
        /// Where the clip was when the tick began, before its scroll.
        let began: CGFloat
        let ending: Bool
        let doc: (CGFloat, CGFloat)
        let origin: (CGFloat, CGFloat)
        let live: Bool
        let changed: [String]

        var line: String {
            String(format: "tick %d%@, %.0f ms after the last: asked %.0f, clip moved %.0f, rows %@ (%d shared); doc %.0f>%.0f, origin %.0f>%.0f>%.0f, live %@",
                   n, ending ? " (the last)" : "", gap, asked, clip, rows.map { String(format: "%.0f", $0) } ?? "unread", shared, doc.0, doc.1, origin.0,
                   began, origin.1, live ? "yes" : "no")
                + (changed.isEmpty ? "" : "; " + changed.prefix(6).joined(separator: ", "))
        }
    }

    static func medianMove(_ before: [String: CGRect], _ now: [String: CGRect]) -> CGFloat? {
        let moves = now.compactMap { id, frame in before[id].map { frame.minY - $0.minY } }.sorted()
        return moves.isEmpty ? nil : moves[moves.count / 2]
    }

    /// Timing-bound like `flingKeepsUp`: a shared CI runner can't drive a
    /// fling at a steady 120 Hz, so this runs where that one does, in the
    /// nightly Mac perf job (FARCOOLER_PERF=1), not on every push.
    @Test(
        "Flicking up, no row on screen moves except by the scroll",
        .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_PERF"] == "1"))
    func noRowJumps() async throws {
        let frames = NativePolishTests.Frames()
        let (window, _, scroll) = try await Self.session(probed: frames)
        defer { window.close() }
        let height = scroll.contentView.bounds.height
        let stall = Int(ProcessInfo.processInfo.environment["FARCOOLER_FLING_STALL"] ?? "") ?? 0
        var jumps: [String] = []
        // Slow flicks near the end first, where a row built above the screen
        // that changes its height moves everything under it; then faster,
        // all the way to the first row.
        let velocities = [CGFloat(-1_500), -1_500, -3_000, -6_000, -6_000] + Array(repeating: -12_000, count: 12)
        for velocity in velocities where scroll.contentView.bounds.origin.y > -scroll.contentInsets.top + 1 {
            let fling = Fling(scroll, in: window, velocity: velocity)
            let room = scroll.contentView.bounds.origin.y + scroll.contentInsets.top
            var before = Self.shown(frames, height: height)
            var tally = TranscriptJumpTests.Tally(before)
            var built = frames.views.filter { $0.key.hasPrefix("native-row-") }
            var docWas = scroll.documentView?.frame.height ?? 0
            var originWas = scroll.contentView.bounds.origin.y
            // Each tick's record, printed for a fling whose tally is off
            // (ov-708): formatted only then, to keep the tick cheap.
            var ticks: [Tick] = []
            var began: CGFloat = 0
            fling.beforeTick = { _ in began = scroll.contentView.bounds.origin.y }
            fling.onTick = { asked in
                // A busy main thread (`FARCOOLER_FLING_STALL=n`): every nth
                // frame held 25 ms, as `TranscriptJumpTests` holds them.
                if stall > 0, fling.stamps.count % stall == 0 { usleep(25_000) }
                let now = Self.shown(frames, height: height)
                let resizing = abs((scroll.documentView?.frame.height ?? 0) - docWas) > 0.5
                tally.add(asked: asked, clip: scroll.contentView.bounds.origin.y - originWas, resized: resizing, shown: now)
                let all = frames.views.filter { $0.key.hasPrefix("native-row-") }
                // What changed height, or came, for the message.
                let changed = all.compactMap { id, frame -> String? in
                    guard let was = built[id] else { return "\(id) new \(Int(frame.height)) at \(Int(frame.minY))" }
                    return abs(was.height - frame.height) > 1 ? "\(id) \(Int(was.height))>\(Int(frame.height))" : nil
                }
                let gone = built.keys.filter { all[$0] == nil }.map { "\($0) gone" }
                let doc = scroll.documentView?.frame.height ?? 0
                let origin = scroll.contentView.bounds.origin.y
                let moved = origin - originWas
                let geometry = "doc \(Int(docWas))>\(Int(doc)) origin \(Int(originWas))>\(Int(origin))"
                let why = "tick \(fling.stamps.count), \(Int(velocity)) pt/s; \(geometry); \((changed + gone).joined(separator: ", "))"
                // Where the rows above were guessed and are now measured,
                // the lazy stack sizes its content again and moves the clip
                // with it, the rows staying where they are on screen. That
                // frame's own move can't be read against its scroll: the
                // clip applies what was asked a frame or two late, and after
                // a late frame two or three frames' worth at once, which
                // read as 145 to 340 pt "jumps" until ov-699. The fling's
                // tally below reads it instead.
                let resized = abs(doc - docWas) > 0.5
                let stamps = fling.stamps
                ticks.append(Tick(
                    n: stamps.count, gap: stamps.count > 1 ? (stamps[stamps.count - 1] - stamps[stamps.count - 2]) * 1000 : 0, asked: asked,
                    clip: moved, rows: Self.medianMove(before, now), shared: now.filter { before[$0.key] != nil }.count, began: began,
                    ending: fling.ending, doc: (docWas, doc), origin: (originWas, origin), live: LiveScroll.scrolls(scroll), changed: changed + gone))
                // At the top the scroll view stops the clip where the
                // content starts, short of what was asked, and springs back.
                // And a tick that came so late its clip moved several ticks'
                // worth at once (a full run's other suites holding the main
                // thread) has rows drawn for a clip between the two: neither
                // can be read. Neither hides a fold, which moves rows on a
                // tick whose clip moved as asked.
                let atTop = origin < -scroll.contentInsets.top + max(abs(asked) * 3, 1_000)
                let late = abs(moved) > abs(asked) * 3 + 20
                // Flung up, the list never goes down: nothing pulls the
                // reader back toward the end.
                if !resized, !atTop, moved > 1 { jumps.append("the list went down \(Int(moved)) pt (\(why))") }
                for (id, frame) in now where !atTop && !late && !resized {
                    guard let was = before[id] else { continue }
                    // Scrolled by `moved`, a row is drawn that much the
                    // other way, and no further: a fold moves it 48 to 345
                    // pt more.
                    let drift = (frame.minY - was.minY) + moved
                    if abs(drift) > 2 {
                        jumps.append("\(id) by \(Int(drift)) at \(Int(was.minY)) (\(why))")
                        break
                    }
                }
                docWas = doc
                originWas = origin
                built = all
                before = now
            }
            fling.start()
            await fling.finish()
            await T.settle(window, 200)  // capture-exempt: not a capture; the clip catching up with the fling's last frames
            // Over the whole fling, the rows on screen moved by what it
            // scrolled (ov-699): a row that jumped, on a re-estimate frame or
            // any other, leaves the two apart by the jump
            // (`TranscriptJumpTests.theRowCheckSeesAJump`). Not at the top,
            // where the scroll view stops short of what was asked.
            tally.add(asked: 0, clip: scroll.contentView.bounds.origin.y - originWas, shown: Self.shown(frames, height: height))
            if scroll.contentView.bounds.origin.y > -scroll.contentInsets.top + 200, abs(tally.lost) > 12 {
                print("ov-708: a fling at \(Int(velocity)) pt/s, live scrolls \(fling.liveStarts) begun, \(fling.liveEnds) ended:\n  "
                    + ticks.map(\.line).joined(separator: "\n  "))
                jumps.append("a fling at \(Int(velocity)) pt/s moved the rows \(Int(tally.lost)) pt less than it scrolled")
            }
            // A fling that didn't scroll checked nothing: it coasts about
            // half a second's worth of its speed, or to the first row.
            #expect(
                room < 200 || abs(fling.moved) >= min(abs(velocity) * 0.2, room * 0.5),
                "\(Int(velocity)) pt/s moved \(Int(fling.moved)) pt of \(Int(room))")
            await T.settle(window, 200)  // capture-exempt: not a capture; a rest between flings
        }
        #expect(jumps.isEmpty, "rows moved other than by the scroll: \(jumps.prefix(12).joined(separator: "; ")) (\(jumps.count) in all)")
    }
}


/// Opt-in (FARCOOLER_CAPTURE_OUT): a trackpad-speed scroll up the session's
/// end as a frame sequence, a window capture after each frame's events, for
/// judging by eye whether anything jumps (ov-691).
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
struct ConversationScrollCaptures {
    @Test func flickFrames() async throws {
        let out = URL(fileURLWithPath: try #require(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"]))
            .appendingPathComponent("scroll-frames")
        try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
        _ = try ConversationScrollTests()
        let (window, _, scroll) = try await ConversationScrollTests.session()
        defer { window.close() }
        // 25 pt a frame is 3,000 pt/s at 120 Hz, a brisk two-finger scroll.
        for frame in 0..<72 {
            let phase: (scroll: Int64, momentum: Int64) = frame == 0 ? (1, 0) : frame == 71 ? (4, 0) : (2, 0)
            let was = scroll.contentView.bounds.origin.y
            if let event = Fling.wheel(25, phase: phase) { scroll.scrollWheel(with: event) }
            // The frame's scroll applied (the scroll view does on its next
            // frame), or, for the gesture's end, nothing to apply.
            try await Capture.keep(
                window, to: out.appendingPathComponent(String(format: "frame-%02d.png", frame)),
                expecting: "frame \(frame)'s scroll applied", until: { phase.scroll == 4 || scroll.contentView.bounds.origin.y < was - 1 })
        }
    }
}
