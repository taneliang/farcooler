import SwiftUI

/// Several terminals on screen at once.
///
/// The panes are laid out by absolute frame rather than by nested `HStack`s, and
/// that is not a stylistic choice: each pane holds a live `NSView` streaming
/// bytes, so a pane must keep its identity across a layout change. Rebuilding a
/// nested stack when the preset changes would tear down and re-attach every
/// terminal, wiping four screens to move a divider.
///
/// Zoom is drawn as a single pane at full size rather than as a scale transform,
/// because the pane has to actually BE that size — the emulator reflows to its
/// frame, and a scaled-up 40-column pane is just a blurry 40-column pane.
struct TileView: View {
    /// Every group in the workspace, so the bar can offer them. The one drawn is
    /// whichever is active.
    let groups: [PaneGroup]
    let workspace: Workspace
    let binary: String?
    let environment: [String: String]
    let onFocus: (String) -> Void
    let onSelectGroup: (PaneGroup) -> Void
    let onResize: (String, Int, Int) async -> Void

    @ObservedObject private var prefix = PrefixMode.shared

    private var group: PaneGroup? {
        groups.first { $0.isActive } ?? groups.first
    }

    /// The panes to draw, in member order, dropping any the fleet has not caught
    /// up with yet.
    private var panes: [Terminal] {
        (group?.terminals ?? []).compactMap { id in
            workspace.terminals.first { $0.id == id }
        }
    }

    private var zoomed: Terminal? {
        guard let id = group?.zoomed else { return nil }
        return panes.first { $0.id == id }
    }

    var body: some View {
        VStack(spacing: 0) {
            // Only when there is more than one, so a single arrangement is not
            // labelled for the benefit of a choice nobody has.
            if groups.count > 1 {
                GroupBar(groups: groups, onSelect: onSelectGroup)
            }
            panels
        }
        .paneCanvas()
        .overlay(alignment: .bottom) {
            if prefix.armed {
                PrefixHint()
                    .padding(.bottom, 14)
                    .transition(
                        .opacity.combined(with: .move(edge: .bottom))
                            .combined(with: .scale(scale: 0.96)))
            }
        }
        // The hint is a chip that appears on a keystroke, so it gets the bouncier
        // preset: it is arriving, not rearranging.
        .animation(.snappy(duration: 0.22, extraBounce: 0.08), value: prefix.armed)
        // Told from here, because this is the only place that knows a layout is
        // actually on screen. It gates the prefix-less ⌃hjkl bindings: while a
        // single terminal is showing, ⌃L has to still clear it.
        .onAppear { prefix.tiledPanes = panes.count }
        .onChange(of: panes.count) { _, count in prefix.tiledPanes = count }
        .onDisappear { prefix.tiledPanes = 0 }
        .navigationTitle(workspace.windowTitle)
        .navigationSubtitle(workspace.windowSubtitle)
    }

    /// How the panes move.
    ///
    /// The platform's own spring, not a hand-rolled curve. `.smooth` is critically
    /// damped — it settles without overshooting, which is what a terminal needs: a
    /// pane full of text that bounces past its position and comes back reads as a
    /// rendering glitch rather than as motion.
    private var motion: Animation { .smooth(duration: 0.3) }

    /// Zoom gets the tiny bit of energy `.smooth` deliberately lacks.
    ///
    /// Changing the arrangement is a change of layout; zooming is a change of
    /// posture, and a trace of overshoot is what makes it feel like the pane came
    /// forward rather than being swapped. `extraBounce` is kept small — this is one
    /// pane arriving, not a notification.
    private var zoomMotion: Animation { .snappy(duration: 0.28, extraBounce: 0.06) }

    @ViewBuilder
    private var panels: some View {
        if let group {
            GeometryReader { proxy in
                let bounds = CGRect(origin: .zero, size: proxy.size)
                let frames = TileGeometry.frames(
                    count: panes.count,
                    preset: group.layout,
                    ratio: group.share,
                    in: bounds)

            // One ForEach over every pane, always, with the zoomed one given the
            // whole area. Not `if zoomed { one } else { many }`, which was a
            // structural branch: swapping it changed each pane's view identity, so
            // zooming tore down four live NSViews and re-attached them — four
            // terminals replaying their history to move one of them forward.
            //
            // Keeping them all mounted also means un-zooming is instant and the
            // hidden panes never stop streaming.
                ZStack(alignment: .topLeading) {
                    ForEach(Array(panes.enumerated()), id: \.element.id) { index, terminal in
                        let isZoomed = group.zoomed == terminal.id
                        let tiled = frames[min(index, max(frames.count - 1, 0))]
                        let frame = isZoomed && zoomed != nil ? bounds : tiled

                        pane(terminal, group: group, frame: frame)
                            .opacity(zoomed == nil || isZoomed ? 1 : 0)
                            .zIndex(isZoomed ? 1 : 0)
                            // A pane joining or leaving grows and fades rather
                            // than appearing at full size, which at four panes is
                            // the difference between noticing which one arrived
                            // and not.
                            .transition(.scale(scale: 0.97).combined(with: .opacity))
                    }
                }
                .animation(motion, value: group.layout)
                .animation(motion, value: group.share)
                .animation(motion, value: group.terminals)
                .animation(zoomMotion, value: group.zoomed)
            }
        }
    }

    private func pane(_ terminal: Terminal, group: PaneGroup, frame: CGRect) -> some View {
        let isFocused = group.focused == terminal.id

        return TilePane(
            terminal: terminal,
            workspace: workspace,
            binary: binary,
            environment: environment,
            isFocused: isFocused,
            isZoomed: group.zoomed == terminal.id,
            index: (group.terminals.firstIndex(of: terminal.id) ?? 0) + 1,
            onResize: { columns, rows in
                await onResize(terminal.short, columns, rows)
            }
        )
        .frame(width: max(frame.width, 1), height: max(frame.height, 1))
        .offset(x: frame.minX, y: frame.minY)
        .onTapGesture { if !isFocused { onFocus(terminal.id) } }
    }
}

/// One pane, framed.
///
/// The frame is how you know which pane your keystrokes are going to, and with
/// four agents on screen that is the single most important thing the view says.
/// It is a border rather than a dimming of the others, because the others are
/// working and you are reading them.
private struct TilePane: View {
    let terminal: Terminal
    let workspace: Workspace
    let binary: String?
    let environment: [String: String]
    let isFocused: Bool
    let isZoomed: Bool
    let index: Int
    let onResize: (Int, Int) async -> Void

    @ObservedObject private var preferences = Preferences.shared

    private var isLive: Bool {
        let kind = StateKind.parse(terminal.state)
        return kind == .running || kind == .starting
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            if isLive {
                TerminalSurface(
                    terminal: terminal.short,
                    binary: binary,
                    environment: environment,
                    onResize: onResize,
                    fontRevision: preferences.revision
                )
                .id(terminal.id)
            } else {
                ZStack {
                    Color(nsColor: .textBackgroundColor)
                    StatusGlyph(status: terminal.status, size: 10)
                }
            }
        }
        .paneCard(focused: isFocused)
        // Its own, faster animation: which pane has the keyboard has to read as
        // immediate, and a border easing in over the same third of a second as
        // the panes moving would lag the keystroke that caused it.
        .animation(.smooth(duration: 0.12), value: isFocused)
    }

    /// One line, and only what a pane needs that a single terminal does not.
    ///
    /// With one terminal on screen the window title says which it is. With four,
    /// nothing does — so each pane names itself, carries the number `prefix N`
    /// selects, and shows its own status dot. That is the whole header; anything
    /// more would be four copies of chrome.
    private var header: some View {
        HStack(spacing: 6) {
            Text("\(index)")
                .font(.system(size: 9, weight: .semibold, design: .monospaced))
                .foregroundStyle(isFocused ? AnyShapeStyle(Color.accentColor) : AnyShapeStyle(.tertiary))
                .frame(minWidth: 9)

            Text(terminal.title)
                .font(.system(size: 11, weight: isFocused ? .medium : .regular))
                .foregroundStyle(isFocused ? .primary : .secondary)
                .lineLimit(1)

            Spacer(minLength: 4)

            if isZoomed {
                Image(systemName: "arrow.down.right.and.arrow.up.left")
                    .font(.system(size: 8))
                    .foregroundStyle(.tertiary)
            }
            StatusGlyph(status: terminal.status, size: 6)
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 5)
        .background(isFocused ? Color.accentColor.opacity(0.07) : Color.primary.opacity(0.03))
    }
}
