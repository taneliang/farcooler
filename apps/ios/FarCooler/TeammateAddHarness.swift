#if DEBUG
import SwiftUI
import CryptoKit

/// The owner's phone adding a teammate's device (ov-854), for the UI suite
/// (`-teammate-add`).
///
/// The screen is the app's own `AddDeviceView` over its own `CeremonyStore`,
/// and so is everything the store decides: it reads a teammate's code built by
/// the core, asks the directory's answers, has the core vouch for the code,
/// and builds the reply. What's canned is the camera (the code is handed to the
/// store), the team directory, the Face ID tap, which runners this phone
/// administers, and the runner's answer to `teammate.enroll`. The enroller
/// records what it was asked, so a run leaves no key behind.
///
///   -teammate-add            Bob, of Platform, on a phone that administers
///                            build-box and only controls gpu-workstation
///   -teammate-add-long       a long email and long runner and device names
///   -teammate-add-stranger   the directory doesn't list Bob
///   -teammate-add-offline    the directory can't be asked
///   -teammate-add-refused    build-box's Far Cooler is too old to bind
///   -teammate-add-admin      the phone is an Admin on build-box, not the Owner
///   -teammate-add-existing   build-box already has Bob under this account
///   -teammate-add-browser    Bob's code is from Chrome on Mac, whose offer
///                            names a reply key (ov-778): the relay is canned
///                            too, and records each sealed post
///
/// The phone is the Owner on build-box and only a Member on gpu-workstation,
/// where adding people is refused up front.
struct TeammateAddHarness: View {
    static var isRequested: Bool { CommandLine.arguments.contains("-teammate-add") }
    private static func has(_ flag: String) -> Bool { CommandLine.arguments.contains(flag) }

    static let key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIB1iLbeqDzK4CDeUC3t+ffVPDI9Gk+sBwIZqJZW1NfS5 bob"

    private static var email: String {
        has("-teammate-add-long") ? "bartholomew.montgomery-fitzgerald@platform-infrastructure.example.com" : "bob.smith@example.com"
    }

    static let buildBox = Runner(
        label: "build-box", reach: .direct(host: "build-box.example.com", port: 22), user: "alice",
        fingerprint: "SHA256:iDqoxaySm9gzxtvLrNXXpM5PimPLeBknaaNj0Rg7vz4")
    static let gpu = Runner(
        label: "gpu-workstation", reach: .direct(host: "gpu.example.com", port: 22), user: "alice",
        fingerprint: "SHA256:iDqoxaySm9gzxtvLrNXXpM5PimPLeBknaaNj0Rg7vz4")
    static let platform = Runner(
        label: "Platform Builds, EU West (Shared)", reach: .direct(host: "eu-west.platform.example.com", port: 22),
        user: "alice", fingerprint: "SHA256:iDqoxaySm9gzxtvLrNXXpM5PimPLeBknaaNj0Rg7vz4")

    @StateObject private var store = CeremonyStore(
        account: "user_alice", accountEmail: "alice@example.com", deviceName: "Alice’s iPhone")
    @StateObject private var runners = RunnerStore()
    /// What the enroller was asked, in one line, for the suite to read.
    @State private var asked = ""
    /// The sealed replies posted, for the suite to read.
    @State private var sealedPosts: [(ceremony: String, account: String, sealed: String)] = []
    @State private var openedCode = ""
    @State private var postResult = "none"

    var body: some View {
        NavigationStack {
            AddDeviceView(runners: runners, store: store)
        }
        .overlay(alignment: .bottom) {
            Color.clear
                // style-exempt: an invisible test probe, not a rule
                .frame(width: 1, height: 1)
                .accessibilityElement()
                .accessibilityIdentifier("teammate-asked")
                .accessibilityValue(asked)
        }
        .overlay(alignment: .top) {
            Color.clear
                // style-exempt: an invisible test probe, not a rule
                .frame(width: 1, height: 1)
                .accessibilityElement()
                .accessibilityIdentifier("teammate-sealed")
                .accessibilityValue(
                    postResult) // platform copy: a DEBUG test harness readout, never shipped
        }
        .overlay(alignment: .leading) {
            // style-exempt: an invisible accessibility test probe
            Color.clear.frame(width: 1, height: 1).accessibilityElement()
                .accessibilityIdentifier("teammate-opened-code").accessibilityValue(openedCode)
        }
        .task { await prepare() }
    }

    private func prepare() async {
        let offerer = Self.has("-teammate-add-stranger") ? "user_carol" : "user_bob"
        let device =
            Self.has("-teammate-add-long")
            ? "Bartholomew’s Work iPhone (the one with the cracked screen)" // platform copy: DEBUG harness device name fixture
            : Self.has("-teammate-add-browser") ? "Chrome on Mac" : "Bob’s iPhone" // platform copy: a DEBUG test harness fixture, never shipped
        guard
            case .payload(let made) = CeremonyCore.offer(name: device, account: offerer, keyA: Self.key, nodeKey: nil),
            let offer = Self.has("-teammate-add-browser") ? Self.withReplyKey(made) : made
        else { return }
        if Self.has("-teammate-add-authority") {
            store.sealedReply.signingKey = { Self.phonePrivateKey }
            store.sealedReply.runnerAuthority = { _, offer in Self.attest(offer) }
        }
        store.sealedReply.post = { ceremony, account, sealed in
            sealedPosts.append((ceremony, account, sealed))
            let expected = String(decoding: offer, as: UTF8.self)
            let scanned = try? JSONDecoder().decode(CeremonyOffer.self, from: offer)
            if ceremony != scanned?.ceremony || account != scanned?.account {
                postResult = "posted for another offer" // platform copy: DEBUG harness test readout
            } else if case .payload(let data) = CeremonyCore.takeSealed(sealed, secret: SealedReplyHarness.replySecret, expecting: expected),
                let result = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                let manifest = result["manifest"] as? [String: Any],
                let runners = manifest["runners"] as? [[String: Any]],
                runners.count == 1, runners[0]["label"] as? String == "build-box",
                runners[0]["pending"] as? Bool == false,
                let grant = manifest["granted_by"] as? String, let code = result["code"] as? String {
                openedCode = code
                postResult = "\(sealedPosts.count) post for scanned ceremony and \(account); opened build-box for \(grant)" // platform copy: DEBUG harness test readout
            } else {
                postResult = "posted body did not open to the grant" // platform copy: DEBUG harness test readout
            }
            return .posted
        }
        store.teamDirectory = { Self.has("-teammate-add-offline") ? nil : Self.directory }
        store.confirmTap = { _ in .confirmed }
        store.teammateFacts = { runner, _ in
            if runner.id == Self.gpu.id { return TeammateRunnerFacts(caller: .member) }
            let bob = ExistingPerson(handle: "bob", name: "Bob", access: .member)
            return TeammateRunnerFacts(
                caller: Self.has("-teammate-add-admin") ? .admin : .owner,
                candidates: Self.has("-teammate-add-existing") ? [bob] : [],
                matching: Self.has("-teammate-add-existing") ? "bob" : nil, taken: ["dan"])
        }
        store.teammateEnroller = { _, _, _, _, teammate, wanted, facts in
            let bound = wanted.first.map { TeammateAccessRules.bind(teammate, facts: facts[$0.id]) } ?? [:]
            asked =
                "access=\(bound["access"] ?? "") handle=\(bound["handle"] ?? "") name=\(bound["name"] ?? "") "
                + "account=\(teammate.who.account) runners=\(wanted.map(\.label).joined(separator: ","))"
            let outcome: TeammateAddOutcome = Self.has("-teammate-add-refused") ? .tooOld : .written(tunnel: "")
            return Dictionary(uniqueKeysWithValues: wanted.map { ($0.id, outcome) })
        }
        let runnersOnPhone = Self.has("-teammate-add-long") ? [Self.platform, Self.gpu] : [Self.buildBox, Self.gpu]
        store.read(String(decoding: offer, as: UTF8.self), runners: runnersOnPhone, grantingFrom: nil)
    }

    // Throwaway test keys. The runner signs the same offer-bound statement as
    // ceremony.authority; the core verifies this before sealing the reply.
    private static let phonePrivateKey = """
    -----BEGIN OPENSSH PRIVATE KEY-----
    b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
    QyNTUxOQAAACBnQF6TwOKpna/kBNXvfjHY2jJOD71Y7orB82MvwfIcggAAAJhxmOlEcZjp
    RAAAAAtzc2gtZWQyNTUxOQAAACBnQF6TwOKpna/kBNXvfjHY2jJOD71Y7orB82MvwfIcgg
    AAAEB5qaGeYkCb/0QgAnXa6y0/Gb6xpYPP9NZnuB40nFzRK2dAXpPA4qmdr+QE1e9+Mdja
    Mk4PvVjuisHzYy/B8hyCAAAAE2UtbGlhbmdAQ29zbW8ubG9jYWwBAg==
    -----END OPENSSH PRIVATE KEY-----
    """
    private static let runnerKey = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBhm/7FjkRZx+RN5ewWF/zRLqxuMW0DGJVDNEjbHs6RR"

    private static func sshString(_ data: Data) -> Data {
        var size = UInt32(data.count).bigEndian
        return withUnsafeBytes(of: &size) { Data($0) } + data
    }
    private static func sshString(_ text: String) -> Data { sshString(Data(text.utf8)) }

    private static func attest(_ offer: String) -> [String: Any]? {
        guard let data = offer.data(using: .utf8),
            let asked = try? JSONDecoder().decode(CeremonyOffer.self, from: data), let replyKey = asked.reply_key,
            let seed = Data(base64Encoded: "vAAhuCTIb49shswSyRIO5JmAiUxG0Ofdf5kBLIzKCUQ="),
            let key = try? Curve25519.Signing.PrivateKey(rawRepresentation: seed),
            let publicBlob = Data(base64Encoded: String(runnerKey.split(separator: " ")[1]))
        else { return nil }
        let statement: [Any] = [asked.ceremony, asked.account, asked.channel,
            "SHA256:iDqoxaySm9gzxtvLrNXXpM5PimPLeBknaaNj0Rg7vz4", replyKey,
            "SHA256:YRgnX6t+yUKEV+ZHGrBs2T8nVrPItzqXfgxAF/fPqWI"]
        guard let bytes = try? JSONSerialization.data(withJSONObject: statement, options: [.withoutEscapingSlashes]) else { return nil }
        let namespace = "farcooler-ceremony-authority-v1"
        let signed = Data("SSHSIG".utf8) + sshString(namespace) + sshString("") + sshString("sha512") + sshString(Data(SHA512.hash(data: bytes)))
        guard let signature = try? key.signature(for: signed) else { return nil }
        let blob = Data("SSHSIG".utf8) + Data([0, 0, 0, 1]) + sshString(publicBlob) + sshString(namespace)
            + sshString("") + sshString("sha512") + sshString(sshString("ssh-ed25519") + sshString(signature))
        let pem = "-----BEGIN SSH SIGNATURE-----\n" + blob.base64EncodedString(options: [.lineLength64Characters, .endLineWithLineFeed]) + "\n-----END SSH SIGNATURE-----\n"
        return ["runner_key": runnerKey, "signature": pem]
    }

    /// A browser's offer: the same code with a reply key written in, the relay
    /// fixture's throwaway (`SealedReplyHarness.replyKey`).
    private static func withReplyKey(_ offer: Data) -> Data? {
        guard var object = (try? JSONSerialization.jsonObject(with: offer)) as? [String: Any] else { return nil }
        object["reply_key"] = SealedReplyHarness.replyKey
        return try? JSONSerialization.data(withJSONObject: object)
    }

    /// The relay's answers: Alice is an admin of Platform, and Bob is in it
    /// unless this launch says he isn't.
    private static var directory: Data {
        let members: [[String: Any]] =
            [["user": "user_alice", "email": "alice@example.com", "role": "admin"]]
            + (has("-teammate-add-stranger") ? [] : [["user": "user_bob", "email": email, "role": "member"]])
        let object: [String: Any] = [
            "me": [
                "user": ["id": "user_alice", "email": "alice@example.com"],
                "teams": [["id": "t-platform", "name": "Platform", "personal": false, "role": "admin"]],
            ],
            "rosters": [["team": ["id": "t-platform", "name": "Platform", "personal": false], "members": members]],
        ]
        return (try? JSONSerialization.data(withJSONObject: object)) ?? Data()
    }
}
#endif
