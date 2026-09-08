import FarCoolerClient
import SwiftUI
import UIKit

#if DEBUG

/// The tunnel ceremony, driven on a real device without a camera.
///
/// **This is a test harness and nothing in the shipped app reaches it.** It
/// exists because the QR ceremony has exactly one step that cannot be
/// automated: a phone reads the reply with its camera, off another screen, and
/// no CI, no `xcodebuild` and no `devicectl` can hold a phone up to a monitor.
/// Everything either side of that step is ordinary code, and every line of it
/// below is the app's own — ``NodeIdentity``, ``CeremonyStore``,
/// ``CeremonyRunner/asRunner``, ``RunnerStore`` and ``ClientCore`` — called in
/// the order ``JoinView`` calls them. What the harness replaces is the camera:
/// the reply arrives as a launch argument instead of through a lens.
///
/// So a green run here says the ceremony's LOGIC works on this hardware. It
/// deliberately does not say the scanner does, and a report drawn from it must
/// say which of the two it is talking about.
///
/// Every line it prints begins `E2E ` and goes to stdout, which
/// `xcrun devicectl device process launch --console` streams back to the Mac
/// that started it. The private half of the node key is never printed — only
/// its length — because the whole point of the design under test is that it
/// stays in the Keychain.
///
///     xcrun devicectl device process launch --console \
///       --device <udid> com.farcooler.ios.local \
///       -- -tunnel-e2e -e2e-reply '<the reply>' -e2e-dial
struct TunnelE2EHarness: View {
    static var isRequested: Bool { CommandLine.arguments.contains("-tunnel-e2e") }

    /// The value after a flag, or nil. `CommandLine.arguments` is what the app
    /// sees, so this reads exactly what was passed after `--`.
    private static func value(after flag: String) -> String? {
        guard let at = CommandLine.arguments.firstIndex(of: flag),
            CommandLine.arguments.index(after: at) < CommandLine.arguments.endIndex
        else { return nil }
        return CommandLine.arguments[CommandLine.arguments.index(after: at)]
    }

    @State private var lines: [String] = []

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 6) {
                ForEach(Array(lines.enumerated()), id: \.offset) { _, line in
                    Text(line).font(.system(size: 11, design: .monospaced))
                        .textSelection(.enabled)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding()
        }
        .task { await run() }
    }

    private func say(_ text: String) {
        let line = "E2E \(text)"
        print(line)
        // stdout is line-buffered onto a pipe rather than a terminal, so a run
        // that ends in a `dial` that takes thirty seconds would otherwise
        // deliver every earlier line only at the end.
        fflush(stdout)
        NSLog("%@", line)
        lines.append(line)
    }

    private func run() async {
        say("begin device=\(UIDevice.current.systemName) \(UIDevice.current.systemVersion)")

        // Put back the app's remembered runner. `RunnerStore.add` moves the
        // selection, and an earlier run of this harness moved it to a runner
        // that no longer exists — this is how that is undone from a script.
        if let last = Self.value(after: "-e2e-restore-last") {
            UserDefaults.standard.set(last, forKey: "hosts.last")
            say("hosts.last set to \(last)")
        }

        // 1. The mint. Twice, because "one node key per device, not per runner"
        // is a claim about the SECOND call.
        let pair = NodeIdentity.mintIfNeeded()
        say(
            "mint public=\(pair?.publicKey ?? "<none>") "
                + "public_len=\(pair?.publicKey.count ?? 0) "
                + "private_len=\(pair?.privateKey.count ?? 0)")
        let again = NodeIdentity.mintIfNeeded()
        say(
            "mint-again public=\(again?.publicKey ?? "<none>") "
                + "same=\(again?.publicKey == pair?.publicKey)")
        say("keychain-write-status=\(UserDefaults.standard.object(forKey: "nodeKeychainWriteStatus") ?? "none")")
        // The same answer Settings now shows, in the machine form, so a run of
        // this harness and a screenshot from a TestFlight build can be compared
        // without either being translated by hand. It carries the stable word
        // and never key material — `.held` has no payload at all.
        say("mint-status=\(NodeIdentity.status())")

        // 2. The offer, built by the app's own ceremony store.
        //
        // A signed-out phone has no account id, and the ceremony's account
        // check compares the offer's with the reply's — both of which come from
        // this run — so a stand-in is enough for the transport and says so.
        let account = Account.shared.userId.isEmpty ? "e2e-no-account" : Account.shared.userId
        say("account \(account.isEmpty ? "<empty>" : account) signed_in=\(Account.shared.isSignedIn)")
        let store = CeremonyStore(
            account: account, accountEmail: Account.shared.email,
            deviceName: UIDeviceName())
        store.showOffer(publicKey: Identity.publicKey)
        say("offer-phase \(String(describing: store.phase))")
        say("offer \(store.code)")
        say("ssh-public-key \(Identity.publicKey ?? "<none>")")

        // A second offer, to show the same node key is offered again — the
        // "once per device, not once per runner" claim, at the layer a second
        // enrolment would actually go through.
        let second = CeremonyStore(
            account: account, accountEmail: Account.shared.email,
            deviceName: UIDeviceName())
        second.showOffer(publicKey: Identity.publicKey)
        say("offer-2 \(second.code)")

        // The reply. Two ways in, and they answer different questions.
        //
        // `-e2e-reply` is a reply built elsewhere — the real second leg, from a
        // granting device — but it cannot survive a relaunch: `takeReply`
        // checks the reply against the offer THIS process is showing, and a
        // new process shows a new ceremony id. So it only works when the
        // granting side can answer within one launch.
        //
        // `-e2e-token` is the way that works from a script: the harness builds
        // the reply itself, through the same `farcooler_client_ceremony_reply`
        // a granting device calls, and hands it to `takeReply`. What that
        // cannot show is a reply crossing between two devices; what it does
        // show is `manifest` and `accept_manifest` agreeing, and everything
        // downstream of them.
        var reply: String? = Self.value(after: "-e2e-reply")
        // `-e2e-direct <host>:<port>` grants a DIRECT runner instead, which is
        // the check that the common path has not moved.
        let reach: Reach? = {
            if let token = Self.value(after: "-e2e-token") { return .tailcat(token: token) }
            guard let address = Self.value(after: "-e2e-direct"),
                let colon = address.lastIndex(of: ":")
            else { return nil }
            return .direct(
                host: String(address[address.startIndex..<colon]),
                port: Int(address[address.index(after: colon)...]) ?? 22)
        }()
        if reply == nil, let reach {
            let granted = CeremonyRunner(
                id: "11111111-1111-4111-8111-111111111111",
                label: Self.value(after: "-e2e-label") ?? "E2E tunneled",
                alias: "e2e",
                user: Self.value(after: "-e2e-user") ?? "runner",
                host_key: Self.value(after: "-e2e-hostkey") ?? "",
                reach: reach,
                pending: false)
            switch CeremonyCore.reply(offer: Data(store.code.utf8), runners: [granted]) {
            case .refused(let refusal):
                say("reply-build-refused \(String(describing: refusal))")
            case .payload(let data):
                reply = String(decoding: data, as: UTF8.self)
                say("reply-built \(reply ?? "")")
            }
        }
        guard let reply else {
            say("done no-reply-given")
            return
        }

        // 3. The reply, as the camera would have delivered it.
        store.takeReply(reply)
        say("reply-phase \(String(describing: store.phase))")
        let granted = store.granted
        say("granted count=\(granted.count)")
        for entry in granted {
            say(
                "granted-runner label=\(entry.label) user=\(entry.user) "
                    + "pending=\(entry.pending) reach=\(String(describing: entry.reach))")
        }

        // 4. Adoption, exactly as `JoinView.adopt()` does it — and then put the
        // stored list back the way it was found. This is a test harness running
        // inside somebody's real app; it must not leave a runner behind.
        let saved = UserDefaults.standard.data(forKey: "hosts")
        let savedLast = UserDefaults.standard.string(forKey: "hosts.last")
        defer {
            if let saved {
                UserDefaults.standard.set(saved, forKey: "hosts")
            } else {
                UserDefaults.standard.removeObject(forKey: "hosts")
            }
            if let savedLast {
                UserDefaults.standard.set(savedLast, forKey: "hosts.last")
            } else {
                UserDefaults.standard.removeObject(forKey: "hosts.last")
            }
            say("restored hosts.last=\(savedLast ?? "<none>")")
        }
        let runners = RunnerStore()
        var adopted: [Runner] = []
        for entry in granted where !entry.pending {
            let arriving = entry.asRunner
            runners.add(arriving)
            adopted.append(arriving)
        }
        // Read back through a FRESH store, so what is reported is what survived
        // encoding and decoding rather than what was appended in memory.
        let reloaded = RunnerStore()
        for host in reloaded.hosts {
            say("stored label=\(host.label) reach=\(String(describing: host.reach))")
        }

        guard CommandLine.arguments.contains("-e2e-dial") else {
            say("done no-dial-asked")
            return
        }

        // 5. The dial, through the tunnel, with the app's own config builder.
        // A `.tailcat` reach carries a token and this device's node key and no
        // host at all, so there is no address for this to fall back to.
        guard let target = adopted.first else {
            say("done no-runner-to-dial")
            return
        }
        guard let sshKey = Identity.privateKey() else {
            say("dial-refused no-ssh-identity")
            return
        }
        let nodeKey = NodeIdentity.storedPrivateKey
        say("dial reach=\(String(describing: target.reach)) node_key_present=\(nodeKey != nil)")
        let started = Date()
        do {
            let core = ClientCore()
            let data = try await core.connect(
                config: target.config(
                    privateKey: sshKey, nodeKey: nodeKey, derpMap: Account.shared.derpMap))
            let ms = Int(Date().timeIntervalSince(started) * 1000)
            say("connected ms=\(ms) bytes=\(data.count)")
            say("connected-head \(String(decoding: data.prefix(400), as: UTF8.self))")
        } catch {
            let ms = Int(Date().timeIntervalSince(started) * 1000)
            say("dial-failed ms=\(ms) error=\(error)")
        }
        say("done")
    }
}

#endif
