import FarCoolerClient
import Foundation
import SwiftUI

/// The iPhone's side of a Mac's ask (ov-858): a Mac that isn't the runner holds
/// no step-up key, so the runner asks this phone to approve its admin action.
///
/// **The phone must show the action.** Nothing here can be granted from a
/// notification or the lock screen: the push only says there is something to
/// review. The request is listed over the phone's own connection, drawn from
/// the very bytes it signs (the client core decodes them), with the asking
/// Mac's name, and the grant is the step-up key's signature, which asks for
/// Face ID or the passcode itself. A phone with no trusted key lists nothing.
extension Connection {
    /// What this person's Macs are waiting on, newest first. Empty on a runner
    /// that doesn't serve it, on a phone with no step-up key, and while the
    /// runner's host key is one this phone hasn't pinned.
    func pendingApprovals() async -> [PhoneApprovalItem] {
        guard phase == .connected, daemon?.can(.phoneApproval) == true, StepUpSigners.shared.publicKey != nil,
            let pin = hostPin, pin.hasPrefix("SHA256:"),
            let listed = try? await rpc("approval.list", ["hostPin": pin])
        else { return [] }
        return PhoneApprovalItem.items(in: listed)
    }

    /// Sign `item` with this phone's key, after Face ID, and tell the runner.
    func approve(_ item: PhoneApprovalItem) async throws {
        guard let pin = hostPin, pin.hasPrefix("SHA256:") else { throw StepUpError.keyUnavailable }
        let signature = try await StepUpSigners.shared.sign(item.statement, reason: item.sentence(runner: hostLabel))
        _ = try await rpc("approval.grant", ["id": item.id, "hostPin": pin, "signature": signature.base64EncodedString()])
    }

    /// Tell the runner the person said no.
    func decline(_ item: PhoneApprovalItem) async {
        _ = try? await rpc("approval.deny", ["id": item.id])
    }
}

/// The approval on screen, if any, and what to do with it.
@MainActor
final class ApprovalWatcher: ObservableObject {
    struct Asked: Identifiable {
        let item: PhoneApprovalItem
        let connection: Connection
        var id: String { item.id }
    }

    @Published var asked: Asked?
    /// A sentence for an approval that didn't go through, or nil.
    @Published var note: String?
    private var settled: Set<String> = []

    /// Look at every connection once; the first approval not yet decided is shown.
    func look(in connections: [Connection]) async {
        guard asked == nil else { return }
        for connection in connections {
            if let next = await connection.pendingApprovals().first(where: { !settled.contains($0.id) }) {
                asked = Asked(item: next, connection: connection)
                return
            }
        }
    }

    func approve() async {
        guard let current = asked else { return }
        settled.insert(current.id)
        do {
            try await current.connection.approve(current.item)
            note = nil
        } catch let error as StepUpError {
            note = StepUpWords.sentence(for: error, runner: current.connection.hostLabel, device: HardwareKind.kind)
        } catch {
            note = StepUpWords.sentence(
                forRefusal: StepUpRefusals.word(error), runner: current.connection.hostLabel, device: HardwareKind.kind)
        }
        asked = nil
    }

    func decline() async {
        guard let current = asked else { return }
        settled.insert(current.id)
        asked = nil
        await current.connection.decline(current.item)
    }
}

/// The sheet: what is being approved, on which runner, and which Mac asked.
struct ApprovalSheet: View {
    let asked: ApprovalWatcher.Asked
    let approve: () -> Void
    let decline: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(PhoneApprovalWords.title).font(.title2.weight(.semibold))
                .accessibilityIdentifier("approval-title")
            Text(asked.item.sentence(runner: asked.connection.hostLabel))
                .font(.title3).fixedSize(horizontal: false, vertical: true)
                .accessibilityIdentifier("approval-action")
            Text(PhoneApprovalWords.asked(by: asked.item.mac))
                .font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                .accessibilityIdentifier("approval-asked-by")
            Spacer(minLength: 0)
            Button(action: approve) { Text(PhoneApprovalWords.approve).frame(maxWidth: .infinity) }
                .buttonStyle(.borderedProminent).controlSize(.large)
                .accessibilityIdentifier("approval-approve")
            Button(action: decline) { Text(PhoneApprovalWords.decline).frame(maxWidth: .infinity) }
                .buttonStyle(.bordered).controlSize(.large)
                .accessibilityIdentifier("approval-decline")
        }
        .padding(24)
        .presentationDetents([.medium])
        .interactiveDismissDisabled()
    }
}

/// Looks for approvals whenever the app is on screen (a push opens it here),
/// and shows the first as a sheet.
struct ApprovalPrompts: ViewModifier {
    @ObservedObject var fleet: FleetStore
    @Environment(\.scenePhase) private var phase
    @StateObject private var watcher = ApprovalWatcher()

    func body(content: Content) -> some View {
        content
            .task(id: phase) {
                while phase == .active, !Task.isCancelled {
                    await watcher.look(in: fleet.active)
                    try? await Task.sleep(nanoseconds: 3_000_000_000)
                }
            }
            .sheet(item: $watcher.asked) { asked in
                ApprovalSheet(
                    asked: asked, approve: { Task { await watcher.approve() } },
                    decline: { Task { await watcher.decline() } })
            }
            .alert(
                "Couldn’t Approve", isPresented: Binding(get: { watcher.note != nil }, set: { if !$0 { watcher.note = nil } })
            ) {
                Button("OK", role: .cancel) {}
            } message: {
                Text(watcher.note ?? "")
            }
    }
}
