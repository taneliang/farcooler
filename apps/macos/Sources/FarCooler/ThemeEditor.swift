import AppKit
import SwiftUI

/// Nineteen colours, over a terminal actually rendering them.
///
/// The preview is the point. Nineteen hex values tell you nothing about whether
/// a theme is readable, and the one question anybody has about a colour scheme —
/// can I look at this for eight hours — is only answerable by looking at it.
///
/// It is a real `TerminalRenderView` fed a fixture, not a hand-drawn mock:
/// cell colours are resolved inside the VT core on purpose, so three renderers
/// cannot drift, and a mock here would be a fourth renderer drifting from all of
/// them. Previewing means setting a palette on a throwaway core, which is the
/// same call a live terminal takes.
struct ThemeEditor: View {
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
        VStack(spacing: 0) {
            ThemePreview(theme: draft)
                .frame(height: 190)
            Divider()
            Form {
                Section {
                    TextField("Name", text: $draft.name)
                        .autocorrectionDisabled()
                    Toggle("Dark surfaces around the terminal", isOn: $draft.dark)
                } footer: {
                    // Why `dark` is a choice rather than something derived: this
                    // is the reasoning the core's own Theme documents, said where
                    // the person choosing can read it.
                    Text(
                        "Choose whether this theme uses dark surfaces around the terminal.")
                }

                Section("Ground") {
                    well("Background", $draft.background)
                    well("Text", $draft.foreground)
                    well("Cursor", $draft.cursor)
                }

                Section {
                    swatchGrid(range: 0..<8, labels: Self.normalNames)
                } header: {
                    Text("Normal")
                }

                Section {
                    swatchGrid(range: 8..<16, labels: Self.brightNames)
                } header: {
                    Text("Bright")
                } footer: {
                    Text(
                        "The theme controls these 16 ANSI colors. Extended colors are generated "
                        + "automatically.")
                }
            }
            .formStyle(.grouped)
            Divider()
            footer
        }
        // Shorter than the window it opens from. `RunnerSettings` is
        // 620x560 and this was 660 tall — a sheet a hundred points taller
        // than its own host, which either forces the window to grow or hangs
        // off it. The `Form` is `.grouped` and scrolls, so the height was
        // never load-bearing; only the preview strip above it is.
        .frame(width: 560, height: 520)
    }

    private var footer: some View {
        HStack {
            Button("Revert Changes") { draft = original }
                .disabled(draft == original)
            Spacer()
            Button("Cancel") { dismiss() }
            Button("Save") {
                onSave(draft)
                dismiss()
            }
            .keyboardShortcut(.defaultAction)
            .disabled(draft.name.trimmingCharacters(in: .whitespaces).isEmpty)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
    }

    /// One labelled colour well.
    ///
    /// `NSColorWell` through SwiftUI's `ColorPicker`, so it is the system picker
    /// with the eyedropper people already know — picking a colour off a running
    /// terminal is exactly how someone matches a theme to a screenshot.
    private func well(_ label: String, _ packed: Binding<UInt32>) -> some View {
        ColorPicker(label, selection: Binding(
            get: { Color(nsColor: Theme.color(packed.wrappedValue)) },
            set: { packed.wrappedValue = Self.pack($0) }
        ), supportsOpacity: false)
    }

    /// Eight wells in two rows of four, which is as wide as this sheet allows
    /// without the labels truncating.
    private func swatchGrid(range: Range<Int>, labels: [String]) -> some View {
        let columns = [GridItem(.flexible()), GridItem(.flexible())]
        return LazyVGrid(columns: columns, alignment: .leading, spacing: 6) {
            ForEach(Array(range), id: \.self) { index in
                ColorPicker(
                    labels[index % 8],
                    selection: Binding(
                        get: { Color(nsColor: Theme.color(draft.ansi[index])) },
                        set: { draft.ansi[index] = Self.pack($0) }
                    ),
                    supportsOpacity: false)
            }
        }
    }

    /// ANSI's own names, because they are what an escape sequence means and what
    /// every other terminal's settings call them.
    static let normalNames = [
        "Black", "Red", "Green", "Yellow", "Blue", "Magenta", "Cyan", "White",
    ]
    static let brightNames = normalNames.map { "Bright \($0)" }

    /// A SwiftUI colour as `0x00RRGGBB`.
    ///
    /// Through sRGB explicitly. A `Color` can be in any colour space, and taking
    /// its components without converting produces values that look right in the
    /// picker and wrong in the terminal.
    static func pack(_ color: Color) -> UInt32 {
        let ns = NSColor(color).usingColorSpace(.sRGB) ?? .black
        let r = UInt32((ns.redComponent * 255).rounded())
        let g = UInt32((ns.greenComponent * 255).rounded())
        let b = UInt32((ns.blueComponent * 255).rounded())
        return (r << 16) | (g << 8) | b
    }
}

/// A terminal rendering a fixture in the theme being edited.
private struct ThemePreview: NSViewRepresentable {
    let theme: Theme

    func makeNSView(context: Context) -> NSView {
        let container = PreviewContainer()
        container.render(theme)
        return container
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        (nsView as? PreviewContainer)?.render(theme)
    }

    /// Holds a render view and re-feeds it whenever the palette changes.
    ///
    /// A container rather than the render view itself, because that view reports
    /// its geometry to a pane and claims the keyboard when clicked — neither of
    /// which a preview should do. This one owns a view that is never attached to
    /// anything and never given focus.
    final class PreviewContainer: NSView {
        private let view = TerminalRenderView()
        private var lastPalette: [UInt32] = []

        override init(frame: NSRect) {
            super.init(frame: frame)
            view.translatesAutoresizingMaskIntoConstraints = false
            addSubview(view)
            NSLayoutConstraint.activate([
                view.leadingAnchor.constraint(equalTo: leadingAnchor),
                view.trailingAnchor.constraint(equalTo: trailingAnchor),
                view.topAnchor.constraint(equalTo: topAnchor),
                view.bottomAnchor.constraint(equalTo: bottomAnchor),
            ])
        }

        required init?(coder: NSCoder) { fatalError("not used") }

        func render(_ theme: Theme) {
            let palette = theme.packed
            guard palette != lastPalette else { return }
            lastPalette = palette
            // A fresh core each time rather than recolouring in place: the
            // fixture is a few hundred bytes, and rebuilding means the preview
            // cannot accumulate state from a palette that is no longer chosen.
            view.reset(columns: 64, rows: 11)
            view.core.setPalette(palette)
            view.feed(Array(Self.fixture.utf8))
            view.overrideBackground(theme.background)
            view.needsDisplay = true
        }

        /// Output chosen to exercise what a theme actually has to get right:
        /// all sixteen colours, bold, a prompt, a diff, and an agent's furniture.
        static let fixture: String = {
            var out = "\u{1b}[H\u{1b}[2J"
            out += "\u{1b}[1;32m~/project\u{1b}[0m \u{1b}[1;34mmain\u{1b}[0m $ claude\r\n"
            out += "\u{1b}[2m? for shortcuts\u{1b}[0m\r\n"
            out += "\u{1b}[1;35m●\u{1b}[0m Editing \u{1b}[4msrc/main.rs\u{1b}[0m\r\n"
            out += "  \u{1b}[32m+ let theme = Theme::from(config);\u{1b}[0m\r\n"
            out += "  \u{1b}[31m- let theme = Theme::default();\u{1b}[0m\r\n"
            out += "\u{1b}[33m!\u{1b}[0m \u{1b}[1mDo you want to make this edit?\u{1b}[0m\r\n"
            out += "  \u{1b}[36m❯ 1. Yes\u{1b}[0m\r\n"
            out += "    2. No, tell Claude what to do differently\r\n"
            // Every colour, so nothing is untested by the eye.
            out += "\r\n "
            for code in 30...37 { out += "\u{1b}[\(code)m███\u{1b}[0m" }
            out += "\r\n "
            for code in 90...97 { out += "\u{1b}[\(code)m███\u{1b}[0m" }
            return out
        }()
    }
}
