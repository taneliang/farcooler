import SwiftUI

/// Every terminal in the fleet, one tap from whichever one is on screen.
///
/// Ported from nothing — the Mac always has its sidebar on screen, so
/// switching terminals there is a click away regardless of which one is
/// open. A phone's terminal screen is full-bleed with no sidebar to fall
/// back to, and "go back to the list, find the row, tap it" is the wrong
/// cost for something as routine as glancing at a second agent. This strip
/// makes every terminal one tap away without ever leaving the screen that
/// made checking on it worthwhile.
///
/// Deliberately flat across the whole fleet rather than scoped to the current
/// workspace: the 3am case this exists for is "is the OTHER agent still
/// blocked", which is exactly as likely to be in a different worktree as the
/// same one.
struct TerminalTabStrip: View {
    let workspaces: [Workspace]
    let current: Terminal
    let onSelect: (Terminal) -> Void

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 6) {
                    ForEach(workspaces) { workspace in
                        let numbering = workspace.ordinals()
                        ForEach(workspace.terminals) { terminal in
                            TabChip(
                                terminal: terminal,
                                ordinal: numbering[terminal.id],
                                isCurrent: terminal.id == current.id,
                                onTap: { onSelect(terminal) }
                            )
                            .id(terminal.id)
                        }
                    }
                }
                .padding(.horizontal, 8)
                .padding(.vertical, 6)
            }
            // Scrolled to the open terminal on first layout, not on every
            // fleet refresh: `.onAppear` fires once, `onChange(of: current)`
            // is what re-centers on a genuine tap (below) — a poll landing
            // mid-scroll must not fight the user's own gesture.
            .onAppear { proxy.scrollTo(current.id, anchor: .center) }
            .onChange(of: current.id) { _, id in
                withAnimation(.easeOut(duration: 0.2)) { proxy.scrollTo(id, anchor: .center) }
            }
        }
        // No slab behind it.
        //
        // This was a flat dark bar, which was right when everything above it
        // was a flat dark terminal. It is wrong under a floating glass composer:
        // a card resting on a slab reads as two competing surfaces, and the
        // bottom of the screen ends up heavier than anything else on it. The
        // chips carry their own glass now and the strip itself is nothing —
        // content passes behind it, which is what makes the glass mean
        // something.
    }
}

/// One tab. Compact by necessity — this is a phone, and the strip has to
/// hold a whole fleet's worth of these on one line.
private struct TabChip: View {
    let terminal: Terminal
    let ordinal: Int?
    let isCurrent: Bool
    let onTap: () -> Void

    private var kind: StateKind { StateKind.parse(terminal.state) }
    private var wantsAttention: Bool { terminal.agent.wantsAttention }

    var body: some View {
        Button(action: onTap) {
            HStack(spacing: 5) {
                // Same dot, same colours as the fleet list — `processColour`
                // is shared rather than redefined here so a terminal cannot
                // read green in one screen and red in the other.
                Circle().fill(processColour(kind)).frame(width: 6, height: 6)
                Text(terminal.displayName(ordinal: ordinal))
                    .font(.system(.caption, design: .monospaced))
                    .fontWeight(wantsAttention ? .semibold : .regular)
                    .lineLimit(1)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 7)
            .modifier(ChipGlass(isCurrent: isCurrent))
            .overlay(
                Capsule()
                    .strokeBorder(
                        wantsAttention ? attentionColour(terminal.agent) : .clear, lineWidth: 1.5)
            )
            .foregroundStyle(isCurrent ? Color.white : Color.white.opacity(0.75))
        }
        .buttonStyle(.plain)
    }
}


/// A chip's surface: glass on iOS 26, the nearest material before it.
///
/// A capsule rather than a rounded rectangle, because that is the shape the
/// platform gives a floating, tappable pill — and because a rounded rectangle
/// next to the composer's rounded rectangle read as a smaller version of the
/// same thing rather than as a different kind of control.
private struct ChipGlass: ViewModifier {
    let isCurrent: Bool

    func body(content: Content) -> some View {
        if #available(iOS 26.0, *) {
            content.glassEffect(isCurrent ? .regular.tint(.white.opacity(0.18)) : .regular, in: .capsule)
        } else {
            content.background(
                Capsule().fill(isCurrent ? Color.white.opacity(0.18) : Color.white.opacity(0.07)))
        }
    }
}
