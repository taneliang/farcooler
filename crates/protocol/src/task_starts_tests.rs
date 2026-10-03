//! The wire for when a task starts (ov-212) and who works it (ov-213).

use prost::Message;
use prost_types::FileDescriptorSet;

use crate::capability;

fn file() -> prost_types::FileDescriptorProto {
    let set = FileDescriptorSet::decode(&include_bytes!(concat!(env!("OUT_DIR"), "/farcooler_descriptor.bin"))[..])
        .expect("the build writes a descriptor");
    set.file.into_iter().find(|f| f.package() == "farcooler.v1").expect("farcooler.v1")
}

/// The tags these take, pinned: a field renumbered between two Canary
/// pushes is a phone and a runner disagreeing with no error anywhere, and
/// none of these is in a shipped baseline yet for proto-lint to hold.
#[test]
fn waits_and_workers_hold_their_tags() {
    let file = file();
    let number = |m: &str, f: &str| {
        file.message_type
            .iter()
            .find(|x| x.name() == m)
            .and_then(|x| x.field.iter().find(|x| x.name() == f))
            .unwrap_or_else(|| panic!("{m} has no field {f}"))
            .number()
    };
    assert_eq!(number("Task", "workspace_id"), 15, "the last field before these");
    assert_eq!(number("Task", "wait"), 16);
    assert_eq!(number("Task", "waiting_on"), 17);
    assert_eq!(number("Task", "workers"), 18);
    assert_eq!(number("Request", "task_set_wait"), 130);
    assert_eq!(number("Request", "task_set_line"), 131);
    assert_eq!(number("Request", "task_worker_set"), 132);
    assert_eq!(number("TaskWorker", "orchestrator_terminal"), 12);

    let kinds = file.enum_type.iter().find(|e| e.name() == "TaskNoteKind").expect("TaskNoteKind");
    let value = |name: &str| kinds.value.iter().find(|v| v.name() == name).map(|v| v.number());
    assert_eq!(value("TASK_NOTE_KIND_CREATED"), Some(8));
    assert_eq!(value("TASK_NOTE_KIND_WAIT"), Some(9));
    assert_eq!(value("TASK_NOTE_KIND_WORKER"), Some(10));
}

/// Two capabilities, so either half can ship alone, each advertised and
/// each owning its methods.
#[test]
fn waits_and_workers_are_two_advertised_capabilities() {
    assert_eq!(capability::TASK_WAITS, "task_waits");
    assert_eq!(capability::TASK_WORKERS, "task_workers");
    assert!(capability::ALL.contains(&capability::TASK_WAITS), "the daemon would not advertise it");
    assert!(capability::ALL.contains(&capability::TASK_WORKERS), "the daemon would not advertise it");
    assert_eq!(capability::for_method("task.set_wait"), Some(capability::TASK_WAITS));
    assert_eq!(capability::for_method("task.set_line"), Some(capability::TASK_WAITS));
    assert_eq!(capability::for_method("task.worker"), Some(capability::TASK_WORKERS));
}
