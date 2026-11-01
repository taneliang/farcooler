import Testing

@testable import Far_Cooler

/// ⌘K finds a plan's themes and tickets by name, and opens their pages in
/// their workspace (ov-298).
struct PalettePlanTests {
    static let plans = [
        PalettePlanItem(host: "", workspace: "w", workspaceName: "Billing", page: .theme("t1"), name: "Invoices", detail: "1 of 5 done"),
        PalettePlanItem(host: "", workspace: "w", workspaceName: "Billing", page: .ticket("t1"), name: "PDF export", detail: "Building"),
    ]

    @Test("A theme and a ticket are found by name, and open their pages")
    func findsThemesAndTickets() {
        let theme = PaletteIndex.matching("invoices", in: [], plans: Self.plans).first
        #expect(theme?.action == .openPlan(host: "", workspace: "w", page: .theme("t1")))
        #expect(theme?.kind == "theme" && theme?.detail == "Billing · 1 of 5 done")
        let ticket = PaletteIndex.matching("pdf", in: [], plans: Self.plans).first
        #expect(ticket?.action == .openPlan(host: "", workspace: "w", page: .ticket("t1")))
        #expect(ticket?.kind == "ticket")
        #expect(!PaletteIndex.matching("zzz", in: [], plans: Self.plans).contains { $0.kind == "theme" || $0.kind == "ticket" })
    }
}
