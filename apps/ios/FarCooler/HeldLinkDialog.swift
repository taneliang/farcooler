import SwiftUI

/// The link a long press on terminal output landed on.
///
/// A URL is titled with itself, because "Open Link" without saying which link
/// is a button that asks you to trust output an agent produced. A task key's
/// link (ov-215) is titled with the key and opens the task in the app, through
/// the linker, and never through the system: it would hand `farcooler://` to
/// whichever channel's app claimed it last.
private struct HeldLinkDialog: ViewModifier {
    @Binding var link: String?
    let linker: () -> TaskKeyLinker

    private var task: String? {
        link.flatMap(URL.init(string:)).flatMap(TaskKeyLinks.parse)?.key
    }

    func body(content: Content) -> some View {
        content.confirmationDialog(
            task ?? link ?? "",
            isPresented: Binding(get: { link != nil }, set: { if !$0 { link = nil } }),
            titleVisibility: .visible
        ) {
            if let task, let link, let url = URL(string: link) {
                Button("Open Task") { linker().follow(url) }
                Button("Copy") { UIPasteboard.general.string = task }
            } else if let link, let url = URL(string: link) {
                Button("Open Link") { UIApplication.shared.open(url) }
                Button("Copy Link") { UIPasteboard.general.string = link }
            }
            Button("Cancel", role: .cancel) {}
        }
    }
}

extension View {
    /// The held-link dialog: a URL's Open Link and Copy Link, or a task key's
    /// Open Task and Copy. The linker is built only when Open Task is tapped,
    /// not on every pass of a view that redraws as a finger scrolls.
    func heldLinkDialog(_ link: Binding<String?>, linker: @autoclosure @escaping () -> TaskKeyLinker) -> some View {
        modifier(HeldLinkDialog(link: link, linker: linker))
    }
}
