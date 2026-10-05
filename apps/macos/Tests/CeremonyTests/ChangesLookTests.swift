import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The Changes pane's chrome (ov-224): warning strips are inset amber rows, not
/// full-bleed orange bands, and the file filter is the system field.
@MainActor
struct ChangesLookTests {
    private func bitmap(_ appearance: NSAppearance.Name, scale: Int) throws -> NSBitmapImageRep {
        let worktree = Worktree(
            id: "co", short: "co", task: "overnight", branch: "main", repository: "overnight", host: "",
            path: "/tmp/overnight", state: "active", terminals: [])
        let store = ChangesStore(client: DaemonClient(target: ""), worktree: worktree)
        store.changeSet = ChangeSet(
            branch: "b", baseRef: "main", baseSource: "guessed", baseCommit: "", headCommit: "", insertions: 1, deletions: 0,
            commits: [], files: [ChangedFile(path: "a.swift", status: .modified, oldPath: nil, insertions: 1, deletions: 0, binary: false)],
            workingTree: nil)
        store.selectedFile = "a.swift"
        let size = CGSize(width: 980, height: 300)
        let host = NSHostingView(rootView: ChangesPane(changes: store, isFocused: true, agents: []).frame(width: size.width, height: size.height))
        host.appearance = NSAppearance(named: appearance)
        host.frame = CGRect(origin: .zero, size: size)
        host.layoutSubtreeIfNeeded()
        return try #require(host.lookBitmap(scale: scale))
    }

    @Test("The guessed-base warning is an amber row inset from the pane's edges, not a band across it", arguments: lookScales)
    func theWarningIsAnInsetRow(scale: Int) throws {
        for name in [NSAppearance.Name.aqua, .darkAqua] {
            let rep = try bitmap(name, scale: scale)
            // The row sits in the first 30 pt: 8 pt in from the left edge.
            let edge = rep.color(atPoint: 2, 14)
            let inside = rep.color(atPoint: 40, 14)
            #expect(edge != inside, "the warning reaches the pane's edge in \(name.rawValue) at \(scale)x")
            // And it is amber over the paper: warmer than the paper beside it.
            #expect(inside.redComponent > inside.blueComponent, "\(name.rawValue) at \(scale)x")
        }
    }

    // Off on CI (ov-287): CI's headless runner draws the system text field
    // without its white well at either scale (1 row lighter than the paper,
    // where every local Mac draws about 22), so this pixel check describes
    // the OS's drawing there, not the app's. It still runs in every train's
    // local gates, where it goes red on a gray wash.
    @Test(
        "The file filter is the system's rounded field: a well lighter than the paper around it, not a gray wash",
        .enabled(if: ProcessInfo.processInfo.environment["CI"] == nil, "CI draws the system field without its well"),
        arguments: lookScales)
    func theFilterIsTheSystemField(scale: Int) throws {
        let rep = try bitmap(.aqua, scale: scale)
        // The paper beside the column's left edge, below the field. Measured at
        // 0.98 here; the well is 1.00, a 0.02 step that holds at both scales
        // because the field's fill is a solid color, not a gradient or a hairline.
        let paper = rep.color(atPoint: 2, 70).brightnessComponent
        // The field is the first thing under the strip, at the column's left.
        var wells = 0
        for y in 30..<90 where rep.color(atPoint: 100, y).brightnessComponent > paper + 0.01 { wells += 1 }
        // A gray wash is darker than the paper, so it never counts; the real
        // well is about 22 pt tall, and 8 is well under that.
        #expect(wells > 8, "no system text field was drawn at \(scale)x: \(wells) rows lighter than the paper (\(paper))")
    }
}
