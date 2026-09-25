import Foundation
import FoundationModels
import Synchronization

/// A task's short name: what its worktree directory, its branch and its
/// sidebar row are called.
///
/// The ⌘N panel used to name a task after its whole description cut to 42
/// characters, so a sidebar full of tasks read as a column of truncated
/// sentences. A name is a handful of words now — at most `maxLength`
/// characters — and the full description is still the agent's first message,
/// so nothing the person typed is lost; it just is not the label.
///
/// Two ways to get one. `heuristic` is instant and always answers: the
/// description with its filler words taken out. `TaskNamer` asks Apple's
/// on-device model first when this Mac has one, and falls back to the
/// heuristic whenever the model is missing, slow or says something unusable.
enum TaskName {
    /// The longest name either path produces. Long enough for three or four
    /// real words, short enough that a sidebar row shows all of it.
    static let maxLength = 24
    static let maxWords = 4

    /// Words that carry no meaning in a task description: articles,
    /// pronouns, politeness, and the "can you" / "I want to" that starts half
    /// of what people type into a prompt box.
    static let fillerWords: Set<String> = [
        "a", "able", "about", "actually", "again", "all", "also", "am", "an", "and", "any",
        "are", "as", "at", "be", "been", "being", "but", "by", "can", "could", "do", "does",
        "even", "ever", "for", "from", "get", "gets", "go", "got", "had", "has", "have", "hey",
        "how", "i", "if", "im", "in", "into", "is", "it", "its", "ive", "just", "now",
        "sometimes", "still", "yet",
        "kind", "kinda", "let", "lets", "like", "look", "maybe", "me", "might", "my", "need",
        "needs", "of", "ok", "okay", "on", "or", "our", "please", "pls", "really", "should",
        "so", "some", "something", "sort", "that", "the", "their", "them", "then", "there",
        "these", "this", "those", "to", "try", "up", "us", "very", "wanna", "want", "was", "we",
        "were", "what", "when", "where", "which", "while", "who", "why", "will", "with",
        "would", "you", "your",
    ]

    /// The words of `text`, lowercased, accents folded, and split on anything
    /// that is not a letter or digit a directory name can hold.
    ///
    /// ASCII only, because the directory is: `WorktreeName.slug` turns every
    /// other character into a dash, so a word outside ASCII would reach the
    /// sidebar as dashes. Folding first keeps `naïve` as `naive` rather than
    /// losing it.
    static func words(_ text: String) -> [String] {
        text.folding(options: [.diacriticInsensitive, .caseInsensitive], locale: .init(identifier: "en_US"))
            .lowercased()
            .split { !($0.isASCII && ($0.isLetter || $0.isNumber)) }
            .map(String.init)
    }

    /// As many of `words` as fit in `maxLength`, hyphenated, at most
    /// `maxWords` of them. A first word longer than the whole budget is cut.
    static func pack(_ words: [String]) -> String {
        var out = ""
        for word in words.prefix(maxWords) {
            let next = out.isEmpty ? word : out + "-" + word
            if next.count > maxLength {
                if out.isEmpty { out = String(word.prefix(maxLength)) }
                break
            }
            out = next
        }
        return out
    }

    /// The name with no model: the description's meaningful words, in order.
    ///
    /// "Please fix the flaky reconnect test in the iOS app" is
    /// `fix-flaky-reconnect-test`. A description that is nothing but filler
    /// keeps its own first words rather than coming out empty, and one with no
    /// word a directory can hold at all is `task`.
    static func heuristic(_ description: String) -> String {
        let all = words(description)
        let meaningful = all.filter { !fillerWords.contains($0) }
        let name = pack(meaningful.isEmpty ? all : meaningful)
        return name.isEmpty ? "task" : name
    }

    /// A model's answer as a name, or nil when there is nothing usable in it.
    ///
    /// Models decorate — "fix - reconnect - test", a trailing period, a line
    /// of explanation — so only the first line is read, and it is run through
    /// the same `words` and `pack` as the heuristic. More words than a name
    /// has is a sentence, not a name, and is refused.
    static func fromModel(_ answer: String) -> String? {
        let line = answer.split(whereSeparator: \.isNewline).first.map(String.init) ?? ""
        let found = words(line)
        guard !found.isEmpty, found.count <= maxWords * 2 else { return nil }
        let name = pack(found)
        return name.isEmpty ? nil : name
    }

    /// `name`, or `name-2`, `name-3` … when a worktree already has it. Still
    /// within `maxLength`: the stem is cut to make room for the suffix.
    static func unique(_ name: String, taken: Set<String>) -> String {
        guard taken.contains(name) else { return name }
        for n in 2... {
            let suffix = "-\(n)"
            var stem = String(name.prefix(maxLength - suffix.count))
            while stem.hasSuffix("-") { stem.removeLast() }
            let candidate = stem + suffix
            if !taken.contains(candidate) { return candidate }
        }
        return name
    }
}

/// Names a task, asking a model first when there is one.
///
/// **The deadline is hard.** A model that has not answered by `timeout` is
/// abandoned and the heuristic is used; the call returns at the deadline
/// whether or not the model notices it was cancelled. The race is two
/// unstructured tasks and a continuation resumed exactly once, rather than a
/// task group, because a task group waits for every child on the way out —
/// a model call that ignored cancellation would hold the name, and the
/// panel, for as long as it liked.
///
/// The model is a closure so a test can stand in for it: slow, broken or
/// talkative, without a model on the machine.
struct TaskNamer: Sendable {
    typealias Model = @Sendable (String) async throws -> String

    let model: Model?
    let timeout: Duration

    /// Measured on this project's own task descriptions with the on-device
    /// model on an M-series Mac: warm, 266–412 ms for twenty descriptions
    /// (p50 341 ms, p90 398 ms); the first call after launch, before the model
    /// is warm, 1.16 s. The ~400 ms first proposed would throw away one answer
    /// in ten from a warm model for no gain — nobody waits on this, since the
    /// panel names while you type (see `QuickCreate`) — so the deadline is
    /// twice the warm worst case. It bounds a model that has hung; it is not
    /// a latency budget.
    static let defaultTimeout: Duration = .milliseconds(800)

    init(model: Model?, timeout: Duration = TaskNamer.defaultTimeout) {
        self.model = model
        self.timeout = timeout
    }

    func name(for description: String) async -> String {
        let fallback = TaskName.heuristic(description)
        guard let model else { return fallback }
        let answer = await Self.race(model, description, timeout)
        return answer.flatMap(TaskName.fromModel) ?? fallback
    }

    private static func race(
        _ model: @escaping Model, _ description: String, _ timeout: Duration
    ) async -> String? {
        let resumed = Mutex(false)
        return await withCheckedContinuation { (continuation: CheckedContinuation<String?, Never>) in
            let finish: @Sendable (String?) -> Void = { value in
                let first = resumed.withLock { done in
                    defer { done = true }
                    return !done
                }
                if first { continuation.resume(returning: value) }
            }
            let work = Task {
                finish(try? await model(description))
            }
            Task {
                try? await Task.sleep(for: timeout)
                work.cancel()
                finish(nil)
            }
        }
    }

    /// Apple's on-device model, when this Mac has it on: Apple Intelligence
    /// enabled, the model downloaded. `nil` otherwise, and the heuristic
    /// names everything — which is also every Mac this app supports that is
    /// not Apple silicon.
    ///
    /// No `#available` check: `FoundationModels` is macOS 26.0 and so is this
    /// app's deployment target. What varies between Macs is availability, and
    /// that is asked at runtime.
    static let onDevice = TaskNamer(model: OnDeviceNamer.model())
}

/// The on-device model's half of `TaskNamer`.
enum OnDeviceNamer {
    static let instructions = """
        You name a coding task in two to four words, like a short git branch name. \
        Reply with only those words in lowercase, separated by single hyphens. \
        Use the task's key nouns and verb. No punctuation, no explanation.
        """

    static func model() -> TaskNamer.Model? {
        guard SystemLanguageModel.default.availability == .available else { return nil }
        return { description in
            let session = LanguageModelSession(instructions: instructions)
            let response = try await session.respond(
                to: description,
                options: GenerationOptions(samplingMode: .greedy, maximumResponseTokens: 16))
            return response.content
        }
    }

    /// Load the model before it is needed. The first answer after launch
    /// took 1.16 s cold and ~0.35 s warm; the panel calls this when it opens.
    static func prewarm() {
        guard SystemLanguageModel.default.availability == .available else { return }
        LanguageModelSession(instructions: instructions).prewarm()
    }
}
