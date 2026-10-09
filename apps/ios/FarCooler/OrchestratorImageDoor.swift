import PhotosUI
import SwiftUI
import UIKit

/// The way to put an image into a terminal-mode orchestrator's message.
///
/// The orchestrator's pane is its own screen (`OrchestratorSegment`), not a
/// shell pane, so `ShellPaneChromeModifier`'s image menu never reached it. The
/// conversation's composer takes photos itself (`NativeComposer`), and so does
/// a chat pane's (`AgentView`); what was left with no door at all is the
/// orchestrator drawn as a terminal, where the runner's projector is off or the
/// conversation is put away. This is that door: a photo from the library, or
/// the image on the pasteboard, sent the way a shell pane's is, by
/// `ImagePasteQueue`, whose chips `OrchestratorSegment` draws over the pane.
struct OrchestratorImageDoor: ViewModifier {
    let terminal: Terminal
    @ObservedObject var connection: Connection
    @ObservedObject var pastes: ImagePasteQueue
    /// Whether the pane is the one on screen: a hidden pane has no toolbar.
    let isVisible: Bool

    @ObservedObject private var nativePanes = NativePanes.shared
    @State private var showPicker = false
    @State private var picked: PhotosPickerItem?

    /// Not while a conversation covers the terminal (its composer has its own
    /// photo button), and not on a chat pane (nor a diff), which has one too.
    private var offered: Bool {
        isVisible && !terminal.isAgentPane && !terminal.isClientDrawn && !nativePanes.covered.contains(terminal.id)
    }

    func body(content: Content) -> some View {
        content
            .toolbar {
                if offered {
                    ToolbarItem(placement: .topBarTrailing) { menu }
                }
            }
            // Hung off the pane, not the menu: a picker inside a `Menu` has no
            // live view hierarchy to present from (`ShellPaneChromeModifier`).
            .photosPicker(isPresented: $showPicker, selection: $picked, matching: .images)
            .onChange(of: picked) { _, item in
                guard let item else { return }
                Task {
                    // The original file, not an `Image`: re-encoding a
                    // screenshot would smear the small text that is usually
                    // the reason for sending one.
                    let data = try? await item.loadTransferable(type: Data.self)
                    picked = nil
                    if let data { deliver(data) }
                }
            }
            #if DEBUG
            .onReceive(NotificationCenter.default.publisher(for: HarnessTaps.orchestratorPhoto)) { _ in
                guard isVisible else { return }
                let size = CGSize(width: 64, height: 64)
                let png = UIGraphicsImageRenderer(size: size).pngData { context in
                    UIColor.systemTeal.setFill()  // style-exempt: a test photo's pixels, not UI
                    context.fill(CGRect(origin: .zero, size: size))
                }
                deliver(png)
            }
            #endif
    }

    /// The one path a photo takes from the picker, a paste or the harness.
    private func deliver(_ data: Data) {
        guard let image = UIImage(data: data) else { return }
        pastes.send(image, terminal: terminal.id, core: connection.core)
    }

    private var menu: some View {
        Menu {
            Button {
                showPicker = true
            } label: {
                Label("Choose Photo", systemImage: "photo")
            }
            .accessibilityIdentifier("orchestrator-choose-photo")
            if UIPasteboard.general.hasImages {
                Button {
                    if let image = UIPasteboard.general.image {
                        pastes.send(image, terminal: terminal.id, core: connection.core)
                    }
                } label: {
                    Label("Paste Image", systemImage: "doc.on.clipboard")
                }
                .accessibilityIdentifier("orchestrator-paste-image")
            }
        } label: {
            Image(systemName: "photo.badge.plus")
        }
        .accessibilityLabel("Send an image")
        .accessibilityIdentifier("orchestrator-image-menu")
    }
}
