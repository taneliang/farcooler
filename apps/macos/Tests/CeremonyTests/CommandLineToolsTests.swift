import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

@MainActor
struct CommandLineToolsTests {
    /// The names in `~/.local/bin` are the names the fence's forced command
    /// asks sshd to run — `forced_program` in `crates/fence/src/lib.rs`, which
    /// has no fallback on purpose. A tool added here with its bare name would
    /// be linked under one name and looked for under another, and the failure
    /// lands on the device being added rather than on this Mac.
    @Test func everyToolIsLinkedUnderItsChannelName() {
        for tool in CommandLineTools.tools {
            #expect(tool.link == CommandLineTools.channelName(for: tool.bundled))
        }
    }

    /// Both halves, and only these two: the CLI a shell finds and the daemon an
    /// SSH session execs. `build-app.sh` copies exactly these out of cargo.
    @Test func bothBinariesAreLinked() {
        #expect(CommandLineTools.tools.map(\.bundled) == ["farcooler", "farcoolerd"])
    }

    /// A non-stable build hands back the bare names it may have claimed before
    /// it knew about channels — and a stable build has none to hand back,
    /// because the bare names are its own.
    @Test func onlyANonStableBuildHasNamesToHandBack() {
        let stranded = CommandLineTools.strandedLinks.map(\.link)

        if AppVersion.isRelease {
            #expect(stranded.isEmpty)
        } else {
            #expect(stranded == ["farcooler", "farcoolerd"])
        }
    }
}
