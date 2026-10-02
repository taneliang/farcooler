import SwiftUI

extension Text {
    /// A text field's prompt, in the system's placeholder weight.
    ///
    /// A bare `Text` as `prompt:` inside a grouped form drew at 86% of a typed
    /// value's brightness, so "user@host, or an SSH alias" and "feat/my-change"
    /// read as values already filled in (ov-81 P7). Tertiary is the step that
    /// separates a suggestion from an answer in both appearances.
    static func fieldPrompt(_ text: String) -> Text {
        Text(verbatim: text).foregroundStyle(.tertiary)
    }
}
