//! The wire for orchestrator pages (ov-269): experimental, additive, removable.

use prost::Message;
use prost_types::FileDescriptorSet;

use crate::capability;
use crate::method::Method;
use crate::v1;

fn file() -> prost_types::FileDescriptorProto {
    let set = FileDescriptorSet::decode(&include_bytes!(concat!(env!("OUT_DIR"), "/farcooler_descriptor.bin"))[..])
        .expect("the build writes a descriptor");
    set.file.into_iter().find(|f| f.package() == "farcooler.v1").expect("farcooler.v1")
}

fn number(m: &str, f: &str) -> i32 {
    file()
        .message_type
        .iter()
        .find(|x| x.name() == m)
        .and_then(|x| x.field.iter().find(|x| x.name() == f))
        .unwrap_or_else(|| panic!("{m} has no field {f}"))
        .number()
}

const PAGE_METHODS: [&str; 5] = ["page.list", "page.get", "page.set", "page.remove", "page.stats"];

/// The tags pages take, pinned. Requests 170-179 and results 170-179 are
/// pages' alone, and the arms before them are other lanes' and the plan
/// layer's: a tag here that drifted would decode as another method.
#[test]
fn pages_hold_their_tags() {
    assert_eq!(number("Request", "plan_events"), 169, "the last request tag before them");
    for (i, name) in ["page_list", "page_get", "page_set", "page_remove", "page_stats"].iter().enumerate() {
        assert_eq!(number("Request", name), 170 + i as i32, "{name}");
    }
    assert_eq!(number("Result", "plan_event_list"), 163, "the last result tag before them");
    for (i, name) in ["board_page_list", "board_page", "page_set_result", "page_stats_list"].iter().enumerate() {
        assert_eq!(number("Result", name), 170 + i as i32, "{name}");
    }
    assert_eq!(number("Event", "plan_changed"), 27, "the last event tag before it");
    assert_eq!(number("Event", "pages_changed"), 28);
}

/// Pages are one capability, advertised, and own every one of their methods,
/// and none of them needs the plan layer.
#[test]
fn pages_are_one_advertised_capability_owning_every_method() {
    assert_eq!(capability::BOARD_PAGES, "board_pages");
    assert!(capability::ALL.contains(&capability::BOARD_PAGES), "the daemon would not advertise it");
    for method in PAGE_METHODS {
        assert_eq!(capability::for_method(method), Some(capability::BOARD_PAGES), "{method}");
    }
    let owned = Method::ALL.iter().filter(|m| m.capability() == capability::BOARD_PAGES).count();
    assert_eq!(owned, PAGE_METHODS.len(), "a method pages own that this test does not name");
    assert_ne!(capability::BOARD_PAGES, capability::BOARD_PLAN);
}

/// Nothing pages carry is a field of anything that exists: no task or plan
/// message has a field naming them, which is what lets them be removed.
#[test]
fn no_task_or_plan_message_carries_a_page() {
    let file = file();
    for name in [
        "Task", "TaskDetail", "TaskList", "TaskNote", "TaskWorker", "TaskBlock", "Workspace", "Plan", "PlanCard",
        "PlanCoverage", "Lane", "LaneAgent", "BoardTheme", "BoardThemeView", "PlanEvent",
    ] {
        let message = file.message_type.iter().find(|m| m.name() == name).unwrap_or_else(|| panic!("no {name}"));
        for field in &message.field {
            assert!(
                !field.name().contains("page") && !field.type_name().contains("Page"),
                "{name}.{} names pages",
                field.name()
            );
        }
    }
}

/// Twelve pages of 32 KiB, with room for their rows, fit one control envelope:
/// what lets `page.list` carry every document at once.
#[test]
fn a_whole_list_with_documents_fits_one_envelope() {
    let page = v1::BoardPage {
        id: bytes::Bytes::from_static(&[1; 16]),
        slot: "a".repeat(40),
        title: "t".repeat(60 * 4),
        summary: "s".repeat(120 * 4),
        anchor_kind: "theme".into(),
        anchor: "0".repeat(36),
        doc_json: "x".repeat(farcooler_core_doc_bytes()),
        revision: u64::MAX,
        ordinal: u32::MAX,
        actor: "agent:00000000-0000-0000-0000-000000000000".into(),
        updated_at_ms: i64::MAX,
    };
    let list = v1::BoardPageList { pages: vec![page; 12] };
    let envelope = v1::Response {
        request_id: bytes::Bytes::from_static(&[9; 16]),
        outcome: Some(v1::response::Outcome::Result(v1::Result { value: Some(v1::result::Value::BoardPageList(list)) })),
    };
    assert!(envelope.encoded_len() < crate::MAX_CONTROL_ENVELOPE_BYTES, "{} bytes", envelope.encoded_len());
}

/// The document cap, 32 KiB, spelled once in `farcooler_core::page_doc`; this
/// crate doesn't depend on it, so the envelope test holds the number here and
/// the daemon's test holds the two together.
fn farcooler_core_doc_bytes() -> usize {
    32 * 1024
}

/// A runner built before pages receives a page request and an app built
/// before them receives a page event, and each sees what it always saw.
///
/// The runner decodes `Request.method` and finds no such method, so it
/// refuses with `CAPABILITY_UNSUPPORTED` before looking at the payload it
/// cannot read; the app decodes `Event` and sees no payload.
#[test]
fn builds_before_pages_ignore_them() {
    #[derive(Clone, PartialEq, prost::Message)]
    struct OldRequest {
        #[prost(bytes = "bytes", tag = "1")]
        request_id: bytes::Bytes,
        #[prost(string, tag = "2")]
        method: String,
        #[prost(string, repeated, tag = "7")]
        required_capabilities: Vec<String>,
        #[prost(oneof = "old_request::Payload", tags = "20")]
        payload: Option<old_request::Payload>,
    }
    mod old_request {
        #[derive(Clone, PartialEq, prost::Oneof)]
        pub enum Payload {
            #[prost(message, tag = "20")]
            Empty(super::v1::Empty),
        }
    }
    /// `Event` as it was at `plan_changed = 27`.
    #[derive(Clone, PartialEq, prost::Message)]
    struct OldEvent {
        #[prost(uint64, tag = "2")]
        sequence: u64,
        #[prost(oneof = "old_event::Payload", tags = "23")]
        payload: Option<old_event::Payload>,
    }
    mod old_event {
        #[derive(Clone, PartialEq, prost::Oneof)]
        pub enum Payload {
            #[prost(message, tag = "23")]
            EventsMissed(super::v1::Empty),
        }
    }

    let request = v1::Request {
        request_id: bytes::Bytes::from_static(&[1; 16]),
        method: "page.set".into(),
        required_capabilities: vec![capability::BOARD_PAGES.into()],
        payload: Some(v1::request::Payload::PageSet(v1::PageSet {
            workspace_id: bytes::Bytes::from_static(&[2; 16]),
            slot: "train".into(),
            doc_json: "{}".into(),
            ..Default::default()
        })),
        ..Default::default()
    };
    let old = OldRequest::decode(request.encode_to_vec().as_slice()).expect("an old runner decodes it");
    assert_eq!(old.method, "page.set", "it still reads the method it will refuse by name");
    assert_eq!(old.required_capabilities, [capability::BOARD_PAGES], "and the capability it does not have");
    assert!(old.payload.is_none(), "a payload it has never heard of is no payload at all");

    let event = v1::Event {
        event_id: bytes::Bytes::from_static(&[3; 16]),
        sequence: 5,
        payload: Some(v1::event::Payload::PagesChanged(v1::PagesChanged {
            workspace_id: bytes::Bytes::from_static(&[2; 16]),
            slot: "train".into(),
            revision: 3,
            actor: "manager".into(),
            removed: false,
        })),
    };
    let old = OldEvent::decode(event.encode_to_vec().as_slice()).expect("an old app decodes it");
    assert_eq!(old.sequence, 5);
    assert!(old.payload.is_none(), "an arm it has never heard of is no event at all");
}
