import SwiftUI

/// What the People screen can do, per platform (ov-856). A closure left nil
/// draws no control for it, so a screen never offers what the runner would
/// refuse from this device.
public struct PeopleAccessActions {
    /// Remove a device, and with it what it added when `withAdded`.
    public var removeDevice: ((PeopleAccess.Device, Bool) async -> Void)?
    /// Hold a device at Viewer (the runner's own Mac sets it).
    public var limitToViewer: ((PeopleAccess.Device) async -> Void)?
    /// Give a person another Access (the runner's own Mac sets it).
    public var setAccess: ((PeopleAccess.Person, PersonAccess) async -> Void)?
    /// Trust a device's waiting key.
    public var approve: ((PeopleAccess.Device) async -> Void)?
    /// Offer this device's own key for approval.
    public var allowAdminActions: (() async -> Void)?
    /// This device asks the person's iPhone to approve its admin actions
    /// (ov-858), as a Mac that isn't the runner does: it keeps no key of its own.
    public var asksPhone: Bool

    public init(
        removeDevice: ((PeopleAccess.Device, Bool) async -> Void)? = nil,
        limitToViewer: ((PeopleAccess.Device) async -> Void)? = nil,
        setAccess: ((PeopleAccess.Person, PersonAccess) async -> Void)? = nil,
        approve: ((PeopleAccess.Device) async -> Void)? = nil,
        allowAdminActions: (() async -> Void)? = nil,
        asksPhone: Bool = false
    ) {
        self.asksPhone = asksPhone
        self.removeDevice = removeDevice
        self.limitToViewer = limitToViewer
        self.setAccess = setAccess
        self.approve = approve
        self.allowAdminActions = allowAdminActions
    }
}

/// The People screen's sections (ov-856, §8): each person with their Access
/// and their devices under them, then **Check These Devices** and **Waiting
/// for Approval** when they have any. The same view on the Mac's Settings and
/// the iPhone's Runner > People, so the words and the order are one.
public struct PeopleAccessSections: View {
    let access: PeopleAccess
    let runner: String
    let actions: PeopleAccessActions
    /// The code this device shows while its own key waits, when it just asked.
    let shownCode: String?
    /// The iPhone has been asked, and the Mac is waiting for its answer.
    let waitingOnPhone: Bool

    @State private var removing: PeopleAccess.Device?

    public init(access: PeopleAccess, runner: String, actions: PeopleAccessActions, shownCode: String? = nil, waitingOnPhone: Bool = false) {
        self.waitingOnPhone = waitingOnPhone
        self.access = access
        self.runner = runner
        self.actions = actions
        self.shownCode = shownCode
    }

    public var body: some View {
        Group {
            Section {
                ForEach(access.people) { person in
                    PersonRows(person: person, access: access, runner: runner, actions: actions, remove: { removing = $0 })
                }
            } header: {
                Text("Access")
            } footer: {
                Text("Admins can add and remove people and their devices. A Viewer can see the board, never a terminal.")
                    .fixedSize(horizontal: false, vertical: true)
            }
            if !access.toCheck.isEmpty {
                Section {
                    ForEach(access.toCheck) { DeviceRow(device: $0, access: access, runner: runner, actions: actions, remove: { removing = $0 }) }
                } header: {
                    Text(PeopleAccess.checkHeader).accessibilityIdentifier("people-check")
                } footer: {
                    Text(PeopleAccess.checkFooter).fixedSize(horizontal: false, vertical: true)
                }
            }
            if !access.shownWaiting.isEmpty {
                Section {
                    ForEach(access.shownWaiting) { WaitingRow(device: $0, access: access, actions: actions) }
                } header: {
                    Text(PeopleAccess.waitingHeader).accessibilityIdentifier("people-waiting")
                } footer: {
                    Text(PeopleAccess.waitingFooter(isSocket: access.isSocket)).fixedSize(horizontal: false, vertical: true)
                }
            }
            if let me = access.thisDevice, access.yourAccess == .owner || access.yourAccess == .admin {
                if actions.asksPhone {
                    PhoneApprovesSection(waiting: waitingOnPhone)
                } else {
                    ThisDeviceAdminSection(device: me, shownCode: shownCode, actions: actions)
                }
            }
        }
        .confirmationDialog(
            removing.map { "Remove \($0.name)?" } ?? "", isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } }),
            titleVisibility: .visible, presenting: removing
        ) { device in
            Button("Remove Device", role: .destructive) { run(device, false) }
            if device.added > 0 {
                Button("Remove Device and \(device.added) It Added", role: .destructive) { run(device, true) }
            }
            Button("Cancel", role: .cancel) {}
        } message: { device in
            Text(Self.removalMessage(device, runner: runner))
        }
    }

    /// What removing a device does, said before it is done.
    public static func removalMessage(_ device: PeopleAccess.Device, runner: String) -> String {
        var line = "\(device.name) loses its access to \(runner) right away."
        if device.hasShell { line += " Its shell goes too: it can no longer use ssh, git or Zed here." }
        if device.added > 0 {
            line += device.added == 1 ? " It added one other device." : " It added \(device.added) other devices."
        }
        return line
    }

    private func run(_ device: PeopleAccess.Device, _ withAdded: Bool) {
        Task { await actions.removeDevice?(device, withAdded) }
    }
}

private struct PersonRows: View {
    let person: PeopleAccess.Person
    let access: PeopleAccess
    let runner: String
    let actions: PeopleAccessActions
    let remove: (PeopleAccess.Device) -> Void
    @State private var choosing: PersonAccess?

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(person.name).lineLimit(2)
                        .accessibilityIdentifier("people-person-\(person.handle.isEmpty ? "owner" : person.handle)")
                    Text(person.access.summary)
                        .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: 8)
                if person.isYou && person.access != .owner { Text("You").font(.caption).foregroundStyle(.secondary) }
                accessControl
            }
            ForEach(person.devices) { device in
                DeviceRow(device: device, access: access, runner: runner, actions: actions, remove: remove)
                    .padding(.leading, 14)
            }
        }
        .accessibilityElement(children: .contain)
    }

    @ViewBuilder private var accessControl: some View {
        let choices = access.accessChoices(for: person)
        if let setAccess = actions.setAccess, !choices.isEmpty {
            Picker("Access", selection: Binding(get: { choosing ?? person.access }, set: { new in
                guard new != person.access else { return }
                choosing = new
                Task { await setAccess(person, new); choosing = nil }
            })) {
                ForEach(Array(Set(choices + [person.access])).sorted { $0.rawValue < $1.rawValue }) { Text($0.title).tag($0) }
            }
            .labelsHidden().pickerStyle(.menu).fixedSize().disabled(choosing != nil)
            .accessibilityLabel("\(person.name)’s access")
            .accessibilityIdentifier("people-access-\(person.handle)")
        } else {
            Text(person.access.title).font(.callout).foregroundStyle(.secondary)
                .accessibilityIdentifier("people-access-\(person.handle.isEmpty ? "owner" : person.handle)")
        }
    }
}

private struct DeviceRow: View {
    let device: PeopleAccess.Device
    let access: PeopleAccess
    let runner: String
    let actions: PeopleAccessActions
    let remove: (PeopleAccess.Device) -> Void

    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            VStack(alignment: .leading, spacing: 3) {
                Text(device.name).font(.subheadline).lineLimit(2).accessibilityIdentifier("device-\(device.id)")
                if let by = device.addedBy {
                    Text(by).font(.caption).foregroundStyle(.secondary).lineLimit(2)
                }
                if device.viewerOnThisDevice {
                    Text(PeopleAccess.viewerBadge)
                        .font(.caption2.weight(.medium))
                        .padding(.horizontal, 6).padding(.vertical, 2)
                        .background(.quaternary, in: Capsule())
                        .accessibilityIdentifier("device-viewer-\(device.id)")
                }
                if device.hasShell {
                    HStack(spacing: 4) {
                        Image(systemName: "exclamationmark.triangle.fill")
                        Text(PeopleAccess.shellBadge)
                    }
                    .font(.caption.weight(.medium)).foregroundStyle(.orange)
                    Text(PeopleAccess.shellWarning(runner: runner))
                        .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                        .accessibilityIdentifier("device-shell-\(device.id)")
                }
            }
            Spacer(minLength: 4)
            if actions.removeDevice != nil || actions.limitToViewer != nil {
                Menu {
                    if let limit = actions.limitToViewer, !device.viewerOnThisDevice, access.mayLimit(device) {
                        Button(PeopleAccess.limitToViewer) { Task { await limit(device) } }
                    }
                    if actions.removeDevice != nil {
                        Button(PeopleAccess.removeDevice, role: .destructive) { remove(device) }
                    }
                } label: {
                    Image(systemName: "ellipsis.circle").imageScale(.large)
                }
                .menuStyle(.button).buttonStyle(.borderless).fixedSize()
                .accessibilityLabel("\(device.name) options")
                .accessibilityIdentifier("device-menu-\(device.id)")
            }
        }
        .accessibilityElement(children: .contain)
    }
}

private struct WaitingRow: View {
    let device: PeopleAccess.Device
    let access: PeopleAccess
    let actions: PeopleAccessActions

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                VStack(alignment: .leading, spacing: 2) {
                    Text(device.name).lineLimit(2).accessibilityIdentifier("waiting-\(device.id)")
                    if !device.personName.isEmpty {
                        Text(device.personName).font(.caption).foregroundStyle(.secondary)
                            .accessibilityIdentifier("waiting-person-\(device.id)")
                    }
                }
                Spacer(minLength: 8)
                Text(PeopleAccess.spoken(device.code))
                    .font(.title3.monospacedDigit().weight(.semibold))
                    .accessibilityLabel("Code \(device.code.map(String.init).joined(separator: " "))")
                    .accessibilityIdentifier("waiting-code-\(device.id)")
            }
            Text(PeopleAccess.approvalPrompt(for: device))
                .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            if let approve = actions.approve, access.mayAllow(device) {
                Button("Allow") { Task { await approve(device) } }
                    .buttonStyle(.bordered)
                    .accessibilityIdentifier("waiting-allow-\(device.id)")
            }
        }
        .accessibilityElement(children: .contain)
    }
}

/// This Mac's place in admin actions (ov-858): it keeps no key, so the
/// person's iPhone approves each one, and says so while it waits.
private struct PhoneApprovesSection: View {
    let waiting: Bool

    var body: some View {
        Section {
            if waiting {
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text(PhoneApprovalWords.waiting)
                }
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("phone-approval-waiting")
            } else {
                Label("Your iPhone Approves Admin Actions", systemImage: "iphone")
                    .foregroundStyle(.secondary)
                    .accessibilityIdentifier("phone-approval-on")
            }
        } header: {
            Text("This Device")
        } footer: {
            Text("Changes to who has access ask your iPhone first. Open Far Cooler there and approve with Face ID.")
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

private struct ThisDeviceAdminSection: View {
    let device: PeopleAccess.Device
    let shownCode: String?
    let actions: PeopleAccessActions

    var body: some View {
        Section {
            switch device.stepUp {
            case .trusted:
                Label("Admin Actions Are On", systemImage: "checkmark.circle.fill").foregroundStyle(.secondary)
                    .accessibilityIdentifier("admin-actions-on")
            case .proposed:
                waiting(code: shownCode ?? device.code)
            case .none, .expired:
                if let shownCode { waiting(code: shownCode) } else if let allow = actions.allowAdminActions {
                    Button("Allow Admin Actions") { Task { await allow() } }
                        .accessibilityIdentifier("admin-actions-allow")
                }
            }
        } header: {
            Text("This Device")
        } footer: {
            Text(Self.footer(for: device.stepUp)).fixedSize(horizontal: false, vertical: true)
        }
    }

    private func waiting(code: String) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Approve on another device").font(.subheadline)
            Text(PeopleAccess.spoken(code)).font(.title2.monospacedDigit().weight(.semibold))
                .accessibilityIdentifier("admin-actions-code")
        }
        .accessibilityElement(children: .contain)
    }

    static func footer(for standing: StepUpKeyStanding) -> String {
        switch standing {
        case .trusted: "Changes to who has access ask for Face ID or your passcode first."
        case .proposed: "Open People on another of your devices and allow this one. It checks this code."
        case .none, .expired:
            "Changes to who has access then ask for Face ID or your passcode on this device."
        }
    }
}
