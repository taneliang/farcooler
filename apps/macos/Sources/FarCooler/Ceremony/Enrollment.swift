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
    /// Enroll a device's keys on each runner, and answer with a transcript for
    /// the screen. Nil when everything worked.
    ///
    /// `keyB` is a Mac's shell key and nil for everything else — there is no Zed
    /// on a phone. Its scope is not a parameter because it cannot vary.
    ///
    /// One runner at a time rather than concurrently: these are writes to the
    /// same kind of file on different machines, and a transcript that interleaves
    /// is a transcript nobody can read. The list is short.
    static func enroll(
        keyA: String, keyB: String?, label: String, clientID: String, scope: String,
        on runners: [CeremonyRunner]
    ) async -> String? {
        var failures: [String] = []
        for runner in runners {
            let a = await write(
                key: keyA, label: label, clientID: clientID, scope: scope, shell: false, to: runner)
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
                to: runner)
            if !b.ok {
                // Named, because a runner that took Key A and refused Key B is a
                // real outcome with a real consequence — Far Cooler works there
                // and Zed does not — and a transcript that said only the runner's
                // name would send somebody to re-run the whole ceremony.
                failures.append("\(runner.label) (shell access): \(b.output)")
            }
        }
        guard !failures.isEmpty else { return nil }
        return failures.joined(separator: "\n\n")
    }

    /// One `client enroll`, one line in the runner's fence.
    ///
    /// `--shell-access` is the flag that picks the plain line. It is a flag with
    /// no value rather than `--shell-access true` so that its absence is the
    /// restricted line, which is what every existing caller means.
    private static func write(
        key: String, label: String, clientID: String, scope: String, shell: Bool,
        to runner: CeremonyRunner
    ) async -> (ok: Bool, output: String) {
        var arguments = [
            "--json", "--runner", runner.id, "client", "enroll",
            "--key", key, "--label", label, "--client-id", clientID, "--scope", scope,
        ]
        if shell { arguments.append("--shell-access") }
        return await CLI.run(arguments)
    }

    /// The sentence shown when some runners could not be written to.
    ///
    /// It does not say why. From here the cause is not knowable — a runner
    /// asleep, an ssh key that is not loaded, a daemon that is not installed
    /// and a fence that is damaged all look identical — and a screen that
    /// guesses is how an app ends up telling somebody to loosen an sshd setting
    /// that was never the problem.
    static let couldNotReachAll =
        "Some runners couldn’t be updated. Far Cooler will try again when this Mac reconnects."
}
