//! What `farcooler page schema` prints: the nine blocks with one example each,
//! and the same vocabulary as JSON Schema (ov-269, ov-281), live data
//! references included (ov-306).
//!
//! Written by hand beside `page_doc`, because no schema generator is in the
//! tree. What keeps it honest is the tests: every example here is parsed by the
//! validator, every block and reference kind is named in both outputs, and the
//! JSON Schema's limits are read off [`Caps::default`].

use serde_json::{Value, json};

use crate::page_doc::{BLOCK_TYPES, Caps, STALE_AFTER_MAX_MIN, State, Target, VERSION};

/// One example per block, as the CLI prints it and a test parses it.
pub const EXAMPLES: [(&str, &str); 9] = [
    ("heading", r#"{"type":"heading","text":"Lanes"}"#),
    ("text", r#"{"type":"text","md":"Phones and the LFS record: **one build**, one review.","tone":"attention"}"#),
    (
        "stats",
        r#"{"type":"stats","items":[{"label":"Main","ref":{"ci":"main"}},{"label":"In review","ref":{"cards":"in_review"}},{"label":"Fixing","value":"1","tone":"attention"}]}"#,
    ),
    ("progress", r#"{"type":"progress","label":"Cards closed","done":2,"total":4,"detail":"since Monday"}"#),
    (
        "table",
        r#"{"type":"table","columns":[{"title":"Lane"},{"title":"State","grow":true}],"rows":[[{"ref":{"lane":"mac-ux"}},{"ref":{"lane":"mac-ux"},"show":"state"}],["notes","Fixing, round 1"]]}"#,
    ),
    (
        "list",
        r#"{"type":"list","items":[{"text":"Should Plan hide Unread on phones?","state":"waiting","ref":{"ask":"ov-274"}}]}"#,
    ),
    (
        "timeline",
        r#"{"type":"timeline","entries":[{"at":"2026-10-04T15:02:00-07:00","text":"Picked four lanes","ref":{"task":"ov-274"}}]}"#,
    ),
    (
        "steps",
        r#"{"type":"steps","steps":[{"label":"Build","state":"done"},{"label":"Review","state":"active"},{"label":"Land","state":"todo"}]}"#,
    ),
    (
        "links",
        r#"{"type":"links","items":[{"page":"spend"},{"url":"https://github.com/example/repo/actions/runs/812","label":"CI run"}]}"#,
    ),
];

/// The block reference `page schema` prints.
pub fn reference_text() -> String {
    let caps = Caps::default();
    let mut out = String::new();
    out += &format!(
        "A page is one JSON document: {{\"v\":{VERSION},\"title\":\"...\",\"summary\":\"...\",\"blocks\":[...]}}.\n"
    );
    out += &format!(
        "The title is at most {} characters and the summary at most {}. Publishing replaces the page whole.\n\n",
        caps.title_chars, caps.summary_chars
    );
    out += &format!("The blocks are {}. Each has \"type\" and these fields (? is optional):\n\n", crate::page_doc::BLOCK_TYPES.join(", "));
    let fields = [
        ("heading", "text"),
        ("text", "md (a Markdown subset: paragraphs, bullets, bold, italic, code, https links whose words don't name another domain), tone?"),
        ("stats", "items[1..6] of {label, value and/or ref (with show?), detail?, tone?}; beside a ref, value is what older apps draw"),
        ("progress", "label, done, total, detail?, parts?[..6] of {label, count}"),
        ("table", "columns[1..8] of {title, align?, grow?}, rows[..50] of one cell per column; a cell is a string or {text?, ref?, show?, tone?, mono?}"),
        ("list", "items[1..50] of {text, state?, detail?, ref?, tone?}"),
        ("timeline", "order? (newest or given), entries[1..50] of {at, text, ref?}; at is RFC 3339 or milliseconds"),
        ("steps", "steps[2..12] of {label, state}"),
        ("links", "items[1..12], each a reference"),
    ];
    for (kind, shape) in fields {
        let example = EXAMPLES.iter().find(|(k, _)| *k == kind).map_or("", |(_, e)| *e);
        out += &format!("  {kind}: {shape}\n    {example}\n");
    }
    out += &format!("\nA state is {}; its word is always drawn. A tone is neutral or attention (amber, for what needs the owner).\n", crate::page_doc::State::WORDS.join(", "));
    out += &format!(
        "A reference names one of {}, and can have a label: {{\"lane\":\"mac-ux\"}}, {{\"terminal\":{{\"worktree\":\"integ-10\",\"name\":\"build\"}}}}.\n",
        Target::KINDS.join(", ")
    );
    out += "The app draws a reference's current state, so prefer a reference to a copy of a status. A link is https only, and its domain is drawn beside it.\n";
    out += "In a cell or a figure, show picks the live value: state (a lane's), spend (a lane's or a theme's); with text, the text is drawn and the ref is only the link.\n";
    out += &format!(
        "Live data is a reference too: {{\"ci\":\"main\"}}, {{\"ci\":\"<sha>\"}} or {{\"ci\":\"run:<id>\"}} (read through gh while named), {{\"cards\":\"in_review\"}} ({}). Never type a figure the board knows.\n",
        crate::page_doc::CARD_STATUSES.join(", ")
    );
    out += &format!(
        "\nLimits: {} KiB, {} blocks, {} characters in md, {} in any other string, {} references, {} pages per workspace.\n",
        caps.document_bytes / 1024,
        caps.blocks,
        caps.md_chars,
        caps.string_chars,
        caps.refs,
        crate::page_doc::MAX_PAGES
    );
    out += "Unknown fields are refused, so a misspelling fails when you publish. Don't put anything on a page you wouldn't put in a task note: everyone who can read the board can read it.\n";
    out
}

fn string(max: usize) -> Value {
    json!({"type": "string", "minLength": 1, "maxLength": max})
}

fn tone() -> Value {
    json!({"enum": ["neutral", "attention"]})
}

fn state() -> Value {
    json!({"enum": State::WORDS})
}

fn reference_schema(caps: &Caps) -> Value {
    json!({
        "type": "object",
        "description": "Exactly one target key, and an optional label.",
        "properties": {
            "task": string(40), "ask": string(40), "lane": string(caps.string_chars),
            "theme": string(caps.string_chars), "worktree": string(caps.string_chars),
            "page": {"type": "string", "pattern": "^[a-z0-9][a-z0-9-]{0,39}$"},
            "terminal": {
                "type": "object", "additionalProperties": false, "required": ["worktree", "name"],
                "properties": {"worktree": string(caps.string_chars), "name": string(caps.string_chars)},
            },
            "url": {"type": "string", "pattern": "^https://", "maxLength": caps.string_chars},
            "ci": {"type": "string", "pattern": "^(main|[0-9A-Fa-f]{7,40}|run:[0-9]{1,20})$"},
            "cards": {"enum": crate::page_doc::CARD_STATUSES},
            "label": string(caps.string_chars),
        },
        "additionalProperties": false,
        "minProperties": 1,
        "maxProperties": 2,
    })
}

/// The vocabulary as JSON Schema (draft 2020-12), for editors and tests.
///
/// It says what a document looks like; the validator also checks what a schema
/// can't (a row's cell count against its columns, a bar's parts against its
/// total, that a `show` goes with a lane).
pub fn json_schema() -> Value {
    let caps = Caps::default();
    let s = caps.string_chars;
    let cell = json!({
        "oneOf": [
            {"type": "string", "maxLength": s},
            {
                "type": "object", "additionalProperties": false, "minProperties": 1,
                "properties": {
                    "text": {"type": "string", "maxLength": s}, "ref": reference_schema(&caps),
                    "show": {"enum": ["state", "spend"]}, "tone": tone(), "mono": {"type": "boolean"},
                },
            },
        ],
    });
    let block = |kind: &str, required: &[&str], properties: Value| {
        let mut props = properties;
        props["type"] = json!({"const": kind});
        let mut needed = vec!["type"];
        needed.extend_from_slice(required);
        json!({"type": "object", "additionalProperties": false, "required": needed, "properties": props})
    };
    let blocks = vec![
        block("heading", &["text"], json!({"text": string(s)})),
        block("text", &["md"], json!({"md": {"type": "string", "minLength": 1, "maxLength": caps.md_chars}, "tone": tone()})),
        block(
            "stats",
            &["items"],
            json!({"items": {"type": "array", "minItems": 1, "maxItems": caps.stats_items, "items": {
                "type": "object", "additionalProperties": false, "required": ["label"],
                "properties": {
                    "label": string(s), "value": string(s), "ref": reference_schema(&caps),
                    "show": {"enum": ["state", "spend"]}, "detail": string(s), "tone": tone(),
                },
            }}}),
        ),
        block(
            "progress",
            &["label", "done", "total"],
            json!({
                "label": string(s), "done": {"type": "integer", "minimum": 0}, "total": {"type": "integer", "minimum": 1},
                "detail": string(s),
                "parts": {"type": "array", "minItems": 1, "maxItems": caps.progress_parts, "items": {
                    "type": "object", "additionalProperties": false, "required": ["label", "count"],
                    "properties": {"label": string(s), "count": {"type": "integer", "minimum": 0}},
                }},
            }),
        ),
        block(
            "table",
            &["columns", "rows"],
            json!({
                "columns": {"type": "array", "minItems": 1, "maxItems": caps.table_columns, "items": {
                    "type": "object", "additionalProperties": false, "required": ["title"],
                    "properties": {"title": string(s), "align": {"enum": ["start", "center", "end"]}, "grow": {"type": "boolean"}},
                }},
                "rows": {"type": "array", "maxItems": caps.table_rows, "items": {"type": "array", "items": cell}},
            }),
        ),
        block(
            "list",
            &["items"],
            json!({"items": {"type": "array", "minItems": 1, "maxItems": caps.list_items, "items": {
                "type": "object", "additionalProperties": false, "required": ["text"],
                "properties": {"text": string(s), "state": state(), "detail": string(s), "ref": reference_schema(&caps), "tone": tone()},
            }}}),
        ),
        block(
            "timeline",
            &["entries"],
            json!({
                "order": {"enum": ["newest", "given"]},
                "entries": {"type": "array", "minItems": 1, "maxItems": caps.timeline_entries, "items": {
                    "type": "object", "additionalProperties": false, "required": ["at", "text"],
                    "properties": {"at": {"oneOf": [{"type": "string", "format": "date-time"}, {"type": "integer", "minimum": 0}]},
                                   "text": string(s), "ref": reference_schema(&caps)},
                }},
            }),
        ),
        block(
            "steps",
            &["steps"],
            json!({"steps": {"type": "array", "minItems": 2, "maxItems": caps.steps, "items": {
                "type": "object", "additionalProperties": false, "required": ["label", "state"],
                "properties": {"label": string(s), "state": state()},
            }}}),
        ),
        block(
            "links",
            &["items"],
            json!({"items": {"type": "array", "minItems": 1, "maxItems": caps.links_items, "items": reference_schema(&caps)}}),
        ),
    ];
    debug_assert_eq!(blocks.len(), BLOCK_TYPES.len());
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "A Far Cooler orchestrator page",
        "type": "object",
        "additionalProperties": false,
        "required": ["v", "title", "blocks"],
        "properties": {
            "v": {"const": VERSION},
            "title": string(caps.title_chars),
            "summary": {"type": "string", "maxLength": caps.summary_chars},
            "glance": string(caps.glance_chars),
            "stale_after_min": {"type": "integer", "minimum": 1, "maximum": STALE_AFTER_MAX_MIN},
            "blocks": {"type": "array", "minItems": 1, "maxItems": caps.blocks, "items": {"oneOf": blocks}},
        },
    })
}
