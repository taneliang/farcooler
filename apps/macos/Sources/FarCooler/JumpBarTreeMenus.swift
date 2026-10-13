import AgentKit
import Foundation

// The menus of the jump bar's crumbs when it walks the sidebar's outline
// (ov-512): each crumb's menu lists that level's siblings in the sidebar, the
// one you're at checked. The workspace's, first, is still every workspace.

extension JumpMenus {
    /// The menu for `node`'s crumb: the rows beside it in the outline, in the
    /// sidebar's order. A row that goes nowhere (a subagent, which runs inside
    /// the orchestrator; a group that is no section of the Plan) isn't listed.
    /// `tab` adds the card's tabs first, as its children, on a card's own crumb.
    static func siblings(
        of node: OneTreeNode, in outline: SidebarOutline, resolve: (OneTreeTarget) -> Selection?,
        tab: (task: String, tab: TaskTab)? = nil
    ) -> JumpMenu {
        var sections: [JumpSection] = []
        if let tab, case .task(tab.task)? = node.target {
            let open = resolve(.task(tab.task))
            sections.append(
                JumpSection(
                    title: node.key,
                    items: open.map { open in
                        TaskTab.allCases.map { each in
                            JumpItem(
                                id: "tab|\(each.rawValue)", title: each.title, current: each == tab.tab,
                                target: .tab(open, each))
                        }
                    } ?? []))
        }
        let rows = outline.siblings(of: node.id).filter { $0.kind != .place && $0.kind != .doneFold }
        sections.append(
            JumpSection(
                title: "",
                items: rows.compactMap { row -> JumpItem? in
                    guard let target = SidebarOutline.destination(of: row), let place = resolve(target) else { return nil }
                    return JumpItem(
                        id: "tree|\(row.id)", title: row.key.isEmpty ? row.title : "\(row.key) \(row.title)",
                        subtitle: row.detail.isEmpty ? nil : row.detail, key: row.key.isEmpty ? nil : row.key,
                        glyph: row.glyph, current: row.id == node.id, target: .go(place))
                }))
        return JumpMenu(sections)
    }
}

extension ContentView {
    /// The menus for `path`'s crumbs, built only when the bar asks: the
    /// workspaces', then each row's siblings.
    func treeMenus(_ path: TreePath, place: Selection, host: String, summary: WorkspaceSummary?) -> JumpMenuSource {
        guard let summary else { return .none }
        let workspaces = jumpMenus(Array(path.crumbs.prefix(1)), place: place, host: host, summary: summary)
        let tab: (task: String, tab: TaskTab)? = {
            guard case .workspace(_, _, .task(let id)?) = place else { return nil }
            return (id, taskTab(for: place))
        }()
        return JumpMenuSource(count: path.crumbs.count) {
            let first = workspaces.build()
            return first + path.nodes.enumerated().map { index, node in
                JumpMenus.siblings(
                    of: node, in: path.outline, resolve: { treeSelection($0, host: host, workspace: summary.id) },
                    tab: index == path.nodes.count - 1 ? tab : nil)
            }
        }
    }
}
