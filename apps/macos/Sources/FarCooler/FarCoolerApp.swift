import SwiftUI

struct FarCoolerApp: App {
    /// Present only to catch the APNs device token, which arrives nowhere else.
    @NSApplicationDelegateAdaptor(PushDelegate.self) private var pushDelegate
    @State private var showsCLIToolsPrompt = false

    var body: some Scene {
        WindowGroup {
            ContentView()
                .onAppear {
                    Appearance.apply(Preferences.shared.appearance)
                    promptForCLIToolsIfNeeded()
                }
                // Small enough that revealing the sidebar never has to grow the
                // window.
                //
                // This was 900×560, and that one number caused every sidebar
                // complaint at once. A 900 minimum meant a 900-wide window was
                // sitting exactly AT its minimum, so ⌘B could not give the
                // sidebar's width back out of the detail — it had to widen the
                // window instead. Hence the window creeping left and growing on
                // every reveal, and hence the jerk: an AppKit window resize
                // running against the sidebar's own slide animation.
                //
                // 640×420 is a real minimum — enough for a usable terminal beside
                // the sidebar — rather than a preferred size expressed as a floor.
                .frame(minWidth: 600, minHeight: 400)
                .alert("Install command-line tools?", isPresented: $showsCLIToolsPrompt) {
                    Button("Install") { CommandLineTools().install() }
                    Button("Not Now", role: .cancel) {}
                } message: {
                    Text(
                        "Lets your terminal and SSH sessions find farcooler. "
                            + "You can always do this later in Settings."
                    )
                }
        }
        .windowStyle(.titleBar)
        .commands { FarCoolerCommands() }

        // A real Settings scene, so ⌘, works the way it does in every other Mac
        // app without anyone wiring it up.
        Settings { SettingsView() }
    }

    /// Catches the person who never opens Settings. Fires at most once, ever,
    /// per machine — the flag is set the moment the decision to show is made,
    /// not from inside the button actions, so quitting with the alert still on
    /// screen can't leave it primed to reappear next launch.
    private func promptForCLIToolsIfNeeded() {
        let key = "hasPromptedCLIToolsInstall"
        guard !UserDefaults.standard.bool(forKey: key) else { return }

        let tools = CommandLineTools()
        guard tools.state == .notInstalled else { return }

        UserDefaults.standard.set(true, forKey: key)
        showsCLIToolsPrompt = true
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
        if let path = ProcessInfo.processInfo.environment["FARCOOLER_RENDER_PROBE"] {
            MainActor.assumeIsolated { RenderProbe.run(writingTo: path) }
        }
        if let action = ProcessInfo.processInfo.environment["FARCOOLER_SERVICE_PROBE"] {
            MainActor.assumeIsolated { ServiceProbe.run(action) }
        }
        if let action = ProcessInfo.processInfo.environment["FARCOOLER_CLI_TOOLS_PROBE"] {
            MainActor.assumeIsolated { CLIToolsProbe.run(action) }
        }
        FarCoolerApp.main()
    }
}
