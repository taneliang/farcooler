import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// The reply is measured before a key is written (ov-485 fix 1, H1): a grant
/// too large for one code is refused with nothing written anywhere, and a
/// fleet of seven that grows to tunnels with addresses still pairs.
@MainActor
struct ReplyFitsFirstTests {
    static let keyA = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIB1iLbeqDzK4CDeUC3t+ffVPDI9Gk+sBwIZqJZW1NfS5 device-a"
    static let nodeKey = "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"

    final class Writes: @unchecked Sendable {
        var calls = 0
    }

    private func store(granting count: Int) throws -> CeremonyStore {
        let code = try #require(
            CeremonyFFI.offer(name: "iPhone", account: "acct_1", keyA: Self.keyA, keyB: nil, nodeKey: Self.nodeKey))
        let store = CeremonyStore()
        store.authenticate = { _ in true }
        let rows = (0..<count).map { i in
            CeremonyStore.RunnerRow(
                runner: CeremonyRunner(
                    id: "you@r\(i)", label: "r\(i)", alias: "r\(i)", user: "you",
                    host_key: "SHA256:iDqoxaySm9gzxtvLrNXXpM5PimPLeBknaaNj0Rg7vz4",
                    reach: .direct(host: "r\(i).tail-1234.ts.net", port: 22), pending: true),
                granted: true, note: nil)
        }
        store.scan(code, at: Date(), account: "acct_1", email: "a@b", runners: rows)
        return store
    }

    /// Too many for one code even at their addresses: refused, and the
    /// enrollment that writes keys and starts tunnels never runs.
    @Test func aGrantTooLargeForOneCodeWritesNothing() async throws {
        let store = try store(granting: 9)
        let writes = Writes()
        await store.confirm(reason: "test") { runners in
            writes.calls += 1
            return Enrollment.Outcome(written: Set(runners.map(\.id)))
        }
        #expect(writes.calls == 0, "keys were written before the reply was measured")
        guard case .refused(.tooLarge) = store.phase else {
            Issue.record("not refused as too large: \(store.phase)")
            return
        }
    }

    /// Seven runners that all answer with a token pair: the reply gives up
    /// what it must and keeps every runner.
    @Test func sevenRunnersThatAllAnswerWithATokenStillPair() async throws {
        let store = try store(granting: 7)
        let token = String(repeating: "t", count: 184)
        await store.confirm(reason: "test") { runners in
            var outcome = Enrollment.Outcome(written: Set(runners.map(\.id)))
            for runner in runners { outcome.tunneled[runner.id] = token }
            return outcome
        }
        guard case .showingManifest(let reply) = store.phase else {
            Issue.record("no reply: \(store.phase)")
            return
        }
        for i in 0..<7 { #expect(reply.contains(#""label":"r\#(i)""#), "r\(i) was dropped") }

        // Never silent (N1): the runners given only their address are named
        // on the finish screen, and the confirm screen predicted the same ones.
        let fellBack = CeremonyStore.addressOnly(granted: granted(7, token: token), reply: reply)
        #expect(!fellBack.isEmpty, "nothing fell back, so this tests nothing")
        #expect(store.addressOnlyNote == TunnelRoute.addressOnlySentence(fellBack.map(\.label)))
        let predicted = try predicted(7)
        #expect(predicted == Set(fellBack.map(\.id)), "the confirm screen warned about different runners")
    }

    private func granted(_ count: Int, token: String) -> [CeremonyRunner] {
        (0..<count).map { i in
            CeremonyRunner(
                id: "you@r\(i)", label: "r\(i)", alias: "r\(i)", user: "you",
                host_key: "SHA256:iDqoxaySm9gzxtvLrNXXpM5PimPLeBknaaNj0Rg7vz4",
                reach: .tailcat(token: token), pending: false,
                alternative: .direct(host: "r\(i).tail-1234.ts.net", port: 22))
        }
    }

    private func predicted(_ count: Int) throws -> Set<String> {
        let store = try store(granting: count)
        guard case .confirming(let confirmation) = store.phase else { return [] }
        return CeremonyStore.predictedAddressOnly(
            offer: confirmation.offerJSON, rows: confirmation.rows, budget: codeBudgetBytes)
    }
}
