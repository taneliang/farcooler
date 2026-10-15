import Foundation

/// Putting a device's key into a runner's `~/.ssh/authorized_keys`.
///
/// **The daemon owns the write**, on every runner including this Mac. Not a
/// shell command appending a line: that file is the one whose corruption costs
/// somebody SSH access to their own machine, so the write is
/// descriptor-anchored, `O_NOFOLLOW`, locked, atomic, `fsync`ed twice and
/// backed up — `crates/daemon/src/enrollment.rs` over `crates/fence/src/lib.rs`.
/// The app's part is to ask.
///
/// ## Two surfaces, on purpose
///
/// The daemon serves `client.enroll`, `client.list` and `client.revoke`. A phone
/// reaches them through `farcooler_client_call` in `crates/client/src/ffi.rs`; a
/// Mac reaches them through `farcooler client` (`crates/cli/src/clients.rs`).
/// Each route is right for its side and wrong for the other: the CLI runs real
/// `ssh`, so it inherits the agent, the passphrase prompt, `ProxyJump` and
/// everything else a person has already set up — and a phone has no `ssh` at all.
///
/// What this file builds, exactly:
///
/// ```
/// farcooler --json --runner you@box client enroll --key <public key> \
///     --label "iPhone 17" --client-id <uuid> --scope control --node-key <43 chars>
/// farcooler --json --runner you@box client enroll --key <public key> \
///     --label "MacBook Air" --client-id <uuid> --scope host_admin --shell-access
/// ```
///
/// `--shell-access` is a flag with no value, so its ABSENCE is the restricted
/// line — which is what every caller that predates Key B means. It maps to
/// `ClientEnroll.shell_access`; the daemon refuses it with any scope but
/// `host_admin`, and that refusal is the daemon's, not this app's.
///
/// A runner that could not be written to is reported in a sentence and the
/// ceremony still shows the reply — because the reply is what the new device
/// needs, and a runner that was asleep is an ordinary outcome the manifest
/// already carries as `pending`.
///
/// **Which is why this answers with an ``Outcome`` and not a sentence.** It
/// used to answer with the transcript alone, so the only record of which
/// runners took the key was prose meant for a human, and the records handed to
/// the new device were built somewhere else entirely — with `pending: false`
/// hardcoded on every one of them. A Mac that failed to write told the new
/// device that runner was ready, and the new device found out by failing to
/// connect with nothing on screen to explain it.
///
/// ## A Mac is TWO enrollments
///
/// Key A's line is `restrict,command="~/.local/bin/farcoolerd… --stdio …"`, the
/// channel's own daemon named by path rather than left to a login shell's
/// `PATH`. Key B's line is
/// plain, and that is the whole point of it: a forced command means sshd runs
/// that program and only that program, so a Key B carrying one could not open a
/// shell and Zed would still be locked out.
///
/// `ClientEnroll` now chooses between those two SHAPES — `shellAccess`, which is
/// `--shell-access` on the command line below — and the message still carries no
/// field for options, a forced command, or a key written as it arrived. That
/// absence is the guard rail, and it survives: the daemon renders both lines from
/// the key's own key data with a comment it chose.
///
/// Both calls send the SAME client id, which is what lets one `client revoke`
/// remove both of a Mac's lines in one write — and what makes the removal copy
/// ("this takes that Mac's ssh, git and Zed access away too") true rather than a
/// hope. Key B goes with `--scope host_admin`: an unrestricted line on an account
/// is a shell, a shell is every power that account has, and the daemon refuses
/// the pair otherwise.
enum Enrollment {
    /// What the writes actually did.
    ///
    /// Two answers, and they are not the same answer. The transcript is for the
    /// person reading the screen; ``written`` is what the reply's `pending`
    /// flags are computed from, and it is the one the new device acts on
    /// forever after. Guessing the second from the first — or, as this used to
    /// do, not carrying it at all — is how a runner nobody could reach was
    /// announced as ready.
    struct Outcome: Sendable, Equatable {
        /// The runner ids whose Key A line is in the file.
        ///
        /// **Key A, specifically.** That is the restricted line carrying the
        /// forced command, so it is the key the new device connects to Far
        /// Cooler with — which is exactly what `pending` is a statement about.
        /// Key B is Zed, Git and Terminal on a Mac; a runner that took Key A
        /// and refused Key B is reachable, and calling it pending would tell
        /// the new device to ignore a runner it can talk to. That failure gets
        /// ``shellRefused`` and its own sentence instead.
        var written: Set<String> = []
        /// The runner ids that took Key A and refused Key B. Empty for a phone,
        /// which has no second key.
        var shellRefused: Set<String> = []
        /// The runners this enrollment decided to offer through the tunnel, and
        /// the token each one answered with.
        ///
        /// The DECISION, not the raw answer: only a runner
        /// ``Enrollment/reach(granting:token:)`` said yes about appears here,
        /// so nothing downstream re-decides it from a token lying around.
        ///
        /// Since ov-485 that is every runner that answered with a token: the
        /// tunnel first, with the address kept beside it as the alternative.
        /// Empty means no runner could give the new device a tunnel — it named
        /// no node key, or every runner predates the tunnel — and the fleet
        /// pairs at its addresses, as it always did.
        var tunneled: [String: String] = [:]
        /// The CLI's own words, for the runners something went wrong on. Nil
        /// when every write landed.
        var transcript: String?

        /// Nothing was written, and the caller already has the sentence for it.
        static func nothingWritten(_ transcript: String) -> Outcome {
            Outcome(written: [], shellRefused: [], transcript: transcript)
        }

        /// `runners`, each carrying what this outcome says about it.
        ///
        /// The one place `pending` is decided on this Mac. `pending` means "this
        /// runner does not have the new device's key", so it is the ABSENCE of
        /// a successful write and never a default — a runner that failed, was
        /// unreachable, or was never attempted at all is missing from
        /// ``written`` and travels pending, which is a true statement about
        /// that runner's `authorized_keys` in all three cases.
        ///
        /// Nothing here retries any of them, and nothing anywhere else does
        /// either. Pending is the end state.
        func granting(_ runners: [CeremonyRunner]) -> [CeremonyRunner] {
            runners.map { runner in
                var runner = runner
                runner.pending = !written.contains(runner.id)
                // The tunnel is the route, and the address moves beside it as
                // the alternative (ov-485). Still one route at a time: the new
                // device dials `reach`, and only a person answering "Connect
                // directly to … instead?" ever moves it to `alternative`. A
                // runner not in ``tunneled`` keeps the address it arrived with.
                if let token = tunneled[runner.id] {
                    if case .direct = runner.reach { runner.alternative = runner.reach }
                    runner.reach = .tailcat(token: token)
                }
                return runner
            }
        }
    }

    /// One `client enroll`, as the thing that runs it.
    ///
    /// Injected so that ``enroll(keyA:keyB:label:clientID:scope:nodeKey:on:using:)`` —
    /// which is where every decision about what got written is made — can be
    /// tested without a runner to write to. The default is the real CLI, and no
    /// caller in the app passes anything else.
    typealias Writer = @Sendable ([String]) async -> (ok: Bool, output: String)

    /// Enroll a device's keys on each runner, and answer with what landed.
    ///
    /// `keyB` is a Mac's shell key and nil for everything else — there is no Zed
    /// on a phone. Its scope is not a parameter because it cannot vary.
    ///
    /// `nodeKey` is the new device's tailcat node public key, exactly as its
    /// offer carried it, and empty for a device that has none. It goes on Key
    /// A's call and never on Key B's — see
    /// ``arguments(key:label:clientID:scope:shell:nodeKey:runner:)``. An empty
    /// one changes nothing about this enrollment: no runner is asked to join a
    /// tunnel, and the pairing is the direct one it has always been.
    ///
    /// One runner at a time rather than concurrently: these are writes to the
    /// same kind of file on different machines, and a transcript that interleaves
    /// is a transcript nobody can read. The list is short.
    static func enroll(
        keyA: String, keyB: String?, label: String, clientID: String, scope: String,
        nodeKey: String, on runners: [CeremonyRunner],
        using run: @escaping Writer = {
            let result = await CLI.run($0)
            // The reply a caller reads a token from is stdout alone; a failure
            // is shown in everything the CLI said.
            return (result.ok, result.ok ? result.output : result.said)
        }
    ) async -> Outcome {
        var outcome = Outcome()
        var failures: [String] = []
        for runner in runners {
            let a = await write(
                key: keyA, label: label, clientID: clientID, scope: scope, shell: false,
                nodeKey: nodeKey, to: runner, using: run)
            if !a.ok {
                // Key B is skipped when Key A did not land, and only then. Not a
                // rollback — the two lines are independent and one without the
                // other is a coherent state — but everything that stops Key A
                // (runner asleep, no agent, no daemon) stops Key B a second later,
                // so trying anyway buys a second ssh timeout and a second copy of
                // the same message in the transcript.
                failures.append("\(runner.label): \(a.output)")
                continue
            }
            // Recorded the moment the daemon answered yes, and only then. This
            // is the fact the new device is about to be handed.
            outcome.written.insert(runner.id)

            // Read off KEY A's reply and nowhere else. Key A's line is the one
            // carrying the node key, so it is the only call that can admit
            // anybody to a tunnel — and Key B's reply, which is a second `--json`
            // object in the same transcript, would answer with an empty
            // `connBlob` and silently undo the decision if it were read too.
            if case .tailcat(let token) = Self.reach(granting: runner, token: Self.token(in: a.output)) {
                outcome.tunneled[runner.id] = token
            }

            guard let keyB else { continue }
            // Sequential here only because the transcript should read in order.
            // It used to have to be: the daemon read the fence OUTSIDE its
            // writer's lock, so a Mac firing both of its keys at one runner could
            // have the loser's key silently dropped. That is closed —
            // `fence::update` now holds the lock across the read, the decision and
            // the write, and `rpc_over_socket.rs`'s
            // `a_macs_two_enrollments_may_land_at_the_same_moment` fires exactly
            // this pair concurrently and asserts both lines survive.
            // No node key on this one, and the empty string says so rather than
            // the argument being absent: a plain line has no forced command to
            // carry one, the daemon refuses the pair, and one device's node key
            // belongs on one line anyway.
            let b = await write(
                key: keyB, label: label, clientID: clientID, scope: "host_admin", shell: true,
                nodeKey: "", to: runner, using: run)
            if !b.ok {
                outcome.shellRefused.insert(runner.id)
                // Named, because a runner that took Key A and refused Key B is a
                // real outcome with a real consequence — Far Cooler works there
                // and Zed does not — and a transcript that said only the runner's
                // name would send somebody to re-run the whole ceremony.
                failures.append("\(runner.label) (shell access): \(b.output)")
            }
        }
        outcome.transcript = failures.isEmpty ? nil : failures.joined(separator: "\n\n")
        return outcome
    }

    /// One `client enroll`, one line in the runner's fence.
    ///
    /// `--shell-access` is the flag that picks the plain line. It is a flag with
    /// no value rather than `--shell-access true` so that its absence is the
    /// restricted line, which is what every existing caller means.
    private static func write(
        key: String, label: String, clientID: String, scope: String, shell: Bool, nodeKey: String,
        to runner: CeremonyRunner, using run: Writer
    ) async -> (ok: Bool, output: String) {
        await run(
            arguments(
                key: key, label: label, clientID: clientID, scope: scope, shell: shell,
                nodeKey: nodeKey, runner: runner.id))
    }

    /// The command line for one enrollment.
    ///
    /// Split out so the one decision in it can be tested without a runner to
    /// write to: **this Mac takes no `--runner` at all.**
    ///
    /// The empty target is how the whole app names this Mac — the local daemon,
    /// over a Unix socket, no ssh anywhere in it — and every other caller says
    /// so by OMITTING the flag: `DaemonClient`, `Runners` and
    /// `RunnerSettingsStore` all spell it `target.isEmpty ? [] : [...]`. This
    /// was the one place that passed it unconditionally, and `--runner ""` does
    /// not mean "local" to the CLI. clap hands the command `Some("")`, which is
    /// a target like any other, so it took the ssh path with an empty
    /// destination and failed with "Could not reach  over ssh." — the double
    /// space being the whole of the runner's name, and the phone's key never
    /// reaching the Mac it was being added to.
    ///
    /// ## `--node-key`, and why it is on one of a Mac's two calls
    ///
    /// The new device's tailcat node public key, straight off its offer: 43
    /// characters of unpadded base64-URL, or empty from a device that has none.
    /// Handing it to the daemon is the ONLY way it reaches the runner's line —
    /// `enrollment::enroll` passes it to `fence::render`, and nothing else in
    /// the tree writes a node key onto a line somebody else's key is on. A
    /// pairing that omitted it wrote a perfectly good `authorized_keys` line
    /// that the runner's tunnel then refused to admit, and the symptom was ten
    /// seconds of silence and `no_answer` on the phone.
    ///
    /// **Only on the restricted line.** The key is written INTO the forced
    /// command, and Key B's line has no forced command to write it into — so
    /// the daemon refuses `--node-key` beside `--shell-access` rather than
    /// dropping it, and a caller told "written" about a key that went nowhere
    /// would wait forever for a tunnel that admits it. The `!shell` guard here
    /// is not a second copy of that rule: it is this Mac never asking for the
    /// pair in the first place, so the refusal stays a bug report rather than a
    /// thing people meet.
    ///
    /// **Empty means "this device asked for no tunnel", and the flag is left
    /// off entirely.** Not `--node-key ""`: the daemon reads absence and an
    /// unusable key differently, and `fence::render` refuses the empty string
    /// as unusable — so passing it would refuse a v=1 device, or a phone whose
    /// own mint failed, an enrollment it is entitled to.
    static func arguments(
        key: String, label: String, clientID: String, scope: String, shell: Bool,
        nodeKey: String, runner: String
    ) -> [String] {
        var arguments = ["--json"]
        if !runner.isEmpty { arguments += ["--runner", runner] }
        arguments += [
            "client", "enroll",
            "--key", key, "--label", label, "--client-id", clientID, "--scope", scope,
        ]
        if shell { arguments.append("--shell-access") }
        if !shell, !nodeKey.isEmpty { arguments += ["--node-key", nodeKey] }
        return arguments
    }

    // MARK: - Which reach the reply carries

    /// The reach the reply carries for one runner: its tunnel, or the address
    /// it was granted under.
    ///
    /// **The tunnel first, the address only on request** (owner, Oct 13,
    /// ov-485). A runner that answered with a token is granted through the
    /// tunnel, whatever its address, and ``Outcome/granting(_:)`` keeps the
    /// address beside it as the alternative. The new device dials the tunnel;
    /// when that fails it says so and asks before it dials the address, and it
    /// never switches on its own (``TunnelRoute``).
    ///
    /// This used to keep every address that travels and grant the tunnel only
    /// to a runner stranded on a LAN name, because a reply held one route and a
    /// tunnel that failed had nothing to fall back on. The alternative is what
    /// changed that: a failed tunnel is now a question with an answer, not a
    /// runner that is gone.
    ///
    /// No token, no tunnel: a runner serving none, one too old to have one, and
    /// one that could not start one all pair at their address, and a device
    /// that named no node key is never given a token at all. A tunnel that
    /// will not come up never fails a pairing.
    static func reach(granting runner: CeremonyRunner, token: String) -> CeremonyReach {
        // Already a tunnel, so there is nothing to decide. Falling through
        // would overwrite that runner's token with the token of whatever runner
        // this enrollment happened to be talking to.
        guard case .direct = runner.reach else { return runner.reach }
        guard !token.isEmpty else { return runner.reach }
        return .tailcat(token: token)
    }

    /// The tunnel token in one `client enroll --json` reply, or `""`.
    ///
    /// **Line by line, not on the whole string.** `CLI.run` hands back stdout
    /// and stderr concatenated, and the CLI's stderr carries whatever `tracing`
    /// wrote — so the reply is one line among several and parsing the join would
    /// fail on every runner that logged anything. The first line that is a JSON
    /// object naming `connBlob` is the reply: `client enroll --json` prints
    /// exactly one, and prints it before it returns.
    ///
    /// Anything else is `""`, which reads as "no tunnel" and keeps the address.
    /// That covers an older CLI whose reply has no such field, a reply this
    /// build cannot parse, and a runner that failed — none of which is a reason
    /// to fail a pairing, and all of which are a reason not to hand somebody a
    /// token nothing vouched for.
    ///
    /// The value is not inspected beyond being non-empty. A token is the Go
    /// side's own encoding of a node's address, its key and a DERP map; this
    /// app has no business having an opinion about its shape, and a length check
    /// invented here would be a rule with no owner.
    static func token(in output: String) -> String {
        for line in output.components(separatedBy: .newlines) {
            guard let data = line.data(using: .utf8),
                let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                let blob = object["connBlob"] as? String
            else { continue }
            return blob
        }
        return ""
    }

    /// The sentence shown when some runners could not be written to.
    ///
    /// It does not say why. From here the cause is not knowable — a runner
    /// asleep, an ssh key that is not loaded, a daemon that is not installed
    /// and a fence that is damaged all look identical — and a screen that
    /// guesses is how an app ends up telling somebody to loosen an sshd setting
    /// that was never the problem. The transcript under it carries the CLI's
    /// own words, which is where a cause appears when there is one to report.
    ///
    /// **It does not promise a retry either.** It used to end "Far Cooler will
    /// try again when this Mac reconnects", and nothing in this repo retries an
    /// enrollment: there is no queue, no reconnect hook, and by design there
    /// will not be one. So that sentence left somebody waiting for a write that
    /// is never attempted — the worst shape a failure can take, because it asks
    /// for patience instead of action. iOS states the rule above
    /// `CeremonyStore.note(about:outcomes:)` ("It deliberately does NOT say the
    /// key will be written later"); this is the Mac's half of it. What is left
    /// is what is true — those runners do not have the key — followed by the
    /// one thing that changes it.
    ///
    /// **A Mac, where iOS says "a device".** Granting is a Mac-and-CLI
    /// capability by design: `client.enroll` is served at `Scope::HostAdmin`
    /// (`crates/daemon/src/rpc.rs`) and a phone is enrolled at `control`, so
    /// sending somebody to their phone here would be sending them to a second
    /// failure with a less useful message.
    ///
    /// "The new device" rather than "this device": the device being enrolled is
    /// the one that showed the code, and on a Mac's screen "this device" reads
    /// as the Mac doing the adding.
    static let couldNotReachAll =
        "Some runners don’t have the new device’s key yet. You can add it later from a Mac "
        + "that can reach them."

    /// The transcript shown when the scanned code's key could not be read.
    ///
    /// Nothing is enrolled in that case — see the `clientID` branch in
    /// `AddDeviceView.confirm(name:)`, which has no id to enroll under and
    /// refuses to invent one — so this says what happened rather than leaving
    /// the transcript empty. An empty transcript would show no sentence at all,
    /// and "nothing was written and nothing was said" is the one outcome here
    /// that a person cannot detect.
    ///
    /// It takes the transcript's slot rather than a new screen: the box under
    /// ``couldNotReachAll`` is the place this flow already puts "what the thing
    /// that talked to the runners had to say", and one sentence is what there is
    /// to say. The reply code is still shown, because the new device still needs
    /// it — the runners in it simply have no line for this key.
    static let unreadableKey =
        "Far Cooler couldn’t read the new device’s key, so no runner was updated."

    /// The sentence for a Mac that took Key A everywhere and lost Key B
    /// somewhere.
    ///
    /// A separate outcome because it has a separate consequence: Far Cooler
    /// works on those runners and Zed, Git and Terminal do not, and
    /// ``couldNotReachAll`` would send somebody looking for a runner that is
    /// missing when none of them is. Only a Mac can reach this — a phone has no
    /// second key.
    static let shellAccessIncomplete =
        "The new Mac can use Far Cooler with these runners, but Zed, Git, and Terminal on it "
        + "can’t reach all of them yet."

    /// What to say when runners were granted through the tunnel.
    ///
    /// Builds the one picture a person needs: Tailcat is the route and works
    /// from anywhere, and the address is kept for the day it doesn't. Not a
    /// warning: nothing went wrong, so it is drawn in the secondary color, not
    /// the orange ``couldNotReachAll`` uses. It names no runner; one sentence
    /// covers a fleet.
    ///
    /// When some runners, not all, were granted at their address alone, the
    /// warning above this one names those, so this one says "the other
    /// runners" rather than appearing to contradict it (ov-530 R2-3).
    static func reachedThroughTheTunnel(others: Bool) -> String {
        // With some runners already at their address, "each runner" would
        // include the ones the warning above names (ov-530 S3).
        "The new device connects to \(others ? "the other" : "these") runners through Tailcat, Far Cooler’s "
            + "tunnel, from any network. If Tailcat can’t connect, it’ll offer "
            + (others ? "their addresses instead." : "each runner’s address instead.")
    }

    /// What to say about this enrollment, or nil when there is nothing to say.
    ///
    /// Chosen from the RECORDS, not from whether a transcript exists: the
    /// records are what the new device is being handed, so a sentence disagreeing
    /// with them is the screen and the code saying different things about the
    /// same runners. iOS makes the same choice in
    /// `CeremonyStore.note(about:outcomes:)`.
    ///
    /// Neither sentence promises a retry, because nothing retries. See
    /// ``couldNotReachAll``.
    static func note(about granting: [CeremonyRunner], outcome: Outcome) -> String? {
        if granting.contains(where: \.pending) { return couldNotReachAll }
        return outcome.shellRefused.isEmpty ? nil : shellAccessIncomplete
    }
}
