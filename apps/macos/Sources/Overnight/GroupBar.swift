import SwiftUI

/// The workspace's layouts, when it has more than one.
///
/// The feature existed before this did, and that was the problem. Several sets of
/// tiles per worktree is exactly what people want — three agents arranged and a
/// fourth left running behind them — but with nothing on screen naming the groups
/// there was no way to tell they existed, let alone that `⌃B n` switched between
/// them.
///
/// Hidden at one group, because a bar labelled "1" beside a single arrangement is
/// a control for a choice nobody has. It appears the moment there is a choice,
/// which is the moment it starts meaning something.
struct GroupBar: View {
    let groups: [PaneGroup]
    let onSelect: (PaneGroup) -> Void

    var body: some View {
        HStack(spacing: 4) {
            ForEach(Array(groups.enumerated()), id: \.element.id) { index, group in
                pill(group, position: index + 1)
            }
            Spacer(minLength: 0)
        }
        .padding(.bottom, Pane.inset)
        .animation(.smooth(duration: 0.2), value: groups.map(\.isActive))
    }

    private func pill(_ group: PaneGroup, position: Int) -> some View {
        let active = group.isActive
        return Button {
            onSelect(group)
        } label: {
            HStack(spacing: 5) {
                Text(label(group, position: position))
                    .font(.system(size: 11, weight: active ? .medium : .regular))
                Text("\(group.members.count)")
                    .font(.system(size: 10, design: .monospaced))
                    .foregroundStyle(.tertiary)
            }
            .foregroundStyle(active ? AnyShapeStyle(.primary) : AnyShapeStyle(.secondary))
            .padding(.horizontal, 9)
            .padding(.vertical, 4)
            .background(
                RoundedRectangle(cornerRadius: 5)
                    .fill(active ? Color.primary.opacity(0.09) : .clear)
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help(active ? "Showing this layout" : "Show this layout  (⌃B n)")
    }

    /// A group's name, or its number when the name IS its number.
    ///
    /// Unnamed groups are named after their position, so printing both gave "1. 1".
    private func label(_ group: PaneGroup, position: Int) -> String {
        group.name == "\(position)" ? "Layout \(position)" : group.name
    }
}
