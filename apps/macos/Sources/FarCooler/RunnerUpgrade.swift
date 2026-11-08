import Foundation
import LocalAuthentication

/// Finishing this Mac's runner upgrade from the app (ov-984).
///
/// A new build whose runner's data is older than it can serve starts that
/// runner in recovery: it answers nothing but the five upgrade steps, so the
/// window has nothing to show. Before this, the only way through was
/// `farcooler workflow-cutover` five times from a terminal outside every pane.
/// Here it's one button.
///
/// - **The person, at the app.** Touch ID, or the login password, before any
///   step runs; a cancel runs nothing. Each step is the bundled CLI run as
///   this app's own child, so the runner's recovery seat check sees the app
///   on the caller's chain (`caller_seat::recovery_seat`), exactly as it sees
///   any other app action. Nothing goes through a pane, and the runner's check
///   is unchanged: the prompt is the person's word here, not a new way in.
/// - **Where it stands.** It asks the runner first (`inspect`) and starts at
///   the step the runner is on, so pressing it again after a failure goes on
///   rather than starting over.
/// - **Recover.** Before Commit the runner can put the original data back,
///   exactly as it was. From Commit on it can't (a committed upgrade only goes
///   forward), so Recover runs the steps that are left. Either way the person
///   presses one button and is told which it was.
@MainActor
final class RunnerUpgrade: ObservableObject {
    /// This Mac's runner's.
    static let local = RunnerUpgrade()

    /// The runner's five steps, in order.
    enum Step: Int, CaseIterable, Sendable {
        case prepare, commit, verify, release, finish

        /// The CLI's word for it (`workflow-cutover <op>`).
        var op: String {
            switch self {
            case .prepare: "prepare"
            case .commit: "commit"
            case .verify: "verify"
            case .release: "release"
            case .finish: "finish"
            }
        }

        /// What's happening, in plain words, while it runs.
        var title: String {
            switch self {
            case .prepare: "Getting your data ready"
            case .commit: "Making a backup and switching over"
            case .verify: "Checking everything came across"
            case .release: "Handing your data to this version"
            case .finish: "Restarting Far Cooler"
            }
        }

        /// What went wrong, when it didn't.
        var failure: String {
            switch self {
            case .prepare: "Far Cooler couldn’t get your data ready for this version."
            case .commit: "Far Cooler couldn’t make a backup and switch your data over."
            case .verify: "Far Cooler couldn’t confirm that everything came across."
            case .release: "Far Cooler couldn’t hand your data to this version."
            case .finish: "Far Cooler couldn’t finish the upgrade and restart."
            }
        }

        /// Whether the runner's data is past the point of putting it back:
        /// Commit has run when this step starts.
        var isCommitted: Bool { self.rawValue > Step.commit.rawValue }
    }

    /// What failed: a step, or putting the data back.
    enum Failed: Equatable, Sendable {
        case step(Step)
        case recover
    }

    enum Phase: Equatable {
        /// Waiting for the button. `notice` says how the last try ended, when
        /// there's something to say.
        case ready(notice: String?)
        /// Touch ID or the password, on screen.
        case confirming
        /// This step is running.
        case running(Step)
        /// Putting the original data back.
        case recovering
        /// `failed` didn't go: `detail` is the runner's own words.
        case failed(Failed, detail: String)
        /// All five ran; the window reads the runner again.
        case done
    }

    /// Whether the person said yes.
    enum Confirmed: Equatable, Sendable {
        case yes
        /// They closed the prompt.
        case cancelled
        /// It couldn't be asked: why, as a sentence.
        case unavailable(String)
    }

    /// How it reaches the runner and the person; replaced in tests.
    struct Driver: Sendable {
        /// Run the CLI with these arguments, as this app's child.
        var run: @Sendable ([String]) async -> CLI.Result
        /// Ask for Touch ID or the login password, saying `reason`.
        var confirm: @Sendable (String) async -> Confirmed
        /// A new attempt's name, for a runner not held under one already.
        var attempt: @Sendable () -> String

        static let live = Driver(
            run: { await CLI.run($0) },
            confirm: { await RunnerUpgrade.askThePerson(reason: $0) },
            attempt: { RunnerUpgrade.newAttempt() })
    }

    @Published private(set) var phase: Phase = .ready(notice: nil)
    /// The steps this run is taking, from where the runner stood.
    @Published private(set) var steps: [Step] = Step.allCases

    private let driver: Driver

    init(driver: Driver = .live) {
        self.driver = driver
    }

    /// Whether a run or a recover is under way, so the button waits.
    var isBusy: Bool {
        switch phase {
        case .confirming, .running, .recovering: true
        case .ready, .failed, .done: false
        }
    }

    /// What the Touch ID prompt says after "Far Cooler is trying to".
    static let finishReason = "finish upgrading this Mac’s runner"
    static let recoverReason = "recover this Mac’s runner from an upgrade that didn’t finish"

    /// Said under a failure: what Recover will do from here.
    static func recoverCopy(for failed: Failed) -> String {
        switch failed {
        case .step(let step) where step.isCommitted:
            "Your data is already upgraded and backed up. Recover finishes the steps that are left."
        case .step:
            "Your original data is kept. Recover puts it back exactly as it was."
        case .recover:
            "Your data hasn’t been changed by this. Try Recover again."
        }
    }

    /// Said once the original data is back.
    static let recoveredNotice = "Your data is back as it was before the upgrade. Finish Upgrade tries again."

    /// After Touch ID, run every step left. `refresh` runs once the runner has
    /// restarted, or once it stopped, so the window reads it again.
    func finish(refresh: @escaping @MainActor () async -> Void = {}) async {
        guard !isBusy else { return }
        guard await confirmed(Self.finishReason) else { return }
        await runForward(refresh: refresh)
    }

    /// After Touch ID: before Commit, put the original data back; after it,
    /// run the steps left; with nothing begun, go back to the start.
    func recover(refresh: @escaping @MainActor () async -> Void = {}) async {
        guard !isBusy else { return }
        guard await confirmed(Self.recoverReason) else { return }
        phase = .recovering
        let standing = await stand()
        switch standing {
        case .failed(let detail):
            phase = .failed(.recover, detail: detail)
        case .at(let step, _) where step.isCommitted:
            // Committed: the only way out is forward.
            await runForward(refresh: refresh)
        case .nothingBegun:
            phase = .ready(notice: nil)
        case .at:
            let ran = await driver.run(["--json", "workflow-cutover", "recover"])
            guard ran.ok else {
                phase = .failed(.recover, detail: ran.said)
                return
            }
            // The runner ends its recovery once the data is back, and starts
            // in recovery again, ready for another try.
            await refresh()
            phase = .ready(notice: Self.recoveredNotice)
        }
    }

    // MARK: - Running

    private func confirmed(_ reason: String) async -> Bool {
        let before = phase
        phase = .confirming
        switch await driver.confirm(reason) {
        case .yes:
            return true
        case .cancelled:
            phase = Self.settled(before)
            return false
        case .unavailable(let why):
            phase = .ready(notice: why)
            return false
        }
    }

    /// A phase to go back to when the person closed the prompt: the failure
    /// stays on screen, so Recover is still there.
    private static func settled(_ phase: Phase) -> Phase {
        switch phase {
        case .confirming, .running, .recovering, .done: .ready(notice: nil)
        case .ready, .failed: phase
        }
    }

    private func runForward(refresh: @escaping @MainActor () async -> Void) async {
        let standing = await stand()
        let start: Step
        var attempt: String?
        switch standing {
        case .failed(let detail):
            phase = .failed(.step(.prepare), detail: detail)
            return
        case .nothingBegun:
            start = .prepare
        case .at(let step, let held):
            start = step
            attempt = held
        }
        steps = Step.allCases.filter { $0.rawValue >= start.rawValue }
        for step in steps {
            phase = .running(step)
            var args = ["--json", "workflow-cutover", step.op]
            if step == .prepare { args += ["--attempt", attempt ?? driver.attempt()] }
            let ran = await driver.run(args)
            guard ran.ok else {
                phase = .failed(.step(step), detail: ran.said)
                return
            }
        }
        // Finish ends the runner's recovery; it starts again as usual.
        await refresh()
        phase = .done
    }

    /// Where the runner stands, from `inspect`.
    enum Standing: Equatable {
        /// No attempt on it: the next step is Prepare, under a new attempt.
        case nothingBegun
        /// The next step, and the attempt it's under.
        case at(Step, attempt: String?)
        /// The runner couldn't say: its own words.
        case failed(String)
    }

    private func stand() async -> Standing {
        let ran = await driver.run(["--json", "workflow-cutover", "inspect"])
        guard ran.ok else { return .failed(ran.said) }
        return Self.standing(inspected: ran.output)
    }

    /// The next step for what `inspect` printed. The authority's phase says
    /// how far the data has come (`legacy`, `quiesced`, `committed`,
    /// `released`); Verify leaves it `committed`, and checking again is safe.
    static func standing(inspected output: String) -> Standing {
        guard let data = output.data(using: .utf8),
            let body = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return .failed(output.isEmpty ? "The runner didn’t say where the upgrade stands." : output) }
        let attempt = body["attempt"] as? String
        let phase = (body["authority"] as? [String: Any])?["phase"] as? String
        switch phase {
        case "quiesced": return .at(.commit, attempt: attempt)
        case "committed": return .at(.verify, attempt: attempt)
        case "released": return .at(.finish, attempt: attempt)
        default:
            // Held by a live Prepare, or begun and stopped: Prepare under
            // that attempt, which the runner refuses under any other.
            if attempt != nil || body["artifacts"] as? Bool == true { return .at(.prepare, attempt: attempt) }
            return .nothingBegun
        }
    }

    // MARK: - Live

    /// Touch ID or the login password, with a context of its own each time,
    /// so no earlier unlock answers for this one.
    nonisolated static func askThePerson(reason: String) async -> Confirmed {
        let context = LAContext()
        context.touchIDAuthenticationAllowableReuseDuration = 0
        var problem: NSError?
        guard context.canEvaluatePolicy(.deviceOwnerAuthentication, error: &problem) else {
            if problem?.code == LAError.passcodeNotSet.rawValue {
                return .unavailable("Set a login password on this Mac to finish the upgrade.")
            }
            return .unavailable("This Mac couldn’t ask for Touch ID or your password. Try again.")
        }
        do {
            return try await context.evaluatePolicy(.deviceOwnerAuthentication, localizedReason: reason)
                ? .yes : .cancelled
        } catch let error as LAError where [.userCancel, .appCancel, .systemCancel].contains(error.code) {
            return .cancelled
        } catch {
            return .unavailable("Far Cooler couldn’t confirm it’s you. Try again.")
        }
    }

    /// `mac-20261108-155012`: letters, digits and dashes, as the runner takes.
    nonisolated static func newAttempt(now: Date = Date()) -> String {
        let format = DateFormatter()
        format.locale = Locale(identifier: "en_US_POSIX")
        format.timeZone = TimeZone(identifier: "UTC")
        format.dateFormat = "yyyyMMdd-HHmmss"
        return "mac-" + format.string(from: now)
    }
}
