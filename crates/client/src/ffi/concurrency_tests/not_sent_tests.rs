//! Which failed input the phones may keep for Try Again (ov-250).
//!
//! A write the transport never began fails as `not_sent`; one it began, or
//! one that failed after the runner may have answered, does not.
//!
//! Two cases that look odd and are safe. `terminal.resize` is urgent too, so
//! it carries the flag as well; the phones read it only for typed input. And a
//! framing error on the link reaches the answer as `NotSent` with
//! `disconnected` false: the input still never reached the wire, so holding it
//! for Try Again is right, and the slot stays filled.

use std::time::{Duration, Instant};

use farcooler_transport::ClientError;

use super::{answer_for, call, rig};
use crate::ffi::*;

fn lines_for(results: Vec<Result<(), Lost>>) -> Vec<Value> {
    let queue = Arc::new(std::sync::Mutex::new(VecDeque::new()));
    for (i, outcome) in results.into_iter().enumerate() {
        push_call(&queue, i as u64, outcome.map(|()| json!({})), true);
    }
    locked(&queue).iter().map(|l| serde_json::from_str(l).unwrap()).collect()
}

#[test]
fn input_the_transport_never_wrote_says_not_sent() {
    let unwritten: SessionError = ClientError::NotWritten(Box::new(ClientError::Closed)).into();
    let written: SessionError = ClientError::Closed.into();
    let lines = lines_for(vec![Err(Lost::Call(unwritten)), Err(Lost::Call(written))]);
    assert_eq!(lines[0]["not_sent"], true, "{}", lines[0]);
    assert_eq!(lines[0]["disconnected"], true, "still a dropped link: {}", lines[0]);
    assert!(lines[1].get("not_sent").is_none(), "it may have been written: {}", lines[1]);
}

/// Through the whole path: a terminal write on a session whose connection
/// has ended but whose slot is not yet emptied.
#[test]
fn a_key_typed_on_a_link_that_just_died_is_not_sent() {
    let rig = rig();
    rig.runner.abort();
    // The reader notices the close on its own. Wait until it has, so the key
    // below is queued on a link already known dead, however loaded the machine.
    let session = locked(&unsafe { as_handle(rig.handle) }.unwrap().session).clone().expect("session");
    let deadline = Instant::now() + Duration::from_secs(60);
    while !session.link_ended() {
        assert!(Instant::now() < deadline, "the reader never saw the close");
        std::thread::sleep(Duration::from_millis(10));
    }
    let terminal = uuid::Uuid::now_v7().to_string();
    let ticket = call(rig.handle, "terminal.write", json!({ "terminal": terminal, "hex": "6c" }));
    let line = answer_for(rig.handle, ticket, Instant::now() + Duration::from_secs(60)).expect("answered");
    let line: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(line["ok"], false, "{line}");
    assert_eq!(line["not_sent"], true, "queued on a dead link, never written: {line}");
    unsafe { farcooler_client_free(rig.handle) };
}
