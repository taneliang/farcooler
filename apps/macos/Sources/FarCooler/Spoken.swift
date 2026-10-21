import SwiftUI

/// What a control says aloud and on hover: its VoiceOver label and value and its
/// tooltip (ov-603).
struct SpokenEntry: Equatable {
    let label: String
    let value: String
    let help: String
}

/// The strings every `.spoken` control under a view draws, so a test reads
/// them off the real, hosted view. A hosted view's accessibility tree isn't
/// readable in a test, and the strings as statics can't catch a control that
/// stops using them; a preference travels with the control that draws it.
struct SpokenStrings: PreferenceKey {
    static let defaultValue: [SpokenEntry] = []
    static func reduce(value: inout [SpokenEntry], nextValue: () -> [SpokenEntry]) { value += nextValue() }
}

extension View {
    /// One call for what a control says: the tooltip, the VoiceOver label and
    /// the VoiceOver value (empty when it has none), all published through
    /// `SpokenStrings`. Use it on an icon-only control, whose glyph says
    /// neither, so no button carries a tooltip a test can't see.
    func spoken(label: String, value: String? = nil, help: String) -> some View {
        self
            .help(help)
            .accessibilityLabel(label)
            .accessibilityValue(value ?? "")
            .preference(key: SpokenStrings.self, value: [SpokenEntry(label: label, value: value ?? "", help: help)])
    }
}
