import Foundation
import Testing

@testable import Far_Cooler

/// The four rules that make Far Cooler's `~/.ssh/config` block behave, and the
/// one that keeps the two keys apart.
///
/// Each of these has teeth. An alias that shadows `github.com` breaks every
/// push from this Mac; two runners on one host sharing a `Host` entry sends
/// one's traffic to the other's account; a block below an `Include` is inert
/// because `ssh_config` is first-match-wins; and Key A in this file would make
/// deleting the block break Far Cooler, which is the one thing the two-key
/// split exists to prevent.
struct SshConfigTests {
    private func runner(
        id: String, label: String, user: String = "you", address: String = "box.tail-1234.ts.net"
    ) -> CeremonyRunner {
        runner(id: id, label: label, user: user, reach: .direct(host: address, port: 22))
    }

    private func runner(
        id: String, label: String, user: String = "you", reach: CeremonyReach
    ) -> CeremonyRunner {
        CeremonyRunner(
            id: id, label: label, alias: "", user: user,
            host_key: "SHA256:iDqoxaySm9gzxtvLrNXXpM5PimPLeBknaaNj0Rg7vz4",
            reach: reach, pending: false)
    }

    /// `alice@box` and `bob@box` are two runners on one machine, and both want
    /// `Host box`. Only one can have it.
    @Test func twoRunnersOnOneHostGetDifferentAliases() {
        let resolved = SshConfig.aliases(
            for: [
                runner(id: "alice@box", label: "box", user: "alice"),
                runner(id: "bob@box", label: "box", user: "bob"),
            ],
            avoiding: [])

        let first = resolved.aliases["alice@box"]
        let second = resolved.aliases["bob@box"]
        #expect(first == "box")
        #expect(second != nil && second != first)
        #expect(second == "bob-box")
        #expect(resolved.message != nil, "a renamed alias has to be said out loud")
    }

    /// A runner labeled `github.com` must not take over git.
    @Test func anExistingPatternCausesASuffixAndAMessage() {
        let resolved = SshConfig.aliases(
            for: [runner(id: "r1", label: "github.com")],
            avoiding: ["github.com"])

        #expect(resolved.aliases["r1"] != "github.com")
        #expect(resolved.message?.contains("github.com") == true)
    }

    /// Nothing collides, nothing is said. A message about a rename that did not
    /// happen is noise, and noise is what makes the real one invisible.
    @Test func noCollisionSaysNothing() {
        let resolved = SshConfig.aliases(for: [runner(id: "r1", label: "box")], avoiding: [])
        #expect(resolved.aliases["r1"] == "box")
        #expect(resolved.message == nil)
    }

    /// A label is user-supplied and arrives in a scanned code. `Host my runner`
    /// is two patterns, and the second one matches something.
    @Test func aLabelWithSpacesCannotBecomeTwoPatterns() {
        #expect(!SshConfig.slug("my runner").contains(" "))
        #expect(!SshConfig.slug("../../etc").contains("/"))
        #expect(SshConfig.slug("").isEmpty == false, "an empty label still needs a name")
    }

    /// `Host *` at the top of the file, an `Include`, `host=` with no space,
    /// and mixed case: everything ssh would match has to count as taken.
    @Test func everyFormOfHostCounts() throws {
        let directory = try temporaryDirectory()
        let included = directory.appendingPathComponent("extra")
        try "Host build-vm\n  HostName 10.0.0.4\n".write(
            to: included, atomically: true, encoding: .utf8)

        let config = directory.appendingPathComponent("config")
        try """
        Include \(included.path)
        Host *
          ServerAliveInterval 30
        HOST=github.com
        Host work-mini deploy-box
        """.write(to: config, atomically: true, encoding: .utf8)

        let patterns = SshConfig.patternsInUse(from: config)
        #expect(patterns.contains("*"))
        #expect(patterns.contains("github.com"), "`HOST=` is a keyword ssh reads")
        #expect(patterns.contains("work-mini"))
        #expect(patterns.contains("deploy-box"), "one Host line can name several patterns")
        #expect(patterns.contains("build-vm"), "an Include is content ssh reads at that point")
    }

    /// Far Cooler's own block is skipped, or every rewrite finds last time's
    /// aliases and suffixes them again: `box`, `box-2`, `box-3`, once per
    /// enrollment, forever.
    @Test func farCoolersOwnBlockIsNotACollision() throws {
        let directory = try temporaryDirectory()
        let config = directory.appendingPathComponent("config")
        try """
        \(SshConfig.beginMarker)
        Host box
          HostName box.tail-1234.ts.net
        \(SshConfig.endMarker)
        Host work-mini
          HostName 10.0.0.9
        """.write(to: config, atomically: true, encoding: .utf8)

        let patterns = SshConfig.patternsInUse(from: config)
        #expect(!patterns.contains("box"), "our own alias is not somebody else's pattern")
        #expect(patterns.contains("work-mini"), "and everything outside the fence still counts")
    }

    /// **Key A is never in this file.**
    ///
    /// Far Cooler passes it with `-i` on the command line, so deleting this
    /// block takes Zed's access away and leaves Far Cooler's untouched. The
    /// moment an `IdentityFile` here points at the key directory, that stops
    /// being true and a person tidying their config loses their fleet.
    @Test func keyAIsNeverInTheConfig() {
        let keyA = DeviceKey.directory.appendingPathComponent("user_01ABC")
        #expect(throws: SshConfigError.self) {
            try SshConfig.assertNotKeyA("  IdentityFile \(keyA.path)")
        }
        #expect(throws: SshConfigError.self) {
            try SshConfig.assertNotKeyA("  IdentityFile \(SshConfig.tildeCollapsed(keyA))")
        }
    }

    /// And the block this actually composes passes that check, with Key B.
    @Test func theBlockNamesKeyBAndPinsTheAddress() throws {
        let identity = ShellKey.directory.appendingPathComponent("farcooler-macbook-air")
        let lines = SshConfig.block(
            for: runner(id: "r1", label: "box"), alias: "box", identity: identity)

        #expect(lines == [
            "Host box",
            "  HostName box.tail-1234.ts.net",
            "  Port 22",
            "  User you",
            "  IdentityFile ~/.ssh/farcooler-macbook-air",
            "  IdentitiesOnly yes",
        ])
        // The fence writer refuses an entry carrying a newline or a marker, and
        // its read-back check counts non-empty lines. A blank separator here
        // would make the file fail to read back as what was written.
        for line in lines {
            #expect(!line.contains("\n") && !line.trimmingCharacters(in: .whitespaces).isEmpty)
            try SshConfig.assertNotKeyA(line)
        }
    }

    /// A tunneled runner has no address, so `HostName` has nothing true to say
    /// and OpenSSH is given a command it can execute instead.
    ///
    /// **The token never lands in this file.** It is read by every editor and
    /// tool on the machine, and the id is enough — the CLI resolves the token
    /// from its own runner store.
    @Test func aTunneledRunnerGetsAProxyCommandAndNoHostName() throws {
        let identity = ShellKey.directory.appendingPathComponent("farcooler-macbook-air")
        let lines = SshConfig.block(
            for: runner(id: "r1", label: "box", reach: .tailcat(token: "tc-secret")),
            alias: "box",
            identity: identity)

        #expect(lines == [
            "Host box",
            "  ProxyCommand farcooler runner pipe r1",
            "  User you",
            "  IdentityFile ~/.ssh/farcooler-macbook-air",
            "  IdentitiesOnly yes",
        ])
        for line in lines {
            #expect(!line.contains("tc-secret"), "the token landed in ssh config: \(line)")
            #expect(!line.contains("\n") && !line.trimmingCharacters(in: .whitespaces).isEmpty)
            try SshConfig.assertNotKeyA(line)
        }
    }

    /// The home directory is collapsed, because this file is frequently in a
    /// dotfiles repository shared between machines where `/Users/someone` is
    /// wrong on every machine but one.
    @Test func theIdentityPathIsWrittenRelativeToHome() {
        let inside = URL(fileURLWithPath: NSHomeDirectory()).appendingPathComponent(".ssh/key")
        #expect(SshConfig.tildeCollapsed(inside) == "~/.ssh/key")
        #expect(SshConfig.tildeCollapsed(URL(fileURLWithPath: "/etc/ssh/key")) == "/etc/ssh/key")
    }

    // MARK: - The write

    // Six tests over the one thing this file used to refuse to do. They aim at a
    // scratch path, never `~/.ssh/config`: a suite that rewrote the config of
    // whoever ran it would be worse than no suite. What is NOT here is the byte
    // mechanics — the lock, the two `fsync`s, the checksummed backup — which
    // belong to `crates/fence/src/lib.rs` and are tested beside the
    // `authorized_keys` fixtures. What is here is the boundary: that Swift reaches
    // that writer at all, and that each word it can answer becomes the right
    // sentence.

    /// **The block goes above an `Include`, or the whole feature is inert.**
    ///
    /// `ssh_config` is first-match-wins per keyword. A block appended below an
    /// `Include ~/.ssh/config.d/*` or a `Host *` never gets its `IdentityFile`
    /// offered — ssh tries the wrong key and Zed reports an authentication
    /// failure for a runner that enrolled perfectly. This is the test that says
    /// `Placement::First` is actually what crossed the FFI.
    @Test func theBlockLandsAboveEverythingSshAlreadyReads() throws {
        let config = try scratchConfig(
            """
            Include ~/.ssh/config.d/*
            Host *
              ServerAliveInterval 30
            """)
        let identity = ShellKey.directory.appendingPathComponent("farcooler-macbook-air")
        let lines = SshConfig.block(
            for: runner(id: "r1", label: "box"), alias: "box", identity: identity)

        try SshConfig.write(lines, identity: identity, into: config)

        let written = try String(contentsOf: config, encoding: .utf8)
        let begin = try #require(written.range(of: SshConfig.beginMarker))
        let include = try #require(written.range(of: "Include ~/.ssh/config.d/*"))
        #expect(begin.lowerBound < include.lowerBound, "a block below an Include wins nothing")
        #expect(written.contains("  IdentityFile ~/.ssh/farcooler-macbook-air"))
        #expect(written.contains(SshConfig.endMarker))
        #expect(written.contains("ServerAliveInterval 30"), "everything outside the fence survives")
    }

    /// A second write replaces the block rather than stacking another one, and
    /// leaves what is outside it alone. This is the ordinary case — every
    /// enrollment rewrites the whole block.
    @Test func writingTwiceLeavesOneBlock() throws {
        let config = try scratchConfig("Host work-mini\n  HostName 10.0.0.9\n")
        let identity = ShellKey.directory.appendingPathComponent("farcooler-macbook-air")

        try SshConfig.write(["Host box", "  HostName box.tail-1234.ts.net"], identity: identity,
            into: config)
        try SshConfig.write(["Host mini", "  HostName mini.tail-1234.ts.net"], identity: identity,
            into: config)

        let written = try String(contentsOf: config, encoding: .utf8)
        #expect(written.components(separatedBy: SshConfig.beginMarker).count - 1 == 1)
        #expect(written.contains("Host mini"))
        #expect(!written.contains("Host box"), "the block is composed whole every time")
        #expect(written.contains("Host work-mini"), "a person's own entry is not ours to touch")
    }

    /// Nothing enrolled means no block, not two comment lines left behind
    /// forever. A Mac that revoked everything should look untouched.
    @Test func anEmptyBlockIsRemovedRatherThanLeftEmpty() throws {
        let config = try scratchConfig("Host work-mini\n  HostName 10.0.0.9\n")
        let identity = ShellKey.directory.appendingPathComponent("farcooler-macbook-air")

        try SshConfig.write(["Host box"], identity: identity, into: config)
        try SshConfig.write([], identity: identity, into: config)

        let written = try String(contentsOf: config, encoding: .utf8)
        #expect(!written.contains(SshConfig.beginMarker))
        #expect(written.contains("Host work-mini"))
    }

    /// A fence somebody edited by hand is refused, not repaired — and the file is
    /// left exactly as it was. Guessing where the block ends rewrites lines Far
    /// Cooler did not write, in the file that decides whether ssh works at all.
    @Test func aDamagedFenceRefusesAndChangesNothing() throws {
        let broken = """
        \(SshConfig.beginMarker)
        Host box
        \(SshConfig.beginMarker)
        Host mini
        \(SshConfig.endMarker)
        """
        let config = try scratchConfig(broken)

        #expect(throws: SshConfigError.damaged) {
            try SshConfig.write(["Host new"], identity: ShellKey.directory, into: config)
        }
        #expect(try String(contentsOf: config, encoding: .utf8) == broken)
        // And the sentence a person reads is this app's, never a Rust error's.
        // `FenceError::Damaged` carries "3 opening and 1 closing markers", which
        // is a log line.
        let sentence = try #require(SshConfigError.damaged.errorDescription)
        #expect(sentence.contains("~/.ssh/config") && sentence.contains("Nothing was changed"))
        #expect(!sentence.contains("marker") || !sentence.contains("closing"))
    }

    /// A path with no home directory under it answers `missing`, and this app
    /// turns that word into a sentence. The word crossing the FFI is the contract;
    /// the sentence is ours.
    ///
    /// TWO levels missing, not one, and that is the actual contract rather than a
    /// quirk of this test: the writer CREATES the last directory component at
    /// 0700, because a runner where nobody has ever used SSH has no `.ssh` yet and
    /// 0700 is the mode sshd's `StrictModes` insists on. So `missing` means the
    /// directory above that one is gone — a home directory that is not there.
    @Test func aPathWithNoHomeDirectoryUnderItSaysSoInThisAppsWords() throws {
        let nowhere = try temporaryDirectory()
            .appendingPathComponent("no-such-home")
            .appendingPathComponent(".ssh")
            .appendingPathComponent("config")

        #expect(throws: SshConfigError.missing) {
            try SshConfig.write(["Host box"], identity: ShellKey.directory, into: nowhere)
        }
        // Every failure sentence says the same two things, because they are the
        // two a person needs: nothing changed, and the keys are still enrolled.
        for error: SshConfigError in [.missing, .io, .writerUnavailable] {
            let sentence = try #require(error.errorDescription)
            #expect(sentence.contains("~/.ssh/config"))
            #expect(sentence.contains("keys were enrolled"), "\(error): \(sentence)")
        }
    }

    /// Key A is refused before a byte is written, not after.
    ///
    /// The check is on the composed line rather than trusted from the call site,
    /// and it has to run first: a file already rewritten is not somewhere to
    /// discover that the two keys got mixed up.
    @Test func keyAIsRefusedBeforeTheFileIsTouched() throws {
        let original = "Host work-mini\n  HostName 10.0.0.9\n"
        let config = try scratchConfig(original)
        let keyA = DeviceKey.directory.appendingPathComponent("user_01ABC")

        #expect(throws: SshConfigError.keyAInConfig) {
            try SshConfig.write(
                ["Host box", "  IdentityFile \(SshConfig.tildeCollapsed(keyA))"],
                identity: keyA, into: config)
        }
        #expect(try String(contentsOf: config, encoding: .utf8) == original)
    }

    private func scratchConfig(_ contents: String) throws -> URL {
        let config = try temporaryDirectory().appendingPathComponent("config")
        try contents.write(to: config, atomically: true, encoding: .utf8)
        return config
    }

    private func temporaryDirectory() throws -> URL {
        let directory = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("farcooler-ssh-config-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        return directory
    }
}
