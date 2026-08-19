import Foundation
import Testing

@testable import AgentKit

/// Reading and writing the file every glance surface renders from.
///
/// The container is injected as a directory so these run on a Mac with no App
/// Group at all — the interesting behavior is atomicity and the shape of a
/// failure, neither of which needs a real group to exercise.
struct SnapshotStoreTests {
    private func snapshot(_ status: String) -> FleetSnapshot {
        FleetSnapshot(
            agents: [FleetSnapshot.Agent(
                id: "t1", label: "claude", machine: "orchard", status: status,
                glyph: "●", headline: "claude 4m", line: "Writing fruit.txt",
                feed: [], rank: 0, turnFailed: false, activityChangedAt: nil)],
            capturedAt: Date(timeIntervalSince1970: 42), complete: true)
    }

    @Test func aSnapshotComesBackAsItWentIn() throws {
        let dir = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }

        try SnapshotStore.write(snapshot("working"), toContainer: dir)
        #expect(SnapshotStore.read(fromContainer: dir) == snapshot("working"))
    }

    /// Nothing written yet is nil, not a throw and not an empty snapshot: a
    /// caller has to be able to tell "never polled" from "polled, found none".
    @Test func anAbsentSnapshotReadsAsNil() {
        let dir = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent(UUID().uuidString)
        #expect(SnapshotStore.read(fromContainer: dir) == nil)
    }

    /// A half-written file is a widget showing something that was never true.
    /// The write replaces atomically, so a reader sees the old one or the new
    /// one and never a torn one.
    @Test func aSecondWriteReplacesTheFirstWhole() throws {
        let dir = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }

        try SnapshotStore.write(snapshot("working"), toContainer: dir)
        try SnapshotStore.write(snapshot("blocked"), toContainer: dir)
        #expect(SnapshotStore.read(fromContainer: dir)?.agents.first?.status == "blocked")
    }

    /// Garbage on disk reads as nil rather than throwing into a widget's
    /// timeline provider, which has nowhere to put an error.
    @Test func anUnreadableFileReadsAsNil() throws {
        let dir = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }

        try Data("not json".utf8).write(to: dir.appendingPathComponent("fleet.json"))
        #expect(SnapshotStore.read(fromContainer: dir) == nil)
    }
}
