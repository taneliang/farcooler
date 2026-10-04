import AgentKit
import SwiftUI

/// The shape of the glass family that rests over an agent chat (ov-223): the
/// plan, the queued message and the composer. Concentric with the pane card they
/// sit in, `Radius.medium` padded 10 pt, which is the 6 pt minimum: the composer
/// used to be 20 pt and the queue's bubble 14, three radii for one family.
enum ChatFamily {
    /// The pane card's radius less the 10 pt the family is inset from it.
    static let radius = Radius.concentric(outer: Radius.medium, padding: 10)
}

extension Shape where Self == RoundedRectangle {
    /// A member of the chat's glass family, continuous like every card.
    static var chatFamily: RoundedRectangle {
        RoundedRectangle(cornerRadius: ChatFamily.radius, style: .continuous)
    }
}
