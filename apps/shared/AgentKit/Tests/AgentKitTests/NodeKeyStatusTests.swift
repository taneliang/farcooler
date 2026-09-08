import Foundation
import Testing

@testable import AgentKit

/// The rules behind the one line a phone with no tunnel key now shows.
///
/// Every one of these guards something that was, until this file existed,
/// expressible nowhere a machine could read it. The iOS target has no unit-test
/// target and CI's iOS job ends at `build-for-testing`, so the sentences, the
/// word that chooses between them, and the decision to show nothing at all when
/// the key is fine would each have been a claim made only by a `View` body.
struct NodeKeyStatusTests {

    // MARK: - Reading the mint's answer

    /// The shape `farcooler_client_mint_node_key` answers on a build that can
    /// mint. Both halves, both non-empty, and nothing is refused.
    @Test func aPairComesBackAsAPair() {
        #expect(
            NodeKeyMint.read(["private_key": "aaa", "public_key": "bbb"])
                == .pair(privateKey: "aaa", publicKey: "bbb"))
    }

    /// The reason that used to be thrown away. `{"error":"no_tailcat"}` is what
    /// every build with no Go archive answers — `crates/tailcat/src/stub.rs`
    /// and `helper.rs` both return `NoTailcatLinked` — and it is the answer the
    /// owner's phone was giving into a `nil` nobody could see.
    @Test(arguments: [
        ("no_tailcat", RunnerTrouble.TunnelWord.notInThisBuild),
        ("io", RunnerTrouble.TunnelWord.unspecified),
        ("derp", RunnerTrouble.TunnelWord.rendezvous),
        ("no_answer", RunnerTrouble.TunnelWord.noAnswer),
    ])
    func anErrorComesBackAsItsWord(word: String, kind: RunnerTrouble.TunnelWord) {
        #expect(NodeKeyMint.read(["error": word]) == .refused(kind))
    }

    /// Every way the answer can fail to be a usable pair without carrying a
    /// word: no answer at all (a zero-length return, a return longer than the
    /// buffer, bytes that are not JSON), an object with neither shape in it, a
    /// half that is missing, and a half that is empty.
    ///
    /// The last two matter for the reason `crates/client/src/ffi.rs` refuses a
    /// public half its own fence would not write: an offer carrying an empty or
    /// half-empty node key looks filled in and grants a tunnel that admits
    /// nobody, and tailcat ignores an unrecognized client in silence.
    ///
    /// All of them land on `unspecified` rather than on a fifth word, which is
    /// the same call `linked.rs` makes for `EINVAL` and `ERANGE`.
    ///
    /// A plain loop rather than `@Test(arguments:)`, because `[String: Any]` is
    /// not `Sendable` and a parameterized case cannot take one.
    @Test func anythingElseIsRefusedWithoutInventingAWord() {
        let answers: [[String: Any]?] = [
            nil,
            [:],
            ["public_key": "bbb"],
            ["private_key": "aaa"],
            ["private_key": "", "public_key": "bbb"],
            ["private_key": "aaa", "public_key": ""],
            ["error": "a_word_this_build_has_never_heard_of"],
            ["error": ""],
        ]
        for answer in answers {
            #expect(NodeKeyMint.read(answer) == .refused(.unspecified), "\(answer ?? [:])")
        }
    }

    // MARK: - The three answers

    /// A mint that refused never reaches the key store. There is nothing to
    /// keep, and deleting and re-adding a Keychain item every time a settings
    /// screen opens would be work done for a device that can never benefit.
    @Test func aRefusedMintIsNotStored() {
        var asked = 0
        let status = NodeKeyStatus.after(.refused(.notInThisBuild)) { _, _ in
            asked += 1
            return 0
        }
        #expect(status == .notMinted(.notInThisBuild))
        #expect(asked == 0)
    }

    /// A pair that the key store kept is a device with a tunnel key, and it is
    /// stored exactly once.
    @Test func aPairTheKeyStoreKeptIsHeld() {
        var asked = 0
        let status = NodeKeyStatus.after(.pair(privateKey: "aaa", publicKey: "bbb")) { _, _ in
            asked += 1
            return 0
        }
        #expect(status == .held)
        #expect(asked == 1)
    }

    /// A pair the key store refused carries the refusal, and does NOT read as
    /// a device that has a key.
    ///
    /// `-25299` is `errSecDuplicateItem`; the value is immaterial and the fact
    /// that it survives is not. It is the only thing that makes a Keychain
    /// refusal searchable, and `TunnelE2EHarness` — which records it under
    /// `nodeKeychainWriteStatus` today — is `#if DEBUG` and unreachable from
    /// the TestFlight build the owner runs.
    @Test func aPairTheKeyStoreRefusedCarriesTheRefusal() {
        let status = NodeKeyStatus.after(.pair(privateKey: "aaa", publicKey: "bbb")) { _, _ in
            -25299
        }
        #expect(status == .notStored(status: -25299))
    }

    // MARK: - What a person is told

    /// A device whose key is fine says nothing. This is the decision about
    /// whether the notice appears at all, and it is here rather than in the
    /// settings screen so that it is a value a test can read back.
    @Test func aDeviceThatHasAKeySaysNothing() {
        #expect(NodeKeyStatus.held.sentence == nil)
    }

    /// Every other state says something. A status with no sentence would be a
    /// fault that is once again invisible, which is the entire defect this file
    /// was written for.
    @Test(arguments: [
        NodeKeyStatus.notMinted(.notInThisBuild),
        NodeKeyStatus.notMinted(.unspecified),
        NodeKeyStatus.notMinted(.rendezvous),
        NodeKeyStatus.notMinted(.noAnswer),
        NodeKeyStatus.notStored(status: -25299),
    ])
    func everyOtherStateSaysSomething(status: NodeKeyStatus) {
        let sentence = status.sentence
        #expect(sentence?.isEmpty == false)
    }

    /// The fact and its consequence, which is what the whole notice is for: a
    /// runner added by this phone is reached by address rather than through the
    /// tunnel, and until now nothing anywhere said so.
    @Test(arguments: [
        NodeKeyStatus.notMinted(.notInThisBuild),
        NodeKeyStatus.notMinted(.unspecified),
        NodeKeyStatus.notMinted(.rendezvous),
        NodeKeyStatus.notMinted(.noAnswer),
    ])
    func aMintThatNeverHappenedNamesTheConsequence(status: NodeKeyStatus) {
        let sentence = status.sentence ?? ""
        #expect(sentence.contains("no tunnel key"))
        #expect(sentence.contains("reached by their address"))
    }

    /// The cause is carried, not dropped. A build with no tunnel in it says so;
    /// anything else declines to guess.
    ///
    /// `derp` and `no_answer` share the generic on purpose. Minting reaches no
    /// network — `linked.rs` says so where it refuses to borrow their words for
    /// a buffer error — so a sentence about a rendezvous service could never be
    /// shown, and copy nothing can display is the trap `RunnerRefusal`'s header
    /// names.
    @Test func theWordChoosesTheCause() {
        #expect(
            NodeKeyStatus.notMinted(.notInThisBuild).sentence?.contains(
                "doesn’t include the tunnel") == true)
        for word in [
            RunnerTrouble.TunnelWord.unspecified, .rendezvous, .noAnswer,
        ] {
            #expect(
                NodeKeyStatus.notMinted(word).sentence?.contains(
                    "couldn’t make one on this device") == true,
                "\(word)")
        }
    }

    /// A key that was made and could not be saved is a DIFFERENT fault, and has
    /// to read as one.
    ///
    /// `mintIfNeeded` hands the pair back regardless of the write, so the offer
    /// does carry a public half, the runner IS granted a tunnel, and what
    /// breaks is the later dial rather than the pairing. Saying "this device
    /// has no tunnel key" here would send whoever is diagnosing it to the
    /// tunnel archive, which is not where the fault is.
    @Test func aKeyThatCouldNotBeSavedReadsAsItsOwnFault() {
        let sentence = NodeKeyStatus.notStored(status: -25299).sentence ?? ""
        #expect(sentence.contains("couldn’t save it"))
        #expect(sentence.contains("through the tunnel won’t connect"))
        #expect(!sentence.contains("has no tunnel key"))
        // The one number a person is given, because it is the only thing that
        // makes a Keychain refusal searchable and `TunnelE2EHarness`, which
        // prints it today, is `#if DEBUG` and unreachable from TestFlight.
        #expect(sentence.contains("-25299"))
    }

    // MARK: - What must never reach a screen

    /// **The word crosses the FFI; the app owns the sentence.** No machine word
    /// and no Rust error string is ever printed. `TunnelError`'s own `Display`
    /// text — "this build has no tunnel it can reach" — is written for a daemon
    /// log, and these sentences exist to keep it off a phone.
    @Test(arguments: [
        NodeKeyStatus.notMinted(.notInThisBuild),
        NodeKeyStatus.notMinted(.unspecified),
        NodeKeyStatus.notMinted(.rendezvous),
        NodeKeyStatus.notMinted(.noAnswer),
        NodeKeyStatus.notStored(status: -25299),
    ])
    func noMachineWordReachesAScreen(status: NodeKeyStatus) {
        let sentence = status.sentence ?? ""
        for word in RunnerTrouble.TunnelWord.allCases {
            #expect(!sentence.contains(word.rawValue), "\(word.rawValue) in: \(sentence)")
        }
        #expect(!sentence.contains("tailcat"))
        #expect(!sentence.contains("node key"))
    }

    /// **Never print, log or display key material.** `TunnelE2EHarness` prints
    /// the private half's LENGTH and never the half itself, deliberately, and
    /// nothing on this path may be looser than the debug tool is.
    ///
    /// Driven end to end — a mint answer with both halves in it, through
    /// ``NodeKeyStatus/after(_:store:)``, to the sentence — rather than
    /// asserted about a constant, because the way this rule would actually be
    /// broken is somebody putting the public half into the ``notStored``
    /// sentence to make it easier to diagnose. That is the branch a pair
    /// reaches, and it is the branch this walks.
    @Test func noKeyMaterialCanReachASentence() {
        let secret = "PRIVATEHALF-must-never-be-shown"
        let offered = "PUBLICHALF-not-on-a-screen-either"
        let mint = NodeKeyMint.read(["private_key": secret, "public_key": offered])
        #expect(mint == .pair(privateKey: secret, publicKey: offered))

        // The store gets both halves, which is the whole of where they may go.
        var handed: [String] = []
        let kept = NodeKeyStatus.after(mint) { priv, pub in
            handed = [priv, pub]
            return 0
        }
        #expect(handed == [secret, offered])
        // A device that holds a key says nothing at all.
        #expect(kept.sentence == nil)

        // And when the store refuses, the sentence it produces carries the
        // refusal and neither half.
        let refused = NodeKeyStatus.after(mint) { _, _ in -25299 }
        let sentence = refused.sentence ?? ""
        #expect(sentence.contains("-25299"))
        #expect(!sentence.contains(secret))
        #expect(!sentence.contains(offered))
    }
}
