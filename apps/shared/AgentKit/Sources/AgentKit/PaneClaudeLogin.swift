import Foundation

/// Whose Claude login a pane's agent runs on (ov-597), as the runner names
/// it, and the sentence a pane's info says about it.
///
/// Present only on a runner where members have logins of their own, so a
/// single-user setup never draws it. The Mac decodes it from the CLI's JSON
/// and the phones from the bridge's; both print
/// `{handle, name, owner}`.
///
/// The words are here so the Mac and the iPhone say the same thing, and the
/// tests pin them once. Android says them in `PaneClaudeLogin.kt`.
public struct PaneClaudeLogin: Equatable, Hashable, Sendable, Decodable {
    /// The member's handle; "" for the runner's owner.
    public let handle: String
    /// The name people read; "" for the owner, whose name the runner doesn't
    /// hold, and from a member with none.
    public let name: String
    /// Whether it is the runner's own login, the owner's.
    public let owner: Bool

    public init(handle: String, name: String, owner: Bool) {
        self.handle = handle
        self.name = name
        self.owner = owner
    }

    /// What the pane's info says: "Uses your Claude login" for the viewer's
    /// own, else whose it is.
    ///
    /// `viewer` is who this device is on the runner, as `RunnerPeople.you`
    /// says it: "" for the owner, a member's handle, nil if not known yet.
    /// Not known reads the pane by name, which is true for everyone; it never
    /// says "your" on a guess.
    public func sentence(viewer: String?) -> String {
        if let viewer, viewer == (owner ? "" : handle), viewer != RunnerPeople.released {
            return "Uses your Claude login"
        }
        if owner { return "Uses the owner’s Claude login" }
        let who = name.trimmingCharacters(in: .whitespaces)
        return "Uses \(who.isEmpty ? handle : who)’s Claude login"
    }
}
