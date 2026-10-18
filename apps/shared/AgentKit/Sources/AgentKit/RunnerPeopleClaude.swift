import Foundation

/// What People says about each person's Claude login (ov-584), and whether
/// this device gets the Claude section. The sign-in itself is
/// `ClaudeSignIn` and `ClaudeSignInModel`.
extension RunnerPeople {
    /// This device's own Claude login, for the Claude section above People:
    /// only for a member on their own device, and only where the runner can
    /// sign them in (`claude_sign_in`). The owner never has one here, and a
    /// released device is nobody's. A member the list doesn't show reads as
    /// not signed in.
    public func ownClaudeLogin(signInOffered: Bool) -> ClaudeLogin? {
        guard signInOffered, isMember else { return nil }
        return members.first { $0.handle == you }?.claudeLogin ?? .signedOut
    }

    /// The line under a member's name about their Claude login, or nil where
    /// there is nothing to say: the runner doesn't know logins, or the row is
    /// this device's own, which the Claude section covers.
    ///
    /// The owner reads the account's email; any other member reads the state
    /// alone, whatever the runner sent.
    public func claudeLine(of member: Member) -> String? {
        guard let login = member.claudeLogin, !isYou(member) else { return nil }
        switch login.state {
        case .none: return ClaudeSignIn.notSignedInLine
        case .stoppedWorking: return ClaudeSignIn.stoppedWorkingLine
        case .signedIn:
            return you.isEmpty ? ClaudeSignIn.signedInLine(email: login.email) : "Signed in to Claude"
        }
    }

    /// The line at the head of People. Where the runner keeps a login for
    /// each person it says so, to everyone but a member, whose Claude section
    /// says it; before that it names the one account all the work ran on.
    public func accountLine(signInOffered: Bool) -> String? {
        guard signInOffered else { return accountLine }
        // The owner's line is the owner's: a released device is nobody's and
        // reads the account as someone else's, as it did before sign-in.
        if isMember { return nil }
        return you.isEmpty ? ClaudeSignIn.ownerTrustLine : accountLine
    }
}
