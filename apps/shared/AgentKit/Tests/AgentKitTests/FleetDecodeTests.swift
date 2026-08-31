import Foundation
import Testing

@testable import AgentKit

/// Every key the client core puts on a fleet, decoded into the types the phone
/// actually renders.
///
/// The fleet payload is built in one place — the `json!` block in
/// `Session::fleet`, `crates/client/src/session.rs:254-375` — and `Connection`
/// decodes it with a plain `JSONDecoder` (`Connection.swift:584`). Those two
/// facts are why `is_main_checkout` could be wrong for as long as the phone had
/// it: `JSONDecoder` ignores keys nobody declared and leaves an OPTIONAL whose
/// key never arrived as nil. Not a throw, not a warning, not a log line — a
/// workspace that answered "no" to "are you the primary checkout?" and got
/// offered a Remove Worktree button it must never be offered. See `07e75e8`.
///
/// So the spelling is what is tested. The JSON below is transcribed key-for-key
/// from that `json!` block, and every assertion is that a value put on the wire
/// came out the other side under the name this app reads it by. A field renamed
/// on either end fails here rather than going quiet on a phone. Android has had
/// exactly this test since `231f81a` (`FleetDecodeTest.kt`); this is the phone's,
/// and it is the reason `CoreModel.swift` moved into AgentKit at all — the iOS
/// target's only test bundle is a UI-testing one, which cannot `@testable
/// import` the app module, so nothing on that side could reach these types.
///
/// Values are deliberately non-default. `false` where the default is `false`,
/// or `0` where a missing number would read as `0`, is a test that passes when
/// the decode does nothing at all.
///
/// A NON-optional field misspelled here fails loudly instead — the decode
/// throws and takes the fixture with it. That half of the bug class was never
/// the dangerous one; it is the optionals that go quiet, and every field added
/// to these types after the first release is optional on purpose.
struct FleetDecodeTests {
    /// Transcribed from `Session::fleet`. Key order follows the `json!` block so
    /// the two can be read side by side.
    static let fleetJSON = """
    {
      "runtime_healthy": true,
      "live_panes": 4,
      "workspaces": [
        {
          "id": "8f14e45f-ce5b-4a5e-9c2b-000000000001",
          "short": "8f14e4",
          "repository": "1c383cd3-0b0f-4a63-b8a1-000000000002",
          "task": "Widen the model",
          "branch": "widen-the-model",
          "worktree": "/Users/e/src/overnight-widen",
          "state": "worktree_missing",
          "isMainCheckout": true,
          "terminals": [
            {
              "id": "aab3238922bcc25a6f606eb525ffdc56",
              "short": "aab323",
              "title": "Fix the parser",
              "preset": "claude",
              "state": "exited",
              "activity": "done",
              "activitySince": 1755900000000,
              "exitCode": 101,
              "exitSignal": 9,
              "turnStartedAt": 1755899000000,
              "blockedQuestion": "Run `rm -rf build`?",
              "feed": ["Reading watch.rs.", "Rewrote the poller.", "Ran the suite."],
              "said": "Rewrote the poller so the filesystem says when something moved.",
              "subagents": ["explore", "plan"],
              "glyph": "✓",
              "headline": "Done · claude",
              "line": "3/7 · Designing test matrix",
              "rank": 199999940,
              "planDone": 3,
              "planTotal": 7,
              "turnFailed": true,
              "epoch": 12,
              "paneMode": "changes",
              "chatCapable": true,
              "agentSessionId": "01J8Z2",
              "agentMode": "plan",
              "availableAgentModes": ["plan", "edit"]
            }
          ]
        }
      ]
    }
    """

    static func decodeFleet(_ json: String = fleetJSON) throws -> Fleet {
        try JSONDecoder().decode(Fleet.self, from: Data(json.utf8))
    }

    private func terminal() throws -> Terminal {
        try #require(Self.decodeFleet().workspaces.first?.terminals.first)
    }

    /// The two snake_case keys, which are the ONLY two on this payload.
    ///
    /// `Fleet`'s `CodingKeys` is the one mapping in `CoreModel.swift`, and its
    /// existence beside twenty-odd camelCase properties is what made
    /// `is_main_checkout` look like it belonged. It does not: the outer object
    /// is snake_case and everything inside it is camelCase, because
    /// `Session::fleet` writes them that way.
    @Test func theFleetEnvelopeIsSnakeCaseAndOnlyTheEnvelope() throws {
        let fleet = try Self.decodeFleet()
        #expect(fleet.runtimeHealthy)
        #expect(fleet.livePanes == 4)
        #expect(fleet.workspaces.count == 1)
    }

    @Test func everyWorkspaceFieldOnTheWireLandsOnTheModel() throws {
        let workspace = try #require(Self.decodeFleet().workspaces.first)
        #expect(workspace.id == "8f14e45f-ce5b-4a5e-9c2b-000000000001")
        #expect(workspace.short == "8f14e4")
        // A UUID, not a name. The CLI sends a display name under this key and
        // the Mac reads it as one; see `repository`'s own doc comment.
        #expect(workspace.repository == "1c383cd3-0b0f-4a63-b8a1-000000000002")
        #expect(workspace.task == "Widen the model")
        #expect(workspace.branch == "widen-the-model")
        #expect(workspace.worktree == "/Users/e/src/overnight-widen")
        #expect(workspace.state == "worktree_missing")
        #expect(workspace.worktreeMissing)
        #expect(!workspace.isHidden)
        #expect(workspace.terminals.count == 1)
    }

    /// The bug `07e75e8` fixed, standing as a test.
    ///
    /// `isMainCheckout` and not `is_main_checkout`. Respell the property the
    /// Mac's way and this fails: the key stops matching, the optional is nil,
    /// and `isPrimaryCheckout` answers false for the one worktree the phone must
    /// never offer to remove.
    @Test func theRepositorysOwnCheckoutSaysSo() throws {
        let workspace = try #require(Self.decodeFleet().workspaces.first)
        #expect(workspace.isMainCheckout == true)
        #expect(workspace.isPrimaryCheckout)
    }

    @Test func everyTerminalFieldOnTheWireLandsOnTheModel() throws {
        let terminal = try terminal()
        #expect(terminal.id == "aab3238922bcc25a6f606eb525ffdc56")
        #expect(terminal.short == "aab323")
        #expect(terminal.title == "Fix the parser")
        #expect(terminal.preset == "claude")
        #expect(terminal.state == "exited")
        #expect(terminal.activity == "done")
        #expect(terminal.activitySince == 1_755_900_000_000)
        #expect(terminal.exitCode == 101)
        #expect(terminal.exitSignal == 9)
        #expect(terminal.turnStartedAt == 1_755_899_000_000)
        #expect(terminal.blockedQuestion == "Run `rm -rf build`?")
        #expect(terminal.feed == ["Reading watch.rs.", "Rewrote the poller.", "Ran the suite."])
        #expect(terminal.said == "Rewrote the poller so the filesystem says when something moved.")
        #expect(terminal.subagents == ["explore", "plan"])
        #expect(terminal.glyph == "✓")
        #expect(terminal.headline == "Done · claude")
        #expect(terminal.line == "3/7 · Designing test matrix")
        #expect(terminal.rank == 199_999_940)
        #expect(terminal.planDone == 3)
        #expect(terminal.planTotal == 7)
        #expect(terminal.turnFailed == true)
        #expect(terminal.epoch == 12)
        #expect(terminal.paneMode == "changes")
        #expect(terminal.chatCapable == true)
        #expect(terminal.agentSessionId == "01J8Z2")
        #expect(terminal.agentMode == "plan")
        #expect(terminal.availableAgentModes == ["plan", "edit"])
    }

    /// The derivations the two screens actually draw, off the decoded fields.
    ///
    /// Here because a field that decodes and is then read by nothing is only
    /// half-wired — which is the quieter half of this bug class, and what
    /// `Branch.updatedAt` had been for as long as the branch picker existed.
    @Test func theDerivationsReadTheFieldsTheyClaimTo() throws {
        let terminal = try terminal()
        #expect(terminal.agent == .done)
        #expect(terminal.turnDidFail)
        #expect(terminal.activityLabel == "Failed")
        #expect(terminal.runDidFail)
        #expect(terminal.isChangesPane)
        #expect(!terminal.isAgentPane)
        #expect(terminal.canSwitchPaneMode)
        #expect(terminal.recentSteps == ["Reading watch.rs.", "Rewrote the poller.", "Ran the suite."])
        #expect(terminal.lastSaid == "Rewrote the poller so the filesystem says when something moved.")
        #expect(terminal.runningSubagents == ["explore", "plan"])
        #expect(terminal.signalLine == "3/7 · Designing test matrix")
        #expect(terminal.sortRank == 199_999_940)
        // A named conversation beats the preset, and needs no ordinal.
        #expect(terminal.label == "Fix the parser")
        #expect(terminal.displayName(ordinal: 2) == "Fix the parser")
    }

    /// A daemon old enough to send none of the optional keys.
    ///
    /// The reason every field added after the first release is optional rather
    /// than defaulted: Swift's synthesized `Decodable` throws on a missing key
    /// regardless of any default, so one absent field would fail the decode of
    /// the WHOLE fleet and show "no workspaces" for a runner full of them.
    @Test func anOlderDaemonSendingOnlyTheOriginalKeysStillDecodes() throws {
        let fleet = try Self.decodeFleet("""
        {
          "runtime_healthy": false,
          "live_panes": 0,
          "workspaces": [
            {
              "id": "w",
              "short": "w",
              "task": "t",
              "branch": "b",
              "state": "active",
              "terminals": [
                { "id": "t", "short": "t", "title": "", "preset": "zsh",
                  "state": "running", "epoch": 1 }
              ]
            }
          ]
        }
        """)
        let workspace = try #require(fleet.workspaces.first)
        #expect(workspace.repository == nil)
        #expect(workspace.worktree == nil)
        #expect(workspace.isMainCheckout == nil)
        // Absent reads as "not the primary checkout", which is the direction
        // that OFFERS the removal — safe only because the daemon refuses it
        // independently. See `isPrimaryCheckout`.
        #expect(!workspace.isPrimaryCheckout)

        let terminal = try #require(workspace.terminals.first)
        #expect(terminal.agent == .none)
        #expect(terminal.label == "shell")
        // Absent means "nobody said", never "it exited cleanly".
        #expect(!terminal.runDidFail)
        #expect(terminal.sortRank == UInt32.max)
        #expect(terminal.recentSteps.isEmpty)
        #expect(terminal.lastSaid == nil)
    }

    /// A daemon NEWER than this app, sending a key nothing here declares.
    ///
    /// `JSONDecoder` ignores it, which is the behavior that makes this whole
    /// file necessary and is also the behavior we want: a phone that refused to
    /// draw a fleet because the runner had learned a new word would be worse
    /// than one that draws it without the new word.
    /// The trace's two keys, which `Session::fleet` does NOT send yet.
    ///
    /// **Deliberately not in `fleetJSON`.** That fixture's contract is that it
    /// is transcribed from the producer, key for key, and adding a key the
    /// producer does not write would make the one payload in this file that
    /// claims to be real stop being real. So the shape is asserted on its own
    /// payload, and the absence is asserted on the real one.
    ///
    /// What this holds down is the SPELLING and the ENCODING, which are the two
    /// ways this can go quiet: `Data` decodes from base64, which is what JSON
    /// has for bytes, and a key that does not match leaves an optional nil with
    /// no error anywhere. When the trace is projected into `Session::fleet` —
    /// one line beside `"planDone"` — this is the test that says the two ends
    /// agree.
    @Test func theTraceDecodesFromBase64UnderTheseTwoKeys() throws {
        // A version-1, five-minute-bucket trace whose thirteenth code bucket is
        // 1 and whose rest is zero: `0x10`, then 24 zero bytes, `01 00`, then
        // 39 zeroes. Sixty-six bytes exactly, base64 on the wire.
        var wire = Data([0x10])
        wire.append(contentsOf: Array(repeating: UInt8(0), count: 24))
        wire.append(contentsOf: [0x01, 0x00])
        wire.append(contentsOf: Array(repeating: UInt8(0), count: 39))
        let base64 = wire.base64EncodedString()

        let json = #"""
            {
              "runtime_healthy": true,
              "live_panes": 1,
              "fleetTrace": "BASE64",
              "workspaces": [
                {
                  "id": "w", "short": "w", "task": "t", "branch": "b",
                  "state": "ready",
                  "terminals": [
                    {
                      "id": "t1", "short": "t1", "title": "claude",
                      "preset": "claude", "state": "running", "epoch": 1,
                      "activityTrace": "BASE64"
                    }
                  ]
                }
              ]
            }
            """#.replacingOccurrences(of: "BASE64", with: base64)

        let fleet = try Self.decodeFleet(json)
        #expect(fleet.fleetTrace == wire)
        let terminal = try #require(fleet.workspaces.first?.terminals.first)
        #expect(terminal.activityTrace == wire)
        // And it is a trace this build will draw, rather than 66 bytes that
        // happen to survive the trip.
        let trace = try #require(ActivityTrace(terminal.activityTrace))
        #expect(trace.span == .hour)
        #expect(trace.code(12) == 1)
        #expect(trace.tallestOutput == 0)
    }

    /// And the producer as it stands today: no key, so no trace, so no drawing.
    ///
    /// Not a placeholder assertion. `Session::fleet` has no line for either
    /// field, so every Apple surface draws no trace on every runner right now —
    /// and the thing that must NOT happen is that a missing key decodes into
    /// sixty-six zeroes and every row grows a flat line at zero.
    /// A fleet with no trace keys at all decodes as ABSENT, not as a zeroed
    /// row. Older runners send nothing here, and a flat zero trace and "this
    /// pane has no history" are different claims — only one of them is true of
    /// a runner that has never heard of the field.
    ///
    /// Named for what it guards rather than for the state of the producer: it
    /// was written when `Session::fleet` sent no trace at all, and that gap is
    /// since closed.
    @Test func aFleetWithNoTraceKeysDecodesAsAbsent() throws {
        let fleet = try Self.decodeFleet()
        #expect(fleet.fleetTrace == nil)
        let terminal = try #require(fleet.workspaces.first?.terminals.first)
        #expect(terminal.activityTrace == nil)
        #expect(ActivityTrace(terminal.activityTrace) == nil)
    }

    @Test func aNewerDaemonSendingAnUnknownKeyStillDecodes() throws {
        let fleet = try Self.decodeFleet("""
        {
          "runtime_healthy": true,
          "live_panes": 1,
          "workspaces": [
            {
              "id": "w", "short": "w", "task": "t", "branch": "b",
              "state": "active", "somethingNewer": 42,
              "terminals": [
                { "id": "t", "short": "t", "title": "", "preset": "claude",
                  "state": "running", "epoch": 1, "alsoNewer": ["x"] }
              ]
            }
          ]
        }
        """)
        #expect(fleet.workspaces.first?.terminals.first?.preset == "claude")
    }
}

/// The branch list, whose one field this app had never declared.
///
/// Built by `Session::branches`, `crates/client/src/session.rs:1074-1090`.
/// Small enough to transcribe whole, and worth transcribing because `updatedAt`
/// is the exact shape of the trap this file exists for: it has been on the wire
/// since the call existed, the phone declared no property for it, and nothing
/// anywhere said so.
struct BranchDecodeTests {
    static let branchesJSON = """
    {
      "branches": [
        {
          "name": "feat/widen-the-model",
          "local": true,
          "remote": "origin",
          "checkedOut": false,
          "subject": "Widen the model",
          "updatedAt": 1755900000000
        },
        {
          "name": "colleagues-work",
          "local": false,
          "remote": "origin",
          "checkedOut": false,
          "subject": "Handed over",
          "updatedAt": null
        }
      ]
    }
    """

    private func branches() throws -> [Branch] {
        try JSONDecoder().decode(BranchList.self, from: Data(Self.branchesJSON.utf8)).branches
    }

    @Test func everyBranchFieldOnTheWireLandsOnTheModel() throws {
        let branch = try #require(branches().first)
        #expect(branch.name == "feat/widen-the-model")
        #expect(branch.local)
        #expect(branch.remote == "origin")
        #expect(!branch.checkedOut)
        #expect(branch.subject == "Widen the model")
        #expect(branch.updatedAt == 1_755_900_000_000)
        #expect(!branch.isRemoteOnly)
        #expect(try #require(branches().last).isRemoteOnly)
    }

    /// The unit, which is the whole difference between the two producers.
    ///
    /// `Session::branches` sends `t.seconds * 1000`; the CLI sends `t.seconds`,
    /// and the Mac's `BranchInfo.age` subtracts it from `timeIntervalSince1970`
    /// with no scaling. Delete the `/ 1000` in `Branch.age(at:)` — which is
    /// exactly what copying the Mac's line would do — and this branch dates to
    /// 1970 and reads as twenty thousand days old.
    @Test func ageReadsTheTimestampAsMilliseconds() throws {
        let branch = try #require(branches().first)
        let tip = Date(timeIntervalSince1970: 1_755_900_000)
        #expect(branch.age(at: tip.addingTimeInterval(3 * 3600)) == "3h")
        #expect(branch.age(at: tip.addingTimeInterval(90 * 60)) == "1h")
        #expect(branch.age(at: tip.addingTimeInterval(4 * 86_400)) == "4d")
        // Under a minute still says a minute rather than "0m", same as the Mac.
        #expect(branch.age(at: tip.addingTimeInterval(20)) == "1m")
    }

    /// Nothing to say beats saying nothing well.
    @Test func aBranchWithNoCommitterDateHasNoAge() throws {
        let branch = try #require(branches().last)
        #expect(branch.updatedAt == nil)
        #expect(branch.age(at: Date()).isEmpty)
    }

    /// A phone whose clock runs behind the runner's does not print a negative
    /// age. The same guard the fleet's two clocks needed, for the same reason.
    @Test func aTipDatedInTheFutureReadsAsNow() throws {
        let branch = try #require(branches().first)
        let tip = Date(timeIntervalSince1970: 1_755_900_000)
        #expect(branch.age(at: tip.addingTimeInterval(-60)) == "now")
    }
}
