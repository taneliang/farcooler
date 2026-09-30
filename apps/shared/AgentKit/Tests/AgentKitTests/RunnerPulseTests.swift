import Foundation
import Testing

@testable import AgentKit

/// Telling a live runner from a stale one while the app is suspended (ov-53).
///
/// The relay reports how long ago each beating runner was heard; these are
/// the phone's half: when that's quiet, and what the widget says about it.
///
/// Serialized because two tests answer through `StubbedRelay.answer`, one
/// static, and two at once would each read the other's answer.
@Suite(.serialized)
struct RunnerPulseTests {
    /// Two missed beats and five minutes' slack: fifteen minutes at the
    /// shipped five-minute beat. Just inside is live; just past is quiet.
    ///
    /// Mutation: `quietAfter` returning `beatEvery` alone. Red.
    @Test func aRunnerIsQuietPastTwoMissedBeatsAndSlack() {
        #expect(RunnerPulse.quietAfter(beatEvery: 300) == 15 * 60)
        #expect(!RunnerPulse(label: "Studio", heardAgo: 14 * 60_000, beatEvery: 300).isQuiet)
        #expect(RunnerPulse(label: "Studio", heardAgo: 16 * 60_000, beatEvery: 300).isQuiet)
        // Judged by its own promise, not the shipped one.
        #expect(!RunnerPulse(label: "Slow", heardAgo: 16 * 60_000, beatEvery: 600).isQuiet)
    }

    /// Only the quiet ones are named, once each, in the relay's order.
    @Test func onlyQuietRunnersAreNamedOnce() {
        let pulses = [
            RunnerPulse(label: "This Mac", heardAgo: 3_600_000, beatEvery: 300),
            RunnerPulse(label: "Studio", heardAgo: 1_000, beatEvery: 300),
            RunnerPulse(label: "This Mac", heardAgo: 7_200_000, beatEvery: 300),
            RunnerPulse(label: "Attic", heardAgo: 3_600_000, beatEvery: 300),
        ]
        #expect(RunnerPulse.quiet(pulses) == ["This Mac", "Attic"])
    }

    /// The relay's answer decodes; anything else is nil, never a throw.
    @Test func theRelaysAnswerDecodes() {
        let json = #"{"runners":[{"label":"Studio","heardAgo":1200,"beatEvery":300}]}"#
        #expect(
            RunnerPulse.decode(Data(json.utf8))
                == [RunnerPulse(label: "Studio", heardAgo: 1200, beatEvery: 300)])
        #expect(RunnerPulse.decode(Data(#"{"error":"unauthorized"}"#.utf8)) == nil)
    }

    /// A quiet runner gets ov-50's words, and outranks "from notifications".
    /// Nothing quiet is exactly today's hedge.
    ///
    /// Mutation: `hedge(quiet:)` ignoring `quiet`. Red.
    @Test func aQuietRunnerIsLostTouch() {
        let partial = FleetSnapshot(
            agents: [], capturedAt: Date(timeIntervalSince1970: 1), complete: false)
        #expect(partial.hedge(quiet: ["Studio"]) == .lostTouch(["Studio"]))
        #expect(partial.hedge(quiet: ["Studio"])?.footer == "lost touch with Studio")
        #expect(partial.hedge(quiet: []) == .fromNotifications)
        #expect(partial.hedge(quiet: []) == partial.hedge)

        let whole = FleetSnapshot(
            agents: [], capturedAt: Date(timeIntervalSince1970: 1), complete: true)
        #expect(whole.hedge(quiet: []) == nil)
        #expect(whole.hedge(quiet: ["Studio"]) == .lostTouch(["Studio"]))
    }

    /// A runner the app lost touch with AND the relay calls quiet is named
    /// once, the app's word first.
    @Test func aRunnerLostAndQuietIsNamedOnce() {
        let lost = FleetSnapshot(
            agents: [], capturedAt: Date(timeIntervalSince1970: 1), complete: false,
            lostRunners: ["Orchard"])
        #expect(lost.hedge(quiet: ["Studio", "Orchard"]) == .lostTouch(["Orchard", "Studio"]))
    }

    // MARK: - The runner's own name (review C3)

    /// A runner's own name beats its pairing label, which is "This Mac" for
    /// every Mac and names nothing on a phone.
    ///
    /// Mutation: `displayName` returning `label`. Red.
    @Test func aQuietRunnerIsNamedByItsOwnName() {
        let pulses = [
            RunnerPulse(label: "This Mac", name: "Studio", heardAgo: 3_600_000, beatEvery: 300),
            RunnerPulse(label: "orchard", name: nil, heardAgo: 3_600_000, beatEvery: 300),
        ]
        #expect(RunnerPulse.quiet(pulses) == ["Studio", "orchard"])
        let json = #"{"runners":[{"label":"This Mac","name":"Studio","heardAgo":1,"beatEvery":300}]}"#
        #expect(RunnerPulse.decode(Data(json.utf8))?.first?.displayName == "Studio")
    }

    // MARK: - The widget's plan (review C4, C6)

    private let now = Date(timeIntervalSince1970: 100_000)
    private let lost = RunnerPulse(label: "Studio", heardAgo: 3_600_000, beatEvery: 300)

    private func snapshot(complete: Bool, age: TimeInterval) -> FleetSnapshot {
        FleetSnapshot(agents: [], capturedAt: now.addingTimeInterval(-age), complete: complete)
    }

    /// The widget looks again only while some runner beats, or after a fetch
    /// that failed; otherwise it keeps `.never` and the budget.
    ///
    /// Mutations: `.failed` answering no look (a failed fetch parks the
    /// widget); an empty answer asking for a look. Red.
    @Test func theWidgetLooksAgainWhileARunnerBeatsOrAfterAFailure() {
        let old = snapshot(complete: false, age: 7200)
        let later = now.addingTimeInterval(30 * 60)
        #expect(RunnerPulse.plan(snapshot: old, reading: .noCredential, at: now).nextLook == nil)
        #expect(RunnerPulse.plan(snapshot: old, reading: .refused, at: now).nextLook == nil)
        #expect(RunnerPulse.plan(snapshot: old, reading: .answered([]), at: now).nextLook == nil)
        #expect(RunnerPulse.plan(snapshot: old, reading: .failed, at: now).nextLook == later)
        #expect(RunnerPulse.plan(snapshot: old, reading: .answered([lost]), at: now).nextLook == later)
        #expect(RunnerPulse.plan(snapshot: old, reading: .failed, at: now).quiet == [])
    }

    /// A fresh, complete app snapshot wins over the relay: the app heard its
    /// runners over its own links more recently than one could go quiet.
    /// An old or partial one doesn't.
    ///
    /// Mutation: `plan` ignoring the snapshot. Red.
    @Test func aFreshCompleteAppSnapshotWinsOverTheRelay() {
        let fresh = snapshot(complete: true, age: 60)
        #expect(RunnerPulse.plan(snapshot: fresh, reading: .answered([lost]), at: now).quiet == [])
        let old = snapshot(complete: true, age: 3600)
        #expect(RunnerPulse.plan(snapshot: old, reading: .answered([lost]), at: now).quiet == ["Studio"])
        let partial = snapshot(complete: false, age: 60)
        #expect(
            RunnerPulse.plan(snapshot: partial, reading: .answered([lost]), at: now).quiet == ["Studio"])
    }

    // MARK: - Asking the relay

    /// 200 decodes, 401 is refused (asking again won't help), anything else
    /// failed (it might).
    @Test func aFetchTellsRefusedFromFailed() async {
        let credential = PulseCredential(relay: "https://pulse.test", token: "t", account: "u")
        let session = StubbedRelay.session()
        StubbedRelay.answer = (200, #"{"runners":[{"label":"Studio","heardAgo":1,"beatEvery":300}]}"#)
        #expect(
            await RunnerPulse.fetch(credential, session: session)
                == .answered([RunnerPulse(label: "Studio", heardAgo: 1, beatEvery: 300)]))
        #expect(StubbedRelay.lastAuthorization == "Bearer t")
        StubbedRelay.answer = (401, #"{"error":"unauthorized"}"#)
        #expect(await RunnerPulse.fetch(credential, session: session) == .refused)
        StubbedRelay.answer = (404, #"{"error":"not found"}"#)
        #expect(await RunnerPulse.fetch(credential, session: session) == .failed)
    }

    // MARK: - The phone's token (review C1, S1)

    /// Two registrations in a row send the same token, so neither strands the
    /// widget whichever the relay sees last. A new relay setting keeps it.
    ///
    /// Mutation: `token` making a new one every call. Red.
    @Test func twoRegistrationsInARowSendTheSameToken() throws {
        let vault = MemoryVault()
        let first = try #require(PulseStore.token(relay: "https://a", account: "u1", in: vault))
        let second = try #require(PulseStore.token(relay: "https://a", account: "u1", in: vault))
        #expect(first == second)
        #expect(first.count == 64 && first.allSatisfy(\.isHexDigit))
        let moved = PulseStore.token(relay: "https://b", account: "u1", in: vault)
        #expect(moved == first)
        #expect(PulseStore.read(from: vault)?.relay == "https://b")
        #expect(PulseStore.read(from: vault)?.token == first)
    }

    /// Another account gets another token, and clearing forgets it.
    ///
    /// Mutation: `token` ignoring `account`. Red.
    @Test func anotherAccountGetsAnotherToken() throws {
        let vault = MemoryVault()
        let mine = try #require(PulseStore.token(relay: "https://a", account: "u1", in: vault))
        let theirs = try #require(PulseStore.token(relay: "https://a", account: "u2", in: vault))
        #expect(mine != theirs)
        vault.delete()
        #expect(PulseStore.read(from: vault) == nil)
    }

    // MARK: - The watch (ov-71)

    /// The phone files its credential in the watch's application context
    /// under one key, and the watch reads it back; a context with none, or
    /// one it can't read, carries none.
    ///
    /// Mutation: `carried(in:)` reading another key. Red.
    @Test func aWatchContextCarriesTheCredentialOrNothing() throws {
        let credential = PulseCredential(relay: "https://pulse.test", token: "t", account: "u")
        let context: [String: Any] = [
            "snapshot": Data(), PulseCredential.watchContextKey: try #require(credential.contextValue),
        ]
        #expect(PulseCredential.carried(in: context) == credential)
        #expect(PulseCredential.carried(in: ["snapshot": Data()]) == nil)
        #expect(PulseCredential.carried(in: [PulseCredential.watchContextKey: Data("x".utf8)]) == nil)
    }

    /// The watch keeps what the phone last sent: a new credential is filed, the
    /// same one again is no change, and a context without one (the phone
    /// signed out) forgets it. The answer is whether anything changed, which
    /// is when the complication is worth reloading.
    ///
    /// Mutations: `adopt` never deleting; `adopt` reporting a change for the
    /// same credential. Red.
    @Test func theWatchKeepsWhatThePhoneLastSent() {
        let vault = MemoryVault()
        let credential = PulseCredential(relay: "https://pulse.test", token: "t", account: "u")
        #expect(PulseStore.adopt(credential, in: vault))
        #expect(PulseStore.read(from: vault) == credential)
        #expect(!PulseStore.adopt(credential, in: vault))
        #expect(PulseStore.adopt(nil, in: vault))
        #expect(PulseStore.read(from: vault) == nil)
        #expect(!PulseStore.adopt(nil, in: vault))
    }

    /// The watch's vault is a file in its own App Group container, which the
    /// watch app writes and the complication reads.
    ///
    /// Mutation: `delete` leaving the file. Red.
    @Test func aContainerVaultKeepsAndForgets() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("pulse-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let vault = ContainerPulseVault(container: directory)
        #expect(vault.read() == nil)
        #expect(vault.write(Data("one".utf8)))
        #expect(vault.write(Data("two".utf8)))
        #expect(vault.read() == Data("two".utf8))
        vault.delete()
        #expect(vault.read() == nil)
    }

    /// A surface may look at its own pace: the watch's complication spends a
    /// tighter budget than the phone's widget.
    ///
    /// Mutation: `plan` ignoring `every`. Red.
    @Test func aSurfaceLooksAgainAtItsOwnPace() {
        let old = snapshot(complete: false, age: 7200)
        let hour = now.addingTimeInterval(60 * 60)
        #expect(
            RunnerPulse.plan(snapshot: old, reading: .answered([lost]), at: now, every: 3600).nextLook
                == hour)
        #expect(RunnerPulse.plan(snapshot: old, reading: .failed, at: now, every: 3600).nextLook == hour)
    }

    /// `look` is the whole of what a surface does: no credential is no
    /// fetch and today's hedge; a credential asks the relay and plans.
    ///
    /// Mutation: `look` planning `.failed` without a credential. Red.
    @Test func aLookWithNoCredentialAsksNothing() async {
        let old = snapshot(complete: false, age: 7200)
        let none = await RunnerPulse.look(snapshot: old, credential: nil, at: now)
        #expect(none == RunnerPulse.Plan(quiet: [], nextLook: nil))

        let credential = PulseCredential(relay: "https://pulse.test", token: "t", account: "u")
        StubbedRelay.answer = (200, #"{"runners":[{"label":"Studio","heardAgo":3600000,"beatEvery":300}]}"#)
        let asked = await RunnerPulse.look(
            snapshot: old, credential: credential, at: now, session: StubbedRelay.session())
        #expect(asked.quiet == ["Studio"])
    }
}

/// A vault in memory, for tests with no Keychain group.
final class MemoryVault: PulseVault, @unchecked Sendable {
    private let lock = NSLock()
    private var data: Data?
    func read() -> Data? { lock.withLock { data } }
    @discardableResult func write(_ data: Data) -> Bool { lock.withLock { self.data = data }; return true }
    func delete() { lock.withLock { data = nil } }
}

/// A relay that answers what the test last said, through `URLProtocol`.
final class StubbedRelay: URLProtocol, @unchecked Sendable {
    nonisolated(unsafe) static var answer: (Int, String) = (200, "{}")
    nonisolated(unsafe) static var lastAuthorization: String?

    static func session() -> URLSession {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [StubbedRelay.self]
        return URLSession(configuration: configuration)
    }

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        Self.lastAuthorization = request.value(forHTTPHeaderField: "Authorization")
        let (status, body) = Self.answer
        let response = HTTPURLResponse(
            url: request.url!, statusCode: status, httpVersion: nil, headerFields: nil)!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data(body.utf8))
        client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading() {}
}
