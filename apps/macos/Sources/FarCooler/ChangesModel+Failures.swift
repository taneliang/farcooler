import Foundation

/// A diff read that failed (ov-155).
///
/// The failure used to travel as an empty `FileDiff`, so the pane drew "No
/// textual changes" for a file it never read, and a gap whose lines could not
/// be read was recorded as "too many to show". Neither can be retried, and
/// both say something untrue.
struct DiffReadFailure: Error, Equatable {
    /// The CLI's own words, kept for diagnosis and never drawn in a row.
    var message: String?
}

extension ChangesStore {
    /// What a status reply this app could not decode says in the error box.
    static let unreadableReply = "The reply wasn’t in a form Far Cooler could read."

    /// File one path as unreadable, unless a good copy is already showing.
    ///
    /// A poll that re-reads a file and fails must not blank a diff someone is
    /// reading; the next poll tries again. A first read that fails has nothing
    /// to keep, so the pane shows the failure row.
    func noteFailedRead(of path: String) {
        guard fileDiffs[path] == nil else { return }
        fileFailures.insert(path)
    }

    /// Try Again on a file's failure row.
    func retry(_ path: String) async {
        fileFailures.remove(path)
        await read(path)
    }
}
