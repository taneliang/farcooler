//! The wire for read state on the runner (ov-113).

use prost::Message;
use prost_types::FileDescriptorSet;

use crate::capability;
use crate::v1;

fn file() -> prost_types::FileDescriptorProto {
    let set = FileDescriptorSet::decode(&include_bytes!(concat!(env!("OUT_DIR"), "/farcooler_descriptor.bin"))[..])
        .expect("the build writes a descriptor");
    set.file.into_iter().find(|f| f.package() == "farcooler.v1").expect("farcooler.v1")
}

/// The tags these take, pinned, for the reason the waits' are.
#[test]
fn read_state_holds_its_tags() {
    let file = file();
    let number = |m: &str, f: &str| {
        file.message_type
            .iter()
            .find(|x| x.name() == m)
            .and_then(|x| x.field.iter().find(|x| x.name() == f))
            .unwrap_or_else(|| panic!("{m} has no field {f}"))
            .number()
    };
    assert_eq!(number("Request", "task_worker_set"), 132, "the last request tag before it");
    assert_eq!(number("Request", "workspace_mark_read"), 133);
    assert_eq!(number("Result", "task_usage"), 121, "the last result tag before it");
    assert_eq!(number("Result", "board_reads"), 122);
    assert_eq!(number("Event", "notice"), 25, "the last event tag before it");
    assert_eq!(number("Event", "board_reads_changed"), 26);
    assert_eq!(number("TaskList", "items"), 1);
    assert_eq!(number("TaskList", "reads"), 2);
    assert_eq!(number("WorkspaceMarkRead", "floor_ms"), 2);
    assert_eq!(number("WorkspaceMarkRead", "seeds_floor"), 4);
}

#[test]
fn read_state_is_one_advertised_capability_owning_its_write() {
    assert_eq!(capability::BOARD_READS, "board_reads");
    assert!(capability::ALL.contains(&capability::BOARD_READS), "the daemon would not advertise it");
    assert_eq!(capability::for_method("workspace.mark_read"), Some(capability::BOARD_READS));
}

/// A client built before this decodes a `TaskList` and an `Event` the new
/// runner sends, and sees what it always saw: the tasks, and no event.
#[test]
fn an_older_client_ignores_the_new_fields() {
    /// `TaskList` as it was: `items` and nothing else.
    #[derive(Clone, PartialEq, prost::Message)]
    struct OldTaskList {
        #[prost(message, repeated, tag = "1")]
        items: Vec<v1::Task>,
    }
    /// `Event` as it was at `notice = 25`: no arm 26.
    #[derive(Clone, PartialEq, prost::Message)]
    struct OldEvent {
        #[prost(bytes = "bytes", tag = "1")]
        event_id: bytes::Bytes,
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

    let reads = v1::BoardReads {
        workspace_id: bytes::Bytes::from_static(&[1; 16]),
        floor_ms: 7,
        opened: vec![v1::TaskRead { task_id: bytes::Bytes::from_static(&[2; 16]), opened_ms: 9 }],
    };
    let list = v1::TaskList { items: vec![v1::Task::default()], reads: Some(reads.clone()) };
    let old = OldTaskList::decode(list.encode_to_vec().as_slice()).expect("an old client decodes it");
    assert_eq!(old.items.len(), 1);

    let event = v1::Event {
        event_id: bytes::Bytes::from_static(&[3; 16]),
        sequence: 4,
        payload: Some(v1::event::Payload::BoardReadsChanged(reads)),
    };
    let old = OldEvent::decode(event.encode_to_vec().as_slice()).expect("an old client decodes it");
    assert_eq!(old.sequence, 4);
    assert!(old.payload.is_none(), "an arm it has never heard of is no event at all");
}
