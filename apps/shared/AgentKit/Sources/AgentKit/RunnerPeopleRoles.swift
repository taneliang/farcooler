import Foundation

/// Roles on a shared runner (ov-784, slice 2 of ov-753), and what each lets
/// this device do, so the Mac, the iPhone and their tests ask one place.
///
/// The runner decides all of it (`people::gate`); these rules only keep an
/// app from offering a control the runner would refuse.
///
/// - **The owner** gives roles, and does everything else.
/// - **A Maintainer** edits any workspace's charter, renames a workspace,
///   changes its prefix, and sets the settings that don't act as the owner.
/// - **A workspace's lead** edits that workspace's charter.
/// - **A member** reads the charter, and changes none of these.
/// - **A removed member's device** reads nothing, and is told so.
extension RunnerPeople {
    /// What the owner has made a member: `member.list`'s `role`.
    public enum Role: String, CaseIterable, Identifiable, Sendable {
        case member
        case maintainer

        public var id: String { rawValue }

        /// The word in the picker and on a Maintainer's row.
        public var title: String {
            switch self {
            case .member: return "Member"
            case .maintainer: return "Maintainer"
            }
        }
    }

    /// Whether this device is the runner's owner's: the one that gives roles.
    public var isOwner: Bool { you.isEmpty }

    /// Whether this device was a member's and the runner removed them: it
    /// reads nothing there, and its app says so.
    public var isReleased: Bool { you == Self.released }

    /// This device's member's role, or nil for the owner, a removed member's
    /// device, and a runner before roles.
    public var yourRole: Role? {
        guard isMember else { return nil }
        return members.first { $0.handle == you }?.role
    }

    /// Whether this device may rename a workspace, change its prefix, edit
    /// any charter, and set the settings that don't act as the owner: the
    /// owner's, and a Maintainer's.
    public var keepsWorkspaces: Bool { isOwner || yourRole == .maintainer }

    /// Whether this device may edit `workspace`'s charter: the owner, a
    /// Maintainer, or the member who leads it. The lead is "" or "owner"
    /// for the owner's lead (`WorkspaceSummary.lead`).
    public func editsCharter(of workspace: WorkspaceSummary) -> Bool {
        keepsWorkspaces || (isMember && workspace.lead == you)
    }

    /// Whether the People list offers a role picker on each member: to the
    /// owner alone, on a runner that keeps roles (`member_roles`).
    public func offersRoles(runnerHasRoles: Bool) -> Bool { isOwner && runnerHasRoles }

    /// What the role picker is for, under the People list.
    public static let rolesPurpose =
        "A Maintainer keeps the team’s workspaces in order. They can edit any charter, rename a workspace, "
        + "change its task prefix, and set its pull request size, cost line, nudge hours and wake on answer. "
        + "Only you can make someone a Maintainer."

    /// The picker's label, read out with each member's name.
    public static let roleLabel = "Role"

    /// A role change the runner didn't take.
    public static let roleNotSaved = "The runner didn’t change that role. Check that this device is still the owner’s, then try again."

    /// A device whose member was removed from the runner: the screen in
    /// place of the runner's work, on every app. It names nobody: the owner
    /// or any runner admin may have done it (`member.remove` is a runner
    /// admin's). The owner can always add them back, so they're who to ask.
    public static let removedTitle = "You Were Removed from This Runner"
    public static let removedBody =
        "Someone who manages this runner removed you, so this device can’t see the runner’s work anymore. "
        + "To come back, ask the runner’s owner to add you again."

    /// The title where several runners may be on screen: "You Were Removed
    /// from studio".
    public static func removedTitle(from runner: String) -> String {
        runner.isEmpty ? removedTitle : "You Were Removed from \(runner)"
    }
}
