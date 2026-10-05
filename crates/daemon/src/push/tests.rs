//! The push bodies and the pairing file, tested. Its own file to keep
//! `push.rs` inside the size budget (ov-310).

use super::*;

#[test]
fn a_beat_carries_its_promise_its_runner_its_name_and_nothing_else() {
    // Three programs read this body. The relay reads `beatEvery` in
    // seconds and clamps it; a rename or a unit change here is a runner
    // the phone judges by the wrong clock.
    let sent = serde_json::to_value(beat_body(Some("0190-abc"))).expect("serialize");
    assert_eq!(sent["beatEvery"], 300, "{sent}");
    assert_eq!(sent["install"], "0190-abc", "{sent}");
    assert_eq!(sent["version"], farcooler_protocol::BUILD, "{sent}");
    // The name the phone says "lost touch with" — never "This Mac".
    assert_eq!(sent["name"], runner_name(), "{sent}");
    // Which runner, as its phone knows it (`Host.runner_id`), so a watch
    // can tell which agents are this runner's (ov-71).
    assert_eq!(sent["runner"], crate::service::stable_host_id("0190-abc").to_string(), "{sent}");
    let keys: Vec<&str> = sent.as_object().expect("an object").keys().map(String::as_str).collect();
    assert_eq!(keys.len(), 5, "a beat says nothing else: {sent}");
}

/// The runner id a beat carries is the one `Host.runner_id` gives a
/// phone, pinned as text: the relay keys it and the phone hashes it, and
/// a change of spelling on one side is a watch that attributes nothing.
/// `services/relay/test/relay.test.ts` and `RunnerPulseTests` pin the
/// same literal.
#[test]
fn a_beats_runner_is_the_hosts_runner_id() {
    assert_eq!(
        crate::service::stable_host_id("install-with-more-than-sixteen-bytes").to_string(),
        "7537626f-0002-415e-1e11-000d48034210"
    );
}

#[test]
fn a_runner_has_a_name_and_it_is_not_a_domain() {
    let name = runner_name();
    assert!(!name.is_empty(), "a runner with no name would be named by its pairing label");
    assert!(name.chars().count() <= 64, "{name}");
    // A bare hostname is cut at its first dot: `studio.local` is Studio's.
    assert_eq!(short_host("studio.local"), "studio");
    assert_eq!(short_host("studio"), "studio");
}

/// Accept one request on `listener`, answer `status`, and hand back its
/// request line and body.
async fn one_request(listener: tokio::net::TcpListener, status: &'static str) -> (String, serde_json::Value) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (mut socket, _) = listener.accept().await.unwrap();
    let mut seen = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = socket.read(&mut buf).await.unwrap();
        seen.extend_from_slice(&buf[..n]);
        let text = String::from_utf8_lossy(&seen).to_string();
        if let Some(end) = text.find("\r\n\r\n") {
            let length = text[..end]
                .lines()
                .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap()))
                .unwrap_or(0);
            if seen.len() >= end + 4 + length {
                let reply = format!("HTTP/1.1 {status}\r\ncontent-length: 2\r\n\r\n{{}}");
                socket.write_all(reply.as_bytes()).await.unwrap();
                let line = text.lines().next().unwrap_or_default().to_string();
                return (line, serde_json::from_slice(&seen[end + 4..end + 4 + length]).unwrap());
            }
        }
        assert!(n != 0, "the relay's socket closed before a whole request");
    }
}

#[tokio::test]
async fn forgetting_a_pairing_withdraws_the_runner_first() {
    // Stop Notifying: the runner is fine, and must leave the phone's pulse
    // rather than read as "lost touch" for a day.
    let dir = tempfile::tempdir().expect("tempdir");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    Pairing { relay: format!("http://{}", listener.local_addr().unwrap()), token: "t".into() }
        .save_in(dir.path())
        .expect("save");
    let relay = tokio::spawn(one_request(listener, "200 OK"));
    assert!(forget_and_withdraw(dir.path()).await, "the withdrawal landed");
    let (line, body) = relay.await.unwrap();
    assert!(line.starts_with("POST /v1/heartbeat "), "{line}");
    assert_eq!(body["withdrawn"], true, "{body}");
    assert!(body.get("install").is_none(), "no install id on disk, none sent: {body}");
    assert!(Pairing::load_in(dir.path()).is_none(), "forget means forgotten");
    assert!(!dir.path().join("install-id").exists(), "forget mints no install id");
}

/// A withdrawal names the runner by its install id, so the relay can
/// clear every pairing of it (ov-77).
///
/// Mutation: `forget_and_withdraw` passing `None`. Red.
#[tokio::test]
async fn forgetting_a_pairing_names_the_runner_by_its_install_id() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("install-id"), "0199-abc\n").unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    Pairing { relay: format!("http://{}", listener.local_addr().unwrap()), token: "t".into() }
        .save_in(dir.path())
        .expect("save");
    let relay = tokio::spawn(one_request(listener, "200 OK"));
    assert!(forget_and_withdraw(dir.path()).await);
    let (_, body) = relay.await.unwrap();
    assert_eq!(body["withdrawn"], true, "{body}");
    assert_eq!(body["install"], "0199-abc", "{body}");
}

#[tokio::test]
async fn forgetting_still_forgets_when_the_relay_is_unreachable() {
    let dir = tempfile::tempdir().expect("tempdir");
    // A port nobody listens on: bound, then dropped.
    let addr = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap();
    Pairing { relay: format!("http://{addr}"), token: "t".into() }.save_in(dir.path()).expect("save");
    assert!(!forget_and_withdraw(dir.path()).await, "nothing landed");
    assert!(Pairing::load_in(dir.path()).is_none(), "the local unpair happens anyway");
}

#[test]
fn a_saved_pairing_comes_back_and_can_be_forgotten() {
    // The whole contract of `farcooler push pair|status|forget`, which had
    // no test at all: whether a runner is paired is the difference between
    // being told an agent is stuck and finding out in the morning.
    let dir = tempfile::tempdir().expect("tempdir");
    assert!(Pairing::load_in(dir.path()).is_none(), "nothing is paired yet");

    Pairing { relay: "https://mine.example".into(), token: "t".into() }
        .save_in(dir.path())
        .expect("save");
    let back = Pairing::load_in(dir.path()).expect("saved");
    assert_eq!(back.token, "t");
    assert_eq!(back.relay, "https://mine.example");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(config_path(dir.path()))
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "a bearer token must not be readable by anyone else");
    }

    Pairing::forget_in(dir.path());
    assert!(Pairing::load_in(dir.path()).is_none(), "forget means forgotten");
}

#[test]
fn a_pairing_round_trips_and_defaults_its_relay() {
    // The relay is optional in the file so an existing pairing keeps
    // working when the default moves, and a self-hoster can set it without
    // the daemon needing a different shape of config.
    let parsed: Pairing = serde_json::from_str(r#"{"token":"abc"}"#).expect("parse");
    assert_eq!(parsed.token, "abc");
    assert!(parsed.relay.starts_with("https://"));
    assert_eq!(parsed.relay, default_relay(), "an absent relay is this channel's own");
}

/// Build the body `notify` sends, without sending it.
///
/// The request itself is a network call no test can reach through, and the
/// part worth guarding is not the sending — it is the JSON, which three
/// separate programs have to agree about.
fn body(started_at: Option<i64>) -> serde_json::Value {
    stats_body(started_at, None, None, None, &[], None)
}

/// The same body with a row's numbers on it. See `Notification::insertions`.
fn stats_body(
    started_at: Option<i64>,
    insertions: Option<u32>,
    deletions: Option<u32>,
    commits: Option<u32>,
    trace: &[u8],
    trace_anchor: Option<i64>,
) -> serde_json::Value {
    serde_json::to_value(
        wire_body(&Outgoing {
            title: "claude",
            subtitle: "3/7 · Designing test matrix",
            status: "working",
            label: "claude",
            terminal: Some("term-1"),
            started_at,
            insertions,
            deletions,
            commits,
            trace,
            trace_anchor,
            ..Outgoing::default()
        })
        .expect("an agent notice with a terminal"),
    )
    .expect("serialize")
}

#[test]
fn a_turn_clock_goes_out_as_a_number_or_not_at_all() {
    // The seam this field exists to close, and the one place it can be
    // checked. The phone renders a native timer from `startedAt` and its
    // decoder takes a NUMBER — a string reads back as nil there, which
    // costs the timer and reports nothing at all. Nothing else in this
    // repository would notice.
    let sent = body(Some(1_755_000_000_000));
    assert_eq!(
        sent["startedAt"],
        serde_json::json!(1_755_000_000_000_i64),
        "the relay reads `startedAt`, not `started_at`"
    );
    assert!(sent["startedAt"].is_number(), "a string decodes to nil on the phone: {sent}");

    // Between turns there is no clock, and no key. A `null` would be
    // harmless and a `0` would not: it is a decodable instant, and the card
    // would come up counting the fifty-odd years since January 1970.
    let quiet = body(None);
    assert!(
        quiet.get("startedAt").is_none(),
        "no turn is running, so the card must be given no clock: {quiet}"
    );
}

#[test]
fn a_row_sends_its_numbers_or_no_key_at_all() {
    // The three counts and the trace are what a card row is made of, and
    // this is the only place on this side that spells their keys. The
    // relay reads them by name and stores what it reads; a rename here
    // would show up as a fleet card with no numbers on it, which is a card
    // that still renders and says less — the kind of failure nobody
    // reports.
    let trace = vec![0x10u8; farcooler_core::trace::ENCODED_LEN];
    let sent = stats_body(None, Some(142), Some(37), Some(4), &trace, Some(5_960_000));
    assert_eq!(sent["insertions"], serde_json::json!(142));
    assert_eq!(sent["deletions"], serde_json::json!(37));
    assert_eq!(sent["commits"], serde_json::json!(4));

    // Base64 and not sixty-six numbers. The card these end up on has a hard
    // 4KB ceiling and several rows to fit inside it — see the field's own
    // note — so this is a size contract, not a formatting preference.
    let encoded = sent["trace"].as_str().expect("a trace is a string");
    assert_eq!(encoded.len(), 88, "66 bytes is 88 base64 characters: {encoded}");
    assert_eq!(
        farcooler_core::base64::decode(encoded).as_deref(),
        Some(trace.as_slice()),
        "the relay stores this string and the widget decodes it; it has to round trip"
    );

    // The anchor is its own key and a NUMBER, beside the blob and never
    // inside it: the relay carries it as a column without decoding base64,
    // and the card's decoder reads it with `try?` as an integer, so a string
    // here would cost the shared axis without costing anything visible.
    assert_eq!(sent["traceAnchor"], serde_json::json!(5_960_000_i64));
    assert!(sent.get("trace_anchor").is_none(), "the relay reads `traceAnchor`: {sent}");

    // Absent is not zero, on every one of them. A worktree nobody has
    // probed and one with no base have both said nothing, and a card
    // drawing `+0 −0` over either would report a measurement nobody made.
    // A trace is the same shape of claim: no bytes means no history seen,
    // where sixty-six zeroes would mean thirteen buckets of observed quiet.
    let quiet = stats_body(None, None, None, None, &[], None);
    for key in ["insertions", "deletions", "commits", "trace", "traceAnchor"] {
        assert!(quiet.get(key).is_none(), "nothing measured `{key}`, so no key: {quiet}");
    }

    // And an anchor is never sent without the bytes it anchors, whatever
    // the caller handed in.
    let lone = stats_body(None, None, None, None, &[], Some(5_960_000));
    assert!(lone.get("traceAnchor").is_none(), "an anchor for no trace: {lone}");
}

/// The count's key is the relay's spelling. A `needs_you` key arrives
/// there as `undefined`, the machine's count never moves, and the lock
/// screen keeps showing the blocked count as if nothing were decided.
#[test]
fn the_body_spells_needs_you_in_camel_case() {
    let agent = serde_json::to_value(
        wire_body(&Outgoing {
            title: "Billing · claude needs you",
            status: "blocked",
            label: "claude",
            terminal: Some("term-1"),
            workspace: Some("Billing"),
            needs_you: Some(3),
            ..Outgoing::default()
        })
        .unwrap(),
    )
    .unwrap();
    assert_eq!(agent["needsYou"], serde_json::json!(3), "{agent}");
    assert!(agent.get("needs_you").is_none(), "{agent}");
    assert_eq!(agent["workspace"], "Billing");
    assert!(agent.get("kind").is_none(), "an agent notice has no kind: {agent}");

    let count = serde_json::to_value(
        wire_body(&Outgoing { kind: Some("count"), needs_you: Some(0), ..Outgoing::default() }).unwrap(),
    )
    .unwrap();
    let mut keys: Vec<_> = count.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(keys, ["kind", "needsYou", "version"], "a count is the count and nothing else: {count}");
    assert_eq!(count["needsYou"], serde_json::json!(0), "zero is a count, and is sent");
}

/// A decision is about a task, which is not a roster row: no terminal,
/// and none of an agent notice's status fields.
#[test]
fn a_decision_notice_names_its_task_and_no_terminal() {
    let decision = serde_json::to_value(
        wire_body(&Outgoing {
            kind: Some("decision"),
            title: "Billing · bil-7 needs a decision",
            subtitle: "Which PDF library?",
            terminal: Some("term-1"),
            task: Some("bil-7"),
            workspace: Some("Billing"),
            needs_you: Some(2),
            ..Outgoing::default()
        })
        .unwrap(),
    )
    .unwrap();
    for key in ["terminal", "status", "label", "failed"] {
        assert!(decision.get(key).is_none(), "a decision carries no `{key}`: {decision}");
    }
    assert_eq!(decision["task"], "bil-7");
    assert_eq!(decision["kind"], "decision");
    assert_eq!(decision["title"], "Billing · bil-7 needs a decision");
}

/// A task notice (ov-94) is about a task: no terminal, none of an agent
/// notice's fields, its id, class and level, and a decision's options,
/// which no other class carries, ever.
#[test]
fn a_task_notice_names_its_task_its_id_and_never_a_terminal() {
    let options = vec!["pdfkit".to_string(), "pdf.js".to_string()];
    let notice = |event: &'static str| {
        serde_json::to_value(
            wire_body(&Outgoing {
                kind: Some("task"),
                title: "ov-90 Wake the agent",
                subtitle: "Needs your decision · Which?",
                terminal: Some("term-1"),
                status: "blocked",
                task: Some("ov-90"),
                install: Some("install-1"),
                notice_id: Some("t:r-1:ov-90"),
                event: Some(event),
                level: Some("time-sensitive"),
                options: &options,
                ..Outgoing::default()
            })
            .unwrap(),
        )
        .unwrap()
    };
    let decision = notice("decision");
    for key in ["terminal", "status", "label", "failed", "ask", "alert"] {
        assert!(decision.get(key).is_none(), "a task notice carries no `{key}`: {decision}");
    }
    assert_eq!(decision["kind"], "task");
    assert_eq!(decision["task"], "ov-90");
    assert_eq!(decision["noticeId"], "t:r-1:ov-90");
    assert_eq!(decision["event"], "decision");
    assert_eq!(decision["level"], "time-sensitive");
    assert_eq!(decision["options"], serde_json::json!(["pdfkit", "pdf.js"]));
    assert_eq!(decision["runner"], crate::service::stable_host_id("install-1").to_string());
    let review = notice("review");
    assert!(review.get("options").is_none(), "only a decision has buttons: {review}");
}

/// An agent working on a task sends its notice for the card alone:
/// `alert: false`, and every other agent notice says nothing about it.
#[test]
fn an_agent_notice_says_alert_false_only_when_its_task_alerts() {
    let agent = |alert| {
        serde_json::to_value(
            wire_body(&Outgoing { terminal: Some("term-1"), status: "blocked", alert, ..Outgoing::default() })
                .unwrap(),
        )
        .unwrap()
    };
    assert_eq!(agent(false)["alert"], false);
    assert!(agent(true).get("alert").is_none());
}

/// A decision names the runner it is on as a phone knows it
/// (`Host.runner_id`), the beat's own spelling, so a task key on two
/// runners can be routed (ov-72). So does an agent notice, so a tap can
/// wait for the runner its pane is on (ov-183). A count doesn't, and none
/// with no install id.
#[test]
fn a_decision_names_its_runner_as_the_phone_knows_it() {
    let sent = |kind: Option<&'static str>, install| {
        serde_json::to_value(
            wire_body(&Outgoing {
                kind,
                terminal: Some("term-1"),
                task: Some("bil-7"),
                install,
                ..Outgoing::default()
            })
            .unwrap(),
        )
        .unwrap()
    };
    let install = "install-with-more-than-sixteen-bytes";
    assert_eq!(
        sent(Some("decision"), Some(install))["runner"],
        "7537626f-0002-415e-1e11-000d48034210"
    );
    assert!(sent(Some("decision"), None).get("runner").is_none());
    assert_eq!(sent(None, Some(install))["runner"], "7537626f-0002-415e-1e11-000d48034210");
    assert!(sent(None, None).get("runner").is_none());
    assert!(sent(Some("count"), Some(install)).get("runner").is_none());
}

/// Every kind names the runner by its install id, so the relay can tell
/// two Macs both labeled "This Mac" apart, and a re-paired runner from a
/// second one. Absent when the caller has none, never `""`.
#[test]
fn every_kind_carries_the_install_id() {
    for outgoing in [
        Outgoing { terminal: Some("term-1"), install: Some("0199-abc"), ..Outgoing::default() },
        Outgoing { kind: Some("decision"), title: "bil-7", install: Some("0199-abc"), ..Outgoing::default() },
        Outgoing { kind: Some("count"), needs_you: Some(1), install: Some("0199-abc"), ..Outgoing::default() },
    ] {
        let sent = serde_json::to_value(wire_body(&outgoing).unwrap()).unwrap();
        assert_eq!(sent["install"], "0199-abc", "{sent}");
    }
    let bare = serde_json::to_value(
        wire_body(&Outgoing { kind: Some("count"), needs_you: Some(1), ..Outgoing::default() }).unwrap(),
    )
    .unwrap();
    assert!(bare.get("install").is_none(), "{bare}");
}

/// An ask as the contract spells it: 45 bytes of id, a tool, and `until`
/// in milliseconds.
fn an_ask() -> WireAsk {
    let until = std::time::UNIX_EPOCH + std::time::Duration::from_millis(1_790_551_063_000);
    WireAsk::new("hook-ask-0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b", Some("Bash"), until).expect("a valid ask")
}

fn agent_body(status: &str, ask: &WireAsk) -> serde_json::Value {
    serde_json::to_value(
        wire_body(&Outgoing {
            title: "claude needs you",
            status,
            label: "claude",
            terminal: Some("term-1"),
            ask: Some(ask),
            ..Outgoing::default()
        })
        .unwrap(),
    )
    .unwrap()
}

/// A blocked notice carries the ask open on its pane, so the card can
/// answer it with the app suspended (ov-57 C2.1).
#[test]
fn a_blocked_notice_carries_its_ask() {
    let sent = agent_body("blocked", &an_ask());
    assert_eq!(
        sent["ask"],
        serde_json::json!({
            "id": "hook-ask-0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b",
            "tool": "Bash",
            "until": 1_790_551_063_000_i64,
        }),
        "{sent}"
    );
    assert!(sent["ask"]["until"].is_i64(), "milliseconds, as a number: {sent}");
}

/// Only `blocked` carries an ask, whatever the caller handed in.
#[test]
fn a_working_notice_never_carries_an_ask() {
    for status in ["working", "done"] {
        let sent = agent_body(status, &an_ask());
        assert!(sent.get("ask").is_none(), "{status}: {sent}");
    }
}

/// A `kind:"ask"` notice names its pane, and says "none open now" by
/// having no `ask` key at all, never `"ask": null`.
#[test]
fn an_ask_notice_carries_terminal_and_omits_an_absent_ask() {
    let ask = an_ask();
    let told = serde_json::to_value(
        wire_body(&Outgoing {
            kind: Some("ask"),
            title: "never sent",
            status: "blocked",
            terminal: Some("term-1"),
            needs_you: Some(1),
            ask: Some(&ask),
            ..Outgoing::default()
        })
        .unwrap(),
    )
    .unwrap();
    let mut keys: Vec<_> = told.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(keys, ["ask", "kind", "needsYou", "terminal", "version"], "{told}");
    assert_eq!(told["ask"]["id"], ask.id());

    let none = serde_json::to_value(
        wire_body(&Outgoing { kind: Some("ask"), terminal: Some("term-1"), ..Outgoing::default() }).unwrap(),
    )
    .unwrap();
    assert!(none.get("ask").is_none(), "absent, never null: {none}");
    assert_eq!(none["terminal"], "term-1");
    assert!(
        wire_body(&Outgoing { kind: Some("ask"), ask: Some(&ask), ..Outgoing::default() }).is_none(),
        "an ask notice about no pane is not a body"
    );
}

/// With ov-61: an ask notice names the runner as every kind does.
#[test]
fn an_ask_notice_carries_the_install_id() {
    let sent = serde_json::to_value(
        wire_body(&Outgoing {
            kind: Some("ask"),
            terminal: Some("term-1"),
            install: Some("0199-abc"),
            ..Outgoing::default()
        })
        .unwrap(),
    )
    .unwrap();
    assert_eq!(sent["install"], "0199-abc", "{sent}");
}

/// The contract's bounds, applied before anything crosses: a bad id or
/// end costs the ask, a bad tool costs only the tool.
#[test]
fn an_ask_crosses_only_inside_its_bounds() {
    let until = std::time::UNIX_EPOCH + std::time::Duration::from_millis(1_790_551_063_000);
    let id = "hook-ask-0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b";
    assert!(WireAsk::new("chat-1", None, until).is_none(), "not a hook ask");
    assert!(WireAsk::new("hook-ask-", None, until).is_none(), "no tail");
    assert!(WireAsk::new(&format!("hook-ask-{}", "a".repeat(56)), None, until).is_none(), "65 bytes");
    assert!(WireAsk::new(&format!("hook-ask-{}", "a".repeat(55)), None, until).is_some(), "64 bytes");
    assert!(WireAsk::new("hook-ask-a b", None, until).is_none(), "a space");
    assert!(WireAsk::new(id, None, std::time::UNIX_EPOCH).is_none(), "an end of zero");
    for bad in ["", "Bash ls", "rm -rf /;", &"x".repeat(65)] {
        let ask = WireAsk::new(id, Some(bad), until).expect("a bad tool keeps the ask");
        assert_eq!(ask.tool, None, "{bad:?}");
    }
    for good in ["Bash", "mcp__github__create_issue", "a.b:c-d_e", &"x".repeat(64)] {
        assert_eq!(WireAsk::new(id, Some(good), until).unwrap().tool.as_deref(), Some(good));
    }
}

/// And an agent notice that names no pane is not sent at all.
#[test]
fn an_agent_notice_without_a_terminal_is_not_a_body() {
    assert!(wire_body(&Outgoing { title: "claude needs you", status: "blocked", ..Outgoing::default() }).is_none());
}

#[test]
fn a_retirement_names_terminals_and_nothing_else() {
    // The other end 400s a body whose `terminals` is not an array, and this
    // is the only place on this side that spells the key — so a rename here
    // would show up as cards that never come down, on a route whose whole
    // job is that they do. It carries no content for the same reason
    // `Notification` explains at length: the relay is a delivery service,
    // and a payload it does not hold is a payload it cannot leak. A
    // terminal id is a UUID this runner minted and says nothing about the
    // work in the pane.
    let terminals = vec!["term-1".to_string(), "term-2".to_string()];
    let sent = serde_json::to_value(Retirement { terminals: &terminals }).expect("serialize");
    assert_eq!(sent, serde_json::json!({ "terminals": ["term-1", "term-2"] }));
}

#[test]
fn each_channel_talks_to_its_own_relay() {
    // One deployment per channel: its own database and its own WorkOS
    // environment. Two channels sharing a relay would mean a preview
    // pairing could notify a stable app — an app that cannot reach the
    // runner that sent it, because they are different binaries at
    // different paths.
    use farcooler_protocol::Channel;
    let urls: Vec<_> = [Channel::Local, Channel::Canary, Channel::Preview, Channel::Stable]
        .iter()
        .map(|c| match c {
            Channel::Stable => "https://relay.farcooler.com",
            Channel::Preview => "https://relay-preview.farcooler.com",
            Channel::Canary => "https://relay-canary.farcooler.com",
            Channel::Local => "https://relay-local.farcooler.com",
        })
        .collect();
    let unique: std::collections::BTreeSet<_> = urls.iter().collect();
    assert_eq!(unique.len(), 4, "two channels cannot share a relay: {urls:?}");

    // Stable's is compiled into App Store binaries that cannot be told a
    // new one for days. It does not move.
    assert_eq!(urls[3], "https://relay.farcooler.com");

    let explicit: Pairing =
        serde_json::from_str(r#"{"token":"abc","relay":"https://mine.example"}"#)
            .expect("parse");
    assert_eq!(explicit.relay, "https://mine.example");
}
