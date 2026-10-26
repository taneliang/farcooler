import SwiftUI

/// A shared runner's people, in its settings (ov-508): whose Claude account
/// the work here runs on, then each member with where their rep works. The
/// Mac's section of the same name, in the same words (`RunnerPeople`).
struct RunnerPeopleSection: View {
    let people: RunnerPeople
    let workspaceNames: [String: String]
    /// Whether the runner signs each person in to Claude (ov-584), which
    /// changes what the head of People says about whose login the work uses.
    var signInOffered = false
    /// Give a member a role (ov-784): the owner's alone, on a runner with
    /// roles (`RunnerPeople.offersRoles`); nil draws no picker.
    var setRole: ((String, RunnerPeople.Role) async -> Void)?

    var body: some View {
        Section {
            if let line = people.accountLine(signInOffered: signInOffered) {
                Text(line)
                    .accessibilityIdentifier("runner-people-account")
            }
            ForEach(people.members) { member in
                row(member)
            }
        } header: {
            Text("People")
        } footer: {
            VStack(alignment: .leading, spacing: 6) {
                Text(RunnerPeople.repPurpose)
                if setRole != nil { Text(RunnerPeople.rolesPurpose) }
            }
        }
    }

    /// One member: their name, their handle and where their rep works,
    /// their Claude login, and You on this phone's own.
    private func row(_ member: RunnerPeople.Member) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            VStack(alignment: .leading, spacing: 2) {
                // Two lines on a phone: a name cut short is the one
                // thing that says who the row is.
                Text(member.name)
                    .lineLimit(2)
                // Three: a teammate's email can be most of a line.
                Text(
                    RunnerPeople.detail(
                        of: member, reps: RunnerPeople.reps(of: member, workspaceNames: workspaceNames),
                        role: setRole == nil)
                )
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(3)
                // A member's Claude login (ov-584): the state, and for
                // the owner the account's email, cut in the middle so
                // its domain stays.
                if signInOffered, let claude = people.claudeLine(of: member) {
                    Text(claude)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
            }
            Spacer(minLength: 8)
            if people.isYou(member) {
                Text("You")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            // The owner's picker, in the member's own row (ov-784): the name
            // takes what's left and wraps, so the choice stays with whose it is.
            if let setRole {
                RolePicker(member: member, setRole: setRole)
            }
        }
        .accessibilityElement(children: setRole == nil ? .combine : .contain)
        .accessibilityIdentifier("runner-person-\(member.handle)")
    }
}

/// One member's role, as the owner sets it: Member or Maintainer, from a
/// menu, held at the choice while the runner takes it.
private struct RolePicker: View {
    let member: RunnerPeople.Member
    let setRole: (String, RunnerPeople.Role) async -> Void
    @State private var choosing: RunnerPeople.Role?

    var body: some View {
        Picker(selection: Binding(
            get: { choosing ?? member.role ?? .member },
            set: { role in
                choosing = role
                Task {
                    await setRole(member.handle, role)
                    choosing = nil
                }
            })
        ) {
            ForEach(RunnerPeople.Role.allCases) { Text($0.title).tag($0) }
        } label: {
            Text(RunnerPeople.roleLabel)
        }
        .labelsHidden()
        .pickerStyle(.menu)
        .fixedSize()
        .disabled(choosing != nil)
        .accessibilityLabel("\(member.name)’s \(RunnerPeople.roleLabel.lowercased())")
        .accessibilityIdentifier("runner-person-role-\(member.handle)")
    }
}
