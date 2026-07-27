import Foundation

/// Serializes terminal input.
///
/// Terminal input MUST arrive in the order it was typed. Spawning one task per
/// keystroke does not do that: the tasks race, and typing "echo" can arrive as
/// "ehco". This actor keeps a FIFO buffer and drains it with exactly one
/// in-flight send at a time, so ordering is a property of the design rather than
/// a matter of luck.
///
/// Draining also coalesces whatever accumulated while a send was in flight, so
/// fast typing costs one round trip instead of one per character.
actor InputQueue {
    private var pending: [UInt8] = []
    private var draining = false

    /// Sends a contiguous byte run. Returns when that run has been delivered.
    typealias Sink = @Sendable ([UInt8]) async -> Void

    private let sink: Sink

    init(sink: @escaping Sink) {
        self.sink = sink
    }

    /// Queue bytes for delivery. Returns immediately; ordering is preserved.
    func submit(_ bytes: [UInt8]) {
        guard !bytes.isEmpty else { return }
        pending.append(contentsOf: bytes)
        guard !draining else { return }
        draining = true
        Task { await self.drain() }
    }

    private func drain() async {
        while !pending.isEmpty {
            // Take everything queued so far as one ordered run.
            let batch = pending
            pending.removeAll(keepingCapacity: true)
            await sink(batch)
        }
        draining = false
    }
}
