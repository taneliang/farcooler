import Foundation

/// The extra read-only folders a runner shares with Files (ov-232), named in
/// its config and never addable from here.
enum ReadOnlyFolders {
    /// One runner's folders, by name.
    struct Group: Equatable, Hashable {
        var host: String
        var names: [String]
    }

    /// The names in `status --json`'s `readOnlyFolders`, in the order the
    /// runner lists them. None unless the runner has the `read_only_folders`
    /// capability (`offered`): a runner that doesn't has none, whatever else
    /// it said.
    static func names(in status: [String: Any], offered: Bool) -> [String] {
        guard offered, let list = status["readOnlyFolders"] as? [[String: Any]] else { return [] }
        return list.compactMap { ($0["name"] as? String).flatMap { $0.isEmpty ? nil : $0 } }
    }

    /// The runners that share any, in the order given.
    static func groups(_ folders: [(host: String, names: [String])]) -> [Group] {
        folders.filter { !$0.names.isEmpty }.map { Group(host: $0.host, names: $0.names) }
    }

    /// What Files calls a folder: its name, and its runner when that
    /// disambiguates.
    static func title(_ name: String, host: String) -> String {
        host.isEmpty ? name : "\(name) · \(runnerName(host))"
    }

    /// A runner by name rather than by its ssh target: "This Mac", or the
    /// host without its `user@`.
    static func runnerName(_ host: String) -> String {
        host.isEmpty ? "This Mac" : String(host.split(separator: "@", omittingEmptySubsequences: false).last ?? "")
    }
}
