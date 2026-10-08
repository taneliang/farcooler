import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// A card's question lists every option as a radio row with its whole text,
/// then an Answer button (ov-431). Choosing sends nothing; Answer sends the
/// chosen option's text, once. Drawn offscreen in a real window and pressed
/// as a pointer would, so the rows, their wrapping and the send are the view's.
@MainActor
@Suite(.serialized)
struct TaskQuestionOptionsTests {
    static let options = [
        "Overlay on hover: the actions float over the row's trailing edge with a fade, so the text uses the full width",
        "Keep the reserved width",
        "Always show the actions, dimmed until the pointer is over the row",
    ]

    final class Sent { var answers: [String] = [] }

    struct Hosted: View {
        let width: CGFloat
        let options: [String]
        let sent: Sent
        let seen: NavigatorFilterTests.Seen

        var body: some View {
            QuestionAnswers(
                offer: TaskCard.Offer(
                    question: TaskQuestion(id: "q", body: "Which?", options: options), options: options, typed: false),
                onAnswer: { answer in
                    sent.answers.append(answer)
                    return true
                },
                draft: .none
            )
            .padding(.horizontal, 12)
            .frame(width: width, alignment: .topLeading)
            .frame(maxHeight: .infinity, alignment: .topLeading)
            .environment(\.gridProbing, true)
            .overlayPreferenceValue(ProbedViewsKey.self) { probed in
                GeometryReader { proxy in
                    let _ = seen.views = Dictionary(
                        probed.map { ($0.id, proxy[$0.bounds]) }, uniquingKeysWith: { first, _ in first })
                    Color.clear
                }
            }
        }
    }

    @MainActor final class Drawn {
        let seen = NavigatorFilterTests.Seen()
        let sent = Sent()
        var host: NSHostingView<Hosted>!
        var window: NavigatorFilterTests.KeyWindow!
        let height: CGFloat = 700

        func settle() async {
            for _ in 0..<15 {
                host.layoutSubtreeIfNeeded()
                try? await Task.sleep(for: .milliseconds(20))
            }
        }

        func press(_ id: String) -> Bool {
            guard let frame = seen.views[id] else { return false }
            let at = NSPoint(x: frame.midX, y: height - frame.midY)
            for type in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
                window.sendEvent(
                    NSEvent.mouseEvent(
                        with: type, location: at, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                        windowNumber: window.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 1)!)
            }
            return true
        }
    }

    static func draw(width: CGFloat, options: [String] = options) async -> Drawn {
        let drawn = Drawn()
        drawn.host = NSHostingView(rootView: Hosted(width: width, options: options, sent: drawn.sent, seen: drawn.seen))
        drawn.window = NavigatorFilterTests.KeyWindow(
            contentRect: NSRect(x: -4000, y: -4000, width: width, height: drawn.height), styleMask: [.borderless],
            backing: .buffered, defer: false)
        drawn.window.isReleasedWhenClosed = false
        drawn.window.contentView = drawn.host
        drawn.window.makeKeyAndOrderFront(nil)
        await drawn.settle()
        return drawn
    }

    @Test("Every option is its own row with its whole text, at a narrow and a wide width")
    func optionsWrapInFull() async throws {
        let wide = await Self.draw(width: 900)
        defer { wide.window.close() }
        let narrow = await Self.draw(width: 300)
        defer { narrow.window.close() }
        let oneLine = try #require(wide.seen.views["task-card-option-1"]).height
        for (i, _) in Self.options.enumerated() {
            let w = try #require(wide.seen.views["task-card-option-\(i)"], "option \(i) wide")
            let n = try #require(narrow.seen.views["task-card-option-\(i)"], "option \(i) narrow")
            #expect(n.maxX <= 300, "option \(i) stays inside the card")
            // The same text takes more lines in less width: it wraps and is not cut.
            #expect(n.height >= w.height - 1, "option \(i) narrow \(n.height) vs wide \(w.height)")
        }
        // The long first option: one line wide, several narrow.
        let long = try #require(narrow.seen.views["task-card-option-0"])
        #expect(long.height > oneLine * 2.5, "the long option wraps in full at 300: \(long.height) vs \(oneLine)")
        // Stacked, not side by side.
        let a = try #require(narrow.seen.views["task-card-option-0"])
        let b = try #require(narrow.seen.views["task-card-option-1"])
        #expect(b.minY >= a.maxY - 1, "options stack")
    }

    @Test("Choosing an option sends nothing; Answer sends the chosen option's text once")
    func choosingThenAnswering() async throws {
        let drawn = await Self.draw(width: 600)
        defer { drawn.window.close() }
        // Answer with nothing chosen does nothing.
        #expect(drawn.press("task-card-answer"))
        await drawn.settle()
        #expect(drawn.sent.answers.isEmpty, "Answer is off until a choice")
        #expect(drawn.press("task-card-option-1"))
        await drawn.settle()
        #expect(drawn.sent.answers.isEmpty, "choosing sends nothing")
        #expect(drawn.press("task-card-answer"))
        for _ in 0..<50 where drawn.sent.answers.isEmpty { await drawn.settle() }
        #expect(drawn.sent.answers == ["Keep the reserved width"])
    }
}
