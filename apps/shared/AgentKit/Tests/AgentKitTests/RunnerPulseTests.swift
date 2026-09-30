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

    // MARK: - Review round (ov-71 M1, M3)

    /// Only the quiet runner's working agents stop being stated as now
    /// (review M3, second round): the relay names each runner by a key only
    /// its account can make, and the watch hashes each agent's runner id
    /// (`hostRunner`, `Host.runner_id` as the phone learned it) the same way.
    /// Another runner's agents stay as they were. An agent with no runner id
    /// (a push, an old runner) or a quiet runner with no key (an old daemon)
    /// can't be told apart, so it says less rather than more. Blocked and done
    /// hold. The glance and the staleness moments follow `quietened`.
    ///
    /// Mutations: `plan` dimming every agent while anyone is quiet;
    /// `quietened` ignoring its ids; the key not lowercasing. Red.
    @Test func onlyAQuietRunnersAgentsStopBeingStated() {
        let quietId = "7537626F-0002-415E-1E11-000D48034210"
        func agent(_ id: String, _ status: String, host: String?) -> FleetSnapshot.Agent {
            var agent = FleetSnapshot.Agent(
                id: id, label: id, machine: "studio", status: status, glyph: "", headline: id,
                line: "", feed: [], rank: 0, turnFailed: false, activityChangedAt: now,
                observedAt: now)
            agent.hostRunner = host
            return agent
        }
        let fleet = FleetSnapshot(
            agents: [
                agent("quiet-work", "working", host: quietId),
                agent("quiet-ask", "blocked", host: quietId),
                agent("live-work", "working", host: "11111111-2222-3333-4444-555555555555"),
                agent("pushed", "working", host: nil),
            ],
            capturedAt: now.addingTimeInterval(-7200), complete: false)
        #expect(
            RunnerPulse.key(account: "user_1", runner: quietId)
                == "cbe66b5a6a925dacbda3748006d23728f80a570efd7f7661b032dc69eeddf883")
        let studio = RunnerPulse(
            label: "Studio", heardAgo: 3_600_000, beatEvery: 300,
            runner: RunnerPulse.key(account: "user_1", runner: quietId))
        let plan = RunnerPulse.plan(
            snapshot: fleet, reading: .answered([studio]), at: now, account: "user_1")
        #expect(plan.unstated == ["quiet-work", "pushed"])

        let shown = fleet.quietened(plan.unstated)
        #expect(shown.confidence(in: shown.agents[0], at: now) == .lastSeen)
        #expect(shown.confidence(in: shown.agents[1], at: now) == .known)
        #expect(shown.agents[2].runnerAnswering == nil)
        #expect(shown.agents[3].runnerAnswering == false)
        #expect(fleet.quietened([]) == fleet)

        // A quiet runner too old to send its id: nothing can be told apart.
        let old = RunnerPulse(label: "Studio", heardAgo: 3_600_000, beatEvery: 300)
        let blanket = RunnerPulse.plan(
            snapshot: fleet, reading: .answered([old]), at: now, account: "user_1")
        #expect(blanket.unstated == ["quiet-work", "live-work", "pushed"])
        // Nobody quiet, nobody unstated.
        let live = RunnerPulse(label: "Studio", heardAgo: 1_000, beatEvery: 300, runner: "k")
        #expect(
            RunnerPulse.plan(snapshot: fleet, reading: .answered([live]), at: now, account: "user_1")
                .unstated.isEmpty)
    }

    /// A push names no runner id, so the agent keeps the one the app wrote:
    /// otherwise every alert would make its agent "can't tell apart".
    ///
    /// Mutation: `merging` not carrying `hostRunner`. Red.
    @Test func aPushKeepsItsAgentsRunnerId() {
        var polled = FleetSnapshot.Agent(
            id: "a", label: "a", machine: "studio", status: "working", glyph: "", headline: "a",
            line: "", feed: [], rank: 0, turnFailed: false, activityChangedAt: now)
        polled.hostRunner = "h"
        let fleet = FleetSnapshot(agents: [polled], capturedAt: now, complete: true)
        var pushed = polled
        pushed.hostRunner = nil
        pushed.status = "blocked"
        #expect(fleet.merging(pushed, at: now).agents.first?.hostRunner == "h")
    }

    /// A credential the watch holds but can't read yet (the file is
    /// protected until the watch unlocks) is a look that failed, not a
    /// watch with no credential: it looks again rather than parking.
    ///
    /// Mutation: `look` ignoring `held`. Red.
    @Test func aCredentialTheWatchCantReadYetLooksAgain() async {
        let old = snapshot(complete: false, age: 7200)
        let locked = await RunnerPulse.look(snapshot: old, credential: nil, held: true, at: now)
        #expect(locked == RunnerPulse.Plan(quiet: [], nextLook: now.addingTimeInterval(30 * 60)))
    }

    /// Every change to the phone's credential is announced, so the watch
    /// link sends the watch a context with the new one, or without one,
    /// at once rather than at its next poll. The same token again isn't.
    ///
    /// Mutation: `token` not posting for a new token. Red.
    @Test func aNewCredentialIsAnnounced() throws {
        let vault = MemoryVault()
        let counter = Counter()
        let observer = NotificationCenter.default.addObserver(
            forName: PulseStore.changed, object: nil, queue: nil) { _ in counter.bump() }
        defer { NotificationCenter.default.removeObserver(observer) }
        _ = try #require(PulseStore.token(relay: "https://a", account: "u1", in: vault))
        #expect(counter.value == 1)
        _ = PulseStore.token(relay: "https://a", account: "u1", in: vault)
        #expect(counter.value == 1)
        _ = PulseStore.token(relay: "https://a", account: "u2", in: vault)
        #expect(counter.value == 2)
        PulseStore.clear()
        #expect(counter.value == 3)
    }

    /// Sign-out tells the relay to forget the pulse token beside the refresh
    /// token, so the watch's copy reads nothing after (M1).
    ///
    /// Mutation: `logoutBody` leaving the pulse token out. Red.
    @Test func signOutSendsThePulseTokenToForget() {
        let body = Account.logoutBody(refresh: "rt", pulse: "p")
        #expect(body?["refreshToken"] as? String == "rt")
        #expect(body?["pulseToken"] as? String == "p")
        #expect(Account.logoutBody(refresh: nil, pulse: "p")?["pulseToken"] as? String == "p")
        #expect(Account.logoutBody(refresh: nil, pulse: nil) == nil)
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

/// A count a notification observer can bump from any thread.
final class Counter: @unchecked Sendable {
    private let lock = NSLock()
    private var count = 0
    var value: Int { lock.withLock { count } }
    func bump() { lock.withLock { count += 1 } }
}
