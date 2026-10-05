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
            // A task key with a card shows the card instead (`TaskKeyHeldPreview`).
            isPresented: Binding(get: { link != nil && heldCard(link, linker) == nil }, set: { if !$0 { link = nil } }),
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

/// The card of the task a held link names, or nil: a URL, or a key with no
/// card. The linker is only built while a link is held.
@MainActor
func heldCard(_ link: String?, _ linker: () -> TaskKeyLinker) -> TaskKeyCard? {
    guard let link, let url = URL(string: link), TaskKeyLinks.parse(url) != nil else { return nil }
    return linker().card(for: url)
}

/// A long press on a task key in terminal output (ov-299): the task's card,
/// as a context menu's preview, beside the finger, with Open Task and Copy.
private struct TaskKeyHeldPreview: ViewModifier {
    @Binding var link: String?
    let point: CGPoint
    let linker: () -> TaskKeyLinker

    func body(content: Content) -> some View {
        let card = heldCard(link, linker)
        content.popover(
            isPresented: Binding(get: { card != nil }, set: { if !$0 { link = nil } }),
            attachmentAnchor: .rect(.rect(CGRect(x: point.x - 1, y: point.y - 1, width: 2, height: 2)))
        ) {
            if let card, let link, let url = URL(string: link) {
                VStack(alignment: .leading, spacing: 0) {
                    TaskKeyCardView(card: card)
                    Button {
                        self.link = nil
                        linker().follow(url)
                    } label: {
                        Label("Open Task", systemImage: "arrow.up.forward.square")
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .padding(14)
                    Button {
                        self.link = nil
                        UIPasteboard.general.string = card.key
                    } label: {
                        Label("Copy", systemImage: "doc.on.doc").frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .padding(14)
                }
                .presentationCompactAdaptation(.popover)
            }
        }
    }
}

extension View {
    /// A held task key's card, at `point` in this view, when the key has one;
    /// the held-link dialog takes everything else.
    func taskKeyHeldPreview(
        _ link: Binding<String?>, at point: CGPoint, linker: @autoclosure @escaping () -> TaskKeyLinker
    ) -> some View {
        modifier(TaskKeyHeldPreview(link: link, point: point, linker: linker))
    }

    /// The held-link dialog: a URL's Open Link and Copy Link, or a task key's
    /// Open Task and Copy. The linker is built only when Open Task is tapped,
    /// not on every pass of a view that redraws as a finger scrolls.
    func heldLinkDialog(_ link: Binding<String?>, linker: @autoclosure @escaping () -> TaskKeyLinker) -> some View {
        modifier(HeldLinkDialog(link: link, linker: linker))
    }
}
