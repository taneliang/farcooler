import Foundation

/// Putting a device's key into a runner's `~/.ssh/authorized_keys`.
///
/// **The daemon owns the write**, on every runner including this Mac. Not a
/// shell command appending a line: that file is the one whose corruption costs
/// somebody SSH access to their own machine, so the write is
/// descriptor-anchored, `O_NOFOLLOW`, locked, atomic, `fsync`ed twice and
/// backed up — `crates/daemon/src/enrollment.rs` over `crates/daemon/src/fence.rs`.
/// The app's part is to ask.
///
/// ## THE ONE THING STILL MISSING: `farcooler client`
///
/// The daemon serves `client.enroll`, `client.list` and `client.revoke`, and the
/// client core's `farcooler_client_call` dispatch in `crates/client/src/ffi.rs`
/// now has an arm for all three — so a PHONE can reach them. **The CLI still has
/// no `client` subcommand**, so every command this file builds is not a command,
/// and a Mac's enrollment fails on every runner with a usage error. Checked
/// against `crates/cli/src/main.rs`'s `Command` enum, which has `root`, `repo`,
/// `theme`, `settings`, `adapter`, `workspace`, `terminal`, `changes`,
/// `worktree`, `layout`, `push` and `runner`, and no `client`.
///
/// The CLI is the right route for a Mac and the wrong one for a phone, for the
/// same reason each: the CLI runs real `ssh`, so it gets the agent, the
/// passphrase prompt, `ProxyJump` and everything else a person has already set
/// up — and a phone has no `ssh` at all. So both surfaces are wanted, and this
/// enum is the Mac's half, ready for the subcommand. What it needs to accept,
/// exactly:
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
/// Until the subcommand exists this reports what it could not do, in a sentence,
/// and the ceremony still shows the reply — because the reply is what the new
/// device needs, and a runner that was asleep is already an ordinary outcome the
/// manifest carries as `pending`.
///
/// ## A Mac is TWO enrollments
///
/// Key A's line is `restrict,command="farcoolerd --stdio …"`. Key B's line is
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
            // **SEQUENTIAL, AND THIS MUST STAY SEQUENTIAL.** Not a style
            // preference and not something to fold into a `withTaskGroup` or a
            // parallel map later: `crates/daemon/src/enrollment.rs` does its
            // read-modify-write with the READ OUTSIDE the writer's lock, so two
            // enrollments landing in the same instant each rebuild the fence from
            // a snapshot taken before the other's write and the loser's key is
            // silently gone. Every other caller is a person driving one ceremony,
            // which is what bounds that window — a Mac firing both of its keys at
            // one runner at once is the single easiest way in the product to hit
            // it, and the symptom is a Mac that enrolled "successfully" and has no
            // shell access, or worse, no Far Cooler access.
            //
            // The fix belongs in the writer, not here. Closing it means giving a
            // routine whose failure mode is losing SSH access a new shape, which
            // is not a change to make in passing.
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
        "Some runners didn't take the key. They'll be added when this Mac next reaches them."
}
