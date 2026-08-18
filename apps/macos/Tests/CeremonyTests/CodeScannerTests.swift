import Foundation
import Testing

@testable import Far_Cooler

struct CodeScannerTests {
    @Test func aQueuedStopFinishesBeforeAnImmediateRestart() async {
        let queue = CaptureSessionQueue(label: "com.farcooler.scanner.tests")
        let probe = OverlapProbe()

        queue.enqueue { probe.run("stop") }
        await queue.perform { probe.run("start") }

        #expect(probe.maximumConcurrentCalls == 1)
        #expect(probe.completedCalls == ["stop", "start"])
    }
}

private final class OverlapProbe: @unchecked Sendable {
    private let lock = NSLock()
    private var activeCalls = 0
    private(set) var maximumConcurrentCalls = 0
    private(set) var completedCalls: [String] = []

    func run(_ name: String) {
        lock.withLock {
            activeCalls += 1
            maximumConcurrentCalls = max(maximumConcurrentCalls, activeCalls)
        }
        Thread.sleep(forTimeInterval: 0.03)
        lock.withLock {
            activeCalls -= 1
            completedCalls.append(name)
        }
    }
}
