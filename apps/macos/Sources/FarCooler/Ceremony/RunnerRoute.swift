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
    /// The runner's name as the granting device listed it (ov-485 N6).
    func label(for id: String) -> String?
    func remember(host: String, port: Int, label: String, for id: String)
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

    func label(for id: String) -> String? {
        (all[id]?["label"] as? String).flatMap { $0.isEmpty ? nil : $0 }
    }

    func remember(host: String, port: Int, label: String, for id: String) {
        var all = all
        all[id] = ["host": host, "port": port, "label": label]
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

    /// What a person calls `target`: its runner name, else the alias.
    func name(of target: String) -> String {
        entry(for: target).flatMap { alternatives.label(for: $0.key) } ?? target
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

/// Why a runner's connection failed, as far as the route question cares: the
/// tunnel never reached it, or anything else (ov-485 fix 1, M2).
///
/// Typed from one positive signal and nothing else: the line `farcooler runner
/// pipe` writes on stderr when its dial never reached the runner, which `ssh`
/// and the CLI pass through. A refused key, a host key that doesn't match the
/// pin, a build too old to talk, output nobody can read: none carries it, so
/// none asks. Wording that drifts can only make the app ask LESS, never more.
enum RouteFailure: Equatable {
    case tunnelNeverReachedTheRunner
    case other

    /// `crates/cli/src/runner_pipe.rs`'s `TUNNEL_UNREACHED`, the other copy.
    static let unreachedMarker = "farcooler-route: tunnel-unreached"

    init(_ reason: String) {
        let marked = reason.split(whereSeparator: \.isNewline).contains {
            $0.trimmingCharacters(in: .whitespaces) == Self.unreachedMarker
        }
        self = marked ? .tunnelNeverReachedTheRunner : .other
    }
}

/// One question, for every Tailcat runner that failed together.
struct RouteQuestion: Equatable {
    struct Runner: Equatable {
        var target: String
        var address: String
        /// The runner's name, which the question says (N6).
        var name: String
    }

    var runners: [Runner]

    var title: String { DirectRouteOffer(address: "").title }

    /// The owner's question, naming each runner when there are several, and
    /// what choosing it does: it sticks until Use Tailcat.
    var message: String {
        let question = DirectRouteOffer.question(for: runners.map { (name: $0.name, address: $0.address) }) ?? ""
        let afterwards = runners.count == 1 ? DirectRouteOffer.afterwards : DirectRouteOffer.afterwardsForSeveral
        return "\(question)\n\n\(afterwards)"
    }
}

/// "Connecting through Tailcat failed. Connect directly to <address> instead?"
/// on the Mac, when a runner granted through Tailcat stops answering because
/// the tunnel never reached it, and it has an address beside it (ov-485).
///
/// - **Never answered for the person.** Not Now is the default button, so a
///   Return meant for a terminal pane leaves the runner on Tailcat (H2).
/// - **Never inside a store update.** `heard` only notes the runner; the
///   question is put up later, from its own task (H2).
/// - **One question at a time, for everything that failed together** (M3).
///   Failures are collected for ``gathering`` seconds, which covers a wake or a
///   network change taking every runner down at once, and asked as one
///   question naming each runner. Runners that fail while a question is up
///   wait for it to be answered.
/// - Asked again about a runner only after it has answered once.
@MainActor
final class RoutePrompt {
    static let shared = RoutePrompt()

    var routes = RouteSwitch()
    /// Shows the question and answers true for Connect Directly.
    var ask: @MainActor (RouteQuestion, @escaping @MainActor (Bool) -> Void) -> Void = RoutePrompt.alert
    /// Says, plainly, that a runner couldn't be moved (L5).
    var tell: @MainActor (String) -> Void = RoutePrompt.notice
    /// Dials the runner again, on its new route.
    var reconnect: @MainActor (String) -> Void = { FleetStore.shared.reconnect($0) }
    /// Seconds to gather failures before asking. Near zero in a test.
    var gathering: Double = 5

    /// Runners asked about, or waiting to be, since they last answered.
    private(set) var asked: Set<String> = []
    /// Failed, and waiting for the question.
    private(set) var waiting: [RouteQuestion.Runner] = []
    private var scheduled: Task<Void, Never>?
    /// A question is on screen.
    private(set) var showing = false

    /// A runner's state changed. Notes it when it is the failure the question
    /// is for; forgets having asked once it connects. Never presents.
    func heard(target: String, state: HostState) {
        guard !target.isEmpty else { return }
        switch state {
        case .connected:
            asked.remove(target)
            waiting.removeAll { $0.target == target }
        case .unreachable(let reason):
            guard !asked.contains(target),
                let offer = TunnelRoute.offer(
                    onTheTunnel: routes.route(of: target) == .tunnel,
                    failedBeforeTheRunner: RouteFailure(reason) == .tunnelNeverReachedTheRunner,
                    alternative: routes.alternative(of: target))
            else { return }
            asked.insert(target)
            waiting.append(.init(target: target, address: offer.address, name: routes.name(of: target)))
            schedule()
        case .connecting, .reconnecting, .notInstalled:
            return
        }
    }

    private func schedule() {
        guard scheduled == nil, !showing else { return }
        let wait = gathering
        scheduled = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(wait))
            guard let self, !Task.isCancelled else { return }
            self.scheduled = nil
            self.presentWaiting()
        }
    }

    private func presentWaiting() {
        guard !showing, !waiting.isEmpty else { return }
        let question = RouteQuestion(runners: waiting)
        waiting = []
        showing = true
        ask(question) { [weak self] yes in
            guard let self else { return }
            self.showing = false
            if yes {
                // Only the runners still failing (N5): one that came back
                // through Tailcat while the question was up has answered, and
                // `heard(.connected)` took it out of `asked`. Each runner keeps
                // its own route afterwards, in Settings > Runners (N6).
                for runner in question.runners where self.asked.contains(runner.target) {
                    if self.routes.useAddress(for: runner.target) {
                        self.reconnect(runner.target)
                    } else {
                        self.tell(Self.couldNotMove(runner.target))
                    }
                }
            }
            if !self.waiting.isEmpty { self.schedule() }
        }
    }

    static func couldNotMove(_ target: String) -> String {
        "Far Cooler couldn’t update ~/.ssh/config, so \(target) is still connecting through Tailcat."
    }

    /// Not Now is the default, so Return answers it; Connect Directly has no
    /// key at all, so only a click answers it.
    static func makeAlert(_ question: RouteQuestion) -> NSAlert {
        let alert = NSAlert()
        alert.messageText = question.title
        alert.informativeText = question.message
        alert.addButton(withTitle: DirectRouteOffer.notNow).keyEquivalent = "\r"
        alert.addButton(withTitle: DirectRouteOffer.connectDirectly).keyEquivalent = ""
        return alert
    }

    private static func alert(_ question: RouteQuestion, _ answer: @escaping @MainActor (Bool) -> Void) {
        let alert = makeAlert(question)
        let done: (NSApplication.ModalResponse) -> Void = { answer($0 == .alertSecondButtonReturn) }
        if let window = NSApp.keyWindow ?? NSApp.mainWindow {
            alert.beginSheetModal(for: window, completionHandler: done)
        } else {
            // From its own task, never from inside a store update, so a modal
            // run loop here re-enters nothing.
            done(alert.runModal())
        }
    }

    private static func notice(_ text: String) {
        let alert = NSAlert()
        alert.messageText = text
        alert.addButton(withTitle: "OK")
        if let window = NSApp.keyWindow ?? NSApp.mainWindow {
            alert.beginSheetModal(for: window)
        } else {
            alert.runModal()
        }
    }
}
