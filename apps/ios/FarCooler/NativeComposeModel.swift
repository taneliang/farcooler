import SwiftUI
import UIKit

/// What the conversation composer adds to a pane's model (ov-404): the images
/// that wait to go with a message, and Stop and Send Now. The rules are
/// AgentKit's (`AgentConversation`, `OutgoingImage`); what's here is the
/// model's state and its calls.
extension NativePaneModel {
    /// A picked or pasted image, made ready off the main thread: converted
    /// where the runner wouldn't take it as it is, and its chip drawn small.
    struct Prepared: @unchecked Sendable {
        let image: OutgoingImage
        let thumbnail: UIImage?
    }

    /// `datas` read as images, off the main thread, which a 48 MP photo's
    /// decode and re-encode would otherwise hold. An entry that isn't an
    /// image, or didn't load (nil, as an iCloud photo not on the device
    /// comes), is left out; the count says how many were.
    nonisolated static func prepare(_ datas: [Data?]) async -> (ready: [Prepared], unreadable: Int) {
        await Task.detached(priority: .userInitiated) {
            var ready: [Prepared] = []
            var unreadable = 0
            for data in datas {
                guard let data, let image = OutgoingImage.make(data) else {
                    unreadable += 1
                    continue
                }
                ready.append(Prepared(image: image, thumbnail: image.thumbnail().map { UIImage(cgImage: $0) }))
            }
            return (ready, unreadable)
        }.value
    }

    /// The one path a photo takes into the composer, from the picker, a paste
    /// or a harness standing in for either: read, converted and added, or
    /// said why not.
    func attach(picked datas: [Data?]) async {
        guard rich else { return }
        let (ready, unreadable) = await Self.prepare(datas)
        attach(ready)
        if unreadable > 0 { issue = .said(Self.unreadableImage) }
    }

    /// Add `new` after the images already waiting, up to `mostImages`, and
    /// keep each on disk for a restart (ov-622), unless it came from there.
    func attach(_ new: [Prepared], keeping: Bool = true) {
        guard rich else { return }
        let room = max(0, AgentConversation.mostImages - images.count)
        for item in new.prefix(room) {
            thumbnails[item.image.id] = item.thumbnail
            images.append(item.image)
            if keeping { draftImages.keep(item.image, terminal: terminal) }
        }
        if new.count > room { issue = .said(AgentConversation.tooManyImages) }
    }

    /// Take the image `id` out of the message.
    func detach(_ id: UUID) {
        images.removeAll { $0.id == id }
        thumbnails[id] = nil
        draftImages.drop(id, terminal: terminal)
    }

    /// The photos the last launch left in this pane's draft, back in the
    /// composer once the runner takes photos (ov-622).
    func restoreImages() {
        guard rich, !imagesRestored else { return }
        imagesRestored = true
        let sentBefore = sent
        Task {
            let kept = await draftImages.restored(terminal: terminal)
            // Not into a composer that has sent since: it is a new message.
            guard !kept.isEmpty, sent == sentBefore else { return }
            let ready = await Task.detached(priority: .userInitiated) {
                kept.map { Prepared(image: $0, thumbnail: $0.thumbnail().map { UIImage(cgImage: $0) }) }
            }.value
            // Nor if it sent while the thumbnails were made.
            guard sent == sentBefore else { return }
            let have = Set(images.map(\.id))
            attach(ready.filter { !have.contains($0.image.id) }, keeping: false)
        }
    }

    /// Whether the message has anything to send: words, not only spaces, or
    /// a photo. The composer's trailing action is Send while it does (ov-677).
    var hasContent: Bool {
        !images.isEmpty || !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    /// How many more images the message takes.
    var imageRoom: Int { max(0, AgentConversation.mostImages - images.count) }

    static let unreadableImage =
        "That photo couldn’t be read. If it lives in iCloud, open it in Photos first so it downloads."

    // MARK: - claude's suggested prompt (ov-409)

    /// The prompt claude's own box suggests, offered as the composer's
    /// placeholder while nothing is typed (`AgentConversation.suggestion`).
    var suggestion: String? { store.suggestion(draft: draft) }

    /// claude's generic `Try "…"` example, in place of "Message Claude" while
    /// nothing is typed. A hint: a tap and Tab do not take it, as in claude.
    var hint: String? { store.hint(draft: draft) }

    /// A tap on the suggestion, or Tab from a hardware keyboard: it becomes
    /// the draft, to edit. Never sent.
    func takeSuggestion() {
        guard let words = suggestion else { return }
        draft = words
        suggestionsTaken += 1
    }

    // MARK: - Stop and Send Now

    /// Whether claude is working on a turn, as the newest turn's row says
    /// (`AgentConversation.isWorking`: not while a dialog is up).
    var working: Bool {
        for id in store.ids.reversed() {
            if case .turn(let turn)? = store.box(id)?.row.kind {
                return AgentConversation.isWorking(newestTurn: turn)
            }
        }
        return false
    }

    /// Whether Stop is offered: the runner serves it, the pane is claude,
    /// whose keys it presses (never codex's), and claude is working.
    var offersStop: Bool { keys != nil && AgentConversation.pressesKeys(preset: preset) && working && !store.isStale }

    /// Whether Send Now is offered on a Queued row.
    var offersSendNow: Bool { offersStop }

    /// Stop the turn.
    func stop() async { await press(.stop) }

    /// Send what waits in claude's queue now: a Queued row's Send Now.
    func sendNow() async { await press(.sendNow) }

    private func press(_ key: AgentConversation.PaneKey) async {
        guard let keys, pressing == nil, offersStop else { return }
        pressing = key
        defer { pressing = nil }
        issue = nil
        do {
            switch key {
            case .stop: try await keys.interrupt(terminal: terminal)
            case .sendNow: try await keys.sendNow(terminal: terminal)
            }
        } catch {
            issue = AgentConversation.keyIssue(for: Self.failure(error), key)
        }
    }
}
