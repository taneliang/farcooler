import Foundation
import Testing

@testable import Far_Cooler

/// The test process never runs an installed Far Cooler's CLI (ov-1091).
///
/// It is a Local build, so `CLI.binary` used to fall through to
/// `~/.local/bin/farcooler-local`: every test whose `DaemonClient` had no stub
/// ran the owner's installed Local app against the owner's Local install, and a
/// September one wrote `farcooler-<install>.tmux.conf` into the leak check's
/// TMPDIR. On a Mac with an app installed this fails without the switch that
/// `QuietTestProcess` sets at load; on CI, with none installed, it passes
/// either way, which is the behaviour the Mac now matches.
struct TestProcessCLITests {
    @Test func theTestProcessTurnsOffTheInstalledCLI() {
        #expect(ProcessInfo.processInfo.environment["FARCOOLER_NO_INSTALLED_CLI"] != nil)
    }

    @Test func theCLIIsNeverOneAnAppInstalled() {
        guard let binary = CLI.binary else { return }
        let home = NSHomeDirectory()
        #expect(!binary.hasPrefix("/usr/local/bin/"), "\(binary)")
        #expect(!binary.hasPrefix("\(home)/.local/bin/"), "\(binary)")
    }
}
