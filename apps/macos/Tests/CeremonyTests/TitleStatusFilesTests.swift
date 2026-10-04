import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Show Files (ov-189) beside Open in Editor and Changes takes room the
/// status area counts (`TitleStatusRoom.files`): in a real window at every
/// width the owner uses, the area takes the form its room says, and no item
/// goes to the overflow menu (integ-8, where ov-189 met ov-214).
@MainActor
@Suite(.serialized)
struct TitleStatusFilesTests {
    typealias Harness = TitleBarHarness

    /// The owner's widths, and the window's minimum: at the minimum, every
    /// item fits too. With the minimum back at 600, the tray overflows and
    /// this goes red (integ-9).
    nonisolated static let widths: [CGFloat] = [1790, 1200, 900, 700, MainWindowChrome.minimumWidth]

    private static func root(files: Bool) -> Harness.Root<Color> {
        Harness.Root(words: Harness.Words(), files: files, content: Color.clear)
    }

    @Test("With Show Files in the toolbar, the status area takes the form its room says and pushes nothing out", arguments: widths)
    func fitsWithFiles(width: CGFloat) async throws {
        let wide = try await Harness.window(Self.root(files: true), width: 1790)
        let everything = Harness.itemsShown(in: wide)
        wide.close()

        let window = try await Harness.window(Self.root(files: true), width: width)
        defer { window.close() }
        let form = TitleStatusRoom(
            switcherTitle: "Main", switcherRepository: "overnight", editor: true, changes: true, files: true,
            trouble: nil, needsYou: 11
        ).form(window: width)
        let status = try #require(Harness.status(in: window), "no status area in the toolbar at \(width)")
        #expect(status.width == form.width, "at \(width): \(status.width) wide, not \(form)'s \(form.width)")
        #expect(Harness.itemsShown(in: window) == everything, "at \(width) an item went to the overflow menu")
        let switcher = try #require(Harness.switcher(in: window))
        #expect(status.frame.minX >= switcher.maxX, "at \(width) the status area overlaps the switcher")
    }

    /// The widths where counting Show Files changes the form: one point
    /// narrower than the room without it would still take the wider form.
    static func boundaries() -> [CGFloat] {
        func room(_ files: Bool) -> TitleStatusRoom {
            TitleStatusRoom(
                switcherTitle: "Main", switcherRepository: "overnight", editor: true, changes: true, files: files,
                trouble: nil, needsYou: 11)
        }
        return stride(from: CGFloat(600), through: 1790, by: 1).filter { w in
            room(true).form(window: w) != room(false).form(window: w)
                && room(true).form(window: w - 1) == room(false).form(window: w - 1)
        }.map { $0 + 8 }
    }

    /// Every toolbar item's frame in the window, but the status area's.
    static func otherItems(in window: NSWindow, status: CGRect) -> [CGRect] {
        var frames: [CGRect] = []
        func walk(_ view: NSView) {
            if String(describing: type(of: view)).hasPrefix("ToolbarItemHostingView"), !view.isHidden,
                view.frame.width > 0
            {
                let frame = view.convert(view.bounds, to: nil)
                if !frame.contains(CGPoint(x: status.midX, y: status.midY)) { frames.append(frame) }
            }
            for sub in view.subviews { walk(sub) }
        }
        if let root = window.contentView?.superview { walk(root) }
        return frames
    }

    @Test("Where Show Files narrows the form, the status area overlaps no other item")
    func filesCountsWhereItMatters() async throws {
        let widths = Self.boundaries()
        #expect(!widths.isEmpty, "counting Show Files never changes the form")
        for width in widths {
            let window = try await Harness.window(Self.root(files: true), width: width)
            defer { window.close() }
            let status = try #require(Harness.status(in: window), "no status area at \(width)")
            for other in Self.otherItems(in: window, status: status.frame) {
                #expect(!other.intersects(status.frame), "at \(width) the status area \(status.frame) overlaps \(other)")
            }
        }
    }

    @Test("Show Files is an item of its own, so the room's count of it is real")
    func filesIsAnItem() async throws {
        let without = try await Harness.window(Self.root(files: false), width: 1790)
        let before = Harness.itemsShown(in: without)
        without.close()
        let with = try await Harness.window(Self.root(files: true), width: 1790)
        defer { with.close() }
        #expect(Harness.itemsShown(in: with) > before)
    }
}
