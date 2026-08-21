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
///     --label "iPhone 17" --client-id <uuid> --scope control
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
                return runner
            }
        }
    }

    /// One `client enroll`, as the thing that runs it.
    ///
    /// Injected so that ``enroll(keyA:keyB:label:clientID:scope:on:using:)`` —
    /// which is where every decision about what got written is made — can be
    /// tested without a runner to write to. The default is the real CLI, and no
    /// caller in the app passes anything else.
    typealias Writer = @Sendable ([String]) async -> (ok: Bool, output: String)

    /// Enroll a device's keys on each runner, and answer with what landed.
    ///
    /// `keyB` is a Mac's shell key and nil for everything else — there is no Zed
    /// on a phone. Its scope is not a parameter because it cannot vary.
    ///
    /// One runner at a time rather than concurrently: these are writes to the
    /// same kind of file on different machines, and a transcript that interleaves
    /// is a transcript nobody can read. The list is short.
    static func enroll(
        keyA: String, keyB: String?, label: String, clientID: String, scope: String,
        on runners: [CeremonyRunner], using run: @escaping Writer = { await CLI.run($0) }
    ) async -> Outcome {
        var outcome = Outcome()
        var failures: [String] = []
        for runner in runners {
            let a = await write(
                key: keyA, label: label, clientID: clientID, scope: scope, shell: false,
                to: runner, using: run)
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

            guard let keyB else { continue }
            // Sequential here only because the transcript should read in order.
            // It used to have to be: the daemon read the fence OUTSIDE its
            // writer's lock, so a Mac firing both of its keys at one runner could
            // have the loser's key silently dropped. That is closed —
            // `fence::update` now holds the lock across the read, the decision and
            // the write, and `rpc_over_socket.rs`'s
            // `a_macs_two_enrollments_may_land_at_the_same_moment` fires exactly
            // this pair concurrently and asserts both lines survive.
            let b = await write(
                key: keyB, label: label, clientID: clientID, scope: "host_admin", shell: true,
                to: runner, using: run)
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
        key: String, label: String, clientID: String, scope: String, shell: Bool,
        to runner: CeremonyRunner, using run: Writer
    ) async -> (ok: Bool, output: String) {
        await run(
            arguments(
                key: key, label: label, clientID: clientID, scope: scope, shell: shell,
                runner: runner.id))
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
    static func arguments(
        key: String, label: String, clientID: String, scope: String, shell: Bool, runner: String
    ) -> [String] {
        var arguments = ["--json"]
        if !runner.isEmpty { arguments += ["--runner", runner] }
        arguments += [
            "client", "enroll",
            "--key", key, "--label", label, "--client-id", clientID, "--scope", scope,
        ]
        if shell { arguments.append("--shell-access") }
        return arguments
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
