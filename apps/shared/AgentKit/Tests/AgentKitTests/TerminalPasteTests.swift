import Testing

@testable import AgentKit

private let code = "Zk3_pQ9xLmN2vB7cD4fG8hJ1kR5tY6uW0aS-eXiOoPqRsTuVwXyZ0123456789abc#9f8e7d6c5b4a39281706f5e4d3c2b1a0"

/// Stands in for the core's encoder in a program that asked for bracketed
/// paste. The real bytes are pinned in `crates/vt/tests/login_code_paste.rs`.
private func bracketed(_ text: String) -> [UInt8] {
    Array("\u{1b}[200~".utf8) + Array(text.utf8) + Array("\u{1b}[201~".utf8)
}

@Test func aPastedCodeGoesOutAsTheEncodersBytesInOneWrite() {
    var asked: [String] = []
    let bytes = TerminalPaste.bytes(for: code, mayType: true) { text in
        asked.append(text)
        return bracketed(text)
    }
    #expect(asked == [code], "the whole code is encoded once, not scalar by scalar")
    #expect(bytes == bracketed(code))
    #expect(bytes?.starts(with: Array("\u{1b}[200~".utf8)) == true)
}

@Test func aPasteThatSkipsTheEncoderIsCaught() {
    // The old shape: typed text, unbracketed. It must not equal a paste.
    let typed = Array(code.utf8)
    #expect(TerminalPaste.bytes(for: code, mayType: true, encode: bracketed) != typed)
}

@Test func theTrailingNewlineStaysInsideTheBracket() {
    let bytes = TerminalPaste.bytes(for: code + "\n", mayType: true, encode: bracketed)
    #expect(bytes == bracketed(code + "\n"))
}

@Test func aPhoneThatMayNotTypeSendsNothingAndEncodesNothing() {
    var encoded = false
    let bytes = TerminalPaste.bytes(for: code, mayType: false) { _ in
        encoded = true
        return []
    }
    #expect(bytes == nil)
    #expect(!encoded)
}

@Test func anEmptyOrMissingClipboardSendsNothing() {
    #expect(TerminalPaste.bytes(for: nil, mayType: true, encode: bracketed) == nil)
    #expect(TerminalPaste.bytes(for: "", mayType: true, encode: bracketed) == nil)
}
