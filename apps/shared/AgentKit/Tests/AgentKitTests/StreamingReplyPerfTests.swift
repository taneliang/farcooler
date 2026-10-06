// ov-382: a long reply streams without its whole text being drawn again on
// every delta.
//
// The forensics (ov-358 §3) found the main thread re-parsing and re-laying
// out the WHOLE reply on each 200 ms poll: 34% of the main thread at 12.8K
// characters and 97% at 25.6K. What a delta should cost is the paragraph it
// lands in. The gate counts that, as the pieces whose `body` a delta runs,
// since a count holds on any machine and a time doesn't; the times are
// printed beside it for the report.
#if os(macOS)
import AppKit
import SwiftUI
import Testing
@testable import AgentKit

@MainActor
@Suite(.serialized)
struct StreamingReplyPerfTests {
    /// Prose shaped like a reply: inline markup, a paragraph break every few
    /// sentences, a short list now and then. The same generator the
    /// forensics measured with. Without `lists`, one prose run of
    /// paragraphs, as a long explanation is.
    static func prose(_ chars: Int, seed: Int, lists: Bool = true) -> String {
        let words = "the quick brown fox jumps over a lazy dog while `code` and **bold** text with [links](https://example.com) flow by in a reply"
            .split(separator: " ").map(String.init)
        var out = ""
        var i = seed
        while out.count < chars {
            out += words[i % words.count] + " "
            i += 7
            if i % 23 == 0 { out += "\n\n" }
            if lists, i % 97 == 0 { out += "\n\n- a bullet item\n- another one\n\n" }
        }
        return out
    }

    /// A reply hosted in an offscreen window, drawn the way a transcript
    /// row is: laid out at the row's width and displayed.
    final class Host {
        let window: NSWindow
        let host: NSHostingView<AnyView>

        init(width: CGFloat = 760) {
            host = NSHostingView(rootView: AnyView(EmptyView()))
            window = NSWindow(
                contentRect: NSRect(x: -9000, y: -9000, width: width, height: 900),
                styleMask: [.borderless], backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.contentView = host
            window.orderFrontRegardless()
        }

        /// One delta's worth of work: the new text, laid out and drawn.
        func show(_ text: String, streaming: Bool) -> Double {
            let start = ContinuousClock.now
            host.rootView = AnyView(
                AgentReplyText(text: text, trailingClearance: 32, streaming: streaming)
                    .frame(width: 740))
            host.layoutSubtreeIfNeeded()
            window.displayIfNeeded()
            let d = ContinuousClock.now - start
            return Double(d.components.seconds) * 1000 + Double(d.components.attoseconds) / 1e15
        }

        func close() { window.close() }
    }

    /// The cheapest of `deltas` successive deltas into a reply that starts
    /// at `chars` characters. The minimum, not the median: noise from
    /// whatever else the machine is doing only ever adds.
    static func perDelta(at chars: Int, lists: Bool = true, deltas: Int = 16) -> Double {
        let streaming = true
        let host = Host()
        defer { host.close() }
        var text = prose(chars, seed: 5, lists: lists)
        _ = host.show(text, streaming: streaming)
        _ = host.show(text, streaming: streaming)
        var best = Double.infinity
        for k in 0..<deltas {
            // Words, as `stream_to_events` cuts them, into the open paragraph.
            text += ["streaming ", "words ", "arrive ", "a few ", "at a time "][k % 5]
            best = min(best, host.show(text, streaming: streaming))
        }
        return best
    }

    /// The bytes of text one delta into a reply of `chars` draws again.
    static func bytesPerDelta(at chars: Int, lists: Bool) -> Int {
        let host = Host()
        defer { host.close() }
        var text = prose(chars, seed: 5, lists: lists)
        _ = host.show(text, streaming: true)
        text += "streaming "
        _ = host.show(text, streaming: true)
        text += "words "
        let before = MarkdownPiece.drawnBytes
        _ = host.show(text, streaming: true)
        return MarkdownPiece.drawnBytes - before
    }

    @Test(arguments: [true, false])
    func aDeltaRedrawsItsParagraphNotTheWholeReply(lists: Bool) {
        let reply = Self.prose(25_600, seed: 5, lists: lists)
        let redrawn = Self.bytesPerDelta(at: 25_600, lists: lists)
        let small = Self.perDelta(at: 1_600, lists: lists)
        let large = Self.perDelta(at: 25_600, lists: lists)
        print(String(
            format: "ov-382 (%@) a delta into %d bytes redraws %d; per delta: 1.6K %.2f ms, 25.6K %.2f ms",
            lists ? "lists" : "one prose run", reply.utf8.count, redrawn, small, large))
        // The open paragraph, and the one before it when the delta closed
        // it: a few hundred bytes. Before ov-382 it was the whole reply, or
        // without the split, the whole of its last prose run.
        #expect(redrawn < 2_000, "a delta into \(reply.utf8.count) bytes redrew \(redrawn)")
    }

    /// Streaming, only the last piece is open and drawn without selection;
    /// the runs before the last are the settled reply's own, so settling
    /// redraws only the last; and settled, every piece is selectable and a
    /// prose run is one piece, selectable across its paragraphs.
    @Test func streamingDiffersFromSettledOnlyAtTheTail() {
        let text = Self.prose(8_000, seed: 5) + "\n\nOne more paragraph.\n\nAnother.\n\nAnd the one being written"
        let streaming = MarkdownText.pieces(text, streaming: true)
        let settled = MarkdownText.pieces(text)
        #expect(streaming.dropLast().allSatisfy { !$0.open })
        #expect(streaming.last?.open == true)
        #expect(settled.allSatisfy { !$0.open })
        guard case let .prose(tail)? = settled.last?.run else {
            Issue.record("the reply should end in prose")
            return
        }
        #expect(tail.count >= 3, "a settled prose run is gathered, so a selection can cross it")
        #expect(streaming.count == settled.count + tail.count - 1)
        #expect(Array(streaming.prefix(settled.count - 1)) == Array(settled.prefix(settled.count - 1)))
        #expect(streaming.suffix(tail.count).map(\.run) == tail.map { .prose([$0]) })
    }

    /// A streaming reply's pieces leave the merged-text memo alone: their
    /// entries would never be read again once the run is gathered, and they
    /// evict settled rows' (ov-382 review). A settled reply uses it.
    @Test func aStreamingReplyLeavesTheMemoAlone() {
        let text = Self.prose(8_000, seed: 5, lists: false)
        Markdown.mergedCache.removeAll()
        let host = Host()
        defer { host.close() }
        _ = host.show(text, streaming: true)
        _ = host.show(text + "more ", streaming: true)
        #expect(Markdown.mergedCache.misses == 0, "streaming took \(Markdown.mergedCache.misses) memo slots")
        _ = host.show(text, streaming: false)
        #expect(Markdown.mergedCache.misses > 0, "a settled reply skipped the memo")
    }

    /// Settling a reply redraws the pieces of its last run, not the reply.
    @Test func settlingRedrawsOnlyTheLastRun() {
        let host = Host()
        defer { host.close() }
        let text = Self.prose(25_600, seed: 5)
        _ = host.show(text, streaming: true)
        _ = host.show(text, streaming: true)
        let before = MarkdownPiece.drawnBytes
        _ = host.show(text, streaming: false)
        let redrawn = MarkdownPiece.drawnBytes - before
        guard case let .prose(last)? = MarkdownText.pieces(text).last?.run else {
            Issue.record("the reply should end in prose")
            return
        }
        let lastRun = last.reduce(0) { $0 + $1.utf8.count }
        #expect(redrawn <= lastRun, "settling redrew \(redrawn) bytes; its last run is \(lastRun)")
    }
}
#endif
