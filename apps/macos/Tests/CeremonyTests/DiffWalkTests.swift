import Foundation
import Testing

@testable import Far_Cooler

/// Where Next and Previous go in the diff pane.
///
/// This target holds the rules that are invisible by looking, and movement
/// through a diff earned its place here the moment a commit could be handed off
/// to. Two facts make it dangerous. `moveHunk` FALLS THROUGH to `moveFile` at
/// both ends of a file, so any change to what the end of a file list means
/// silently changes hunk navigation as well; and Branch and Uncommitted have
/// wrapped that list modulo since the pane existed, which is behavior nobody
/// asked to change and nobody would notice breaking until they pressed Next on
/// the last file of a forty-file branch and nothing happened.
///
/// So the first suite here is a DIFFERENTIAL one. `Legacy` below is the
/// arithmetic exactly as it stood in `ChangesPane` before the commit walk
/// arrived, transcribed from `09b1e1f`, and `agreesWithTheOldPane` sweeps every
/// combination of direction, file count, current file and hunk state through
/// both. It is not a description of the wrap; it is the wrap, kept alive beside
/// the thing that replaced it.
struct DiffWalkWrapTests {
    /// `moveFile` and `moveHunk` as they were, in one place and answering in
    /// the vocabulary the new code speaks.
    ///
    /// Deliberately a transcription rather than a tidy-up. Its value is that
    /// nothing about it was thought through a second time: the `?? (direction >
    /// 0 ? -1 : 0)` that makes Next-with-nothing-open mean "the first file", the
    /// `+ files.count` before the modulo that makes Previous-from-the-first
    /// mean "the last", and the two separate paths out of a hunk list are all
    /// here in the shape they had.
    enum Legacy {
        static func moveFile(_ direction: Int, files: Int, current: Int?) -> DiffStep {
            guard files > 0 else { return .stay }
            let from = current ?? (direction > 0 ? -1 : 0)
            let next = (from + direction + files) % files
            return .file(next)
        }

        static func moveHunk(
            _ direction: Int, hunks: [String], lastHunk: String?, files: Int, current: Int?
        ) -> DiffStep {
            guard !hunks.isEmpty else {
                return moveFile(direction, files: files, current: current)
            }
            if let lastHunk, let index = hunks.firstIndex(of: lastHunk) {
                let next = index + direction
                guard hunks.indices.contains(next) else {
                    return moveFile(direction, files: files, current: current)
                }
                return .hunk(hunks[next])
            }
            guard let target = direction > 0 ? hunks.first : hunks.last else {
                return moveFile(direction, files: files, current: current)
            }
            return .hunk(target)
        }
    }

    /// The proof that Branch and Uncommitted move exactly as they used to.
    ///
    /// Every direction, every file count from none to five, every possible
    /// current file including none at all, every hunk list from none to three,
    /// and every `lastHunkJump` including one belonging to a file that is no
    /// longer on screen. If the new walk and the old pane ever disagree under
    /// `.wrap`, this says which input did it.
    @Test func agreesWithTheOldPane() {
        let hunkLists: [[String]] = [[], ["h0"], ["h0", "h1"], ["h0", "h1", "h2"]]
        // `stale` is the case that used to have its own branch in `moveHunk`:
        // a hunk id left over from a file that has since scrolled out of the
        // scope, which is neither "no hunk visited" nor "a hunk in this list".
        let lastHunks: [String?] = [nil, "h0", "h1", "h2", "stale"]

        for direction in [-1, 1] {
            for files in 0...5 {
                var currents: [Int?] = [nil]
                currents.append(contentsOf: (0..<files).map { Optional($0) })
                for current in currents {
                    // The plain file move, which is `moveFile` with no hunks in
                    // play at all.
                    #expect(
                        DiffWalk.step(
                            direction, hunks: [], after: nil, files: files, at: current,
                            boundary: .wrap)
                            == Legacy.moveFile(direction, files: files, current: current),
                        """
                        moveFile direction \(direction) files \(files) \
                        at \(String(describing: current))
                        """)

                    for hunks in hunkLists {
                        for lastHunk in lastHunks {
                            #expect(
                                DiffWalk.step(
                                    direction, hunks: hunks, after: lastHunk, files: files,
                                    at: current, boundary: .wrap)
                                    == Legacy.moveHunk(
                                        direction, hunks: hunks, lastHunk: lastHunk, files: files,
                                        current: current),
                                """
                                moveHunk direction \(direction) files \(files) \
                                at \(String(describing: current)) hunks \(hunks) \
                                after \(String(describing: lastHunk))
                                """)
                        }
                    }
                }
            }
        }
    }

    /// The four wrap cases named, so a failure above has something to be read
    /// against rather than only a sweep that went red.
    @Test func theLastFileWrapsToTheFirst() {
        #expect(
            DiffWalk.step(1, hunks: [], after: nil, files: 3, at: 2, boundary: .wrap) == .file(0))
    }

    @Test func theFirstFileWrapsToTheLast() {
        #expect(
            DiffWalk.step(-1, hunks: [], after: nil, files: 3, at: 0, boundary: .wrap)
                == .file(2))
    }

    /// Nothing open yet. Next means the first file and Previous means the last,
    /// which is what the `-1` in the old code bought.
    @Test func withNoFileOpenNextIsTheFirstAndPreviousIsTheLast() {
        #expect(
            DiffWalk.step(1, hunks: [], after: nil, files: 3, at: nil, boundary: .wrap) == .file(0))
        #expect(
            DiffWalk.step(-1, hunks: [], after: nil, files: 3, at: nil, boundary: .wrap)
                == .file(2))
    }

    /// A pane with nothing in it. The old `moveFile` returned early and the new
    /// one says so out loud.
    @Test func noFilesIsNowhereToGo() {
        #expect(
            DiffWalk.step(1, hunks: [], after: nil, files: 0, at: nil, boundary: .wrap) == .stay)
    }
}

/// The hunk half, which the file half is reached through.
struct DiffWalkHunkTests {
    private let hunks = ["a#0", "a#1", "a#2"]

    /// Entering a file from the direction of travel: down the page means its
    /// first hunk, up the page means its last.
    @Test func aFileIsEnteredFromTheEndYouAreTravelingTowards() {
        #expect(
            DiffWalk.step(1, hunks: hunks, after: nil, files: 2, at: 0, boundary: .wrap)
                == .hunk("a#0"))
        #expect(
            DiffWalk.step(-1, hunks: hunks, after: nil, files: 2, at: 0, boundary: .wrap)
                == .hunk("a#2"))
    }

    @Test func theMiddleOfAFileIsJustTheNextHunk() {
        #expect(
            DiffWalk.step(1, hunks: hunks, after: "a#1", files: 2, at: 0, boundary: .wrap)
                == .hunk("a#2"))
        #expect(
            DiffWalk.step(-1, hunks: hunks, after: "a#1", files: 2, at: 0, boundary: .wrap)
                == .hunk("a#0"))
    }

    /// The fall-through, and the reason this whole file exists: past the last
    /// hunk of a file, Next becomes a file move. A control that stopped here
    /// would need a second control beside it to get past every file.
    @Test func pastTheLastHunkTheSameKeyMovesFile() {
        #expect(
            DiffWalk.step(1, hunks: hunks, after: "a#2", files: 3, at: 0, boundary: .wrap)
                == .file(1))
        #expect(
            DiffWalk.step(-1, hunks: hunks, after: "a#0", files: 3, at: 1, boundary: .wrap)
                == .file(0))
    }

    /// A hunk id left over from a file that is no longer on screen is not a
    /// position in this file's list, so it is entered from the end like any
    /// other first visit — never treated as index 0.
    @Test func aHunkFromAnotherFileIsNotAPosition() {
        #expect(
            DiffWalk.step(-1, hunks: hunks, after: "b#4", files: 2, at: 0, boundary: .wrap)
                == .hunk("a#2"))
    }
}

/// What the end of a commit's file list means, which is the behavior the walk
/// was pulled out of the view to add.
struct DiffWalkHandOffTests {
    @Test func theEndOfACommitIsTheNextCommit() {
        #expect(
            DiffWalk.step(
                1, hunks: [], after: nil, files: 3, at: 2, boundary: .handOff("beef"))
                == .commit("beef"))
    }

    /// Through the hunks too, which is the fall-through this had to be checked
    /// against: the last hunk of the last file of a commit is one press from
    /// the next commit, not from the top of the same one.
    @Test func theLastHunkOfTheLastFileIsAlsoTheNextCommit() {
        #expect(
            DiffWalk.step(
                1, hunks: ["h0"], after: "h0", files: 2, at: 1, boundary: .handOff("beef"))
                == .commit("beef"))
    }

    /// Backwards is the same deal in reverse — which is what makes walking a
    /// branch the other way possible at all.
    @Test func theStartOfACommitIsThePreviousCommit() {
        #expect(
            DiffWalk.step(
                -1, hunks: [], after: nil, files: 3, at: 0, boundary: .handOff("cafe"))
                == .commit("cafe"))
    }

    /// A merge whose branch brought nothing, or a commit that touched only a
    /// mode. It is a commit you walk THROUGH: a walk that got stuck on the one
    /// commit with nothing in it to read would be the worst place to stop.
    @Test func anEmptyCommitIsWalkedThroughRatherThanStuckIn() {
        #expect(
            DiffWalk.step(1, hunks: [], after: nil, files: 0, at: nil, boundary: .handOff("beef"))
                == .commit("beef"))
    }

    /// The end of the branch. Nothing to hand off to and nothing to wrap to —
    /// looping from the last file of the last commit to the first file of the
    /// same commit is a loop pretending to be progress.
    @Test func theEndOfTheBranchStops() {
        #expect(
            DiffWalk.step(1, hunks: [], after: nil, files: 3, at: 2, boundary: .stop) == .stay)
        #expect(
            DiffWalk.step(-1, hunks: [], after: nil, files: 3, at: 0, boundary: .stop) == .stay)
    }

    /// Stopping is only ever at the EDGE. Inside a commit the walk moves
    /// normally whatever the boundary says, which is the check that catches a
    /// `.stop` accidentally applied to every press.
    @Test func stopIsOnlyAtTheEdge() {
        #expect(
            DiffWalk.step(1, hunks: [], after: nil, files: 3, at: 0, boundary: .stop) == .file(1))
    }
}

/// Which of those three a given moment is, which is the whole policy.
struct DiffBoundaryTests {
    private func commit(_ sha: String) -> ChangeCommit {
        // Through the decoder, so this cannot drift from the wire the pane
        // actually reads.
        try! JSONDecoder().decode(
            ChangeCommit.self,
            from: Data(
                #"{"sha":"\#(sha)","subject":"s","author":"a","timestamp":0}"#.utf8))
    }

    /// The behavior the differential suite above pins, chosen here: neither of
    /// these scopes has anywhere to hand off to, because each is already the
    /// whole of what it can show.
    @Test func branchAndUncommittedWrap() {
        for scope in [DiffScope.branch, .local] {
            #expect(
                DiffWalk.boundary(1, scope: scope, next: commit("a"), previous: commit("b"))
                    == .wrap)
            #expect(
                DiffWalk.boundary(-1, scope: scope, next: commit("a"), previous: commit("b"))
                    == .wrap)
        }
    }

    @Test func aCommitHandsOffInEitherDirection() {
        #expect(
            DiffWalk.boundary(1, scope: .commit, next: commit("a"), previous: commit("b"))
                == .handOff("a"))
        #expect(
            DiffWalk.boundary(-1, scope: .commit, next: commit("a"), previous: commit("b"))
                == .handOff("b"))
    }

    /// The two ends of the branch, and the case an amend mid-review creates:
    /// a commit the branch no longer lists has no neighbors in either
    /// direction, so both ends stop rather than guessing at a position.
    @Test func anEndOfTheBranchStops() {
        #expect(DiffWalk.boundary(1, scope: .commit, next: nil, previous: commit("b")) == .stop)
        #expect(DiffWalk.boundary(-1, scope: .commit, next: commit("a"), previous: nil) == .stop)
        #expect(DiffWalk.boundary(1, scope: .commit, next: nil, previous: nil) == .stop)
    }
}
