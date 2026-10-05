import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The Changes pane's chrome (ov-224): warning strips are inset amber rows, not
/// full-bleed orange bands, and the file filter is the system field.
@MainActor
struct ChangesLookTests {
    private func filterHost(_ appearance: NSAppearance.Name) throws -> NSHostingView<some View> {
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
        return host
    }

    private func bitmap(_ appearance: NSAppearance.Name, scale: Int) throws -> NSBitmapImageRep {
        try #require(filterHost(appearance).lookBitmap(scale: scale))
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

    /// Every view under `root`, depth first.
    private func descendants(of root: NSView) -> [NSView] { [root] + root.subviews.flatMap { descendants(of: $0) } }

    // Structural, so it holds on any OS's drawing (ov-287): CI's headless
    // runner draws the system field without its white well, so a pixel check
    // there describes the OS, not the app. The filter must be an AppKit text
    // field with the rounded bezel the `.roundedBorder` style asks for. A gray
    // fill, a plain field or a hand-drawn shape has no bezeled NSTextField.
    @Test("The file filter is the system's rounded field, not a gray wash", arguments: lookScales)
    func theFilterIsTheSystemField(scale: Int) throws {
        let host = try filterHost(.aqua)
        let fields = descendants(of: host).compactMap { $0 as? NSTextField }.filter { $0.placeholderString == "Filter files" || ($0.cell as? NSTextFieldCell)?.placeholderString == "Filter files" }
        let field = try #require(fields.first, "no AppKit text field carries the \"Filter files\" prompt at \(scale)x")
        #expect(field.isBezeled, "the filter is not bezeled: it is drawn by hand")
        #expect(field.bezelStyle == .roundedBezel, "the filter's bezel is not the system's rounded one")
        #expect(field.isEditable)
    }
}
