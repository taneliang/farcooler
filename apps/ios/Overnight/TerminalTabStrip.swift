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
                HStack(spacing: 4) {
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
                .padding(.horizontal, 5)
                .padding(.vertical, 5)
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
        // ONE surface, not one per chip.
        //
        // Three versions: a flat dark bar (right when everything above it was a
        // flat dark terminal, wrong under a floating composer), then a chip
        // apiece on glass — which put five or ten separate floating objects
        // below the one floating object that matters, and read as a browser tab
        // strip pasted under a chat.
        //
        // A single pill holding them is one sibling of the composer rather than
        // a crowd competing with it, and it is the shape iOS 26 gives a
        // floating group of controls.
        .modifier(GlassSurface(radius: 20))
        .padding(.horizontal, 10)
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
                // Not monospaced. The strip names TERMINALS, which are things
                // in this app, not text a terminal is showing — and monospace
                // beside a chat's body font is what made it read as a piece of
                // some other program's chrome.
                // Capped, because a chip now carries the CONVERSATION's name
                // rather than "claude 2", and an agent will happily call one
                // "Complete D17 authorization decision for Overnight" — which
                // filled the strip with a single tab and pushed every other
                // pane off the end of it.
                Text(terminal.displayName(ordinal: ordinal))
                    .font(.caption)
                    .fontWeight(wantsAttention ? .semibold : .regular)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .frame(maxWidth: 120, alignment: .leading)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
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
        // A fill, not a second pane of glass. Glass inside glass has nothing
        // new to refract and just muddies the edge that was doing the work.
        content.background(
            Capsule().fill(isCurrent ? Color.white.opacity(0.16) : Color.clear))
    }
}
