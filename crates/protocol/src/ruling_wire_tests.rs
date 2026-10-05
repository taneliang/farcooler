//! The wire for rulings (ov-304), the plan layer's decided-for-you calls:
//! experimental, additive, removable with the layer.

use prost::Message;
use prost_types::FileDescriptorSet;

use crate::capability;
use crate::method::Method;
use crate::v1;

fn number(m: &str, f: &str) -> i32 {
    let set = FileDescriptorSet::decode(&include_bytes!(concat!(env!("OUT_DIR"), "/farcooler_descriptor.bin"))[..])
        .expect("the build writes a descriptor");
    let file = set.file.into_iter().find(|f| f.package() == "farcooler.v1").expect("farcooler.v1");
    file.message_type
        .iter()
        .find(|x| x.name() == m)
        .and_then(|x| x.field.iter().find(|x| x.name() == f))
        .unwrap_or_else(|| panic!("{m} has no field {f}"))
        .number()
}

/// Request and result tags from 180, after the pages' 170s, and the plan's
/// next field: pinned, so a drift can't decode as another lane's method.
#[test]
fn rulings_hold_their_tags() {
    assert_eq!(number("Request", "page_stats"), 174, "the highest request tag before them");
    assert_eq!(number("Request", "ruling_add"), 180);
    assert_eq!(number("Request", "ruling_set"), 181);
    assert_eq!(number("Result", "page_stats_list"), 173, "the highest result tag before them");
    assert_eq!(number("Result", "board_ruling"), 180);
    assert_eq!(number("Plan", "coverage"), 6, "the plan's last field before them");
    assert_eq!(number("Plan", "rulings"), 7);
}

/// One advertised capability owns both writes.
#[test]
fn rulings_are_one_advertised_capability() {
    assert_eq!(capability::BOARD_RULINGS, "board_rulings");
    assert!(capability::ALL.contains(&capability::BOARD_RULINGS), "the daemon would not advertise it");
    for method in ["ruling.add", "ruling.set"] {
        assert_eq!(capability::for_method(method), Some(capability::BOARD_RULINGS), "{method}");
    }
    let owned = Method::ALL.iter().filter(|m| m.capability() == capability::BOARD_RULINGS).count();
    assert_eq!(owned, 2, "a method rulings own that this test does not name");
}

/// An app built before rulings reads a plan that carries them as the plan it
/// always read: the new field is skipped, and nothing else moves.
#[test]
fn a_plan_with_rulings_reads_the_same_to_an_older_app() {
    /// `Plan` as it was at `coverage = 6`, the fields an older app reads.
    #[derive(Clone, PartialEq, prost::Message)]
    struct OldPlan {
        #[prost(int64, tag = "1")]
        now_ms: i64,
        #[prost(bytes = "bytes", repeated, tag = "4")]
        order: Vec<bytes::Bytes>,
    }
    let plan = v1::Plan {
        now_ms: 42,
        order: vec![bytes::Bytes::from_static(&[7; 16])],
        rulings: vec![v1::BoardRuling {
            number: 12,
            decision: "The inbox is amber.".into(),
            state: v1::BoardRulingState::Standing as i32,
            ..Default::default()
        }],
        ..Default::default()
    };
    let old = OldPlan::decode(plan.encode_to_vec().as_slice()).expect("an older app decodes it");
    assert_eq!(old, OldPlan { now_ms: 42, order: vec![bytes::Bytes::from_static(&[7; 16])] });
    let back = v1::Plan::decode(plan.encode_to_vec().as_slice()).unwrap();
    assert_eq!(back.rulings[0].number, 12, "and this build reads the ruling");
}

/// No message a task travels in has a field for a ruling.
#[test]
fn no_task_message_carries_a_ruling() {
    let set = FileDescriptorSet::decode(&include_bytes!(concat!(env!("OUT_DIR"), "/farcooler_descriptor.bin"))[..]).unwrap();
    let file = set.file.into_iter().find(|f| f.package() == "farcooler.v1").unwrap();
    for name in ["Task", "TaskDetail", "TaskList", "TaskNote", "TaskWorker", "TaskBlock", "Workspace"] {
        let message = file.message_type.iter().find(|m| m.name() == name).unwrap_or_else(|| panic!("no {name}"));
        for field in &message.field {
            assert!(
                !field.name().contains("ruling") && !field.type_name().ends_with(".BoardRuling"),
                "{name}.{} names a ruling",
                field.name()
            );
        }
    }
}
