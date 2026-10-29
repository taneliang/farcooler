import Foundation
import SwiftUI
import UIKit

/// The People screen's Access, devices and keys for one runner (ov-856, §8):
/// what to draw, and what each button does through a step-up.
///
/// Every change goes the runner's way: a phone below `host_admin` asks the
/// person for Face ID (or the passcode) once per action, and the runner holds
/// it to that exact action. The words a refusal uses are AgentKit's.
@MainActor
final class PeopleAccessModel: ObservableObject {
    @Published private(set) var access: PeopleAccess?
    /// The six digits this phone shows while its own key waits.
    @Published private(set) var shownCode: String?
    /// A sentence for the last action that didn't happen, or nil.
    @Published var note: String?

    private let connection: Connection
    private let runner: String
    private var members: Data?

    init(connection: Connection, runner: String) {
        self.connection = connection
        self.runner = runner
    }

    /// Read again from `member.list`'s answer (and `client.list`), on a runner
    /// that records the people's access (`person_access`).
    func load(members: Data) async {
        self.members = members
        guard connection.daemon?.can(.personAccess) == true else { access = nil; return }
        let read = await connection.loadPeopleAccess(members: members)
        access = read?.recorded == true ? read : nil
    }

    private func reload() async {
        guard let data = try? await connection.rpc("member.list") else { return }
        await load(members: data)
    }

    private var device: String { HardwareKind.kind }

    /// What this phone can offer, by what the runner would accept from it.
    func actions() -> PeopleAccessActions {
        guard connection.daemon?.can(.stepUp) == true else { return PeopleAccessActions() }
        return PeopleAccessActions(
            removeDevice: { [weak self] device, withAdded in await self?.remove(device, withAdded: withAdded) },
            // Signed by this phone's own key, so offered once it holds one (§8).
            limitToViewer: hasTrustedKey ? { [weak self] device in await self?.limit(device) } : nil,
            setAccess: hasTrustedKey ? { [weak self] person, new in await self?.setAccess(person, to: new) } : nil,
            approve: { [weak self] device in await self?.approve(device) },
            allowAdminActions: { [weak self] in await self?.allowAdminActions() })
    }

    private var hasTrustedKey: Bool { access?.thisDevice?.stepUp == .trusted }

    private func perform(_ method: String, _ args: [String: Any], reason: String) async -> Bool {
        guard let stepUp = connection.stepUp() else {
            note = StepUpWords.sentence(for: .keyUnavailable, runner: runner, device: device)
            return false
        }
        do {
            _ = try await stepUp.perform(method, args, reason: reason, hasTrustedKey: hasTrustedKey)
            note = nil
            return true
        } catch let error as StepUpError {
            note = StepUpWords.sentence(for: error, runner: runner, device: device)
        } catch {
            note = StepUpWords.sentence(
                forRefusal: StepUpRefusals.word(error), runner: runner, device: device)
        }
        return false
    }

    func remove(_ target: PeopleAccess.Device, withAdded: Bool) async {
        let reason = StepUpReason.removeDevice(target.name, alsoRemoving: withAdded ? target.added : 0, runner: runner)
        let args: [String: Any] = ["clientId": target.id, "withAdded": withAdded]
        if await perform("client.revoke", args, reason: reason) { await reload() }
    }

    /// Hold one device at Viewer, with Face ID for exactly that device.
    func limit(_ target: PeopleAccess.Device) async {
        let reason = StepUpReason.limit(target.name, runner: runner)
        let args: [String: Any] = ["clientId": target.id, "cap": "viewer"]
        if await perform("client.set_cap", args, reason: reason) { await reload() }
    }

    /// Give a person another Access, with Face ID for exactly that person and access.
    func setAccess(_ person: PeopleAccess.Person, to new: PersonAccess) async {
        let reason = StepUpReason.setAccess(person.name, to: new, runner: runner)
        let args: [String: Any] = ["handle": person.handle, "access": new.rawValue]
        if await perform("member.set_access", args, reason: reason) { await reload() }
    }

    func approve(_ target: PeopleAccess.Device) async {
        let reason = StepUpReason.approve(device: target.name)
        let args: [String: Any] = ["clientId": target.id, "code": target.code]
        if await perform("client.approve_step_up", args, reason: reason) { await reload() }
    }

    /// Offer this phone's key and show the digits another of the person's
    /// devices checks.
    func allowAdminActions() async {
        guard let stepUp = connection.stepUp() else {
            note = StepUpWords.sentence(for: .keyUnavailable, runner: runner, device: device)
            return
        }
        do {
            shownCode = try await stepUp.register()
            note = nil
            await reload()
        } catch let error as StepUpError {
            note = StepUpWords.sentence(for: error, runner: runner, device: device)
        } catch {
            note = StepUpWords.sentence(forRefusal: StepUpRefusals.word(error), runner: runner, device: device)
        }
    }
}

/// A refusal's word, from what the client core threw.
enum StepUpRefusals {
    static func word(_ error: Error) -> String {
        guard case ClientCore.CoreError.rejected(_, let word, let what) = error else { return "" }
        return what ?? word ?? ""
    }
}

/// "iPhone" or "iPad": how a sentence names this device.
enum HardwareKind {
    static var kind: String { UIDevice.current.userInterfaceIdiom == .pad ? "iPad" : "iPhone" }
}

/// The Access sections for one runner, observing their model.
struct PeopleAccessBlock: View {
    @ObservedObject var model: PeopleAccessModel
    let runner: String

    var body: some View {
        if let access = model.access {
            PeopleAccessSections(access: access, runner: runner, actions: model.actions(), shownCode: model.shownCode)
            if let note = model.note {
                Section {
                    Text(note).foregroundStyle(.red).fixedSize(horizontal: false, vertical: true)
                        .accessibilityIdentifier("people-access-note")
                }
            }
        }
    }
}
