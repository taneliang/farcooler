import SwiftUI

/// Something a person asked for that the runner didn't do, in words for a screen.
///
/// A close, a hide, a mode switch and a create each used to swallow the
/// runner's answer with `try?`, so a refusal looked exactly like success: the
/// tab stayed, the row didn't move, the sheet closed on nothing (ov-179). The
/// `Connection` call now hands one of these back and the screen that made the
/// request says it.
struct ActionFailure: Identifiable, Equatable {
    let id = UUID()
    /// What didn't happen, as an alert title.
    let title: String
    /// Why, or what to do: the runner's own sentence for a refusal it named,
    /// and `otherwise` for one it didn't. Never a raw error.
    let sentence: String

    /// A failure from a thrown core error. A named refusal gets its table
    /// sentence; anything else gets `otherwise`.
    init(_ error: Error, title: String, otherwise: String) {
        self.title = title
        self.sentence = ClientCore.trouble(error, otherwise: otherwise).sentence
    }
}

extension View {
    /// An alert for the failure in `failure`, cleared when dismissed.
    func actionFailureAlert(_ failure: Binding<ActionFailure?>) -> some View {
        alert(
            failure.wrappedValue?.title ?? "",
            isPresented: Binding(
                get: { failure.wrappedValue != nil },
                set: { if !$0 { failure.wrappedValue = nil } }),
            presenting: failure.wrappedValue
        ) { _ in
            Button("OK", role: .cancel) {}
        } message: { shown in
            Text(shown.sentence)
        }
    }
}
