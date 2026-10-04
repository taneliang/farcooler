import AgentKit
import SwiftUI

// The board's shared components (ov-92): one collapsible section, one section
// title style and one navigator row style, so every disclosure in the
// navigator looks, moves and answers the keyboard and VoiceOver the same way.
// ov-101 made them the whole app's: every collapsible is a
// `CollapsibleSection`, or, where its rows are siblings of its header in a
// flat lazy list, a `DisclosureButton`; both draw the one `DisclosureChevron`
// and move on `BoardMotion`. The owner, 2 Oct: "it's very cool that the
// worktrees list in the board animates when opening and closing. why don't we
// do that with the other collapsibles as well? … make board UI components
// reusable so that you can ensure that they look and feel consistent."

/// How a section's title is set.
enum SectionHeaderStyle: Equatable {
    /// A navigator section's: Orchestrator, Tasks, Worktrees. Small capitals,
    /// secondary, like a Mac source list's.
    case navigator
    /// A group inside one: a task status, Unread. The pane's
    /// heading size, semibold.
    case group
    /// A quiet group: Hidden, under Worktrees.
    case minor

    @MainActor var font: Font {
        switch self {
        case .navigator: .system(size: WorkspaceStyle.PaneText.secondary, weight: .semibold)
        case .group: .system(size: WorkspaceStyle.PaneText.title, weight: .semibold)
        case .minor: .system(size: WorkspaceStyle.PaneText.secondary)
        }
    }

    var uppercased: Bool { self == .navigator }
}

/// A section's title in its style, and its tone: primary, quiet (an empty
/// section, a navigator heading) or the accent (Needs Decision).
struct SectionTitle: View {
    enum Tone { case primary, quiet, accent }

    let text: String
    var style: SectionHeaderStyle = .group
    var tone: Tone = .primary
    /// The row it's marked on for `GridGeometryTests`, if any.
    var gridRow: String?

    var body: some View {
        if let gridRow { title.gridMark(gridRow, .text) } else { title }
    }

    private var title: some View {
        Text(text)
            .font(style.font)
            .textCase(style.uppercased ? .uppercase : nil)
            .kerning(style.uppercased ? 0.4 : 0)
            .foregroundStyle(color)
            .lineLimit(1)
            .truncationMode(.tail)
    }

    private var color: Color {
        if style != .group { return .secondary }
        switch tone {
        case .primary: return .primary
        case .quiet: return .secondary
        case .accent: return .accentColor
        }
    }
}

/// How a section's header and content sit: the room between them, the
/// chevron's cell, the header's least height and its insets. One per place a
/// section is drawn, so a section's look is chosen by naming where it is.
struct SectionMetrics: Equatable {
    /// Between the header and what it opens.
    var spacing: CGFloat
    /// The chevron's cell, which is one column of the grid where there is one.
    var chevronWidth: CGFloat = ColumnGrid.step
    /// Where the chevron sits in its cell: the navigator's is centered in
    /// the glyph column, so it shares an x with the glyphs in boxes.
    var chevronAlignment: Alignment = .leading
    /// Room after the chevron's cell, before the title.
    var chevronGap: CGFloat = 0
    var minHeight: CGFloat = ColumnGrid.rowHeight
    /// Over and under the header's title, inside its hit target: the
    /// navigator's `NavigatorRhythm.air`, so a header's slot is its line
    /// and the same air a row has.
    var headerAir: CGFloat = 0
    /// Around the header alone: the content keeps its own.
    var headerInsets = EdgeInsets()
    /// Whether its header is a heading for VoiceOver's rotor: a section's
    /// is; a tool call's or a card's details' isn't (ov-101 review).
    var isHeading = false

    /// The board's navigator: its sections, task statuses and groups.
    /// Its header's slot is its title and `NavigatorRhythm.air`, and its
    /// content's first slot touches it (ov-243).
    static let navigator = SectionMetrics(
        spacing: NavigatorRhythm.row, chevronWidth: NavigatorGrid.mark, chevronAlignment: .center, chevronGap: NavigatorGrid.gap,
        minHeight: 0, headerAir: NavigatorRhythm.air, isHeading: true)
    /// A small disclosure in running text: a thought, a card's details, a
    /// settings row's.
    static let inline = SectionMetrics(spacing: 6, chevronWidth: 14, minHeight: 0)
    /// The header of a filled box whose content is under a divider: a tool
    /// call, a subagent's block, the plan.
    static let card = SectionMetrics(
        spacing: 0, chevronWidth: 14, minHeight: 0,
        headerInsets: EdgeInsets(top: 6, leading: 9, bottom: 6, trailing: 9))
}

/// What opens and closes a section.
enum SectionToggle {
    /// The whole header row: a section heading.
    case row
    /// The chevron alone, the rest of the header being a control of its own
    /// (a row you select, as Finder's outline rows are).
    case chevron
}

/// The disclosure chevron, the one way the app draws one (ov-101): at the
/// leading edge (the owner's rule: on the left), pointing right closed and
/// down open, turning on whatever animation the change runs in. No view draws
/// a disclosure chevron of its own; `CollapsibleSectionTests` fails on one.
struct DisclosureChevron: View {
    let expanded: Bool
    /// Drawn: none on a section that can't open, since a dimmed chevron read
    /// as a disabled control (ov-104 review).
    var visible = true

    var body: some View {
        Image(systemName: "chevron.right")
            .font(.system(size: 9, weight: .bold))
            .rotationEffect(.degrees(expanded ? 90 : 0))
            .foregroundStyle(.secondary)
            .opacity(visible ? 1 : 0)
            .accessibilityHidden(true)
    }
}

/// A disclosure's chevron on its own, as a button, for the disclosures whose
/// rows are siblings of their header in a flat, lazy list rather than inside
/// it, which `CollapsibleSection` can't hold: a workspace's worktrees and a
/// worktree's terminals in the sidebar, a file's lines in the diff. Both of
/// those lists were flattened on purpose, so that only the rows on screen are
/// built. It turns on the shared spring (`BoardMotion.toggle`); the list gives
/// its rows `BoardMotion.rowTransition`.
struct DisclosureButton: View {
    let expanded: Bool
    /// What VoiceOver hears, before Expanded or Collapsed.
    let accessibilityLabel: String
    /// The row its chevron is marked on for `GridGeometryTests`, if any.
    var gridRow: String?
    var width: CGFloat = ColumnGrid.step
    var height: CGFloat = 16
    let action: () -> Void

    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.boardMotionSlowdown) private var slowdown

    var body: some View {
        Button {
            BoardMotion.toggle(reduceMotion: reduceMotion, slowedBy: slowdown, action)
        } label: {
            DisclosureChevron(expanded: expanded)
                .frame(width: width, height: height, alignment: .leading)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .modifier(OptionalGridMark(row: gridRow))
        .accessibilityLabel(accessibilityLabel)
        .accessibilityValue(expanded ? "Expanded" : "Collapsed")
    }
}

private struct OptionalGridMark: ViewModifier {
    let row: String?
    func body(content: Content) -> some View {
        if let row { content.gridMark(row, .chevron) } else { content }
    }
}

/// Every collapsible section the app draws: a header row (its chevron at
/// column A, its title at B, an accessory and its count trailing) that
/// expands and collapses on the shared spring, its content fading and sliding
/// in and out as it goes, and only fading under Reduce Motion; its state kept
/// by key or by the caller; Space, ← and → on a focused header; and
/// VoiceOver hearing a button with its state.
///
/// Every one registers its `id` (`CollapsibleSectionsKey`), so a test can say
/// which sections were drawn through it.
struct CollapsibleSection<Label: View, Accessory: View, Content: View>: View {
    /// Which section, for the registry, the grid's marks and the
    /// accessibility identifier.
    let id: String
    var style: SectionHeaderStyle = .group
    var metrics: SectionMetrics = .navigator
    var toggle: SectionToggle = .row
    /// Whether it can open at all: an empty task status can't.
    var canExpand = true
    var count: Int?
    /// What VoiceOver hears for the header, before its state.
    let accessibilityLabel: String
    /// Whether the header's label fills the row, making the whole row the
    /// toggle; off where an accessory sits right after the chevron.
    var fillsRow = true
    /// The header's label, given whether it's open.
    @ViewBuilder let label: (Bool) -> Label
    @ViewBuilder let accessory: () -> Accessory
    @ViewBuilder let content: () -> Content
    /// What VoiceOver offers on the header besides opening and closing it:
    /// a hover-only accessory's action (`headerAction(_:)`).
    var headerAction: SectionHeaderAction?

    /// The caller's state, when it keeps it.
    private var binding: Binding<Bool>?
    /// Kept here, by `key` in `defaults`, otherwise.
    private let key: String?
    private let defaults: UserDefaults
    @State private var stored: Bool
    /// Closing, until its spring settles: what the content is clipped for.
    @State private var clip = SectionClip()
    /// The pointer is over the header row (`sectionHeaderHovered`).
    @State private var headerHovered = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.boardMotionSlowdown) private var slowdown

    /// A section whose open state the caller keeps.
    init(
        id: String, style: SectionHeaderStyle = .group, metrics: SectionMetrics = .navigator,
        toggle: SectionToggle = .row, isExpanded: Binding<Bool>, canExpand: Bool = true,
        count: Int? = nil, accessibilityLabel: String, fillsRow: Bool = true,
        @ViewBuilder label: @escaping (Bool) -> Label, @ViewBuilder accessory: @escaping () -> Accessory,
        @ViewBuilder content: @escaping () -> Content
    ) {
        self.id = id
        self.style = style
        self.metrics = metrics
        self.toggle = toggle
        self.canExpand = canExpand
        self.count = count
        self.accessibilityLabel = accessibilityLabel
        self.fillsRow = fillsRow
        self.label = label
        self.accessory = accessory
        self.content = content
        self.binding = isExpanded
        self.key = nil
        self.defaults = .standard
        _stored = State(initialValue: isExpanded.wrappedValue)
    }

    /// A section that keeps its own open state, under `key` in `defaults`.
    init(
        id: String, style: SectionHeaderStyle = .group, metrics: SectionMetrics = .navigator,
        toggle: SectionToggle = .row, key: String, defaults: UserDefaults = .standard,
        expandedByDefault: Bool = true, canExpand: Bool = true, count: Int? = nil, accessibilityLabel: String,
        fillsRow: Bool = true,
        @ViewBuilder label: @escaping (Bool) -> Label, @ViewBuilder accessory: @escaping () -> Accessory,
        @ViewBuilder content: @escaping () -> Content
    ) {
        self.id = id
        self.style = style
        self.metrics = metrics
        self.toggle = toggle
        self.canExpand = canExpand
        self.count = count
        self.accessibilityLabel = accessibilityLabel
        self.fillsRow = fillsRow
        self.label = label
        self.accessory = accessory
        self.content = content
        self.binding = nil
        self.key = key
        self.defaults = defaults
        _stored = State(initialValue: Self.read(key: key, defaults: defaults, default: expandedByDefault))
    }

    /// The row its marks are reported on: its id up to the first dot, so
    /// every task status's header is one row, `status`.
    static func gridRow(_ id: String) -> String { String(id.split(separator: ".").first ?? "") }

    /// The stored open state: absent, `default`.
    static func read(key: String, defaults: UserDefaults, default expanded: Bool) -> Bool {
        defaults.object(forKey: key).map { _ in defaults.bool(forKey: key) } ?? expanded
    }

    /// What its content does coming and going: fades and slides down a
    /// rhythm in, and fades out fast, before the headers under it have
    /// closed up over it (the rows' rule, `BoardMotion.rowTransition`); only
    /// fades under Reduce Motion.
    static func contentTransition(reduceMotion: Bool, slowedBy: Double = 1) -> AnyTransition {
        guard !reduceMotion else { return .opacity }
        return .asymmetric(
            insertion: .opacity.combined(with: .offset(y: -ColumnGrid.rhythm)),
            removal: .opacity.animation(.easeOut(duration: 0.08 * slowedBy)))
    }

    private var expanded: Bool { canExpand && (binding?.wrappedValue ?? stored) }

    /// Open or close it, on the shared spring.
    private func set(_ open: Bool) {
        guard canExpand, open != expanded else { return }
        let closing = open ? nil : clip.close()
        if open { clip.open() }
        BoardMotion.toggle(reduceMotion: reduceMotion, slowedBy: slowdown) {
            if let binding { binding.wrappedValue = open } else { stored = open }
        } completion: {
            if let closing { clip.settle(closing) }
        }
        if let key { defaults.set(open, forKey: key) }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
            // In a container of its own, clipped while it closes: the
            // container's height springs to nothing while the leaving content
            // is still drawn at its full height, so the clip hides it behind
            // the edge that's rising, and the next section's header never
            // slides over its fading rows (ov-109's live check, in slowed
            // frames). Only while it closes: a task row moving into an open
            // section flies in from outside it (`matchedGeometryEffect`), and
            // a clip would cut it off at the edge.
            VStack(alignment: .leading, spacing: 0) {
                if expanded {
                    content()
                        .padding(.top, metrics.spacing)  // rhythm-exempt: the navigator's is NavigatorRhythm.row
                        .transition(Self.contentTransition(reduceMotion: reduceMotion, slowedBy: slowdown))
                }
            }
            .clipShape(ClipWhile(clips: clip.closing))
        }
        // Closed from outside, too: a transcript row folding itself once the
        // turn moves on. `set` has already said so for a click. Opened again
        // before that close settles, it stops clipping at once (ov-101
        // review): an opening section is never clipped.
        .onChange(of: expanded) { was, now in
            if now {
                clip.open()
            } else if was, !clip.closing {
                let closing = clip.close()
                let settle = 0.6 * slowdown
                Task {
                    try? await Task.sleep(for: .seconds(settle))
                    clip.settle(closing)
                }
            }
        }
        // Added to, not set: a section inside another one registers too.
        .transformPreference(CollapsibleSectionsKey.self) { $0.insert(id) }
    }

    private var chevron: some View {
        // Marked on the glyph itself, not its cell, so a test reads where
        // the caret really draws.
        DisclosureChevron(expanded: expanded, visible: canExpand)
            .gridMark(Self.gridRow(id), .chevron)
            .frame(width: metrics.chevronWidth, alignment: metrics.chevronAlignment)
            .padding(.trailing, metrics.chevronGap)
    }

    private var header: some View {
        HStack(spacing: 0) {
            switch toggle {
            case .row:
                toggleButton {
                    HStack(spacing: 0) {
                        chevron
                        label(expanded)
                        if fillsRow { Spacer(minLength: SidebarGrid.gap) }
                    }
                }
            case .chevron:
                toggleButton { chevron }
                label(expanded)
                if fillsRow { Spacer(minLength: SidebarGrid.gap) }
            }
            accessory()
                .environment(\.sectionHeaderHovered, headerHovered)
            if !fillsRow { Spacer(minLength: 0) }
            if let count { SectionCount(count: count).accessibilityHidden(true) }
        }
        .padding(metrics.headerInsets)  // rhythm-exempt: none in the navigator; a card's own
        .contentShape(Rectangle())
        .onHover { headerHovered = $0 }
        .probed("section-header-\(id)")
    }

    private func toggleButton<Face: View>(@ViewBuilder _ face: () -> Face) -> some View {
        Button { set(!expanded) } label: {
            face()
                .padding(.vertical, metrics.headerAir)  // rhythm-exempt: the navigator's is NavigatorRhythm.air
                .frame(minHeight: metrics.minHeight)  // rhythm-exempt: 0 in the navigator
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(!canExpand)
        .onKeyPress(.leftArrow) {
            guard expanded else { return .ignored }
            set(false)
            return .handled
        }
        .onKeyPress(.rightArrow) {
            guard !expanded, canExpand else { return .ignored }
            set(true)
            return .handled
        }
        .onKeyPress(.space) {
            set(!expanded)
            return .handled
        }
        .accessibilityLabel(count.map { "\(accessibilityLabel), \($0)" } ?? accessibilityLabel)
        .accessibilityValue(canExpand ? (expanded ? "Expanded" : "Collapsed") : "")
        .accessibilityAddTraits(metrics.isHeading ? [.isHeader, .isButton] : .isButton)
        .accessibilityActions {
            if let headerAction { Button(headerAction.name, action: headerAction.perform) }
        }
        .accessibilityIdentifier("section-\(id)")
    }
}

extension CollapsibleSection where Label == SectionTitle {
    /// A section titled in its style, keeping its own state by `key`.
    init(
        _ title: String, id: String, style: SectionHeaderStyle = .group, metrics: SectionMetrics = .navigator,
        tone: SectionTitle.Tone = .primary,
        key: String, defaults: UserDefaults = .standard, expandedByDefault: Bool = true, canExpand: Bool = true,
        count: Int? = nil,
        @ViewBuilder accessory: @escaping () -> Accessory, @ViewBuilder content: @escaping () -> Content
    ) {
        self.init(
            id: id, style: style, metrics: metrics, key: key, defaults: defaults, expandedByDefault: expandedByDefault,
            canExpand: canExpand, count: count, accessibilityLabel: title,
            label: { _ in SectionTitle(text: title, style: style, tone: tone, gridRow: Self.gridRow(id)) }, accessory: accessory,
            content: content)
    }

    /// A section titled in its style, whose state the caller keeps.
    init(
        _ title: String, id: String, style: SectionHeaderStyle = .group, metrics: SectionMetrics = .navigator,
        tone: SectionTitle.Tone = .primary,
        isExpanded: Binding<Bool>, canExpand: Bool = true, count: Int? = nil,
        @ViewBuilder accessory: @escaping () -> Accessory, @ViewBuilder content: @escaping () -> Content
    ) {
        self.init(
            id: id, style: style, metrics: metrics, isExpanded: isExpanded, canExpand: canExpand, count: count,
            accessibilityLabel: title, label: { _ in SectionTitle(text: title, style: style, tone: tone, gridRow: Self.gridRow(id)) },
            accessory: accessory, content: content)
    }
}

extension CollapsibleSection where Label == SectionTitle, Accessory == EmptyView {
    init(
        _ title: String, id: String, style: SectionHeaderStyle = .group, metrics: SectionMetrics = .navigator,
        tone: SectionTitle.Tone = .primary,
        key: String, defaults: UserDefaults = .standard, expandedByDefault: Bool = true, canExpand: Bool = true,
        count: Int? = nil, @ViewBuilder content: @escaping () -> Content
    ) {
        self.init(
            title, id: id, style: style, metrics: metrics, tone: tone, key: key, defaults: defaults,
            expandedByDefault: expandedByDefault, canExpand: canExpand, count: count, accessory: { EmptyView() },
            content: content)
    }

    init(
        _ title: String, id: String, style: SectionHeaderStyle = .group, metrics: SectionMetrics = .navigator,
        tone: SectionTitle.Tone = .primary,
        isExpanded: Binding<Bool>, canExpand: Bool = true, count: Int? = nil,
        @ViewBuilder content: @escaping () -> Content
    ) {
        self.init(
            title, id: id, style: style, metrics: metrics, tone: tone, isExpanded: isExpanded, canExpand: canExpand, count: count,
            accessory: { EmptyView() }, content: content)
    }
}

/// A header's count (ov-104, owner: "parentheses are a plain-text habit"):
/// right-aligned at the trailing edge, tertiary, in tabular digits, as Mail
/// and Xcode draw theirs. Never "Title (N)".
struct SectionCount: View {
    let text: String

    init(count: Int) { text = "\(count)" }
    /// Progress, "3 of 7": the plan's.
    init(_ done: Int, of total: Int) { text = "\(done) of \(total)" }

    var body: some View {
        Text(text)
            .font(.system(size: WorkspaceStyle.PaneText.secondary))
            .monospacedDigit()
            .foregroundStyle(.tertiary)
            .lineLimit(1)
    }
}

/// A group's header that doesn't open and close: Unread's Finished, New and
/// Activity, the History page's Today and Earlier. Its title in the quiet
/// group style, and its count trailing (`SectionCount`).
///
/// Set apart from the rows over it and close to its own (ov-177, the owner:
/// "more vertical spacing around subheadings … it's a little hard to notice
/// them"): a group that `follows` another has `above` more room over it
/// than the rows have between them, and its first row touches its slot, as a
/// row touches the next (`NavigatorRhythm`, ov-243).
struct GroupHeader: View {
    let title: String
    let count: Int?
    /// Another group's rows are over it; false for the first under its
    /// section's header, which has room enough.
    var follows = false

    /// The room a following group's header takes over its slot: a
    /// subgroup's (`NavigatorRhythm.subgroup`).
    static let above = NavigatorRhythm.subgroup

    var body: some View {
        HStack(spacing: SidebarGrid.gap) {
            SectionTitle(text: title, style: .minor).probed("subgroup-title")
            Spacer(minLength: 0)
            if let count { SectionCount(count: count) }
        }
        .padding(.vertical, NavigatorRhythm.air)
        .padding(.top, follows ? Self.above : 0)  // rhythm-exempt: Self.above is NavigatorRhythm.subgroup
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(count.map { "\(title), \($0)" } ?? title)
        .accessibilityAddTraits(.isHeader)
    }
}

/// Whether a section's content is clipped: only while it closes, until that
/// close settles or it opens again, whichever comes first. Each close is
/// numbered, so a close that settles after the section has opened (and
/// perhaps closed again) changes nothing.
struct SectionClip: Equatable {
    private(set) var closing = false
    private var generation = 0

    /// It began to close: clip, until `settle` with what this returns.
    mutating func close() -> Int {
        generation += 1
        closing = true
        return generation
    }

    /// It began to open: never clipped.
    mutating func open() {
        generation += 1
        closing = false
    }

    /// The close numbered `close` has settled.
    mutating func settle(_ close: Int) {
        if close == generation { closing = false }
    }
}

/// The section's own bounds while `clips`, and otherwise a bound far
/// outside them: a clip that comes and goes without the content under it
/// changing identity.
struct ClipWhile: Shape {
    var clips: Bool

    func path(in rect: CGRect) -> Path {
        Path(clips ? rect : rect.insetBy(dx: -10_000, dy: -10_000))
    }
}

/// The sections drawn through `CollapsibleSection`, by id.
struct CollapsibleSectionsKey: PreferenceKey {
    static let defaultValue: Set<String> = []
    static func reduce(value: inout Set<String>, nextValue: () -> Set<String>) {
        value.formUnion(nextValue())
    }
}

extension View {
    /// A row in the navigator (the orchestrator's, a worktree's): its inset
    /// on the grid, and its selection drawn as a Mac list draws one, in the
    /// accent while the navigator has the keyboard, else gray, reaching
    /// `NavigatorGrid.outset` past the row's content on both sides. `box`
    /// names the row for `GridGeometryTests`, which reads where it starts.
    func navigatorRow(
        selected: Bool, keyed: Bool, minHeight: CGFloat = 0,
        leading: CGFloat = NavigatorGrid.textInset, trailing: CGFloat = ColumnGrid.step, box: String? = nil
    ) -> some View {
        modifier(
            NavigatorRowStyle(
                selected: selected, keyed: keyed, minHeight: minHeight, leading: leading, trailing: trailing, box: box))
    }
}

struct NavigatorRowStyle: ViewModifier {
    let selected: Bool
    let keyed: Bool
    /// None in the navigator: a row's slot is its lines and its air, so
    /// every row reads `2 * NavigatorRhythm.air` from the next (ov-243).
    let minHeight: CGFloat
    /// Its content's inset from the content column's edge: the text
    /// column's (`NavigatorGrid.textInset`), or 0 for a row that draws its
    /// own glyph column first, as the orchestrator's does.
    var leading: CGFloat = NavigatorGrid.textInset
    /// Its inset at the trailing edge: a task row's runs to the header's
    /// count, a rhythm in, rather than a step (ov-104).
    var trailing: CGFloat = ColumnGrid.step
    var box: String?

    /// The selection's fill: `Fill.selection`, shared with the task cards'.
    static func fill(keyed: Bool) -> Color { Fill.selection(active: keyed) }

    func body(content: Content) -> some View {
        content
            .padding(.leading, leading)
            .padding(.trailing, trailing)
            .padding(.vertical, NavigatorRhythm.air)
            .frame(minHeight: minHeight)  // rhythm-exempt: 0 in the navigator, a caller's own elsewhere
            .background {
                ZStack {
                    if selected { RoundedRectangle.control.fill(Self.fill(keyed: keyed)) }
                    if let box { Color.clear.gridMark(box, .box) }
                }
                .boxOutset()
            }
            .contentShape(Rectangle())
    }
}
