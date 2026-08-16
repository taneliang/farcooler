import AppKit
import SwiftUI

/// ⌘P — one field for going anywhere and starting anything.
///
/// The app already had two ways to reach something: the sidebar's filter, and
/// ⌃⌘N for whatever is blocked. Both answer a question you have already
/// finished forming. The one missing is what every switcher exists for — you
/// know which terminal you want but not what it is called or where it is among
/// seventeen, and the only description you actually hold of it is what is on
/// its screen.
///
/// So: two modes, one field, and which one you get is decided by whether you
/// have typed anything rather than by a control you have to find.
///
///   empty      a recency-ordered grid of live screens. You recognize the
///              terminal you want instead of recalling its name, which is the
///              whole reason Alt-Tab shows pictures.
///   anything   a flat list of everything that word could mean — terminals,
///              worktrees, and the two things you might want to CREATE.
///
/// The mixing in the second mode is the point, not a shortcut. "auth" is as
/// likely to mean "start something called auth" as "go to auth", and splitting
/// those into separate panels asks the user to classify their own intent before
/// they have finished having it. Ranking, not routing.
struct CommandPalette: View {
    let workspaces: [Workspace]
    /// What the window is looking at, used for two small things: which tile the
    /// highlight avoids starting on, and where a new terminal goes when the
    /// query matched no worktree.
    let current: ContentView.Selection?
    /// A terminal's rendered screen. Passed as a function rather than as a
    /// `DaemonClient`, so the panel has no way to write anything — it can look
    /// and it can report what was chosen, and nothing else.
    let screen: (String) async -> String
    let onRun: (PaletteAction) -> Void
    let onClose: () -> Void

    @State private var query = ""
    @State private var highlight = 0
    @StateObject private var previews = ScreenPreviews()

    /// Three across, and the tile width follows from it rather than the other
    /// way round. Four would put the preview below the width where a wrapped
    /// agent transcript is still recognizable, which is the only thing the
    /// picture is for.
    private static let columns = 3
    private static let panelWidth: CGFloat = 720
    private static let gutter: CGFloat = 14
    private static let gap: CGFloat = 10
    /// Enough to recognize a screen by, not enough to read it. Eight lines
    /// catches a prompt and the question above it, which is what a tile is for.
    fileprivate static let previewLines = 8

    private static var tileWidth: CGFloat {
        (panelWidth - gutter * 2 - gap * CGFloat(columns - 1)) / CGFloat(columns)
    }

    /// Three rows of tiles, and a sliver of the fourth.
    ///
    /// Derived rather than picked, because a tile's height moves with the
    /// terminal font. A row cut exactly in half looks like a clipping bug; a row
    /// cut at its top edge reads as what it is, which is that there is more
    /// below — and there has to be an edge showing at all, or a grid that
    /// scrolls looks like a grid that ends.
    private static var gridHeight: CGFloat {
        SwitcherTile.height * 3 + gap * 2 + gutter + 12
    }

    private var trimmed: String { query.trimmingCharacters(in: .whitespacesAndNewlines) }
    private var isSwitcher: Bool { trimmed.isEmpty }

    private var entries: [PaletteEntry] {
        isSwitcher
            ? PaletteIndex.recent(in: workspaces)
            : PaletteIndex.matching(
                query, in: workspaces, current: currentWorkspace,
                currentTerminal: selectedTerminalRecord)
    }

    /// The `Terminal` record behind `currentTerminal`, so the palette can
    /// offer to toggle ITS mode rather than only knowing its id.
    private var selectedTerminalRecord: Terminal? {
        guard let currentTerminal else { return nil }
        return workspaces.lazy.flatMap(\.terminals).first { $0.id == currentTerminal }
    }

    private var currentWorkspace: String? {
        switch current {
        case .workspace(_, let id): return id
        case .terminal(_, let workspace, _): return workspace
        case nil: return nil
        }
    }

    private var currentTerminal: String? {
        if case .terminal(_, _, let id) = current { return id }
        return nil
    }

    /// The terminals the panel is currently showing a picture of.
    ///
    /// Empty in list mode, which is what stops the refresh loop dead: typing is
    /// not a reason to keep a dozen subprocesses running.
    private var previewTargets: [String] {
        isSwitcher ? entries.compactMap { $0.terminal?.short } : []
    }

    var body: some View {
        VStack(spacing: 0) {
            field
            Divider()
            results
            Divider()
            footer
        }
        .frame(width: Self.panelWidth)
        .background(.regularMaterial)
        .clipShape(RoundedRectangle(cornerRadius: 10))
        .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(Color.primary.opacity(0.08)))
        .shadow(color: .black.opacity(0.22), radius: 26, y: 10)
        .onAppear { highlight = openingHighlight }
        .onChange(of: trimmed) { _, _ in highlight = openingHighlight }
        // Keyed on the SET of terminals, not their order, so a tile changing
        // activity — which reorders the grid every few seconds on a busy fleet —
        // does not cancel and restart a sweep that is halfway through.
        .task(id: previewTargets.sorted()) {
            let targets = previewTargets
            guard !targets.isEmpty else { return }
            while !Task.isCancelled {
                await previews.refresh(targets, lines: Self.previewLines, using: screen)
                // Slow on purpose. These are pictures for recognizing a terminal
                // by, not a second place to watch one work — the pane behind the
                // panel is already that. Two seconds is under the time it takes
                // to read a grid of twelve, so nothing is ever visibly stale
                // when you look at it.
                try? await Task.sleep(for: .seconds(2))
            }
        }
    }

    /// Where the highlight starts.
    ///
    /// On the second tile when the first is the terminal you are already in.
    /// That is Alt-Tab's rule and the reason Alt-Tab is one keystroke rather
    /// than two: the overwhelmingly common ask is "the other one", and a
    /// switcher that opens pointing at where you already are makes you move
    /// before it can do anything for you.
    private var openingHighlight: Int {
        guard isSwitcher, let currentTerminal, entries.count > 1 else { return 0 }
        if case .openTerminal(_, let terminal) = entries[0].action, terminal == currentTerminal {
            return 1
        }
        return 0
    }

    // MARK: - Field

    private var field: some View {
        HStack(spacing: 9) {
            Image(systemName: "magnifyingglass")
                .font(.system(size: 12))
                .foregroundStyle(.tertiary)

            PaletteField(
                text: $query,
                placeholder: "Go to a terminal, a worktree, or start something",
                horizontalMoves: isSwitcher,
                onMove: move,
                onSubmit: submit,
                onCancel: onClose
            )
            .frame(height: 21)
        }
        .padding(.horizontal, Self.gutter)
        .padding(.vertical, 11)
    }

    // MARK: - Results

    @ViewBuilder
    private var results: some View {
        if entries.isEmpty {
            empty
        } else {
            ScrollViewReader { proxy in
                ScrollView {
                    if isSwitcher { grid } else { list }
                }
                .frame(maxHeight: isSwitcher ? Self.gridHeight : 340)
                .onChange(of: highlight) { _, index in
                    guard entries.indices.contains(index) else { return }
                    withAnimation(.snappy(duration: 0.12)) {
                        proxy.scrollTo(entries[index].id, anchor: .center)
                    }
                }
            }
        }
    }

    private var grid: some View {
        LazyVGrid(
            columns: Array(
                repeating: GridItem(.fixed(Self.tileWidth), spacing: Self.gap),
                count: Self.columns),
            spacing: Self.gap
        ) {
            ForEach(Array(entries.enumerated()), id: \.element.id) { index, entry in
                SwitcherTile(
                    entry: entry,
                    lines: entry.terminal.map { previews.tails[$0.short] ?? [] } ?? [],
                    width: Self.tileWidth,
                    isHighlighted: index == highlight
                )
                .id(entry.id)
                .onTapGesture { onRun(entry.action) }
            }
        }
        .padding(Self.gutter)
    }

    private var list: some View {
        LazyVStack(spacing: 1) {
            ForEach(Array(entries.enumerated()), id: \.element.id) { index, entry in
                PaletteRow(entry: entry, isHighlighted: index == highlight)
                    .id(entry.id)
                    .onTapGesture { onRun(entry.action) }
            }
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 7)
    }

    private var empty: some View {
        VStack(spacing: 5) {
            Text(isSwitcher ? "Nothing running" : "Nothing matches")
                .font(.callout.weight(.medium))
            Text(isSwitcher ? "Type to start something" : "“\(trimmed)”")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 36)
    }

    // MARK: - Footer

    private var footer: some View {
        HStack(spacing: 8) {
            if isSwitcher {
                let hidden = workspaces.reduce(0) { $0 + $1.terminals.count } - entries.count
                // Said out loud, because a grid that silently stops at twelve
                // looks like a fleet that stops at twelve.
                Text(
                    hidden > 0
                        ? "\(entries.count) most recent · type to reach the other \(hidden)"
                        : "Most recent first")
            } else {
                Text("\(entries.count) result\(entries.count == 1 ? "" : "s")")
            }

            Spacer(minLength: 10)
            Text(isSwitcher ? "↑↓←→ move" : "↑↓ move")
            Text("↩ open")
            Text("esc close")
        }
        .font(.system(size: 11))
        .foregroundStyle(.tertiary)
        .padding(.horizontal, Self.gutter)
        .padding(.vertical, 7)
    }

    // MARK: - Keyboard

    private func move(_ direction: PaletteMove) {
        let count = entries.count
        guard count > 0 else { return }
        let stride = isSwitcher ? Self.columns : 1

        var next = highlight
        switch direction {
        case .next, .right: next += 1
        case .previous, .left: next -= 1
        case .up: next -= stride
        case .down: next += stride
        }

        // Wraps, for the same reason ⌘] does: a list you can walk off the end of
        // is one you have to look down at to find out where you are.
        highlight = ((next % count) + count) % count
    }

    private func submit() {
        guard entries.indices.contains(highlight) else { return }
        onRun(entries[highlight].action)
    }
}

// MARK: - Tiles and rows

/// One terminal, as a picture of itself.
///
/// The header is deliberately three facts and no more — what it is called,
/// where it lives, and whether it wants you. Everything else a terminal could
/// say about itself is on the screen underneath, which is the part worth the
/// space.
private struct SwitcherTile: View {
    let entry: PaletteEntry
    let lines: [String]
    let width: CGFloat
    let isHighlighted: Bool

    private var status: Status { entry.terminal?.status ?? .running }
    private var wantsAttention: Bool { status.wantsAttention }

    /// Room for exactly the lines asked for, held constant whatever the
    /// terminal has to show. A tile that grew and shrank with its content would
    /// reflow the whole grid every two seconds.
    fileprivate static var previewHeight: CGFloat {
        ScreenPreviewText.height(lines: CommandPalette.previewLines) + 8
    }

    /// What one tile occupies, for sizing the scroller around the grid.
    ///
    /// The two text lines are approximated from their point sizes rather than
    /// measured. Being a point or two out only changes how much of the fourth
    /// row shows, and paying a layout pass to get that exactly right would be
    /// precision spent where nothing depends on it.
    fileprivate static var height: CGFloat {
        16 + 15 + 13 + 10 + previewHeight
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            HStack(spacing: 5) {
                StatusGlyph(status: status, size: 7)
                Text(entry.title)
                    .font(.system(size: 12, weight: .medium))
                    .lineLimit(1)
                Spacer(minLength: 4)
                if let duration = entry.terminal?.displayDuration {
                    Text(duration)
                        .font(.system(size: 10))
                        .foregroundStyle(.tertiary)
                        .monospacedDigit()
                }
            }

            Text(entry.detail)
                .font(.system(size: 10))
                .foregroundStyle(.secondary)
                .lineLimit(1)

            preview
        }
        .padding(8)
        .frame(width: width)
        .background(
            RoundedRectangle(cornerRadius: 8)
                .fill(fill)
        )
        .overlay(
            RoundedRectangle(cornerRadius: 8)
                .strokeBorder(border, lineWidth: isHighlighted ? 2 : 1)
        )
        .contentShape(RoundedRectangle(cornerRadius: 8))
    }

    private var preview: some View {
        ScreenPreviewText(lines: lines, width: width - 16 - 12)
            .padding(.horizontal, 6)
            .padding(.vertical, 4)
            // Bottom-aligned, because a terminal's present tense is at the
            // bottom of it — a half-full screen should sit on the floor of the
            // tile the way it sits on the floor of a pane, not float at the top.
            .frame(
                maxWidth: .infinity, minHeight: Self.previewHeight,
                maxHeight: Self.previewHeight, alignment: .bottomLeading
            )
            .background(Color(nsColor: Palette.background))
            .clipShape(RoundedRectangle(cornerRadius: 5))
    }

    /// The highlight is the accent ring a focused pane already wears — this
    /// panel exists to move that focus, so it should show what it is about to
    /// do in the language the window already uses. Attention keeps the orange
    /// it has everywhere else, and the two never collide: one is a ring, one is
    /// a wash.
    private var border: Color {
        if isHighlighted { return .accentColor }
        if wantsAttention { return .orange.opacity(0.55) }
        return Color.primary.opacity(0.08)
    }

    private var fill: Color {
        wantsAttention ? .orange.opacity(0.07) : Color.primary.opacity(0.04)
    }
}

/// One search result.
private struct PaletteRow: View {
    let entry: PaletteEntry
    let isHighlighted: Bool

    var body: some View {
        HStack(spacing: 9) {
            Group {
                if let terminal = entry.terminal {
                    StatusGlyph(status: terminal.status, size: 8)
                } else if let symbol = entry.symbol {
                    Image(systemName: symbol)
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                }
            }
            .frame(width: 14)

            VStack(alignment: .leading, spacing: 1) {
                Text(entry.title).font(.system(size: 13)).lineLimit(1)
                if !entry.detail.isEmpty {
                    Text(entry.detail)
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }

            Spacer(minLength: 8)

            Text(entry.kind)
                .font(.system(size: 10))
                .foregroundStyle(.tertiary)
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 5)
        .background(
            RoundedRectangle(cornerRadius: 6)
                .fill(isHighlighted ? Color.primary.opacity(0.09) : .clear)
        )
        .overlay(
            RoundedRectangle(cornerRadius: 6)
                .strokeBorder(isHighlighted ? Color.accentColor : .clear, lineWidth: 1)
        )
        .contentShape(Rectangle())
    }
}
