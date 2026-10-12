import AgentKit
import Foundation

/// A path to the CLI that the app owns and that outlives the app's own.
///
/// A tunneled runner's `ProxyCommand` names a program, and that line sits in a
/// file nobody is going to rewrite when the app moves. The bundle's path is
/// wrong in three ordinary cases: the app was run from a quarantined download
/// (macOS runs it from a randomized `AppTranslocation` path that is gone after
/// the next launch), it was renamed (`Far Cooler 2.app`), or it was moved. So
/// the block names this link instead, and the link is repointed at the bundle's
/// CLI every time the app has a reason to look (a join, and launch).
///
/// Under `~/.config/farcooler/<channel>/`, beside the device keys: the one place
/// this app already owns on disk. Space-free, so the word `ssh` hands the shell
/// needs no quoting of its own; `%d` is `ssh`'s own spelling of the home
/// directory.
enum StableCLI {
    static func url(home: URL = URL(fileURLWithPath: NSHomeDirectory())) -> URL {
        home.appendingPathComponent(".config/farcooler")
            .appendingPathComponent(AppVersion.channel)
            .appendingPathComponent("bin/farcooler")
    }

    /// The `ProxyCommand` word for ``url(home:)``.
    static var word: String {
        "'%d/.config/farcooler/\(AppVersion.channel)/bin/farcooler'"
    }

    /// Point the link at `bundled`, and say whether it now does.
    ///
    /// Refuses a CLI that is nowhere (a build with none) and one inside an App
    /// Translocation directory, whose path is not there tomorrow. A block that
    /// names a link to either would be a tunnel that works until the next
    /// launch.
    @discardableResult
    static func refresh(bundled: String? = CLI.binary, at link: URL = url()) -> Bool {
        guard let bundled, !bundled.contains("/AppTranslocation/") else { return false }
        let fileManager = FileManager.default
        let directory = link.deletingLastPathComponent()
        do {
            try fileManager.createDirectory(
                at: directory, withIntermediateDirectories: true,
                attributes: [.posixPermissions: 0o700])
            if (try? fileManager.destinationOfSymbolicLink(atPath: link.path)) == bundled {
                return true
            }
            try? fileManager.removeItem(at: link)
            try fileManager.createSymbolicLink(atPath: link.path, withDestinationPath: bundled)
            return true
        } catch {
            return false
        }
    }

    /// At launch: keep a link that already exists pointing at this app. A Mac
    /// with no tunneled runners never gets one made.
    static func refreshIfPresent() {
        let link = url()
        guard (try? FileManager.default.destinationOfSymbolicLink(atPath: link.path)) != nil
        else { return }
        refresh(at: link)
    }
}
