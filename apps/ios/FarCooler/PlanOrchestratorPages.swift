import SwiftUI

// Orchestrator pages on a phone (ov-269 design 6.1, 6.3 and 6.5; ov-285): the
// Pages section in the Plan view, a page pushed, and the sections a theme's
// page draws for the pages anchored to it. The blocks are AgentKit's
// `PageView`, the one renderer the Mac draws with; this file is only where
// they live on a phone and what a reference opens.
//
// Navigation is the only action a page has. A reference pushes what it names;
// a question still waiting goes back to Needs You, where it's answered; an
// `https` link opens in the browser after `PageView` checks it again, with its
// domain drawn beside it.

/// The Pages section of the Plan view: one row per page that isn't drawn
/// inside a theme, each opening the page, pushed.
struct PlanPagesSection: View {
    let state: PageListState?
    let plan: PlanModel
    let read: () async -> Void
    let onOpen: (PhonePlanPage) -> Void

    var body: some View {
        switch state {
        case .none:
            EmptyView()
        case .unavailable:
            Section {
                PlanNotice(title: PageWords.couldntRead, detail: nil)
                    .accessibilityIdentifier("plan-pages-unavailable")
                Button(PlanWords.tryAgain) { Task { await read() } }
                    .accessibilityIdentifier("plan-pages-retry")
            } header: {
                // No count: the read failed, so how many there are isn't known.
                PlanHeader(title: "Pages", count: nil).accessibilityIdentifier("plan-pages")
            }
        case .loaded(let pages):
            let listed = PageShelf.listed(pages, plan: plan)
            if !listed.isEmpty {
                Section {
                    ForEach(listed) { page in
                        PlanPageRow(page: page, now: Self.now()) { onOpen(.page(page.slot)) }
                    }
                } header: {
                    PlanHeader(title: "Pages", count: listed.count).accessibilityIdentifier("plan-pages")
                }
            }
        }
    }

    static func now() -> Int64 { Int64(Date().timeIntervalSince1970 * 1000) }
}

/// One page's row: its title, its one-line summary and when it was updated.
struct PlanPageRow: View {
    let page: BoardPage
    let now: Int64
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(alignment: .firstTextBaseline, spacing: PaneMetrics.card) {
                Image(systemName: "doc.richtext")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .frame(width: 22, alignment: .leading)
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 2) {
                    Text(page.title)
                        .font(.subheadline.weight(.medium))
                        .lineLimit(2)
                    if !page.summary.isEmpty {
                        Text(page.summary)
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                            .lineLimit(2)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    Text(PageWords.updated(page, now: now))
                        .font(.footnote)
                        .foregroundStyle(.tertiary)
                }
                Spacer(minLength: 0)
                Image(systemName: "chevron.forward")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.tertiary)
                    .accessibilityHidden(true)
            }
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Self.spoken(page, now: now))
        .accessibilityAddTraits(.isButton)
        .accessibilityIdentifier("plan-page-\(page.slot)")
    }

    /// "Page, Train integ-10, In review · 3 of 4 lanes green, Updated 12 min
    /// ago", as the Mac's row says it.
    static func spoken(_ page: BoardPage, now: Int64) -> String {
        (["Page", page.title] + (page.summary.isEmpty ? [] : [page.summary]) + [PageWords.updated(page, now: now)])
            .joined(separator: ", ")
    }
}

/// A page pushed: its title, when it was written and by whom, then its
/// blocks, in one scrolling column. Tables stack and steps go down the page,
/// as a phone in portrait is narrower than `PageLayout.narrow`.
struct PlanOrchestratorPage: View {
    let page: BoardPage
    let world: PageWorld
    let onDestination: (PageDestination) -> Void

    var body: some View {
        ScrollView {
            PageView(page: page, world: world, onOpen: onDestination)
                .padding(.horizontal, PaneMetrics.edge)
                .padding(.vertical, PaneMetrics.card)
        }
        .background(Color(.systemGroupedBackground))
        .accessibilityIdentifier("plan-orchestrator-page")
    }
}

/// The pages anchored to a theme, as sections of its page: each headed by
/// its title, with "From the orchestrator · 2 h ago" opening it whole.
struct PlanAnchoredPages: View {
    let pages: [BoardPage]
    let world: PageWorld
    let onOpen: (PhonePlanPage) -> Void
    let onDestination: (PageDestination) -> Void

    var body: some View {
        ForEach(pages) { page in
            Section {
                if let doc = page.doc {
                    PageBlocksView(doc: doc.under(page.title), world: world, onOpen: onDestination)
                        .padding(.vertical, PaneMetrics.tight)
                        .accessibilityElement(children: .contain)
                        .accessibilityIdentifier("plan-anchored-\(page.slot)")
                } else {
                    Text(PageWords.unreadable).foregroundStyle(.secondary)
                }
            } header: {
                HStack(alignment: .firstTextBaseline) {
                    Text(page.title).accessibilityAddTraits(.isHeader)
                    Spacer()
                    Button {
                        onOpen(.page(page.slot))
                    } label: {
                        HStack(spacing: PaneMetrics.tight) {
                            Text("\(PageWords.fromTheOrchestrator) · \(PlanWords.ago(page.updatedAtMs, now: world.nowMs))")
                            Image(systemName: "chevron.forward").imageScale(.small)
                        }
                        .font(.footnote)
                        .textCase(nil)
                    }
                    .buttonStyle(.plain)
                    .foregroundStyle(.secondary)
                    .accessibilityLabel("Open \(page.title), \(PageWords.updated(page, now: world.nowMs))")
                    .accessibilityIdentifier("plan-anchored-open-\(page.slot)")
                }
            }
        }
    }
}
