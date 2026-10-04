import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The long-press history menu (ov-248), as the rows the title bar's Back
/// and Forward hand to the system's menu: the place the window is at
/// checked, Forward's stops above it and Back's below. Drawn as a menu is,
/// in both appearances, so it can be looked at; the real menu is AppKit's
/// and can't be rendered offscreen. Written where `FARCOOLER_GLANCE_OUT`
/// says.
@MainActor
struct HistoryMenuSpecimenTests {
    private static let rows: [PlaceRow] = [
        PlaceRow(spot: .forward(1), title: "Billing", subtitle: nil, symbol: "square.stack.3d.up"),
        PlaceRow(spot: .forward(0), title: "ov-233 Apps: relaunch restores windows", subtitle: "Main", symbol: "checklist"),
        PlaceRow(spot: .current, title: "ov-248 Mac: Back's long press shows history", subtitle: "Main", symbol: "checklist"),
        PlaceRow(spot: .back(0), title: "ov-248-lane", subtitle: "Main", symbol: "arrow.triangle.branch"),
        PlaceRow(spot: .back(1), title: "Done History", subtitle: "Main", symbol: "clock.arrow.circlepath"),
        PlaceRow(spot: .back(2), title: "Needs You", subtitle: nil, symbol: "tray"),
    ]

    private struct Specimen: View {
        var body: some View {
            VStack(alignment: .leading, spacing: 2) {
                ForEach(HistoryMenuSpecimenTests.rows) { row in
                    HStack(spacing: 6) {
                        Image(systemName: "checkmark").frame(width: 14).opacity(row.current ? 1 : 0)
                        Image(systemName: row.symbol).frame(width: 18)
                        Text(row.line).lineLimit(1)
                        Spacer(minLength: 0)
                    }
                    .padding(.horizontal, 10).padding(.vertical, 3)
                }
            }
            .font(.system(size: 13))
            .padding(6)
            .frame(width: 400, alignment: .leading)
            .background(.regularMaterial)  // style-exempt: a specimen of the system menu, not app chrome
            .clipShape(RoundedRectangle(cornerRadius: 10))  // style-exempt: a specimen of the system menu
            .padding(20)
        }
    }

    @Test("Write the history menu sheets")
    func writeSheets() throws {
        let directory = URL(
            fileURLWithPath: ProcessInfo.processInfo.environment["FARCOOLER_GLANCE_OUT"]
                ?? FileManager.default.currentDirectoryPath + "/.build/glance")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        for dark in [false, true] {
            let host = NSHostingView(
                rootView: Specimen().background(dark ? Color(white: 0.12) : Color(white: 0.96)))
            host.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
            host.frame = CGRect(origin: .zero, size: host.fittingSize)
            host.layoutSubtreeIfNeeded()
            let rep = try #require(host.bitmapImageRepForCachingDisplay(in: host.bounds))
            host.cacheDisplay(in: host.bounds, to: rep)
            let png = try #require(rep.representation(using: .png, properties: [:]))
            try png.write(to: directory.appendingPathComponent("history-menu-\(dark ? "dark" : "light").png"))
        }
    }
}
