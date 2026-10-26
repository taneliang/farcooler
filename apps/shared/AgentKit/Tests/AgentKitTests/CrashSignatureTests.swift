import Foundation
import Testing

@testable import AgentKit

/// A crash's signature out of MetricKit's JSON, and nothing else (ov-797).
/// `test/fixtures/crash-report/metrickit-crash.json` is an
/// `MXCrashDiagnostic`'s `jsonRepresentation()` shaped like ov-787's crash, an
/// uncaught Objective-C exception, with `SENTINEL` in every field that must
/// not leave the phone; `signature.json` is what the phone sends, which the
/// runner's tests (`crates/daemon/src/crash_reports_tests.rs`,
/// `crates/client/src/session/crash_report.rs`) read back.
struct CrashSignatureTests {
    static func root() -> URL {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        return root
    }

    static func fixture(_ name: String) throws -> Data {
        try Data(contentsOf: root().appendingPathComponent("test/fixtures/crash-report/\(name)"))
    }

    static func signature() throws -> CrashSignature {
        try #require(CrashSignature.read(diagnostic: fixture("metrickit-crash.json"), kind: .crash, ownBinary: "FarCooler"))
    }

    @Test("MetricKit's JSON reads to the signature the runner's tests read back")
    func theFixtureReadsToTheSignature() throws {
        let wire = try Self.signature().wire
        let expected = try #require(JSONSerialization.jsonObject(with: Self.fixture("signature.json")) as? [String: Any])
        #expect(NSDictionary(dictionary: wire).isEqual(to: expected), "\(wire)")
    }

    @Test("The top frames, then the app's own first frame when none of those is")
    func theTopFramesAndTheAppsOwn() throws {
        let frames = try Self.signature().frames
        #expect(frames.map(\.depth) == Array(0..<10) + [11])
        #expect(frames.first?.binary == "libsystem_kernel.dylib")
        #expect(frames.filter(\.own).map(\.depth) == [11])
        #expect(try Self.signature().exceptionName == "NSInvalidArgumentException")
    }

    @Test("Nothing but the signature leaves: no message, thread name, call tree, device or UUID")
    func nothingMoreLeaves() throws {
        let wire = try Self.signature().wire
        #expect(Set(wire.keys) == ["kind", "exception_type", "exception_code", "signal", "exception_name", "frames", "app_version", "app_build", "os"])
        for frame in try #require(wire["frames"] as? [[String: Any]]) {
            #expect(Set(frame.keys) == ["binary", "offset", "symbol", "depth", "own"])
        }
        let sent = String(decoding: try JSONSerialization.data(withJSONObject: wire), as: UTF8.self)
        for never in ["SENTINEL", "subFrames", "callStack", "threadName", "binaryUUID", "address", "composedMessage", "4242"] {
            #expect(!sent.contains(never), "\(never) left the phone: \(sent)")
        }
        #expect(try Self.signature().frames.count <= CrashSignature.topFrames + 1)
    }

    @Test("An exception name that isn't a class name is dropped, not sent")
    func aMessageForANameIsDropped() throws {
        var object = try #require(JSONSerialization.jsonObject(with: Self.fixture("metrickit-crash.json")) as? [String: Any])
        var meta = try #require(object["diagnosticMetaData"] as? [String: Any])
        meta["exceptionReason"] = ["exceptionName": "NSInvalidArgumentException: the user typed this"]
        meta["osVersion"] = "iPhone OS 26.1\nSENTINEL"
        object["diagnosticMetaData"] = meta
        let read = try #require(CrashSignature.read(diagnostic: JSONSerialization.data(withJSONObject: object), kind: .crash, ownBinary: "FarCooler"))
        #expect(read.exceptionName == "")
        #expect(read.os == "iPhone OS 26.1SENTINEL", "control characters go")
    }

    @Test("A hang with no attributed thread signs the first; a diagnostic with no stack signs nothing")
    func aHangAndNoStack() throws {
        let hang: [String: Any] = [
            "callStackTree": ["callStacks": [["callStackRootFrames": [["binaryName": "/usr/lib/UIKitCore", "offsetIntoBinaryTextSegment": 12, "subFrames": []]]]]],
            "diagnosticMetaData": ["appBuildVersion": "7", "hangDuration": "3 sec"],
        ]
        let read = try #require(CrashSignature.read(diagnostic: JSONSerialization.data(withJSONObject: hang), kind: .hang, ownBinary: "FarCooler"))
        #expect(read.kind == .hang && read.frames == [.init(binary: "UIKitCore", offset: 12, symbol: "", depth: 0, own: false)])
        #expect(read.wire["exception_type"] == nil && read.wire["signal"] == nil)
        #expect(CrashSignature.read(diagnostic: Data("{}".utf8), kind: .crash, ownBinary: "FarCooler") == nil)
    }

    @Test("Only the owner's own runner, once it has said so, is sent a crash")
    func onlyTheOwnersRunner() {
        #expect(CrashOutbox.mayReport(runnerTakesCrashes: true, you: ""))
        #expect(!CrashOutbox.mayReport(runnerTakesCrashes: true, you: nil), "not said yet")
        #expect(!CrashOutbox.mayReport(runnerTakesCrashes: true, you: "bob"), "a member's runner")
        #expect(!CrashOutbox.mayReport(runnerTakesCrashes: true, you: RunnerPeople.released), "released")
        #expect(!CrashOutbox.mayReport(runnerTakesCrashes: false, you: ""), "a runner without crash.report")
    }

    @Test("The outbox keeps the latest twenty, and takes one off by value")
    func theOutboxKeepsTheLatest() throws {
        let one = try Self.signature()
        var box = CrashOutbox()
        for build in 0..<25 {
            var s = one
            s.appBuild = "\(build)"
            box.add([s])
        }
        #expect(box.waiting.count == CrashOutbox.limit)
        #expect(box.waiting.first?.appBuild == "5")
        box.remove(box.waiting[0])
        #expect(box.waiting.first?.appBuild == "6")
        let kept = try JSONDecoder().decode(CrashOutbox.self, from: JSONEncoder().encode(box))
        #expect(kept == box)
    }
}
