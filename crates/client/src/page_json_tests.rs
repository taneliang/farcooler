use super::*;
use uuid::Uuid;

fn fixtures() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures")
}

/// The train page, as the runner stores it: the normalized document the core
/// crate's own test holds to `pages/normalized/train.json`.
pub(crate) fn the_train_page() -> pb::BoardPage {
    let doc = std::fs::read_to_string(fixtures().join("pages/normalized/train.json")).unwrap();
    let doc_json = serde_json::to_string(&serde_json::from_str::<Value>(&doc).unwrap()).unwrap();
    pb::BoardPage {
        id: bytes::Bytes::copy_from_slice(Uuid::from_u128(0x3001).as_bytes()),
        slot: "train".into(),
        title: "Train integ-10".into(),
        summary: "In review · 3 of 4 lanes green".into(),
        anchor_kind: "theme".into(),
        anchor: Uuid::from_u128(0x1001).to_string(),
        doc_json,
        revision: 3,
        ordinal: 0,
        actor: "manager".into(),
        updated_at_ms: 1_791_151_320_000,
    }
}

/// The page the CLI's `page show --json` prints and the phones decode is one
/// file, written here and read by the CLI's test and by the apps'. Run with
/// `FARCOOLER_WRITE_FIXTURES=1` to rewrite it after a deliberate change.
#[test]
fn a_page_is_the_shape_the_apps_read() {
    let shown = page_json(&the_train_page());
    let path = fixtures().join("page.json");
    if std::env::var_os("FARCOOLER_WRITE_FIXTURES").is_some() {
        std::fs::write(&path, serde_json::to_string_pretty(&shown).unwrap() + "\n").unwrap();
    }
    let committed: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(shown, committed);
}

/// The document passes through unchanged: the apps read blocks, not a string.
#[test]
fn the_document_passes_through_as_json_not_as_text() {
    let shown = page_json(&the_train_page());
    assert_eq!(shown["doc"]["v"], 1);
    assert_eq!(shown["doc"]["blocks"][0]["type"], "text");
    assert_eq!(shown["doc"]["blocks"][4]["rows"][0][0]["ref"]["lane"], "ov-274-phones");
    assert_eq!((shown["slot"].as_str(), shown["revision"].as_u64(), shown["anchor_kind"].as_str()), (Some("train"), Some(3), Some("theme")));
}

/// A page listed without its document says so, instead of an empty one.
#[test]
fn a_page_read_without_its_document_has_a_null_one() {
    let mut page = the_train_page();
    page.doc_json.clear();
    assert!(page_json(&page)["doc"].is_null());
    page.doc_json = "not json".into();
    assert!(page_json(&page)["doc"].is_null(), "a document the runner wrote is JSON; one that isn't draws nothing");
    let list = pages_json(&pb::BoardPageList { pages: vec![the_train_page(), page] });
    assert_eq!(list["pages"].as_array().unwrap().len(), 2);
}
