import AgentKit
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
    /// A binary as it exists in two places at once.
    ///
    /// The two names are different on purpose, and the asymmetry is the whole
    /// point. Inside the bundle it is whatever cargo produced — `build-app.sh`
    /// copies `target/release/farcooler` straight in, on every channel, so
    /// there is exactly one name to look for there. In `~/.local/bin` it has
    /// to be this build's channel name, because that directory is shared: a
    /// Mac with the release app and a canary one installed has both, and a
    /// canary bundle claiming `farcooler` would take the release build's place
    /// on the PATH of every SSH session into this machine — including the one
    /// a release client opens, which would then be talking to the canary
    /// daemon without either side having any way to notice.
    struct Tool {
        /// What it is called inside `Contents/Resources`.
        let bundled: String
        /// What this build calls it in `~/.local/bin`.
        let link: String
    }

    static let tools: [Tool] = ["farcooler", "farcoolerd"].map {
        Tool(bundled: $0, link: channelName(for: $0))
    }

    /// `farcooler` → `farcooler-canary`, and `farcooler` on stable.
    ///
    /// The same scheme as `Channel::cli_binary_name` in `crates/protocol`, and
    /// it has to stay the same scheme: what this links is what the CLI, the
    /// daemon and every client look for by name.
    ///
    /// `nonisolated` because it is a string and a stamp in `Info.plist` and
    /// touches nothing this class owns — `CLI.binary` needs the same answer
    /// and has no reason to hop to the main actor to get it.
    nonisolated static func channelName(for binary: String) -> String {
        AppVersion.isRelease ? binary : "\(binary)-\(AppVersion.channel)"
    }

    /// Names a non-stable build used to claim before it knew about channels,
    /// and now has to hand back.
    ///
    /// Only ones pointing INTO THIS BUNDLE are touched — this app is giving up
    /// a name it took, not evicting whatever release install may have since
    /// taken it properly.
    static var strandedLinks: [Tool] {
        AppVersion.isRelease ? [] : tools.map { Tool(bundled: $0.bundled, link: $0.bundled) }
    }

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

    private func localBinURL(for tool: Tool) -> URL {
        localBinDirectory.appendingPathComponent(tool.link)
    }

    private func bundledBinaryURL(for tool: Tool) -> URL {
        Bundle.main.bundleURL
            .appendingPathComponent("Contents/Resources")
            .appendingPathComponent(tool.bundled)
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

        let slots = Self.tools.map(slotState(for:))
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
            for tool in Self.tools {
                let link = localBinURL(for: tool).path
                // A symlink already there (e.g. a previous install) is
                // replaced; nothing else reaches this point, since a real
                // file would have read as `.conflict` above and returned already.
                if (try? FileManager.default.destinationOfSymbolicLink(atPath: link)) != nil {
                    try FileManager.default.removeItem(atPath: link)
                }
                try FileManager.default.createSymbolicLink(
                    atPath: link, withDestinationPath: bundledBinaryURL(for: tool).path)
            }
        } catch {
            state = .unavailable((error as NSError).localizedDescription)
            return
        }
        releaseStrandedLinks()
        refresh()
    }

    /// Remove only the symlinks this app owns. A conflicting path is left
    /// exactly as `install()` would have left it: untouched.
    func uninstall() {
        for tool in Self.tools where slotState(for: tool) == .ours {
            try? FileManager.default.removeItem(atPath: localBinURL(for: tool).path)
        }
        releaseStrandedLinks()
        refresh()
    }

    /// Drop any bare-named link this build made before it knew its channel.
    ///
    /// A canary app that had already linked `~/.local/bin/farcooler` at itself
    /// keeps holding that name after this build starts linking
    /// `farcooler-canary` — and holding it is the whole problem, since the
    /// release app can then never claim it and every SSH session into this Mac
    /// keeps reaching the canary build. Removed only where the link points
    /// into THIS bundle, which is the same rule `uninstall` follows: this app
    /// takes back what it put there and touches nothing else.
    private func releaseStrandedLinks() {
        for tool in Self.strandedLinks where slotState(for: tool) == .ours {
            try? FileManager.default.removeItem(atPath: localBinURL(for: tool).path)
        }
    }

    private func slotState(for tool: Tool) -> SlotState {
        let path = localBinURL(for: tool).path
        // Read as a symlink first: this also catches a *dangling* symlink
        // (target currently missing), which `fileExists` alone would follow
        // through and misreport as "missing" rather than "ours but broken".
        if let destination = try? FileManager.default.destinationOfSymbolicLink(atPath: path) {
            return destination == bundledBinaryURL(for: tool).path ? .ours : .conflict(path)
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
