import Foundation
import Testing

@testable import Far_Cooler

/// A runner paired before the tunnel gets this Mac's key on its line, once,
/// over the access the Mac already has (ov-485, `client.set_node_key`).
struct NodeKeyRegistrationTests {
    final class Calls: @unchecked Sendable {
        private let lock = NSLock()
        private var all: [[String]] = []
        var list: [[String]] { lock.withLock { all } }
        func add(_ arguments: [String]) { lock.withLock { all.append(arguments) } }
    }

    static let key = "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"

    private func registration(
        _ calls: Calls, ok: Bool = true, key: String? = NodeKeyRegistrationTests.key,
        tunneled: Bool = false, remembered: RegisteredKeys = RegisteredKeys(defaultsKey: nil)
    ) -> NodeKeyRegistration {
        var registration = NodeKeyRegistration()
        registration.key = { key }
        registration.run = { calls.add($0); return (ok, ok ? #"{"connBlob":""}"# : "no tunnel") }
        registration.tunneled = { _ in tunneled }
        registration.remembered = remembered
        return registration
    }

    /// The call, exactly: this Mac's own key, on the runner's own connection,
    /// with no client id to name anybody else's line.
    @Test func aDirectRunnerIsGivenThisMacsKey() async {
        let calls = Calls()
        let outcome = await registration(calls).register("you@box")
        #expect(outcome == .registered)
        #expect(calls.list == [["--json", "--runner", "you@box", "client", "set-node-key", "--node-key", Self.key]])
    }

    /// Once per runner and key: a second reconnect asks nothing.
    @Test func aRunnerThatSaidYesIsNotAskedAgain() async {
        let calls = Calls()
        let kept = RegisteredKeys(defaultsKey: nil)
        _ = await registration(calls, remembered: kept).register("you@box")
        let again = await registration(calls, remembered: kept).register("you@box")
        #expect(again == .alreadyRegistered)
        #expect(calls.list.count == 1)
    }

    /// A runner that said no is tried once per launch, not on every reconnect.
    @Test func aRunnerThatSaidNoIsTriedOncePerLaunch() async {
        let calls = Calls()
        let kept = RegisteredKeys(defaultsKey: nil)
        #expect(await registration(calls, ok: false, remembered: kept).register("you@box") == .refused)
        #expect(await registration(calls, ok: false, remembered: kept).register("you@box") == .alreadyRegistered)
        #expect(calls.list.count == 1)
    }

    /// Nothing for this Mac, a tunneled runner (its grant carried the key), or
    /// a Mac with no key to give.
    @Test func nothingIsSentWhereThereIsNothingToAdd() async {
        let calls = Calls()
        #expect(await registration(calls).register("") == .skipped)
        #expect(await registration(calls, tunneled: true).register("fc-cosmo") == .skipped)
        #expect(await registration(calls, key: nil).register("you@box") == .skipped)
        #expect(calls.list.isEmpty)
    }
}
