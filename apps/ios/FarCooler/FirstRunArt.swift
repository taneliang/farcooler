import SwiftUI

// The drawings behind the phones' first-run and empty states (ov-205). Each
// is static and gray: it shows the shape of what will appear, so the sentence
// beside it doesn't have to describe the screen. Nothing shimmers or pulses,
// because that would read as loading, and nothing is loading.

/// Placeholder task rows: the shape tasks will take, not a loading state.
struct TaskSkeleton: View {
    static let widths: [CGFloat] = [0.72, 0.54, 0.63]

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            ForEach(Self.widths.indices, id: \.self) { i in
                HStack(alignment: .top, spacing: 12) {
                    Circle()
                        .strokeBorder(.quaternary, lineWidth: 1.5)
                        .frame(width: 10, height: 10)
                        .padding(.top, 1)
                    GeometryReader { geometry in
                        VStack(alignment: .leading, spacing: 5) {
                            Capsule().fill(.quaternary)
                                .frame(width: geometry.size.width * Self.widths[i], height: 8)
                            Capsule().fill(.quaternary)
                                .frame(
                                    width: geometry.size.width * Self.widths[(i + 1) % 3] * 0.5,
                                    height: 6)
                        }
                    }
                    .frame(height: 20)
                }
            }
        }
        .accessibilityHidden(true)
    }
}

/// What the orchestrator does: your request in a bubble, then the tasks it
/// splits it into, each ending in the branch its agent works on. The first
/// appearance fades the rows in once; Reduce Motion skips that.
struct OrchestratorVignette: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var shown = false

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: "person.wave.2")
                .font(.system(size: 18))
                .foregroundStyle(.tertiary)
            VStack(alignment: .leading, spacing: 8) {
                // style-exempt: a drawing of a speech bubble, not a surface
                RoundedRectangle(cornerRadius: 9)
                    .strokeBorder(.quaternary, lineWidth: 1)
                    .overlay(alignment: .leading) {
                        Capsule().fill(.quaternary).frame(width: 72, height: 6).padding(.leading, 10)
                    }
                    .frame(width: 104, height: 24)
                ForEach(0..<3, id: \.self) { i in
                    HStack(spacing: 6) {
                        Rectangle().fill(.quaternary).frame(width: 10, height: 1)
                        Circle().strokeBorder(.quaternary, lineWidth: 1.2).frame(width: 7, height: 7)
                        Capsule().fill(.quaternary).frame(width: [56, 38, 48][i], height: 6)
                        Image(systemName: "arrow.triangle.branch")
                            .font(.system(size: 9))
                            .foregroundStyle(.quaternary)
                    }
                    .opacity(shown || reduceMotion ? 1 : 0)
                    .animation(
                        reduceMotion ? nil : .easeOut(duration: 0.2).delay(0.06 * Double(i)), value: shown)
                }
            }
        }
        .accessibilityHidden(true)
        .onAppear { shown = true }
    }
}

/// This phone, three dots, and a runner: "this reaches that" without a word.
struct PhoneOnboardingMark: View {
    var body: some View {
        HStack(spacing: 14) {
            Image(systemName: DeviceKind.current == "iPad" ? "ipad" : "iphone")
                .font(.system(size: 38, weight: .thin))
            HStack(spacing: 5) {
                ForEach(0..<3, id: \.self) { _ in
                    Circle().fill(.quaternary).frame(width: 3, height: 3)
                }
            }
            Image(systemName: "server.rack")
                .font(.system(size: 42, weight: .thin))
        }
        .foregroundStyle(.tertiary)
        .accessibilityHidden(true)
    }
}
