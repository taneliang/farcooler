import AgentKit
import SwiftUI

/// A shared runner's people, in its settings sheet (ov-508): whose Claude
/// account the work here runs on, then each member with where their rep works.
///
/// In the runner's settings rather than the window's sidebar, because a
/// member is a member of the runner, not of one workspace (the team design,
/// §2), and the sidebar is for work. A rep is per workspace, so each member's
/// row names the boards theirs work in. The owner isn't listed: `member.list`
/// is everyone but them, and the account line already says whose it is.
struct RunnerPeopleSection: View {
    let people: RunnerPeople
    let workspaceNames: [String: String]

    var body: some View {
        Section {
            if let line = people.accountLine(signInOffered: people.offersClaudeSignIn) {
                Text(line)
                    .fixedSize(horizontal: false, vertical: true)
                    .identified("runner-people-account")
            }
            ForEach(people.members) { member in
                MemberRow(
                    member: member, you: people.isYou(member),
                    reps: RunnerPeople.reps(of: member, workspaceNames: workspaceNames),
                    claude: people.offersClaudeSignIn ? people.claudeLine(of: member) : nil)
            }
        } header: {
            Text("People")
        } footer: {
            Text(RunnerPeople.repPurpose)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    /// Each workspace's name by id on `target`'s runner, as this Mac last read
    /// its fleet: the Settings window has no client of its own.
    @MainActor
    static func workspaceNames(on target: String, in fleet: FleetStore = .shared) -> [String: String] {
        let summaries = fleet.clients[target]?.fleet.workspaces ?? []
        return Dictionary(summaries.map { ($0.id, $0.name) }, uniquingKeysWith: { first, _ in first })
    }
}

/// One member: their name, then their handle and where their rep works.
private struct MemberRow: View {
    let member: RunnerPeople.Member
    let you: Bool
    let reps: String
    /// Their Claude login (ov-584): the state, and for the owner the email.
    var claude: String?

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            VStack(alignment: .leading, spacing: 2) {
                Text(member.name)
                    .lineLimit(1)
                    .truncationMode(.tail)
                Text(RunnerPeople.detail(of: member, reps: reps))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(3)
                    .fixedSize(horizontal: false, vertical: true)
                if let claude {
                    // Cut in the middle, so an email keeps its domain.
                    Text(claude)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
            }
            Spacer(minLength: 8)
            if you {
                Text("You")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .accessibilityElement(children: .combine)
        .identified("runner-person-\(member.handle)")
    }
}
