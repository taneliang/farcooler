import Foundation
import ServiceManagement

/// Registering the daemon to start at login.
///
/// Without this the daemon lives only as long as something starts it, which
/// means a host is reachable only while the user has already opened the app.
/// For a product whose premise is agents working while you are asleep, that is
/// the wrong default: the point is to close the laptop, and still be able to
/// look in from a phone at 3am.
///
/// Registration goes through `SMAppService`, so nothing is ever written to
/// `~/Library/LaunchAgents`. The plist lives inside the bundle and addresses
/// the daemon with `BundleProgram`, so moving or replacing the app moves the
/// daemon with it rather than leaving a launchd job pointing at a path that no
/// longer exists.
@MainActor
final class ServiceRegistration: ObservableObject {
    /// What the system says about the registration, in the user's terms.
    enum State: Equatable {
        case notRegistered
        case registered
        /// Registered, but the user has to approve it in System Settings.
        /// macOS does not let an app enable itself silently.
        case awaitingApproval
        case unavailable(String)

        var isOn: Bool { self == .registered }
    }

    @Published private(set) var state: State = .notRegistered

    private let plistName = "com.farcooler.daemon.plist"

    private var service: SMAppService {
        SMAppService.agent(plistName: plistName)
    }

    init() {
        refresh()
    }

    /// Is the agent plist actually inside this bundle?
    ///
    /// This distinction is why the check exists. `SMAppService` answers
    /// `.notFound` both for "you have never registered this" and for "there is
    /// no such plist", which are opposite situations: the first wants a button
    /// offering to turn it on, the second is a broken build. Only the bundle
    /// can tell them apart.
    private var plistIsBundled: Bool {
        guard let library = Bundle.main.bundleURL.appendingPathComponent("Contents/Library")
            .appendingPathComponent("LaunchAgents")
            .appendingPathComponent(plistName) as URL?
        else { return false }
        return FileManager.default.fileExists(atPath: library.path)
    }

    func refresh() {
        // Running from a bare executable rather than a bundle, SMAppService has
        // no plist to find. Say so plainly instead of reporting "not
        // registered", which would suggest a button would fix it.
        guard Bundle.main.bundleURL.pathExtension == "app" else {
            state = .unavailable("Run Far Cooler from the app bundle to enable this.")
            return
        }

        switch service.status {
        case .enabled: state = .registered
        case .requiresApproval: state = .awaitingApproval
        case .notRegistered: state = .notRegistered
        // `.notFound` is what a never-registered agent reports, so it is the
        // ordinary starting state — not an error — as long as the plist is
        // there to register.
        case .notFound:
            state = plistIsBundled
                ? .notRegistered
                : .unavailable("This build has no LaunchAgent to register.")
        @unknown default:
            state = .unavailable("The system reported a status Far Cooler does not recognise.")
        }
    }

    /// Start the daemon at login from now on.
    func register() {
        do {
            try service.register()
            refresh()
            // An unsigned or ad-hoc-signed build often registers but stays in
            // `requiresApproval` until the user turns it on in System Settings.
            // Reporting that as success would leave them wondering why nothing
            // happened.
            if state == .awaitingApproval {
                openLoginItemsSettings()
            }
        } catch {
            state = .unavailable(readable(error))
        }
    }

    /// Stop starting it at login.
    ///
    /// This stops only the daemon. Terminals keep running — they belong to
    /// tmux, not to us — and no worktree or database is touched.
    func unregister() {
        Task {
            do {
                try await service.unregister()
            } catch {
                state = .unavailable(readable(error))
                return
            }
            refresh()
        }
    }

    private func openLoginItemsSettings() {
        SMAppService.openSystemSettingsLoginItems()
    }

    /// Turn the two failures a user can actually act on into instructions, and
    /// everything else into the system's own words rather than a guess.
    private func readable(_ error: Error) -> String {
        let ns = error as NSError
        switch ns.code {
        case 1:  // kSMErrorInternalFailure / not signed acceptably
            return
                "macOS refused the registration. A locally built, ad-hoc-signed app "
                + "usually needs to be enabled by hand in System Settings › General › Login Items."
        case 2:
            return "The daemon is already registered by another copy of Far Cooler."
        default:
            return ns.localizedDescription
        }
    }
}

/// Drive registration from the command line, for checking it without a window.
///
///     FARCOOLER_SERVICE_PROBE=status './Far Cooler.app/Contents/MacOS/Far Cooler'
///
/// Login-item registration is one of those things that behaves differently for
/// a signed app, an ad-hoc-signed one, and a bare executable. A probe makes
/// which case you are in a fact rather than a guess.
@MainActor
enum ServiceProbe {
    static func run(_ action: String) -> Never {
        let service = ServiceRegistration()
        switch action {
        case "register": service.register()
        case "unregister":
            service.unregister()
            // unregister is async; give it a moment before reporting.
            RunLoop.main.run(until: Date().addingTimeInterval(1.5))
        default: break
        }
        service.refresh()
        print("bundle: \(Bundle.main.bundleURL.path)")
        print("state:  \(service.state)")
        exit(0)
    }
}
