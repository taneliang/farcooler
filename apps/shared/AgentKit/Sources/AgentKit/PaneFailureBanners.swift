import Foundation

// The banners beside an agent pane's composer (ov-179 fix round).
//
// A refused control used to take the one send-failure slot, which dropped the
// only Retry for an unsent message, and a Retry that worked left its banner on
// screen. Two slots and a key fix both, and with no UIKit in them they can be
// tested.

/// What is said beside the composer: an unsent message first, then a refused
/// control.
public struct PaneFailureBanners<Failure: Identifiable> {
    /// The unsent message's failure. Kept while a control fails.
    public var send: Failure?
    /// The latest refused control.
    public private(set) var control: Failure?
    private var controlKey: String?

    /// No banners.
    public init() {}

    /// Everything to draw, the send first.
    public var all: [Failure] { [send, control].compactMap { $0 } }

    /// A control refused. Replaces an earlier control's banner, never the send's.
    public mutating func controlFailed(_ failure: Failure, key: String) {
        control = failure
        controlKey = key
    }

    /// A control worked: its own banner goes, another control's stays.
    public mutating func controlSucceeded(key: String) {
        guard controlKey == key else { return }
        control = nil
        controlKey = nil
    }

    /// Dismiss one banner by id.
    public mutating func dismiss(_ id: Failure.ID) {
        if send?.id == id { send = nil }
        if control?.id == id {
            control = nil
            controlKey = nil
        }
    }
}
