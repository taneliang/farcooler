import SwiftUI

/// A shared runner's people, in its settings (ov-508): whose Claude account
/// the work here runs on, then each member with where their rep works. The
/// Mac's section of the same name, in the same words (`RunnerPeople`).
struct RunnerPeopleSection: View {
    let people: RunnerPeople
    let workspaceNames: [String: String]

    var body: some View {
        Section {
            Text(people.accountLine)
                .accessibilityIdentifier("runner-people-account")
            ForEach(people.members) { member in
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    VStack(alignment: .leading, spacing: 2) {
                        // Two lines on a phone: a name cut short is the one
                        // thing that says who the row is.
                        Text(member.name)
                            .lineLimit(2)
                        // Three: a teammate's email can be most of a line.
                        Text(
                            RunnerPeople.detail(
                                of: member, reps: RunnerPeople.reps(of: member, workspaceNames: workspaceNames))
                        )
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(3)
                    }
                    Spacer(minLength: 8)
                    if people.isYou(member) {
                        Text("You")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("runner-person-\(member.handle)")
            }
        } header: {
            Text("People")
        } footer: {
            Text(RunnerPeople.repPurpose)
        }
    }
}
