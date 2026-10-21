import AgentKit
import SwiftUI

/// Whether members sign in to each agent with their own account on a shared
/// runner (ov-652): one switch per agent, which only the runner's admins can
/// turn, and everyone else reads. The iPhone's section of the same name says
/// the same words (`MemberOwnLogins`).
struct MemberLoginsSection: View {
    let logins: MemberOwnLogins
    /// This device is the runner owner's, whose account a shared login is.
    let owner: Bool
    /// Turn one switch. Nil where this device may not.
    let set: ((MemberOwnLogins.Agent, Bool) async -> Void)?

    /// A change on its way: the switches wait, so two flips can't land out
    /// of order.
    @State private var changing = false

    var body: some View {
        Section {
            ForEach(MemberOwnLogins.Agent.allCases) { agent in
                let on = logins.requiresOwn(agent)
                Toggle(isOn: Binding(get: { on }, set: { flip(agent, to: $0) })) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(MemberOwnLogins.toggleTitle(agent))
                        Text(MemberOwnLogins.stateLine(agent, on: on))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                .disabled(set == nil || changing)
                .identified("member-own-login-\(agent.rawValue)")
            }
        } header: {
            Text(MemberOwnLogins.sectionTitle)
        } footer: {
            Text(MemberOwnLogins.footer(owner: owner, mayChange: set != nil))
                .fixedSize(horizontal: false, vertical: true)
                .identified("member-own-logins-footer")
        }
    }

    private func flip(_ agent: MemberOwnLogins.Agent, to on: Bool) {
        guard let set, !changing else { return }
        changing = true
        Task {
            await set(agent, on)
            changing = false
        }
    }
}
