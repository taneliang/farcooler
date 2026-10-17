import AgentKit
import SwiftUI

/// The Welcome page's work (ov-519): what this Mac's runner has said, and
/// adding a repository, then going to it.
extension ContentView {
    /// The Welcome page's "This Mac" step, from what the local runner said.
    /// Checking until it has said anything.
    func macStep() -> (MacStep, HarnessAvailability) {
        #if DEBUG
            // A capture of the failing pages, which need a Mac without tmux or
            // an agent: the real page and window, with the step it would have
            // drawn (`FARCOOLER_CAPTURE_MAC_STEP`, debug builds only).
            switch ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_MAC_STEP"] {
            case "noTmux": return (.noTmux, HarnessAvailability(agentsFound: ["claude", "codex"]))
            case "noAgent": return (.noAgent, HarnessAvailability(agentsFound: []))
            default: break
            }
        #endif
        guard let local = store.clients[""], local.daemonBuild != nil || local.daemonBuildUnreadable else {
            return (.checking, HarnessAvailability(agentsFound: nil))
        }
        let agents = local.availability
        // `runtime_healthy` is false when the runner can't inventory its tmux.
        let state = LocalRunnerState.running(tmuxFound: local.fleet.runtimeHealthy, agents: agents)
        return (WelcomeSteps(local: state, hasRepositories: false, orchestratorSeated: false).mac, agents)
    }

    /// Add the repository at `path` on this Mac as the Add Repository sheet
    /// does, then open its Main. Nil on success, else the words it failed with.
    func registerOnThisMac(_ path: String) async -> String? {
        let host = ""
        if let why = store.refusal(for: host) { return why }
        guard let client = store.clients[host] else { return "That runner isn’t connected." }
        let covered = store.roots.contains { entry in
            guard entry.host == host, let root = entry.root.path else { return false }
            return path == root || path.hasPrefix(root + "/")
        }
        if !covered {
            // The folder around it, so the next repository there needs no
            // permission; the repository's own folder when the first is refused
            // (a repository in the home folder).
            let parent = (path as NSString).deletingLastPathComponent
            if await client.addRoot(parent) != nil, let words = await client.addRoot(path) { return words }
            await client.refreshRoots()
        }
        if let words = await client.registerRepository(path) { return words }
        await repositoryRegistered(on: host)
        return nil
    }

    /// A repository was registered on `host`: read the runner again, and open
    /// the new repository's Main, where the orchestrator's invitation says
    /// what to do next, but only when the window had nowhere to be (the
    /// Welcome page): somebody already in a workspace keeps their place.
    func repositoryRegistered(on host: String) async {
        let known = Set(store.repositories.map(\.repository.id))
        // Registering adopts every worktree the repository already has, main
        // checkout included, so there is nothing left to offer to import.
        // Refreshed on the runner it was registered on: no event pushes a
        // repository or root change, so any other host would leave that
        // host's switcher and navigator stale.
        await store.clients[host]?.refreshRepositories()
        await store.clients[host]?.refresh()
        if let next = Self.selectionAfterRegistering(
            known: known, in: store.repositories, fleet: store.fleet, current: selection)
        {
            selection = next
            // Then how agents should work there (ov-519 phase 2b).
            openCharterSetupAfterRegistering(next)
        }
    }

    /// Where the window goes once a registration has been read: the new
    /// repository's Main, or nowhere when that can't be said, or when `current`
    /// is a place somebody chose.
    nonisolated static func selectionAfterRegistering(
        known: Set<String>, in repositories: [(host: String, repository: Repository)], fleet: Fleet,
        current: Selection?
    ) -> Selection? {
        guard current == nil else { return nil }
        return FleetPlaceholder.mainOfNewRepository(known: known, in: repositories, fleet: fleet).map {
            .workspace(host: $0.host, workspace: $0.workspace.id, focus: nil)
        }
    }
}
