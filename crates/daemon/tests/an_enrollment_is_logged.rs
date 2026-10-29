//! Every `client.enroll` is logged at INFO when it is asked and when it is
//! answered, whatever answered it, the scope gate included (ov-828): who
//! asked, for which device, and the outcome. Never a key.
//!
//! The owner's phone failed to add a browser to Cosmo and the runner's log
//! said nothing, because the only line was written after the scope gate that
//! refused it.
//!
//! Its own test binary, alone in its process, because a log capture races
//! every other test that reaches the same call site on another thread.

use std::sync::{Arc, Mutex};

use farcooler_daemon::rpc::RpcFactory;
use farcooler_daemon::service::Service;
use farcooler_protocol::v1::{self as pb, Request, Scope, request, response};
use farcooler_transport::{Handler as _, Peer};

const KEY_PHONE: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAICMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMj phone";
const KEY_BROWSER: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFJhEUXBcOjzsos5aFyw6hfDrERWHei9akIudYJGG2gR browser";
const NODE_BROWSER: &str = "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl tracing_subscriber::fmt::MakeWriter<'_> for Sink {
    type Writer = Sink;
    fn make_writer(&self) -> Sink {
        self.clone()
    }
}

fn factory(svc: &Arc<Service>, device: Option<&str>, scope: Scope) -> RpcFactory {
    let peer = Peer { client_id: device.map(str::to_string), scope };
    RpcFactory::new(svc.clone(), farcooler_daemon::watch::Watcher::new(svc.clone()), Arc::new(tokio::sync::Notify::new()), peer)
}

async fn call(factory: &RpcFactory, req: Request) -> Result<pb::result::Value, String> {
    match factory.handle(req).await.outcome {
        Some(response::Outcome::Result(r)) => Ok(r.value.expect("a value")),
        Some(response::Outcome::Error(e)) => Err(format!("{}:{}", farcooler_core::error::word_for(e.code), e.what)),
        other => panic!("no outcome: {other:?}"),
    }
}

fn enroll(key: &str, client_id: &str, scope: Scope, node_key: &str) -> Request {
    let p = pb::ClientEnroll {
        public_key: key.into(),
        label: "Chrome on Mac".into(),
        client_id: client_id.into(),
        scope: scope as i32,
        shell_access: false,
        node_key: node_key.into(),
        for_teammate: false,
        step_up_key: None,
        teammate: None,
    };
    Request { method: "client.enroll".into(), payload: Some(request::Payload::ClientEnroll(p)), ..Default::default() }
}

#[tokio::test]
async fn every_enrollment_is_logged_asked_and_answered() {
    let sink = Sink::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(sink.clone())
        .with_ansi(false)
        // The daemon's own default: a test at `debug` would pass against a log
        // that says nothing in production.
        .with_max_level(tracing::Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("the only subscriber in this binary");

    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let ssh = dir.path().join("home").join(".ssh");
    std::fs::create_dir_all(&ssh).unwrap();
    std::fs::set_permissions(&ssh, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
    let svc = Arc::new(Service::open_in(state).await.unwrap().enrolling_into(ssh.join("authorized_keys")));
    let local = factory(&svc, None, Scope::HostAdmin);
    call(&local, enroll(KEY_PHONE, "phone-o", Scope::Control, "")).await.expect("the owner's phone");

    let phone = factory(&svc, Some("phone-o"), Scope::Control);
    call(&phone, enroll(KEY_BROWSER, "browser-1", Scope::Control, NODE_BROWSER)).await.expect("enrolled");
    call(&phone, enroll(KEY_BROWSER, "browser-1", Scope::Control, NODE_BROWSER)).await.expect("again");
    let read = factory(&svc, Some("phone-r"), Scope::Read);
    call(&read, enroll(KEY_BROWSER, "browser-2", Scope::Read, "")).await.expect_err("refused at the gate");
    // The caller's text, before anything judged it: never logged as it came.
    call(&read, enroll(KEY_BROWSER, "evil\nINFO forged=1", Scope::Read, "")).await.expect_err("refused at the gate");
    // Another method carrying the payload is no enrollment.
    let mut other = enroll(KEY_BROWSER, "browser-3", Scope::Control, "");
    other.method = "client.list".into();
    call(&phone, other).await.expect("a list");

    let logged = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
    let lines: Vec<&str> = logged.lines().collect();
    let asked: Vec<&&str> = lines.iter().filter(|l| l.contains("a device asked to enroll another")).collect();
    assert_eq!(asked.len(), 5, "{logged}");
    assert!(!logged.contains("forged=1") && !logged.contains("browser-3"), "{logged}");
    assert!(lines.iter().any(|l| l.contains("client=\"(invalid)\"")), "{logged}");
    assert!(asked.iter().all(|l| l.contains(" INFO ")), "{logged}");
    let answered = |by: &str, client: &str, outcome: &str| {
        lines.iter().any(|l| {
            l.contains(" INFO ")
                && l.contains(&format!("by={by:?} "))
                && l.contains(&format!("client={client:?} "))
                && l.contains(&format!("outcome=\"{outcome}\""))
        })
    };
    assert!(answered("-", "phone-o", "enrolled"), "{logged}");
    assert!(answered("phone-o", "browser-1", "enrolled"), "{logged}");
    assert!(answered("phone-o", "browser-1", "already_enrolled"), "{logged}");
    assert!(answered("phone-r", "browser-2", "scope-denied"), "{logged}");
    for secret in ["AAAAC3Nza", NODE_BROWSER] {
        assert!(!logged.contains(secret), "{secret} reached the log: {logged}");
    }
}
