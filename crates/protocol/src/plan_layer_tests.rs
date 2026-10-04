//! The wire for the plan layer (ov-268): experimental, additive, removable.

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

const LAYER_METHODS: [&str; 10] = [
    "plan.get",
    "plan.set",
    "plan.events",
    "board_theme.create",
    "board_theme.update",
    "board_theme.cards",
    "lane.create",
    "lane.update",
    "lane.cards",
    "lane.agent",
];

/// The tags the layer takes, pinned. Request 160-169 and result 160-163 are
/// the layer's alone, and the arms before them are other lanes': a tag here
/// that drifted would decode as another lane's method.
#[test]
fn the_layer_holds_its_tags() {
    assert_eq!(number("Request", "terminal_rename"), 140, "the last request tag before it");
    assert_eq!(number("Request", "worktree_file"), 151, "the highest request tag before it");
    let requests = [
        "plan_get",
        "board_theme_create",
        "board_theme_update",
        "board_theme_cards",
        "lane_create",
        "lane_update",
        "lane_cards",
        "lane_agent_set",
        "plan_set",
        "plan_events",
    ];
    for (i, name) in requests.iter().enumerate() {
        assert_eq!(number("Request", name), 160 + i as i32, "{name}");
    }
    assert_eq!(number("Result", "worktree_file"), 151, "the highest result tag before it");
    for (i, name) in ["board_theme_view", "lane", "plan", "plan_event_list"].iter().enumerate() {
        assert_eq!(number("Result", name), 160 + i as i32, "{name}");
    }
    assert_eq!(number("Event", "board_reads_changed"), 26, "the last event tag before it");
    assert_eq!(number("Event", "plan_changed"), 27);
}

/// The layer is one capability, advertised, and owns every one of its methods.
#[test]
fn the_layer_is_one_advertised_capability_owning_every_method() {
    assert_eq!(capability::BOARD_PLAN, "board_plan");
    assert!(capability::ALL.contains(&capability::BOARD_PLAN), "the daemon would not advertise it");
    for method in LAYER_METHODS {
        assert_eq!(capability::for_method(method), Some(capability::BOARD_PLAN), "{method}");
    }
    let owned = Method::ALL.iter().filter(|m| m.capability() == capability::BOARD_PLAN).count();
    assert_eq!(owned, LAYER_METHODS.len(), "a method the layer owns that this test does not name");
}

/// Nothing in the layer is a field of anything that exists: no message a task
/// travels in has a field naming it, which is what lets it be removed.
#[test]
fn no_task_message_carries_the_layer() {
    let file = file();
    for name in ["Task", "TaskDetail", "TaskList", "TaskNote", "TaskWorker", "TaskBlock", "Workspace"] {
        let message = file.message_type.iter().find(|m| m.name() == name).unwrap_or_else(|| panic!("no {name}"));
        for field in &message.field {
            let field_type = field.type_name();
            assert!(
                !(field.name().contains("lane") || field.name().contains("theme") || field.name().contains("plan"))
                    && !(field_type.ends_with(".Lane") || field_type.ends_with(".BoardTheme") || field_type.ends_with(".Plan")),
                "{name}.{} names the plan layer",
                field.name()
            );
        }
    }
}

/// A runner built before the layer receives a layer request and an app built
/// before it receives a layer event, and each sees what it always saw.
///
/// The runner decodes `Request.method` and finds no such method, so it
/// refuses with `CAPABILITY_UNSUPPORTED` before looking at the payload it
/// cannot read (`Rpc::handle`); the app decodes `Event` and sees no payload.
#[test]
fn builds_before_the_layer_ignore_it() {
    /// `Request` as it was at `terminal_rename = 140` and `worktree_file = 151`:
    /// the envelope, and no arm in 160-169.
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
    /// `Event` as it was at `board_reads_changed = 26`.
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
        method: "plan.get".into(),
        required_capabilities: vec![capability::BOARD_PLAN.into()],
        payload: Some(v1::request::Payload::PlanGet(v1::PlanGetRequest {
            workspace_id: bytes::Bytes::from_static(&[2; 16]),
            include_closed: false,
        })),
        ..Default::default()
    };
    let old = OldRequest::decode(request.encode_to_vec().as_slice()).expect("an old runner decodes it");
    assert_eq!(old.method, "plan.get", "it still reads the method it will refuse by name");
    assert_eq!(old.required_capabilities, [capability::BOARD_PLAN], "and the capability it does not have");
    assert!(old.payload.is_none(), "a payload it has never heard of is no payload at all");

    let event = v1::Event {
        event_id: bytes::Bytes::from_static(&[3; 16]),
        sequence: 5,
        payload: Some(v1::event::Payload::PlanChanged(v1::PlanChanged {
            workspace_id: bytes::Bytes::from_static(&[2; 16]),
            actor: "manager".into(),
        })),
    };
    let old = OldEvent::decode(event.encode_to_vec().as_slice()).expect("an old app decodes it");
    assert_eq!(old.sequence, 5);
    assert!(old.payload.is_none(), "an arm it has never heard of is no event at all");
}
