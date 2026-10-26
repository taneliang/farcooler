import Foundation

/// A value and a number that moves whenever the value changes, and only
/// then (ov-791): what a cache compares in place of the value. Setting an
/// equal value, as a board read again that changed nothing does, keeps the
/// number, so nothing built from it is built again.
public struct Versioned<Value: Equatable> {
    public var value: Value {
        didSet { if value != oldValue { version &+= 1 } }
    }
    public private(set) var version = 0

    public init(_ value: Value) {
        self.value = value
    }
}
