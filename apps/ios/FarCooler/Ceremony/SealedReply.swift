import FarCoolerClient
import Foundation
import SwiftUI

// The reply, sealed for a browser that can't scan it (ov-770).
//
// A browser has no camera, so when the offer this phone scanned names a
// `reply_key`, the reply it built is also sealed to that key, posted to the
// relay this phone is signed in to, and a six-character code goes up beside
// the QR code for the person to match against the browser's.
//
// THE SEAL AND THE CODE ARE RUST'S (`crates/ceremony/src/seal.rs`), through
// `farcooler_client_ceremony_seal`. Nothing here encrypts, hashes or compares:
// this file hands over the two strings the ceremony already holds, posts what
// comes back, and draws the code. An offer without a reply key never reaches
// the core here, so that ceremony is exactly the QR-only one it was.
//
// Nothing here is logged. `sealed` is the fleet's topology, encrypted, and the
// reply it came from holds every runner's address and host key.

extension CeremonyCore {
    /// Seal a reply to the reply key its offer named. `offer` is what the scan
    /// returned and `reply` what `reply(offer:runners:)` built for it;
    /// `heldFor` is this device's time since the scan, judged in Rust, so an
    /// offer read past the window is `stale` and nothing is sealed.
    ///
    /// Answers `{"sealed", "code"}`.
    static func seal(offer: Data, reply: Data, heldFor: TimeInterval) -> Answer {
        guard let offerText = String(data: offer, encoding: .utf8),
            let replyText = String(data: reply, encoding: .utf8)
        else { return .refused(.unknown) }
        return offerText.withCString { offer in
            replyText.withCString { reply in
                answer { farcooler_client_ceremony_seal(offer, reply, milliseconds(heldFor), $0, $1) }
            }
        }
    }

    /// Take a sealed reply with the reply key's secret half: the new device's
    /// side, which a phone never is today. The UI suite opens what the phone
    /// posted with it (`SealedReplyHarness`).
    static func takeSealed(_ sealed: String, secret: String, expecting: String) -> Answer {
        sealed.withCString { sealed in
            secret.withCString { secret in
                expecting.withCString { expecting in
                    answer { farcooler_client_ceremony_take_sealed(sealed, secret, expecting, false, 0, $0, $1) }
                }
            }
        }
    }
}

/// The sealed half of a reply this phone built: sealing it, posting it once,
/// and the code to show (ov-770). Owned by ``CeremonyStore``, which hands it
/// each reply it shows.
@MainActor
final class SealedReply: ObservableObject {
    enum State: Equatable {
        /// The offer asked for no sealed reply: the QR code is the whole reply.
        case none
        case sending
        /// The relay holds it for the browser.
        case sent
        /// It couldn't be sealed or posted. The QR code is still on screen.
        case notSent
    }

    @Published private(set) var state: State = .none
    /// The six characters both screens show, as Rust computed them over the
    /// reply that was sealed. Nil unless a reply was sealed.
    @Published private(set) var code: String?
    /// Each runner the reply grants and the host key it pins, for the person
    /// who wants the whole fingerprint rather than the code.
    @Published private(set) var hostKeys: [(label: String, hostKey: String)] = []

    /// One POST to the relay this app is signed in to, through its session.
    /// A harness answers it.
    var post: (_ ceremony: String, _ account: String, _ sealed: String) async -> CeremonyReplyPost = {
        let account = Account.shared
        guard account.isSignedIn else { return .notSignedIn }
        return await account.directoryClient.postCeremonyReply(ceremony: $0, account: $1, sealed: $2)
    }

    /// The post started by the last reply, for a test to wait on.
    private(set) var sending: Task<Void, Never>?

    /// Seal and post `reply`, when `offer` asked for that.
    ///
    /// `offer` is the scan's answer, checked by the core a moment ago; its
    /// `reply_key` is read here only to decide whether to ask the core at all.
    /// Every refusal posts nothing: the core is the only judge of the key, the
    /// freshness window and whether the reply answers this offer.
    func send(offer: Data, reply: Data, heldFor: TimeInterval) {
        reset()
        guard let asked = try? JSONDecoder().decode(CeremonyOffer.self, from: offer),
            asked.reply_key != nil
        else { return }
        guard case .payload(let data) = CeremonyCore.seal(offer: offer, reply: reply, heldFor: heldFor),
            let sealed = try? JSONDecoder().decode(Sealed.self, from: data)
        else {
            state = .notSent
            return
        }
        code = sealed.code
        hostKeys = ((try? JSONDecoder().decode(CeremonyManifest.self, from: reply))?.runners ?? [])
            .map { (label: $0.label, hostKey: $0.host_key) }
        state = .sending
        let post = self.post
        sending = Task {
            let outcome = await post(asked.ceremony, asked.account, sealed.sealed)
            state = outcome == .posted ? .sent : .notSent
        }
    }

    func reset() {
        sending?.cancel()
        sending = nil
        state = .none
        code = nil
        hostKeys = []
    }

    private struct Sealed: Decodable {
        let sealed: String
        let code: String
    }
}

/// The code, under the reply's QR code, while a sealed reply is out.
///
/// Nothing at all for a reply that wasn't sealed, so the QR-only screen is
/// the one it always was.
struct SealedReplySection: View {
    @ObservedObject var reply: SealedReply
    let deviceName: String

    var body: some View {
        if reply.state != .none {
            VStack(spacing: 8) {
                if let code = reply.code, reply.state != .notSent {
                    Text(
                        reply.state == .sent
                            ? "Sent to \(deviceName). Check that it shows this code:"
                            : "Sending to \(deviceName)…"
                    )
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .accessibilityIdentifier("sealed-reply-status")
                    Text(Self.spaced(code))
                        .font(.system(.title, design: .monospaced).weight(.semibold))
                        .textSelection(.enabled)
                        .accessibilityLabel(code)
                        .accessibilityIdentifier("sealed-reply-code")
                    if !reply.hostKeys.isEmpty {
                        DisclosureGroup("Host Keys") {
                            ForEach(Array(reply.hostKeys.enumerated()), id: \.offset) { _, row in
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(row.label).font(.footnote)
                                    Text(row.hostKey.isEmpty ? "Not pinned" : row.hostKey)
                                        .font(.system(.caption, design: .monospaced))
                                        .foregroundStyle(.secondary)
                                        .textSelection(.enabled)
                                }
                                .frame(maxWidth: .infinity, alignment: .leading)
                            }
                        }
                        .font(.footnote)
                    }
                } else {
                    Text("Couldn’t send this to \(deviceName). If it has a camera, scan the code above with it.")
                        .font(.footnote)
                        .foregroundStyle(.orange)
                        .multilineTextAlignment(.center)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityIdentifier("sealed-reply-status")
                }
            }
        }
    }

    /// "ABC 234": two halves, which is how a person reads six characters.
    static func spaced(_ code: String) -> String {
        guard code.count == 6 else { return code }
        return "\(code.prefix(3)) \(code.suffix(3))"
    }
}
