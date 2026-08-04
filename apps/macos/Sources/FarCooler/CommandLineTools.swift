import Foundation

/// Symlinking the app's own bundled `farcooler` and `farcoolerd` into
/// `~/.local/bin`, so a shell — including one an SSH session execs on this
/// Mac — can find them.
///
/// Without this, "run the app once" (see docs/remote-hosts.md) is true for
/// the login-item daemon and false for everything else: an SSH-invoked shell
/// has no idea either binary exists, `farcoolerd --stdio` finds nothing, and
/// the client reports the closed pipe as "did not answer" — indistinguishable,
/// from the wire, from a hung daemon.
@MainActor
final class CommandLineTools: ObservableObject {
    static let binaryNames = ["farcooler", "farcoolerd"]

    enum State: Equatable {
        case notInstalled
        case installed
        /// Something at this path is not a symlink to this app's own binary —
        /// a real file, or a symlink pointing somewhere else. Named so the
        /// user can go look, rather than something this app will overwrite.
        case conflict(String)
        case unavailable(String)
    }

    @Published private(set) var state: State = .notInstalled

    private enum SlotState: Equatable {
        case missing
        case ours
        case conflict(String)
    }

    private var localBinDirectory: URL {
        FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".local/bin")
    }

    private func localBinURL(for name: String) -> URL {
        localBinDirectory.appendingPathComponent(name)
    }

    private func bundledBinaryURL(for name: String) -> URL {
        Bundle.main.bundleURL
            .appendingPathComponent("Contents/Resources")
            .appendingPathComponent(name)
    }

    init() {
        refresh()
    }

    /// What's actually on disk, in the user's terms.
    func refresh() {
        // Mirrors ServiceRegistration's identical guard: a bare executable has
        // no Contents/Resources to point a symlink at.
        guard Bundle.main.bundleURL.pathExtension == "app" else {
            state = .unavailable("Run Far Cooler from the app bundle to enable this.")
            return
        }

        let slots = Self.binaryNames.map(slotState(for:))
        let conflicts = slots.compactMap { slot -> String? in
            if case .conflict(let path) = slot { return path }
            return nil
        }

        if let path = conflicts.first {
            state = .conflict("\(path) already exists and isn't managed by Far Cooler.")
        } else if slots.allSatisfy({ $0 == .ours }) {
            state = .installed
        } else {
            state = .notInstalled
        }
    }

    /// Symlink both binaries in. Refuses outright if either slot is a
    /// conflict — this never overwrites a path it did not create.
    func install() {
        if case .conflict = state { return }

        do {
            try FileManager.default.createDirectory(
                at: localBinDirectory, withIntermediateDirectories: true)
            for name in Self.binaryNames {
                let link = localBinURL(for: name).path
                // A symlink already there (e.g. a previous install) is
                // replaced; nothing else reaches this point, since a real
                // file would have read as `.conflict` above and returned already.
                if (try? FileManager.default.destinationOfSymbolicLink(atPath: link)) != nil {
                    try FileManager.default.removeItem(atPath: link)
                }
                try FileManager.default.createSymbolicLink(
                    atPath: link, withDestinationPath: bundledBinaryURL(for: name).path)
            }
        } catch {
            state = .unavailable((error as NSError).localizedDescription)
            return
        }
        refresh()
    }

    /// Remove only the symlinks this app owns. A conflicting path is left
    /// exactly as `install()` would have left it: untouched.
    func uninstall() {
        for name in Self.binaryNames where slotState(for: name) == .ours {
            try? FileManager.default.removeItem(atPath: localBinURL(for: name).path)
        }
        refresh()
    }

    private func slotState(for name: String) -> SlotState {
        let path = localBinURL(for: name).path
        // Read as a symlink first: this also catches a *dangling* symlink
        // (target currently missing), which `fileExists` alone would follow
        // through and misreport as "missing" rather than "ours but broken".
        if let destination = try? FileManager.default.destinationOfSymbolicLink(atPath: path) {
            return destination == bundledBinaryURL(for: name).path ? .ours : .conflict(path)
        }
        return FileManager.default.fileExists(atPath: path) ? .conflict(path) : .missing
    }
}

/// Drive install/uninstall from the command line, for checking it without a
/// window.
///
///     FARCOOLER_CLI_TOOLS_PROBE=install './Far Cooler.app/Contents/MacOS/Far Cooler'
@MainActor
enum CLIToolsProbe {
    static func run(_ action: String) -> Never {
        let tools = CommandLineTools()
        switch action {
        case "install": tools.install()
        case "uninstall": tools.uninstall()
        default: break
        }
        tools.refresh()
        print("bundle: \(Bundle.main.bundleURL.path)")
        print("state:  \(tools.state)")
        exit(0)
    }
}
