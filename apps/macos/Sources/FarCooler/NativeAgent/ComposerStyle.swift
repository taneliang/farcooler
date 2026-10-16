import AppKit
import SwiftUI

/// The colors the conversation composer draws its marks in (ov-498). The view
/// draws with these and `NativeComposerAppearanceTests` measures the same
/// constants, so changing one here changes what the test checks. They sit on
/// the box's glass, which shows the pane's own color through it.
enum ComposerStyle {
    /// The paperclip.
    static let paperclip = NSColor.secondaryLabelColor
    /// The empty box's hint and claude's suggested next prompt.
    static let placeholder = NSColor.tertiaryLabelColor
    /// Send when there is something to send.
    static let sendReady = NSColor.controlAccentColor
    /// Send when there is not.
    static let sendIdle = NSColor.tertiaryLabelColor
}
