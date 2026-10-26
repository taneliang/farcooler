import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// Adding a teammate's device by QR (ov-525, ruling R-65): a code from another
/// account goes to the teammate check, is refused when the directory can't
/// vouch for it, and is enrolled at the scope chosen, bound to the teammate on
/// every runner that took it.
@MainActor
struct TeammateCeremonyTests {
    static let keyA = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIB1iLbeqDzK4CDeUC3t+ffVPDI9Gk+sBwIZqJZW1NfS5 device-a"
    static let keyB = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDMdwe233CUbxjpEHkissIUGdCxhkTsDE/Zg7f+LB6S+ device-b"

    /// The relay's answers: Alice is an admin of Platform, and Bob is in it.
    static func directory(bobIn: Bool = true, withCarol: Bool = false) -> Data {
        let members: [[String: Any]] =
            [["user": "user_alice", "email": "alice@example.com", "role": "admin"]]
            + (bobIn ? [["user": "user_bob", "email": "bob.smith@example.com", "role": "member"]] : [])
            + (withCarol ? [["user": "user_carol", "email": "carol@example.com", "role": "member"]] : [])
        let object: [String: Any] = [
            "me": [
                "user": ["id": "user_alice", "email": "alice@example.com"],
                "teams": [["id": "t-platform", "name": "Platform", "personal": false, "role": "admin"]],
            ],
            "rosters": [["team": ["id": "t-platform", "name": "Platform", "personal": false], "members": members]],
        ]
        return try! JSONSerialization.data(withJSONObject: object)
    }

    static let row = CeremonyStore.RunnerRow(
        runner: CeremonyRunner(
            id: "alice@build-box", label: "build-box", alias: "build-box", user: "alice",
            host_key: "SHA256:iDqoxaySm9gzxtvLrNXXpM5PimPLeBknaaNj0Rg7vz4",
            reach: .direct(host: "build-box.example.com", port: 22), pending: true),
        granted: true, note: nil)

    final class Writes: @unchecked Sendable {
        var calls = 0
    }

    /// Scan Bob's code on Alice's Mac, with the directory answering `directory`.
    private func scanned(keyB: String? = nil, directory: Data?) async throws -> CeremonyStore {
        let code = try #require(
            CeremonyFFI.offer(name: "Bob’s iPhone", account: "user_bob", keyA: Self.keyA, keyB: keyB))
        let store = CeremonyStore()
        store.authenticate = { _ in true }
        store.teamDirectory = { directory }
        store.scan(code, at: Date(), account: "user_alice", email: "alice@example.com", runners: [Self.row])
        #expect(store.phase == .checkingTeam, "nothing about another account's code is shown before the check")
        await store.teamCheck?.value
        return store
    }

    @Test func aTeammatesCodeOpensTheirConfirmationWithTheDirectorysEmail() async throws {
        let store = try await scanned(directory: Self.directory())
        guard case .confirming(let confirmation) = store.phase, let teammate = confirmation.teammate else {
            Issue.record("not confirming a teammate: \(store.phase)")
            return
        }
        #expect(teammate.who == CeremonyTeammate(account: "user_bob", email: "bob.smith@example.com", team: "Platform"))
        #expect((teammate.handle, teammate.name, teammate.scope) == ("bob-smith", "Bob Smith", .control))
        #expect(teammate.grantedBy == "alice@example.com")
        #expect(confirmation.shell == nil)
        // Nothing is granted to a teammate by default, this Mac included,
        // though the rows arrived checked (review F1).
        #expect(Self.row.granted && confirmation.rows.allSatisfy { !$0.granted })
    }

    @Test func anUnreachableDirectoryRefusesAndWritesNothing() async throws {
        let store = try await scanned(directory: nil)
        #expect(store.phase == .refused(.directoryUnreachable))
        // And the sheet's own confirm does nothing from there.
        let writes = Writes()
        await store.confirm(reason: "test") { _ in
            writes.calls += 1
            return Enrollment.Outcome()
        }
        #expect(writes.calls == 0)
    }

    @Test func anAccountOnNoneOfYourTeamsIsRefusedWithTheWayIn() async throws {
        let store = try await scanned(directory: Self.directory(bobIn: false))
        #expect(store.phase == .refused(.notATeammate("alice@example.com")))
        let refusal = Refusal.notATeammate("alice@example.com")
        #expect(refusal.detail.contains("sign in to alice@example.com"))
        // The body leads with the way in rather than saying the title again.
        #expect(!refusal.detail.contains("isn’t on any of your teams"))
    }

    @Test func aTeammatesMacAskingForAShellIsRefused() async throws {
        let store = try await scanned(keyB: Self.keyB, directory: Self.directory())
        #expect(store.phase == .refused(.teammateShell))
    }

    /// A device the directory doesn't list is told plainly that their account
    /// hasn't confirmed it, and given the next step now that certifying in
    /// person exists (review L2, ov-550): sign in on it, then scan it from a
    /// confirmed device, or, for a first device (`keys: []`), signing in is
    /// enough. Never calls the scanned device "this", which on the admin's
    /// Mac reads as the Mac.
    @Test func anUnlistedDeviceIsToldItsAccountHasNotConfirmedIt() {
        let refusal = Refusal.unregisteredKey
        #expect(refusal.title == "Their account hasn’t confirmed that device")
        for words in [refusal.title, refusal.detail] {
            #expect(!words.contains("This device") && !words.contains("this device"), "\(words)")
        }
        #expect(refusal.detail.contains("their account has confirmed"))
        #expect(refusal.detail.contains("sign in to Far Cooler on it"))
        #expect(refusal.detail.contains("scan its code from another of their devices that’s already confirmed"))
        #expect(refusal.detail.contains("If it’s their first device, signing in is enough."))
        #expect(refusal.detail.hasSuffix("Then scan its code here again."))
    }

    /// Confirmed, the reply names who granted it, and a handle the runner
    /// would refuse stops the tap before anything is written.
    @Test func theReplyNamesWhoGrantedItAndABadHandleWritesNothing() async throws {
        let store = try await scanned(directory: Self.directory())
        store.setGrant(true, for: Self.row.id)
        store.setTeammate { $0.handle = "Bob Smith" }
        let writes = Writes()
        await store.confirm(reason: "test") { runners in
            writes.calls += 1
            return Enrollment.Outcome(written: Set(runners.map(\.id)))
        }
        #expect(writes.calls == 0, "a handle no runner takes was written")

        store.setTeammate { $0.handle = "bob" }
        await store.confirm(reason: "test") { runners in
            writes.calls += 1
            return Enrollment.Outcome(written: Set(runners.map(\.id)))
        }
        #expect(writes.calls == 1)
        guard case .showingManifest(let reply) = store.phase else {
            Issue.record("no reply: \(store.phase)")
            return
        }
        let manifest = try JSONDecoder().decode(CeremonyManifest.self, from: Data(reply.utf8))
        #expect(manifest.granted_by == "alice@example.com")
        #expect(manifest.account == "user_bob", "the reply is addressed to Bob's own account")
    }

    // MARK: - Enrolling and binding

    /// A runner whose CLI answers from a script and records every call.
    final class Runner: @unchecked Sendable {
        var listed = #"{"members":[],"claude_account":"","you":""}"#
        /// What `client enroll` answers.
        var enrolls: [(ok: Bool, output: String)] = [(true, #"{"client":{"scope":"control"},"alreadyEnrolled":false,"connBlob":""}"#)]
        var addFails = false
        var revokeFails = false
        var listFails = false
        /// A runner from before member accounts: its list doesn't say it keeps them.
        var old = false
        /// Whether its list says it checks what a bind read (ov-786).
        var checksRead = false
        /// What each later `member list` answers, in turn, once the first has.
        var relisted: [String] = []
        /// What each `member add` answers, in turn, before `addFails` decides.
        var adds: [(ok: Bool, output: String)] = []
        private(set) var calls: [[String]] = []

        func run(_ arguments: [String]) async -> (ok: Bool, output: String) {
            calls.append(arguments)
            if arguments.contains("list") {
                if listFails { return (false, "Could not reach build-box over ssh.") }
                if calls.filter({ $0.contains("list") }).count > 1, !relisted.isEmpty { listed = relisted.removeFirst() }
                var said = old ? listed : String(listed.dropLast()) + #","member_accounts":true}"#
                if checksRead { said = String(said.dropLast()) + #","member_devices_read":true}"# }
                return (true, "Welcome to build-box\n" + said)
            }
            if arguments.contains("enroll") { return enrolls.count > 1 ? enrolls.removeFirst() : enrolls[0] }
            if arguments.contains("add"), !adds.isEmpty { return adds.removeFirst() }
            if arguments.contains("add") { return addFails ? (false, "error: handle") : (true, "{}") }
            if arguments.contains("revoke") { return revokeFails ? (false, "Could not reach build-box over ssh.") : (true, "{}") }
            return (true, "{}")
        }

        func call(_ verb: String) -> [String]? { calls.first { $0.contains(verb) } }
        func verbs() -> [String] {
            calls.map { call in ["list", "enroll", "add", "revoke"].first(where: call.contains) ?? "?" }
        }
    }

    /// A journal in a temporary folder of its own, removed by the test.
    static func journal() -> (TeammateJournal, URL) {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent("teammate-journal-\(UUID().uuidString)")
        return (TeammateJournal(file: folder.appendingPathComponent("journal.json")), folder)
    }

    static let bob = TeammateEnrollment.Member(
        handle: "bob", name: "Bob Smith", account: "user_bob", email: "bob@example.com")
    static let runner = row.runner

    func enroll(
        _ runner: Runner, on runners: [CeremonyRunner] = [TeammateCeremonyTests.runner], scope: TeammateScope = .control,
        journal: TeammateJournal
    ) async -> Enrollment.Outcome {
        await TeammateEnrollment.enroll(
            keyA: Self.keyA, label: "Bob’s iPhone", clientID: "farcooler-0a0b", scope: scope, nodeKey: "",
            member: Self.bob, on: runners, journal: journal, using: { await runner.run($0) })
    }

    @Test func aTeammateIsEnrolledAtTheirScopeWithNoShellAndBound() async throws {
        let (journal, folder) = Self.journal()
        defer { try? FileManager.default.removeItem(at: folder) }
        let runner = Runner()
        let outcome = await enroll(runner, scope: .read, journal: journal)
        #expect(outcome.written == [Self.runner.id])
        let enroll = try #require(runner.call("enroll"))
        #expect(enroll.contains("read") && !enroll.contains("--shell-access"))
        #expect(runner.calls.filter { $0.contains("enroll") }.count == 1, "one key, never Key B")
        let add = try #require(runner.call("add"))
        #expect(
            Array(add.drop(while: { $0 != "member" }))
                == [
                    "member", "add", "bob", "--name", "Bob Smith", "--device", "farcooler-0a0b",
                    "--account", "user_bob", "--email", "bob@example.com",
                ])
        #expect(runner.call("revoke") == nil)
        #expect(journal.entries.isEmpty, "a bound key stays noted")
    }

    /// One runner at a time: each is read, written and bound before the next
    /// is touched, so only one runner's key is ever unbound (review F2).
    @Test func eachRunnerIsBoundBeforeTheNextIsWritten() async throws {
        let (journal, folder) = Self.journal()
        defer { try? FileManager.default.removeItem(at: folder) }
        let runner = Runner()
        var second = Self.runner
        second.id = "alice@staging"
        _ = await enroll(runner, on: [Self.runner, second], journal: journal)
        #expect(runner.verbs() == ["list", "enroll", "add", "list", "enroll", "add"])
    }

    /// A write reported as failed may have landed (ssh dropped after the
    /// daemon wrote it): the key is taken off anyway (review F2).
    @Test func aWriteThatFailedIsTakenOffAnyway() async throws {
        let (journal, folder) = Self.journal()
        defer { try? FileManager.default.removeItem(at: folder) }
        let runner = Runner()
        runner.enrolls = [(false, "Connection to build-box closed by remote host.")]
        let outcome = await enroll(runner, journal: journal)
        #expect(outcome.written.isEmpty)
        let revoke = try #require(runner.call("revoke"), "a write that may have landed was left")
        #expect(Array(revoke.suffix(3)) == ["client", "revoke", "farcooler-0a0b"])
        #expect(runner.call("add") == nil)
        #expect(journal.entries.isEmpty)
        #expect(outcome.transcript?.contains(TeammateEnrollment.takenOff) == true)
    }

    /// A key that can't be recorded as Bob's would be the owner's there, so
    /// it's taken off again and the runner goes pending.
    @Test func aKeyThatCantBeRecordedIsTakenOffAgain() async throws {
        let (journal, folder) = Self.journal()
        defer { try? FileManager.default.removeItem(at: folder) }
        let runner = Runner()
        runner.addFails = true
        let outcome = await enroll(runner, journal: journal)
        #expect(outcome.written.isEmpty)
        let revoke = try #require(runner.call("revoke"))
        #expect(Array(revoke.suffix(3)) == ["client", "revoke", "farcooler-0a0b"])
        #expect(outcome.transcript?.contains(TeammateEnrollment.notRecorded) == true)
        #expect(journal.entries.isEmpty)
    }

    /// The bind and the take-off both fail: said loudly, and kept in the
    /// journal for the next launch to take off (review F2).
    @Test func aKeyThatCantBeBoundOrTakenOffIsLoudAndKept() async throws {
        let (journal, folder) = Self.journal()
        defer { try? FileManager.default.removeItem(at: folder) }
        let runner = Runner()
        runner.addFails = true
        runner.revokeFails = true
        let outcome = await enroll(runner, journal: journal)
        #expect(outcome.written.isEmpty)
        #expect(outcome.transcript?.contains(TeammateEnrollment.stillThere) == true)
        #expect(journal.entries == [.init(clientID: "farcooler-0a0b", runner: Self.runner.id, label: "build-box")])
    }

    /// The app quit between the write and the bind: the next launch takes
    /// the key off, and leaves alone a key the runner shows bound.
    @Test func theNextLaunchTakesOffWhatWasLeftUnbound() async throws {
        let (journal, folder) = Self.journal()
        defer { try? FileManager.default.removeItem(at: folder) }
        try journal.add(clientID: "farcooler-0a0b", runner: "alice@build-box", label: "build-box")
        try journal.add(clientID: "farcooler-0c0d", runner: "alice@staging", label: "staging")
        let runner = Runner()
        runner.listed =
            #"{"members":[{"handle":"carol","name":"Carol","devices":["farcooler-0c0d"],"reps":0,"rep_panes":[]}],"claude_account":"","you":""}"#
        await TeammateEnrollment.reconcile(journal: journal, using: { await runner.run($0) })
        let revokes = runner.calls.filter { $0.contains("revoke") }
        #expect(revokes.count == 1 && revokes[0].last == "farcooler-0a0b", "\(revokes)")
        #expect(journal.entries.isEmpty)

        // A runner that can't be reached stays noted for the launch after.
        try journal.add(clientID: "farcooler-0a0b", runner: "alice@build-box", label: "build-box")
        runner.revokeFails = true
        runner.listed = #"{"members":[],"claude_account":"","you":""}"#
        await TeammateEnrollment.reconcile(journal: journal, using: { await runner.run($0) })
        #expect(journal.entries.count == 1)
    }

    /// A key already here is always taken off, both its lines, and written
    /// again at the chosen scope (review F8, re-review R3). The ordinary Mac
    /// shape is the one that matters: Key A at Control, which the answer
    /// reports, and a Key B shell line under the same client id, which it
    /// doesn't mention.
    @Test func aKeyAlreadyHereIsRewrittenAtTheChosenScope() async throws {
        let (journal, folder) = Self.journal()
        defer { try? FileManager.default.removeItem(at: folder) }
        let runner = Runner()
        runner.enrolls = [
            (true, #"{"client":{"scope":"control","shellAccess":false},"alreadyEnrolled":true,"connBlob":""}"#),
            (true, #"{"client":{"scope":"control"},"alreadyEnrolled":false,"connBlob":""}"#),
        ]
        let outcome = await enroll(runner, scope: .control, journal: journal)
        #expect(runner.verbs() == ["list", "enroll", "revoke", "enroll", "add"], "a Key B shell line could survive")
        #expect(outcome.written == [Self.runner.id])
    }

    /// A runner from before member accounts is refused before any key is
    /// written or noted (re-review R4).
    @Test func anOldRunnerIsRefusedBeforeAnyWrite() async throws {
        let (journal, folder) = Self.journal()
        defer { try? FileManager.default.removeItem(at: folder) }
        let runner = Runner()
        runner.old = true
        let outcome = await enroll(runner, journal: journal)
        #expect(runner.verbs() == ["list"], "a key was written to a runner that can't record whose it is")
        #expect(outcome.transcript?.contains(TeammateEnrollment.runnerTooOld) == true)
        #expect(journal.entries.isEmpty)
    }

    /// A journal that can't be saved stops the write (re-review R2).
    @Test func anUnsavableJournalWritesNothing() async throws {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent("teammate-journal-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: folder) }
        // Its folder is a file, so nothing can be saved under it.
        try Data().write(to: folder)
        let journal = TeammateJournal(file: folder.appendingPathComponent("journal.json"))
        let runner = Runner()
        let outcome = await enroll(runner, journal: journal)
        #expect(runner.verbs() == ["list"], "a key was written with no note of it")
        #expect(outcome.transcript?.contains(TeammateEnrollment.journalUnwritable) == true)
    }

    /// A journal that doesn't decode is moved aside, never overwritten.
    @Test func anUnreadableJournalIsKeptAside() throws {
        let (journal, folder) = Self.journal()
        defer { try? FileManager.default.removeItem(at: folder) }
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        try Data("not json".utf8).write(to: folder.appendingPathComponent("journal.json"))
        try journal.add(clientID: "farcooler-0a0b", runner: "alice@build-box", label: "build-box")
        #expect(journal.entries.count == 1)
        let aside = try FileManager.default.contentsOfDirectory(atPath: folder.path).filter { $0.contains("unreadable") }
        #expect(aside.count == 1)
        #expect(try String(contentsOf: folder.appendingPathComponent(aside[0]), encoding: .utf8) == "not json")
    }

    /// A runner whose list can't be read keeps its entry: the key may be
    /// bound to a teammate who's working (re-review R1).
    @Test func reconcileNeverRevokesWhatItCantRead() async throws {
        let (journal, folder) = Self.journal()
        defer { try? FileManager.default.removeItem(at: folder) }
        try journal.add(clientID: "farcooler-0a0b", runner: "alice@build-box", label: "build-box")
        let runner = Runner()
        runner.listFails = true
        await TeammateEnrollment.reconcile(journal: journal, using: { await runner.run($0) })
        #expect(runner.call("revoke") == nil, "a key was taken off a runner whose people weren't read")
        #expect(journal.entries.count == 1)
        // And an answer that doesn't parse is the same.
        runner.listFails = false
        runner.listed = "not json}"
        runner.old = true
        await TeammateEnrollment.reconcile(journal: journal, using: { await runner.run($0) })
        #expect(runner.call("revoke") == nil)
        #expect(journal.entries.count == 1)
    }

    /// Bob already on the runner keeps his handle, name and other devices.
    @Test func aTeammateAlreadyHereGainsTheDevice() async throws {
        let (journal, folder) = Self.journal()
        defer { try? FileManager.default.removeItem(at: folder) }
        let runner = Runner()
        runner.listed =
            #"{"members":[{"handle":"bobby","name":"Bobby","devices":["farcooler-old"],"reps":0,"rep_panes":[],"account":"user_bob","email":"bob@example.com"},{"handle":"bob","name":"Another Bob","devices":[],"reps":0,"rep_panes":[]}],"claude_account":"","you":""}"#
        let outcome = await enroll(runner, journal: journal)
        #expect(outcome.written == [Self.runner.id])
        let add = try #require(runner.call("add"))
        #expect(add.contains("bobby") && add.contains("Bobby") && add.contains("farcooler-old"))
        #expect(add.contains("farcooler-0a0b"))
    }

    /// A runner that checks what a bind read is told it (ov-786): the
    /// devices its list showed under the handle being bound, and none for a
    /// handle not there yet.
    @Test func aBindSaysWhatItRead() async throws {
        for (listed, read) in [
            (Self.bobbyWithOld, "--if-devices=farcooler-old"),
            (#"{"members":[],"claude_account":"","you":""}"#, "--if-devices="),
        ] {
            let (journal, folder) = Self.journal()
            defer { try? FileManager.default.removeItem(at: folder) }
            let runner = Runner()
            runner.checksRead = true
            runner.listed = listed
            _ = await enroll(runner, journal: journal)
            let add = try #require(runner.call("add"))
            #expect(add.last == read, "\(add)")
        }
    }

    /// Another Add Teammate gave Bob a device between the read and the bind
    /// (ov-786): the runner refuses the stale list, and the bind reads again
    /// and goes through once more, keeping that device and taking nothing off.
    @Test func aStaleBindReadsAgainAndKeepsTheDeviceAddedSince() async throws {
        let (journal, folder) = Self.journal()
        defer { try? FileManager.default.removeItem(at: folder) }
        let runner = Runner()
        runner.checksRead = true
        runner.listed = Self.bobbyWithOld
        runner.relisted = [Self.bobbyWithOld.replacingOccurrences(of: #"["farcooler-old"]"#, with: #"["farcooler-old","farcooler-mac"]"#)]
        runner.adds = [(false, Self.stale), (true, "{}")]
        let outcome = await enroll(runner, journal: journal)
        #expect(runner.verbs() == ["list", "enroll", "add", "list", "add"])
        let again = try #require(runner.calls.last)
        #expect(again.contains("farcooler-old") && again.contains("farcooler-mac") && again.contains("farcooler-0a0b"))
        #expect(again.last == "--if-devices=farcooler-old,farcooler-mac")
        #expect(outcome.written == [Self.runner.id])
        #expect(outcome.transcript == nil)
        #expect(journal.entries.isEmpty)
    }

    /// Stale twice: it reads again once, not forever, and a key it couldn't
    /// bind is taken off like any other.
    @Test func aBindStaleTwiceIsNotRecorded() async throws {
        let (journal, folder) = Self.journal()
        defer { try? FileManager.default.removeItem(at: folder) }
        let runner = Runner()
        runner.checksRead = true
        runner.adds = [(false, Self.stale), (false, Self.stale)]
        let outcome = await enroll(runner, journal: journal)
        #expect(runner.verbs() == ["list", "enroll", "add", "list", "add", "revoke"])
        #expect(outcome.written.isEmpty)
        #expect(outcome.transcript?.contains(TeammateEnrollment.notRecorded) == true)
    }

    /// Another admin's ceremony bound the same device while this one's bind
    /// was refused as stale: the fresh read shows it held, so its key is
    /// never taken off here, whoever holds it (review of ov-786).
    @Test func aDeviceBoundMeanwhileIsNeverTakenOff() async throws {
        let held = { (account: String) in
            #"{"members":[{"handle":"rob","name":"Rob","devices":["farcooler-0a0b"],"reps":0,"rep_panes":[],"account":"\#(account)","email":""}],"claude_account":"","you":""}"#
        }
        // Someone else's: said, and not bound again. Bob's: a second stale
        // refusal is not recorded, and still nothing is taken off.
        for (account, adds) in [("user_rob", [(false, Self.stale)]), ("user_bob", [(false, Self.stale), (false, Self.stale)])] {
            let (journal, folder) = Self.journal()
            defer { try? FileManager.default.removeItem(at: folder) }
            let runner = Runner()
            runner.checksRead = true
            runner.relisted = [held(account)]
            runner.adds = adds
            let outcome = await enroll(runner, journal: journal)
            #expect(runner.call("revoke") == nil, "a device bound meanwhile was taken off: \(account)")
            #expect(outcome.written.isEmpty)
            #expect(outcome.transcript != nil)
            #expect(journal.entries.isEmpty, "\(account)")
        }
    }

    static let bobbyWithOld =
        #"{"members":[{"handle":"bobby","name":"Bobby","devices":["farcooler-old"],"reps":0,"rep_panes":[],"account":"user_bob","email":"bob@example.com"}],"claude_account":"","you":""}"#
    /// What the CLI says, under `--json`, for a bind the runner found stale.
    static let stale =
        "their devices changed since you read them, so nothing was changed.\ncode: resource-conflict\nwhat: devices_changed"

    /// A new Bob where a hand-added `bob` exists gets a handle of his own.
    @Test func aTakenHandleIsMadeUnique() async throws {
        let (journal, folder) = Self.journal()
        defer { try? FileManager.default.removeItem(at: folder) }
        let runner = Runner()
        runner.listed =
            #"{"members":[{"handle":"bob","name":"Another Bob","devices":[],"reps":0,"rep_panes":[]}],"claude_account":"","you":""}"#
        var thisMac = Self.runner
        thisMac.id = ""
        _ = await enroll(runner, on: [thisMac], journal: journal)
        let add = try #require(runner.call("add"))
        #expect(add.contains("bob-2"))
        #expect(!add.contains("--runner"), "this Mac is named by no flag")
    }

    /// A device already Bob's whose record can't be updated: a member's Mac
    /// unbinds it on writing the key, so it only views; the owner's Mac
    /// unbinds nothing, so it keeps the access chosen (ov-720).
    @Test func aRecordThatWontUpdateIsDescribedByWhoIsEnrolling() async throws {
        for (you, viewsOnly) in [("", false), ("alice", true)] {
            let (journal, folder) = Self.journal()
            defer { try? FileManager.default.removeItem(at: folder) }
            let runner = Runner()
            runner.addFails = true
            runner.listed =
                #"{"members":[{"handle":"bob","name":"Bob","devices":["farcooler-0a0b"],"reps":0,"rep_panes":[],"account":"user_bob","email":"bob@example.com"}],"claude_account":"","you":"\#(you)"}"#
            let outcome = await enroll(runner, journal: journal)
            let said = try #require(outcome.transcript)
            #expect(said.contains(TeammateEnrollment.notUpdated(asMember: viewsOnly)))
            #expect(said.contains("only view") == viewsOnly, "you: \(you)")
            #expect(said.contains("still has the access you chose") == !viewsOnly, "you: \(you)")
        }
    }

    /// A key that was already another person's here is left alone, with
    /// nothing written: taking it off would take their access away.
    @Test func aKeyThatIsSomeoneElsesHereIsLeftAlone() async throws {
        let (journal, folder) = Self.journal()
        defer { try? FileManager.default.removeItem(at: folder) }
        let runner = Runner()
        runner.listed =
            #"{"members":[{"handle":"carol","name":"Carol","devices":["farcooler-0a0b"],"reps":0,"rep_panes":[],"account":"user_carol","email":"c@example.com"}],"claude_account":"","you":""}"#
        let outcome = await enroll(runner, journal: journal)
        #expect(outcome.written.isEmpty)
        #expect(runner.verbs() == ["list"], "something was written to another person's device")
        #expect(outcome.transcript?.contains(TeammateEnrollment.boundToSomeoneElse) == true)
        #expect(journal.entries.isEmpty)
    }
}

/// Two teammate checks overlapping (review F3): the earlier code's slow answer
/// never opens its sheet over the later code.
@MainActor
struct TeammateScanRaceTests {
    /// Holds every directory answer until it's let go.
    @MainActor final class Gate {
        var held: [CheckedContinuation<Void, Never>] = []
        func answer() async -> Data? {
            await withCheckedContinuation { held.append($0) }
            return TeammateCeremonyTests.directory(withCarol: true)
        }
    }

    @Test func aStaleCheckNeverOpensItsSheetOverALaterCode() async throws {
        let bob = try #require(
            CeremonyFFI.offer(name: "Bob’s iPhone", account: "user_bob", keyA: TeammateCeremonyTests.keyA, keyB: nil))
        let carol = try #require(
            CeremonyFFI.offer(name: "Carol’s iPhone", account: "user_carol", keyA: TeammateCeremonyTests.keyB, keyB: nil))
        let gate = Gate()
        let store = CeremonyStore()
        store.teamDirectory = { await gate.answer() }
        store.scan(bob, at: Date(), account: "user_alice", email: "alice@example.com", runners: [])
        let first = store.teamCheck
        while gate.held.count < 1 { await Task.yield() }
        // The admin scans Carol's code while Bob's check is still out, and
        // Bob's answer arrives first, while Carol's is still being checked.
        store.scan(carol, at: Date(), account: "user_alice", email: "alice@example.com", runners: [])
        let second = store.teamCheck
        while gate.held.count < 2 { await Task.yield() }
        gate.held[0].resume()
        await first?.value
        #expect(store.phase == .checkingTeam, "Bob's late answer opened a sheet over Carol's code")
        gate.held[1].resume()
        await second?.value
        guard case .confirming(let confirmation) = store.phase else {
            Issue.record("not confirming: \(store.phase)")
            return
        }
        #expect(confirmation.teammate?.who.account == "user_carol")
        #expect(confirmation.offer.name == "Carol’s iPhone")
    }
}
