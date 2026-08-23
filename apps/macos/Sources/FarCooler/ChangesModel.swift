import AgentKit
import Foundation
import SwiftUI

/// What a worktree changed.
///
/// Decoded from `farcooler changes … --json`, which is the same path every other
/// read in this app takes. The daemon owns all of it; nothing here computes
/// state, it draws what the runner derived — the same contract the terminal
/// rows keep.

struct ChangeSet: Decodable, Equatable {
    var branch: String
    var baseRef: String
    /// Where `baseRef` came from: `recorded`, `upstream`, `pr_base`,
    /// `default_branch`, `guessed`, or `unknown`.
    ///
    /// Optional for the reason `ChangeCommit`'s own comment gives at length: it
    /// decodes INSIDE this type, and a runner whose `farcooler` predates the key
    /// would otherwise fail the whole change set over it. Such a runner is not
    /// hypothetical — the daemon has sent `base_source` since `BaseSource`
    /// existed, but the CLI's copy of the JSON builder did not print it until
    /// `c2f1117`, so every runner installed before that answers without it.
    var baseSource: String?
    var baseCommit: String
    var headCommit: String
    var insertions: Int
    var deletions: Int
    var commits: [ChangeCommit]
    var files: [ChangedFile]
    var workingTree: WorkingTree?

    enum CodingKeys: String, CodingKey {
        case branch
        case baseRef = "base_ref"
        case baseSource = "base_source"
        case baseCommit = "base_commit"
        case headCommit = "head_commit"
        case insertions, deletions, commits, files
        case workingTree = "working_tree"
    }

    static let empty = ChangeSet(
        branch: "", baseRef: "", baseSource: nil, baseCommit: "", headCommit: "",
        insertions: 0, deletions: 0, commits: [], files: [], workingTree: nil)

    var isDirty: Bool {
        guard let w = workingTree else { return false }
        return !w.staged.isEmpty || !w.unstaged.isEmpty || !w.untracked.isEmpty
            || !w.conflicted.isEmpty
    }

    /// Whether nothing knew what this branch is based on, so a local `main` was
    /// assumed.
    ///
    /// The one source worth warning about, and the phone's `baseIsGuessed` makes
    /// the same call for the same reason: the other five are recorded facts, and
    /// a guess is the only one that can silently produce a diff that looks
    /// exactly like a right one. An absent key reads as not-a-guess, which is
    /// the honest answer for a runner that cannot say — see `BaseSource` in the
    /// protocol, where `unknown` is likewise not a guess.
    var baseIsGuessed: Bool { baseSource == "guessed" }
}

/// One commit on this branch, as the history picker draws it.
///
/// ## Everything past `timestamp` is optional, and has to be
///
/// Swift's synthesized `Decodable` throws on a missing key, and this type
/// decodes INSIDE `ChangeSet` — so a runner whose `farcooler` predates any one
/// of these fields would fail the decode of the ENTIRE change set, every file
/// and every commit, over one absent key. Optional is not a nicety here: it is
/// what lets an older runner draw a commit with no rationale and no counts,
/// which is exactly what that runner knows.
///
/// ## The counts
///
/// They were hardcoded zeroes in `commits_since` for as long as that function
/// existed, and `change_set_json` did not project them — which is why a
/// commit's `+N −M` used to appear only once it had been SELECTED, summed from
/// the file list that selection fetches. `--shortstat` on the same `git log`
/// answers now, with `--diff-merges=first-parent`, so a row agrees with the
/// file list it opens.
///
/// **Zero still means unknown**, which is why `counts` returns nil for it. A
/// runner on a git older than 2.31 rejects `--diff-merges` and the daemon
/// retries without it, leaving merges at the zeroes they always had — so
/// `+0 −0` would be a fact nobody established. A commit that genuinely changed
/// nothing is rare enough that staying quiet about it is the better trade.
struct ChangeCommit: Decodable, Equatable, Identifiable {
    var sha: String
    var subject: String
    /// Everything the author wrote after the subject line.
    ///
    /// It matters more for agent work than for human work. An agent's commit
    /// body is usually the closest thing to a written rationale for what it
    /// did — why this approach, what it decided against, what it could not
    /// finish — and it is the cheapest context available before reading a
    /// single line of diff.
    var body: String?
    var author: String
    var timestamp: Int
    var filesChanged: Int?
    var insertions: Int?
    var deletions: Int?

    enum CodingKeys: String, CodingKey {
        case sha, subject, body, author, timestamp, insertions, deletions
        case filesChanged = "files_changed"
    }

    var id: String { sha }
    var short: String { String(sha.prefix(8)) }

    /// The two line counts, when the daemon actually counted them.
    ///
    /// Nil rather than `(0, 0)` for the reason the type comment gives: on this
    /// wire a zero is indistinguishable from "this runner could not tell you".
    var counts: (insertions: Int, deletions: Int)? {
        let plus = insertions ?? 0
        let minus = deletions ?? 0
        guard plus > 0 || minus > 0 else { return nil }
        return (plus, minus)
    }

    /// The body with its surrounding whitespace gone, or nil if there was none.
    ///
    /// Nil rather than an empty string so every caller's `if let` is the same
    /// shape whether the field was absent (an older runner) or present and
    /// empty (a commit with only a subject, which is most of them).
    var bodyText: String? {
        let trimmed = (body ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }

    /// The first paragraph worth putting in a history row.
    ///
    /// The first paragraph rather than the first line, because a body is prose
    /// that was wrapped for a terminal and its first line is half a sentence.
    ///
    /// Trailer-only paragraphs are skipped. Every commit in this repository
    /// ends in `Co-Authored-By:` and most agent commits add more of the same,
    /// so a row whose one line of preview reads `Co-Authored-By: Claude …` has
    /// spent the most valuable line in the popover saying nothing. Skipped, not
    /// stripped: nothing here removes a line from a body anyone asks to read.
    ///
    /// The same rule the phone applies in its own `ChangeCommit.bodyPreview`,
    /// written out again rather than shared: `AgentKit` is where one copy of
    /// this belongs, and moving it there is a change to a module both apps
    /// compile. It is a hoist waiting to happen, not a rule with two opinions.
    var bodyPreview: String? {
        guard let bodyText else { return nil }
        for paragraph in bodyText.components(separatedBy: "\n\n") {
            let lines = paragraph.split(separator: "\n", omittingEmptySubsequences: true)
            guard !lines.isEmpty else { continue }
            guard lines.allSatisfy({ Self.isTrailer(String($0)) }) else {
                return lines.joined(separator: " ").trimmingCharacters(in: .whitespaces)
            }
        }
        return nil
    }

    /// `Key: value` at the start of a line, which is git's own shape for a
    /// trailer. Deliberately loose — this decides what to show first, not what
    /// to keep, so a false positive costs a paragraph its place in a preview
    /// and never a line of the body.
    private static func isTrailer(_ line: String) -> Bool {
        guard let colon = line.firstIndex(of: ":"), colon > line.startIndex else { return false }
        let key = line[line.startIndex..<colon]
        guard line.index(after: colon) < line.endIndex else { return false }
        guard line[line.index(after: colon)] == " " else { return false }
        return key.allSatisfy { $0.isLetter || $0 == "-" }
    }

    /// How long ago this commit was made, in the same shorthand a branch row
    /// and a running turn already use: `12m`, `3h`, `2d`.
    ///
    /// `now` is an ARGUMENT rather than a `Date()` read in here, for the reason
    /// `Terminal.turnDuration` spells out — a clock read inside a property is
    /// an input SwiftUI cannot observe, so the string freezes until something
    /// unrelated forces a redraw. Nothing drives this one on a timer, and
    /// nothing needs to: the picker computes `now` when it opens, which is the
    /// only moment anybody reads it.
    func age(at now: Date) -> String {
        let seconds = now.timeIntervalSince1970 - Double(timestamp)
        // A commit dated in the future is a clock that disagrees, not a
        // negative age. Saying "now" is the smallest honest thing to say.
        guard seconds >= 60 else { return "now" }
        if seconds < 3600 { return "\(Int(seconds / 60))m" }
        if seconds < 86_400 { return "\(Int(seconds / 3600))h" }
        return "\(Int(seconds / 86_400))d"
    }

    /// The full date, for the tooltip. `2d` is what you scan; this is what you
    /// check when `2d` is the thing you are trying to remember. It takes no
    /// clock because it is not relative to one.
    var made: String {
        Date(timeIntervalSince1970: Double(timestamp))
            .formatted(date: .abbreviated, time: .shortened)
    }
}

struct ChangedFile: Decodable, Equatable, Identifiable {
    var path: String
    var status: ChangedFileStatus?
    var oldPath: String?
    var insertions: Int
    var deletions: Int
    var binary: Bool

    var id: String { path }
    var name: String { (path as NSString).lastPathComponent }

    /// Whether a tool wrote this file rather than a person or an agent.
    ///
    /// The rule and all of its reasoning live in `GeneratedFile.isGenerated`,
    /// which is shared with the phone — including the part that matters most,
    /// that this belongs on the host and is a stopgap until it gets there. The
    /// phone had it first and had it alone; two clients deciding separately
    /// what a lockfile is would be two answers to what a branch changed, on
    /// the two screens most likely to be open at once.
    var isGenerated: Bool { GeneratedFile.isGenerated(path) }

    enum CodingKeys: String, CodingKey {
        case path, status, insertions, deletions, binary
        case oldPath = "old_path"
    }
}

enum ChangedFileStatus: String, Decodable {
    case added, modified, deleted, renamed, copied
    case typeChanged = "type_changed"
    case untracked, conflicted

    var mark: String {
        switch self {
        case .added, .untracked: return "A"
        case .modified: return "M"
        case .deleted: return "D"
        case .renamed: return "R"
        case .copied: return "C"
        case .typeChanged: return "T"
        case .conflicted: return "!"
        }
    }

    var label: String {
        switch self {
        case .added: return "Added"
        case .modified: return "Modified"
        case .deleted: return "Deleted"
        case .renamed: return "Renamed"
        case .copied: return "Copied"
        case .typeChanged: return "Type changed"
        case .untracked: return "Untracked"
        case .conflicted: return "Conflicted"
        }
    }
}

/// What `changes files <workspace> <sha> --json` answers.
///
/// A wrapper around one list because the CLI prints an object rather than a
/// bare array, which is what `changes.commit_files` hands the phone through the
/// FFI too. One shape for both clients is the point: the file rows here are the
/// same `ChangedFile` the change set's own files decode into.
struct CommitFiles: Decodable, Equatable {
    var files: [ChangedFile]
}

/// What `changes diff <workspace> <path> --json` answers: one file's patch, and
/// the things about it that are not lines of patch.
///
/// ## Why this is not read out of the human output any more
///
/// It was, until `c2f1117` gave the command a `--json` at all, and the three
/// fields below `lines` died in that pipe — the human output states them as
/// prose a reader understands and a parser does not. Losing them is worse than
/// a missing feature: an empty hunk list with no reason attached reads as
/// "nothing changed", so this pane told people a submodule was unchanged, which
/// is the one thing it could not have been. `DaemonClient.parseUnified` is
/// still here and still parses that output, for a runner too old to answer in
/// JSON; what it cannot do is answer any of these three questions.
///
/// ## The shape
///
/// Built by `farcooler_client::changes_json::file_diff_json`, which is the same
/// function the phones reach over the FFI — so this decodes the same bytes
/// `apps/ios/FarCooler/Changes.swift` decodes, and the two clients cannot
/// disagree about what a hunk is.
///
/// Hunks are flattened on the way in, exactly as the phone flattens them: this
/// pane draws one list and finds its own gaps from the jump between two line
/// numbers — see `ChangesPane.body(of:lines:)`, which does not want `@@`
/// headers and would have to throw them away again.
struct FileDiff {
    /// Every line of the patch, hunk boundaries gone.
    var lines: [DiffComputation.Line] = []

    /// Why there are no lines, when the reason is not "nothing changed":
    /// `binary`, `submodule`, `combined_diff` or `malformed`.
    ///
    /// A `String` rather than an enum, matching the phone, so a code this
    /// version has never heard of degrades to one unrecognized reason instead
    /// of failing the decode and taking the whole patch with it.
    var unsupported: String?

    /// Hunks were left out: this is not the whole patch.
    ///
    /// The protocol is blunt about why it travels — "a client showing a
    /// truncated diff must say so; one that does not is claiming the rest of
    /// the file is unchanged". This client could not say so, because it could
    /// not see it.
    var truncated = false

    /// The patch is one parent's view of a merge.
    ///
    /// Not an error and not a truncation — the lines are real. What it changes
    /// is what they MEAN, which is why it is drawn above them rather than
    /// instead of them.
    var firstParentOfMerge = false

    /// What to say in place of the lines, when there are none for a reason.
    ///
    /// Nil when the file is genuinely unchanged, which is the case the pane
    /// still answers with "No textual changes".
    ///
    /// The four sentences are the phone's own, verbatim from `ChangesStore.reason`
    /// in `apps/ios/FarCooler/Changes.swift`: the situation is identical on both
    /// screens, and a Mac that phrased a submodule differently would be two
    /// products describing one fact.
    var unsupportedNote: String? {
        switch unsupported {
        case nil: return nil
        case "binary": return Self.binaryNote
        case "submodule": return "Submodule"
        case "combined_diff": return Self.mergeNote
        default: return "This patch could not be read"
        }
    }

    /// One parent's view of a merge, said the same way whether the patch came
    /// with it or not.
    ///
    /// `combined_diff` means the daemon refused a merge's combined patch — its
    /// lines carry a column per parent and an ordinary parser reads them as
    /// nonsense — and showed nothing. `firstParentOfMerge` means it made the
    /// same decision and had a patch to show for it. The fact a reader needs is
    /// identical, so the sentence is, and it lives in one constant so it cannot
    /// quietly become two.
    static let mergeNote = "A merge commit, shown against its first parent"

    /// What a binary file says in this pane.
    ///
    /// The phone says "Binary file". This pane has said this longer, from the
    /// change set's own `binary` flag, back when that was the only way it could
    /// know — so the two arms share the sentence the pane already had rather
    /// than the pane growing a second one for the same fact. That is the drift
    /// `c2f1117` just deleted on the Rust side, and it is not worth reopening
    /// here to save four words.
    static let binaryNote = "Binary file — nothing to show"

    /// What a diff the daemon cut short says.
    ///
    /// The protocol's own instruction, in a sentence: "a client showing a
    /// truncated diff must say so; one that does not is claiming the rest of
    /// the file is unchanged". So this says the opposite of that claim, out
    /// loud, rather than naming which cap was hit — the caps are the daemon's
    /// business and the reader's question is only whether they are looking at
    /// all of it.
    static let truncatedNote = "Truncated — the rest of this file isn’t shown"
}

extension FileDiff: Decodable {
    private enum CodingKeys: String, CodingKey {
        case hunks, unsupported, truncated, firstParentOfMerge
    }

    private struct WireHunk: Decodable {
        var lines: [WireLine]
    }

    private struct WireLine: Decodable {
        var kind: String
        var oldNumber: Int?
        var newNumber: Int?
        var text: String
    }

    /// `hunks` is required and everything else is not, and that asymmetry is
    /// load-bearing: a missing `hunks` is how `DaemonClient.changesDiff` learns
    /// it is talking to a runner too old to have printed JSON at all, and falls
    /// back to reading the same bytes as human output. See there.
    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let hunks = try c.decode([WireHunk].self, forKey: .hunks)
        unsupported = try c.decodeIfPresent(String.self, forKey: .unsupported)
        truncated = try c.decodeIfPresent(Bool.self, forKey: .truncated) ?? false
        firstParentOfMerge = try c.decodeIfPresent(Bool.self, forKey: .firstParentOfMerge) ?? false

        var out: [DiffComputation.Line] = []
        for hunk in hunks {
            for line in hunk.lines {
                out.append(
                    DiffComputation.Line(
                        id: out.count,
                        kind: {
                            switch line.kind {
                            case "added": return .added
                            case "removed": return .removed
                            default: return .context
                            }
                        }(),
                        oldNumber: line.oldNumber,
                        newNumber: line.newNumber,
                        text: line.text))
            }
        }
        lines = out
    }
}

struct WorkingTree: Decodable, Equatable {
    var staged: [String]
    var unstaged: [String]
    var untracked: [String]
    var conflicted: [String]
    var changes: [WorkingTreeFile]?
}

/// One dirty path, in one of the groups git puts it in.
///
/// A file that is staged AND modified again appears twice, once per group,
/// because those are two different diffs of it — so the counts here are summed
/// per path rather than looked up, and `files` does that.
///
/// The counts are decoded leniently because a runner can be behind this app: the
/// daemon wrote zeroes into these fields for as long as they existed and only
/// fills them now, and an app that refused to decode a change set without them
/// would show an older runner nothing at all rather than a number less.
struct WorkingTreeFile: Decodable, Equatable {
    var path: String
    var status: ChangedFileStatus
    var oldPath: String?
    var insertions: Int
    var deletions: Int
    var binary: Bool

    enum CodingKeys: String, CodingKey {
        case path, status, insertions, deletions, binary
        case oldPath = "old_path"
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        path = try c.decode(String.self, forKey: .path)
        status = try c.decode(ChangedFileStatus.self, forKey: .status)
        oldPath = try c.decodeIfPresent(String.self, forKey: .oldPath)
        insertions = try c.decodeIfPresent(Int.self, forKey: .insertions) ?? 0
        deletions = try c.decodeIfPresent(Int.self, forKey: .deletions) ?? 0
        binary = try c.decodeIfPresent(Bool.self, forKey: .binary) ?? false
    }
}

/// One worktree's line in the fleet's changed-since-you-looked list.
///
/// This used to carry counts of open and answered comments. Those came from the
/// review buffer, which is gone; what is left is the question the sidebar
/// actually asks.
///
/// It did not ask all of it. `changedSinceReviewed` was decoded here from the
/// beginning and read by nothing on this platform for as long — the sidebar
/// drew the counts and ignored the watermark, so a worktree you had finished
/// reading looked exactly like one you had not. The phone had been gating both
/// its Needs You list and its widget count on this pair the whole time
/// (`FleetView.rule(for:inbox:)`), which is why the drift showed up as the two
/// clients disagreeing about which worktrees still wanted you rather than as
/// anything visibly wrong here.
struct InboxRow: Decodable, Equatable, Identifiable {
    /// The worktree this row is about, as a full UUID — or, from a runner older
    /// than the shape unification, as the eight-character short.
    ///
    /// One key that used to have two meanings, which is why `short` exists
    /// below and why nothing here keys a dictionary off this field directly.
    /// `farcooler changes inbox --json` sent the SHORT under this name and the
    /// FFI the phones read sent the UUID, so the Mac's inbox map and the
    /// phones' were keyed by different things for the same call. Both producers
    /// are `changes_json::inbox_json` now and both send the UUID here.
    var workspaceId: String
    /// The eight characters a person types, sent alongside the UUID.
    ///
    /// Optional because a runner can be older than this app: before the two
    /// builders became one there was no `short` key, and the short was in
    /// `workspaceId` instead. `DaemonClient.refreshChangesInbox` prefers this
    /// and falls back, so an old runner keeps working rather than dropping every
    /// count out of the sidebar.
    var short: String?
    var changedSinceReviewed: Bool
    var insertions: Int
    var deletions: Int

    var id: String { workspaceId }
    var hasDiff: Bool { insertions > 0 || deletions > 0 }

    enum CodingKeys: String, CodingKey {
        case workspaceId = "workspace_id"
        case short, insertions, deletions
        case changedSinceReviewed = "changed_since_reviewed"
    }
}

/// The inbox as a whole, and the two envelopes it has arrived in.
///
/// `changes inbox --json` prints `{"items": [...], "elsewhere": n}` now, the
/// same object the FFI hands the phones. It used to print a bare array, so a
/// Mac talking to a runner that has not been updated still gets one — and a
/// decoder that only knew the object would empty every diff count in the
/// sidebar the moment it met an older runner, silently, since the failure path
/// here is `return`.
///
/// `elsewhere` is deliberately not read. The daemon hard-codes it to zero in
/// `review_ops::inbox` with no path that sets it, so drawing it would put a
/// number on screen that is zero by construction; Android records the same
/// waiting reader in its own `InboxReply`. Decoded here so that the key is
/// accounted for rather than merely ignored.
struct InboxReply: Decodable {
    var items: [InboxRow]
    var elsewhere: Int?

    /// The rows, whichever envelope they came in.
    static func rows(from data: Data) -> [InboxRow]? {
        let d = JSONDecoder()
        if let reply = try? d.decode(InboxReply.self, from: data) { return reply.items }
        return try? d.decode([InboxRow].self, from: data)
    }
}

/// Which comparison the diff tile is showing.
///
/// `commit` used to be absent, on the grounds that it needed a picker the PR
/// tile would have to draw first. That tile never arrived and the wait was
/// unnecessary: the change set has carried `commits` the whole time — decoded
/// here since the beginning and rendered by nothing — so the picker had nothing
/// left to guess at. It lives in the diff strip, beside the file jumper.
///
/// The sha is NOT an associated value of this case, and that is the one
/// decision here worth defending. This enum is the tag type of the segmented
/// comparison control in the pane header, which needs `allCases`, a stable
/// `rawValue` and `Codable`; a payload takes all three away and rewrites a
/// control that is not this feature's to rewrite. Which commit is being shown
/// is a second question, so it is a second property — `ChangesStore.selectedCommit`.
enum DiffScope: String, CaseIterable, Identifiable, Codable {
    case branch
    case local
    case commit

    /// The two the comparison control offers, which is not every case.
    ///
    /// A `Commit` segment would be a control that cannot answer its own
    /// question: clicking it says nothing about WHICH commit, so it would have
    /// to open something, and the thing it would open is already one control to
    /// the right. While a commit is on screen this control shows no segment
    /// selected — the truth of it, since neither the whole branch nor the
    /// uncommitted work is what is being drawn — and clicking either segment is
    /// one of the two ways back.
    static var allCases: [DiffScope] { [.branch, .local] }

    var id: String { rawValue }

    var label: String {
        switch self {
        case .branch: return "Branch"
        case .local: return "Uncommitted"
        case .commit: return "Commit"
        }
    }
}

/// What the end of a file list means, which is not the same question in every
/// scope.
enum DiffBoundary: Equatable {
    /// Round the list. What Branch and Uncommitted have always done and go on
    /// doing: there is nothing beyond a scope that is already everything, and a
    /// Next that did nothing at the last file would be a control that stops
    /// working without saying why.
    case wrap
    /// Carry on into this commit. Only while one commit is on screen, and only
    /// while the branch has another one in that direction.
    case handOff(String)
    /// The end of the branch, in that direction. Nothing to wrap to, because
    /// wrapping from the last file of the last commit to the first file of the
    /// same commit is a loop pretending to be progress.
    case stop
}

/// What one press of a movement control does, decided before anything moves.
enum DiffStep: Equatable {
    case hunk(String)
    /// An index into the reading order — see `ChangesStore.reviewOrder`.
    case file(Int)
    case commit(String)
    case stay
}

/// Where Next and Previous go.
///
/// The arithmetic lived in two methods on the pane until the commit walk
/// arrived, and it had to come out for one reason: hunk movement FALLS THROUGH
/// to file movement at both ends of a file, so changing what the end of a file
/// list means silently changes hunk navigation too. Neither behavior was
/// observable from a test while the decision was made inside a SwiftUI view,
/// and the wrap this replaces is behavior two of the three scopes still rely
/// on.
///
/// Deliberately knows nothing about a store, a scope or a view: it is handed
/// how many files there are, which one is open, which hunks that file has, and
/// what the end of the list means. See `ChangesPane.boundary(_:)` for who
/// decides the last of those.
enum DiffWalk {
    /// What running out of files in this direction means, given what is on
    /// screen and what the branch has either side of it.
    ///
    /// The policy, separated from the arithmetic below so it can be read and
    /// tested on its own — it is the one thing this change actually decides.
    static func boundary(
        _ direction: Int, scope: DiffScope, next: ChangeCommit?, previous: ChangeCommit?
    ) -> DiffBoundary {
        guard scope == .commit else { return .wrap }
        guard let neighbor = direction > 0 ? next : previous else { return .stop }
        return .handOff(neighbor.sha)
    }

    /// `direction` is +1 or −1. `hunks` is empty for a plain file move, which
    /// is what makes one function serve both controls.
    static func step(
        _ direction: Int,
        hunks: [String],
        after lastHunk: String?,
        files: Int,
        at current: Int?,
        boundary: DiffBoundary
    ) -> DiffStep {
        if !hunks.isEmpty {
            if let lastHunk, let index = hunks.firstIndex(of: lastHunk) {
                let next = index + direction
                if hunks.indices.contains(next) { return .hunk(hunks[next]) }
                // Off the end of this file's hunks, so this becomes a file
                // move. The fall-through IS the feature: reading a diff is one
                // continuous downward motion, and a control that stopped at the
                // last hunk of every file would need a second control beside it
                // to get past each one.
            } else {
                // No hunk visited in this file yet — either it was just opened
                // or the last hunk belonged to a file that is no longer on
                // screen. Enter it from the end the reader is traveling
                // towards.
                return .hunk(direction > 0 ? hunks[0] : hunks[hunks.count - 1])
            }
        }
        return fileStep(direction, files: files, at: current, boundary: boundary)
    }

    private static func fileStep(
        _ direction: Int, files: Int, at current: Int?, boundary: DiffBoundary
    ) -> DiffStep {
        guard files > 0 else {
            // A commit that changed nothing is one you walk THROUGH, not one
            // the walk ends in.
            if case .handOff(let sha) = boundary { return .commit(sha) }
            return .stay
        }
        // No file open yet: Next means the first, Previous means the last. The
        // −1 is what makes `-1 + 1` the first file rather than the second.
        let from = current ?? (direction > 0 ? -1 : 0)
        let next = from + direction
        if (0..<files).contains(next) { return .file(next) }
        switch boundary {
        case .wrap: return .file((next + files) % files)
        case .handOff(let sha): return .commit(sha)
        case .stop: return .stay
        }
    }
}

// MARK: - Where a review note can be sent

extension Workspace {
    /// The panes in this worktree a review note can be handed to.
    ///
    /// `isAgentPane` OR `canSwitchPaneMode`, because both are the daemon's word
    /// for "an agent is in here" and only the first is about what is currently
    /// DRAWN. A claude the reader has flipped back to its raw terminal is still
    /// an agent holding an ACP session, and `terminal agent-prompt` reaches it;
    /// excluding it would mean a review with nowhere to send to for the sole
    /// reason that somebody wanted to watch the tty.
    ///
    /// The phone has this filter too and the two are deliberately NOT one
    /// function: `Workspace` and `Terminal` are declared once per app, and this
    /// app's `canSwitchPaneMode` says `&& !isChangesPane` where the phone's
    /// does not — because a Mac can put a diff in a pane and the daemon refuses
    /// to switch that one. `ReviewAgentTarget` is shared; the two meanings of
    /// "can this pane hold a chat" are not the same fact.
    ///
    /// `short`, not `id`, because that is what a command line takes — the same
    /// identifier `AgentStream` passes to `terminal agent-prompt` for a typed
    /// message. The phone's targets carry its FFI's id for the same reason.
    func reviewAgentTargets() -> [ReviewAgentTarget] {
        let numbering = ordinals()
        return terminals
            .filter { $0.isAgentPane || $0.canSwitchPaneMode }
            .map {
                ReviewAgentTarget(
                    id: $0.short, name: $0.displayName(ordinal: numbering[$0.id]),
                    showsChat: $0.isAgentPane)
            }
    }
}

/// What one worktree changed, and the file currently open.
@MainActor
final class ChangesStore: ObservableObject {
    @Published var changeSet: ChangeSet = .empty

    /// Every file's diff, by path, as it is read.
    ///
    /// The whole branch is one scroll, so this holds more than the file being
    /// looked at — but it is filled a file at a time, as each scrolls into view,
    /// rather than up front. A branch that changed forty files would otherwise
    /// pay forty round trips before it drew anything, to show a screenful.
    ///
    /// The whole `FileDiff` and not just its lines, because "no lines" is four
    /// different answers and this pane used to give one of them to all four.
    @Published var fileDiffs: [String: FileDiff] = [:]

    /// Files currently being read, so a section can say so rather than look
    /// empty — an unread file and a file with no hunks are not the same thing.
    @Published var loadingFiles: Set<String> = []

    /// The longest line read so far, in characters.
    ///
    /// The diff's width is set from this rather than measured. A scroll view
    /// that has to work out how wide its content is asks every row, which makes
    /// a lazy stack build all of them — and that is what hung the whole app when
    /// a horizontal scroll wrapped the file list. The font is monospaced, so
    /// characters times one advance is the exact answer without asking anybody.
    @Published private(set) var widestLine = 0

    /// The same files read again with enough context to fill the gaps between
    /// their hunks. Only for files somebody asked to expand.
    @Published var fullDiffs: [String: [DiffComputation.Line]] = [:]

    /// Which gaps have been opened, as `path#index`.
    ///
    /// Per gap rather than per file, because that is what is being asked: a
    /// four-thousand-line file with six hunks has six separate questions in it,
    /// and answering all of them because one was asked buries the change under
    /// the code that did not change — which is the thing a diff exists to
    /// avoid.
    @Published var openGaps: Set<String> = []

    /// Files folded down to their heading. Kept in the store so a tmux layout
    /// switch does not reopen a generated lockfile the user deliberately hid.
    @Published var collapsedFiles: Set<String> = []

    /// Where the scroll was, held OUTSIDE the view.
    ///
    /// A pane belongs to a tmux layout, and switching layouts tears its view
    /// down — so the scroll started at the top every time you came back, which
    /// on a nine-thousand-line diff meant losing your place for looking at
    /// another arrangement for a moment. The store outlives the view, so this
    /// does too.
    ///
    /// Where the scroll was, as an offset the view puts back when it returns.
    ///
    /// The rows are in a LAZY stack, and that is the whole difficulty: when
    /// this pane reappears only a screenful of rows exists, the scroll view's
    /// content is only that tall, and a request to scroll nine thousand lines
    /// down is clamped to the bottom of what has been built. One scroll cannot
    /// get there. So the view asks repeatedly — see `ChangesPane.restore()` —
    /// and each attempt builds more rows for the next one to use.
    ///
    /// A row identity was tried first and is worse, not better. Anchoring to a
    /// row is the operation a lazy stack is supposed to be good at, but the
    /// only rows this view can name are file headings, and the reader is
    /// usually in the middle of a long file whose heading scrolled off screens
    /// ago — unrealized, unreported, and so unavailable to anchor to. It put
    /// every return at the top of whatever file was showing.
    ///
    /// Deliberately NOT `@Published`: it changes on every frame of a scroll,
    /// and republishing that would rebuild the diff under the hand doing the
    /// scrolling.
    var scrollTarget: CGPoint = .zero

    /// The handle used to put the scroll back. Held here for the same reason
    /// the anchor is — the view that would otherwise own it does not survive a
    /// layout switch.
    ///
    /// Not `@Published`, and that matters: the scroll view writes back into
    /// this binding on every frame of a scroll, so publishing it would rebuild
    /// the whole diff — nine thousand rows of it — once per frame while
    /// somebody drags. `$changes.scrollPosition` works regardless; a binding
    /// into an `ObservableObject` does not need the property to be published,
    /// it needs the object to be observed.
    var scrollPosition = ScrollPosition(idType: String.self)

    /// The file the jump bar last went to. Highlights it; hides nothing.
    @Published var selectedFile: String?
    @Published var scope: DiffScope = .branch {
        didSet {
            // Leaving the commit view forgets WHICH commit, because every way
            // out of it means the same thing: the reader is done with that
            // commit. Without this, clicking `Branch` in the header control —
            // which this file does not own and cannot ask to clear anything —
            // would leave a sha behind, and the next commit chosen from the
            // picker would be silently compared against a stale file list.
            guard scope != .commit, oldValue == .commit else { return }
            selectedCommit = nil
            commitFiles = []
            commitUnreadable = false
        }
    }
    @Published var loading = false
    @Published var error: String?

    /// Which commit is on screen, when the scope is `.commit`. Nil otherwise,
    /// and kept nil by `scope`'s own `didSet` rather than by every caller.
    @Published private(set) var selectedCommit: String?

    /// What that commit touched, from `changes files` — a separate call,
    /// because the change set deliberately carries no per-commit file lists.
    @Published private(set) var commitFiles: [ChangedFile] = []

    /// Set when that call failed, so the pane can say which nothing it is
    /// showing. A commit can genuinely stop being readable while the pane is
    /// open — a rebase or an amend rewrites the branch under it — and "Nothing
    /// changed here" is the wrong sentence for a commit that could not be read.
    @Published private(set) var commitUnreadable = false

    /// Bumped every time the cached diffs are thrown away.
    ///
    /// Two jobs, both of which were bugs before it existed. It is part of the
    /// id each file heading's `task` is keyed on, so a reload re-asks for the
    /// files on screen: keyed on the path alone, a heading that stayed realized
    /// across a scope change never ran its task again, and the file sat at `…`
    /// forever because nothing else asks. And every read compares the
    /// generation it started in against the current one before storing what it
    /// got, so a diff still in flight when the reader switches commits is
    /// dropped rather than filed under the new commit's identical path.
    @Published private(set) var generation = 0

    let client: DaemonClient
    let workspace: Workspace

    /// What the reader wants to tell the agent, collected across the review.
    ///
    /// A separate object rather than more `@Published`s here, because its
    /// lifetime is different in the way that matters: everything else on this
    /// store is derived from the daemon and can be thrown away and re-read,
    /// while a comment is the only thing in this pane that a person typed and
    /// that nothing else in the world has a copy of. It persists itself; see
    /// `ReviewCommentQueue`.
    let comments: ReviewCommentQueue

    init(client: DaemonClient, workspace: Workspace) {
        self.client = client
        self.workspace = workspace
        // Keyed by worktree, which is what is being reviewed — so two changes
        // panes in one layout share a queue rather than writing two, and so a
        // runner that drops and comes back finds the notes still there. That
        // second case is not hypothetical: `FleetStore` builds a fresh
        // `DaemonClient` when a runner returns and `ContentView` therefore
        // builds a fresh store, which would otherwise be an empty outbox.
        comments = ReviewCommentQueue(workspace: workspace.id) { target, text in
            guard let message = await client.agentPrompt(terminal: target.id, text: text)
            else { return nil }
            return Self.trouble(saying: message)
        }
    }

    /// The CLI's failure, as something worth putting beside the notes it kept.
    ///
    /// One sentence this app wrote, with `farcooler`'s own words underneath
    /// rather than as the sentence — the rule `ChangesPane.problem` already
    /// keeps for the same reason: stderr set as body text under a heading the
    /// app wrote is the app appearing to say it. The phone reads its FFI's
    /// error more finely because what IT gets back is a Rust enum's
    /// description, which is not a sentence at all; a CLI's stderr already is
    /// one, and paraphrasing it would be this app guessing at a diagnosis it
    /// was handed.
    private static func trouble(saying message: String) -> ReviewTrouble {
        let words = message.trimmingCharacters(in: .whitespacesAndNewlines)
        return ReviewTrouble(
            sentence: "Couldn’t send these. They’re still here.",
            transcript: words.isEmpty ? nil : words)
    }

    /// The files this scope is about.
    ///
    /// Branch is what the branch COMMITTED — `git diff base…HEAD` — and its
    /// per-file counts are what the header above this list sums.
    ///
    /// That used to be the same number the sidebar draws, and is not any more:
    /// `7927c13` moved `change_set::shortstat` to compare the base against the
    /// WORKING TREE and taught it to count untracked lines, so a dirty
    /// worktree's row reads higher than its own panel. Both numbers are honest
    /// about what they count and nothing has been decided about which one this
    /// header should be.
    ///
    /// Local is what has not been committed yet, and it used to be neither. The
    /// scope switched which diff each FILE showed while the file LIST stayed
    /// the branch's committed one — so a file an agent had just written, the
    /// only kind of file Local exists to show, never appeared in the list at
    /// all, and the files that did appear showed an empty local diff. The
    /// working tree already travels in the same response; this reads it.
    ///
    /// Commit is one commit against its FIRST parent, which is what the daemon
    /// diffs and what `changes files` counts — never `git show`, whose combined
    /// diff for a merge an ordinary parser reads as nonsense. Both halves of
    /// this view come from that same first-parent comparison, so the file list
    /// and the patches under it cannot disagree about what the commit did.
    var files: [ChangedFile] {
        switch scope {
        case .branch:
            return changeSet.files
        case .commit:
            return commitFiles
        case .local:
            return dirtyPaths.map { path in
                // Read from the working tree the daemon sent, not counted from
                // the diffs on screen.
                //
                // Counting the hunks was right about COST and wrong about
                // correctness: a per-file round trip to say what is already
                // drawn would be absurd, but the hunks only contain the number
                // once they have been fetched, and `fileDiffs` fills lazily from
                // a `.task(id:)` on each realized row. So the header started
                // near zero on a worktree long enough to scroll and climbed as
                // the reader went down it, and every scope switch and every
                // Refresh emptied the cache and started over. There is no round
                // trip here either: `apply_uncommitted_counts` fills these in
                // the same `change_set` this pane already read.
                //
                // Summed rather than found, because a file that is staged and
                // then modified again is in both groups with its own diff in
                // each.
                let counts = changeSet.workingTree?.changes?.filter { $0.path == path } ?? []
                return ChangedFile(
                    path: path,
                    status: localStatus(path),
                    oldPath: nil,
                    insertions: counts.reduce(0) { $0 + $1.insertions },
                    deletions: counts.reduce(0) { $0 + $1.deletions },
                    binary: counts.contains { $0.binary })
            }
        }
    }

    /// The files in reading order: the one order the pane draws AND the one
    /// order Next and Previous walk.
    ///
    /// One property rather than a sort inside the view. A position counts the
    /// list the reader is looking at, so a display order that differed from the
    /// navigation order would make Next jump backwards up the screen — and the
    /// phone had already learned where that bites: it puts generated files last
    /// precisely so Next never drops somebody into the middle of a regenerated
    /// lockfile eleven files before the end.
    ///
    /// This is the seam `1c81111` named and left returning `files` unchanged,
    /// against the day the Mac learned what a generated file is. It has, so the
    /// rule lands here and the movement arithmetic was never re-opened —
    /// `moveFile`, `moveHunk` and the hand-off between commits all walk this
    /// property and none of them had to know. `GeneratedFile.reviewOrder` is a
    /// stable partition, so a comparison with no lockfile in it is still
    /// `files`, byte for byte, which is what makes this inert on most branches.
    ///
    /// It reorders `.local` too, and that is deliberate rather than incidental:
    /// a regenerated lockfile is exactly as unhelpful to walk into before it is
    /// committed as after. Inside each group `files`' own order survives —
    /// staged, then unstaged, then untracked, each alphabetical.
    var reviewOrder: [ChangedFile] { GeneratedFile.reviewOrder(files, path: \.path) }

    /// The two groups themselves, for the counts that hold them apart.
    ///
    /// Split rather than filtered away: a lockfile that moved is a real fact
    /// about a branch, and hiding it would be this pane deciding what the
    /// reader is allowed to see. Only its SIZE is misleading — see
    /// `GeneratedFile.isGenerated` — so both are listed and only the numbers
    /// separate.
    var generatedFiles: [ChangedFile] { files.filter(\.isGenerated) }
    var handWrittenFiles: [ChangedFile] { files.filter { !$0.isGenerated } }

    /// What a person or an agent wrote, added up, for the pane's headline.
    ///
    /// Summed from the file list rather than taken from the change set, which
    /// is the only way to get it: `changeSet.insertions` is the whole
    /// comparison and the lockfile inside it is the number this split exists to
    /// stop showing. See `TileView.changeCount`, which is the one caller and
    /// reaches for these only when there is something generated to hold apart.
    var writtenInsertions: Int { handWrittenFiles.reduce(0) { $0 + $1.insertions } }
    var writtenDeletions: Int { handWrittenFiles.reduce(0) { $0 + $1.deletions } }
    var generatedInsertions: Int { generatedFiles.reduce(0) { $0 + $1.insertions } }
    var generatedDeletions: Int { generatedFiles.reduce(0) { $0 + $1.deletions } }

    /// Whether git has never seen this file.
    ///
    /// It has no diff and cannot be given one: `git diff` compares against
    /// something recorded, and there is nothing recorded for a file that was
    /// only just written. It still belongs in the list — a file an agent
    /// created is the most interesting thing in a local change set — so the
    /// row says which kind of nothing it is showing.
    func isUntracked(_ path: String) -> Bool {
        // Nothing in a commit is untracked — committing is what tracking IS —
        // and the working tree's list is about right now, not about then. Left
        // to speak, it would refuse to fetch the diff of a file that a commit
        // changed and somebody has since deleted and rewritten, and the row
        // would claim git had never seen a file the commit demonstrably had.
        guard scope != .commit else { return false }
        return changeSet.workingTree?.untracked.contains(path) ?? false
    }

    /// Every path git calls dirty, in one order: staged, then unstaged, then
    /// untracked, each alphabetical, and never twice — a file can be staged and
    /// modified again, and it is one file.
    private var dirtyPaths: [String] {
        guard let w = changeSet.workingTree else { return [] }
        var seen = Set<String>()
        return (w.staged + w.unstaged + w.conflicted + w.untracked).filter { seen.insert($0).inserted }
    }

    private func localStatus(_ path: String) -> ChangedFileStatus {
        guard let tree = changeSet.workingTree else { return .modified }
        if tree.conflicted.contains(path) { return .conflicted }
        if tree.untracked.contains(path) { return .untracked }
        return tree.changes?.first(where: { $0.path == path })?.status ?? .modified
    }

    /// Whether this store has ever read the worktree.
    private var hasLoaded = false

    /// Read it, but only the first time.
    ///
    /// The pane calls this every time it appears, and it appears every time its
    /// tmux layout is switched back to. `load()` throws away every file diff it
    /// holds — deliberately, since they are diffs against a base that may have
    /// moved — so calling it on each appearance meant glancing at another
    /// arrangement and coming back to an empty diff that had to fetch itself
    /// again, with every expanded gap closed and the scroll at the top. The
    /// poll below is what keeps this current; arriving is not new information.
    func loadIfNeeded() async {
        guard !hasLoaded else { return }
        await load()
    }

    func load(fresh: Bool = false) async {
        hasLoaded = true
        loading = true
        defer { loading = false }

        var args = ["changes", "status", workspace.short, "--json"]
        if fresh { args.append("--fresh") }
        if let data = await client.changesJSON(args) {
            changeSet = (try? JSONDecoder().decode(ChangeSet.self, from: data)) ?? .empty
            error = nil
        } else {
            // A failure is NOT an empty diff. Saying so was a real bug once: a
            // runner whose daemon predates this answered NOT_FOUND to every
            // call, and the pane drew a worktree with no changes.
            changeSet = .empty
            error = client.changesError
        }

        // Before the reset below, because everything after it asks `files`
        // what is on screen and, in this scope, `files` IS this list.
        if scope == .commit, let sha = selectedCommit {
            await readCommitFiles(sha)
        }

        reset()
        if let open = selectedFile, !files.contains(where: { $0.path == open }) {
            selectedFile = nil
        }
        let live = Set(files.map(\.path))
        collapsedFiles = collapsedFiles.intersection(live)
    }

    /// Throw away every diff read so far, and say so.
    ///
    /// Read again rather than kept: these are diffs against a base that may
    /// have just moved, and a stale hunk is worse than a missing one. The
    /// expanded gaps go with them for the same reason — context recovered from
    /// a diff that no longer exists is not context.
    ///
    /// The generation bump is what makes the emptying visible. Nothing in the
    /// view asks for a file's diff a second time on its own: the heading does
    /// it once, from a `task` keyed on a path that has not changed. So clearing
    /// the cache under a heading that is still on screen used to leave that
    /// file reading `…` until somebody scrolled it away and back — which is
    /// exactly what switching between two commits that touch the same file
    /// does.
    private func reset() {
        fileDiffs = [:]
        fullDiffs = [:]
        fullContext = [:]
        openGaps = []
        tooWide = []
        widestLine = 0
        generation &+= 1
    }

    /// Show one commit, against its first parent.
    ///
    /// The file list is a second call and not a filter over the branch's: the
    /// change set carries per-file counts for the WHOLE branch and nothing
    /// per-commit, on purpose — a branch that regenerated a lockfile touches
    /// thousands of files, and shipping that per commit to draw a picker is the
    /// cost this product does not pay.
    func select(commit sha: String) async {
        // Already showing it. Re-reading would cost a round trip to arrive at
        // the same list, and would take the reader's scroll position and every
        // gap they had opened with it.
        guard scope != .commit || selectedCommit != sha else { return }
        scope = .commit
        selectedCommit = sha
        // The same flag a full load raises, and for the same reason: arriving
        // from the branch, this scope's file list is empty until the call
        // below answers, and an empty list with nothing to say about why is a
        // pane asserting that the commit changed nothing.
        loading = true
        defer { loading = false }
        // Read BEFORE anything is thrown away, which is what makes going from
        // one commit to the next look like a switch rather than a reload: the
        // previous commit's files and patches stay on screen until there is a
        // whole new set to put in their place.
        await readCommitFiles(sha)
        guard selectedCommit == sha else { return }
        reset()
        selectedFile = nil
        collapsedFiles = []
    }

    /// Back to the whole branch: merge base to HEAD, every commit at once.
    ///
    /// No round trip. The change set never stopped being read — the poll keeps
    /// it current while the pane is open — so the branch's file list is already
    /// here and only the commit's patches have to go.
    func showWholeBranch() {
        guard scope == .commit else { return }
        scope = .branch
        reset()
        selectedFile = nil
    }

    /// Which files one commit touched, from
    /// `changes files <workspace> <sha> --json`.
    ///
    /// JSON rather than the human table, so a row arrives with the status git
    /// gave it. The table carries only the two counts and the path, and this
    /// pane badged every file in a commit with a dot for as long as that was
    /// the only thing it could ask for.
    private func readCommitFiles(_ sha: String) async {
        let data = await client.changesJSON(["changes", "files", workspace.short, sha, "--json"])
        // The reader moved on while this was in flight. Filing an older
        // commit's list under the newer one's sha is the whole reason this is
        // checked: both answers are well-formed, and the wrong one is
        // indistinguishable from the right one once it has landed.
        guard selectedCommit == sha, scope == .commit else { return }
        guard let data else {
            commitFiles = []
            commitUnreadable = true
            return
        }
        if let answer = try? JSONDecoder().decode(CommitFiles.self, from: data) {
            commitFiles = answer.files
            commitUnreadable = false
            return
        }
        // A runner whose `farcooler` predates `--json` on this command ignores
        // the flag and prints the table anyway. Reading it back is the
        // difference between such a runner showing a commit's files without
        // their letters — which is what it always did — and showing a warning
        // triangle where a file list used to be.
        guard let text = String(data: data, encoding: .utf8) else {
            commitFiles = []
            commitUnreadable = true
            return
        }
        let rows = Self.parseCommitFiles(text)
        // Not `rows.isEmpty` on its own: a commit really can touch nothing.
        // Text that parsed to nothing while carrying something is the case
        // that has to read as unreadable.
        commitFiles = rows
        let sawSomething = !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        commitUnreadable = rows.isEmpty && sawSomething
    }

    /// `+12    -3     some/path.swift`, one file per line.
    ///
    /// The fallback path only, for a runner too old to answer `changes files`
    /// in JSON. Fields are padded to a fixed width rather than delimited, so
    /// this reads the two counts off the front and takes ALL of the rest as the
    /// path — splitting on whitespace instead would lose the second half of
    /// every file whose name contains a space.
    ///
    /// A row from here carries no status because this format has none to carry,
    /// and `M` would be a guess that reads as a verdict: "Modified" beside a
    /// file the commit created. The badge says only that the file changed,
    /// which is the whole of what this text says.
    nonisolated static func parseCommitFiles(_ text: String) -> [ChangedFile] {
        var out: [ChangedFile] = []
        for raw in text.split(separator: "\n", omittingEmptySubsequences: true) {
            var rest = raw[...]
            guard rest.hasPrefix("+") else { continue }
            rest = rest.dropFirst()
            let insertions = rest.prefix(while: \.isNumber)
            rest = rest.dropFirst(insertions.count).drop(while: { $0 == " " })
            guard rest.hasPrefix("-") else { continue }
            rest = rest.dropFirst()
            let deletions = rest.prefix(while: \.isNumber)
            rest = rest.dropFirst(deletions.count).drop(while: { $0 == " " })
            let path = String(rest)
            guard !path.isEmpty else { continue }
            out.append(
                ChangedFile(
                    path: path,
                    status: nil,
                    oldPath: nil,
                    insertions: Int(insertions) ?? 0,
                    deletions: Int(deletions) ?? 0,
                    binary: false))
        }
        return out
    }

    /// The commits this branch made, newest first.
    ///
    /// The daemon logs them with `--reverse` so `changes status` reads as a
    /// story from the base forward, which is right for a transcript and wrong
    /// for a picker: the commit somebody wants to look at is almost always the
    /// one they just made.
    var commitsNewestFirst: [ChangeCommit] { changeSet.commits.reversed() }

    /// What the change set knows about the commit on screen, if it still knows
    /// anything. Nil for a commit that has left the branch — an amend or a
    /// rebase during a review does exactly that — while the diff itself stays
    /// readable, because the object it names is still in the repository.
    var selectedCommitInfo: ChangeCommit? {
        guard let selectedCommit else { return nil }
        return changeSet.commits.first { $0.sha == selectedCommit }
    }

    // MARK: - The branch, one commit at a time

    /// The commits in the order they are READ, which is the order they were
    /// made: base forward.
    ///
    /// The opposite of `commitsNewestFirst`, and both are right for what they
    /// are for. A picker is reached for with one commit in mind and it is
    /// almost always the newest, so that list leads with it. Working THROUGH a
    /// branch is a different activity: each commit is one intention, and an
    /// agent's intentions only make sense forwards — the third commit fixes
    /// what the second one introduced, and read backwards it is a repair to
    /// something that has not happened yet. `commits_since` already logs with
    /// `--reverse` for exactly this reading, so this is the wire's own order.
    var commitsInOrder: [ChangeCommit] { changeSet.commits }

    var commitIndex: Int? {
        guard let sha = selectedCommit else { return nil }
        return commitsInOrder.firstIndex { $0.sha == sha }
    }

    /// `3 of 12`, for the strip beside the commit picker.
    ///
    /// Shorter than the phone's `Commit 3 of 12` on purpose. The clock glyph
    /// and the sha immediately to its left already say what is being counted,
    /// and this strip has a file path and the hunk controls to fit as well; the
    /// chevrons' tooltips say the whole sentence.
    ///
    /// Nil for a commit the branch no longer lists, which an amend mid-review
    /// produces: it has no position in a sequence it is not in, and inventing
    /// one would be this strip's one claim that could be flatly wrong.
    var commitPositionLabel: String? {
        guard let commitIndex else { return nil }
        return "\(commitIndex + 1) of \(commitsInOrder.count)"
    }

    var nextCommit: ChangeCommit? {
        guard let commitIndex, commitIndex + 1 < commitsInOrder.count else { return nil }
        return commitsInOrder[commitIndex + 1]
    }

    var previousCommit: ChangeCommit? {
        guard let commitIndex, commitIndex > 0 else { return nil }
        return commitsInOrder[commitIndex - 1]
    }

    /// Begin the itinerary: the first commit the branch made.
    ///
    /// The way in that the history popover is not. A list is the right shape
    /// for "which one of these", and the wrong shape for "start at the
    /// beginning and keep going" — which for an agent-authored branch is the
    /// reading that makes it legible, and the reading the chevrons exist to
    /// support.
    func startAtFirstCommit() async {
        guard let first = commitsInOrder.first else { return }
        await select(commit: first.sha)
    }

    /// Move one commit along the branch, without going back out to the picker.
    ///
    /// This is what makes commit-by-commit a path rather than a detour: the
    /// reader finishes a commit's last file and the same control that has been
    /// moving them through files moves them to the next intention. Returning to
    /// a list between every commit is twelve extra trips on a branch of twelve,
    /// and a reason not to read it this way at all.
    func showNextCommit() async {
        guard let next = nextCommit else { return }
        await select(commit: next.sha)
    }

    func showPreviousCommit() async {
        guard let previous = previousCommit else { return }
        await select(commit: previous.sha)
    }

    /// Read one file's diff, if it has not been read already.
    ///
    /// Idempotent and safe to call from `onAppear` on every section, which is
    /// exactly how it is called: the scroll decides what gets read.
    func ensure(_ path: String) async {
        // An untracked file has nothing to fetch: git has no recorded version
        // to diff it against, so the call would spend a round trip to come back
        // empty — once per file, and again on every poll.
        guard !isUntracked(path) else { return }
        guard fileDiffs[path] == nil, !loadingFiles.contains(path) else { return }
        await read(path)
    }

    private func read(_ path: String) async {
        let asked = generation
        loadingFiles.insert(path)
        let answer = await diff(path)
        let lines = answer.lines
        loadingFiles.remove(path)
        // What was being compared changed while this was in flight, so these
        // lines answer a question nobody is asking any more. Keyed on the path
        // alone they would file perfectly, under a heading now showing a
        // different commit — a wrong diff that looks exactly like a right one.
        guard asked == generation else { return }
        fileDiffs[path] = answer
        // The expanded copy described the file as it was a moment ago, and its
        // line numbers no longer line up with the hunks around them. Dropped
        // rather than re-fetched: a gap somebody opened once is cheap to open
        // again, and showing the wrong lines in it is not.
        fullDiffs.removeValue(forKey: path)
        fullContext.removeValue(forKey: path)
        openGaps = openGaps.filter { !$0.hasPrefix("\(path)#") }
        tooWide = tooWide.filter { !$0.hasPrefix("\(path)#") }
        widestLine = max(widestLine, lines.map(\.text.count).max() ?? 0)
    }

    /// One file's patch, for whatever is being compared.
    ///
    /// Every scope goes through the client's own `changesDiff`, including the
    /// commit one — it takes the sha as a parameter beside the scope, because
    /// `DiffScope` is the tag of a segmented control and cannot carry an
    /// associated value without losing `CaseIterable`.
    ///
    /// This briefly assembled the commit invocation itself and went out through
    /// `changesJSON`, which republishes the client's error state as a side
    /// effect of asking for a patch — so a per-file read inside a commit
    /// repainted rather more than the file it was for.
    private func diff(_ path: String, context: Int = 0) async -> FileDiff {
        await client.changesDiff(
            workspace: workspace.short, path: path, scope: scope, context: context,
            commit: selectedCommit)
    }

    /// Open one of the gaps a diff leaves between its hunks.
    ///
    /// The lines are not invented and not cached from anywhere: git is asked
    /// for the same diff again with enough context to cover this gap, and the
    /// answer is kept so opening another gap in the same file usually costs
    /// nothing.
    ///
    /// Enough for THIS gap, not the whole file. Asking for the file — a context
    /// of fifty thousand — collapses every hunk into one, and one hunk the size
    /// of Cargo.lock is past the four-thousand-line cap the daemon puts on a
    /// single file's diff, so the answer came back with no hunks at all and the
    /// gap opened onto nothing. Asked for what the gap actually needs, the
    /// hunks stay separate and the total stays inside the cap.
    func open(gap: Int, of size: Int, in path: String) async {
        let need = min(size + 4, 2_000)
        if (fullContext[path] ?? 0) < need {
            let asked = generation
            // The lines alone. A gap is filled with context this file's own
            // hunks left out, and the three notices belong to the file — they
            // were answered once, by `read`, and re-filing them from a
            // wide-context re-read would say them twice.
            let lines = await diff(path, context: need).lines
            // The same in-flight check `read` makes, for the same reason: these
            // are the unchanged lines of a file as some OTHER comparison saw
            // it, and they would slot into the gap without looking wrong.
            guard asked == generation else { return }
            guard !lines.isEmpty else {
                // The daemon refused to render a diff that wide. Recorded, so
                // the row can say so rather than being a control that does
                // nothing when clicked.
                tooWide.insert("\(path)#\(gap)")
                return
            }
            fullDiffs[path] = lines
            fullContext[path] = need
            widestLine = max(widestLine, lines.map(\.text.count).max() ?? 0)
        }
        openGaps.insert("\(path)#\(gap)")
    }

    /// How much context each file's expanded copy was read with, so a wider gap
    /// knows it has to ask again.
    private var fullContext: [String: Int] = [:]

    /// Gaps the daemon would not render, so their rows can say why.
    @Published var tooWide: Set<String> = []

    /// The unchanged lines between two line numbers of a file, once it has been
    /// read with full context. Empty until then, which is the same as the gap
    /// not being open.
    func context(in path: String, after: Int, before: Int) -> [DiffComputation.Line] {
        guard let full = fullDiffs[path] else { return [] }
        return full.filter { line in
            guard let n = line.newNumber else { return false }
            return n > after && n < before
        }
    }

    /// Keep the diff current while somebody is looking at it.
    ///
    /// A poll rather than an event, because there is no event to have: the
    /// daemon can see a commit land through two stats, but a file an agent
    /// edited in place is invisible until something reads the worktree — and
    /// watching every worktree in the fleet for that, all the time, is the cost
    /// this product deliberately does not pay to keep a sidebar warm.
    ///
    /// So the one worktree being READ pays it, and only while it is on screen:
    /// the task this runs in is cancelled the moment the pane goes away. The
    /// read is deliberately not `fresh` — a non-fresh change set is answered
    /// from cache after two stats and a digest over the files git already calls
    /// dirty, so a worktree where nothing happened costs almost nothing and one
    /// where something did returns the new set without being asked twice.
    func follow() async {
        while !Task.isCancelled {
            try? await Task.sleep(for: .seconds(3))
            guard !Task.isCancelled else { return }
            await poll()
        }
    }

    private func poll() async {
        // Not while a full load is running: the two would race to assign
        // `changeSet`, and the loser would be the newer answer half the time.
        guard !loading else { return }
        let args = ["changes", "status", workspace.short, "--json"]
        guard let data = await client.changesJSON(args),
            let next = try? JSONDecoder().decode(ChangeSet.self, from: data),
            next != changeSet
        else { return }

        let before = Dictionary(
            changeSet.files.map { ($0.path, $0) }, uniquingKeysWith: { a, _ in a })
        changeSet = next

        // Only what actually moved is re-read. Dropping every cached diff on
        // any change would re-fetch forty files because one line changed in one
        // of them, and the scroll would empty out and refill under whoever was
        // reading it.
        //
        // Which files those are depends on the scope, because the two ask
        // different questions. A branch file's committed diff changes only when
        // its `numstat` entry does. A local file's diff changes every time
        // anybody saves it, and its entry — a path in a list of dirty paths —
        // says nothing at all about its contents, so every dirty file that has
        // been read is re-read. That set is small by construction: it is the
        // files git already calls dirty, intersected with the ones scrolled to.
        //
        // A commit is finished. Its patch is the difference between two
        // objects that already exist and cannot change, so there is nothing for
        // a poll to find — and re-reading it every three seconds would spend a
        // round trip per file on screen to fetch the bytes already on screen.
        let stale: [String]
        switch scope {
        case .branch:
            stale = next.files.filter { before[$0.path] != $0 }.map(\.path)
        case .local:
            stale = files.map(\.path)
        case .commit:
            stale = []
        }

        let live = Set(files.map(\.path))
        for path in fileDiffs.keys where !live.contains(path) {
            fileDiffs.removeValue(forKey: path)
        }
        for path in stale where fileDiffs[path] != nil {
            // Re-read rather than merely forgotten: nothing would ask for it
            // again on its own, because the row that asks does so once, keyed
            // on a path that has not changed.
            await read(path)
        }
        if let open = selectedFile, !live.contains(open) {
            selectedFile = nil
        }
        // The sidebar counts come from a different call, and a diff that has
        // moved on while the row beside it has not is the same worktree
        // described two ways — which is the bug the base fix was about.
        await client.refreshChangesInbox()
    }

    /// Move the daemon's watermark to where this worktree is now.
    ///
    /// What it is FOR is mostly not on this screen: `changed_since_reviewed` is
    /// what puts a worktree in the phone's Needs You list and in the home
    /// screen widget's count, so the reader who has just read a branch here is
    /// clearing a row they would otherwise be shown again on a device in their
    /// pocket. The sidebar's counts answer for it locally — see
    /// `WorkspaceSection.changeCountsText` — which is the whole of what the Mac
    /// draws from this watermark and is deliberately not more.
    ///
    /// It used to call `load()`, and that was wrong in a way nothing could have
    /// noticed while the method had no caller. Marking read tells the daemon
    /// when you last looked; it changes nothing about the diff, and `load()`
    /// runs `reset()` — every fetched patch dropped, every expanded gap closed,
    /// every file re-fetched as the reader scrolls back down. So the one
    /// gesture that means "I have finished reading this" would have thrown away
    /// the reading. It refreshes the INBOX instead, which is the only thing
    /// that actually changed and the only thing with anything to redraw.
    ///
    /// Directly rather than through `refreshChangesInboxSoon()`: the coalescing
    /// floor exists to keep a fleet of polling panes off a timer, and this is a
    /// click that has to answer for itself.
    func markRead() async {
        await client.changesMarkRead(workspace: workspace.short)
        await client.refreshChangesInbox()
    }
}
