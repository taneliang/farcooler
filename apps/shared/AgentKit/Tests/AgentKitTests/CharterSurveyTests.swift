import Foundation
import Testing

@testable import AgentKit

// The first run's Charter screen (ov-519 phase 2b): the survey's JSON as rows
// (From, Default, Changed), the toggles as `workspace write-charter`'s
// arguments, and the words.

/// `farcooler --json repo survey`, as `crates/cli/src/repo_survey.rs`'s
/// `survey_json` prints it, for a repository with GitHub's rules and a
/// CLAUDE.md.
private let surveyed = """
    {"repository":"7b0c","answers":[
     {"key":"workflow","heading":"## Workflow","value":"pull_requests","text":"PR: each change is a pull request into main that a person approves.","source":"main requires pull requests"},
     {"key":"done_means","heading":"## Done means","value":"","text":"Ci.yml runs cargo test.","source":".github/workflows/ci.yml"},
     {"key":"review","heading":"## Review","value":"","text":"I review each pull request.","source":"default"},
     {"key":"who_decides","heading":"## Who decides","value":"","text":"Make a reversible call yourself.","source":"default"},
     {"key":"reaching_me","heading":"## Reaching me","value":"","text":"Ask me on the board.","source":"default"},
     {"key":"lanes","heading":"## Lanes","value":"2","text":"Up to 2 agents at once.","source":"default"},
     {"key":"autonomy","heading":"## Autonomy","value":"","text":"Agents may commit and push their own branches.","source":"default"},
     {"key":"initiative","heading":"## Autonomy","value":"on","text":"Initiative: on.","source":"default"},
     {"key":"instructions","heading":"## Anything else","value":"","text":"Agents follow CLAUDE.md, which they already read.","source":"CLAUDE.md (32 lines)"},
     {"key":"harness","heading":"","value":"claude","text":"","source":"claude is installed"},
     {"key":"a_newer_key","heading":"## Later","value":"","text":"Something new.","source":"default"}
    ],"agents":["claude","codex"],"charter_draft":"<!-- charter -->","took_ms":840,"read_at":1}
    """

private func charterForm(_ json: String = surveyed) throws -> CharterForm {
    CharterForm(survey: try #require(CharterSurvey(json: Data(json.utf8))))
}

// MARK: - Survey JSON to rows

@Test func theSurveysJSONReadsAsAnswersWithTheirSource() throws {
    let survey = try #require(CharterSurvey(json: Data(surveyed.utf8)))
    #expect(survey.answer("workflow")?.signal == "main requires pull requests")
    #expect(survey.answer("review")?.signal == nil, "\"default\" is no signal")
    #expect(survey.answer("lanes")?.value == "2")
    #expect(survey.agents == ["claude", "codex"])
    #expect(survey.tookMilliseconds == 840)
    #expect(CharterSurvey(json: Data("not json".utf8)) == nil)
    #expect(CharterSurvey(json: Data("{\"wrote\":true}".utf8)) == nil)
}

@Test func eachRowSaysFromOrDefault() throws {
    let rows = try charterForm().rows
    #expect(rows.map(\.id) == ["workflow", "done_means", "review", "autonomy", "lanes", "initiative", "also"])
    func row(_ id: String) -> CharterRow? { rows.first { $0.id == id } }
    #expect(row("workflow")?.provenance == .from("main requires pull requests"))
    #expect(row("workflow")?.control == .landing)
    #expect(row("done_means")?.provenance == .from(".github/workflows/ci.yml"))
    #expect(row("done_means")?.text == ["Ci.yml runs cargo test."])
    #expect(row("review")?.provenance == .byDefault(nil))
    #expect(row("lanes")?.provenance == .byDefault(FirstRunCopy.Charter.lanesDefault))
    #expect(row("lanes")?.control == .lanes)
    #expect(row("initiative")?.control == .initiative)
    #expect(row("also")?.provenance == .from("CLAUDE.md (32 lines)"))
    // Not drawn: the skill's defaults, a phone's questions, the harness (its
    // own picker) and a key this build doesn't know.
    for hidden in ["who_decides", "reaching_me", "harness", "a_newer_key"] { #expect(row(hidden) == nil) }
}

@Test func aToggleMovedFromTheSuggestionSaysChanged() throws {
    var form = try charterForm()
    #expect(form.suggested == CharterChoices(landing: .pullRequests, lanes: .count(2), initiative: true))
    form.choices.landing = .direct
    form.choices.lanes = .noLimit
    form.choices.initiative = false
    let rows = form.rows
    func row(_ id: String) -> CharterRow? { rows.first { $0.id == id } }
    #expect(row("workflow")?.provenance == .changed(suggested: "pull requests"))
    #expect(row("lanes")?.provenance == .changed(suggested: "2 at once"))
    #expect(row("initiative")?.provenance == .changed(suggested: "having it on"))
    #expect(row("workflow")?.provenance.tag == "Changed")
    #expect(row("workflow")?.provenance.detail == "Far Cooler suggested pull requests.")
    // Moved back, it's the survey's again.
    form.choices = form.suggested
    #expect(form.rows.first?.provenance == .from("main requires pull requests"))
    #expect(!form.isChanged)
}

@Test func aSurveyWithNothingToSayStillDrawsTheToggles() throws {
    let bare = try charterForm("{\"answers\":[]}")
    #expect(bare.suggested == CharterChoices(landing: .direct, lanes: .count(2), initiative: true))
    #expect(bare.rows.map(\.id) == ["workflow", "lanes", "initiative"])
    #expect(bare.rows.first?.provenance == .byDefault(FirstRunCopy.Charter.landingDefault))
}

@Test func agentsAtOnceOffersOneToFourNoLimitAndTheSurveysOwn() throws {
    #expect(try charterForm().laneOptions == [.count(1), .count(2), .count(3), .count(4), .noLimit])
    let six = surveyed.replacingOccurrences(of: "\"value\":\"2\"", with: "\"value\":\"6\"")
    #expect(try charterForm(six).laneOptions == [.count(1), .count(2), .count(3), .count(4), .count(6), .noLimit])
    let none = surveyed.replacingOccurrences(of: "\"value\":\"2\"", with: "\"value\":\"no_limit\"")
    #expect(try charterForm(none).suggested.lanes == .noLimit)
    #expect(CharterLanes(value: "0") == nil && CharterLanes(value: "100") == nil && CharterLanes(value: "two") == nil)
}

/// Who reviews and Agents may are written for the way work lands: with it
/// changed, the screen says their words will follow (review S1).
@Test func changingHowWorkLandsSaysTheRowsWrittenFromItWillFollow() throws {
    var form = try charterForm()
    func note(_ id: String) -> String? { form.rows.first { $0.id == id }?.note }
    #expect(note("review") == nil && note("autonomy") == nil)
    form.choices.landing = .direct
    #expect(note("review") == FirstRunCopy.Charter.followsLanding)
    #expect(note("autonomy") == FirstRunCopy.Charter.followsLanding)
    #expect(note("done_means") == nil, "Done means doesn't depend on it")
}

// MARK: - Toggles to write-charter

/// The way work lands is always sent, so the charter is the one on screen
/// even when the runner's second survey reads GitHub differently (review M1).
/// Lanes and initiative, always the default, go only when changed.
@Test func howWorkLandsIsAlwaysSentAndTheRestOnlyWhenChanged() throws {
    let form = try charterForm()
    #expect(
        form.writeArguments(workspace: "ws-1", draft: false)
            == ["workspace", "write-charter", "ws-1", "--landing", "pull-requests", "--json"])
    #expect(
        form.writeArguments(workspace: "ws-1", draft: true)
            == ["workspace", "write-charter", "ws-1", "--draft", "--landing", "pull-requests", "--json"])
    let defaulted = try charterForm("{\"answers\":[]}")
    #expect(defaulted.writeArguments(workspace: "w", draft: false) == ["workspace", "write-charter", "w", "--landing", "direct", "--json"])
}

@Test func eachChangedToggleIsItsFlag() throws {
    var form = try charterForm()
    form.choices.landing = .direct
    #expect(form.writeArguments(workspace: "w", draft: false) == ["workspace", "write-charter", "w", "--landing", "direct", "--json"])
    form.choices = form.suggested
    form.choices.lanes = .noLimit
    #expect(
        form.writeArguments(workspace: "w", draft: false)
            == ["workspace", "write-charter", "w", "--landing", "pull-requests", "--lanes", "none", "--json"])
    form.choices.lanes = .count(4)
    form.choices.initiative = false
    #expect(
        form.writeArguments(workspace: "w", draft: true)
            == ["workspace", "write-charter", "w", "--draft", "--landing", "pull-requests", "--lanes", "4", "--initiative", "off", "--json"])
    var direct = try charterForm("{\"answers\":[{\"key\":\"workflow\",\"value\":\"direct\",\"source\":\"default\"}]}")
    direct.choices.landing = .pullRequests
    #expect(direct.writeArguments(workspace: "w", draft: false).contains("pull-requests"))
}

@Test func writeCharterSaysWhetherItWrote() {
    #expect(CharterWritten(json: Data("{\"workspace\":\"w\",\"wrote\":false,\"draft\":false,\"text\":\"x\"}".utf8)) == CharterWritten(wrote: false))
    #expect(CharterWritten(json: Data("{\"wrote\":true}".utf8))?.wrote == true)
    #expect(CharterWritten(json: Data("[]".utf8)) == nil)
}

// MARK: - Copy

@Test func theCharterScreensReadInTheAppsOwnVoice() {
    let stock = ["seamless", "effortless", "unlock", "supercharge", "elevate", "dive in", "get started", "let’s"]
    for string in FirstRunCopy.Charter.all {
        #expect(!string.contains("!"), "an exclamation mark: \(string)")
        #expect(!string.contains("'"), "a straight apostrophe: \(string)")
        #expect(!string.contains(" — "), "a spaced em dash: \(string)")
        #expect(!string.contains("..."), "three periods, not an ellipsis: \(string)")
        #expect(!string.hasSuffix(" ") && !string.isEmpty, "\(string)")
        for phrase in stock { #expect(!string.lowercased().contains(phrase), "\"\(phrase)\": \(string)") }
        // British spellings the owner has ruled out.
        for word in ["colour", "authoris", "organis", "centre", "behaviour"] {
            #expect(!string.lowercased().contains(word), "US English: \(string)")
        }
        // The owner dropped the token and privacy line (ov-519).
        #expect(!string.lowercased().contains("token"), "\(string)")
    }
}

@Test func buttonsAreTitleCase() {
    // Apple capitalizes pronouns: "Talk It Through", never "Talk it Through".
    let small: Set = ["a", "an", "and", "the", "to", "of", "in", "on"]
    for button in FirstRunCopy.Charter.buttons {
        for (i, word) in button.split(separator: " ").enumerated() where i == 0 || !small.contains(String(word)) {
            #expect(word.first?.isUppercase == true, "\(button)")
        }
    }
    // A button that leads to more talk says so.
    #expect(FirstRunCopy.Charter.talkItThrough.hasSuffix("…"))
}

@Test func theCharterScreenNamesTheRepositoryAndAsksNoQuestionAboutPhones() {
    #expect(FirstRunCopy.Charter.title("harbor") == "How should agents work in harbor?")
    #expect(FirstRunCopy.Charter.readingTitle("harbor") == "Reading harbor")
    #expect(!FirstRunCopy.Charter.all.contains { $0.contains("Questions reach you") })
}
