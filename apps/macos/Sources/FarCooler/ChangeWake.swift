import Combine
import Foundation

/// The wait between a restore's passes (ov-296): until any of `publishers`
/// says it's about to change, or `limit` has passed, whichever comes first.
///
/// A restore asks again as its runner comes up. It used to ask every 250 ms,
/// so a window whose runner was ready sat on the placeholder for up to a
/// quarter of a second more before it went back to its place. Woken by the
/// store's own change, it goes back on the next turn of the main actor;
/// `limit` still bounds the wait, for anything a publisher doesn't announce.
///
/// `objectWillChange` fires before the write, inside the setter, so the wake
/// is a hop onto the main actor: it runs after the job that made the write,
/// and the pass that follows reads the value written.
@MainActor
enum ChangeWake {
    static func next(of publishers: [ObservableObjectPublisher], orAfter limit: Duration) async {
        let once = Once()
        var subscription: AnyCancellable?
        await withTaskCancellationHandler {
            await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
                once.continuation = continuation
                guard !Task.isCancelled else { return once.resume() }
                subscription = Publishers.MergeMany(publishers).first().sink { _ in
                    Task { @MainActor in once.resume() }
                }
                once.timer = Task { @MainActor in
                    try? await Task.sleep(for: limit)
                    once.resume()
                }
            }
        } onCancel: {
            Task { @MainActor in once.resume() }
        }
        subscription?.cancel()
    }

    /// Resumes its continuation at most once, whoever gets there first.
    @MainActor private final class Once {
        var continuation: CheckedContinuation<Void, Never>?
        var timer: Task<Void, Never>?

        func resume() {
            timer?.cancel()
            timer = nil
            continuation?.resume()
            continuation = nil
        }
    }
}
