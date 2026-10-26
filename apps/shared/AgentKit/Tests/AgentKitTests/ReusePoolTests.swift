import Testing
@testable import AgentKit

/// A conversation's text views are kept for reuse (ov-788): the same object
/// comes back, cleared as it was kept, and never more than the limit.
@MainActor
@Suite struct ReusePoolTests {
    final class View {
        var text = ""
        var selected = false
    }

    static func pool(_ limit: Int) -> ReusePool<View> {
        ReusePool(limit: limit) { view in
            view.text = ""
            view.selected = false
        }
    }

    @Test func aKeptOneComesBackCleared() {
        let pool = Self.pool(4)
        let view = View()
        view.text = "A long reply's paragraph"
        view.selected = true
        pool.keep(view)
        let again = pool.take()
        #expect(again === view, "a new one was made where a kept one would do")
        #expect(again?.text == "" && again?.selected == false, "it kept the row it served")
        #expect(pool.take() == nil)
    }

    @Test func itKeepsNoMoreThanItsLimit() {
        let pool = Self.pool(2)
        let views = (0..<5).map { _ in View() }
        views.forEach(pool.keep)
        pool.keep(views[0])
        #expect(pool.count == 2)
        pool.drain()
        #expect(pool.count == 0)
    }
}
