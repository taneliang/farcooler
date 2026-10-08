import SwiftUI

// Layout probes for the UI tests (ov-422, ov-424). A row that reads as one
// accessibility element hides its parts from XCUITest, so a test could not say
// where the icon sat or whether the age wrapped. A probe publishes a part's
// frame, and for text what it measured, as an element of its own beside the row.
// Nothing is drawn, and a release build has none of it.

#if DEBUG

    private struct ProbeEntry {
        let frame: Anchor<CGRect>
        let note: String
    }

    private struct ProbeKey: PreferenceKey {
        static var defaultValue: [String: ProbeEntry] = [:]
        static func reduce(value: inout [String: ProbeEntry], nextValue: () -> [String: ProbeEntry]) {
            value.merge(nextValue()) { _, new in new }
        }
    }

    /// What a `Text` needed against what it got: its lines, and whether any
    /// text was cut off, read from a hidden twin laid out at the real width
    /// with no line limit.
    private struct TextFit: ViewModifier {
        let id: String
        let text: String
        let font: Font
        @State private var size = CGSize.zero
        @State private var ideal: CGFloat = 0
        @State private var line: CGFloat = 0

        func body(content: Content) -> some View {
            content
                .onGeometryChange(for: CGSize.self) { $0.size } action: { size = $0 }
                .background(alignment: .topLeading) {
                    Text(text).font(font).fixedSize(horizontal: false, vertical: true)
                        .frame(width: size.width, alignment: .leading)
                        .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { ideal = $0 }
                        .hidden()
                }
                .background(alignment: .topLeading) {
                    Text("Ag").font(font).fixedSize()
                        .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { line = $0 }
                        .hidden()
                }
                .frameProbe(id, note: note)
        }

        private var note: String {
            guard line > 0 else { return "" }
            let lines = Int((size.height / line).rounded())
            let cut = size.height < ideal - 1
            return "lines=\(lines) cut=\(cut)"
        }
    }

    extension View {
        /// Publish this view's frame as `id`; `probesShown()` draws it.
        func frameProbe(_ id: String, note: String = "") -> some View {
            anchorPreference(key: ProbeKey.self, value: .bounds) { [id: ProbeEntry(frame: $0, note: note)] }
        }

        /// Publish this `Text`'s frame and fit (`lines=N cut=B`) as `id`.
        func textFitProbe(_ id: String, text: String, font: Font) -> some View {
            modifier(TextFit(id: id, text: text, font: font))
        }

        /// Draw the probes published below this view, as invisible elements at their frames.
        func probesShown() -> some View {
            overlayPreferenceValue(ProbeKey.self) { entries in
                GeometryReader { proxy in
                    ForEach(entries.keys.sorted(), id: \.self) { id in
                        let frame = proxy[entries[id]!.frame]
                        Color.clear  // style-exempt: DEBUG probe: an invisible element at a part's frame, only the UI tests read it
                            .frame(width: frame.width, height: frame.height)
                            .position(x: frame.midX, y: frame.midY)
                            .accessibilityElement()
                            .accessibilityIdentifier(id)
                            .accessibilityValue(entries[id]!.note)
                    }
                }
                .allowsHitTesting(false)
            }
        }
    }

#else

    extension View {
        func frameProbe(_ id: String, note: String = "") -> some View { self }
        func textFitProbe(_ id: String, text: String, font: Font) -> some View { self }
        func probesShown() -> some View { self }
    }

#endif
