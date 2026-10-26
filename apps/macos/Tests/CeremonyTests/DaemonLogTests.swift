import Foundation
import Testing

@testable import Far_Cooler

/// Where the Help item looks for the runner's log (ov-798): the same folder
/// `crates/daemon/src/daemon_log.rs` writes.
struct DaemonLogTests {
    @Test func aNamedHomeKeepsItsLogsInsideItself() {
        let dir = DaemonLog.folder(environment: ["FARCOOLER_HOME": "/tmp/fc9"], channel: "canary", home: "/Users/x")
        #expect(dir.path == "/tmp/fc9/logs")
    }

    @Test(arguments: [
        ("stable", "FarCooler"), ("preview", "FarCoolerPreview"),
        ("canary", "FarCoolerCanary"), ("local", "FarCoolerLocal"),
    ])
    func eachChannelHasItsOwnFolderUnderLibraryLogs(channel: String, name: String) {
        let dir = DaemonLog.folder(environment: [:], channel: channel, home: "/Users/x")
        #expect(dir.path == "/Users/x/Library/Logs/com.farcooler.\(name)")
        #expect(DaemonLog.file(folder: dir).lastPathComponent == "farcoolerd.log")
    }
}
