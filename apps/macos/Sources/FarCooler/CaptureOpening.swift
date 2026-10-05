import SwiftUI

// A test hook for the real-window captures (ov-284): a relaunch reopens a
// window through `Destination`, which keeps a plan page as its workspace
// (ov-273), so `RealWindowCaptures` can't reach a plan page by relaunching.
// It posts the place as `SelectionMemory.key` spells it, and the window opens
// it as a click would. Debug builds only; never input.

#if DEBUG
    extension Notification.Name {
        /// `object` is a selection as `SelectionMemory.encode` writes it.
        static let captureOpen = Notification.Name("com.farcooler.capture.open")
    }

    private struct CaptureOpening: ViewModifier {
        @Binding var selection: ContentView.Selection?

        func body(content: Content) -> some View {
            content.onReceive(NotificationCenter.default.publisher(for: .captureOpen)) { note in
                if let saved = note.object as? String, let next = SelectionMemory.decode(saved) { selection = next }
            }
        }
    }
#endif

extension View {
    /// Open the place a capture posts, in a debug build; nothing otherwise.
    func captureOpening(_ selection: Binding<ContentView.Selection?>) -> some View {
        #if DEBUG
            modifier(CaptureOpening(selection: selection))
        #else
            self
        #endif
    }
}
