import SwiftUI

// The board's shared components (ov-92): one collapsible section, one section
// title style and one navigator row style, so every disclosure in the
// navigator looks, moves and answers the keyboard and VoiceOver the same way.
// The owner, 2 Oct: "it's very cool that the worktrees list in the board
// animates when opening and closing. why don't we do that with the other
// collapsibles as well? … make board UI components reusable so that you can
// ensure that they look and feel consistent." ov-101 moves the rest of the
// app's collapsibles onto these.

/// How a section's title is set.
enum SectionHeaderStyle: Equatable {
    /// A navigator section's: Orchestrator, Tasks, Worktrees. Small capitals,
    /// secondary, like a Mac source list's.
    case navigator
    /// A group inside one: a task status, Since Last Visit. The pane's
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

/// Every collapsible section the board draws: a header row (its chevron at
/// column A, its title at B, an accessory and its count trailing) that
/// expands and collapses on the shared spring, its content fading and sliding
/// in and out as it goes; its state kept by key or by the caller; Space, ←
/// and → on a focused header; and VoiceOver hearing a button with its state.
///
/// Every one registers its `id` (`CollapsibleSectionsKey`), so a test can say
/// which sections were drawn through it.
struct CollapsibleSection<Label: View, Accessory: View, Content: View>: View {
    /// Which section, for the registry, the grid's marks and the
    /// accessibility identifier.
    let id: String
    var style: SectionHeaderStyle = .group
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

    /// The caller's state, when it keeps it.
    private var binding: Binding<Bool>?
    /// Kept here, by `key` in `defaults`, otherwise.
    private let key: String?
    private let defaults: UserDefaults
    @State private var stored: Bool

    /// A section whose open state the caller keeps.
    init(
        id: String, style: SectionHeaderStyle = .group, isExpanded: Binding<Bool>, canExpand: Bool = true,
        count: Int? = nil, accessibilityLabel: String, fillsRow: Bool = true,
        @ViewBuilder label: @escaping (Bool) -> Label, @ViewBuilder accessory: @escaping () -> Accessory,
        @ViewBuilder content: @escaping () -> Content
    ) {
        self.id = id
        self.style = style
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
        id: String, style: SectionHeaderStyle = .group, key: String, defaults: UserDefaults = .standard,
        expandedByDefault: Bool = true, canExpand: Bool = true, count: Int? = nil, accessibilityLabel: String,
        fillsRow: Bool = true,
        @ViewBuilder label: @escaping (Bool) -> Label, @ViewBuilder accessory: @escaping () -> Accessory,
        @ViewBuilder content: @escaping () -> Content
    ) {
        self.id = id
        self.style = style
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

    private var expanded: Bool { canExpand && (binding?.wrappedValue ?? stored) }

    /// Open or close it, on the shared spring.
    private func set(_ open: Bool) {
        guard canExpand, open != expanded else { return }
        withAnimation(WorkspaceMotion.spring) {
            if let binding { binding.wrappedValue = open } else { stored = open }
        }
        if let key { defaults.set(open, forKey: key) }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: ColumnGrid.rhythm) {
            header
            if expanded {
                content()
                    .transition(
                        .asymmetric(
                            insertion: .opacity.combined(with: .offset(y: -ColumnGrid.rhythm)),
                            removal: .opacity))
            }
        }
        // Added to, not set: a section inside another one registers too.
        .transformPreference(CollapsibleSectionsKey.self) { $0.insert(id) }
    }

    private var header: some View {
        HStack(spacing: 0) {
            Button { set(!expanded) } label: {
                HStack(spacing: 0) {
                    Image(systemName: "chevron.right")
                        .font(.system(size: 9, weight: .bold))
                        .rotationEffect(.degrees(expanded ? 90 : 0))
                        .foregroundStyle(.secondary)
                        .opacity(canExpand ? 1 : 0.35)
                        .frame(width: ColumnGrid.step, alignment: .leading)
                        .gridMark(Self.gridRow(id), .chevron)
                    label(expanded)
                    if fillsRow { Spacer(minLength: SidebarGrid.gap) }
                }
                .frame(minHeight: ColumnGrid.rowHeight)
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
            .accessibilityAddTraits([.isHeader, .isButton])
            .accessibilityIdentifier("section-\(id)")
            accessory()
            if !fillsRow { Spacer(minLength: 0) }
            if let count {
                Text("\(count)")
                    .font(.system(size: WorkspaceStyle.PaneText.secondary))
                    .monospacedDigit()
                    .foregroundStyle(.secondary)
                    .accessibilityHidden(true)
            }
        }
    }
}

extension CollapsibleSection where Label == SectionTitle {
    /// A section titled in its style, keeping its own state by `key`.
    init(
        _ title: String, id: String, style: SectionHeaderStyle = .group, tone: SectionTitle.Tone = .primary,
        key: String, defaults: UserDefaults = .standard, expandedByDefault: Bool = true, canExpand: Bool = true,
        count: Int? = nil,
        @ViewBuilder accessory: @escaping () -> Accessory, @ViewBuilder content: @escaping () -> Content
    ) {
        self.init(
            id: id, style: style, key: key, defaults: defaults, expandedByDefault: expandedByDefault,
            canExpand: canExpand, count: count, accessibilityLabel: title,
            label: { _ in SectionTitle(text: title, style: style, tone: tone, gridRow: Self.gridRow(id)) }, accessory: accessory,
            content: content)
    }

    /// A section titled in its style, whose state the caller keeps.
    init(
        _ title: String, id: String, style: SectionHeaderStyle = .group, tone: SectionTitle.Tone = .primary,
        isExpanded: Binding<Bool>, canExpand: Bool = true, count: Int? = nil,
        @ViewBuilder accessory: @escaping () -> Accessory, @ViewBuilder content: @escaping () -> Content
    ) {
        self.init(
            id: id, style: style, isExpanded: isExpanded, canExpand: canExpand, count: count,
            accessibilityLabel: title, label: { _ in SectionTitle(text: title, style: style, tone: tone, gridRow: Self.gridRow(id)) },
            accessory: accessory, content: content)
    }
}

extension CollapsibleSection where Label == SectionTitle, Accessory == EmptyView {
    init(
        _ title: String, id: String, style: SectionHeaderStyle = .group, tone: SectionTitle.Tone = .primary,
        key: String, defaults: UserDefaults = .standard, expandedByDefault: Bool = true, canExpand: Bool = true,
        count: Int? = nil, @ViewBuilder content: @escaping () -> Content
    ) {
        self.init(
            title, id: id, style: style, tone: tone, key: key, defaults: defaults,
            expandedByDefault: expandedByDefault, canExpand: canExpand, count: count, accessory: { EmptyView() },
            content: content)
    }

    init(
        _ title: String, id: String, style: SectionHeaderStyle = .group, tone: SectionTitle.Tone = .primary,
        isExpanded: Binding<Bool>, canExpand: Bool = true, count: Int? = nil,
        @ViewBuilder content: @escaping () -> Content
    ) {
        self.init(
            title, id: id, style: style, tone: tone, isExpanded: isExpanded, canExpand: canExpand, count: count,
            accessory: { EmptyView() }, content: content)
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
    /// accent while the navigator has the keyboard, else gray.
    func navigatorRow(selected: Bool, keyed: Bool, minHeight: CGFloat = ColumnGrid.twoLineRowHeight) -> some View {
        modifier(NavigatorRowStyle(selected: selected, keyed: keyed, minHeight: minHeight))
    }
}

struct NavigatorRowStyle: ViewModifier {
    let selected: Bool
    let keyed: Bool
    let minHeight: CGFloat

    /// The selection's fill, shared with the task cards'.
    static func fill(keyed: Bool) -> Color {
        keyed ? Color.accentColor.opacity(0.18) : Color.primary.opacity(0.08)
    }

    func body(content: Content) -> some View {
        content
            .padding(.horizontal, ColumnGrid.step)
            .padding(.vertical, ColumnGrid.rhythm / 2)
            .frame(minHeight: minHeight)
            .background {
                if selected { RoundedRectangle(cornerRadius: 8).fill(Self.fill(keyed: keyed)) }
            }
            .contentShape(Rectangle())
    }
}
