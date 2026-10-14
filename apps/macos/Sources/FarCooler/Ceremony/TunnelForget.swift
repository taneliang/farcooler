import Foundation

/// Taking a tunneled runner back out: its entry in Far Cooler's block in
/// `~/.ssh/config`, and its token and host key in `runners.json`.
///
/// Without this, `ssh cosmo` from Terminal still dials a runner somebody
/// removed, and the token — the one field worth stealing in `runners.json` —
/// outlives the runner it opened.
struct TunnelForget {
    var keeper: TunnelRunnerKeeper = CLINodeKeyTool()
    var alternatives: RouteAlternatives = DefaultsRouteAlternatives()
    var readEntries: @Sendable () -> [SshConfig.Entry] = { SshConfig.entries() }
    var writeConfig: @Sendable ([String], URL) throws -> Void = {
        try SshConfig.write($0, identity: $1)
    }

    /// True when `target` was a tunneled runner and its token is gone.
    ///
    /// The store is cleaned even when the config could not be written: an entry
    /// pointing at nothing is a failed `ssh`, and a token with no entry is a
    /// secret nothing uses.
    @discardableResult
    func forget(target: String) async -> Bool {
        let before = readEntries()
        guard let gone = before.first(where: { $0.alias == target && $0.tunneled }) else {
            return false
        }
        try? writeConfig(SshConfig.render(before.filter { $0.key != gone.key }), ShellKey.directory)
        alternatives.forget(gone.key)
        return await keeper.forget([gone.key])
    }
}
