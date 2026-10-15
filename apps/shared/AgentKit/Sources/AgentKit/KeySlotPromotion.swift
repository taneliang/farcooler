import Foundation

/// One of a phone's three key slots during "Make a New Key" (ov-569).
public enum KeySlot: String, Sendable, CaseIterable {
    case current, next, previous
}

/// Where the slots live: the Keychain on an iPhone. Private keys pass through
/// here, so only the app implements it; AgentKit only orders the steps.
public protocol KeySlotStore {
    func read(_ slot: KeySlot) -> String?
    /// False when the store refused, and then the slot is as it was.
    func write(_ value: String, to slot: KeySlot) -> Bool
    func delete(_ slot: KeySlot)
}

/// Putting the new key in use, so that stopping at any step (a crash, a
/// refused write) and running it again ends the same way, and neither key is
/// ever lost (fix round 1, M1).
///
/// The old key is copied to `previous`, never over an old key already there;
/// then `current` becomes the new key; then the waiting copy is deleted. Run
/// again after `current` already holds the new key, it only deletes the
/// waiting copy, rather than copying the new key over the old one.
public enum KeySlotPromotion {
    /// True once the new key is in use and the old one is in `previous`.
    public static func promote(_ store: KeySlotStore) -> Bool {
        guard let next = store.read(.next) else { return false }
        guard let current = store.read(.current) else { return false }
        if current == next {
            // Stopped after the swap: the old key is in `previous` already.
            guard store.read(.previous) != nil else { return false }
            store.delete(.next)
            return true
        }
        switch store.read(.previous) {
        case nil:
            guard store.write(current, to: .previous) else { return false }
        case current?:
            break
        default:
            // Another old key, still kept for some runner: never overwritten.
            return false
        }
        guard store.write(next, to: .current) else { return false }
        store.delete(.next)
        return true
    }
}
