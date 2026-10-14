import AgentKit
import AppKit
import Foundation

/// The address kept beside a runner this Mac was granted through Tailcat, by
/// the local id its `ProxyCommand` names (ov-485).
///
/// Kept by this app, not in `runners.json`: nothing a `ProxyCommand` runs ever
/// reads it, because nothing dials it without a person choosing to. A protocol
/// so the route switch is tested without the app's defaults.
protocol RouteAlternatives: Sendable {
    func address(for id: String) -> (host: String, port: Int)?
    func remember(host: String, port: Int, for id: String)
    func forget(_ id: String)
}

struct DefaultsRouteAlternatives: RouteAlternatives {
    /// Channel-scoped like every other per-install fact.
    var key = "tunnelAlternatives.\(AppVersion.channel)"

    private var all: [String: [String: Any]] {
        UserDefaults.standard.dictionary(forKey: key) as? [String: [String: Any]] ?? [:]
    }

    func address(for id: String) -> (host: String, port: Int)? {
        guard let entry = all[id], let host = entry["host"] as? String, !host.isEmpty,
            let port = entry["port"] as? Int
        else { return nil }
        return (host, port)
    }

    func remember(host: String, port: Int, for id: String) {
        var all = all
        all[id] = ["host": host, "port": port]
        UserDefaults.standard.set(all, forKey: key)
    }

    func forget(_ id: String) {
        var all = all
        all[id] = nil
        UserDefaults.standard.set(all, forKey: key)
    }
}

/// Which route a runner granted through Tailcat takes, and moving it.
///
/// **The route is the `Host` entry in Far Cooler's block in `~/.ssh/config`**,
/// because that is what every `ssh` on this Mac reads: Far Cooler's own CLI,
/// Zed, git. On the tunnel the entry has a `ProxyCommand`; on its address it
/// has `HostName` and `Port` in that line's place. Everything else stays,
/// including the host key pin (`HostKeyAlias farcooler-<id>` and
/// `KnownHostsCommand`), so the address is held to the key the granting device
/// pinned exactly as the tunnel is (R-48). One route at a time, by
/// construction: an entry has one or the other.
struct RouteSwitch {
    var alternatives: RouteAlternatives = DefaultsRouteAlternatives()
    var readEntries: @Sendable () -> [SshConfig.Entry] = { SshConfig.entries() }
    var writeConfig: @Sendable ([String], URL) throws -> Void = {
        try SshConfig.write($0, identity: $1)
    }

    /// The route `target` takes, or nil when it isn't a Tailcat runner with an
    /// address kept beside it — then there is nothing to choose between.
    func route(of target: String) -> TunnelRoute.Current? {
        guard let entry = entry(for: target), let address = alternatives.address(for: entry.key)
        else { return nil }
        return entry.onItsAddress
            ? .direct(address: TunnelRoute.address(host: address.host, port: address.port)) : .tunnel
    }

    /// The address a person would be offered for `target`, as they read it.
    func alternative(of target: String) -> String? {
        guard let entry = entry(for: target), let address = alternatives.address(for: entry.key)
        else { return nil }
        return TunnelRoute.address(host: address.host, port: address.port)
    }

    /// Move `target` to its address. True when the entry was rewritten.
    @discardableResult
    func useAddress(for target: String) -> Bool {
        guard let entry = entry(for: target), !entry.onItsAddress,
            let address = alternatives.address(for: entry.key)
        else { return false }
        return rewrite(entry, to: Self.onItsAddress(entry.lines, host: address.host, port: address.port))
    }

    /// Move `target` back to Tailcat. True when the entry was rewritten.
    @discardableResult
    func useTunnel(for target: String) -> Bool {
        guard let entry = entry(for: target), entry.onItsAddress,
            let lines = Self.onTheTunnel(entry.lines, id: entry.key)
        else { return false }
        return rewrite(entry, to: lines)
    }

    private func entry(for target: String) -> SshConfig.Entry? {
        readEntries().first { $0.alias == target && $0.tunneled }
    }

    private func rewrite(_ entry: SshConfig.Entry, to lines: [String]) -> Bool {
        let entries = readEntries().map { have -> SshConfig.Entry in
            guard have.key == entry.key else { return have }
            var moved = have
            moved.lines = lines
            moved.onItsAddress = !entry.onItsAddress
            return moved
        }
        do {
            try writeConfig(SshConfig.render(entries), ShellKey.directory)
            return true
        } catch {
            return false
        }
    }

    /// The tunnel entry's lines with its `ProxyCommand` swapped for the address.
    static func onItsAddress(_ lines: [String], host: String, port: Int) -> [String] {
        lines.flatMap { line -> [String] in
            keyword(of: line) == "proxycommand" ? ["  HostName \(host)", "  Port \(port)"] : [line]
        }
    }

    /// The address entry's lines with the `ProxyCommand` back in its place.
    ///
    /// The program is read back off the entry's own `KnownHostsCommand`, which
    /// names the same CLI the `ProxyCommand` did and stayed while the runner
    /// was on its address. Nil when that line isn't there to read: then this
    /// entry wasn't written by the switch above, and nothing is guessed.
    static func onTheTunnel(_ lines: [String], id: String) -> [String]? {
        // Everything between the keyword and ` runner known-host`, as written:
        // the program may be one quoted word with spaces in it.
        guard
            let known = lines.first(where: { keyword(of: $0) == "knownhostscommand" }),
            let start = known.range(of: "KnownHostsCommand "),
            let end = known.range(of: " runner known-host ", range: start.upperBound..<known.endIndex)
        else { return nil }
        let program = String(known[start.upperBound..<end.lowerBound])
        guard lines.contains(where: { keyword(of: $0) == "hostname" }) else { return nil }
        return lines.compactMap { line -> String? in
            switch keyword(of: line) {
            case "hostname": return "  ProxyCommand \(program) runner pipe \(id)"
            case "port": return nil
            default: return line
            }
        }
    }

    private static func keyword(of line: String) -> String {
        line.split(separator: " ", omittingEmptySubsequences: true).first.map { $0.lowercased() } ?? ""
    }
}

/// "Connecting through Tailcat failed. Connect directly to <address> instead?"
/// on the Mac: asked once per failure, when a runner granted through Tailcat
/// stops answering and has an address beside it (ov-485).
///
/// Never answered on the person's behalf. Choosing it moves that runner to its
/// address until somebody moves it back in Settings > Runners; "Not Now" leaves
/// it on Tailcat, retrying as every runner does, and it is asked again only
/// after the runner has answered once.
@MainActor
final class RoutePrompt {
    static let shared = RoutePrompt()

    var routes = RouteSwitch()
    /// Shows the question and answers true for Connect Directly.
    var ask: @MainActor (DirectRouteOffer, @escaping @MainActor (Bool) -> Void) -> Void = RoutePrompt.alert
    /// Dials the runner again, on its new route.
    var reconnect: @MainActor (String) -> Void = { FleetStore.shared.reconnect($0) }

    /// Runners asked about since they last answered.
    private(set) var asked: Set<String> = []

    /// A runner's state changed. Asks when it is the failure the question is
    /// for; forgets having asked once it connects.
    func heard(target: String, state: HostState) {
        guard !target.isEmpty else { return }
        switch state {
        case .connected:
            asked.remove(target)
        case .unreachable(let reason):
            guard !asked.contains(target),
                let offer = TunnelRoute.offer(
                    onTheTunnel: routes.route(of: target) == .tunnel,
                    failedBeforeTheRunner: Self.failedBeforeTheRunner(reason),
                    alternative: routes.alternative(of: target))
            else { return }
            asked.insert(target)
            ask(offer) { [weak self] yes in
                guard yes, let self, self.routes.useAddress(for: target) else { return }
                self.reconnect(target)
            }
        case .connecting, .reconnecting, .notInstalled:
            return
        }
    }

    /// Whether the runner's sshd never answered. A refused key, a host key
    /// that doesn't match the pin, or a build too old to talk would say the
    /// same thing at the address, so none of them asks.
    nonisolated static func failedBeforeTheRunner(_ reason: String) -> Bool {
        let answered = [
            "Permission denied", "Host key verification failed", "update the older side",
        ]
        return !answered.contains { reason.localizedCaseInsensitiveContains($0) }
    }

    private static func alert(_ offer: DirectRouteOffer, _ answer: @escaping @MainActor (Bool) -> Void) {
        let alert = NSAlert()
        alert.messageText = offer.title
        alert.informativeText = offer.question
        alert.addButton(withTitle: DirectRouteOffer.connectDirectly)
        alert.addButton(withTitle: DirectRouteOffer.notNow)
        let done: (NSApplication.ModalResponse) -> Void = { answer($0 == .alertFirstButtonReturn) }
        if let window = NSApp.keyWindow ?? NSApp.mainWindow {
            alert.beginSheetModal(for: window, completionHandler: done)
        } else {
            done(alert.runModal())
        }
    }
}
