import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// ov-81 P1: the board header never asks for more width than the board has,
/// from the 1180 pt window's board down to the board's minimum.
@MainActor
struct BoardHeaderFitTests {
    private func width(
        of title: String, waiting: Int, offersWrites: Bool, trouble: String? = nil,
        proposed: CGFloat
    ) -> CGFloat {
        let header = BoardHeader(
            title: title, waiting: waiting, reading: false, trouble: trouble,
            offersWrites: offersWrites,
            newTaskOpen: .constant(false), onCreate: { _ in true }, onRefresh: {})
        let host = NSHostingController(rootView: header)
        return host.sizeThatFits(in: CGSize(width: proposed, height: 200)).width
    }

    @Test("The header fits every board width from the minimum up", arguments: [
        WorkspaceColumns.boardMinimum, 300, 340, 400, 480, 640, 900, 1180,
    ])
    func fitsAtEveryWidth(board: CGFloat) {
        for (title, waiting) in [("Billing", 0), ("Billing · shop", 3), ("A Rather Long Workspace Name Indeed", 12)] {
            let used = width(
                of: title, waiting: waiting, offersWrites: true,
                trouble: "Couldn’t refresh", proposed: board)
            #expect(used <= board, "\(title) at \(board) asked for \(used)")
        }
    }

    /// Drawn at exactly `board` wide, whether anything is inked in the 8 pt
    /// at either edge, where the header's padding (34 pt leading, 16
    /// trailing) should be bare. A row wider than the board is centered and
    /// clipped, so its controls land there.
    private func edgesAreBare(board: CGFloat, title: String, waiting: Int) -> Bool {
        let header = BoardHeader(
            title: title, waiting: waiting, reading: false, trouble: "Couldn’t refresh",
            offersWrites: true,
            newTaskOpen: .constant(false), onCreate: { _ in true }, onRefresh: {})
        // No window and a bitmap of its own at a fixed 2x: a window's minimum
        // width, the screen's backing scale and the machine's appearance all
        // differ between a desk and a CI runner, and none of them is what's
        // being measured. (ov-82: this failed at 280 on CI and passed here.)
        let height: CGFloat = 44
        let host = NSHostingView(rootView: header.frame(width: board, height: height))
        host.appearance = NSAppearance(named: .aqua)
        host.frame = NSRect(x: 0, y: 0, width: board, height: height)
        host.layoutSubtreeIfNeeded()
        let scale: CGFloat = 2
        guard let rep = NSBitmapImageRep(
            bitmapDataPlanes: nil, pixelsWide: Int(board * scale), pixelsHigh: Int(height * scale),
            bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
            colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)
        else { return false }
        rep.size = host.bounds.size
        host.cacheDisplay(in: host.bounds, to: rep)
        let edge = Int(8 * scale)
        for y in 0..<rep.pixelsHigh {
            let reference = rep.colorAt(x: edge + 2, y: y)
            for x in list(0..<edge) + list((rep.pixelsWide - edge)..<rep.pixelsWide) {
                guard let c = rep.colorAt(x: x, y: y), let r = reference else { return false }
                if abs(c.brightnessComponent - r.brightnessComponent) > 0.04 { return false }
            }
        }
        return true
    }

    private func list(_ r: Range<Int>) -> [Int] { Array(r) }

    @Test("No control is drawn into the header's edge padding at 1180 or the minimum",
        arguments: [WorkspaceColumns.boardMinimum, 340, 1180])
    func controlsStayInside(board: CGFloat) {
        for (title, waiting) in [("Billing", 0), ("A Rather Long Workspace Name Indeed", 12)] {
            #expect(edgesAreBare(board: board, title: title, waiting: waiting), "\(title) at \(board)")
        }
    }

    /// The summary strip, with rows and a long title, never asks for more than
    /// the board has either.
    @Test("The summary strip fits the board at the minimum and at 1180", arguments: [
        WorkspaceColumns.boardMinimum, 1180,
    ])
    func stripFits(board: CGFloat) {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        let store = TaskBoardStore(client: client, workspace: .implicit(repository: "r"))
        let strip = BoardSummaryStrip(
            store: store, defaults: UserDefaults(suiteName: "strip-fit-\(UUID().uuidString)")!)
        let host = NSHostingController(rootView: strip)
        let used = host.sizeThatFits(in: CGSize(width: board, height: 400)).width
        #expect(used <= board, "strip at \(board) asked for \(used)")
    }
}
