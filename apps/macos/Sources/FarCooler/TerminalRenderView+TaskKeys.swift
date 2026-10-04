import AgentKit
import AppKit
import CFarCoolerVT

extension TerminalRenderView {
    /// The link under a cell, URL or task key, with where it sits.
    func link(atRow row: Int, column: Int) -> (url: String, span: FarCoolerVtUrlSpan)? {
        core.link(atRow: row, column: column, columns: grid.columns, linker: taskKeyLinker)
    }

    /// Open the link under a cell, if there is one. A task key opens its task
    /// in the app, through the linker, and is never handed to `opener`: the
    /// system would give `farcooler://` to whichever channel's app claimed it
    /// last, a canary's link opening the stable app (`Markdown.openGuard`).
    /// Anything else is a URL the core allowed, and goes to `opener`.
    @discardableResult
    func openLink(
        atRow row: Int, column: Int, opener: (URL) -> Void = { NSWorkspace.shared.open($0) }
    ) -> Bool {
        guard let found = link(atRow: row, column: column), let url = URL(string: found.url) else { return false }
        if taskKeyLinker.follow(url) { return true }
        opener(url)
        return true
    }
}
