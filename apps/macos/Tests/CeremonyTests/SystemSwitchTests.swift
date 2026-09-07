import SwiftUI
import Testing

@testable import Far_Cooler

/// The switches on the startup pane, which were decorative for their whole
/// lives.
///
/// Both were `Toggle(isOn: .constant(…))` with `.onTapGesture`. A `Toggle` is a
/// control with its own gesture and never delivers that tap, and a `.constant`
/// binding cannot move even if it did — so clicking either one did nothing and
/// the switch stayed where it was. The owner reported exactly that.
///
/// These pin the two halves a unit test can reach: that the binding is LIVE
/// rather than constant, and that turning it on asks for on. What no test here
/// can reach is whether the view passes this binding to the `Toggle` at all —
/// this package has no UI automation, and `apps/macos/Package.swift` says a
/// SwiftUI view is verified by looking at it.
@Suite struct SystemSwitchTests {
    /// Turning it on asks for on, not off.
    ///
    /// Swapping the two closures is the mistake this shape invites, since the
    /// call sites read `turnOn:` `turnOff:` in a switch whose cases are the
    /// other way around.
    @Test func turningItOnAsksForOn() {
        var asked: String?
        let binding = SystemSwitch.binding(
            isOn: false, turnOn: { asked = "on" }, turnOff: { asked = "off" })
        binding.wrappedValue = true
        #expect(asked == "on")
    }

    @Test func turningItOffAsksForOff() {
        var asked: String?
        let binding = SystemSwitch.binding(
            isOn: true, turnOn: { asked = "on" }, turnOff: { asked = "off" })
        binding.wrappedValue = false
        #expect(asked == "off")
    }

    /// The binding reads the state it was given rather than a remembered one.
    @Test func itReadsTheStateItWasHanded() {
        #expect(SystemSwitch.binding(isOn: true, turnOn: {}, turnOff: {}).wrappedValue == true)
        #expect(SystemSwitch.binding(isOn: false, turnOn: {}, turnOff: {}).wrappedValue == false)
    }

    /// Nothing is written on the way in.
    ///
    /// Registering can land in `awaitingApproval` rather than `registered`, so
    /// a switch that flipped itself on would claim something macOS has not
    /// agreed to. The next `refresh()` decides what it shows.
    @Test func settingItDoesNotMoveTheSwitchByItself() {
        let binding = SystemSwitch.binding(isOn: false, turnOn: {}, turnOff: {})
        binding.wrappedValue = true
        #expect(binding.wrappedValue == false, "the switch moved itself before the system agreed")
    }
}
