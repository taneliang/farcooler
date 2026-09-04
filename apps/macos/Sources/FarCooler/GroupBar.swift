import SwiftUI

/// The workspace's layouts, when it has more than one.
///
/// The feature existed before this did, and that was the problem. Several sets of
/// tiles per worktree is exactly what people want — three agents arranged and a
/// fourth left running behind them — but with nothing on screen naming the groups
/// there was no way to tell they existed, let alone that `⌃B n` switched between
/// them.
///
/// Hidden at one group, because a bar labeled "1" beside a single arrangement is
/// a control for a choice nobody has. It appears the moment there is a choice,
/// which is the moment it starts meaning something.
struct GroupBar: View {
    let groups: [PaneGroup]
    /// Which pill is lit, by tmux window id.
    ///
    /// The layout on screen, not the one the runner calls active. Those are the
    /// same a round trip after you pick one and different until then, and a bar
    /// that lit the runner's answer would spend that round trip pointing at a
    /// layout you are not looking at. See `TileView.showing`.
    let showing: String
    let onSelect: (PaneGroup) -> Void

    var body: some View {
        // Scrolls, because a worktree now has as many layouts as it has terminals.
        // A layout used to be something you made on purpose and there were two or
        // three; a terminal IS a tmux window and a window IS a layout, so twelve
        // shells means twelve pills. In a fixed row SwiftUI compressed them until
        // the names ran vertically, one letter per line, which is the state this
        // was found in — a bar naming twelve layouts and legible for none of them.
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 4) {
                ForEach(Array(groups.enumerated()), id: \.element.id) { index, group in
                    pill(group, position: index + 1, lit: group.id == showing)
                }
            }
        }
        // The row is one line of text tall whatever happens. Without this the
        // scroll view claims whatever height it is offered and the panes lose it.
        .frame(height: 26)
        .padding(.bottom, Pane.inset)
        // Spring rather than a fixed curve: see `TileView.motion`. A tab
        // strip that takes the same 200ms whether one tab changed or five
        // reads as lag, not as motion.
        .animation(Motion.snap, value: showing)
    }

    private func pill(_ group: PaneGroup, position: Int, lit: Bool) -> some View {
        let active = lit
        return Button {
            onSelect(group)
        } label: {
            HStack(spacing: 5) {
                Text(label(group, position: position))
                    .font(.system(size: 11.5, weight: active ? .medium : .regular))
                    .lineLimit(1)
                // Only where it says something. Every layout has at least one pane,
                // so "1" beside eleven of twelve pills is a column of noise hiding
                // the one that reads "3".
                if group.panes.count > 1 {
                    Text("\(group.panes.count)")
                        .font(.system(size: 10, design: .monospaced))
                        .foregroundStyle(.tertiary)
                }
            }
            .fixedSize()
            .foregroundStyle(active ? AnyShapeStyle(.primary) : AnyShapeStyle(.secondary))
            .padding(.horizontal, 9)
            .padding(.vertical, 4)
            .background(
                RoundedRectangle(cornerRadius: 5)
                    .fill(active ? Color.accentColor.opacity(0.13) : .clear)
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help(active ? "Showing this layout" : "Show this layout  (⌃B n)")
    }

    /// What a layout is called: what is in it.
    ///
    /// Never a name, because a layout has no identity apart from its panes and
    /// naming one would be asking a question at the moment someone least wants
    /// to answer it. tmux does name its windows — after whatever is running,
    /// renamed as that changes — and showing that gave a row of tabs reading
    /// "zsh", "zsh", "shell", "agent", which is worse than nothing: it looks like
    /// a name, so you read it as one, and it means nothing.
    ///
    /// So: the distinct commands inside, most common first. A layout of three
    /// shells and a claude reads `claude · zsh`, and the count beside it says how
    /// many panes there are. Two layouts that genuinely hold the same things read
    /// the same, and the position distinguishes them — which is honest, because
    /// at that point they ARE the same except for where they sit.
    private func label(_ group: PaneGroup, position: Int) -> String {
        var order: [String] = []
        var counts: [String: Int] = [:]
        for pane in group.panes {
            let name = Terminal.name(of: pane.title ?? "")
            if counts[name] == nil { order.append(name) }
            counts[name, default: 0] += 1
        }
        guard !order.isEmpty else { return "\(position)" }

        let ranked = order.sorted { (counts[$0] ?? 0, order.firstIndex(of: $1) ?? 0)
            > (counts[$1] ?? 0, order.firstIndex(of: $0) ?? 0) }
        // Two is as many as fits before the pill starts eating the tab bar; the
        // count already says there are more.
        return ranked.prefix(2).joined(separator: " · ")
    }
}
