//! The wire for trains (ov-309), the plan layer's batches of lanes landing
//! together, and the CI reads the runner keeps for them and for pages:
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

/// Request and result tags from 190, after rulings' 180s, and the plan's next
/// two fields: pinned, so a drift can't decode as another method.
#[test]
fn trains_hold_their_tags() {
    assert_eq!(number("Request", "ruling_set"), 181, "the highest request tag before them");
    assert_eq!(number("Request", "train_start"), 190);
    assert_eq!(number("Request", "train_set"), 191);
    assert_eq!(number("Result", "board_ruling"), 180, "the highest result tag before them");
    assert_eq!(number("Result", "board_train"), 190);
    assert_eq!(number("Plan", "rulings"), 7, "the plan's last field before them");
    assert_eq!(number("Plan", "trains"), 8);
    assert_eq!(number("Plan", "ci"), 9);
}

/// One advertised capability owns both writes.
#[test]
fn trains_are_one_advertised_capability() {
    assert_eq!(capability::BOARD_TRAINS, "board_trains");
    assert!(capability::ALL.contains(&capability::BOARD_TRAINS), "the daemon would not advertise it");
    for method in ["train.start", "train.set"] {
        assert_eq!(capability::for_method(method), Some(capability::BOARD_TRAINS), "{method}");
    }
    let owned = Method::ALL.iter().filter(|m| m.capability() == capability::BOARD_TRAINS).count();
    assert_eq!(owned, 2, "a method trains own that this test does not name");
}

/// An app built before trains reads a plan that carries them, and CI reads,
/// as the plan it always read: the new fields are skipped, nothing else moves.
#[test]
fn a_plan_with_trains_reads_the_same_to_an_older_app() {
    /// `Plan` as it was at `rulings = 7`, the fields an older app reads.
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
        trains: vec![v1::BoardTrain {
            name: "integ-14".into(),
            state: v1::BoardTrainState::Pushed as i32,
            pushed_sha: Some("1a1b3275".into()),
            ..Default::default()
        }],
        ci: vec![v1::BoardCiRead {
            subject: "sha:1a1b3275".into(),
            status: v1::BoardCiStatus::Running as i32,
            jobs: vec![v1::BoardCiJob { name: "CI / rust".into(), state: "running".into(), url: String::new() }],
            ..Default::default()
        }],
        ..Default::default()
    };
    let old = OldPlan::decode(plan.encode_to_vec().as_slice()).expect("an older app decodes it");
    assert_eq!(old, OldPlan { now_ms: 42, order: vec![bytes::Bytes::from_static(&[7; 16])] });
    let back = v1::Plan::decode(plan.encode_to_vec().as_slice()).unwrap();
    assert_eq!((back.trains[0].name.as_str(), back.ci[0].jobs[0].name.as_str()), ("integ-14", "CI / rust"));
}

/// No message a task travels in has a field for a train or a CI read.
#[test]
fn no_task_message_carries_a_train() {
    let set = FileDescriptorSet::decode(&include_bytes!(concat!(env!("OUT_DIR"), "/farcooler_descriptor.bin"))[..]).unwrap();
    let file = set.file.into_iter().find(|f| f.package() == "farcooler.v1").unwrap();
    for name in ["Task", "TaskDetail", "TaskList", "TaskNote", "TaskWorker", "TaskBlock", "Workspace"] {
        let message = file.message_type.iter().find(|m| m.name() == name).unwrap_or_else(|| panic!("no {name}"));
        for field in &message.field {
            assert!(
                // By whole word: `constraints` holds the letters.
                !field.name().split('_').any(|w| w.starts_with("train") || w == "ci")
                    && !field.type_name().contains(".BoardCi")
                    && !field.type_name().ends_with(".BoardTrain"),
                "{name}.{} names a train",
                field.name()
            );
        }
    }
}
