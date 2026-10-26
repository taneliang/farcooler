import Foundation
import MetricKit

/// This phone's crashes and hangs, sent to the owner's own runner (ov-797).
///
/// MetricKit hands a subscriber each crash and hang diagnostic on the launch
/// after it happened. Each is read down to its signature (`CrashSignature`,
/// in AgentKit, where it is tested) the moment it arrives, so nothing else
/// from it is ever kept, and the signatures wait in `CrashOutbox` until a
/// runner that is the owner's takes them: one that advertises
/// `crash_reports` and has said, over this connection, that this phone is
/// its owner's (`CrashOutbox.mayReport`). A member's phone never sends its
/// crashes to the owner's runner, and nothing goes anywhere else.
///
/// Sent once: the first owner's runner to take a signature has it, so a
/// phone with two runners files one card, not two.
@MainActor
final class CrashReports: NSObject, MXMetricManagerSubscriber {
    static let shared = CrashReports()

    private static let key = "crash.outbox"
    private var outbox: CrashOutbox
    private var sending = false
    /// Runners that said they won't take a crash from this phone, by id,
    /// asked no more until the next launch.
    private var notTaking: Set<String> = []

    private override init() {
        let saved = UserDefaults.standard.data(forKey: Self.key)
        outbox = saved.flatMap { try? JSONDecoder().decode(CrashOutbox.self, from: $0) } ?? CrashOutbox()
        super.init()
    }

    /// Subscribe, at launch: MetricKit delivers the last launch's
    /// diagnostics to whoever is subscribed then.
    func start() {
        MXMetricManager.shared.add(self)
    }

    nonisolated func didReceive(_ payloads: [MXDiagnosticPayload]) {
        let own = Bundle.main.executableURL?.lastPathComponent ?? ""
        var read: [CrashSignature] = []
        for payload in payloads {
            for crash in payload.crashDiagnostics ?? [] {
                if let s = CrashSignature.read(diagnostic: crash.jsonRepresentation(), kind: .crash, ownBinary: own) { read.append(s) }
            }
            for hang in payload.hangDiagnostics ?? [] {
                if let s = CrashSignature.read(diagnostic: hang.jsonRepresentation(), kind: .hang, ownBinary: own) { read.append(s) }
            }
        }
        guard !read.isEmpty else { return }
        Task { @MainActor in CrashReports.shared.keep(read) }
    }

    private func keep(_ signatures: [CrashSignature]) {
        outbox.add(signatures)
        save()
    }

    private func save() {
        if let data = try? JSONEncoder().encode(outbox) { UserDefaults.standard.set(data, forKey: Self.key) }
    }

    /// Send what's waiting to `connection`'s runner, if it is the owner's.
    /// Called on every poll; does nothing when nothing waits.
    func send(through connection: Connection) async {
        let runner = connection.host?.id.uuidString ?? ""
        guard !sending, !outbox.waiting.isEmpty, !notTaking.contains(runner),
            CrashOutbox.mayReport(runnerTakesCrashes: connection.daemon?.can(.crashReports) == true, you: connection.talk.people?.you)
        else { return }
        sending = true
        defer { sending = false }
        for signature in outbox.waiting {
            do {
                _ = try await connection.rpc("crash.report", signature.wire)
                outbox.remove(signature)
            } catch let ClientCore.CoreError.rejected(_, word, _) {
                switch CrashOutbox.refused(word: word) {
                case .drop:
                    // Refused for what it is: it never will be taken, so it
                    // isn't kept to be tried forever.
                    outbox.remove(signature)
                case .notThisRunner:
                    notTaking.insert(runner)
                    return
                case .later:
                    return
                }
            } catch {
                // The link: kept for the next poll or another runner.
                break
            }
            save()
        }
    }

    #if DEBUG
    /// Under `-crash-after-launch`, an uncaught exception five seconds in,
    /// as ov-787's was: a debug build's way to see a crash reach the runner.
    /// Launch with it without a debugger attached (`xcrun devicectl device
    /// process launch … -- -crash-after-launch`), then launch again without
    /// it, and the runner files the card.
    static func crashIfAsked() {
        guard ProcessInfo.processInfo.arguments.contains("-crash-after-launch") else { return }
        DispatchQueue.main.asyncAfter(deadline: .now() + 5) {
            NSException(name: .invalidArgumentException, reason: "Asked for by -crash-after-launch (ov-797)").raise()
        }
    }
    #endif
}
