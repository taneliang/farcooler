import AgentKit
import Foundation

// Orchestrator pages on the Mac (ov-269 design 6.1, ov-284): read with the
// plan, listed in the Plan view's Pages section, opened in the main area like
// a theme's or lane's page, and drawn inside a theme's page when anchored to
// it. Behind `board_pages`, and only ever in the Plan view, so with Plan off
// the board draws exactly what it drew before.
//
// Removing pages means deleting this file, `PlanOrchestratorPages.swift`, the
// `page` case of `PlanPage` and the call sites that name `BoardPage`.

extension PlanStore {
    /// Whether this runner keeps pages.
    var pagesAvailable: Bool { client.daemonBuild?.can(.boardPages) == true }

    /// Read every page with its document: one call, at most 12 × 32 KiB.
    /// A runner without pages has none, and a read that fails keeps what
    /// was drawn, as the plan's does.
    func readPages() async {
        guard pagesAvailable else {
            if !pages.isEmpty { pages = [] }
            pagesRead = true
            return
        }
        let (data, _) = await client.pageList(repository: repositoryID, workspace: workspace.boardWorkspace)
        guard let data, let list = try? BoardPageList.decode(data) else { return }
        if list.pages != pages { pages = list.pages }
        pagesRead = true
    }

    /// A page by slot.
    func page(_ slot: String) -> BoardPage? { pages.first { $0.slot == slot } }

    /// The Pages section's rows: pages of their own, and anchored pages whose
    /// theme is gone, minus those hidden here.
    var listedPages: [BoardPage] { Self.listed(pages, plan: plan, hidden: hiddenPages) }

    /// The pages drawn inside `theme`'s page, minus those hidden here.
    func anchoredPages(to theme: String) -> [BoardPage] { Self.anchored(pages, to: theme, plan: plan, hidden: hiddenPages) }

    /// How many of this board's pages are hidden here and would otherwise
    /// be listed or anchored.
    var hiddenCount: Int { pages.filter { hiddenPages.contains($0.slot) }.count }

    /// Hide a page on this Mac.
    func hide(_ slot: String) { hiddenPages.insert(slot) }

    /// Show every hidden page again.
    func showHiddenPages() { hiddenPages = [] }

    /// The themes a page can be drawn inside: those the plan still has and
    /// hasn't dropped.
    static func liveThemes(_ plan: PlanModel) -> Set<String> {
        Set(plan.themes.filter { $0.state != "dropped" }.map(\.id))
    }

    static func listed(_ pages: [BoardPage], plan: PlanModel, hidden: Set<String>) -> [BoardPage] {
        let themes = liveThemes(plan)
        return pages.filter { page in
            guard !hidden.contains(page.slot) else { return false }
            guard let anchor = page.themeAnchor else { return true }
            return !themes.contains(anchor)
        }
    }

    static func anchored(_ pages: [BoardPage], to theme: String, plan: PlanModel, hidden: Set<String>) -> [BoardPage] {
        guard liveThemes(plan).contains(theme) else { return [] }
        return pages.filter { $0.themeAnchor == theme && !hidden.contains($0.slot) }
    }

    static func hiddenKey(host: String, workspace: String) -> String { "board.pages.hidden.\(host).\(workspace)" }
}

extension DaemonClient {
    /// One board's pages with their documents: `farcooler page list --json`,
    /// the shape AgentKit's `BoardPageList` decodes. In the background, as
    /// the plan's read is.
    func pageList(repository: String, workspace: String?) async -> (data: Data?, message: String?) {
        await runRaw(
            ["page", "list", "--repo", repository] + (workspace.map { ["--workspace", $0] } ?? []) + ["--json"],
            background: true)
    }
}
