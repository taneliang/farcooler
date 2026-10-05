import Foundation

// Orchestrator pages (ov-269, the Mac's half is ov-284): a small JSON document
// of typed blocks an orchestrator publishes to a slot on a board, drawn here in
// the app's own type and tokens. The runner validates every page it stores
// (`crates/core/src/page_doc.rs`), and this is the reader: it decodes what
// `farcooler page list --json` and `page show --json` print, the shape held to
// `test/fixtures/page.json` and `test/fixtures/pages/normalized/`.
//
// EXPERIMENTAL, behind `board_pages`, and removable with the rest of the
// feature (design section 4.3): delete PageModel, PageLive and PageView.
//
// A reader is lenient where the runner is strict. A runner newer than this app
// may send a block or a field this build doesn't know, so an unknown block
// decodes to `.unknown` (drawn as its `alt`), an unknown state is `.none`, an
// unknown tone is neutral, and a block that won't decode costs that block, not
// the page.

/// One page as the runner lists it: its row in the overview, and its document
/// when it was read with one.
public struct BoardPage: Decodable, Equatable, Identifiable, Sendable {
    public var id: String
    public var short: String
    /// The name the orchestrator publishes it under: `train`, `spend`.
    public var slot: String
    public var title: String
    public var summary: String
    /// `""`, or `"theme"` when it's drawn inside a theme's page.
    public var anchorKind: String
    /// The theme's id when anchored.
    public var anchor: String
    public var revision: Int64
    public var ordinal: Int64
    public var actor: String
    public var updatedAtMs: Int64
    /// Nil when it was listed without its document, or the document is one
    /// this build can't read at all.
    public var doc: PageDoc?

    public init(
        id: String, short: String = "", slot: String, title: String, summary: String = "", anchorKind: String = "",
        anchor: String = "", revision: Int64 = 1, ordinal: Int64 = 0, actor: String = "", updatedAtMs: Int64 = 0,
        doc: PageDoc? = nil
    ) {
        self.id = id
        self.short = short
        self.slot = slot
        self.title = title
        self.summary = summary
        self.anchorKind = anchorKind
        self.anchor = anchor
        self.revision = revision
        self.ordinal = ordinal
        self.actor = actor
        self.updatedAtMs = updatedAtMs
        self.doc = doc
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        short = try c.decodeIfPresent(String.self, forKey: .short) ?? ""
        slot = try c.decode(String.self, forKey: .slot)
        title = try c.decodeIfPresent(String.self, forKey: .title) ?? ""
        summary = try c.decodeIfPresent(String.self, forKey: .summary) ?? ""
        anchorKind = try c.decodeIfPresent(String.self, forKey: .anchorKind) ?? ""
        anchor = try c.decodeIfPresent(String.self, forKey: .anchor) ?? ""
        revision = try c.decodeIfPresent(Int64.self, forKey: .revision) ?? 0
        ordinal = try c.decodeIfPresent(Int64.self, forKey: .ordinal) ?? 0
        actor = try c.decodeIfPresent(String.self, forKey: .actor) ?? ""
        updatedAtMs = try c.decodeIfPresent(Int64.self, forKey: .updatedAtMs) ?? 0
        doc = try? c.decodeIfPresent(PageDoc.self, forKey: .doc)
    }

    private enum CodingKeys: String, CodingKey {
        case id, short, slot, title, summary, anchorKind, anchor, revision, ordinal, actor, updatedAtMs, doc
    }

    /// The theme it's drawn inside, by id, or nil when it's a page of its own.
    public var themeAnchor: String? { anchorKind == "theme" && !anchor.isEmpty ? anchor : nil }

    /// `page show --json`.
    public static func decode(_ data: Data) throws -> BoardPage {
        try PlanJSON.decoder.decode(BoardPage.self, from: data)
    }
}

/// `page list --json`: `{"pages": [...]}`, in the runner's order.
public struct BoardPageList: Decodable, Equatable, Sendable {
    public var pages: [BoardPage]

    public init(pages: [BoardPage]) { self.pages = pages }

    public static func decode(_ data: Data) throws -> BoardPageList {
        try PlanJSON.decoder.decode(BoardPageList.self, from: data)
    }
}

/// The document: a title, a one-line summary and its blocks.
public struct PageDoc: Decodable, Equatable, Sendable {
    public var v: Int
    public var title: String
    public var summary: String
    /// At most 60 characters, for a glance; nothing in v1 draws it.
    public var glance: String?
    /// Minutes after which "Updated …" reads "Not updated for …".
    public var staleAfterMin: Int?
    public var blocks: [PageBlock]

    public init(v: Int = 1, title: String, summary: String = "", glance: String? = nil, staleAfterMin: Int? = nil, blocks: [PageBlock]) {
        self.v = v
        self.title = title
        self.summary = summary
        self.glance = glance
        self.staleAfterMin = staleAfterMin
        self.blocks = blocks
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        var clamped = false
        v = try c.decodeIfPresent(Int.self, forKey: .v) ?? 1
        title = PageCaps.cut(try c.decodeIfPresent(String.self, forKey: .title) ?? "", PageCaps.title, &clamped)
        summary = PageCaps.cut(try c.decodeIfPresent(String.self, forKey: .summary) ?? "", PageCaps.summary, &clamped)
        glance = try c.decodeIfPresent(String.self, forKey: .glance).map { PageCaps.cut($0, PageCaps.glance, &clamped) }
        staleAfterMin = try c.decodeIfPresent(Int.self, forKey: .staleAfterMin)
        // One block at a time, so a block this build can't read is one
        // `.unknown`, never a page that doesn't draw.
        var list = try c.nestedUnkeyedContainer(forKey: .blocks)
        var blocks: [PageBlock] = []
        // What's left of the document's 32 KiB, counted in the bytes of the
        // words drawn (never more than they serialize to, so a page the
        // runner took always fits), and of its 200 references.
        var bytes = PageCaps.documentBytes
        var refs = PageCaps.refs
        while !list.isAtEnd {
            let any = try list.decode(PageAny.self)
            guard blocks.count < PageCaps.blocks else {
                clamped = true
                break
            }
            var cut = false
            let block = PageBlock(any, clamped: &cut).limitingRefs(&refs, clamped: &cut)
            bytes -= block.textBytes
            guard bytes >= 0 else {
                clamped = true
                break
            }
            blocks.append(block)
            clamped = clamped || cut
        }
        // More than the design allows (a runner that didn't check, or a page
        // edited by hand): what fits is drawn, then one line says so. Never a
        // hang over thousands of rows.
        if clamped { blocks.append(.unknown(type: PageCaps.tooLargeType, alt: PageWords.tooLarge)) }
        self.blocks = blocks
    }

    private enum CodingKeys: String, CodingKey { case v, title, summary, glance, staleAfterMin, blocks }

    /// The blocks to draw under a header that already says `title`: a first
    /// heading that only repeats it is left out, so an anchored section
    /// doesn't say "Risks" twice.
    public func blocks(under title: String) -> [PageBlock] {
        if case .heading(let text)? = blocks.first,
            text.trimmingCharacters(in: .whitespaces).caseInsensitiveCompare(title.trimmingCharacters(in: .whitespaces)) == .orderedSame
        {
            return Array(blocks.dropFirst())
        }
        return blocks
    }

    /// This document under a header that says `title` (`blocks(under:)`).
    public func under(_ title: String) -> PageDoc {
        var doc = self
        doc.blocks = blocks(under: title)
        return doc
    }

    /// A document alone: `test/fixtures/pages/normalized/*.json`.
    public static func decode(_ data: Data) throws -> PageDoc {
        try PlanJSON.decoder.decode(PageDoc.self, from: data)
    }
}

/// A state with a glyph and a word: one fixed set for list items and steps.
public enum PageState: String, Sendable, CaseIterable {
    case done, active, waiting, blocked, failed, todo, none

    /// What it's called, always drawn or spoken beside its glyph. Nil for
    /// `none`, which draws neither.
    public var word: String? {
        switch self {
        case .done: "Done"
        case .active: "Active"
        case .waiting: "Waiting"
        case .blocked: "Blocked"
        case .failed: "Failed"
        case .todo: "To Do"
        case .none: nil
        }
    }

    /// Its SF Symbol: a shape to scan by, never the only signal.
    public var symbol: String {
        switch self {
        case .done: "checkmark.circle"
        case .active: "circle.inset.filled"
        case .waiting: "circle.lefthalf.filled"
        case .blocked: "nosign"
        case .failed: "xmark.circle"
        case .todo: "circle"
        case .none: "circle.dotted"
        }
    }

    init(word: String?) { self = word.flatMap(PageState.init(rawValue:)) ?? .none }
}

/// Neutral, or the amber the app uses for "Needs you". There's no other color.
public enum PageTone: Sendable, Equatable {
    case neutral, attention

    init(word: String?) { self = word == "attention" ? .attention : .neutral }
}

/// What a reference points at: something the app can already open.
public enum PageTarget: Equatable, Hashable, Sendable {
    case task(String)
    case ask(String)
    case lane(String)
    case theme(String)
    case page(String)
    case worktree(String)
    case terminal(worktree: String, name: String)
    case url(String)
    /// CI (ov-306): `main`, a commit's SHA, or `run:<id>`, read by the runner
    /// through `gh` while a page names it.
    case ci(String)
    /// How many of the board's cards are in a status (ov-306): a status's
    /// word, or `open`.
    case cards(String)
    /// A target this build doesn't know: drawn as its label, as plain text.
    case unknown

    /// The name it was written with: what an unresolved reference draws.
    public var rawName: String {
        switch self {
        case .task(let s), .ask(let s), .lane(let s), .theme(let s), .page(let s), .worktree(let s), .url(let s),
            .ci(let s), .cards(let s):
            s
        case .terminal(let worktree, let name): "\(worktree)/\(name)"
        case .unknown: ""
        }
    }

    /// The CI subject a `ci` target is read under: `main`, `run:<id>` or
    /// `sha:<sha>`.
    public var ciSubject: String? {
        guard case .ci(let s) = self else { return nil }
        return s == "main" || s.hasPrefix("run:") ? s : "sha:\(s)"
    }
}

/// A reference: one target, and the words to draw for it when it has them.
public struct PageRef: Equatable, Sendable {
    public var target: PageTarget
    public var label: String?

    public init(_ target: PageTarget, label: String? = nil) {
        self.target = target
        self.label = label
    }

    init?(_ any: PageAny?) {
        guard let o = any?.object else { return nil }
        label = o["label"]?.string
        if let s = o["task"]?.string {
            target = .task(s)
        } else if let s = o["ask"]?.string {
            target = .ask(s)
        } else if let s = o["lane"]?.string {
            target = .lane(s)
        } else if let s = o["theme"]?.string {
            target = .theme(s)
        } else if let s = o["page"]?.string {
            target = .page(s)
        } else if let s = o["worktree"]?.string {
            target = .worktree(s)
        } else if let t = o["terminal"]?.object, let w = t["worktree"]?.string, let n = t["name"]?.string {
            target = .terminal(worktree: w, name: n)
        } else if let s = o["url"]?.string {
            target = .url(s)
        } else if let s = o["ci"]?.string {
            target = .ci(s.lowercased())
        } else if let s = o["cards"]?.string {
            target = .cards(s)
        } else {
            target = .unknown
        }
    }
}

/// What a reference cell draws when it has no text of its own.
public enum PageShow: Sendable, Equatable {
    /// The target's name.
    case name
    /// A lane's state words: "Fixing · round 1".
    case state
    /// A lane's tokens, or a theme's share of its lanes' (ov-306).
    case spend
}

/// A table cell: text, a live reference, or text that links somewhere.
public struct PageCell: Equatable, Sendable {
    public var text: String?
    public var ref: PageRef?
    public var show: PageShow
    public var tone: PageTone
    public var mono: Bool

    public init(text: String? = nil, ref: PageRef? = nil, show: PageShow = .name, tone: PageTone = .neutral, mono: Bool = false) {
        self.text = text
        self.ref = ref
        self.show = show
        self.tone = tone
        self.mono = mono
    }

    init(_ any: PageAny) {
        if let s = any.string {
            self.init(text: s)
            return
        }
        let o = any.object ?? [:]
        let show = PageShow(word: o["show"]?.string)
        self.init(
            text: o["text"]?.string, ref: PageRef(o["ref"]), show: show, tone: PageTone(word: o["tone"]?.string),
            mono: o["mono"]?.bool ?? false)
    }
}

extension PageShow {
    init(word: String?) {
        switch word {
        case "state": self = .state
        case "spend": self = .spend
        default: self = .name
        }
    }
}

/// A figure in a `stats` row: a value as written, or a reference drawn live
/// (ov-306), when `value` is empty.
public struct PageStat: Equatable, Sendable {
    public var label: String
    public var value: String
    public var detail: String?
    public var tone: PageTone
    public var ref: PageRef?
    public var show: PageShow

    public init(
        label: String, value: String, detail: String? = nil, tone: PageTone = .neutral, ref: PageRef? = nil,
        show: PageShow = .name
    ) {
        self.label = label
        self.value = value
        self.detail = detail
        self.tone = tone
        self.ref = ref
        self.show = show
    }
}

/// A column of a table.
public struct PageColumn: Equatable, Sendable {
    public enum Align: Sendable, Equatable { case start, center, end }
    public var title: String
    public var align: Align
    /// Takes the width the others leave.
    public var grow: Bool

    public init(title: String, align: Align = .start, grow: Bool = false) {
        self.title = title
        self.align = align
        self.grow = grow
    }
}

/// A row of a `list` block.
public struct PageItem: Equatable, Sendable {
    public var text: String
    public var state: PageState
    public var detail: String?
    public var ref: PageRef?
    public var tone: PageTone

    public init(text: String, state: PageState = .none, detail: String? = nil, ref: PageRef? = nil, tone: PageTone = .neutral) {
        self.text = text
        self.state = state
        self.detail = detail
        self.ref = ref
        self.tone = tone
    }
}

/// An entry on a timeline: milliseconds since 1970, as the runner stores it.
public struct PageEntry: Equatable, Sendable {
    public var at: Int64
    public var text: String
    public var ref: PageRef?

    public init(at: Int64, text: String, ref: PageRef? = nil) {
        self.at = at
        self.text = text
        self.ref = ref
    }
}

/// A step in a pipeline.
public struct PageStep: Equatable, Sendable {
    public var label: String
    public var state: PageState

    public init(label: String, state: PageState) {
        self.label = label
        self.state = state
    }
}

/// A part of a progress bar.
public struct PagePart: Equatable, Sendable {
    public var label: String
    public var count: Int

    public init(label: String, count: Int) {
        self.label = label
        self.count = count
    }
}

/// The nine blocks, and a tenth for any this build doesn't know.
public enum PageBlock: Equatable, Sendable {
    case heading(String)
    case text(md: String, tone: PageTone)
    case stats([PageStat])
    case progress(label: String, done: Int, total: Int, detail: String?, parts: [PagePart])
    case table(columns: [PageColumn], rows: [[PageCell]])
    case list([PageItem])
    /// Newest first, unless the orchestrator asked for its own order.
    case timeline([PageEntry], given: Bool)
    case steps([PageStep])
    case links([PageRef])
    /// A block from a newer runner, or one that wouldn't decode: its `alt`,
    /// when it has one.
    case unknown(type: String, alt: String?)

    init(_ any: PageAny) {
        var ignored = false
        self.init(any, clamped: &ignored)
    }

    /// A block, held to the design's caps (`PageCaps`); `clamped` is set when
    /// anything was left out or cut short.
    init(_ any: PageAny, clamped: inout Bool) {
        let o = any.object ?? [:]
        let type = o["type"]?.string ?? ""
        var cut = false
        defer { clamped = clamped || cut }
        func items(_ key: String) -> [PageAny] { PageCaps.prefix(o[key]?.array ?? [], PageCaps.items(type, key), &cut) }
        func text(_ value: String?, _ limit: Int = PageCaps.string) -> String? { value.map { PageCaps.cut($0, limit, &cut) } }
        let alt = text(o["alt"]?.string, PageCaps.alt)
        switch type {
        case "heading":
            guard let heading = text(o["text"]?.string) else { break }
            self = .heading(heading)
            return
        case "text":
            guard let md = text(o["md"]?.string, PageCaps.md) else { break }
            self = .text(md: md, tone: PageTone(word: o["tone"]?.string))
            return
        case "stats":
            self = .stats(
                items("items").compactMap { s in
                    let s = s.object ?? [:]
                    let ref = PageRef(s["ref"])
                    guard let label = s["label"]?.string, let value = s["value"]?.string ?? (ref == nil ? nil : "") else {
                        return nil
                    }
                    return PageStat(
                        label: label, value: value, detail: s["detail"]?.string, tone: PageTone(word: s["tone"]?.string),
                        ref: ref, show: PageShow(word: s["show"]?.string))
                })
            return
        case "progress":
            guard let label = o["label"]?.string, let done = o["done"]?.int, let total = o["total"]?.int else { break }
            let parts = items("parts").compactMap { p -> PagePart? in
                guard let label = p.object?["label"]?.string, let count = p.object?["count"]?.int else { return nil }
                return PagePart(label: label, count: count)
            }
            self = .progress(label: label, done: done, total: total, detail: o["detail"]?.string, parts: parts)
            return
        case "table":
            let columns = items("columns").map { c in
                let c = c.object ?? [:]
                let align: PageColumn.Align =
                    switch c["align"]?.string {
                    case "end": .end
                    case "center": .center
                    default: .start
                    }
                return PageColumn(title: c["title"]?.string ?? "", align: align, grow: c["grow"]?.bool ?? false)
            }
            guard !columns.isEmpty else { break }
            self = .table(
                columns: columns,
                rows: items("rows").map { row in
                    PageCaps.prefix(row.array ?? [], columns.count, &cut).map { cell in
                        var cell = PageCell(cell)
                        cell.text = cell.text.map { PageCaps.cut($0, PageCaps.string, &cut) }
                        return cell
                    }
                })
            return
        case "list":
            self = .list(
                items("items").compactMap { i in
                    let i = i.object ?? [:]
                    guard let words = text(i["text"]?.string) else { return nil }
                    return PageItem(
                        text: words, state: PageState(word: i["state"]?.string), detail: text(i["detail"]?.string),
                        ref: PageRef(i["ref"]), tone: PageTone(word: i["tone"]?.string))
                })
            return
        case "timeline":
            let entries = items("entries").compactMap { e -> PageEntry? in
                let e = e.object ?? [:]
                guard let at = e["at"]?.int64, let words = text(e["text"]?.string) else { return nil }
                return PageEntry(at: at, text: words, ref: PageRef(e["ref"]))
            }
            self = .timeline(entries, given: o["order"]?.string == "given")
            return
        case "steps":
            self = .steps(
                items("steps").compactMap { s in
                    guard let label = s.object?["label"]?.string else { return nil }
                    return PageStep(label: label, state: PageState(word: s.object?["state"]?.string))
                })
            return
        case "links":
            self = .links(items("items").compactMap { PageRef($0) })
            return
        default:
            break
        }
        self = .unknown(type: type, alt: alt)
    }
}

extension PageBlock {
    /// The UTF-8 bytes of the words it draws: less than it takes written
    /// out, so what it counts against the document's cap never over-counts.
    var textBytes: Int {
        func n(_ s: String?) -> Int { s?.utf8.count ?? 0 }
        func ref(_ r: PageRef?) -> Int { n(r?.label) + n(r?.target.rawName) }
        switch self {
        case .heading(let text): return n(text)
        case .text(let md, _): return n(md)
        case .stats(let items): return items.reduce(0) { $0 + n($1.label) + n($1.value) + n($1.detail) + ref($1.ref) }
        case .progress(let label, _, _, let detail, let parts): return n(label) + n(detail) + parts.reduce(0) { $0 + n($1.label) }
        case .table(let columns, let rows):
            return columns.reduce(0) { $0 + n($1.title) } + rows.joined().reduce(0) { $0 + n($1.text) + ref($1.ref) }
        case .list(let items): return items.reduce(0) { $0 + n($1.text) + n($1.detail) + ref($1.ref) }
        case .timeline(let entries, _): return entries.reduce(0) { $0 + n($1.text) + ref($1.ref) }
        case .steps(let steps): return steps.reduce(0) { $0 + n($1.label) }
        case .links(let refs): return refs.reduce(0) { $0 + ref($1) }
        case .unknown(_, let alt): return n(alt)
        }
    }

    /// This block with its references held to what's left of the page's
    /// `budget`: past it, a reference is dropped and its words draw as plain
    /// text, and `clamped` says so.
    func limitingRefs(_ budget: inout Int, clamped: inout Bool) -> PageBlock {
        func keep(_ ref: PageRef?) -> PageRef? {
            guard let ref else { return nil }
            guard budget > 0 else {
                clamped = true
                return nil
            }
            budget -= 1
            return ref
        }
        switch self {
        case .table(let columns, let rows):
            return .table(
                columns: columns,
                rows: rows.map { row in
                    row.map { cell in
                        var cell = cell
                        if let ref = cell.ref, keep(ref) == nil {
                            // Words a reference drew live become the name it
                            // was written with.
                            cell.text = cell.text ?? (ref.label ?? ref.target.rawName)
                            cell.ref = nil
                        }
                        return cell
                    }
                })
        case .list(let items):
            return .list(items.map { item in
                var item = item
                item.ref = keep(item.ref)
                return item
            })
        case .timeline(let entries, let given):
            return .timeline(entries.map { entry in
                var entry = entry
                entry.ref = keep(entry.ref)
                return entry
            }, given: given)
        case .links(let refs):
            return .links(refs.compactMap { keep($0) })
        case .stats(let items):
            return .stats(items.map { stat in
                var stat = stat
                if let ref = stat.ref, keep(ref) == nil {
                    stat.value = stat.value.isEmpty ? (ref.label ?? ref.target.rawName) : stat.value
                    stat.ref = nil
                }
                return stat
            })
        default:
            return self
        }
    }
}

/// Any JSON value, read loosely: what lets one malformed block cost only
/// itself.
struct PageAny: Decodable, Equatable, Sendable {
    enum Value: Equatable, Sendable {
        case string(String), number(Double), bool(Bool), array([PageAny]), object([String: PageAny]), null
    }
    var value: Value

    init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if c.decodeNil() {
            value = .null
        } else if let b = try? c.decode(Bool.self) {
            value = .bool(b)
        } else if let n = try? c.decode(Double.self) {
            value = .number(n)
        } else if let s = try? c.decode(String.self) {
            value = .string(s)
        } else if let a = try? c.decode([PageAny].self) {
            value = .array(a)
        } else {
            value = .object(try c.decode([String: PageAny].self))
        }
    }

    var string: String? { if case .string(let s) = value { s } else { nil } }
    var bool: Bool? { if case .bool(let b) = value { b } else { nil } }
    var array: [PageAny]? { if case .array(let a) = value { a } else { nil } }
    var object: [String: PageAny]? { if case .object(let o) = value { o } else { nil } }
    var int64: Int64? {
        if case .number(let n) = value, n.isFinite, n == n.rounded(), abs(n) < 9.0e15 { Int64(n) } else { nil }
    }
    var int: Int? { int64.map { Int(clamping: $0) } }
}

/// The design's limits (section 7), held by the reader as well as the runner,
/// so a page that breaks them draws what fits and says the rest didn't.
public enum PageCaps {
    public static let blocks = 60
    public static let rows = 50
    public static let columns = 8
    public static let items = 50
    public static let md = 2_000
    public static let string = 200
    public static let alt = 500
    public static let title = 60
    public static let summary = 120
    public static let glance = 60
    /// A document, serialized.
    public static let documentBytes = 32 * 1024
    /// References on one page.
    public static let refs = 200
    /// The block a clamped page ends with.
    static let tooLargeType = "too-large"

    /// The most of `key` a `type` block may hold.
    static func items(_ type: String, _ key: String) -> Int {
        switch (type, key) {
        case ("stats", _): 6
        case ("steps", _), ("links", _): 12
        case ("progress", _): 6
        case ("table", "columns"): columns
        case ("table", _): rows
        default: items
        }
    }

    static func prefix<T>(_ values: [T], _ limit: Int, _ cut: inout Bool) -> [T] {
        guard values.count > limit else { return values }
        cut = true
        return Array(values.prefix(limit))
    }

    static func cut(_ text: String, _ limit: Int, _ cut: inout Bool) -> String {
        guard text.count > limit else { return text }
        cut = true
        return String(text.prefix(limit - 1)) + "…"
    }
}
