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

    /// Add `new` after the images already waiting, up to `mostImages`.
    func attach(_ new: [Prepared]) {
        guard rich else { return }
        let room = max(0, AgentConversation.mostImages - images.count)
        for item in new.prefix(room) {
            thumbnails[item.image.id] = item.thumbnail
            images.append(item.image)
        }
        if new.count > room { issue = .said(AgentConversation.tooManyImages) }
    }

    /// Take the image `id` out of the message.
    func detach(_ id: UUID) {
        images.removeAll { $0.id == id }
        thumbnails[id] = nil
    }

    /// How many more images the message takes.
    var imageRoom: Int { max(0, AgentConversation.mostImages - images.count) }

    static let unreadableImage =
        "That photo couldn’t be read. If it lives in iCloud, open it in Photos first so it downloads."

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

    /// Whether Stop is offered: the runner serves it and claude is working.
    var offersStop: Bool { keys != nil && working && !store.isStale }

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
