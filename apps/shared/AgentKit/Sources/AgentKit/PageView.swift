import SwiftUI

// An orchestrator's page, drawn natively (ov-269 design 3, 6; ov-284): the
// nine blocks in the app's own type, spacing and tokens. Shared by the Mac and
// iOS, so it holds no AppKit or UIKit: everything a reference opens goes to
// `onOpen`, and only an `https` link goes to the system, through `openURL`.
//
// Color is for attention alone: amber on a block or cell the orchestrator
// marked `attention`, and on a question that still waits on the owner. Every
// state has its word beside its glyph. Rows have no rules; they're separated
// by spacing and an alternate fill.

/// A page whole: its title, when it was written and by whom, then its
/// blocks.
public struct PageView: View {
    public let page: BoardPage
    public let world: PageWorld
    public let onOpen: (PageDestination) -> Void

    public init(page: BoardPage, world: PageWorld, onOpen: @escaping (PageDestination) -> Void) {
        self.page = page
        self.world = world
        self.onOpen = onOpen
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: Spacing.section + Spacing.group) {
            VStack(alignment: .leading, spacing: Spacing.tight) {
                Text(page.doc?.title ?? page.title)
                    .font(.title2.weight(.semibold))
                    .textSelection(.enabled)
                    .accessibilityAddTraits(.isHeader)
                Text(byline)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .accessibilityIdentifier("page-updated")
            }
            if let doc = page.doc {
                PageBlocksView(doc: doc, world: world, onOpen: onOpen)
            } else {
                Text(PageWords.unreadable)
                    .foregroundStyle(.secondary)
                    .accessibilityIdentifier("page-unreadable")
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        // A container of its own, so the page's id doesn't stamp over every
        // row's inside it (`page-item-0`, `page-steps`).
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("page-\(page.slot)")
    }

    /// "Updated 12 min ago · manager".
    private var byline: String {
        ([PageWords.updated(page, now: world.nowMs)] + (page.actor.isEmpty ? [] : [page.actor])).joined(separator: " · ")
    }
}

/// A document's blocks, top to bottom: what a page draws under its title,
/// and what a theme's page draws for a page anchored to it.
public struct PageBlocksView: View {
    public let doc: PageDoc
    public let world: PageWorld
    public let onOpen: (PageDestination) -> Void
    /// The width to lay out for, when the caller knows it; else measured.
    public var width: CGFloat?
    /// The width offered, measured: what decides whether a table stacks.
    @State private var measured: CGFloat?

    public init(doc: PageDoc, world: PageWorld, width: CGFloat? = nil, onOpen: @escaping (PageDestination) -> Void) {
        self.doc = doc
        self.world = world
        self.width = width
        self.onOpen = onOpen
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: Spacing.section) {
            ForEach(Array(doc.blocks.enumerated()), id: \.offset) { index, block in
                PageBlockView(block: block, world: world, width: width ?? measured, onOpen: onOpen)
                    .padding(.top, Self.gapAbove(block, index: index))
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { measured = $0 }
    }

    /// A heading opens a new section, so it sits further from what's above.
    static func gapAbove(_ block: PageBlock, index: Int) -> CGFloat {
        if case .heading = block, index > 0 { return Spacing.group }
        return 0
    }
}

/// One block.
struct PageBlockView: View {
    let block: PageBlock
    let world: PageWorld
    let width: CGFloat?
    let onOpen: (PageDestination) -> Void
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        switch block {
        case .heading(let text):
            Text(text)
                .font(.headline)
                .foregroundStyle(.secondary)
                .accessibilityAddTraits(.isHeader)
        case .text(let md, let tone):
            PageTextView(md: md, tone: tone)
        case .stats(let items):
            PageStatsView(items: items)
        case .progress(let label, let done, let total, let detail, let parts):
            PageProgressView(label: label, done: done, total: total, detail: detail, parts: parts)
        case .table(let columns, let rows):
            if PageLayout.stacks(columns: columns.count, width: width) {
                PageStackedTable(columns: columns, rows: rows, world: world, onOpen: onOpen)
            } else {
                PageGridTable(columns: columns, rows: rows, world: world, onOpen: onOpen)
            }
        case .list(let items):
            PageListView(items: items, world: world, onOpen: onOpen)
        case .timeline(let entries, let given):
            PageTimelineView(entries: PageLayout.ordered(entries, given: given), world: world, onOpen: onOpen)
        case .steps(let steps):
            PageStepsView(steps: steps, down: PageLayout.stepsDown(width: width))
        case .links(let refs):
            PageFlow(spacing: Spacing.group, lineSpacing: Spacing.group) {
                ForEach(Array(refs.enumerated()), id: \.offset) { _, ref in
                    PageLinkChip(resolved: world.resolve(ref), onOpen: onOpen)
                }
            }
        case .unknown(_, let alt):
            Text(alt ?? PageWords.newerBlock)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityIdentifier("page-block-unknown")
        }
    }
}

// MARK: - Text

/// A text block: the Markdown subset, amber when the orchestrator marked it.
struct PageTextView: View {
    let md: String
    let tone: PageTone
    @Environment(\.taskKeyLinker) private var linker
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.group) {
            ForEach(Array(PageMarkdown.pieces(md).enumerated()), id: \.offset) { _, piece in
                switch piece {
                case .prose(let text):
                    Text(linker.linked(PageMarkdown.inline(text)))
                case .item(let marker, let text, let depth):
                    HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                        Text(marker).foregroundStyle(.secondary)
                        Text(linker.linked(PageMarkdown.inline(text)))
                    }
                    .padding(.leading, CGFloat(depth) * Spacing.section)
                case .plain(let text):
                    Text(verbatim: text)
                }
            }
        }
        .fixedSize(horizontal: false, vertical: true)
        .foregroundStyle(tone == .attention ? AnyShapeStyle(Tint.attention(scheme)) : AnyShapeStyle(.primary))
        .textSelection(.enabled)
        .environment(\.openURL, PageView.openGuard(linker))
    }
}

extension PageView {
    /// What a link in a page's text may open: a task, in the app; an `https`
    /// page, in the browser; nothing else.
    static func openGuard(_ linker: TaskKeyLinker) -> OpenURLAction {
        OpenURLAction { url in
            if TaskKeyLinks.parse(url) != nil {
                MainActor.assumeIsolated { _ = linker.follow(url) }
                return .handled
            }
            return PageLinks.https(url.absoluteString) != nil ? .systemAction : .discarded
        }
    }
}

// MARK: - Stats and progress

/// A row of figures that wraps.
struct PageStatsView: View {
    let items: [PageStat]
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        PageFlow(spacing: Spacing.section * 2, lineSpacing: Spacing.section) {
            ForEach(Array(items.enumerated()), id: \.offset) { _, stat in
                VStack(alignment: .leading, spacing: Spacing.tight / 2) {
                    Text(stat.label)
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                    Text(stat.value)
                        .font(.title2.weight(.semibold).monospacedDigit())
                        .foregroundStyle(stat.tone == .attention ? AnyShapeStyle(Tint.attention(scheme)) : AnyShapeStyle(.primary))
                    if let detail = stat.detail {
                        Text(detail)
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                    }
                }
                .accessibilityElement(children: .combine)
            }
        }
    }
}

/// A bar in neutral fills, with "3 of 10" in words.
struct PageProgressView: View {
    let label: String
    let done: Int
    let total: Int
    let detail: String?
    let parts: [PagePart]
    @Environment(\.colorSchemeContrast) private var contrast

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.tight + Spacing.tight / 2) {
            HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                Text(label).fontWeight(.medium)
                Spacer(minLength: Spacing.group)
                Text(PageWords.progress(done: done, total: total))
                    .monospacedDigit()
                    .foregroundStyle(.secondary)
            }
            bar
            if !parts.isEmpty || detail != nil {
                Text(([detail].compactMap { $0 } + parts.map { "\($0.label) \($0.count)" }).joined(separator: " · "))
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(label)
        .accessibilityValue(
            ([PageWords.progress(done: done, total: total)] + [detail].compactMap { $0 } + parts.map { "\($0.label) \($0.count)" })
                .joined(separator: ", "))
    }

    /// Done darkest, then each part in a lighter fill; without parts, done
    /// alone.
    private var bar: some View {
        GeometryReader { proxy in
            HStack(spacing: 1) {
                ForEach(Array(segments.enumerated()), id: \.offset) { index, fraction in
                    Rectangle()
                        .fill(Self.style(index))
                        .frame(width: max(0, proxy.size.width * fraction - 1))
                }
                Spacer(minLength: 0)
            }
            .clipShape(Capsule())
        }
        .frame(height: PageMetrics.bar)
        .background(Capsule().fill(Fill.inset(contrast)))
    }

    private var segments: [Double] {
        guard total > 0 else { return [] }
        if parts.isEmpty { return [PageLayout.fraction(done: done, total: total)] }
        var left = 1.0
        return parts.prefix(4).map { part in
            let f = min(left, PageLayout.fraction(done: part.count, total: total))
            left -= f
            return f
        }
    }

    static func style(_ index: Int) -> AnyShapeStyle {
        switch index {
        case 0: AnyShapeStyle(.secondary)
        case 1: AnyShapeStyle(.tertiary)
        case 2: AnyShapeStyle(.quaternary)
        default: AnyShapeStyle(.quinary)
        }
    }
}

/// Sizes a page draws with, beside the shared spacing.
enum PageMetrics {
    /// A progress bar's height.
    static let bar: CGFloat = 6
    /// A list row's and a step's glyph column.
    static let glyph: CGFloat = 18
    /// A timeline's time column.
    static let time: CGFloat = 96
}

// MARK: - List, timeline, steps, links

/// The trailing words and control of a row with a live reference: a
/// question's "Needs you" in amber, a card's status, a link's domain.
struct PageRefTrailer: View {
    let resolved: PageResolved
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: Spacing.tight) {
            // A web link with no label of its own has its domain as its name:
            // the row's words are the orchestrator's, so the domain is said
            // here (design 7: a link always shows where it goes).
            if let status = resolved.status ?? (resolved.destination?.isExternal == true ? resolved.name : nil) {
                Text(status)
                    .fontWeight(resolved.statusTone == .attention ? .medium : .regular)
                    .foregroundStyle(resolved.statusTone == .attention ? AnyShapeStyle(Tint.attention(scheme)) : AnyShapeStyle(.secondary))
                    // Wraps rather than truncating: the status is the point
                    // of a live reference (review M2).
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let destination = resolved.destination {
                Image(systemName: destination.isExternal ? "arrow.up.right" : "chevron.forward")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.tertiary)
                    .accessibilityHidden(true)
            }
        }
    }
}

extension PageDestination {
    /// Whether it leaves the app.
    var isExternal: Bool { if case .url = self { true } else { false } }
}

/// A glyph for a state, in its column; nothing for `none`.
struct PageStateGlyph: View {
    let state: PageState
    let tone: PageTone
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        Image(systemName: state.symbol)
            .font(.subheadline)
            .foregroundStyle(tone == .attention ? AnyShapeStyle(Tint.attention(scheme)) : AnyShapeStyle(.secondary))
            .opacity(state == .none ? 0 : 1)
            .frame(width: PageMetrics.glyph)
            .accessibilityHidden(true)
    }
}

/// Rows with a state glyph *and* its word, read-only: a checklist, a risk
/// list, a "waiting on" list. A row with a reference that resolves opens it.
struct PageListView: View {
    let items: [PageItem]
    let world: PageWorld
    let onOpen: (PageDestination) -> Void
    @Environment(\.colorScheme) private var scheme
    @Environment(\.colorSchemeContrast) private var contrast
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(items.enumerated()), id: \.offset) { index, item in
                let resolved = item.ref.map(world.resolve)
                PageRow(destination: resolved?.destination, shaded: index % 2 == 1, onOpen: onOpen) {
                    if PageLayout.trailerBelow(typeSize) {
                        // At accessibility sizes the state and status go under
                        // the words, whole, rather than squeezing beside them.
                        HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                            PageStateGlyph(state: item.state, tone: item.tone)
                            VStack(alignment: .leading, spacing: Spacing.tight / 2) {
                                words(item)
                                if let word = item.state.word { Text(word).foregroundStyle(.secondary) }
                                if let resolved { PageRefTrailer(resolved: resolved).font(.subheadline) }
                            }
                        }
                    } else {
                    HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                        PageStateGlyph(state: item.state, tone: item.tone)
                        VStack(alignment: .leading, spacing: Spacing.tight / 2) {
                            Text(item.text)
                                .foregroundStyle(item.tone == .attention ? AnyShapeStyle(Tint.attention(scheme)) : AnyShapeStyle(.primary))
                                .fixedSize(horizontal: false, vertical: true)
                            if let detail = item.detail {
                                Text(detail)
                                    .font(.subheadline)
                                    .foregroundStyle(.secondary)
                                    .fixedSize(horizontal: false, vertical: true)
                            }
                        }
                        Spacer(minLength: Spacing.group)
                        VStack(alignment: .trailing, spacing: Spacing.tight / 2) {
                            if let word = item.state.word {
                                Text(word).foregroundStyle(.secondary)
                            }
                            if let resolved { PageRefTrailer(resolved: resolved).font(.subheadline) }
                        }
                    }
                    }
                }
                .accessibilityLabel(Self.spoken(item, resolved: resolved))
                .accessibilityIdentifier("page-item-\(index)")
            }
        }
    }

    /// An item's words and detail.
    @ViewBuilder private func words(_ item: PageItem) -> some View {
        Text(item.text)
            .foregroundStyle(item.tone == .attention ? AnyShapeStyle(Tint.attention(scheme)) : AnyShapeStyle(.primary))
            .fixedSize(horizontal: false, vertical: true)
        if let detail = item.detail {
            Text(detail)
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    /// "Blocked, Sidebar tint vs. terminal theme is undecided, Holds
    /// ov-222's last pass, ov-222, Round half-cents, Needs you".
    static func spoken(_ item: PageItem, resolved: PageResolved?) -> String {
        ([item.state.word, item.text, item.detail, resolved?.spoken].compactMap { $0 }).joined(separator: ", ")
    }
}

/// A row that opens its destination when it has one, with the hover and
/// alternate fills every page row shares, and one accessibility element.
struct PageRow<Content: View>: View {
    let destination: PageDestination?
    let shaded: Bool
    let onOpen: (PageDestination) -> Void
    @ViewBuilder let content: () -> Content
    @State private var hovering = false
    @Environment(\.colorSchemeContrast) private var contrast
    @Environment(\.openURL) private var openURL

    var body: some View {
        Group {
            if let destination {
                Button {
                    PageOpen.open(destination, onOpen: onOpen, openURL: openURL)
                } label: {
                    padded.contentShape(.control)
                }
                .buttonStyle(.plain)
                .onHover { hovering = $0 }
                .accessibilityAddTraits(.isButton)
            } else {
                padded
            }
        }
        .accessibilityElement(children: .combine)
    }

    private var padded: some View {
        content()
            .padding(.vertical, Spacing.group - 2)
            .padding(.horizontal, Spacing.group)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background {
                if hovering && destination != nil {
                    RoundedRectangle.control.fill(Fill.hover)
                } else if shaded {
                    RoundedRectangle.control.fill(Fill.inset(contrast))
                }
            }
    }
}

/// The one place a destination is opened: `https` to the system, the rest to
/// the app.
enum PageOpen {
    static func open(_ destination: PageDestination, onOpen: (PageDestination) -> Void, openURL: OpenURLAction) {
        if case .url(let url) = destination {
            if PageLinks.https(url.absoluteString) != nil { openURL(url) }
            return
        }
        onOpen(destination)
    }
}

/// When each thing happened, in the viewer's zone.
struct PageTimelineView: View {
    let entries: [PageEntry]
    let world: PageWorld
    let onOpen: (PageDestination) -> Void
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(entries.enumerated()), id: \.offset) { index, entry in
                let resolved = entry.ref.map(world.resolve)
                PageRow(destination: resolved?.destination, shaded: false, onOpen: onOpen) {
                    if PageLayout.trailerBelow(typeSize) {
                        // At accessibility sizes: the time, the words, then
                        // the status, each whole (review M2).
                        VStack(alignment: .leading, spacing: Spacing.tight / 2) {
                            Text(PageLayout.time(entry.at, now: world.nowMs))
                                .monospacedDigit()
                                .foregroundStyle(.secondary)
                            Text(entry.text).fixedSize(horizontal: false, vertical: true)
                            if let resolved { PageRefTrailer(resolved: resolved).font(.subheadline) }
                        }
                    } else {
                    HStack(alignment: .firstTextBaseline, spacing: Spacing.inset) {
                        Text(PageLayout.time(entry.at, now: world.nowMs))
                            .monospacedDigit()
                            .foregroundStyle(.secondary)
                            .frame(minWidth: PageMetrics.time, alignment: .leading)
                        Text(entry.text).fixedSize(horizontal: false, vertical: true)
                        Spacer(minLength: Spacing.group)
                        if let resolved { PageRefTrailer(resolved: resolved).font(.subheadline) }
                    }
                    }
                }
                .accessibilityLabel(
                    ([PageLayout.time(entry.at, now: world.nowMs), entry.text, resolved?.spoken].compactMap { $0 }).joined(separator: ", "))
                .accessibilityIdentifier("page-entry-\(index)")
            }
        }
    }
}

/// A pipeline of chips: across, wrapping, on a wide surface; down the page
/// on a narrow one.
struct PageStepsView: View {
    let steps: [PageStep]
    let down: Bool

    var body: some View {
        Group {
            if down {
                VStack(alignment: .leading, spacing: Spacing.tight) {
                    ForEach(Array(steps.enumerated()), id: \.offset) { _, step in chip(step) }
                }
            } else {
                PageFlow(spacing: Spacing.tight, lineSpacing: Spacing.group) {
                    ForEach(Array(steps.enumerated()), id: \.offset) { index, step in
                        HStack(spacing: Spacing.tight) {
                            if index > 0 {
                                Image(systemName: "chevron.forward")
                                    .font(.caption.weight(.semibold))
                                    .foregroundStyle(.tertiary)
                                    .accessibilityHidden(true)
                            }
                            chip(step)
                        }
                    }
                }
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("page-steps")
    }

    private func chip(_ step: PageStep) -> some View {
        HStack(spacing: Spacing.tight) {
            PageStateGlyph(state: step.state, tone: .neutral)
            Text(step.label)
                .fontWeight(step.state == .active ? .semibold : .regular)
        }
        .padding(.vertical, Spacing.tight)
        .padding(.trailing, Spacing.group)
        .surface(.inset, in: Capsule())
        .accessibilityElement(children: .ignore)
        .accessibilityLabel([step.label, step.state.word].compactMap { $0 }.joined(separator: ", "))
    }
}

/// A link chip: an app destination, or an `https` page with its domain
/// beside its label. One that resolves to nothing is plain words.
struct PageLinkChip: View {
    let resolved: PageResolved
    let onOpen: (PageDestination) -> Void
    @State private var hovering = false
    @Environment(\.openURL) private var openURL

    var body: some View {
        if let destination = resolved.destination {
            Button {
                PageOpen.open(destination, onOpen: onOpen, openURL: openURL)
            } label: {
                HStack(alignment: .firstTextBaseline, spacing: Spacing.tight) {
                    Text(resolved.name).foregroundStyle(.tint)
                    if destination.isExternal, let domain = resolved.status {
                        Text(domain).foregroundStyle(.secondary)
                    }
                    Image(systemName: destination.isExternal ? "arrow.up.right" : "chevron.forward")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.tertiary)
                }
                .padding(.vertical, Spacing.tight)
                .padding(.horizontal, Spacing.inset)
                .background {
                    if hovering { Capsule().fill(Fill.hover) }
                }
                .surface(.inset, in: Capsule())
                .contentShape(Capsule())
            }
            .buttonStyle(.plain)
            .onHover { hovering = $0 }
            .accessibilityLabel(resolved.spoken)
        } else {
            Text(resolved.name)
                .foregroundStyle(.secondary)
                .padding(.vertical, Spacing.tight)
        }
    }
}
