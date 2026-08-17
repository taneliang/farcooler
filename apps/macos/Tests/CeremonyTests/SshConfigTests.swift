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
        CeremonyRunner(
            id: id, label: label, alias: "", address: address, user: user, port: 22,
            host_key: "SHA256:iDqoxaySm9gzxtvLrNXXpM5PimPLeBknaaNj0Rg7vz4", pending: false)
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
            "  User you",
            "  Port 22",
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

    /// The home directory is collapsed, because this file is frequently in a
    /// dotfiles repository shared between machines where `/Users/someone` is
    /// wrong on every machine but one.
    @Test func theIdentityPathIsWrittenRelativeToHome() {
        let inside = URL(fileURLWithPath: NSHomeDirectory()).appendingPathComponent(".ssh/key")
        #expect(SshConfig.tildeCollapsed(inside) == "~/.ssh/key")
        #expect(SshConfig.tildeCollapsed(URL(fileURLWithPath: "/etc/ssh/key")) == "/etc/ssh/key")
    }

    private func temporaryDirectory() throws -> URL {
        let directory = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("farcooler-ssh-config-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        return directory
    }
}
