import AgentKit
import Foundation

/// Enrolling a teammate's device, and recording whose it is, one runner at a
/// time (ov-525).
///
/// A device bound to nobody on a runner is the OWNER's there
/// (`crates/store/src/people.rs`): it leads every workspace and talks to the
/// orchestrator. So a teammate's key must never stay on a runner unbound, and
/// three things keep it that way (review F2):
///
/// - **One runner at a time.** Read who's there, write Key A, bind it to the
///   teammate's member row, and only then move to the next runner, so the
///   window in which a key is unbound is one runner's round trip.
/// - **A write that failed may have landed.** A dropped ssh after the
///   daemon's write is reported as a failure, so the key is taken off again
///   whenever a write or a bind fails, and taking off a key that isn't there
///   is harmless.
/// - **A journal.** Each runner is noted in ``TeammateJournal`` before its
///   write and crossed off once the key is bound or taken off. Whatever is
///   still noted when the app next opens (it quit, crashed or slept in
///   between, or the runner couldn't be reached to take the key off) is taken
///   off then, unless that runner shows the device bound to someone.
///
/// The reply is built only after this returns, so the teammate's device can't
/// connect anywhere in between.
enum TeammateEnrollment {
    /// Who the teammate is, as the confirmation left it.
    struct Member: Sendable, Equatable {
        var handle: String
        var name: String
        var account: String
        var email: String
        /// What they may do (ov-856). Viewer is written as a read line, so the
        /// runner records it by itself; Admin is given afterwards, once they are
        /// bound, by the owner's socket. A person already here keeps theirs.
        var access: PersonAccess = .member
        /// The handle of the person already on the runner this device is
        /// added to; nil is a New Person (or whoever already has this account).
        var person: String? = nil
    }

    /// Who is already there for `member`: the holder of the device, the person
    /// picked, or whoever has the teammate's account.
    private static func existing(_ member: Member, holder: RunnerPeople.Member?, in people: RunnerPeople) -> RunnerPeople.Member? {
        holder ?? member.person.flatMap { handle in people.members.first { $0.handle == handle } }
            ?? people.members.first { !member.account.isEmpty && $0.account == member.account }
    }

    /// Enroll Key A at `scope` on each runner (never Key B: a teammate gets
    /// no shell) and bind it to `member`, one runner at a time.
    static func enroll(
        keyA: String, label: String, clientID: String, scope: TeammateScope, nodeKey: String,
        member: Member, on runners: [CeremonyRunner], journal: TeammateJournal = .shared,
        using run: @escaping Enrollment.Writer = {
            let result = await CLI.run($0)
            return (result.ok, result.ok ? result.output : result.said)
        }
    ) async -> Enrollment.Outcome {
        await launchReconcileFinished()
        var outcome = Enrollment.Outcome()
        var failures: [String] = []
        for runner in runners {
            let said = await enrollOne(
                keyA: keyA, label: label, clientID: clientID, scope: scope, nodeKey: nodeKey, member: member,
                on: runner, journal: journal, outcome: &outcome, using: run)
            if let said { failures.append("\(runner.label): \(said)") }
        }
        outcome.transcript = failures.isEmpty ? nil : failures.joined(separator: "\n\n")
        return outcome
    }

    /// One runner: who's there, the write, the bind. Nil when the key is
    /// there and bound; otherwise what to say about this runner.
    private static func enrollOne(
        keyA: String, label: String, clientID: String, scope: TeammateScope, nodeKey: String,
        member: Member, on runner: CeremonyRunner, journal: TeammateJournal,
        outcome: inout Enrollment.Outcome, using run: Enrollment.Writer
    ) async -> String? {
        // Who's here first, so a device that's already someone else's on this
        // runner is left exactly as it is, with nothing written.
        let listed = await run(arguments(["member", "list"], runner: runner.id))
        guard listed.ok, let people = people(in: listed.output) else {
            return "\(Self.couldNotRead)\n\n\(listed.output)"
        }
        let holder = people.members.first { $0.devices.contains(clientID) }
        if let holder, !holder.account.isEmpty, holder.account != member.account {
            return Self.boundToSomeoneElse
        }
        // A runner that can't keep the teammate's account is refused here,
        // before any key is written (re-review R4).
        guard keepsAccounts(listed.output) else { return Self.runnerTooOld }

        // Noted before the write, or not written at all: an unjournaled key is
        // one nothing would take off after a crash (re-review R2).
        do {
            try journal.add(clientID: clientID, runner: runner.id, label: runner.label)
        } catch {
            return Self.journalUnwritable
        }
        let enroll = Enrollment.arguments(
            key: keyA, label: label, clientID: clientID, scope: scope.rawValue, shell: false, nodeKey: nodeKey,
            runner: runner.id, forTeammate: true)
        var written = await run(enroll)
        // A key already here keeps its lines, and they may be wider than what
        // was chosen. The answer describes only Key A's line, and a Mac's Key B
        // shell line under the same client id says nothing in it, so a key
        // that was already here is always taken off, both lines at once, and
        // written again at the chosen scope (review F8, re-review R3).
        if written.ok, let found = enrolled(in: written.output), found.already {
            let revoked = await run(revokeArguments(clientID: clientID, runner: runner.id))
            if revoked.ok {
                written = await run(enroll)
            } else {
                written = (false, revoked.output)
            }
        }
        guard written.ok else {
            return await takeOff(clientID, from: runner, journal: journal, because: Self.notAdded, written.output, run)
        }

        let newcomer = Self.existing(member, holder: holder, in: people) == nil
        var added = await bind(
            clientID, to: member, holder: holder, people: people, checked: checksRead(listed.output), on: runner,
            using: run)
        // Their devices changed between the read and the bind: another Add
        // Teammate for them, from this Mac or another (ov-786). Unchecked, one
        // of the two lists would take the other's device off and revoke it, so
        // the runner refused this one, writing nothing. Read again and bind
        // once more from what's there now.
        // Whoever the fresh read shows holding the device, if anyone: bound
        // by another admin's ceremony meanwhile, it's theirs to keep, and
        // its key is never taken off here (review of ov-786).
        var freshHolder: RunnerPeople.Member?
        if !added.ok, TaskFailure.what(in: added.output) == "devices_changed" {
            let again = await run(arguments(["member", "list"], runner: runner.id))
            if again.ok, let fresh = Self.people(in: again.output) {
                freshHolder = fresh.members.first { $0.devices.contains(clientID) }
                if let freshHolder, !freshHolder.account.isEmpty, freshHolder.account != member.account {
                    journal.remove(clientID: clientID, runner: runner.id)
                    return Self.boundToSomeoneElse
                }
                added = await bind(
                    clientID, to: member, holder: freshHolder, people: fresh, checked: true, on: runner, using: run)
            }
        }
        guard added.ok else {
            // Bound before this bind, by an earlier ceremony or by one that
            // ran alongside it. A runner from ov-719 on unbinds a device when
            // a member's Mac writes its key, so it can only view there until
            // a bind succeeds; an older runner kept it bound. Either way
            // there's nothing to undo.
            if holder != nil || freshHolder != nil {
                journal.remove(clientID: clientID, runner: runner.id)
                return "\(Self.notUpdated(asMember: people.isMember))\n\n\(added.output)"
            }
            return await takeOff(clientID, from: runner, journal: journal, because: Self.notRecorded, added.output, run)
        }
        journal.remove(clientID: clientID, runner: runner.id)
        outcome.written.insert(runner.id)
        if case .tailcat(let token) = Enrollment.reach(granting: runner, token: Enrollment.token(in: written.output)) {
            outcome.tunneled[runner.id] = token
        }
        // Admin is the owner's to give, and a new person is a Member until it
        // is: this Mac is the owner's, so it asks the runner's socket. The
        // device is there either way; a failure says what is left to do.
        if newcomer, member.access == .admin {
            let handle = TeammateNames.unique(member.handle, taken: Set(people.members.map(\.handle)))
            let promoted = await run(arguments(["people", "access", handle, "admin"], runner: runner.id))
            if !promoted.ok { return Self.adminNotGiven(member.name) }
        }
        return nil
    }

    /// Bind `clientID` to the teammate on `runner`, as `people` read there. A
    /// teammate already here (by device, or by account) keeps their handle,
    /// name and other devices, and gains this one: `member.set` replaces a
    /// member's devices, so they're read first. `checked` sends what was read
    /// (`--if-devices`, ov-786), so a list built from a read that's gone stale
    /// is refused rather than taking off a device added since.
    private static func bind(
        _ clientID: String, to member: Member, holder: RunnerPeople.Member?, people: RunnerPeople, checked: Bool,
        on runner: CeremonyRunner, using run: Enrollment.Writer
    ) async -> (ok: Bool, output: String) {
        let existing = Self.existing(member, holder: holder, in: people)
        let handle =
            existing?.handle ?? TeammateNames.unique(member.handle, taken: Set(people.members.map(\.handle)))
        var devices = existing?.devices ?? []
        if !devices.contains(clientID) { devices.append(clientID) }
        var add = ["member", "add", handle, "--name", existing?.name ?? member.name]
        for device in devices { add += ["--device", device] }
        add += ["--account", member.account, "--email", member.email]
        if checked {
            // What the runner has as this handle's now: none for a new one.
            let read = people.members.first { $0.handle == handle }?.devices ?? []
            add += ["--if-devices=" + read.joined(separator: ",")]
        }
        return await run(arguments(add, runner: runner.id))
    }

    /// Take the key off a runner after a write or a bind that failed. When
    /// that fails too, the runner stays in the journal for the next launch,
    /// and the sentence says so: never silent.
    private static func takeOff(
        _ clientID: String, from runner: CeremonyRunner, journal: TeammateJournal, because: String,
        _ why: String, _ run: Enrollment.Writer
    ) async -> String {
        let revoked = await run(revokeArguments(clientID: clientID, runner: runner.id, withAdded: true))
        guard revoked.ok else {
            return "\(because) \(Self.stillThere)\n\n\(why)\n\n\(revoked.output)"
        }
        journal.remove(clientID: clientID, runner: runner.id)
        return "\(because) \(Self.takenOff)\n\n\(why)"
    }

    /// Take off every key the journal still holds, unless its runner shows
    /// the device bound to a member: what an earlier enrollment left halfway,
    /// done at launch. A runner that can't be reached stays noted for next
    /// time.
    static func reconcile(
        journal: TeammateJournal = .shared,
        using run: @escaping Enrollment.Writer = {
            let result = await CLI.run($0)
            return (result.ok, result.ok ? result.output : result.said)
        }
    ) async {
        for entry in journal.entries {
            let listed = await run(arguments(["member", "list"], runner: entry.runner))
            // Unread is not unbound: a runner whose list can't be read keeps
            // its entry for next time rather than lose a teammate the key was
            // bound to before the app quit (re-review R1).
            guard listed.ok, let people = people(in: listed.output) else { continue }
            if people.members.contains(where: { $0.devices.contains(entry.clientID) }) {
                journal.remove(clientID: entry.clientID, runner: entry.runner)
                continue
            }
            if await run(revokeArguments(clientID: entry.clientID, runner: entry.runner, withAdded: true)).ok {
                journal.remove(clientID: entry.clientID, runner: entry.runner)
            }
        }
    }

    @MainActor private static var reconciledThisLaunch = false
    /// The launch's reconcile, which a teammate enrollment waits for: it
    /// must not take off a key the new ceremony has just written
    /// (re-review R2).
    @MainActor private static var launchReconcile: Task<Void, Never>?

    /// ``reconcile(journal:using:)`` once per launch, and only when the
    /// journal holds something.
    @MainActor static func reconcileAtLaunch() {
        guard !reconciledThisLaunch else { return }
        reconciledThisLaunch = true
        guard !TeammateJournal.shared.entries.isEmpty else { return }
        launchReconcile = Task { await reconcile() }
    }

    /// Wait for the launch's reconcile, if it's still running.
    @MainActor static func launchReconcileFinished() async {
        await launchReconcile?.value
    }

    /// `farcooler --json [--runner <id>] <rest>`, with no `--runner` for this
    /// Mac, as ``Enrollment/arguments(key:label:clientID:scope:shell:nodeKey:runner:)``
    /// spells it.
    static func arguments(_ rest: [String], runner: String) -> [String] {
        ["--json"] + (runner.isEmpty ? [] : ["--runner", runner]) + rest
    }

    /// `withAdded` takes off, with a teammate's key, every device it added
    /// while it waited on its bind (ov-838): a key from before ov-828 read as
    /// the owner's then, and what it added would otherwise stay the owner's.
    static func revokeArguments(clientID: String, runner: String, withAdded: Bool = false) -> [String] {
        arguments(["client", "revoke", clientID] + (withAdded ? ["--with-added"] : []), runner: runner)
    }

    /// The member list in a `--json` reply: its last line that is an object,
    /// since a login shell may print a banner first.
    static func people(in output: String) -> RunnerPeople? {
        let line = output.split(whereSeparator: \.isNewline).last { $0.hasPrefix("{") }
        return line.flatMap { RunnerPeople(json: Data($0.utf8)) }
    }

    /// Whether `member list --json` says the runner keeps member accounts.
    /// Absent is no: the field is this lane's, and a CLI that doesn't write it
    /// can't vouch for the runner.
    static func keepsAccounts(_ output: String) -> Bool {
        let line = output.split(whereSeparator: \.isNewline).last { $0.hasPrefix("{") }
        guard let line, let object = try? JSONSerialization.jsonObject(with: Data(line.utf8)) as? [String: Any]
        else { return false }
        return object["member_accounts"] as? Bool ?? false
    }

    /// Whether `member list --json` says the runner checks what a bind read
    /// (ov-786). Absent is no: an older runner would drop `--if-devices`, so
    /// the CLI refuses it there, and the bind goes unchecked as before.
    static func checksRead(_ output: String) -> Bool {
        let line = output.split(whereSeparator: \.isNewline).last { $0.hasPrefix("{") }
        guard let line, let object = try? JSONSerialization.jsonObject(with: Data(line.utf8)) as? [String: Any]
        else { return false }
        return object["member_devices_read"] as? Bool ?? false
    }

    /// What `client enroll --json` says it found: whether the key was there
    /// already, at what scope, and whether as a shell line.
    static func enrolled(in output: String) -> (already: Bool, scope: String, shell: Bool)? {
        for line in output.split(whereSeparator: \.isNewline) where line.hasPrefix("{") {
            guard let object = try? JSONSerialization.jsonObject(with: Data(line.utf8)) as? [String: Any],
                let client = object["client"] as? [String: Any]
            else { continue }
            return (
                object["alreadyEnrolled"] as? Bool ?? false, client["scope"] as? String ?? "",
                client["shellAccess"] as? Bool ?? false
            )
        }
        return nil
    }

    /// Couldn't read who's on the runner, so nothing was written there.
    static let couldNotRead =
        "Far Cooler couldn’t read who’s on this runner, so it didn’t add your teammate’s device here."
    /// The write failed, and may have landed anyway.
    static let notAdded =
        "Far Cooler couldn’t add your teammate’s device here, and any access it had here before is gone."
    /// The key went on, and couldn't be recorded as the teammate's.
    static let notRecorded = "Far Cooler couldn’t record this device as your teammate’s here."
    /// A device bound before this ceremony, whose record couldn't be updated.
    /// A member's Mac unbinds it when it writes the key (ov-719), so it can
    /// only view until it's bound again; the owner's Mac unbinds nothing, so
    /// it keeps the access chosen (ov-720).
    static func notUpdated(asMember: Bool) -> String {
        let couldNot = "Far Cooler couldn’t update your teammate’s record here"
        if asMember {
            return couldNot + ", so their device can only view this runner for now. "
                + "Add your teammate again to give it the access you chose."
        }
        return couldNot + ", but their device still has the access you chose. "
            + "Add your teammate again to finish updating it."
    }
    /// A runner from before member accounts, refused before any write.
    static let runnerTooOld =
        "This runner’s Far Cooler is too old to record whose device this is, so nothing was added here. "
        + "Update Far Cooler on it, then add your teammate again."
    /// The journal couldn't be saved, so nothing was written.
    static let journalUnwritable =
        "Far Cooler couldn’t save a note of this change on this Mac, so it didn’t add your teammate’s "
        + "device here. Check that your disk has space, then try again."
    /// The device was added as a Member, and making them an Admin failed.
    static func adminNotGiven(_ name: String) -> String {
        "\(name)’s device was added, but Far Cooler couldn’t make them an Admin here, so they’re a Member for now. "
            + "Change their access in People."
    }
    /// Taking the key off worked.
    static let takenOff = "It took the device’s key off again, so it has no access here."
    /// Taking the key off failed too: loud, with what happens next.
    static let stillThere =
        "It also couldn’t make sure the device’s key isn’t on this runner, so it’ll try again the next "
        + "time Far Cooler opens. Until then, check Settings > Devices for this runner."
    /// The key was already another member's on that runner.
    static let boundToSomeoneElse =
        "This device is already someone else’s on this runner, so it wasn’t added for your teammate."
}

/// The runners a teammate's key may be on, unbound (review F2): noted before
/// each write, crossed off once the key is bound or taken off, and taken off
/// at launch by ``TeammateEnrollment/reconcile(journal:using:)`` if still noted.
///
/// A small JSON file in the app's own support folder, beside its socket.
final class TeammateJournal: @unchecked Sendable {
    struct Entry: Codable, Equatable {
        var clientID: String
        var runner: String
        var label: String
    }

    static let shared = TeammateJournal(
        file: URL(fileURLWithPath: AppControl.socketPath()).deletingLastPathComponent()
            .appendingPathComponent("teammate-enrollments.json"))

    private let file: URL
    private let lock = NSLock()

    init(file: URL) { self.file = file }

    var entries: [Entry] {
        lock.lock()
        defer { lock.unlock() }
        return read()
    }

    /// Throws when the note can't be saved, and the caller writes nothing.
    func add(clientID: String, runner: String, label: String) throws {
        try change { entries in
            if !entries.contains(where: { $0.clientID == clientID && $0.runner == runner }) {
                entries.append(Entry(clientID: clientID, runner: runner, label: label))
            }
        }
    }

    /// Best effort: an entry that stays is taken off at the next launch,
    /// which is the safe way to be wrong.
    func remove(clientID: String, runner: String) {
        try? change { $0.removeAll { $0.clientID == clientID && $0.runner == runner } }
    }

    private func change(_ edit: (inout [Entry]) -> Void) throws {
        lock.lock()
        defer { lock.unlock() }
        var entries = read()
        edit(&entries)
        try FileManager.default.createDirectory(
            at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
        try JSONEncoder().encode(entries).write(to: file, options: .atomic)
    }

    /// The entries, or none. A file that doesn't decode is moved aside, not
    /// overwritten, so whatever it held can still be read by a person.
    private func read() -> [Entry] {
        guard let data = try? Data(contentsOf: file) else { return [] }
        if let entries = try? JSONDecoder().decode([Entry].self, from: data) { return entries }
        let aside = file.deletingPathExtension()
            .appendingPathExtension("unreadable-\(Int(Date().timeIntervalSince1970)).json")
        try? FileManager.default.moveItem(at: file, to: aside)
        return []
    }
}
