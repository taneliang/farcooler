import AgentKit
import AppKit
import SwiftUI
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

    /// The session in a 720 by 900 window, at its end, photos coming late.
    static func session(probed frames: NativePolishTests.Frames? = nil) async throws -> (NSWindow, NativePaneModel, NSScrollView) {
        let model = T.model(try T.terminal(id: terminal))
        model.promptImages.source = RealSession.Photos()
        model.keys = RealSession.Keys()
        let view = NativeAgentView(model: model, isFocused: true, showTerminal: {})
        let window = frames.map { NativePolishTests.window(width: 720, height: 900, NativePolishTests.Framed(frames: $0, content: view)) }
            ?? NativePolishTests.window(width: 720, height: 900, view)
        model.store.apply(try await model.store.ledger.page(T.page(RealSession.rows())))
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
    /// work); a regression of that kind lands well over this.
    static let hitchBudget = 40.0

    /// Opt-in, as `NativeStreamingStallTests` is: a timing gate on a shared
    /// machine, with every other suite drawing on the same main thread, can
    /// starve the display link and fail on nothing of its own. Runs with
    /// FARCOOLER_PERF=1. What it caught is gated in every run by AgentKit's
    /// `theNewestTurnIgnoresTheRowsAfterIt`.
    @Test(
        "Flinging up through a long session keeps up with the display, while a reply streams",
        .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_PERF"] == "1"))
    func flingKeepsUp() async throws {
        let (window, model, scroll) = try await Self.session()
        defer { window.close() }
        let stream = Self.streams(model)
        var ratios: [Double] = []
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
            ratios.append(fling.mainThreadHitchRatio)
            lines.append("\(Int(velocity)) pt/s: \(fling.summary)")
            await T.settle(window, 200)  // capture-exempt: not a capture; a rest between flings
        }
        let photos = model.promptImages.states.values.filter { if case .shown = $0 { true } else { false } }.count
        print("ov-691 fling: \(photos) photos shown\n  " + lines.joined(separator: "\n  "))
        let sorted = ratios.sorted()
        let median = (sorted[(sorted.count - 1) / 2] + sorted[sorted.count / 2]) / 2
        print(String(format: "ov-691 fling: median main-thread hitch %.1f ms/s", median))
        #expect(median <= Self.hitchBudget, "scrolling hitched \(median) ms/s:\n\(lines.joined(separator: "\n"))")
    }

    /// Where each row was drawn on screen, by id, that the window shows.
    static func shown(_ frames: NativePolishTests.Frames, height: CGFloat) -> [String: CGRect] {
        frames.views.filter { $0.key.hasPrefix("native-row-") && $0.value.maxY > 0 && $0.value.minY < height }
    }

    @Test("Flicking up, no row on screen moves except by the scroll")
    func noRowJumps() async throws {
        let frames = NativePolishTests.Frames()
        let (window, _, scroll) = try await Self.session(probed: frames)
        defer { window.close() }
        let height = scroll.contentView.bounds.height
        var jumps: [String] = []
        var estimated: [String] = []
        // Slow flicks near the end first, where a row built above the screen
        // that changes its height moves everything under it; then faster,
        // all the way to the first row.
        let velocities = [CGFloat(-1_500), -1_500, -3_000, -6_000, -6_000] + Array(repeating: -12_000, count: 12)
        for velocity in velocities where scroll.contentView.bounds.origin.y > -scroll.contentInsets.top + 1 {
            let fling = Fling(scroll, in: window, velocity: velocity)
            let room = scroll.contentView.bounds.origin.y + scroll.contentInsets.top
            var before = Self.shown(frames, height: height)
            var built = frames.views.filter { $0.key.hasPrefix("native-row-") }
            var docWas = scroll.documentView?.frame.height ?? 0
            var originWas = scroll.contentView.bounds.origin.y
            fling.onTick = { asked in
                let now = Self.shown(frames, height: height)
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
                // with it, the rows staying where they are on screen: the
                // scroll is then what this tick asked for, not the clip's
                // move. Otherwise it is the clip's move, which follows what
                // was asked a frame or two late.
                let resized = abs(doc - docWas) > 0.5
                let scrolled = resized ? asked : moved
                // At the top the scroll view stops the clip where the
                // content starts, short of what was asked, and springs back.
                // And a tick that came so late its clip moved several ticks'
                // worth at once (a full run's other suites holding the main
                // thread) has rows drawn for a clip between the two: neither
                // can be read. Neither hides a fold, which moves rows on a
                // tick whose clip moved as asked.
                let atTop = origin < -scroll.contentInsets.top + max(abs(asked) * 3, 1_000)
                let late = abs(moved) > abs(asked) * 3 + 20 && !resized
                // Flung up, the list never goes down: nothing pulls the
                // reader back toward the end.
                if !resized, !atTop, moved > 1 { jumps.append("the list went down \(Int(moved)) pt (\(why))") }
                // A fast tick where the lazy stack sized its content again,
                // having guessed thousands of points wrong for rows it never
                // built (8,400 pt seen, about 16,000 pt up the session): the
                // rows can then move up to 340 pt more than the scroll. That
                // is SwiftUI's estimate, not a fold, and it is real; it's
                // kept apart and reported as a known issue below, not let by.
                let estimate = resized && abs(asked) > 40
                for (id, frame) in now where !atTop && !late {
                    guard let was = before[id] else { continue }
                    // Scrolled by `scrolled`, a row is drawn that much the
                    // other way, and no further.
                    let drift = (frame.minY - was.minY) + scrolled
                    // A resize's tick may be off by a little more than a
                    // fling's slowing between two frames, the clip following
                    // what was asked a frame late; never by a fold's 60 pt.
                    if abs(drift) > (resized ? 8 + abs(asked) / 4 : 2) {
                        let jump = "\(id) by \(Int(drift)) at \(Int(was.minY)) (\(why))"
                        if estimate { estimated.append(jump) } else { jumps.append(jump) }
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
            // A fling that didn't scroll checked nothing: it coasts about
            // half a second's worth of its speed, or to the first row.
            #expect(
                room < 200 || abs(fling.moved) >= min(abs(velocity) * 0.2, room * 0.5),
                "\(Int(velocity)) pt/s moved \(Int(fling.moved)) pt of \(Int(room))")
            await T.settle(window, 200)  // capture-exempt: not a capture; a rest between flings
        }
        #expect(jumps.isEmpty, "rows moved other than by the scroll: \(jumps.prefix(12).joined(separator: "; ")) (\(jumps.count) in all)")
        // The lazy stack's own re-estimate on a fast fling far from the end
        // (ov-691 review): a real jump, SwiftUI's, recorded every time it
        // happens and left to its follow-up card rather than hidden.
        withKnownIssue("the lazy stack re-estimates its content mid-fling and rows jump", isIntermittent: true) {
            #expect(estimated.isEmpty, "rows jumped as the lazy stack sized itself again: \(estimated.prefix(6).joined(separator: "; "))")
        }
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
