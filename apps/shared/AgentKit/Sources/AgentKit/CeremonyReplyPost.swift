import Foundation

/// How posting a sealed ceremony reply ended (ov-770).
public enum CeremonyReplyPost: Equatable, Sendable {
    /// The relay holds it for the browser.
    case posted
    /// This device posted for this ceremony already, and the relay kept that
    /// one (409).
    case alreadyPosted
    /// The relay won't take a reply from this account for that one (403): not
    /// the account, and not a teammate it vouches for.
    case refused
    /// Nothing was posted: no account signed in here.
    case notSignedIn
    /// The relay couldn't be asked, or answered something else.
    case failed
}

extension CeremonyReplyPost {
    /// Whether the phone may show the comparison code (ov-770): only once the
    /// relay has answered 200, so the honest reply is stored before anyone
    /// can read a code to grind a forgery against. Every other outcome shows
    /// the failure, never the code.
    public var showsCode: Bool { self == .posted }
}

extension DirectoryClient {
    /// Post a reply the core sealed to a browser's key, for that browser to
    /// take (`POST /v1/ceremony/reply`).
    ///
    /// The body is the three fields the relay takes and nothing else:
    /// `ceremony` and `account` from the offer this device scanned, and
    /// `sealed` exactly as `farcooler_client_ceremony_seal` returned it. It
    /// goes to this account's own relay, through the session, like every
    /// directory call. Nothing about it is logged: `sealed` is the fleet's
    /// topology, encrypted, and the ceremony id is a correlator.
    public func postCeremonyReply(ceremony: String, account: String, sealed: String) async -> CeremonyReplyPost {
        switch await post("/v1/ceremony/reply", ["ceremony": ceremony, "account": account, "sealed": sealed]) {
        case .failure:
            return .failed
        case .success(let reply):
            switch reply.status {
            case 200: return .posted
            case 409: return .alreadyPosted
            case 403: return .refused
            default: return .failed
            }
        }
    }
}
