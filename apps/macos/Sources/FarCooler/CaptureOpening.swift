import SwiftUI

// A test hook for the real-window captures (ov-284): a relaunch reopens a
// window through `Destination`, which keeps a plan page as its workspace
// (ov-273), so `RealWindowCaptures` can't reach a plan page by relaunching.
// It posts the place as `SelectionMemory.key` spells it, and the window opens
// it as a click would; "peek" peeks at the plan over the chat, as ⌥⌘P does
// (ov-298). Debug builds only; never input.

#if DEBUG
    extension Notification.Name {
        /// `object` is a selection as `SelectionMemory.encode` writes it.
        static let captureOpen = Notification.Name("com.farcooler.capture.open")
        /// `object` is a worktree's id: its Files open in the inspector beside
        /// it, as a path clicked in a terminal opens them (ov-297).
        static let captureFiles = Notification.Name("com.farcooler.capture.files")
    }

    private struct CaptureFiles: ViewModifier {
        let open: (String) -> Void

        func body(content: Content) -> some View {
            content.onReceive(NotificationCenter.default.publisher(for: .captureFiles)) { note in
                if let id = note.object as? String { open(id) }
            }
        }
    }

    private struct CaptureOpening: ViewModifier {
        @Binding var selection: ContentView.Selection?
        @Binding var peeking: Bool
        @Binding var fold: TreeFoldRequest

        func body(content: Content) -> some View {
            content.onReceive(NotificationCenter.default.publisher(for: .captureOpen)) { note in
                if note.object as? String == "peek" { return peeking = true }
                // View ▸ Collapse All, Expand All, as the menu asks (ov-334): a
                // capture window isn't key, so the command itself would be ignored.
                if note.object as? String == "fold:collapse" { return fold.collapse() }
                if note.object as? String == "fold:expand" { return fold.expand() }
                if let saved = note.object as? String, let next = SelectionMemory.decode(saved) { selection = next }
            }
        }
    }
#endif

extension View {
    /// Open the Files a capture posts, in a debug build; nothing otherwise.
    func captureFiles(_ open: @escaping (String) -> Void) -> some View {
        #if DEBUG
            modifier(CaptureFiles(open: open))
        #else
            self
        #endif
    }

    /// Open the place a capture posts, in a debug build; nothing otherwise.
    func captureOpening(
        _ selection: Binding<ContentView.Selection?>, peeking: Binding<Bool>, fold: Binding<TreeFoldRequest>
    ) -> some View {
        #if DEBUG
            modifier(CaptureOpening(selection: selection, peeking: peeking, fold: fold))
        #else
            self
        #endif
    }
}
