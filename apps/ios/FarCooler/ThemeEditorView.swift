import SwiftUI

/// Nineteen colours, over a terminal actually rendering them.
///
/// All nineteen on a phone, which was a deliberate choice rather than the easy
/// one: the config format treats the sixteen ANSI colours as optional, so a
/// "grounds only" editor would have been a first-class config and less work. But
/// the runner holding the file is often on a Linux box with no app on it, and a
/// phone that could only edit three colours would send you back to ssh for the
/// other sixteen.
///
/// The preview earns more room here than on the Mac, because the grid is small
/// and a swatch is smaller: nineteen hex values cannot tell you whether you can
/// read this for eight hours, and looking at it can.
struct ThemeEditorView: View {
    @State private var draft: Theme
    private let original: Theme
    private let onSave: (Theme) -> Void
    @Environment(\.dismiss) private var dismiss

    init(theme: Theme, onSave: @escaping (Theme) -> Void) {
        _draft = State(initialValue: theme)
        self.original = theme
        self.onSave = onSave
    }

    var body: some View {
        Form {
            Section {
                ThemePreviewCanvas(theme: draft)
                    .frame(height: 150)
                    .listRowInsets(EdgeInsets())
            }

            Section("Name") {
                TextField("Name", text: $draft.name)
                    .autocorrectionDisabled()
                Toggle("Dark surfaces around the terminal", isOn: $draft.dark)
            }

            Section("Ground") {
                well("Background", $draft.background)
                well("Text", $draft.foreground)
                well("Cursor", $draft.cursor)
            }

            Section("Normal") {
                ForEach(0..<8, id: \.self) { index in
                    ansiWell(index, Self.normalNames[index])
                }
            }

            Section {
                ForEach(8..<16, id: \.self) { index in
                    ansiWell(index, "Bright " + Self.normalNames[index - 8])
                }
            } header: {
                Text("Bright")
            } footer: {
                Text(
                    "Colors above these sixteen are arithmetic every terminal agrees on, so no "
                    + "theme sets them.")
            }
        }
        .navigationTitle(original.name.isEmpty ? "New theme" : original.name)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .cancellationAction) {
                Button("Cancel") { dismiss() }
            }
            ToolbarItem(placement: .confirmationAction) {
                Button("Save") {
                    onSave(draft)
                    dismiss()
                }
                .disabled(draft.name.trimmingCharacters(in: .whitespaces).isEmpty)
            }
        }
    }

    private func well(_ label: String, _ packed: Binding<UInt32>) -> some View {
        ColorPicker(
            label,
            selection: Binding(
                get: { Color(packed: packed.wrappedValue) },
                set: { packed.wrappedValue = $0.packed }
            ),
            supportsOpacity: false)
    }

    private func ansiWell(_ index: Int, _ label: String) -> some View {
        ColorPicker(
            label,
            selection: Binding(
                get: { Color(packed: draft.ansi[index]) },
                set: { draft.ansi[index] = $0.packed }
            ),
            supportsOpacity: false)
    }

    /// ANSI's own names, because they are what an escape sequence means and what
    /// every other terminal's settings call them.
    static let normalNames = [
        "Black", "Red", "Green", "Yellow", "Blue", "Magenta", "Cyan", "White",
    ]
}

/// A terminal rendering a fixture in the theme being edited.
///
/// Draws through the same `VTCore` a live pane uses, fed the same kind of bytes.
/// Cell colours are resolved inside that core precisely so three renderers cannot
/// drift; a hand-drawn preview here would be a fourth drifting from all of them.
private struct ThemePreviewCanvas: View {
    let theme: Theme

    var body: some View {
        GeometryReader { geo in
            Canvas { context, size in
                let grid = Self.grid(theme: theme, columns: Self.columns)
                draw(grid, in: &context, size: size)
            }
            .background(Color(packed: theme.background))
            .frame(width: geo.size.width, height: geo.size.height)
        }
    }

    private static let columns = 52
    private static let rows = 9

    /// A grid rendered by a throwaway core, in the palette being edited.
    ///
    /// Rebuilt on every change rather than recoloured in place: the fixture is a
    /// few hundred bytes, and a fresh core cannot carry state from a palette that
    /// is no longer chosen.
    private static func grid(theme: Theme, columns: Int) -> TerminalGrid? {
        let core = VTCore(columns: columns, rows: rows)
        core.setPalette(theme.packed)
        core.feed(Array(fixture.utf8))
        return core.withSnapshot { TerminalGrid(snapshot: $0) }
    }

    /// Output chosen to exercise what a theme has to get right: every colour,
    /// bold, a prompt, a diff, and an agent waiting on you.
    private static let fixture: String = {
        var out = "\u{1b}[H\u{1b}[2J"
        out += "\u{1b}[1;32m~/project\u{1b}[0m \u{1b}[1;34mmain\u{1b}[0m $ claude\r\n"
        out += "\u{1b}[2m? for shortcuts\u{1b}[0m\r\n"
        out += "\u{1b}[1;35m●\u{1b}[0m Editing \u{1b}[4msrc/main.rs\u{1b}[0m\r\n"
        out += "  \u{1b}[32m+ let theme = Theme::from(config);\u{1b}[0m\r\n"
        out += "  \u{1b}[31m- let theme = Theme::default();\u{1b}[0m\r\n"
        out += "\u{1b}[33m!\u{1b}[0m \u{1b}[1mDo you want to make this edit?\u{1b}[0m\r\n"
        out += "  \u{1b}[36m\u{276f} 1. Yes\u{1b}[0m\r\n"
        out += " "
        for code in 30...37 { out += "\u{1b}[\(code)m\u{2588}\u{2588}\u{2588}\u{1b}[0m" }
        out += "\r\n "
        for code in 90...97 { out += "\u{1b}[\(code)m\u{2588}\u{2588}\u{2588}\u{1b}[0m" }
        return out
    }()

    private func draw(_ grid: TerminalGrid?, in context: inout GraphicsContext, size: CGSize) {
        guard let grid, grid.columns > 0, grid.rows > 0 else { return }
        let cellWidth = size.width / CGFloat(grid.columns)
        let cellHeight = size.height / CGFloat(grid.rows)
        // Sized from the cell rather than a fixed point size, so the preview
        // fills whatever height the form handed it.
        let font = Font.system(size: cellHeight * 0.8, design: .monospaced)
        let ground = Color(packed: theme.background)

        for row in 0..<grid.rows {
            var column = 0
            while column < grid.columns {
                let cell = grid[row, column]
                let step = cell.wide ? 2 : 1
                let origin = CGPoint(x: CGFloat(column) * cellWidth, y: CGFloat(row) * cellHeight)

                // Only what differs from the ground, like the real renderers:
                // filling every cell would be four hundred rects a frame to
                // paint the colour already behind them.
                if cell.background != ground {
                    context.fill(
                        Path(
                            CGRect(
                                origin: origin,
                                size: CGSize(
                                    width: cellWidth * CGFloat(step), height: cellHeight))),
                        with: .color(cell.background))
                }
                if let character = cell.character {
                    context.draw(
                        Text(String(character)).font(font).foregroundColor(cell.foreground),
                        at: origin,
                        anchor: .topLeading)
                }
                column += step
            }
        }
    }
}
