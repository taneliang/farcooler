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

    @Test func aRealFailureStillSaysSo() async {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        client.commandRunnerForTesting = { _ in (nil, "ssh: connect to host box port 22: Connection refused") }
        await client.refresh()
        #expect(client.fleetError != nil)
    }

    @Test func aCancelledSendMayHaveLanded() {
        #expect(!AgentStream.sendIsKnownUnsent(CancellationError()))
    }
}
