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
    let group: PaneGroup
    let workspace: Workspace
    let binary: String?
    let environment: [String: String]
    let onFocus: (String) -> Void
    let onResize: (String, Int, Int) async -> Void

    @ObservedObject private var prefix = PrefixMode.shared

    /// The panes to draw, in member order, dropping any the fleet has not caught
    /// up with yet.
    private var panes: [Terminal] {
        group.terminals.compactMap { id in
            workspace.terminals.first { $0.id == id }
        }
    }

    private var zoomed: Terminal? {
        guard let id = group.zoomed else { return nil }
        return panes.first { $0.id == id }
    }

    var body: some View {
        GeometryReader { proxy in
            let bounds = CGRect(origin: .zero, size: proxy.size)
            ZStack(alignment: .topLeading) {
                if let zoomed {
                    pane(zoomed, frame: bounds)
                } else {
                    let frames = TileGeometry.frames(
                        count: panes.count,
                        preset: group.layout,
                        ratio: group.share,
                        in: bounds)
                    ForEach(Array(panes.enumerated()), id: \.element.id) { index, terminal in
                        if index < frames.count {
                            pane(terminal, frame: frames[index])
                        }
                    }
                }
            }
            // Snappy, not springy. A layout change is a change of view, and a
            // terminal that overshoots and settles reads as a glitch.
            .animation(.easeOut(duration: 0.14), value: group.layout)
            .animation(.easeOut(duration: 0.14), value: group.zoomed)
            .animation(.easeOut(duration: 0.14), value: group.terminals)
        }
        .padding(6)
        .background(Color(nsColor: .underPageBackgroundColor))
        .overlay(alignment: .bottom) {
            if prefix.armed {
                PrefixHint()
                    .padding(.bottom, 14)
                    .transition(.opacity.combined(with: .move(edge: .bottom)))
            }
        }
        .animation(.easeOut(duration: 0.12), value: prefix.armed)
        // Told from here, because this is the only place that knows a layout is
        // actually on screen. It gates the prefix-less ⌃hjkl bindings: while a
        // single terminal is showing, ⌃L has to still clear it.
        .onAppear { prefix.tiledPanes = panes.count }
        .onChange(of: panes.count) { _, count in prefix.tiledPanes = count }
        .onDisappear { prefix.tiledPanes = 0 }
        .navigationTitle(workspace.windowTitle)
        .navigationSubtitle(workspace.windowSubtitle)
    }

    private func pane(_ terminal: Terminal, frame: CGRect) -> some View {
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
        .clipShape(RoundedRectangle(cornerRadius: 7))
        .overlay(
            RoundedRectangle(cornerRadius: 7)
                .strokeBorder(
                    isFocused ? Color.accentColor : Color.primary.opacity(0.10),
                    lineWidth: isFocused ? 2 : 1)
        )
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
