import AppKit
import SwiftUI
import TestWindows

@testable import Far_Cooler

/// Draws a view in a window behind every other (`TestWindow.show`) and reports
/// the views it marked `identified(_:)`, in the order they were drawn, with
/// where. What an empty state offers is a view a test can look for by name.
@MainActor
enum EmptyStateProbe {
    /// What was drawn.
    struct Drawn {
        var views: [(id: String, frame: CGRect)] = []

        var ids: [String] { views.map(\.id) }
        func contains(_ id: String) -> Bool { views.contains { $0.id == id } }
        func count(_ id: String) -> Int { views.filter { $0.id == id }.count }
        func frame(_ id: String) -> CGRect? { views.first { $0.id == id }?.frame }
    }

    private final class Box { var drawn = Drawn() }

    static func draw(
        _ view: some View, width: CGFloat = 900, height: CGFloat = 600, appearance: NSAppearance.Name = .aqua
    ) async -> Drawn {
        let box = Box()
        let host = NSHostingView(
            rootView: view
                .frame(width: width, height: height)
                .environment(\.gridProbing, true)
                .overlayPreferenceValue(ProbedViewsKey.self) { probed in
                    GeometryReader { proxy in
                        let _ = box.drawn.views = probed.map { ($0.id, proxy[$0.bounds]) }
                        Color.clear
                    }
                })
        let window = NSWindow(
            contentRect: NSRect(x: -4000, y: -4000, width: width, height: height),
            styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.appearance = NSAppearance(named: appearance)
        window.contentView = host
        TestWindow.show(window)
        for _ in 0..<15 {
            host.layoutSubtreeIfNeeded()
            try? await Task.sleep(for: .milliseconds(20))
        }
        window.close()
        return box.drawn
    }
}
