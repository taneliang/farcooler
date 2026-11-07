import Foundation
import os

/// Whether an update has to wait because this Mac's runner is mid-upgrade
/// (ov-963 M3).
///
/// An engine cutover that's been started can only be finished or undone by
/// the build that started it. Sparkle replacing the bundle in the middle
/// leaves the runner on a build that can do neither, so the updater asks the
/// CLI (`daemon held`, which reads the runner's own files and nothing else)
/// before it checks, and says why it's waiting when the answer is yes.
enum UpdateHold {
    /// What's said while an update waits, to the menu, an alert or the CLI.
    static let sentence =
        "Far Cooler can’t update yet. This Mac’s runner is mid-upgrade, and updating now could leave it "
        + "unable to finish. Finish the upgrade first, then check again."

    /// The CLI's word for it when `farcooler app update` is the one asking.
    static let word = "cutover-held"

    struct Held: LocalizedError {
        var errorDescription: String? { UpdateHold.sentence }
    }

    /// Throws `Held` when an update must wait. `cached` is the last answer the
    /// app read, or nil before there's been one, which asks `probe` once
    /// rather than guessing either way.
    static func check(cached: Bool?, probe: () -> Bool) throws {
        if cached ?? probe() { throw Held() }
    }

    /// The answer in `daemon held --json`'s stdout. Anything unreadable is
    /// not held: a CLI that can't answer must not freeze updates for good.
    static func held(inOutput output: String) -> Bool {
        guard let data = output.data(using: .utf8),
            let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return false }
        return object["held"] as? Bool == true
    }

    /// Ask the bundled CLI, blocking until it answers. A few milliseconds: it
    /// reads two small files. Call it off the main thread where there's a
    /// choice.
    static func probe() -> Bool {
        guard let binary = CLI.binary else { return false }
        let process = Process()
        process.executableURL = URL(fileURLWithPath: binary)
        process.arguments = ["--json", "daemon", "held"]
        process.environment = CLI.environment
        let out = Pipe()
        process.standardOutput = out
        process.standardError = FileHandle.nullDevice
        do { try process.run() } catch { return false }
        let data = out.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        return process.terminationStatus == 0 && held(inOutput: String(decoding: data, as: UTF8.self))
    }
}

/// The last answer, readable from Sparkle's delegate thread.
final class UpdateHoldCache: Sendable {
    private let state = OSAllocatedUnfairLock<Bool?>(initialState: nil)
    var value: Bool? { state.withLock { $0 } }
    func set(_ held: Bool) { state.withLock { $0 = held } }

    /// Read the CLI again, off the main thread.
    func refresh() async {
        let held = await Task.detached { UpdateHold.probe() }.value
        set(held)
    }
}
