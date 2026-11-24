import Combine
import Foundation
import Testing

@testable import Far_Cooler

/// A read the person cancelled is not a runner failing (M1, ov-157).
///
/// The runner now kills its child when the awaiting task is cancelled, and
/// over ssh that surfaces as "Killed by signal 15." A reconnect cancels the
/// bring-up read and sets `.connecting`; the cancelled read then wrote
/// `.unreachable` over it with that text.
@MainActor
struct CancelledReadTests {
    @Test func aCancelledRefreshLeavesTheStateAlone() async {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        client.commandRunnerForTesting = { _ in
            // Held until the test has cancelled the task, then the failure the
            // killed child reports.
            try? await Task.sleep(for: .milliseconds(200))
            return (nil, "Killed by signal 15.")
        }
        let read = Task { await client.refresh() }
        try? await Task.sleep(for: .milliseconds(50))
        read.cancel()
        await read.value
        #expect(client.state == .connecting, "\(client.state)")
        #expect(client.fleetError == nil)
    }

    @Test(.timeLimit(.minutes(2))) func aStalledReadReportsThatTheRunnerIsNotResponding() async {
        let client = DaemonClient(target: "stalled-runner", notifications: NotificationCenter())
        client.runnerResponseTimeout = 0.05
        let heldRead = AsyncStream.makeStream(of: Void.self)
        let states = AsyncStream.makeStream(of: HostState.self)
        let observation = client.$state.sink { state in
            if state != .connecting { states.continuation.yield(state) }
        }
        defer {
            observation.cancel()
            states.continuation.finish()
            heldRead.continuation.finish()
            client.stopEvents()
        }
        client.commandRunnerForTesting = { _ in
            var wait = heldRead.stream.makeAsyncIterator()
            _ = await wait.next()
            return (nil, nil)
        }
        let read = Task { await client.refresh() }
        var changes = states.stream.makeAsyncIterator()
        let state = await changes.next()
        #expect(state == .unreachable(reason: "Runner not responding"))
        #expect(client.fleetError == "Runner not responding")
        read.cancel()
        await read.value
    }

    @Test func aRealFailureStillSaysSo() async {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        client.commandRunnerForTesting = { _ in (nil, "ssh: connect to host box port 22: Connection refused") }
        await client.refresh()
        #expect(client.fleetError != nil)
    }
}
