import AgentKit
import AppKit
import Foundation

/// Where this Mac's runner keeps its log, and the Help item that shows it
/// (ov-798).
///
/// The runner writes its own log (`crates/daemon/src/daemon_log.rs`), because
/// both ways it is started, this app's CLI and launchd, give it nowhere to
/// write. This only knows the folder, by the same rule: `FARCOOLER_HOME`'s
/// `logs` when that names a home, else `~/Library/Logs/` in a folder named
/// for the channel's bundle id, which is the name the daemon's data folder
/// already has.
enum DaemonLog {
    /// The log folder.
    static func folder(
        environment: [String: String] = ProcessInfo.processInfo.environment,
        channel: String = AppVersion.channel,
        home: String = NSHomeDirectory()
    ) -> URL {
        if let over = environment["FARCOOLER_HOME"] {
            return URL(fileURLWithPath: over).appendingPathComponent("logs")
        }
        let name =
            switch channel {
            case "stable": "FarCooler"
            case "preview": "FarCoolerPreview"
            case "canary": "FarCoolerCanary"
            default: "FarCoolerLocal"
            }
        return URL(fileURLWithPath: home)
            .appendingPathComponent("Library/Logs/com.farcooler.\(name)")
    }

    /// The current file in that folder; older ones are beside it, numbered.
    static func file(folder: URL = folder()) -> URL {
        folder.appendingPathComponent("farcoolerd.log")
    }

    /// Whether there is anything to show: the menu item is dimmed until the
    /// runner has written its first line.
    static var exists: Bool {
        FileManager.default.fileExists(atPath: folder().path)
    }

    /// Select the log in Finder, or open its folder when the current file is
    /// not there.
    @MainActor
    static func reveal() {
        let dir = folder()
        let current = file(folder: dir)
        if FileManager.default.fileExists(atPath: current.path) {
            NSWorkspace.shared.activateFileViewerSelecting([current])
        } else {
            NSWorkspace.shared.open(dir)
        }
    }
}
