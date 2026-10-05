#if os(macOS)
import AppKit
import Foundation
import SwiftUI
import Testing

@testable import AgentKit

/// Keys in running text (ov-299): the renderer finds where each key was drawn,
/// and draws the text exactly as `Text` does.
@MainActor
struct TaskKeyTextTests {
    static let linker: TaskKeyLinker = {
        let row = TaskRow(id: "t190", key: "ov-190", title: "Fix the login", status: .todo, statusSince: .now)
        let board = TaskBoardModel(columns: [TaskBoardColumn(status: .todo, rows: [row])])
        let workspace = WorkspaceSummary(id: "w", name: "Main", taskPrefix: "ov", isMain: true, ordinal: 0)
        let index = TaskKeyIndex(runner: "r1", workspaces: [workspace], boards: ["w": board])
        return TaskKeyLinker(index: index, cards: TaskKeyCards(runner: "r1", boards: ["w": board]), open: { _ in })
    }()

    /// `view` drawn into a bitmap at its fitting size, `width` wide.
    static func render(_ view: some View, width: CGFloat) -> NSBitmapImageRep {
        let host = NSHostingView(rootView: view.frame(width: width, alignment: .leading).fixedSize(horizontal: false, vertical: true))
        host.frame = CGRect(origin: .zero, size: host.fittingSize)
        host.layoutSubtreeIfNeeded()
        let rep = host.bitmapImageRepForCachingDisplay(in: host.bounds)!
        host.cacheDisplay(in: host.bounds, to: rep)
        return rep
    }

    /// `view` drawn by `ImageRenderer`, the path a window's text takes,
    /// on an opaque ground so every glyph's pixels count.
    static func image(_ view: some View, width: CGFloat) throws -> NSBitmapImageRep {
        let renderer = ImageRenderer(
            content: view.frame(width: width, alignment: .leading).fixedSize(horizontal: false, vertical: true)
                .background(Color.black).environment(\.colorScheme, .dark))
        renderer.scale = 2
        return NSBitmapImageRep(cgImage: try #require(renderer.cgImage))
    }

    /// How many pixels differ between two bitmaps of the same size.
    static func differing(_ a: NSBitmapImageRep, _ b: NSBitmapImageRep) -> Int {
        var count = 0
        for y in 0..<a.pixelsHigh {
            for x in 0..<a.pixelsWide where a.colorAt(x: x, y: y) != b.colorAt(x: x, y: y) { count += 1 }
        }
        return count
    }

    @Test("A key's run is found where it was drawn, in selectable text")
    func findsTheKey() throws {
        let linked = Self.linker.linked(AttributedString("Blocked on ov-190 for now"))
        let marked = Self.linker.markedText(linked)
        #expect(marked.keys == 1)
        let rects = TaskKeyRects()
        _ = Self.render(
            marked.text.textRenderer(TaskKeyRecorder(rects: rects)).textSelection(.enabled), width: 400)
        let found = try #require(rects.all.first)
        #expect(found.key == "ov-190")
        // "Blocked on " is drawn first, so the key starts well right of the
        // edge, and it is a word wide: whole points at 1x and 2x alike.
        #expect(found.rect.minX > 40 && found.rect.width > 20 && found.rect.width < 120)
        #expect(rects.key(at: CGPoint(x: found.rect.midX, y: found.rect.midY))?.key == "ov-190")
        #expect(rects.key(at: CGPoint(x: 2, y: found.rect.midY)) == nil, "the text before it isn't the key")
    }

    @Test("Marking keys draws the same pixels as the linked text")
    func drawsAsText() throws {
        let linked = Self.linker.linked(AttributedString("See ov-190 and ov-999, and ov-190 again."))
        let rects = TaskKeyRects()
        let plain = try Self.image(Text(linked), width: 260)
        let marked = try Self.image(
            Self.linker.markedText(linked).text.textRenderer(TaskKeyRecorder(rects: rects)), width: 260)
        #expect(plain.pixelsWide == marked.pixelsWide && plain.pixelsHigh == marked.pixelsHigh)
        #expect(Self.differing(plain, marked) == 0)
        #expect(rects.all.map(\.key) == ["ov-190", "ov-190"], "ov-999 isn't on a board")
    }

    @Test("VoiceOver hears each known key's title after it")
    func spoken() {
        let linked = Self.linker.linked(AttributedString("Blocked on ov-190, not ov-999."))
        #expect(Self.linker.spoken(linked) == "Blocked on ov-190 (Fix the login), not ov-999.")
        #expect(Self.linker.spoken(Self.linker.linked(AttributedString("Nothing"))) == nil)
    }

    @Test("A plan row's keys are marked for their cards, and drawn as the text around them")
    func plainKeys() throws {
        let plain = AttributedString("Waits on ov-190.")
        let rects = TaskKeyRects()
        let marked = Self.linker.markedText(Self.linker.linked(plain), links: false)
        #expect(marked.keys == 1)
        let drawn = try Self.image(marked.text.textRenderer(TaskKeyRecorder(rects: rects)), width: 260)
        #expect(Self.differing(try Self.image(Text(plain), width: 260), drawn) == 0, "no link color")
        #expect(rects.all.map(\.key) == ["ov-190"])
    }

    @Test("Text without a known key isn't marked")
    func noKeys() {
        let linked = Self.linker.linked(AttributedString("Nothing here, utf-8"))
        #expect(Self.linker.markedText(linked).keys == 0)
    }
}
#endif
