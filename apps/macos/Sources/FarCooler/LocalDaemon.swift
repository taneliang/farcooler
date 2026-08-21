import Foundation

/// This Mac's daemon, which the app owns.
///
/// The daemon outlives the app on purpose — the premise is agents working while
/// the laptop is shut — so it cannot be a child process that dies with the
/// window. What it can be is the app's responsibility: the bundle ships the
/// daemon beside the CLI, and before the app talks to anything it makes sure
/// the process answering this machine's socket is that one.
///
/// Without this the first daemon to start kept the socket for as long as it
/// stayed alive, through every rebuild and every update. A CLI and a daemon
/// built from different source speak the same protocol perfectly and still
/// behave like two different programs, and the symptom is a bug that was fixed
/// and tested going on reproducing in front of you. `farcooler status` said
/// MISMATCH, and nothing was obliged to read it.
///
/// Replacing is not free, and this comment used to say it was: "terminals are
/// tmux windows and outlive the daemon entirely; agent shims reconnect on the
/// next start; durable state is committed to SQLite per call." The first and
/// third are true. The second is not, and it is the expensive one — the daemon
/// owns each agent transcript outright, in memory, bounded by
/// `TRANSCRIPT_LIMIT` in `crates/daemon/src/agent_supervisor.rs`, so a shim
/// that reconnects reconnects to a conversation that no longer exists. What a
/// replacement costs is every agent chat history on this Mac, plus anything
/// typed and not yet sent. See `DaemonSkew`.
///
/// Which is why nothing else in the app replaces a daemon without asking. This
/// one call is the exception, deliberately and narrowly: it runs at launch,
/// before the first read, where the alternative is not "keep your history" but
/// "spend the session talking to a different program from the one this app was
/// built with" — the failure it was written for, where a fix that shipped goes
/// on reproducing because yesterday's daemon still holds the socket. Anywhere
/// the daemon is already the right one, `daemon ensure` changes nothing at all.
///
/// It is worth knowing that this window is not zero: an app relaunch is the one
/// moment Far Cooler will replace a daemon on your behalf, and on a Mac that
/// updated overnight that is the moment the local agent histories go. Making
/// that a question rather than a launch step means teaching `ensure` to report
/// what it WOULD do without doing it, and is a change to the CLI's contract
/// (`crates/cli/src/daemon_link.rs`, `Ensured`), not to this file.
@MainActor
final class LocalDaemon: ObservableObject {
    static let shared = LocalDaemon()

    enum State: Equatable {
        case unknown
        /// A daemon built from this app's source is running.
        case running(build: String)
        /// The app could not put one there. The message is the CLI's own.
        case failed(String)

        /// The failure worth putting in front of someone, or nil while it is fine.
        var problem: String? {
            if case .failed(let message) = self { return message }
            return nil
        }
    }

    @Published private(set) var state: State = .unknown

    private var inFlight = false

    /// Leave this machine running the daemon from this bundle.
    ///
    /// Called before the first read, and again whenever the event stream drops —
    /// which is what a daemon going away looks like from here, and the moment
    /// something else could take the socket.
    @discardableResult
    func ensure() async -> State {
        // The window's `.task` and an event-stream reconnect can both arrive at
        // once. Two ensures racing would each see the other's half-replaced
        // daemon, so the second waits for nothing and takes the first's word.
        guard !inFlight else { return state }
        inFlight = true
        defer { inFlight = false }

        let result = await CLI.run(["--json", "daemon", "ensure"])
        guard result.ok, let data = result.output.data(using: .utf8),
            let body = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let build = body["daemonVersion"] as? String
        else {
            state = .failed(result.output.isEmpty ? "The daemon did not start." : result.output)
            return state
        }

        state = .running(build: build)
        return state
    }
}
