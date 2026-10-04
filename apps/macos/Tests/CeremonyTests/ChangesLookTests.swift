import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The Changes pane's chrome (ov-224): warning strips are inset amber rows, not
/// full-bleed orange bands, and the file filter is the system field.
@MainActor
struct ChangesLookTests {
    private func bitmap(_ appearance: NSAppearance.Name) throws -> NSBitmapImageRep {
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
        let rep = try #require(host.bitmapImageRepForCachingDisplay(in: host.bounds))
        host.cacheDisplay(in: host.bounds, to: rep)
        return rep
    }

    private func color(_ rep: NSBitmapImageRep, _ x: Int, _ y: Int) throws -> NSColor {
        try #require(rep.colorAt(x: x * 2, y: y * 2)).usingColorSpace(.sRGB)!
    }

    @Test("The guessed-base warning is an amber row inset from the pane's edges, not a band across it")
    func theWarningIsAnInsetRow() throws {
        for name in [NSAppearance.Name.aqua, .darkAqua] {
            let rep = try bitmap(name)
            // The row sits in the first 30 pt: 8 pt in from the left edge.
            let edge = try color(rep, 2, 14)
            let inside = try color(rep, 40, 14)
            #expect(edge != inside, "the warning reaches the pane's edge in \(name.rawValue)")
            // And it is amber over the paper: warmer than the paper beside it.
            #expect(inside.redComponent > inside.blueComponent, "\(name.rawValue)")
        }
    }

    @Test("The file filter is the system's rounded field: a white well in light, not a gray wash")
    func theFilterIsTheSystemField() throws {
        let rep = try bitmap(.aqua)
        // The field is the first thing under the strip, at the column's left.
        var wells = 0
        for y in 30..<90 where try color(rep, 100, y).brightnessComponent > 0.97 { wells += 1 }
        #expect(wells > 8, "no system text field was drawn")
    }
}
