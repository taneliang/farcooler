import AgentKit
import AppKit
import SwiftUI

/// One jump bar segment's menu (ov-192), drawn in a popover rather than an
/// `NSMenu`, which can type-select but can't filter: its sections, each item
/// with the app's own ring and a checkmark on where you are. Typing filters
/// it in place; the keys go through `JumpBarKeys`, the same as the bar's.
struct JumpMenuView: View {
    let menu: JumpMenu
    let state: JumpBarFocus
    /// A key, for the bar to route. True when it was taken.
    let onKey: (JumpKey) -> Bool
    /// An item clicked.
    let onPick: (JumpTarget) -> Void

    @FocusState private var focused: Bool

    static let minWidth: CGFloat = 220
    static let maxWidth: CGFloat = 440
    static let maxHeight: CGFloat = 440
    /// What a row spends besides its words: its padding, the checkmark's
    /// and the ring's columns, the gaps, and room for a count.
    static let rowChrome: CGFloat = 8 + 4 + 12 + 6 + StatusGlyph.inline + 6 + 6 + 8 + 28 + 8

    /// The menu's width, as an `NSMenu` sizes itself: its widest row, title
    /// and subtitle at the row's size, between `minWidth` and `maxWidth`;
    /// past that, a row's subtitle gives way, then its title's tail.
    static func width(for menu: JumpMenu) -> CGFloat {
        let font = NSFont.systemFont(ofSize: 13)
        func measure(_ text: String) -> CGFloat {
            ceil((text as NSString).size(withAttributes: [.font: font]).width)
        }
        let widest = (menu.items + menu.sections.flatMap(\.more)).map { item in
            measure(item.title) + (item.subtitle.map { measure($0) + 6 } ?? 0)
        }.max() ?? 0
        return min(max(widest + rowChrome, minWidth), maxWidth)
    }

    var body: some View {
        let shown = menu.filtered(state.query)
        VStack(alignment: .leading, spacing: 0) {
            if !state.query.isEmpty {
                HStack(spacing: 6) {
                    Image(systemName: "line.3.horizontal.decrease")
                        .foregroundStyle(.secondary)
                    Text(state.query)
                }
                .font(.system(size: ColumnHeader.textSize))
                .padding(.horizontal, 12)
                .padding(.vertical, 6)
                .accessibilityElement(children: .combine)
                .accessibilityLabel("Filter: \(state.query)")
            }
            if shown.isEmpty {
                Text(state.query.isEmpty ? "Nothing Here" : JumpMenu.noMatches(state.query))
                    .font(.system(size: ColumnHeader.textSize))
                    .foregroundStyle(.secondary)
                    .padding(12)
            } else {
                ScrollViewReader { proxy in
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: 0) {
                            ForEach(shown.sections) { section in
                                if !section.title.isEmpty {
                                Text(section.title)
                                    .font(.system(size: 11, weight: .semibold))
                                    .foregroundStyle(.secondary)
                                    .lineLimit(1)
                                    .padding(.horizontal, 12)
                                    .padding(.top, 8)
                                    .padding(.bottom, 2)
                                    .accessibilityAddTraits(.isHeader)
                                } else {
                                    Color.clear.frame(height: 6)
                                }
                                ForEach(section.items) { item in
                                    JumpMenuRow(item: item, highlighted: item.id == state.highlighted, onPick: onPick)
                                        .id(item.id)
                                }
                            }
                        }
                        .padding(.bottom, 6)
                    }
                    .onChange(of: state.highlighted) { _, id in
                        if let id { proxy.scrollTo(id) }
                    }
                    .onAppear {
                        if let id = state.highlighted { proxy.scrollTo(id, anchor: .center) }
                    }
                }
            }
        }
        .frame(width: Self.width(for: menu))
        .frame(maxHeight: Self.maxHeight)
        .fixedSize(horizontal: false, vertical: true)
        .focusable()
        .focusEffectDisabled()
        .focused($focused)
        .onAppear { focused = true }
        .onKeyPress(phases: .down) { press in
            guard let key = JumpBarKeys.key(press) else { return .ignored }
            return onKey(key) ? .handled : .ignored
        }
        .accessibilityIdentifier("jump-menu")
    }
}

/// One row of a jump bar menu.
private struct JumpMenuRow: View {
    let item: JumpItem
    let highlighted: Bool
    let onPick: (JumpTarget) -> Void
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "checkmark")
                .font(.system(size: 10, weight: .semibold))
                .opacity(item.current ? 1 : 0)
                .frame(width: 12)
                // The row's `.isSelected` trait says it; the mark would say it twice.
                .accessibilityHidden(true)
            // The ring's column, kept for a row with none (a task with no
            // agent), so every title starts at one edge.
            if let status = item.status {
                StatusGlyph(status: status)
            } else {
                Color.clear.frame(width: StatusGlyph.inline, height: StatusGlyph.inline)
                    .accessibilityHidden(true)
            }
            Text(item.title)
                .lineLimit(1)
                .truncationMode(.tail)
                .layoutPriority(1)
            if let subtitle = item.subtitle {
                // Gives way before the title does.
                Text(subtitle)
                    // style-exempt: secondary text on a menu's solid accent highlight, as NSMenu draws it.
                    .foregroundStyle(highlighted ? AnyShapeStyle(.white.opacity(0.75)) : AnyShapeStyle(.secondary))
                    .lineLimit(1)
                    .truncationMode(.tail)
            }
            Spacer(minLength: 8)
            if item.waiting > 0 {
                Text("\(item.waiting)")
                    .monospacedDigit()
                    .foregroundStyle(highlighted ? AnyShapeStyle(.white) : AnyShapeStyle(GlancePalette.amber(scheme)))
                    .accessibilityLabel(item.waiting == 1 ? "1 waiting" : "\(item.waiting) waiting")
            }
        }
        .font(.system(size: 13))
        .foregroundStyle(highlighted ? AnyShapeStyle(.white) : AnyShapeStyle(.primary))
        .padding(.horizontal, 8)
        .frame(height: 24)
        .background {
            RoundedRectangle.control
                // A menu row's highlight is the system's solid accent, as `Fill.selection` says.
                .fill(highlighted ? Color.accentColor : .clear)
                .padding(.horizontal, 4)
        }
        .contentShape(Rectangle())
        .onTapGesture { onPick(item.target) }
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(item.current ? [.isButton, .isSelected] : .isButton)
        .accessibilityAction { onPick(item.target) }
    }
}

extension JumpBarKeys {
    /// The jump bar's key for `press`, or nil for one it leaves alone.
    static func key(_ press: KeyPress) -> JumpKey? {
        key(press.key, characters: press.characters, modifiers: press.modifiers)
    }

    /// The same, from its parts: nil for any key with ⌘ or ⌃, which are the
    /// menu bar's (⌃⌘← is Back even with the bar's menu open).
    static func key(_ key: KeyEquivalent, characters: String, modifiers: EventModifiers) -> JumpKey? {
        guard modifiers.isDisjoint(with: [.command, .control]) else { return nil }
        switch key {
        case .leftArrow: return .left
        case .rightArrow: return .right
        case .upArrow: return .up
        case .downArrow: return .down
        case .return: return .return
        case .space: return .space
        case .escape: return .escape
        case .delete, .deleteForward: return .delete
        default:
            guard let c = characters.first, characters.count == 1,
                c.isLetter || c.isNumber || c.isPunctuation || c.isSymbol
            else { return nil }
            return .character(c)
        }
    }
}
