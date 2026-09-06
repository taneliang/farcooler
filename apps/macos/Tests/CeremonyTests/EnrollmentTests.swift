import Foundation
import Testing

@testable import Far_Cooler

struct EnrollmentTests {
    /// The regression that left a phone unable to reach the Mac it was added
    /// from: this Mac is the EMPTY target, and an empty `--runner` is not
    /// "local" to the CLI — it is an ssh destination with no name.
    @Test func thisMacIsEnrolledWithNoRunnerFlagAtAll() {
        let arguments = Enrollment.arguments(
            key: "ssh-ed25519 AAAAC3Nz", label: "iPhone", clientID: "id-1", scope: "control",
            shell: false, nodeKey: "", runner: "")

        #expect(!arguments.contains("--runner"))
        #expect(!arguments.contains(""))
    }

    @Test func anotherRunnerIsStillNamed() throws {
        let arguments = Enrollment.arguments(
            key: "ssh-ed25519 AAAAC3Nz", label: "iPhone", clientID: "id-1", scope: "control",
            shell: false, nodeKey: "", runner: "e-liang@cosmo")

        let flag = try #require(arguments.firstIndex(of: "--runner"))
        #expect(arguments[flag + 1] == "e-liang@cosmo")
    }

    /// The flag whose ABSENCE is the restricted line, on both routes.
    @Test func shellAccessIsAskedForOnlyWhenItIsWanted() {
        let restricted = Enrollment.arguments(
            key: "k", label: "l", clientID: "c", scope: "control", shell: false, nodeKey: "",
            runner: "")
        let plain = Enrollment.arguments(
            key: "k", label: "l", clientID: "c", scope: "host_admin", shell: true, nodeKey: "",
            runner: "")

        #expect(!restricted.contains("--shell-access"))
        #expect(plain.contains("--shell-access"))
    }

    /// Order matters to nobody but a reader, but the subcommand has to survive
    /// the flag going missing in front of it.
    @Test func theSubcommandSurvivesTheMissingFlag() {
        let arguments = Enrollment.arguments(
            key: "k", label: "l", clientID: "c", scope: "control", shell: false, nodeKey: "",
            runner: "")

        #expect(arguments.prefix(3) == ["--json", "client", "enroll"])
    }

    // MARK: - The node key

    /// A real one, 43 characters of unpadded base64-URL — the key an iPhone
    /// actually minted in `.claude/agent/reports/end-to-end-on-device.md`. The
    /// `_` in it is the alphabet, observed rather than assumed.
    static let nodeKey = "3klO7naorDKjqf2sm4MV0zWlyTdpZn4Blq03K_crbwc"

    /// **The break the on-device run found.** A ceremony wrote a perfectly good
    /// `authorized_keys` line and no `--node-key` on it, so the runner's tunnel
    /// refused the phone that had just been paired to it: `no_answer` after
    /// 10.1 s, against 0.5 s to `connected` once the key was on the line by
    /// hand. `enrollment::enroll` passes this to `fence::render`, and nothing
    /// else in the tree writes a node key onto another device's line.
    @Test func theRestrictedLineCarriesTheDevicesNodeKey() throws {
        let arguments = Enrollment.arguments(
            key: Self.keyA, label: "iPhone 17", clientID: "farcooler-1", scope: "control",
            shell: false, nodeKey: Self.nodeKey, runner: "e-liang@cosmo")

        let flag = try #require(arguments.firstIndex(of: "--node-key"))
        #expect(arguments[flag + 1] == Self.nodeKey)
    }

    /// A device with no node key sends no flag at all, rather than an empty
    /// value.
    ///
    /// `fence::render` refuses any node key `usable_node_key` refuses, and the
    /// empty string is one of those — so `--node-key ""` would refuse a v=1
    /// device, or a phone whose own mint failed, an enrollment it is entitled
    /// to. Absence means "this device asked for no tunnel", which is a
    /// different thing from an unusable key.
    @Test func aDeviceWithNoNodeKeySendsNoFlag() {
        let arguments = Enrollment.arguments(
            key: Self.keyA, label: "MacBook Air", clientID: "farcooler-1", scope: "control",
            shell: false, nodeKey: "", runner: "e-liang@cosmo")

        #expect(!arguments.contains("--node-key"))
        #expect(!arguments.contains(""))
    }

    /// Never on the plain line, even when one is asked for with a key in hand.
    ///
    /// The node key is written INTO the forced command, and Key B's line has no
    /// forced command — the daemon refuses the pair rather than writing a line
    /// that admits nothing, and this is this Mac never asking for it.
    @Test func theShellLineNeverCarriesANodeKey() {
        let plain = Enrollment.arguments(
            key: Self.keyB, label: "MacBook Air", clientID: "farcooler-1", scope: "host_admin",
            shell: true, nodeKey: Self.nodeKey, runner: "e-liang@cosmo")

        #expect(plain.contains("--shell-access"))
        #expect(!plain.contains("--node-key"))
        #expect(!plain.contains(Self.nodeKey))
    }

    /// And the wiring, not only the builder: a Mac's TWO calls, from one
    /// `enroll`, with the key on the restricted one and nothing on the plain
    /// one. `arguments` being right is not the same claim as `enroll` calling it
    /// right, and the second is the one a ceremony runs.
    @Test func oneEnrollSendsTheNodeKeyOnKeyAsCallAlone() async throws {
        let runners = [
            CeremonyRunner(
                id: "e-liang@cosmo", label: "cosmo", alias: "", user: "e-liang",
                host_key: "SHA256:whatever", reach: .direct(host: "cosmo.example", port: 22),
                pending: true)
        ]
        let calls = Recorder()

        _ = await Enrollment.enroll(
            keyA: Self.keyA, keyB: Self.keyB, label: "MacBook Air", clientID: "farcooler-1",
            scope: "control", nodeKey: Self.nodeKey, on: runners,
            using: { arguments in
                await calls.record(arguments)
                return (true, "{}")
            })

        let recorded = await calls.calls
        #expect(recorded.count == 2)
        let restricted = try #require(recorded.first { !$0.contains("--shell-access") })
        let plain = try #require(recorded.first { $0.contains("--shell-access") })
        #expect(restricted.contains("--node-key"))
        #expect(restricted.contains(Self.nodeKey))
        #expect(!plain.contains("--node-key"))
        #expect(!plain.contains(Self.nodeKey))
    }

    /// Every command line one enrollment ran, in order.
    private actor Recorder {
        var calls: [[String]] = []
        func record(_ arguments: [String]) { calls.append(arguments) }
    }

    // MARK: - The id a device is enrolled under

    /// Two real Ed25519 public keys, the same pair
    /// `crates/client/src/ceremony.rs`'s own tests use, so the ids asserted here
    /// and the ids asserted there are the ids of the same keys.
    static let keyA =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIB1iLbeqDzK4CDeUC3t+ffVPDI9Gk+sBwIZqJZW1NfS5 device-a"
    static let keyB =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDMdwe233CUbxjpEHkissIUGdCxhkTsDE/Zg7f+LB6S+ device-b"

    /// Derived from the key and therefore the same every time, which is the
    /// whole point of it: a device re-running the ceremony against a runner it
    /// is already on has to land on the id already in that fence, or the
    /// daemon's "already enrolled" arm — which compares client ids — sees a new
    /// device and the runner gains a second line for one Mac. The Mac used to
    /// mint a `UUID()` here, which is that failure once per run.
    @Test func theClientIDIsDerivedFromTheKeyAndDoesNotChange() throws {
        let first = try #require(DeviceKey.clientID(of: Self.keyA))
        let second = try #require(DeviceKey.clientID(of: Self.keyA))

        #expect(first == second)
        // The format is Rust's, asserted in `ceremony.rs` too. Named here
        // because a Swift-side "improvement" to it would be a Mac spelling one
        // device differently from the phone that added it.
        #expect(first.hasPrefix("farcooler-"))
    }

    /// **The rule that is invisible by looking.** A Mac is TWO enrolled lines
    /// and ONE client, so Key B's line carries KEY A's id — that is what lets a
    /// single `client revoke` take both of them, and what makes the removal copy
    /// about ssh, git and Zed access true.
    ///
    /// Key B derives to a DIFFERENT id, asserted first so that deriving each
    /// line's id from its own key cannot look equivalent to this. It would
    /// silently split one Mac into two clients, and nothing about the enrollment
    /// would fail at the time.
    @Test func bothOfAMacsLinesCarryKeyAsClientID() throws {
        let idA = try #require(DeviceKey.clientID(of: Self.keyA))
        let idB = try #require(DeviceKey.clientID(of: Self.keyB))
        #expect(idA != idB)

        // The two command lines `Enrollment.enroll` builds for one Mac: the
        // restricted line for Key A, the plain one for Key B, one id.
        let restricted = Enrollment.arguments(
            key: Self.keyA, label: "MacBook Air", clientID: idA, scope: "control", shell: false,
            nodeKey: "", runner: "e-liang@cosmo")
        let plain = Enrollment.arguments(
            key: Self.keyB, label: "MacBook Air", clientID: idA, scope: "host_admin", shell: true,
            nodeKey: "", runner: "e-liang@cosmo")

        #expect(clientID(in: restricted) == idA)
        #expect(clientID(in: plain) == idA)
        #expect(clientID(in: plain) != idB)
    }

    /// Text that is not a public key has no id, and the answer is nil — never a
    /// substitute. A `UUID()` fallback here is the bug this replaced, wearing a
    /// disguise: it would enroll a line under an id no `client revoke` can name,
    /// and report success.
    @Test func textThatIsNotAKeyHasNoClientID() {
        #expect(DeviceKey.clientID(of: "ssh-ed25519 AAAAC3Nz") == nil)
        #expect(DeviceKey.clientID(of: "") == nil)
    }

    /// The value `--client-id` carries, or nil when the flag is not there at
    /// all — which is itself a failure worth seeing rather than a crash.
    private func clientID(in arguments: [String]) -> String? {
        guard let flag = arguments.firstIndex(of: "--client-id"), flag + 1 < arguments.count
        else { return nil }
        return arguments[flag + 1]
    }
}
