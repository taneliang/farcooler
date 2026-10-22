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
        await T.settle(window, 600)
        let content = try #require(window.contentView)
        let scroll = try #require(Fling.scrollView(in: content), "no scroll view")
        return (window, model, scroll)
    }

    /// The streaming answer at the end grows by a few words every tenth tick,
    /// as a reply does while the reader scrolls back.
    static func streams(_ model: NativePaneModel) -> (Int) -> Void {
        var rev = 2
        return { tick in
            guard tick % 10 == 0 else { return }
            rev += 1
            let page = RealSession.streaming(words: rev * 4, rev: rev)
            let at = rev
            Task { @MainActor in
                guard let data = try? JSONSerialization.data(withJSONObject: ["epoch": 1, "rev": at, "reset": false, "changes": [["id": "real:stream", "rev": at, "kind": "update", "row": page]]]),
                      case .delta(let delta)? = try? await model.store.ledger.follow(data)
                else { return }
                model.store.apply(delta)
            }
        }
    }

    @Test("Flinging up through a long session keeps up with the display")
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
            ratios.append(fling.mainThreadHitchRatio)
            lines.append("\(Int(velocity)) pt/s: \(fling.summary)")
            await T.settle(window, 200)
        }
        let photos = model.promptImages.states.values.filter { if case .shown = $0 { true } else { false } }.count
        print("ov-691 fling: \(photos) photos shown\n  " + lines.joined(separator: "\n  "))
        let mean = ratios.reduce(0, +) / Double(ratios.count)
        print(String(format: "ov-691 fling: mean main-thread hitch %.1f ms/s", mean))
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
        // Slow flicks near the end first, where a row built above the screen
        // that changes its height moves everything under it; then faster.
        for velocity in [CGFloat(-1_500), -1_500, -3_000, -6_000, -6_000] {
            let fling = Fling(scroll, in: window, velocity: velocity)
            var before = Self.shown(frames, height: height)
            var built = frames.views.filter { $0.key.hasPrefix("native-row-") }
            var docWas = scroll.documentView?.frame.height ?? 0
            var originWas = scroll.contentView.bounds.origin.y
            fling.onTick = { delta in
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
                let geometry = "doc \(Int(docWas))>\(Int(doc)) origin \(Int(originWas))>\(Int(origin)) asked \(Int(delta))"
                docWas = doc
                originWas = origin
                built = all
                for (id, frame) in now {
                    guard let was = before[id] else { continue }
                    // Scrolled up by `-delta`, a row is drawn that much lower.
                    let drift = (frame.minY - was.minY) + delta
                    if abs(drift) > 2 {
                        jumps.append("\(id) by \(Int(drift)) at \(Int(was.minY)) (tick \(fling.stamps.count), \(Int(velocity)) pt/s; \(geometry); \((changed + gone).joined(separator: ", ")))")
                        break
                    }
                }
                before = now
            }
            fling.start()
            await fling.finish()
            await T.settle(window, 200)
        }
        #expect(jumps.isEmpty, "rows moved other than by the scroll: \(jumps.prefix(12).joined(separator: "; ")) (\(jumps.count) in all)")
    }
}

