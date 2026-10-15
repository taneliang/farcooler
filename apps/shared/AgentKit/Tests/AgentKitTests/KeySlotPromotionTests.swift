import Testing

@testable import AgentKit

/// Putting the new key in use survives stopping at every step (ov-569 fix
/// round 1, M1).
struct KeySlotPromotionTests {
    /// Slots in memory that stop working after `budget` writes and deletes,
    /// as a crash would leave them.
    final class Slots: KeySlotStore {
        var values: [KeySlot: String]
        var budget: Int

        init(_ values: [KeySlot: String], budget: Int = .max) {
            self.values = values
            self.budget = budget
        }
        func read(_ slot: KeySlot) -> String? { values[slot] }
        func write(_ value: String, to slot: KeySlot) -> Bool {
            guard budget > 0 else { return false }
            budget -= 1
            values[slot] = value
            return true
        }
        func delete(_ slot: KeySlot) {
            guard budget > 0 else { return }
            budget -= 1
            values[slot] = nil
        }
    }

    @Test("Stopped after any step, running it again puts the new key in use and keeps the old one")
    func everyCrashPoint() {
        for stop in 0...3 {
            let slots = Slots([.current: "old", .next: "new"], budget: stop)
            _ = KeySlotPromotion.promote(slots)
            // Nothing lost at the stop.
            #expect(Set(slots.values.values).isSuperset(of: ["old", "new"]), "stopped after \(stop) steps")
            slots.budget = .max
            let finished = slots.values[.next] == nil
            #expect(KeySlotPromotion.promote(slots) != finished, "stopped after \(stop) steps")
            #expect(slots.values == [.current: "new", .previous: "old"], "stopped after \(stop) steps")
            // And once more is harmless.
            #expect(!KeySlotPromotion.promote(slots))
            #expect(slots.values == [.current: "new", .previous: "old"])
        }
    }

    @Test("An old key already kept for a runner is never written over")
    func neverOverwritesPrevious() {
        let slots = Slots([.current: "middle", .next: "new", .previous: "oldest"])
        #expect(!KeySlotPromotion.promote(slots))
        #expect(slots.values == [.current: "middle", .next: "new", .previous: "oldest"])
    }
}
