use std::ffi::CString;

/// A generated key and the public key derived from it have to agree, or the
/// device offers one identity and shows a human another.
#[test]
fn the_derived_public_key_matches_the_generated_one() {
    let comment = CString::new("test").unwrap();
    let mut buf = vec![0u8; 4096];
    let n = unsafe {
        super::farcooler_client_generate_key(comment.as_ptr(), buf.as_mut_ptr(), buf.len())
    };
    assert!(n > 0 && n <= buf.len());

    let pair: serde_json::Value = serde_json::from_slice(&buf[..n]).unwrap();
    let private = pair["private_key"].as_str().unwrap();
    let generated = pair["public_key"].as_str().unwrap();

    let key = CString::new(private).unwrap();
    let mut out = vec![0u8; 2048];
    let n = unsafe {
        super::farcooler_client_public_key(key.as_ptr(), out.as_mut_ptr(), out.len())
    };
    assert!(n > 0 && n <= out.len());
    let derived = String::from_utf8_lossy(&out[..n]).into_owned();

    // The comment is not part of the key, and `to_openssh` on a parsed key
    // keeps it, so compare the parts that identify it.
    let fields = |line: &str| -> String {
        line.split_whitespace().take(2).collect::<Vec<_>>().join(" ")
    };
    assert_eq!(fields(&derived), fields(generated));
}

#[test]
fn nonsense_is_refused_rather_than_guessed_at() {
    let junk = CString::new("not a key").unwrap();
    let mut out = vec![0u8; 256];
    assert_eq!(
        unsafe { super::farcooler_client_public_key(junk.as_ptr(), out.as_mut_ptr(), out.len()) },
        0
    );
    assert_eq!(
        unsafe { super::farcooler_client_fingerprint(junk.as_ptr(), out.as_mut_ptr(), out.len()) },
        0,
        "a fingerprint of something that will not parse is nothing, not a guess"
    );
}

/// Two notices saying the same thing are one notice.
///
/// The rule the whole push path is affordable because of: a runner samples
/// every second and a working agent moves a row on most of them, so without
/// this a client would make one round trip per sample and the push would be
/// a faster poll.
#[test]
fn the_same_news_twice_is_queued_once() {
    use crate::session::FleetEvent;
    let mut queue = super::EventQueue::default();
    for _ in 0..50 {
        queue.push(super::event_line(&FleetEvent::Fleet));
    }
    assert_eq!(queue.pending.len(), 1);
    assert_eq!(queue.pending[0], r#"{"event":"fleet"}"#);
}

/// A board notice is the line a phone reads to re-read ONE board: the
/// word `task` and the repository, as a uuid string it can hand straight
/// back to `task.list`.
#[test]
fn a_board_notice_names_the_repository_to_read_again() {
    use crate::session::FleetEvent;
    let repository = uuid::Uuid::now_v7();
    let line = super::event_line(&FleetEvent::Task {
        repository,
        workspace: None,
        from_workspace: None,
        actor: "manager".into(),
    });
    let parsed: serde_json::Value = serde_json::from_str(&line).expect("json");
    assert_eq!(parsed["event"], "task");
    assert_eq!(parsed["repository"], repository.to_string());
}

/// Two boards in one repository are two notices, and each names its own.
///
/// A board view is keyed by workspace now. If the line named only the
/// repository, a write to Billing and a write to Main would be the same
/// line, the queue would keep one, and the board it did not name would
/// never re-read. A move names the board it left as well, as
/// `from_workspace`, because that board changed too.
#[test]
fn two_boards_in_one_repository_are_two_notices() {
    use crate::session::FleetEvent;
    let repository = uuid::Uuid::now_v7();
    let main = uuid::Uuid::now_v7();
    let billing = uuid::Uuid::now_v7();
    let on = |workspace, from_workspace| {
        super::event_line(&FleetEvent::Task {
            repository,
            workspace: Some(workspace),
            from_workspace,
            actor: "user".into(),
        })
    };
    let mut queue = super::EventQueue::default();
    queue.push(on(main, None));
    queue.push(on(billing, None));
    queue.push(on(main, None));
    queue.push(on(main, Some(billing)));
    let lines: Vec<serde_json::Value> =
        queue.pending.iter().map(|l| serde_json::from_str(l).expect("json")).collect();
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert_eq!(lines[0]["workspace"], main.to_string());
    assert_eq!(lines[1]["workspace"], billing.to_string());
    assert!(lines[0]["from_workspace"].is_null());
    assert_eq!(lines[2]["workspace"], main.to_string());
    assert_eq!(lines[2]["from_workspace"], billing.to_string());
}

/// A board read's workspace is optional, and a malformed one is refused
/// rather than read as "the whole repository".
#[test]
fn a_board_reads_workspace_is_optional_but_never_guessed() {
    let billing = uuid::Uuid::now_v7();
    let read = |args: serde_json::Value| super::optional_id(&args, "workspace", "task.list");
    assert_eq!(read(serde_json::json!({ "repository": "r" })).unwrap(), None);
    assert_eq!(read(serde_json::json!({ "workspace": null })).unwrap(), None);
    assert_eq!(read(serde_json::json!({ "workspace": billing.to_string() })).unwrap(), Some(billing));
    assert!(read(serde_json::json!({ "workspace": "billing" })).is_err());
    assert!(read(serde_json::json!({ "workspace": 7 })).is_err());
}

/// News about different things is not collapsed together.
///
/// The other half of the rule: coalescing is by what a notice SAYS, and
/// two worktrees moving are two things to re-read.
#[test]
fn news_about_different_things_is_kept_apart() {
    use crate::session::FleetEvent;
    let mut queue = super::EventQueue::default();
    let one = uuid::Uuid::now_v7();
    let two = uuid::Uuid::now_v7();
    queue.push(super::event_line(&FleetEvent::ChangeSet { worktree: one }));
    queue.push(super::event_line(&FleetEvent::ChangeSet { worktree: two }));
    queue.push(super::event_line(&FleetEvent::ChangeSet { worktree: one }));
    assert_eq!(queue.pending.len(), 2);
}


/// A subscription that dies after a newer one is up cannot report the
/// newer one dead.
///
/// Reconnecting opens the new channel and drops the old session, and the
/// old subscription's task learns it is over milliseconds LATER. Without
/// the generation check the client would fall back to fast polling with a
/// working push path underneath it — which is invisible, because
/// everything still works, just at the cost this whole change removes.
#[test]
fn a_dying_subscription_does_not_report_the_live_one_dead() {
    let mut queue = super::EventQueue::default();
    let first = queue.opening();
    queue.opened(first);
    assert!(queue.live);

    let second = queue.opening();
    queue.opened(second);
    queue.closed(first);
    assert!(queue.live, "the first subscription's cleanup must not clear the second's");

    queue.closed(second);
    assert!(!queue.live);
}

/// The fingerprint a device shows belongs to the key it generated.
///
/// The whole point of the entry point: the string on the confirmation screen
/// and the key in the offer have to be the same identity, and the only way
/// to be sure is to derive one from the other.
#[test]
fn the_fingerprint_is_of_the_key_this_device_holds() {
    let mut buf = vec![0u8; 4096];
    let n = unsafe {
        super::farcooler_client_generate_key(std::ptr::null(), buf.as_mut_ptr(), buf.len())
    };
    let pair: serde_json::Value = serde_json::from_slice(&buf[..n]).unwrap();
    let public = CString::new(pair["public_key"].as_str().unwrap()).unwrap();

    let mut out = vec![0u8; 128];
    let n = unsafe {
        super::farcooler_client_fingerprint(public.as_ptr(), out.as_mut_ptr(), out.len())
    };
    let shown = String::from_utf8_lossy(&out[..n]).into_owned();
    assert!(shown.starts_with("SHA256:"), "{shown}");
    // Base64 of 32 bytes, unpadded, behind the prefix — a person compares
    // this against another screen character by character.
    assert_eq!(shown.len(), "SHA256:".len() + 43, "{shown}");
}
