import Foundation

extension Account {
    /// The directory, reached with this account's session.
    ///
    /// Each call refreshes the bearer once if the relay refuses it, as every
    /// other authenticated call does, and answers with the relay's status and
    /// body rather than turning a 4xx into a failure, because the directory's
    /// refusals (a 409, three kinds of 403) are the news.
    ///
    /// Nothing from a body is logged. A failure to get an answer at all is
    /// logged by status only, by `note`'s rule, like the rest.
    public var directoryClient: DirectoryClient {
        DirectoryClient { path, body in
            await Self.authenticating(
                credential: { await self.credential(forceRefresh: $0) },
                request: { bearer in
                    let (status, data) = try await self.send(path, body, bearer: bearer)
                    // The only status that can be repaired by asking again.
                    if status == 401 { throw AccountError.unauthorized }
                    let object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
                    return DirectoryReply(status: status, body: object ?? [:])
                })
        }
    }
}
