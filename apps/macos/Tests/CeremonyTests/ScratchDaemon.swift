import Foundation

@testable import Far_Cooler

/// Ending a test's scratch daemon, and the tmux server it started, which
/// `daemon stop` leaves running. Each server left behind is a process and a
/// shell for as long as the Mac is up; `test.sh` runs every test under
/// `scripts/tmux-leak-check.py`, which fails the run on one.
enum ScratchDaemon {
    /// Stop the daemon `cli` runs under `farcoolerHome`, then its tmux
    /// server, by the socket `status` names in its recovery line.
    static func stop(cli: String, farcoolerHome: String) async {
        var environment = ProcessInfo.processInfo.environment
        environment["FARCOOLER_HOME"] = farcoolerHome
        environment["FARCOOLER_CONFIG"] = farcoolerHome + "/config.toml"
        let status = await ProcessRunner.run(cli, ["status"], environment: environment, deadline: 30)
        _ = await ProcessRunner.run(cli, ["daemon", "stop"], environment: environment, deadline: 30)
        let said = String(decoding: status.stdout, as: UTF8.self)
        guard let socket = said.firstMatch(of: /tmux -L (farcooler-[0-9a-f]+)/)?.1,
            let tmux = ["/opt/homebrew/bin/tmux", "/usr/local/bin/tmux", "/usr/bin/tmux"]
                .first(where: { FileManager.default.isExecutableFile(atPath: $0) })
        else { return }
        _ = await ProcessRunner.run(tmux, ["-L", String(socket), "kill-server"], environment: environment, deadline: 10)
    }
}
