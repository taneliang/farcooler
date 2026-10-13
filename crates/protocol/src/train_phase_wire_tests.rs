//! The wire for a train's phase (ov-503): three additive fields on
//! `BoardTrain`, behind `board_train_phase`, which owns no method.

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

/// The fields take the next free tags on `BoardTrain` and never move.
#[test]
fn the_phase_fields_hold_their_tags() {
    assert_eq!(number("BoardTrain", "card_id"), 17, "the field before them");
    assert_eq!(number("BoardTrain", "phase"), 18);
    assert_eq!(number("BoardTrain", "held_by"), 19);
    assert_eq!(number("BoardTrain", "phase_since"), 20);
}

/// Advertised, and owning no method: only the plan read and the train writes carry it.
#[test]
fn the_phase_is_an_advertised_capability_with_no_method_of_its_own() {
    assert_eq!(capability::BOARD_TRAIN_PHASE, "board_train_phase");
    assert!(capability::ALL.contains(&capability::BOARD_TRAIN_PHASE), "the daemon would not advertise it");
    assert_eq!(Method::ALL.iter().filter(|m| m.capability() == capability::BOARD_TRAIN_PHASE).count(), 0);
}

/// A client built before the fields decodes a train that carries them and
/// reads what it always read; a newer one reads every field back.
#[test]
fn a_train_with_a_phase_reads_the_same_to_an_older_client() {
    /// `BoardTrain` as it was at `card_id = 17`, trimmed to what the test reads.
    #[derive(Clone, PartialEq, prost::Message)]
    struct OldTrain {
        #[prost(string, tag = "3")]
        name: String,
        #[prost(enumeration = "v1::BoardTrainState", tag = "6")]
        state: i32,
        #[prost(bytes = "bytes", repeated, tag = "11")]
        lane_ids: Vec<bytes::Bytes>,
        #[prost(string, tag = "14")]
        title: String,
    }
    let slow = bytes::Bytes::from_static(&[9; 16]);
    let train = v1::BoardTrain {
        name: "train-86".into(),
        state: v1::BoardTrainState::Integrating as i32,
        lane_ids: vec![bytes::Bytes::from_static(&[1; 16]), slow.clone()],
        title: "Train 86".into(),
        phase: v1::BoardTrainPhase::WaitingForLanes as i32,
        held_by: vec![slow.clone()],
        phase_since: 1_234,
        ..Default::default()
    };
    let wire = train.encode_to_vec();
    let old = OldTrain::decode(wire.as_slice()).expect("an older client decodes it");
    assert_eq!(old.name, "train-86");
    assert_eq!(old.state, v1::BoardTrainState::Integrating as i32, "an old client keeps drawing state");
    assert_eq!(old.lane_ids.len(), 2);
    assert_eq!(old.title, "Train 86");

    let back = v1::BoardTrain::decode(wire.as_slice()).unwrap();
    assert_eq!(back, train);
    // And an older runner's train reads as no phase at all.
    let bare = v1::BoardTrain::decode(OldTrain { name: "x".into(), ..Default::default() }.encode_to_vec().as_slice()).unwrap();
    assert_eq!((bare.phase, bare.held_by.len(), bare.phase_since), (v1::BoardTrainPhase::Unspecified as i32, 0, 0));
}
