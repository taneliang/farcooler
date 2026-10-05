import AgentKit
import SwiftUI

// An orchestrator's page on the Mac (ov-269 design 6.1-6.4, ov-284): its row
// in the Plan view's Pages section, the page itself in the main area, and the
// sections a theme's page draws for the pages anchored to it. The blocks are
// AgentKit's `PageView`, shared with iOS; this file is only where they live
// and what a reference opens.

/// One page's row in the Pages section: its title, its one-line summary and
/// when it was updated. Hide Page in its context menu.
struct PlanPageRow: View {
    let page: BoardPage
    let now: Int64
    let selected: Bool
    let keyed: Bool
    let onHide: () -> Void
    let action: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            HStack(alignment: .firstTextBaseline, spacing: 0) {
                Image(systemName: PlanPage.page(page.slot).symbol)
                    .font(.system(size: WorkspaceStyle.PaneText.secondary))
                    .foregroundStyle(.secondary)
                    .accessibilityHidden(true)
                    .glyphColumn()
                VStack(alignment: .leading, spacing: NavigatorRhythm.lineGap) {
                    Text(page.title)
                        .font(.system(size: WorkspaceStyle.PaneText.body, weight: .medium))
                        .lineLimit(1)
                    if !page.summary.isEmpty {
                        Text(page.summary)
                            .font(.system(size: WorkspaceStyle.PaneText.secondary))
                            .foregroundStyle(.secondary)
                            .lineLimit(2)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    Text(PageWords.updated(page, now: now))
                        .font(.system(size: WorkspaceStyle.PaneText.secondary))
                        .foregroundStyle(.tertiary)
                        .probed("plan-page-\(page.slot)-updated")
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .navigatorRow(selected: selected, keyed: keyed, leading: 0)
            .background {
                if hovering && !selected { RoundedRectangle.control.fill(Fill.hover).boxOutset() }
            }
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .contextMenu {
            Button("Hide Page", action: onHide)
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(Self.spoken(page, now: now))
        .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
        .accessibilityAction(named: "Hide Page", onHide)
        .identified("plan-page-\(page.slot)")
    }

    /// "Page, Train integ-10, In review · 3 of 4 lanes green, Updated 12 min
    /// ago".
    static func spoken(_ page: BoardPage, now: Int64) -> String {
        (["Page", page.title] + (page.summary.isEmpty ? [] : [page.summary]) + [PageWords.updated(page, now: now)])
            .joined(separator: ", ")
    }
}

extension PlanPageContext {
    /// What the page's references are drawn from: the board's cards, the
    /// plan when the runner keeps one, the board's pages, and its worktrees
    /// and their terminals by name. Nothing new is asked of the runner.
    @MainActor func world(_ plan: PlanStore, now: Int64 = PlanOverviewView.nowMs()) -> PageWorld {
        var names: [String: String] = [:]
        var terminals: Set<String> = []
        for worktree in worktrees {
            for name in [worktree.task, worktree.branch, (worktree.path as NSString).lastPathComponent] where !name.isEmpty {
                if names[name] == nil { names[name] = worktree.id }
            }
            for terminal in worktree.terminals {
                terminals.insert(PageWorld.terminalKey(worktree: worktree.id, name: terminal.title))
            }
        }
        return PageWorld(
            tasks: Array(rows.values), plan: plan.available && plan.hasRead ? plan.plan : nil, pages: plan.pages,
            worktrees: names, terminals: terminals, nowMs: now)
    }
}

/// A page in the main area, as a theme's or lane's page opens: a document on
/// the paper.
struct PlanOrchestratorPage: View {
    let page: BoardPage
    let world: PageWorld
    let context: PlanPageContext

    var body: some View {
        PlanDocument(id: "plan-orchestrator-page") {
            PageView(page: page, world: world, onOpen: context.onDestination)
        }
    }
}

/// The pages anchored to a theme, as sections of its page: each headed by
/// its title, with "From the orchestrator · 2 h ago" opening it whole.
struct PlanAnchoredPages: View {
    let pages: [BoardPage]
    let world: PageWorld
    let context: PlanPageContext
    var onHide: (String) -> Void = { _ in }

    var body: some View {
        ForEach(pages) { page in
            PlanSection(title: page.title) {
                Button {
                    context.onOpen(.page(page.slot))
                } label: {
                    HStack(spacing: Spacing.tight) {
                        Text("\(PageWords.fromTheOrchestrator) · \(PlanWords.ago(page.updatedAtMs, now: world.nowMs))")
                        Image(systemName: "chevron.forward").imageScale(.small)
                    }
                }
                .buttonStyle(.plain)
                .font(.system(size: WorkspaceStyle.PaneText.secondary))
                .foregroundStyle(.secondary)
                .help("Open this page")
                .accessibilityLabel("Open \(page.title), \(PageWords.updated(page, now: world.nowMs))")
                .identified("plan-anchored-open-\(page.slot)")
            } content: {
                if let doc = page.doc {
                    PageBlocksView(doc: doc, world: world, onOpen: context.onDestination)
                } else {
                    Text(PageWords.unreadable).foregroundStyle(.secondary)
                }
            }
            .contextMenu { Button("Hide Page") { onHide(page.slot) } }
            .identified("plan-anchored-\(page.slot)")
        }
    }
}
