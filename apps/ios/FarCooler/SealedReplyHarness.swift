#if DEBUG
import FarCoolerClient
import SwiftUI

/// The phone answering a browser's offer (ov-770), for the UI suite
/// (`-sealed-reply`).
///
/// The store and the core are the app's own: it reads a browser's offer,
/// grants Cosmo, confirms, seals and posts. What's canned is the Face ID tap,
/// the directory certification, the runner's answer to `client.enroll`, and
/// the relay, which records each post. The harness then opens what was posted
/// with the browser's key, through the core, and shows what came out:
///
///   -sealed-reply              the offer names a reply key
///   -sealed-reply-none         it names none: the QR-only ceremony
///   -sealed-reply-malformed    it names one that isn't a key
struct SealedReplyHarness: View {
    static var isRequested: Bool { CommandLine.arguments.contains("-sealed-reply") }

    /// The relay's fixture key pair (`services/relay/test/fixtures/
    /// ceremony-reply.json`): a throwaway, labeled as one there, that nothing
    /// else uses. The secret is the browser's half, which only this test holds.
    static let replySecret = "WJUKRfiOBgbYeCZumaFOinssQJqCkAFD5Y1HKRKAAFw"
    static let replyKey = "YwbAvkATmjREjY8yCsUr87XR8t4kv8l2bK516AhoEWQ"

    static let browserKey =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFJhEUXBcOjzsos5aFyw6hfDrERWHei9akIudYJGG2gR browser"
    static let hostKey = "SHA256:iDqoxaySm9gzxtvLrNXXpM5PimPLeBknaaNj0Rg7vz4"
    static let cosmo = Runner(label: "Cosmo", address: "cosmo.local", user: "e-liang", fingerprint: hostKey)

    @StateObject private var store = CeremonyStore(
        account: "user_owner", accountEmail: "owner@example.com", deviceName: "Owner’s iPhone")
    @State private var posts: [(ceremony: String, account: String, sealed: String)] = []
    @State private var opened = ""
    @State private var openedCode = ""
    @State private var finished = false

    var body: some View {
        NavigationStack {
            List {
                Text(phase).accessibilityIdentifier("sealed-phase")
                if finished {
                    Text("\(posts.count)").accessibilityIdentifier("sealed-posts")
                    Text(opened).accessibilityIdentifier("sealed-opened")
                    Text(openedCode).accessibilityIdentifier("sealed-opened-code")
                    Text(store.sealedReply.code ?? "none").accessibilityIdentifier("sealed-phone-code")
                    Text(store.code.isEmpty ? "no reply" : "reply on screen").accessibilityIdentifier("sealed-qr")
                }
                SealedReplySection(reply: store.sealedReply, deviceName: "Chrome on Mac")
            }
            .navigationTitle("Add a Device")
            .navigationBarTitleDisplayMode(.inline)
        }
        .task { await run() }
    }

    private var phase: String {
        switch store.phase {
        case .showingManifest: return "showing reply"
        case .refused(let refusal): return "refused \(refusal)"
        default: return "\(store.phase)"
        }
    }

    /// The browser's offer, with the reply key this launch asks for written in.
    private static func offer() -> String? {
        guard
            case .payload(let data) = CeremonyCore.offer(
                name: "Chrome on Mac", account: "user_owner", keyA: browserKey, nodeKey: nil),
            var object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
        else { return nil }
        let arguments = CommandLine.arguments
        if arguments.contains("-sealed-reply-malformed") {
            object["reply_key"] = "not-a-key"
        } else if !arguments.contains("-sealed-reply-none") {
            object["reply_key"] = replyKey
        }
        guard let encoded = try? JSONSerialization.data(withJSONObject: object) else { return nil }
        return String(decoding: encoded, as: UTF8.self)
    }

    private func run() async {
        guard let offer = Self.offer() else { return }
        store.confirmTap = { _ in .confirmed }
        store.certify = { _, _ in }
        store.enroller = { _, _, _, _, runners in
            Dictionary(uniqueKeysWithValues: runners.map { ($0.id, Connection.Enrollment.written(tunnel: "")) })
        }
        store.sealedReply.post = { ceremony, account, sealed in
            posts.append((ceremony, account, sealed))
            return .posted
        }
        store.read(offer, runners: [Self.cosmo], grantingFrom: Self.cosmo)
        if case .confirming = store.phase { await store.confirm() }
        await store.sealedReply.sending?.value
        if let posted = posts.first {
            opened = open(posted, expecting: offer)
        } else {
            opened = "nothing posted"
        }
        finished = true
    }

    /// What the browser would do with the post: take it with its key, for the
    /// offer it showed, and compute the code over what came out.
    private func open(_ posted: (ceremony: String, account: String, sealed: String), expecting offer: String) -> String {
        guard let shown = try? JSONDecoder().decode(CeremonyOffer.self, from: Data(offer.utf8)),
            posted.ceremony == shown.ceremony, posted.account == shown.account
        else { return "posted for another offer" }
        switch CeremonyCore.takeSealed(posted.sealed, secret: Self.replySecret, expecting: offer) {
        case .refused(let refusal):
            return "refused \(refusal)"
        case .payload(let data):
            let manifest = String(decoding: data, as: UTF8.self)
            openedCode = CeremonyCore.text(of: manifest, farcooler_client_ceremony_code) ?? "none"
            return manifest.contains(Self.hostKey) && !posted.sealed.contains(Self.hostKey)
                ? "opened with Cosmo’s host key" : "opened without it"
        }
    }
}
#endif
