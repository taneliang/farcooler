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
            offersWrites: offersWrites, choice: .constant(.auto), drawn: .list,
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
}
