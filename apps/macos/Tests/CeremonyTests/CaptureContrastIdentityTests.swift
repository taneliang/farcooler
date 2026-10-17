import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// A capture's forced contrast keeps the window it's drawn on (ov-600).
///
/// `RealWindowCaptures` lets a window settle, then forces each variant's
/// contrast through `captureContrast()`. When that modifier was an `if`, the
/// first forced value swapped the branch, so everything under it was mounted
/// afresh: every `@State` in `ContentView` was dropped, and each capture drew
/// a window that had just opened (a task's spinner, never its agent).
@MainActor
@Suite(.serialized)
struct CaptureContrastIdentityTests {
    /// Counts each time its view is mounted: a `@StateObject` is made once
    /// per identity, and again only when the view is mounted anew.
    final class Mount: ObservableObject {
        nonisolated(unsafe) static var count = 0
        init() { Self.count += 1 }
    }

    struct Probe: View {
        @StateObject private var mount = Mount()
        @Environment(\.colorSchemeContrast) private var contrast
        let seen: (ColorSchemeContrast) -> Void
        var body: some View {
            let _ = seen(contrast)
            Color.clear
        }
    }

    @Test func forcingContrastKeepsWhatIsMounted() async throws {
        CaptureContrast.shared.forced = nil
        defer { CaptureContrast.shared.forced = nil }
        Mount.count = 0
        var last: ColorSchemeContrast?
        let host = NSHostingView(rootView: Probe { last = $0 }.captureContrast())
        host.frame = CGRect(x: 0, y: 0, width: 20, height: 20)
        host.layoutSubtreeIfNeeded()
        #expect(Mount.count == 1)
        #expect(last == .standard)

        CaptureContrast.shared.forced = .increased
        // Drawn again with the forced value: polled, not slept, so a slow
        // runner waits only as long as it needs.
        let deadline = Date().addingTimeInterval(30)
        while last != .increased, Date() < deadline {
            try await Task.sleep(for: .milliseconds(20))
            host.layoutSubtreeIfNeeded()
        }
        #expect(last == .increased, "the forced contrast never reached the view")
        #expect(Mount.count == 1, "forcing contrast mounted the window's views again")
    }
}
