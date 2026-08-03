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
/// Replacing costs nothing that matters. Terminals are tmux windows and outlive
/// the daemon entirely; agent shims reconnect on the next start; durable state
/// is committed to SQLite per call.
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
