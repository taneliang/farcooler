import Foundation

extension DaemonClient {
    /// Ask the runner to try again to download the large files a worktree
    /// still holds as pointers: `farcooler worktree hydrate-lfs` (ov-199).
    /// Refreshes the fleet so the fresh count reaches the notice, and answers
    /// the reason it couldn't ask, or nil.
    ///
    /// The command is the runner's `worktree.hydrate_lfs`, which rewrites only
    /// untouched pointers through a throwaway index, so it is safe on a
    /// worktree an agent is working in.
    func hydrateLfs(_ worktree: String) async -> String? {
        let (_, message) = await runRaw(["worktree", "hydrate-lfs", worktree, "--json"], background: true)
        await refresh()
        return message
    }
}
