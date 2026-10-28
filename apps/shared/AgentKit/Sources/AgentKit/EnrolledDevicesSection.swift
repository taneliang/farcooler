import SwiftUI

/// The devices that may log in to one runner, each with who added it, and a
/// Revoke that offers to take what the device added too (ov-855).
///
/// Shared by the Mac's runner settings and the iPhone's: the rows, the words
/// and the confirmation are the same, and only how the list is read and how a
/// revoke is sent differ, so those come in as closures.
public struct EnrolledDevicesSection: View {
    let devices: EnrolledDevices
    /// Why the last revoke didn't take, if it didn't.
    let failure: String?
    /// Revoke `device`, and with `withAdded` the devices it added too.
    let revoke: (EnrolledDevices.Device, Bool) async -> Void

    @State private var asking: EnrolledDevices.RevokeChoices?

    public init(
        devices: EnrolledDevices, failure: String? = nil,
        revoke: @escaping (EnrolledDevices.Device, Bool) async -> Void
    ) {
        self.devices = devices
        self.failure = failure
        self.revoke = revoke
    }

    public var body: some View {
        Section {
            if devices.devices.isEmpty {
                Text(EnrolledDevices.emptySentence).foregroundStyle(.secondary).font(.callout)
            }
            ForEach(devices.devices) { device in
                HStack {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(device.title)
                        if let line = devices.addedByLine(for: device) {
                            Text(line)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                                .accessibilityIdentifier("enrolled-device-added-by")
                        }
                    }
                    Spacer()
                    Button("Revoke…", role: .destructive) {
                        asking = devices.revokeChoices(for: device)
                    }
                    .buttonStyle(.plain)
                    .foregroundStyle(.secondary)
                }
            }
            if let failure {
                Text(failure).font(.callout).foregroundStyle(.secondary)
            }
        } header: {
            Text(EnrolledDevices.sectionTitle)
        }
        .confirmationDialog(
            asking?.title ?? "",
            isPresented: Binding(get: { asking != nil }, set: { if !$0 { asking = nil } }),
            titleVisibility: .visible,
            presenting: asking
        ) { choices in
            Button(EnrolledDevices.RevokeChoices.justThisTitle, role: .destructive) {
                Task { await revoke(choices.device, false) }
            }
            if let title = choices.withAddedTitle {
                Button(title, role: .destructive) {
                    Task { await revoke(choices.device, true) }
                }
            }
            Button("Cancel", role: .cancel) {}
        } message: { choices in
            Text(choices.message)
        }
    }
}
