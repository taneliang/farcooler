import SwiftUI

struct FarCoolerApp: App {
    /// Present to catch the APNs device token, which arrives nowhere else, and
    /// to take notification clicks before launching finishes (`PushDelegate`).
    @NSApplicationDelegateAdaptor(PushDelegate.self) private var pushDelegate
    @State private var showsCLIToolsPrompt = false

    var body: some Scene {
        WindowGroup {
            ContentView()
                .onAppear {
                    Appearance.apply(Preferences.shared.appearance)
                    promptForCLIToolsIfNeeded()
                }
                // A real minimum, enough for a usable terminal beside the
                // navigator, rather than a preferred size expressed as a floor.
                // It was 900×560 while the window had a split-view sidebar, and
                // a window sitting at its minimum had to grow to reveal one;
                // the navigator gives its width out of the detail instead
                // (`WorkspaceColumns`), and the old sidebar is gone (ov-178).
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

        // About is its own small window, so the menu item works with no main
        // window open, and nothing is dimmed behind it.
        Window("About Far Cooler", id: AboutView.windowID) { AboutView() }
            .windowResizability(.contentSize)
            .windowStyle(.hiddenTitleBar)
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
        ignoreSIGPIPE()
        // Before the first CLI spawn, and before the window: the answer is
        // wanted by `daemon ensure` at launch and by every pane opened after
        // it. See `CLI.warmSearchPath` — it returns immediately and resolves
        // on its own thread, so this costs nothing here.
        CLI.warmSearchPath()
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

    /// Writing to a dead pipe must be an error, not a death.
    ///
    /// This app is a fleet of pipes: every pane holds a `farcooler terminal
    /// input` child, and for a remote runner that child is ssh-backed. A
    /// laptop that sleeps drops those connections, the children exit, and the
    /// read end of each pipe closes with them.
    ///
    /// Under the default disposition the next keystroke does not fail — it
    /// kills the app. `write(2)` to a pipe with no reader raises SIGPIPE at the
    /// syscall level and the process is gone before `write` returns anything to
    /// check, so `try?` at the call site cannot catch it: a signal is not a
    /// thrown error. Foundation only converts it into one once SIGPIPE is
    /// ignored, which is what this does.
    ///
    /// Found from a launchd record after an overnight disappearance — "exited
    /// due to SIGPIPE", nine seconds after `kCGSDisplayDidWake` and 0.03s after
    /// the app was brought frontmost. There is no crash report to go with it,
    /// because a process killed by SIGPIPE does not produce one, which is
    /// exactly why it read as the app having quietly vanished in the night.
    ///
    /// Process-wide and before anything else, deliberately. Every pipe write in
    /// the app is covered by this one line, including ones nobody has written
    /// yet, and the alternative — auditing each call site forever — is the kind
    /// of discipline that holds until the first new one.
    private static func ignoreSIGPIPE() {
        signal(SIGPIPE, SIG_IGN)
    }
}
