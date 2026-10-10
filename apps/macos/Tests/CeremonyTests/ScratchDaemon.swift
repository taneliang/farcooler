import Foundation

@testable import Far_Cooler

/// Ending a test's scratch daemon, and the tmux server it started, which
/// `daemon stop` leaves running. Each server left behind is a process and a
/// shell for as long as the Mac is up; `test.sh` runs every test under
/// `scripts/tmux-leak-check.py`, which fails the run on one.
enum ScratchDaemon {
    /// The one place a test names `FARCOOLER_HOME`: a scratch home and its
    /// own config.toml together (`config`, beside the home unless a test
    /// keeps it elsewhere), so no launch can read or write this Mac's shared
    /// `[agents] projector` and adapters (ov-394). `ScratchConfigTests` fails
    /// any other test source that names the key.
    static func isolate(_ environment: inout [String: String], home: String, config: String? = nil) {
        environment["FARCOOLER_HOME"] = home
        environment["FARCOOLER_CONFIG"] = config ?? home + "/config.toml"
    }

    /// Stop the daemon `cli` runs under `farcoolerHome`, then its tmux
    /// server, by the socket `status` names in its recovery line.
    static func stop(cli: String, farcoolerHome: String) async {
        var environment = ProcessInfo.processInfo.environment
        isolate(&environment, home: farcoolerHome)
        let status = await ProcessRunner.run(cli, ["status"], environment: environment, deadline: 30)
        _ = await ProcessRunner.run(cli, ["daemon", "stop"], environment: environment, deadline: 30)
        await killWhoeverStillHolds(farcoolerHome + "/farcoolerd.lock")
        let said = String(decoding: status.stdout, as: UTF8.self)
        guard let socket = said.firstMatch(of: /tmux -L (farcooler-[0-9a-f]+)/)?.1,
            let tmux = ["/opt/homebrew/bin/tmux", "/usr/local/bin/tmux", "/usr/bin/tmux"]
                .first(where: { FileManager.default.isExecutableFile(atPath: $0) })
        else { return }
        _ = await ProcessRunner.run(tmux, ["-L", String(socket), "kill-server"], environment: environment, deadline: 10)
    }

    /// The daemon holds its lock file for as long as it lives, so anything
    /// still holding THIS scratch home's lock after `daemon stop` is the test's
    /// own daemon that stop did not reach: one still coming up on a loaded Mac
    /// when the stop asked, say. Left alone it outlives the suite and blocks
    /// removing the worktree. Given a few seconds to go on its own, then killed.
    private static func killWhoeverStillHolds(_ lock: String) async {
        func holders() async -> [pid_t] {
            let ran = await ProcessRunner.run("/usr/sbin/lsof", ["-t", lock], deadline: 10)
            return String(decoding: ran.stdout, as: UTF8.self).split(whereSeparator: \.isWhitespace).compactMap { pid_t($0) }
        }
        for _ in 0..<50 {
            if await holders().isEmpty { return }
            try? await Task.sleep(for: .milliseconds(100))
        }
        for pid in await holders() { kill(pid, SIGKILL) }
    }
}
