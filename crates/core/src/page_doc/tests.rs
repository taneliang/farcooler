use std::path::PathBuf;

use serde_json::Value;

use super::parse::rfc3339_for_test;
use super::*;
use crate::page_schema;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/pages")
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(fixtures().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// The documents an orchestrator would publish, as written.
const ACCEPTED: [&str; 6] = ["train", "spend", "risks", "blocks", "refs", "live"];

fn page(name: &str) -> Page {
    parse(&read(&format!("{name}.json")), &Caps::default()).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn refusal(json: &str) -> PageError {
    parse(json, &Caps::default()).unwrap_err()
}

/// Every accepted fixture normalizes to the committed document, which is what
/// the apps' decoder tests read. Run with `FARCOOLER_WRITE_FIXTURES=1` to
/// rewrite them after a deliberate change.
#[test]
fn the_accepted_documents_normalize_to_the_fixtures_the_apps_read() {
    for name in ACCEPTED {
        let json = page(name).to_json();
        let rel = format!("normalized/{name}.json");
        if std::env::var_os("FARCOOLER_WRITE_FIXTURES").is_some() {
            std::fs::create_dir_all(fixtures().join("normalized")).unwrap();
            let pretty = serde_json::to_string_pretty(&serde_json::from_str::<Value>(&json).unwrap()).unwrap();
            std::fs::write(fixtures().join(&rel), pretty + "\n").unwrap();
        }
        let committed: Value = serde_json::from_str(&read(&rel)).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&json).unwrap(), committed, "{name}");
    }
}

/// Reading what was written changes nothing: the store re-reads its own
/// documents with this validator.
#[test]
fn normalizing_twice_is_the_same_document() {
    for name in ACCEPTED {
        let once = page(name);
        let twice = parse(&once.to_json(), &Caps::default()).unwrap();
        assert_eq!(once, twice, "{name}");
        assert_eq!(once.to_json(), twice.to_json(), "{name}");
    }
}

/// Times are milliseconds once stored, from any of the ways they're written.
#[test]
fn times_become_milliseconds() {
    assert_eq!(rfc3339_for_test("1970-01-01T00:00:00Z"), Some(0));
    assert_eq!(rfc3339_for_test("2000-03-01T00:00:00Z"), Some(951_868_800_000));
    assert_eq!(rfc3339_for_test("2024-02-29T12:00:00Z"), Some(1_709_208_000_000));
    assert_eq!(rfc3339_for_test("2026-10-04T15:02:00-07:00"), Some(1_791_151_320_000));
    assert_eq!(rfc3339_for_test("2026-10-04T22:02:00.250+00:00"), Some(1_791_151_320_250));
    assert_eq!(rfc3339_for_test("2026-10-04T22:02:00.5Z"), Some(1_791_151_320_500));
    for bad in ["2026-02-29T00:00:00Z", "2026-13-01T00:00:00Z", "2026-10-04 15:02:00Z", "2026-10-04T15:02:00", "2026-10-04T24:00:00Z", "2026-10-04T15:02:00+0700", "2026-10-04T15:02:00.Z", "now", ""] {
        assert_eq!(rfc3339_for_test(bad), None, "{bad}");
    }
    let timeline = page("blocks");
    let Block::Timeline { entries, order, .. } = &timeline.blocks[6] else { panic!("the timeline") };
    assert_eq!(*order, Order::Given);
    assert_eq!(entries[0].at, 1_791_151_320_250);
    assert_eq!(entries[1].at, 1_791_150_000_000);
}

/// Text is NFC, so two spellings of one word are one word.
#[test]
fn text_is_normalized_to_nfc() {
    let json = r#"{"v":1,"title":"Café","blocks":[{"type":"heading","text":"x"}]}"#;
    assert_eq!(parse(json, &Caps::default()).unwrap().title, "Caf\u{e9}");
}

/// Every refusal fixture is refused at the path it names, with its words.
#[test]
fn each_refusal_fixture_is_refused_where_and_why_it_says() {
    let index: Vec<Value> = serde_json::from_str(&read("refusals.json")).unwrap();
    assert!(index.len() >= 40, "the index lost entries");
    for entry in &index {
        let file = entry["file"].as_str().unwrap();
        let e = refusal(&read(file));
        assert_eq!(e.path, entry["path"].as_str().unwrap(), "{file}: {e}");
        assert!(e.message.contains(entry["says"].as_str().unwrap()), "{file}: {e}");
    }
}

fn apply(doc: &mut Value, edit: &Value) {
    let pointer = edit["pointer"].as_str().unwrap();
    if edit.get("remove").is_some() {
        let (parent, key) = pointer.rsplit_once('/').unwrap();
        match doc.pointer_mut(parent).unwrap() {
            Value::Object(m) => assert!(m.remove(key).is_some(), "{pointer}"),
            Value::Array(a) => drop(a.remove(key.parse().unwrap())),
            other => panic!("{other}"),
        }
    } else if pointer.is_empty() {
        *doc = edit["value"].clone();
    } else {
        if let Some(slot) = doc.pointer_mut(pointer) {
            *slot = edit["value"].clone();
        } else {
            // A field the fixture lacks: put it in.
            let (parent, key) = pointer.rsplit_once('/').unwrap();
            let parent = if parent.is_empty() { &mut *doc } else { doc.pointer_mut(parent).unwrap() };
            parent.as_object_mut().unwrap().insert(key.to_string(), edit["value"].clone());
        }
    }
}

/// A refusal test that can't fail would pass for any reason, so each fixture
/// is shown to be refused for the reason it names: with its cap removed, or
/// with the one thing it gets wrong put right, it's accepted.
#[test]
fn each_refusal_fixture_is_accepted_with_its_cap_removed_or_its_fault_fixed() {
    let index: Vec<Value> = serde_json::from_str(&read("refusals.json")).unwrap();
    let mut capped = Vec::new();
    for entry in &index {
        let file = entry["file"].as_str().unwrap();
        if let Some(cap) = entry.get("cap").and_then(Value::as_str) {
            capped.push(cap.to_string());
            let doc: Value = serde_json::from_str(&read(file)).unwrap();
            assert!(check_value(&doc, &Caps::default().without(cap)).is_ok(), "{file} is refused for more than {cap}");
            // And only that cap: removing any other leaves it refused.
            for other in Caps::NAMES.iter().filter(|n| **n != cap) {
                if matches!(*other, "blocks" | "document_bytes") && cap == "refs" {
                    continue;
                }
                assert!(check_value(&doc, &Caps::default().without(other)).is_err(), "{file}: {other} isn't what refuses it");
            }
        } else {
            let mut doc: Value = serde_json::from_str(&read(file)).unwrap();
            for edit in entry["fix"].as_array().unwrap_or_else(|| panic!("{file} has neither a cap nor a fix")) {
                apply(&mut doc, edit);
            }
            if let Err(e) = check_value(&doc, &Caps::default()) {
                panic!("{file} is still refused once fixed: {e}");
            }
        }
    }
    for cap in Caps::NAMES {
        assert!(capped.iter().any(|c| c == cap), "no refusal fixture exceeds {cap}");
    }
}

/// A cap name a test misspells is a failure, not the full limits.
#[test]
#[should_panic(expected = "is not a page cap")]
fn removing_a_cap_that_does_not_exist_panics() {
    let _ = Caps::default().without("table_row");
}

/// The mockups' references, in reading order, with the paths the daemon
/// reports them at.
#[test]
fn the_train_page_names_its_references_by_path() {
    let train = page("train");
    let refs: Vec<(String, &'static str, String)> = train
        .references()
        .iter()
        .map(|r| (r.path.clone(), r.reference.target.kind(), r.reference.target.name().to_string()))
        .collect();
    assert_eq!(refs.len(), 17);
    assert_eq!(refs[0], ("blocks[4].rows[0][0]".to_string(), "lane", "ov-274-phones".to_string()));
    assert_eq!(refs[1], ("blocks[4].rows[0][1]".to_string(), "task", "ov-274".to_string()));
    assert_eq!(refs[2].0, "blocks[4].rows[0][3]");
    assert_eq!(refs[11], ("blocks[6].items[0]".to_string(), "ask", "ov-274".to_string()));
    assert_eq!(refs[13], ("blocks[7].entries[1]".to_string(), "task", "ov-274".to_string()));
    assert_eq!(refs[14].0, "blocks[9].items[0]");
}

/// The shape kept for discovery says what's on a page and holds none of it.
#[test]
fn the_shape_counts_blocks_and_reference_kinds_and_holds_no_content() {
    assert_eq!(
        page("train").shape(),
        "heading:2 text:1 stats:1 progress:1 table:1 list:1 timeline:1 steps:1 links:1 \
         ref-task:5 ref-ask:1 ref-lane:7 ref-theme:1 ref-page:1 ref-terminal:1 ref-url:1"
    );
    let shape = page("risks").shape();
    assert_eq!(shape, "heading:1 list:1 ref-task:1 ref-ask:1");
    assert!(!shape.contains("Sidebar"));
}

#[test]
fn a_link_shows_its_domain_and_only_when_it_is_plain() {
    assert_eq!(url_host("https://github.com/example/overnight/actions/runs/812"), Some("github.com"));
    assert_eq!(url_host("https://example.com:8443/x?q=1#top"), Some("example.com"));
    assert_eq!(url_host("https://a-b.example.com"), Some("a-b.example.com"));
    for bad in [
        "http://example.com",
        "https://",
        "https:///path",
        "https://github.com@evil.example/",
        "https://exa mple.com",
        "https://-bad.example.com",
        "https://example..com",
        "https://example.com:port/",
        "https://[::1]/",
        "https://exаmple.com/",
        "ftp://example.com",
    ] {
        assert_eq!(url_host(bad), None, "{bad}");
    }
}

#[test]
fn a_slot_is_lowercase_letters_digits_and_hyphens() {
    for ok in ["train", "spend", "risks-visual", "a", "0", &"a".repeat(40)] {
        assert!(valid_slot(ok), "{ok}");
    }
    for bad in ["", "-a", "A", "a b", "a_b", "é", &"a".repeat(41)] {
        assert!(!valid_slot(bad), "{bad}");
    }
}

/// The version is checked before anything else, so a newer document reads as a
/// version to update for, not as a list of misspellings.
#[test]
fn a_newer_version_says_so_before_its_new_fields() {
    let e = refusal(r#"{"v":2,"title":"x","layout":{},"blocks":[{"type":"gauge"}]}"#);
    assert_eq!(e.path, "v");
    assert_eq!(e.to_string(), "v: This runner draws pages up to version 1. Update the runner, or write version 1.");
}

#[test]
fn a_document_that_is_not_json_says_where() {
    let e = refusal("{\n  \"v\": 1,\n  oops\n}");
    assert_eq!(e.path, "");
    assert_eq!(e.to_string(), "That isn't valid JSON (line 3, column 3).");
}

#[test]
fn the_first_refusal_reads_as_the_design_words_it() {
    let e = refusal(&read("refused/table-rows.json"));
    assert_eq!(e.to_string(), "blocks[0].rows[50]: a table has at most 50 rows.");
    let e = refusal(&read("refused/unknown-block.json"));
    assert_eq!(
        e.to_string(),
        "blocks[0].type: there's no block called diagram. The blocks are heading, text, stats, progress, table, list, timeline, steps and links."
    );
}

// ---- page_schema ----

/// Every example the reference prints is a block the validator accepts, so the
/// reference can't teach a document that is refused.
#[test]
fn every_example_in_the_reference_is_accepted() {
    for (kind, example) in page_schema::EXAMPLES {
        let block: Value = serde_json::from_str(example).unwrap();
        assert_eq!(block["type"], kind);
        let doc = serde_json::json!({"v": 1, "title": "Example", "blocks": [block]});
        check_value(&doc, &Caps::default()).unwrap_or_else(|e| panic!("{kind}: {e}"));
    }
    let kinds: Vec<&str> = page_schema::EXAMPLES.iter().map(|(k, _)| *k).collect();
    assert_eq!(kinds, BLOCK_TYPES, "an example for each block, in the vocabulary's order");
}

#[test]
fn the_reference_names_every_block_and_every_target_inside_sixty_lines() {
    let text = page_schema::reference_text();
    for kind in BLOCK_TYPES {
        assert!(text.contains(&format!("  {kind}: ")), "{kind}");
    }
    for kind in Target::KINDS {
        assert!(text.contains(kind), "{kind}");
    }
    for word in State::WORDS {
        assert!(text.contains(word), "{word}");
    }
    assert!(text.lines().count() <= 60, "{} lines", text.lines().count());
}

#[test]
fn the_json_schema_carries_every_block_and_this_builds_limits() {
    let schema = page_schema::json_schema();
    let blocks = schema["properties"]["blocks"]["items"]["oneOf"].as_array().unwrap();
    let kinds: Vec<&str> = blocks.iter().map(|b| b["properties"]["type"]["const"].as_str().unwrap()).collect();
    assert_eq!(kinds, BLOCK_TYPES);
    let caps = Caps::default();
    assert_eq!(schema["properties"]["blocks"]["maxItems"], caps.blocks);
    assert_eq!(schema["properties"]["title"]["maxLength"], caps.title_chars);
    let table = &blocks[4]["properties"];
    assert_eq!(table["rows"]["maxItems"], caps.table_rows);
    assert_eq!(table["columns"]["maxItems"], caps.table_columns);
    let text = serde_json::to_string(&schema).unwrap();
    for kind in Target::KINDS {
        assert!(text.contains(&format!("\"{kind}\"")), "{kind}");
    }
    assert_eq!(schema["properties"]["v"]["const"], VERSION);
}

// ---- links in text ----

/// Each way Markdown writes a link is read, and only a plain `https` link that
/// says where it goes is drawn.
#[test]
fn a_link_in_text_is_https_and_says_where_it_goes() {
    let ok = |md: &str| {
        let doc = serde_json::json!({"v": 1, "title": "T", "blocks": [{"type": "text", "md": md}]});
        check_value(&doc, &Caps::default()).map(|_| ()).map_err(|e| e.message)
    };
    for good in [
        "no links at all, and a (parenthesis) and a [bracket]",
        "[words](https://github.com/x)",
        "[github.com](https://github.com/x) and [www.github.com](https://github.com/y)",
        "[GitHub.com](https://github.com/x), [see github.com/x](https://github.com/x)",
        "[x](https://example.com/a_(b) \"a title\")",
        "<https://example.com/a>",
        "[ref][1]\n\n[1]: https://example.com/",
        "a bare http://example.com is plain text, and so is mailto:a@b.example",
    ] {
        ok(good).unwrap_or_else(|e| panic!("{good}: {e}"));
    }
    for (bad, says) in [
        ("[x](javascript:alert(1))", "javascript"),
        ("[x](JavaScript:alert(1))", "JavaScript"),
        ("[x](  http://a.example/)", "http"),
        ("[x](data:text/html,hi)", "data"),
        ("[x](//evil.example/)", "has to start with https://"),
        ("[x](<javascript:alert(1)>)", "javascript"),
        ("![i](https://a.example/p.png)[x](ftp://a.example/)", "ftp"),
        ("<mailto:a@b.example>", "mailto"),
        ("[a.example and github.com](https://github.com/x)", "a.example"),
        ("[github.com.evil.example](https://github.com/x)", "github.com.evil.example"),
        ("[x](https://a.example@evil.example/)", "domain a page can't show"),
        // A file name reads as a domain, so a label that is one is refused: the
        // price of a rule that can't be talked round.
        ("[Cargo.toml notes](https://example.com/n)", "Cargo.toml"),
    ] {
        let said = ok(bad).expect_err(bad);
        assert!(said.contains(says), "{bad}: {said}");
        assert!(!said.contains('\u{1b}'), "{bad}: a control character reached the sentence");
    }
}

// ---- live data (ov-306) ----

/// The live page's references: CI by main, SHA (lowercased) and run, card
/// counts, and spend, a figure's among them, each under the subject the
/// runner reads it by.
#[test]
fn live_references_name_what_the_runner_reads() {
    let live = page("live");
    let refs = live.references();
    let at = |path: &str| refs.iter().find(|r| r.path == path).unwrap_or_else(|| panic!("no ref at {path}")).reference;
    assert_eq!(at("blocks[1].items[0]").target, Target::Ci("main".into()));
    assert_eq!(at("blocks[1].items[1]").target, Target::Ci("c85bf83d".into()), "a SHA is kept lowercase");
    assert_eq!(at("blocks[1].items[2]").target, Target::Cards("in_review".into()));
    let subjects: Vec<String> = refs.iter().filter_map(|r| r.reference.target.ci_subject()).collect();
    assert_eq!(subjects, ["main", "sha:c85bf83d", "sha:c85bf83d", "run:37275435256", "main", "main", "sha:c85bf83d"]);
    let Block::Stats { items } = &live.blocks[1] else { panic!("the stats") };
    assert_eq!((items[4].show, items[4].value.as_str()), (Some(Show::Spend), "Visual language"), "the fallback older apps draw");
    assert!(live.shape().contains("ref-ci:7") && live.shape().contains("ref-cards:3"), "{}", live.shape());
}

/// An app from before live data reads a live page with something in every
/// place (review train-1005c M4): a figure keeps a value (it drops a figure
/// without one) and a CI or card-count reference keeps a label (it draws an
/// unknown reference's label), each the reference's own name, which a newer
/// app replaces with the live value.
#[test]
fn an_older_app_reads_live_data_as_its_names() {
    let live = page("live");
    let Block::Stats { items } = &live.blocks[1] else { panic!("the stats") };
    let values: Vec<&str> = items.iter().map(|s| s.value.as_str()).collect();
    assert_eq!(values, ["Main", "c85bf83d", "In review", "Open", "Visual language", "mac-ux"]);
    for at in live.references() {
        if matches!(at.reference.target, Target::Ci(_) | Target::Cards(_)) {
            assert!(at.reference.label.as_deref().is_some_and(|l| !l.is_empty()), "{} has no label", at.path);
        }
    }
    // A value written beside a reference is the fallback, kept as written.
    let json = r#"{"v":1,"title":"T","blocks":[{"type":"stats","items":[{"label":"Main","value":"Green","ref":{"ci":"main"}}]}]}"#;
    let Block::Stats { items } = &parse(json, &Caps::default()).unwrap().blocks[0] else { panic!() };
    assert_eq!((items[0].value.as_str(), items[0].reference.is_some()), ("Green", true));
}
