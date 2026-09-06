//! A DERP relay on loopback, so a tunnel can be tested without the internet.
//!
//! DERP is the RENDEZVOUS for every tailcat connection, not a fallback for one
//! that could not go direct: a runner registers with a relay and a device
//! reaches it there. So two peers in one process, on one machine, still cannot
//! find each other with no relay running. That is why an end-to-end tunnel test
//! needs this and an end-to-end sshd test needs nothing but sshd.
//!
//! ## The trap this file exists to not fall into
//!
//! `InsecureForTests` in the DERP map is NOT enough. In
//! `tailscale.com/derp/derphttp` it only sets `InsecureSkipVerify` on a TLS
//! handshake that still happens, so a client keeps speaking TLS to a
//! plain-HTTP derper and fails with `tls: first record does not look like a
//! TLS handshake`, retrying behind a backoff until the caller's deadline. The
//! only switch that selects plain HTTP for a region-addressed DERP node is the
//! environment knob `TS_DEBUG_USE_DERP_HTTP=1`
//! (`derphttp_client.go:251,264`), and it must be set for BOTH peers.
//!
//! **And it must be in the process environment before this process starts, not
//! set from Rust once it is running.** Go's `os.Getenv` reads the environment
//! the Go runtime copied at startup, and in a `c-archive` build that runtime is
//! initialized by a load-time constructor — before `main`, and long before any
//! test body runs. A `std::env::set_var` here reaches libc's `environ` and
//! never reaches Go. Measured, not assumed: with the knob set only from Rust
//! the dial spends its whole ten-second budget and comes back `no_answer`; with
//! it in the environment at exec the same code carries an SSH session in under
//! a second. [`require_the_plain_http_knob`] is what turns that from a mystery
//! into a sentence, and `scripts/tunnel-e2e.sh` is what sets it.
//!
//! Both peers are one process here — the test is the runner AND the device —
//! so one environment covers both. Two processes would each need it.
//!
//! ## What is deliberately not `derper -dev`
//!
//! The spike's finding 6 recorded `derper -dev`, which is three lines of
//! upstream's `main`: `-a` becomes `:3340`, `tsweb.DevMode` goes on, and the
//! config becomes an ephemeral key. The middle one is a web-UI flag and the
//! outer two are what matter, so this passes `-a 127.0.0.1:<free port>` and a
//! `-c <scratch path>` instead. The property finding 6 was actually about —
//! **plain HTTP, no certificate** — is preserved and is a consequence of the
//! address rather than of `-dev`: upstream serves TLS only when the port is 443
//! or `-certmode` is `manual` (`cmd/derper/derper.go`, `serveTLS`).
//!
//! The reason not to take `-dev` as written is that `:3340` and STUN's `:3478`
//! are FIXED. A live Far Cooler install runs on the machine this is developed
//! on, two of these tests run concurrently under `cargo test`, and a fixed port
//! turns both of those into a flake that looks like a broken tunnel.

use std::io::Write as _;
use std::net::{TcpListener, UdpSocket};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

/// How long to wait for the relay to answer its own probe before giving up.
///
/// Generous, and it can afford to be: it is polled, so a healthy derper costs
/// the first poll interval and nothing else. What this bounds is the machine
/// being busy, not the relay being slow.
const READY: Duration = Duration::from_secs(20);

/// A running relay, its DERP map, and the rule for ending both.
///
/// Dropping this kills the relay **by the pid of the child this struct
/// started** and nothing else. Never by name: a live Far Cooler app runs on
/// the machine this was written on, other lanes run their own scratch daemons,
/// and two sessions have already killed each other's processes here by
/// pattern.
pub struct Derper {
    derper: Child,
    /// Where the DERP map JSON this relay is described by can be fetched.
    ///
    /// Handed to `farcooler_tailcat::set_derp_map_url`, which is process-wide
    /// and therefore reaches the runner half and the device half at once.
    pub map_url: String,
    /// Kept so the relay's scratch key file outlives the relay.
    _dir: tempfile::TempDir,
}

impl Drop for Derper {
    fn drop(&mut self) {
        // `Child::kill` signals this child's pid. Then `wait`, so the relay is
        // reaped rather than left as a zombie for the length of the test run.
        let _ = self.derper.kill();
        let _ = self.derper.wait();
    }
}

/// Refuse to run at all unless the plain-HTTP knob is in the environment.
///
/// Called first by every test in this file's consumer, before a relay is
/// started and long before anything dials. Without it the symptom is a tunnel
/// that comes up, registers, and then times out after ten seconds with
/// `no_answer` — which reads exactly like a revoked device, sends whoever
/// hits it looking at the allowlist, and is a lie the test would be telling
/// about its own subject.
///
/// A panic and not a skip. See this file's consumer for why nothing here is
/// ever allowed to be quietly absent.
pub fn require_the_plain_http_knob() {
    let set = std::env::var("TS_DEBUG_USE_DERP_HTTP").is_ok_and(|v| v == "1");
    assert!(
        set,
        "TS_DEBUG_USE_DERP_HTTP=1 is not in this process's environment, so tailcat \
         would speak TLS to a plain-HTTP relay and every dial here would time out \
         after ten seconds saying the runner did not answer.\n\
         \n\
         It cannot be set from inside the test: Go copies the environment at \
         runtime startup, which in a c-archive build happens before main.\n\
         \n\
         Run this file through ./scripts/tunnel-e2e.sh, which sets it."
    );
}

/// The derper binary this harness runs, or a panic naming how to get one.
///
/// `FARCOOLER_DERPER` first so a machine that already has one built somewhere
/// else can say so; otherwise the path `scripts/build-derper.sh` writes.
fn binary() -> PathBuf {
    if let Ok(named) = std::env::var("FARCOOLER_DERPER") {
        let path = PathBuf::from(named);
        assert!(
            path.exists(),
            "FARCOOLER_DERPER names {}, which does not exist",
            path.display()
        );
        return path;
    }
    // `crates/daemon` → the workspace root, which is where `dist/` lives.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    let path = root.join("dist").join("derper").join("derper");
    assert!(
        path.exists(),
        "no derper at {}, so nothing in this file was tested.\n\
         \n\
           ./scripts/build-derper.sh\n\
         \n\
         It builds one from the same tailscale.com the tunnel already pins in \
         crates/tailcat/go/go.mod, and needs Go only for that build — never for \
         `cargo build` or `cargo test --workspace`.",
        path.display()
    );
    path
}

/// Start a relay and serve a DERP map that points at it.
///
/// Two listeners of ours and one of the relay's:
///
/// - the relay's DERP endpoint, plain HTTP, on a free TCP port;
/// - the relay's STUN endpoint, on a free UDP port, which is what lets
///   `PickBestRegion` measure this region and therefore what lets a runner PIN
///   it. A map with nothing measurable leaves the runner unpinned, which still
///   serves but costs a netcheck on every start;
/// - a one-file HTTP server of ours, serving the map JSON that names the two
///   ports above.
///
/// Ports are taken by binding and releasing rather than by picking a number, so
/// two of these running at once cannot choose the same one. The race that
/// remains is the small one — something else on the machine takes the port
/// between the release and the relay's bind — and it surfaces as this function
/// failing to see a healthy relay, which is the right way round.
pub async fn start() -> Derper {
    require_the_plain_http_knob();

    let binary = binary();
    let dir = tempfile::tempdir().expect("a scratch directory for the relay's key");
    let derp_port = free_tcp_port();
    let stun_port = free_udp_port();

    let log = dir.path().join("derper.log");
    let derper = Command::new(&binary)
        .arg("-a")
        .arg(format!("127.0.0.1:{derp_port}"))
        // The plain-HTTP listener upstream puts on port 80 by default, which a
        // test may not have and does not want: the DERP endpoint above is
        // already plain HTTP because the address is not 443.
        .args(["-http-port", "-1"])
        .arg("-stun-port")
        .arg(stun_port.to_string())
        // Non-root and not `-dev`, so upstream insists on being told where its
        // key lives. It writes a fresh one at this path on first start, which
        // for a scratch directory means every run gets a new relay identity.
        .arg("-c")
        .arg(dir.path().join("derper.key"))
        .stdout(Stdio::from(std::fs::File::create(&log).expect("the relay's log")))
        .stderr(Stdio::from(std::fs::File::create(&log).expect("the relay's log")))
        .spawn()
        .unwrap_or_else(|e| panic!("could not start {}: {e}", binary.display()));

    let map = map_json(derp_port, stun_port);
    let map_url = serve_map(map).await;
    let relay = Derper { derper, map_url, _dir: dir };

    await_relay(derp_port, &log).await;
    relay
}

/// The DERP map that points two peers at one loopback relay.
///
/// Region 900 rather than a real number, so nothing here can be confused with
/// a region tailcat.dev serves. `InsecureForTests` is in the map because
/// upstream's client refuses to consider a non-TLS node without it — it is
/// necessary and, on its own, not sufficient. See this module's header.
fn map_json(derp_port: u16, stun_port: u16) -> String {
    serde_json::json!({
        "Regions": {
            "900": {
                "RegionID": 900,
                "RegionCode": "local",
                "RegionName": "Local derper",
                "Nodes": [{
                    "Name": "900a",
                    "RegionID": 900,
                    "HostName": "localhost",
                    "IPv4": "127.0.0.1",
                    // Loopback IPv6 is deliberately absent rather than "::1".
                    // A node offering both makes the client's choice depend on
                    // the machine's IPv6 configuration, which is one more way
                    // for this to behave differently on a laptop and a CI
                    // runner for a reason that has nothing to do with tunnels.
                    "IPv6": "",
                    "STUNPort": stun_port,
                    "DERPPort": derp_port,
                    "InsecureForTests": true,
                }],
            },
        },
    })
    .to_string()
}

/// Serve one JSON document over HTTP, and answer with its URL.
///
/// Hand-rolled rather than pulled from a crate: it answers one document to any
/// request, which is fourteen lines, and a test-only HTTP dependency in the
/// daemon's tree would be carried by every build of it.
///
/// The listener is leaked into a spawned task on purpose. It lives as long as
/// the test process, which is shorter than a `Derper` needs it for anyway —
/// upstream caches a fetched map process-wide by URL for an hour, so the map
/// may be read once at the start and never again, and may equally be read
/// again during a recovery an hour later.
async fn serve_map(body: String) -> String {
    let listener =
        tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind the map server");
    let port = listener.local_addr().expect("the map server's address").port();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else { return };
            let body = body.clone();
            tokio::spawn(async move {
                // The request is read and thrown away: one document, any path.
                // Bounded so a client that sends nothing cannot park a task
                // here forever.
                let mut request = [0u8; 2048];
                let _ = tokio::time::timeout(
                    Duration::from_secs(5),
                    stream.read(&mut request),
                )
                .await;
                let response = format!(
                    "HTTP/1.1 200 OK\r\n\
                     Content-Type: application/json\r\n\
                     Content-Length: {}\r\n\
                     Connection: close\r\n\
                     \r\n\
                     {body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    format!("http://127.0.0.1:{port}/derpmap.json")
}

/// Wait until the relay answers its own health probe.
///
/// `/derp/probe` and not a bare TCP connect, because the two say different
/// things: a connect succeeds the instant the listener is bound, and a relay
/// that is bound but not yet serving would send the first dial into a retry
/// backoff whose symptom is a timeout ten seconds later in a test about
/// something else.
async fn await_relay(port: u16, log: &std::path::Path) {
    let deadline = Instant::now() + READY;
    let mut last = String::new();
    while Instant::now() < deadline {
        match probe(port).await {
            Ok(()) => return,
            Err(error) => last = error,
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let log = std::fs::read_to_string(log).unwrap_or_default();
    panic!(
        "no derper answered on 127.0.0.1:{port} within {READY:?} (last: {last}).\n\
         The relay's own log:\n{log}"
    );
}

/// One HTTP GET, written by hand for the same reason the map server is.
async fn probe(port: u16) -> Result<(), String> {
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .map_err(|e| e.to_string())?;
    stream
        .write_all(b"GET /derp/probe HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .map_err(|e| e.to_string())?;
    let mut answer = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut answer))
        .await
        .map_err(|_| "the relay accepted and never answered".to_string())?
        .map_err(|e| e.to_string())?;
    let head = String::from_utf8_lossy(&answer);
    if head.starts_with("HTTP/1.1 200") {
        Ok(())
    } else {
        Err(format!("the relay answered {:?}", head.lines().next().unwrap_or_default()))
    }
}

/// A TCP port nothing was listening on a moment ago.
fn free_tcp_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a scratch TCP port");
    listener.local_addr().expect("the scratch port's address").port()
}

/// A UDP port nothing was listening on a moment ago — STUN is UDP, and a free
/// TCP number says nothing about the UDP one.
fn free_udp_port() -> u16 {
    let socket = UdpSocket::bind("127.0.0.1:0").expect("bind a scratch UDP port");
    socket.local_addr().expect("the scratch port's address").port()
}

/// Print a line to the test's own stderr, so a run that hangs says where.
///
/// `cargo test` swallows stdout on a passing test and shows it on a failing
/// one, which is the wrong way round for a test whose interesting failure mode
/// is taking ten seconds and then reporting the wrong cause. Stderr is
/// unbuffered here for the same reason.
pub fn note(what: &str) {
    let mut stderr = std::io::stderr();
    let _ = writeln!(stderr, "    [tunnel] {what}");
    let _ = stderr.flush();
}
