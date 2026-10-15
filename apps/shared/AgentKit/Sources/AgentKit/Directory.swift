import Foundation

/// The team directory, as the Mac reads and writes it (ov-528).
///
/// The relay's directory (`services/relay/src/directory`) is the record of who
/// belongs to which team and which runners each team holds. Until a runner is
/// registered in it, the directory has nothing to say about that runner: no
/// heartbeat's key inventory is stored, and no later feature (offboarding,
/// granting) can name it. This file is the client half of registering one.
///
/// Runner registration is Mac only: phones and Android still pair through
/// `/v1/daemons`. Every app registers its own device's key through it
/// (`DeviceKeyRegistration.swift`, ov-549).

/// One team the signed-in person belongs to, as `directory/me` lists it.
public struct DirectoryTeam: Equatable, Identifiable, Sendable {
    public let id: String
    public let name: String
    /// The team made for this person alone on their first sight of the
    /// directory. A runner joins it unless they choose otherwise (ruling R-67).
    public let isPersonal: Bool
    /// `admin`, `member` or `billing`.
    public let role: String

    public init(id: String, name: String, isPersonal: Bool, role: String) {
        self.id = id
        self.name = name
        self.isPersonal = isPersonal
        self.role = role
    }

    /// Whether this person may add a runner here. A billing member may not
    /// (the relay refuses it with a 403), so the picker never offers the team.
    public var canHoldRunners: Bool { role == "admin" || role == "member" }
}

/// Who `directory/me` says is asking, and their teams.
public struct DirectoryMe: Equatable, Sendable {
    public let account: String
    public let teams: [DirectoryTeam]

    public init(account: String, teams: [DirectoryTeam]) {
        self.account = account
        self.teams = teams
    }

    /// The teams a runner may join, personal team first (the relay's order).
    public var joinable: [DirectoryTeam] { teams.filter(\.canHoldRunners) }

    /// Where a runner goes when nobody chooses.
    public var defaultTeam: DirectoryTeam? { joinable.first(where: \.isPersonal) ?? joinable.first }
}

/// What a directory call can come to, each with the sentence for it.
///
/// A refusal's own code is read from the answer's body (the relay's 403 alone
/// means three things), but only to choose among these cases; no text from a
/// body is ever shown or logged.
public enum DirectoryFailure: Error, Equatable {
    /// The call never got an answer, or the account layer refused it first.
    case account(AccountError)
    /// 404: the team isn't one this person belongs to (any more).
    case teamUnavailable
    /// 403 `forbidden`: their role in that team can't add runners.
    case roleCannotAdd
    /// 403 `stale`: the runner's signature was older than the relay accepts.
    case proofStale
    /// 403 `proof`: the relay didn't accept the signature.
    case proofRefused
    /// 409 `registered`: the runner is in the directory and this person holds no
    /// role on it. It is someone else's.
    case alreadyRegistered
    /// 409 `elsewhere`: it is this person's own runner, in another of their
    /// teams. Registering can't move it; claiming can.
    case inAnotherTeam
    /// 404 on a claim: the team was checked a moment ago, so the runner is
    /// what the relay doesn't have.
    case runnerNotInDirectory
    /// 400 `name`: the relay wouldn't take the name the runner is registered under.
    case nameRefused
    /// 400: the relay couldn't read a field this app sent from the runner.
    case runnerAnswerUnusable
    /// Anything else the relay said.
    case refused(status: Int)

    /// One sentence saying what didn't happen and why.
    public var message: String {
        switch self {
        case .account(let why):
            return why.message
        case .teamUnavailable:
            return "That team isn’t available to you any more, so the runner wasn’t added. Pick another team."
        case .roleCannotAdd:
            return "Your role in that team doesn’t let you add runners, so the runner wasn’t added. "
                + "Ask a team admin, or pick another team."
        case .proofStale:
            return "The runner wasn’t added because its proof was too old by the time it reached the relay. "
                + "Check that the runner’s clock is right, then try again."
        case .proofRefused:
            return "The relay didn’t accept the runner’s proof that it’s yours, so the runner wasn’t added. "
                + "Try again, and if it happens again, check the relay address under Settings ▸ Advanced."
        case .alreadyRegistered:
            return "This runner wasn’t added because someone else registered it. "
                + "Ask them to remove it from their team, then try again."
        case .inAnotherTeam:
            return "This runner wasn’t added because it’s already in another of your teams."
        case .runnerNotInDirectory:
            return "The runner wasn’t moved because the relay doesn’t have it any more. Try adding it again."
        case .nameRefused:
            return "The runner wasn’t added because the relay didn’t accept its name. "
                + "Try again, and if it happens again, rename the computer and retry."
        case .runnerAnswerUnusable:
            return "The runner wasn’t added because its answer didn’t make sense to the relay. "
                + "Reinstall Far Cooler on it, then try again."
        case .refused:
            return "The relay turned this request down, so the runner wasn’t added. Try again in a few minutes."
        }
    }
}

/// One answer from the relay: the status, and the decoded JSON object if there
/// was one.
public struct DirectoryReply {
    public let status: Int
    public let body: [String: Any]

    public init(status: Int, body: [String: Any]) {
        self.status = status
        self.body = body
    }
}

/// The directory's routes this app calls.
///
/// Built around one closure, the way `Account.authenticating` is built around
/// its two, so the whole mapping from a status to a sentence is testable with a
/// relay that is only a script. `Account.directoryClient` supplies the live
/// one.
@MainActor
public struct DirectoryClient {
    public typealias Post = (_ path: String, _ body: [String: Any]) async -> Result<DirectoryReply, AccountError>

    /// Internal, not private: `registerDevice` (DeviceKeyRegistration.swift)
    /// posts through it too.
    let post: Post

    public init(post: @escaping Post) { self.post = post }

    /// The signed-in person and their teams.
    public func me() async -> Result<DirectoryMe, DirectoryFailure> {
        switch await post("/v1/directory/me", [:]) {
        case .failure(let why):
            return .failure(.account(why))
        case .success(let reply):
            // A 404 here is a relay with no directory, not a missing team.
            if reply.status == 404 { return .failure(.account(.endpointMissing)) }
            guard reply.status == 200 else { return .failure(Self.failure(reply)) }
            guard let user = reply.body["user"] as? [String: Any], let account = user["id"] as? String,
                !account.isEmpty, let rows = reply.body["teams"] as? [[String: Any]]
            else { return .failure(.account(.malformedResponse)) }
            let teams = rows.compactMap { row -> DirectoryTeam? in
                guard let id = row["id"] as? String, let name = row["name"] as? String,
                    let role = row["role"] as? String
                else { return nil }
                return DirectoryTeam(
                    id: id, name: name, isPersonal: row["personal"] as? Bool ?? false, role: role)
            }
            return .success(DirectoryMe(account: account, teams: teams))
        }
    }

    /// The directory's own answers about who shares a team with the
    /// signed-in person, for the ceremony's teammate check (ov-525): its
    /// answer to `me`, and its answer to `teams/members` for each team that
    /// isn't personal, as `{"me": …, "rosters": […]}`.
    ///
    /// Passed on as the relay sent them, unjudged: which teams and roles
    /// count is `crates/client/src/ceremony/teammate.rs`'s to decide. A
    /// roster the relay refused is left out, and the core treats a team
    /// it needed and doesn't have as an answer it can't use. Nil when `me`
    /// itself didn't answer, which the core refuses as unreachable.
    public func teamDirectory() async -> Data? {
        guard case .success(let me) = await post("/v1/directory/me", [:]), me.status == 200 else { return nil }
        var rosters: [[String: Any]] = []
        for team in me.body["teams"] as? [[String: Any]] ?? [] where team["personal"] as? Bool == false {
            guard let id = team["id"] as? String else { continue }
            if case .success(let roster) = await post("/v1/directory/teams/members", ["team": id]),
                roster.status == 200
            {
                rosters.append(roster.body)
            }
        }
        return try? JSONSerialization.data(withJSONObject: ["me": me.body, "rosters": rosters])
    }

    /// Add a runner to a team, or rotate the token of one that is already
    /// there, and hand back the token that makes it notify its owner.
    ///
    /// The token is the only value returned, and it is returned as a bare
    /// `String` for the caller to spend at once: it exists in readable form
    /// here and nowhere else, since the relay keeps only its hash.
    public func register(
        team: String, name: String, proof: RunnerProof
    ) async -> Result<String, DirectoryFailure> {
        await tokenFrom("/v1/directory/runners/register", team: team, name: name, proof: proof)
    }

    /// Move a runner to `team`: the runner's own signed claim, which the relay
    /// takes as enough. Only ever asked after `register` said `elsewhere` (the
    /// runner is this person's) and they confirmed, because a claim drops
    /// every role anyone else had on the runner.
    public func claim(
        team: String, name: String, proof: RunnerProof
    ) async -> Result<String, DirectoryFailure> {
        await tokenFrom("/v1/directory/runners/claim", team: team, name: name, proof: proof, claiming: true)
    }

    private func tokenFrom(
        _ path: String, team: String, name: String, proof: RunnerProof, claiming: Bool = false
    ) async -> Result<String, DirectoryFailure> {
        let body: [String: Any] = [
            "team": team, "name": Self.limited(name), "install": proof.install,
            "runnerKey": proof.runnerKey, "issuedAt": proof.issuedAt, "proof": proof.signature,
        ]
        switch await post(path, body) {
        case .failure(let why):
            return .failure(.account(why))
        case .success(let reply):
            guard reply.status == 200 else { return .failure(Self.failure(reply, claiming: claiming)) }
            guard let token = reply.body["token"] as? String, !token.isEmpty else {
                return .failure(.account(.malformedResponse))
            }
            return .success(token)
        }
    }

    /// `name` cut to what the relay accepts: 80 UTF-16 units, which is how its
    /// `nameOf` counts, and never in the middle of a character.
    static func limited(_ name: String) -> String {
        var out = ""
        var units = 0
        for character in name {
            units += character.utf16.count
            if units > 80 { break }
            out.append(character)
        }
        return out
    }

    /// A non-200 answer, named by its status and the relay's own short code.
    static func failure(_ reply: DirectoryReply, claiming: Bool = false) -> DirectoryFailure {
        let code = reply.body["error"] as? String
        switch (reply.status, code) {
        case (404, _): return claiming ? .runnerNotInDirectory : .teamUnavailable
        case (403, "stale"): return .proofStale
        case (403, "proof"): return .proofRefused
        case (403, _): return .roleCannotAdd
        case (409, "elsewhere"): return .inAnotherTeam
        case (409, _): return .alreadyRegistered
        case (400, "team"): return .teamUnavailable
        case (400, "name"): return .nameRefused
        case (400, _): return .runnerAnswerUnusable
        case (429, _): return .account(.rateLimited)
        case (500...599, _): return .account(.relayFailed(status: reply.status))
        default: return .refused(status: reply.status)
        }
    }
}

/// What a runner's own signer printed: the fields the relay's `register`
/// takes beside `team` and `name`.
///
/// None of it is secret (a public key and a signature over a statement that
/// expires in ten minutes), but none of it is logged either.
public struct RunnerProof: Equatable {
    public let install: String
    public let runnerKey: String
    public let issuedAt: Int64
    public let signature: String

    public init(install: String, runnerKey: String, issuedAt: Int64, signature: String) {
        self.install = install
        self.runnerKey = runnerKey
        self.issuedAt = issuedAt
        self.signature = signature
    }

    /// The JSON `farcooler runner sign-registration` prints, or nil when it
    /// isn't that.
    ///
    /// The last line that starts with `{`, because a login shell that prints a
    /// banner (a MOTD, a conda or nvm line) puts it ahead of the answer.
    public init?(printed: String) {
        let line = printed.split(whereSeparator: \.isNewline).last { $0.hasPrefix("{") }
        guard let line, let data = String(line).data(using: .utf8),
            let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let install = object["install"] as? String, !install.isEmpty,
            let key = object["runnerKey"] as? String, !key.isEmpty,
            let issued = object["issuedAt"] as? NSNumber,
            let proof = object["proof"] as? String, !proof.isEmpty
        else { return nil }
        self.init(install: install, runnerKey: key, issuedAt: issued.int64Value, signature: proof)
    }
}
