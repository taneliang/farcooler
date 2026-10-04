import Foundation

// The daemon's answer for one file of a diff, as the phone reads it.
//
// It lived as a private struct in the iOS app, which decoded `hunks` and
// `unsupported` and nothing else. So a patch the daemon had cut off, or a
// merge shown against one of its parents, drew exactly like a whole patch:
// Android and the Mac both say so, and iOS showed the part it was sent and
// called it the file (ov-149). Here, with no UIKit in it, a fixture can be
// decoded and checked without a simulator.

/// One file's diff as `changes.file_diff` returns it.
///
/// Structured hunks rather than unified text, so the numbers are the
/// daemon's and not re-derived from `@@` headers.
public struct ChangesFileDiff: Decodable, Equatable {
    /// The reason the daemon would not render this file, when it would not.
    public var unsupported: String?
    /// The daemon cut this patch off, because it is too big to send whole.
    public var truncated: Bool
    /// A merge, shown against its first parent only.
    public var firstParentOfMerge: Bool
    /// The hunks the daemon sent.
    public var hunks: [Hunk]

    /// A run of lines with no gap in it.
    public struct Hunk: Decodable, Equatable {
        /// The hunk's lines, in order.
        public var lines: [Line]
    }

    /// One line of a hunk.
    public struct Line: Decodable, Equatable {
        /// `added`, `removed` or anything else for context.
        public var kind: String
        /// The line's number in the old file, when it has one.
        public var oldNumber: Int?
        /// The line's number in the new file, when it has one.
        public var newNumber: Int?
        /// The line's text, without its newline.
        public var text: String
    }

    private enum CodingKeys: String, CodingKey {
        case unsupported, truncated, firstParentOfMerge, hunks
    }

    /// Decodes with every field optional but `hunks`, because an older daemon
    /// sends neither flag and its patches are whole.
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        unsupported = try c.decodeIfPresent(String.self, forKey: .unsupported)
        truncated = try c.decodeIfPresent(Bool.self, forKey: .truncated) ?? false
        firstParentOfMerge = try c.decodeIfPresent(Bool.self, forKey: .firstParentOfMerge) ?? false
        hunks = try c.decodeIfPresent([Hunk].self, forKey: .hunks) ?? []
    }

    /// Flattened into the same line model the agent transcript's diffs use.
    ///
    /// Hunk boundaries are not drawn: a phone has no room for a `@@` header,
    /// and the jump in line numbers between two hunks already says a gap is
    /// there.
    public func lines() -> [DiffComputation.Line] {
        var out: [DiffComputation.Line] = []
        for hunk in hunks {
            for line in hunk.lines {
                let kind: DiffComputation.Kind
                switch line.kind {
                case "added": kind = .added
                case "removed": kind = .removed
                default: kind = .context
                }
                out.append(
                    DiffComputation.Line(
                        id: out.count, kind: kind, oldNumber: line.oldNumber,
                        newNumber: line.newNumber, text: line.text))
            }
        }
        return out
    }

    /// The sentences that belong above this patch, in order.
    ///
    /// Notices around a patch that is really there, not reasons a file has no
    /// lines. The words are Android's, so the two phones say the same thing.
    public var notices: [String] {
        var out: [String] = []
        if truncated { out.append(Self.truncatedNotice) }
        if firstParentOfMerge { out.append(Self.mergeNotice) }
        return out
    }

    /// What a cut-off patch says.
    public static let truncatedNotice = "This patch was cut short. It\u{2019}s too big to send whole."
    /// What a merge shown against its first parent says.
    public static let mergeNotice = "This is a merge, shown against its first parent only."
}

/// How much of one file's patch is drawn before the rest is offered.
///
/// Six hundred lines, Android's budget: around eight screenfuls, far more than
/// anyone reads in one sitting on a phone, and it catches the case it exists
/// for, a lockfile or a generated client that is one huge added hunk.
public enum PatchBudget {
    /// Lines drawn before the rest is held back.
    public static let lines = 600

    /// The lines to draw, given whether the reader has asked for all of them.
    public static func visible<T>(_ all: [T], whole: Bool) -> [T] {
        whole ? all : Array(all.prefix(lines))
    }

    /// The label of the row that reveals the rest, or nil when nothing is held back.
    public static func moreLabel(total: Int, whole: Bool) -> String? {
        guard !whole, total > lines else { return nil }
        let rest = total - lines
        return rest == 1 ? "Show 1 More Line" : "Show \(rest) More Lines"
    }
}
