import SwiftUI

struct OvernightApp: App {
    var body: some Scene {
        WindowGroup {
            ContentView()
                .frame(minWidth: 900, minHeight: 560)
        }
        .windowStyle(.titleBar)
        .commands {
            CommandGroup(replacing: .newItem) {}
        }
    }
}

/// The entry point.
///
/// Written out rather than using `@main` on the App so the render probe can run
/// before AppKit takes over the process. Everything else falls straight through
/// to the normal app.
@main
enum Entry {
    static func main() {
        if let path = ProcessInfo.processInfo.environment["OVERNIGHT_RENDER_PROBE"] {
            MainActor.assumeIsolated { RenderProbe.run(writingTo: path) }
        }
        OvernightApp.main()
    }
}
