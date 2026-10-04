import Testing

@testable import AgentKit

private struct Banner: Identifiable {
    let id: Int
}

struct PaneFailureBannersTests {
    @Test func aRefusedControlKeepsTheUnsentMessagesRetry() {
        var banners = PaneFailureBanners<Banner>()
        banners.send = Banner(id: 1)
        banners.controlFailed(Banner(id: 2), key: "mode")
        #expect(banners.all.map(\.id) == [1, 2])
    }

    @Test func aRetryThatWorksClearsItsOwnBannerOnly() {
        var banners = PaneFailureBanners<Banner>()
        banners.controlFailed(Banner(id: 2), key: "mode")
        banners.controlSucceeded(key: "model")
        #expect(banners.all.map(\.id) == [2])
        banners.controlSucceeded(key: "mode")
        #expect(banners.all.isEmpty)
    }

    @Test func dismissingTakesOneBanner() {
        var banners = PaneFailureBanners<Banner>()
        banners.send = Banner(id: 1)
        banners.controlFailed(Banner(id: 2), key: "mode")
        banners.dismiss(2)
        #expect(banners.all.map(\.id) == [1])
    }
}
