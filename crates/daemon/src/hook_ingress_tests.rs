use super::*;

#[test]
fn a_preset_names_its_agent_with_or_without_a_model() {
    assert_eq!(preset_agent("codex"), Some(Agent::Codex));
    assert_eq!(preset_agent("codex:gpt-5.6-terra"), Some(Agent::Codex));
    assert_eq!(preset_agent("cursor"), Some(Agent::Cursor));
    assert_eq!(preset_agent("shell"), None, "a shell pane hosts no agent");
    assert_eq!(preset_agent(""), None);
}

/// The descriptor shortage is the case that matters, and it is the one
/// `ErrorKind` does not name.
///
/// A reader written against `ErrorKind` alone looks complete — it handles
/// `Interrupted` and `ConnectionAborted` — and still ends the listener for
/// good on `EMFILE`, which is both the likeliest cause and the one that
/// mends itself. `EMFILE` and `ENFILE` arrive as `Uncategorized`, which no
/// pattern may match, so they are matched by errno or not at all.
#[test]
fn a_descriptor_shortage_is_a_refused_connection_and_not_a_broken_socket() {
    for errno in [libc::EMFILE, libc::ENFILE, libc::ENOBUFS, libc::ENOMEM] {
        let e = std::io::Error::from_raw_os_error(errno);
        assert!(
            transient(&e),
            "errno {errno} ({e}) arrives as {:?} and must not end the listener",
            e.kind()
        );
    }
    for kind in [std::io::ErrorKind::Interrupted, std::io::ErrorKind::ConnectionAborted] {
        assert!(transient(&std::io::Error::from(kind)), "{kind:?} is about one connection");
    }
}

/// And something that really is the socket still stops it.
///
/// Without this the rule could be "everything is transient", which never
/// returns and never reports — the accept loop would spin forever on a
/// listener that is genuinely gone.
#[test]
fn a_socket_that_is_actually_broken_is_not_treated_as_transient() {
    for errno in [libc::EBADF, libc::EINVAL, libc::ENOTSOCK] {
        let e = std::io::Error::from_raw_os_error(errno);
        assert!(!transient(&e), "errno {errno} ({e}) is the listener itself");
    }
}

/// A refusal that will not go away must not become the log.
///
/// The first is said at once — a runner going deaf has to say so when it
/// happens — and then the rule has to hold its tongue, because the
/// condition it was written for persists and the loop meets it about
/// seven times a second. The nearest wrong implementation is the one this
/// replaced: a line every time, which is thousands an hour under exactly
/// the circumstance the warning was added for.
#[test]
fn a_refusal_that_persists_is_reported_once_and_then_rarely() {
    let mut refusals = Refusals::default();
    let start = std::time::Instant::now();

    assert_eq!(refusals.refused(start), Some(1), "the first is always worth a line");

    // A second of it, at the rate the loop actually runs.
    let mut at = start;
    for _ in 0..7 {
        at += ACCEPT_RETRY_PAUSE;
        assert_eq!(refusals.refused(at), None, "and then it stops talking");
    }

    // Past the reporting interval, one line, carrying what the silence held.
    let later = start + REFUSAL_REPORT_EVERY;
    assert_eq!(
        refusals.refused(later),
        Some(8),
        "the repeat says how many refusals it is standing for"
    );
}

/// And the count is what makes the quiet safe to have.
#[test]
fn recovery_reports_every_refusal_the_quiet_covered() {
    let mut refusals = Refusals::default();
    let start = std::time::Instant::now();
    for i in 0..100 {
        refusals.refused(start + ACCEPT_RETRY_PAUSE * i);
    }

    assert_eq!(
        refusals.recovered(),
        Some(100),
        "every refusal is counted, including the ones no line was written for"
    );
    assert_eq!(refusals.recovered(), None, "and a socket that never stopped says nothing");
}

/// The socket has to fit, and the reason it might not is the runtime
/// directory rather than anything chosen here.
#[test]
fn the_hook_socket_fits_in_a_unix_socket_path() {
    let real = Path::new("/Users/somebody/Library/Application Support/com.farcooler.Far Cooler");
    let path = HookIngress::socket_path(real);
    assert!(
        path.as_os_str().len() <= crate::paths::MAX_SOCKET_PATH,
        "{} is {} bytes, over the {} a bind accepts",
        path.display(),
        path.as_os_str().len(),
        crate::paths::MAX_SOCKET_PATH
    );
}

// -- start_transcript_tail: the side effect `serve` reaches for codex
// and cursor -- exercised directly against `install_sink`, without a
// real socket, since `listen` never returns except on a broken listener.

/// The hook hears back inside its 400 ms whatever the claim check costs.
///
/// `observe_cwd` is a store read, a write when it claims, and the claims
/// ledger's lock. A claude hook's `cwd` inside its pane's own worktree
/// reaches that lock (`claims::observe_in` settles an owned worktree), so
/// holding it here stands in for a claim check that is slow for any
/// reason. Multi-threaded, because the check that waits on it must not
/// take the only thread the reply could be written from.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_reply_does_not_wait_for_the_claim_check() {
    use farcooler_protocol::v1::TerminalIntent;
    use farcooler_store::models::{ClaimSource, PaneMode};

    let dir = tempfile::tempdir().expect("dir");
    let root = std::fs::canonicalize(dir.path()).expect("canonical");
    let repo_path = root.join("repo");
    std::fs::create_dir_all(&repo_path).expect("repo dir");
    let store = Store::open_in_memory().expect("store");
    let host = Uuid::now_v7();
    let root_row = store.create_repository_root(host, &root.to_string_lossy(), 1_000).unwrap();
    let repo = store
        .create_repository(host, root_row.id, "repo", &repo_path.join(".git").to_string_lossy(), "")
        .unwrap();
    let main = store.ensure_main_workspace(repo.id).unwrap();
    let checkout = store.create_worktree(repo.id, "main", &repo_path.to_string_lossy(), true).unwrap();
    store.claim_worktree(checkout.id, main.id, ClaimSource::Explicit).unwrap();
    let term = store.create_terminal(checkout.id, "pane", "claude", TerminalIntent::Running, 80, 24).unwrap();
    let term = store.set_terminal_workspace(term.id, main.id).unwrap();
    store
        .set_pane_mode(term.id, term.resource_version, PaneMode::Terminal, Some("sess-1".to_string()), false)
        .unwrap();

    let claims = Arc::new(crate::claims::Ledger::default());
    let inventory: Arc<dyn RuntimeInventory> =
        Arc::new(farcooler_core::inventory::FakeInventory::default());
    let ingress = HookIngress::new(Arc::new(store), inventory, claims.clone());
    let sock = tempfile::tempdir().expect("dir");
    {
        let ingress = ingress.clone();
        let sock = sock.path().to_path_buf();
        tokio::spawn(async move { ingress.listen(&sock, |_, _| {}).await });
    }
    let socket = HookIngress::socket_path(sock.path());
    for _ in 0..200 {
        if socket.exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    let busy = claims.lock_for_test();
    let line = HookLine {
        agent: Agent::Claude,
        event: "PermissionRequest".to_string(),
        payload: serde_json::json!({
            "session_id": "sess-1",
            "cwd": repo_path,
            "tool_name": "Bash",
            "tool_input": { "command": "touch x" },
        }),
        takes: Vec::new(),
        launch: None,
    };
    // A std socket with the kernel's read timeout, not tokio's timer:
    // with a worker parked in the claim check, nothing is certain to
    // drive tokio's time driver, and a test that hangs proves nothing.
    let frame = farcooler_agent_hooks::wire::encode_line(&line).expect("encode");
    let mut stream = std::os::unix::net::UnixStream::connect(&socket).expect("connect");
    stream.set_read_timeout(Some(std::time::Duration::from_secs(1))).expect("timeout");
    std::io::Write::write_all(&mut stream, frame.as_bytes()).expect("write");
    let mut reply = String::new();
    let heard = std::io::BufRead::read_line(&mut std::io::BufReader::new(&stream), &mut reply);
    drop(busy);
    assert!(
        matches!(heard, Ok(n) if n > 0),
        "the hook heard nothing while the claim check waited: {heard:?}"
    );
    assert!(reply.contains("hold_ms"), "{reply:?}");

    // And the ask it holds names claude's tool and ends when the hold
    // does, which is what the lock screen's card is sent (ov-57).
    let mut open = None;
    for _ in 0..200 {
        open = ingress.asks().open_on(term.id);
        if open.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let open = open.expect("the ask was offered");
    assert_eq!(open.tool.as_deref(), Some("Bash"));
    let left = open.until.duration_since(std::time::SystemTime::now()).expect("not over yet");
    assert!(left <= farcooler_agent_hooks::wire::LONGEST_HOLD, "{left:?}");
    assert!(left > farcooler_agent_hooks::wire::LONGEST_HOLD / 2, "{left:?}");
    drop(stream);
}

fn ingress_for_test() -> HookIngress {
    let store = Arc::new(Store::open_in_memory().expect("store"));
    let inventory: Arc<dyn RuntimeInventory> =
        Arc::new(farcooler_core::inventory::FakeInventory::default());
    HookIngress::new(store, inventory, Arc::default())
}

/// Poll rather than a fixed sleep: both the initial catch-up read and the
/// fallback poll inside `TranscriptTail::follow` (`WAIT_POLL_FALLBACK`)
/// deliver on their own schedule, not this test's.
async fn until_len(events: &Arc<Mutex<Vec<(Uuid, AgentEvent)>>>, n: usize, deadline_ms: u64) {
    let start = std::time::Instant::now();
    loop {
        if events.lock().unwrap().len() >= n
            || start.elapsed() > std::time::Duration::from_millis(deadline_ms)
        {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}

/// The nearest wrong implementation starts a fresh `TranscriptTail` on
/// every payload, which would deliver one appended line as two events —
/// the exact doubling this module's own doc on `tails` warns about.
#[tokio::test]
async fn a_second_payload_for_the_same_terminal_does_not_start_a_second_tail() {
    let ingress = ingress_for_test();
    let events: Arc<Mutex<Vec<(Uuid, AgentEvent)>>> = Arc::new(Mutex::new(Vec::new()));
    let sink_events = events.clone();
    ingress.install_sink(move |terminal, batch| {
        let mut events = sink_events.lock().unwrap();
        for event in batch {
            events.push((terminal, event));
        }
    });

    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("rollout.jsonl");
    std::fs::write(&path, "").expect("seed");
    let f = Facts { transcript_path: Some(path.clone()), ..Facts::default() };
    let terminal = Uuid::from_u128(101);

    ingress.start_transcript_tail(terminal, Agent::Codex, &f);
    ingress.start_transcript_tail(terminal, Agent::Codex, &f);
    assert!(ingress.is_tailing(terminal), "the first call must have started one");

    use std::io::Write;
    let mut file = std::fs::OpenOptions::new().append(true).open(&path).expect("open");
    writeln!(
        file,
        "{}",
        serde_json::json!({
            "type": "event_msg",
            "payload": { "type": "agent_message", "phase": "commentary", "message": "checking" },
        })
    )
    .expect("append");

    until_len(&events, 1, 20_000).await;
    let seen = events.lock().unwrap();
    assert_eq!(
        seen.len(),
        1,
        "one append must reach the sink once, however many times start_transcript_tail was called: {seen:?}"
    );
    assert!(
        matches!(
            &seen[0],
            (t, AgentEvent::Message { role: Role::Agent, text, parent: None })
                if *t == terminal && text == "checking"
        ),
        "got {seen:?}"
    );
}

/// A deleted terminal's held ask is released, not left for its minute: its
/// hook would otherwise keep claude waiting on a pane nobody can answer.
#[tokio::test]
async fn forgetting_a_terminal_releases_its_held_ask() {
    let ingress = ingress_for_test();
    let terminal = Uuid::from_u128(103);
    let (_id, rx) = ingress.asks().hold(terminal);
    ingress.forget(terminal);
    assert!(!ingress.asks().is_holding(terminal));
    assert_eq!(rx.await.expect("the hook is released").decision, None);
}

/// `forget` must silence a running tail, not merely leave `is_tracking`
/// (the assembler map) looking clean. The nearest wrong implementation
/// removes only the assembler entry, which is `forget`'s OLD body — a
/// tail started after that would keep delivering into a transcript whose
/// terminal no longer exists.
#[tokio::test]
async fn forget_silences_a_running_tail() {
    let ingress = ingress_for_test();
    let events: Arc<Mutex<Vec<(Uuid, AgentEvent)>>> = Arc::new(Mutex::new(Vec::new()));
    let sink_events = events.clone();
    // Kept, to count who else holds it: the ingress, this test, and each
    // running tail's thread.
    let sink = ingress.install_sink(move |terminal, batch| {
        let mut events = sink_events.lock().unwrap();
        for event in batch {
            events.push((terminal, event));
        }
    });

    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("rollout.jsonl");
    std::fs::write(&path, "").expect("seed");
    let f = Facts { transcript_path: Some(path.clone()), ..Facts::default() };
    let terminal = Uuid::from_u128(102);
    ingress.start_transcript_tail(terminal, Agent::Codex, &f);

    use std::io::Write;
    let append = |text: &str| {
        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(
            file,
            "{}",
            serde_json::json!({
                "type": "event_msg",
                "payload": { "type": "agent_message", "phase": "commentary", "message": text },
            })
        )
        .unwrap();
    };

    append("before forget");
    until_len(&events, 1, 20_000).await;
    assert_eq!(events.lock().unwrap().len(), 1, "the tail must be delivering before forget");
    let with_the_tail = Arc::strong_count(&sink);

    ingress.forget(terminal);
    assert!(!ingress.is_tailing(terminal), "forget must clear the tracked tail");

    // The positive event: the tail's thread ends and lets go of the sink.
    // A sleep and an unchanged count would pass just as well for a tail
    // still running that had not polled yet. Once the thread is gone,
    // nothing is left that could deliver a line.
    let start = std::time::Instant::now();
    while Arc::strong_count(&sink) >= with_the_tail && start.elapsed() < std::time::Duration::from_secs(10) {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert_eq!(Arc::strong_count(&sink), with_the_tail - 1, "the forgotten tail's thread is still running");

    append("after forget");
    assert_eq!(
        events.lock().unwrap().len(),
        1,
        "a line appended after forget must never reach the sink"
    );
}

/// The dedup invariant `codex_final_answer_is_dropped_because_stop_
/// already_sends_it` (in `transcript_tail`'s own tests) can only see
/// from the decode side: it proves `assistant_text` returns `None` for a
/// `final_answer` line, which is true whether or not anything ELSE ever
/// produces that text. It cannot see the other producer.
///
/// This drives BOTH real inputs into one terminal — codex's own
/// `final_answer` line, sitting in the rollout the tail is reading, AND
/// a `Stop` hook payload carrying the identical text as
/// `last_assistant_message` — and asserts exactly one `Message` reaches
/// the terminal's transcript. The nearest wrong implementation drops the
/// phase filter (forwards every `agent_message`/`AgentMessage`
/// regardless of phase): that implementation draws the same answer
/// twice, once from each producer, and this is the one test in the suite
/// that would actually see it, because it is the only one that gives
/// both producers something to say.
#[tokio::test]
async fn codexs_own_final_answer_line_and_its_stop_payload_do_not_both_land() {
    let ingress = ingress_for_test();
    let events: Arc<Mutex<Vec<(Uuid, AgentEvent)>>> = Arc::new(Mutex::new(Vec::new()));
    let sink_events = events.clone();
    ingress.install_sink(move |terminal, batch| {
        let mut events = sink_events.lock().unwrap();
        for event in batch {
            events.push((terminal, event));
        }
    });

    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("rollout.jsonl");
    // Empty, not merely absent: nothing is written to this file before
    // the tail starts, so whatever `from` the tail computes for itself
    // is 0 regardless of exactly when it computes it — the append below
    // races nothing, because there is nothing in the file yet to have
    // already been "caught up" on.
    std::fs::write(&path, "").expect("seed");
    let f = Facts { transcript_path: Some(path.clone()), ..Facts::default() };
    let terminal = Uuid::from_u128(106);

    ingress.start_transcript_tail(terminal, Agent::Codex, &f);

    let answer = "TCP slow start is a congestion-control mechanism.";
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new().append(true).open(&path).expect("open");
    writeln!(
        file,
        "{}",
        serde_json::json!({
            "type": "event_msg",
            "payload": {
                "type": "agent_message",
                "phase": "final_answer",
                "message": answer,
            },
        })
    )
    .expect("append");
    // A commentary line after it, which the tail does forward. The tail
    // reads the file in order, so once this has arrived the final_answer
    // line has been read too, and anything it was going to send has been
    // sent: the tail has had its turn BEFORE the Stop payload below gives
    // the real producer its turn. A race the other direction (Stop firing
    // before the tail has even looked at the file) would prove nothing,
    // since a dropped implementation and a correct one would both show
    // one Message either way. A fixed sleep here passed whether or not the
    // tail had got that far.
    let sentinel = "Reading the congestion window code.";
    writeln!(
        file,
        "{}",
        serde_json::json!({
            "type": "event_msg",
            "payload": { "type": "agent_message", "phase": "commentary", "message": sentinel },
        })
    )
    .expect("append");
    until_len(&events, 1, 20_000).await;
    assert!(
        matches!(
            events.lock().unwrap().as_slice(),
            [(_, AgentEvent::Message { text, .. })] if text == sentinel
        ),
        "codex's own final_answer line must never reach the sink by itself: {:?}",
        events.lock().unwrap()
    );

    let stop_events = ingress.accept(
        terminal,
        Agent::Codex,
        "Stop",
        &serde_json::json!({
            "hook_event_name": "Stop",
            "session_id": "s",
            "turn_id": "t",
            "last_assistant_message": answer,
        }),
        None,
    );
    events.lock().unwrap().extend(stop_events.into_iter().map(|e| (terminal, e)));

    let seen = events.lock().unwrap();
    let messages: Vec<_> = seen
        .iter()
        .filter(|(_, e)| matches!(e, AgentEvent::Message { role: Role::Agent, text, .. } if text != sentinel))
        .collect();
    assert_eq!(
        messages.len(),
        1,
        "the rollout's own final_answer line and Stop's last_assistant_message name the \
         same turn's closing answer; exactly one of the two producers may land it: {seen:?}"
    );
    assert!(
        matches!(
            messages[0],
            (t, AgentEvent::Message { text, parent: None, .. }) if *t == terminal && text == answer
        ),
        "got {:?}",
        messages[0]
    );
}

/// A tail that could not start must RELEASE the terminal's slot rather
/// than leave it claimed. The nearest wrong implementation is the one
/// this round replaced: insert into `tails` and never look at whether
/// anything actually started, which marks the terminal tailed for the
/// rest of the daemon's life with nothing running behind it and no later
/// payload able to try again — the whole feature silently off for that
/// terminal, at `debug!`.
///
/// `/` is the cheapest transcript path with no parent directory to watch,
/// which is the one case `TranscriptTail::follow` still refuses outright
/// rather than degrading to polling.
#[tokio::test]
async fn a_tail_that_could_not_start_releases_the_terminal_for_the_next_payload() {
    let ingress = ingress_for_test();
    ingress.install_sink(|_, _| {});

    let f = Facts { transcript_path: Some(PathBuf::from("/")), ..Facts::default() };
    let terminal = Uuid::from_u128(107);
    ingress.start_transcript_tail(terminal, Agent::Codex, &f);

    // The slot is claimed synchronously and released by the spawned task,
    // so this polls rather than reading once: what is being asserted is
    // that the release happens at all, not how quickly.
    let start = std::time::Instant::now();
    while ingress.is_tailing(terminal) && start.elapsed() < std::time::Duration::from_secs(10) {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert!(
        !ingress.is_tailing(terminal),
        "a terminal whose tail never started must be left free for its next payload to retry"
    );
}

/// A start that fails after `forget` has already taken its slot must not
/// free the slot the NEXT start claimed since. The nearest wrong
/// implementation removes by terminal alone: the newer tail keeps
/// delivering with no entry left for `forget` to stop it by, and the
/// payload after that starts a second tail on the same file.
///
/// The first start fails in the `Err` arm: its task panics during the
/// catch-up read, because the sink panics on the one line written
/// between the first start's `stat` and its read. The sink holds that
/// panic until `forget` and the second start have both landed, which
/// fixes the order. The second start's `stat` comes after the line, so
/// its tail never reads it.
#[tokio::test]
async fn a_start_that_panics_after_forget_leaves_the_next_starts_slot_alone() {
    let ingress = ingress_for_test();
    let reached = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let (sink_reached, sink_release) = (reached.clone(), release.clone());
    ingress.install_sink(move |_, batch| {
        if batch.iter().any(|e| matches!(e, AgentEvent::Message { text, .. } if text == "boom")) {
            sink_reached.store(true, Ordering::SeqCst);
            // A blocking-pool thread, so this wait stalls nothing the test needs.
            let start = std::time::Instant::now();
            while !sink_release.load(Ordering::SeqCst) && start.elapsed() < std::time::Duration::from_secs(10) {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            panic!("the first start's catch-up read fails here, on purpose");
        }
    });

    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("rollout.jsonl");
    std::fs::write(&path, "").expect("seed");
    let f = Facts { transcript_path: Some(path.clone()), ..Facts::default() };
    let terminal = Uuid::from_u128(108);

    ingress.start_transcript_tail(terminal, Agent::Codex, &f);
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new().append(true).open(&path).expect("open");
    writeln!(
        file,
        "{}",
        serde_json::json!({
            "type": "event_msg",
            "payload": { "type": "agent_message", "phase": "commentary", "message": "boom" },
        })
    )
    .expect("append");

    let start = std::time::Instant::now();
    while !reached.load(Ordering::SeqCst) && start.elapsed() < std::time::Duration::from_secs(10) {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert!(reached.load(Ordering::SeqCst), "the first start never reached its catch-up read");

    ingress.forget(terminal);
    ingress.start_transcript_tail(terminal, Agent::Codex, &f);
    assert!(ingress.is_tailing(terminal), "the second start claimed the slot");
    release.store(true, Ordering::SeqCst);

    // The `Err` arm runs on this runtime once the panic has unwound and
    // this test yields. A second is ample for that.
    let start = std::time::Instant::now();
    while ingress.is_tailing(terminal) && start.elapsed() < std::time::Duration::from_secs(1) {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert!(
        ingress.is_tailing(terminal),
        "the first start's failure freed the slot the second start holds"
    );
}

/// The same race through the `Ok(false)` arm: `/` has no parent directory
/// to watch, so the first start returns `false` (as in
/// `a_tail_that_could_not_start_releases_the_terminal_for_the_next_payload`).
/// There is no signal for when it has, so this waits a second, which is
/// far longer than a failed read of `/` takes.
#[tokio::test]
async fn a_start_that_could_not_start_after_forget_leaves_the_next_starts_slot_alone() {
    let ingress = ingress_for_test();
    ingress.install_sink(|_, _| {});

    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("rollout.jsonl");
    std::fs::write(&path, "").expect("seed");
    let terminal = Uuid::from_u128(109);

    let root = Facts { transcript_path: Some(PathBuf::from("/")), ..Facts::default() };
    ingress.start_transcript_tail(terminal, Agent::Codex, &root);
    ingress.forget(terminal);
    let f = Facts { transcript_path: Some(path), ..Facts::default() };
    ingress.start_transcript_tail(terminal, Agent::Codex, &f);
    assert!(ingress.is_tailing(terminal), "the second start claimed the slot");

    let start = std::time::Instant::now();
    while ingress.is_tailing(terminal) && start.elapsed() < std::time::Duration::from_secs(1) {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert!(
        ingress.is_tailing(terminal),
        "the first start's failure freed the slot the second start holds"
    );
}

/// The helper behind all three failure exits, on its own: a release
/// leaves an entry some other start inserted, and removes its own. The
/// two race tests above can only wait and see that nothing happened, so
/// this is the check that decides it.
#[test]
fn releasing_a_tail_slot_removes_only_the_releasers_own_entry() {
    let tails = Mutex::new(HashMap::new());
    let terminal = Uuid::from_u128(110);
    let a = Arc::new(AtomicBool::new(true));
    let b = Arc::new(AtomicBool::new(true));
    tails.lock().unwrap().insert(terminal, a.clone());

    release_tail_slot(&tails, terminal, &b);
    assert!(tails.lock().unwrap().contains_key(&terminal), "a release by another start took this entry");

    release_tail_slot(&tails, terminal, &a);
    assert!(!tails.lock().unwrap().contains_key(&terminal), "a release by the entry's own start left it");
}

/// A start whose task panics, with no `forget` racing it, frees its own
/// slot through the `Err` arm, so the next payload can start a tail.
#[tokio::test]
async fn a_start_that_panics_releases_the_terminal_for_the_next_payload() {
    let ingress = ingress_for_test();
    ingress.install_sink(|_, batch| {
        if batch.iter().any(|e| matches!(e, AgentEvent::Message { text, .. } if text == "boom")) {
            panic!("the start's catch-up read fails here, on purpose");
        }
    });

    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("rollout.jsonl");
    std::fs::write(&path, "").expect("seed");
    let f = Facts { transcript_path: Some(path.clone()), ..Facts::default() };
    let terminal = Uuid::from_u128(111);

    ingress.start_transcript_tail(terminal, Agent::Codex, &f);
    assert!(ingress.is_tailing(terminal), "the start claimed the slot");
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new().append(true).open(&path).expect("open");
    writeln!(
        file,
        "{}",
        serde_json::json!({
            "type": "event_msg",
            "payload": { "type": "agent_message", "phase": "commentary", "message": "boom" },
        })
    )
    .expect("append");

    let start = std::time::Instant::now();
    while ingress.is_tailing(terminal) && start.elapsed() < std::time::Duration::from_secs(10) {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert!(!ingress.is_tailing(terminal), "a start that panicked kept its slot");
}

/// Claude already streams through `MessageDisplay` — starting a tail for
/// it too would draw its answers a second time from the transcript.
#[tokio::test]
async fn claude_never_gets_a_transcript_tail() {
    let ingress = ingress_for_test();
    ingress.install_sink(|_, _| {});

    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("session.jsonl");
    std::fs::write(&path, "").expect("seed");
    let f = Facts { transcript_path: Some(path), ..Facts::default() };
    let terminal = Uuid::from_u128(103);

    ingress.start_transcript_tail(terminal, Agent::Claude, &f);
    assert!(!ingress.is_tailing(terminal), "claude's own MessageDisplay already streams its prose");
}

/// A payload naming no transcript at all — every real one does
/// (`Facts`'s own doc), but nothing here should assume it.
#[tokio::test]
async fn no_transcript_path_starts_no_tail() {
    let ingress = ingress_for_test();
    ingress.install_sink(|_, _| {});

    let terminal = Uuid::from_u128(104);
    ingress.start_transcript_tail(terminal, Agent::Codex, &Facts::default());
    assert!(!ingress.is_tailing(terminal));
}

/// Before `listen` (or, here, `install_sink`) has run, `self.sink` is
/// `None`. A tail started anyway would be a background thread doing work
/// for a sink that will never exist — reachable only from a test that
/// calls `accept` or `start_transcript_tail` directly, since `serve` is
/// only ever reached through `listen`, but worth refusing rather than
/// assuming.
#[tokio::test]
async fn no_sink_installed_starts_no_tail() {
    let ingress = ingress_for_test();
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("rollout.jsonl");
    std::fs::write(&path, "").expect("seed");
    let f = Facts { transcript_path: Some(path), ..Facts::default() };
    let terminal = Uuid::from_u128(105);

    ingress.start_transcript_tail(terminal, Agent::Codex, &f);
    assert!(!ingress.is_tailing(terminal), "nothing was installed to deliver to");
}
