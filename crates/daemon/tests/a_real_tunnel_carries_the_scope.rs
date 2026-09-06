//! A device dials by token, lands on a real sshd, and gets the scope the file
//! gave it — over a real DERP session, with nothing mocked in between.
//!
//! Until this file there was no run, anywhere, in which the three parts of this
//! transport had ever met. The Go shim's own suite proves what `tailcat.go`
//! does with a `net.Pipe`; the daemon's tunnel tests prove which commands are
//! sent to a helper that is a shell script recording them; `linked.rs` — the
//! only backend that will ever run in production — was type-checked by a
//! `cargo check` and executed by nothing except one minting test. Every one of
//! those is a component agreeing with itself. What was missing is the sentence
//! this file asserts: **the token a runner mints, the node key its
//! `authorized_keys` admits, and the sshd its tunnel forwards to are the same
//! three facts.** A mock of any of them would have agreed with whatever this
//! code believed.
//!
//! The precedent is `a_real_sshd_forces_the_scope.rs`, which stands up a real
//! sshd rather than a mock because "forced" is OpenSSH's promise and not ours.
//! Same argument, one layer out: DERP rendezvous is tailscale's promise, the
//! allowlist is tailcat's, and neither can be settled by a test double.
//!
//! ## What is real here, and the one thing that is not
//!
//! Real: the relay (a `derper` built from the same `tailscale.com` the tunnel
//! pins), the runner's tunnel server and the device's dial (both the linked Go
//! archive, in this process), `authorized_keys` written by `fence::render` and
//! `fence::write`, the allowlist derived by the shipped
//! `allowlist::tunnel_plan`, a real `sshd`, a real forced command, and a real
//! `farcoolerd --stdio` answering the real protocol.
//!
//! Not real: **the port.** `Service::ssh_port()` answers 22 and says in as many
//! words that this is honest rather than a placeholder — nothing in the daemon
//! reads `sshd_config`. So driving this through `allowlist::start_tunnel` would
//! forward the tunnel to `127.0.0.1:22`, which on a developer's machine is that
//! developer's own sshd and on nobody's machine is this test's. This file
//! therefore performs `start_tunnel`'s three steps itself — read the fence,
//! `tunnel_plan`, `serve` — with the scratch sshd's port in place of the
//! constant, and asserts on `tunnel_plan`'s answer so the admission decision is
//! still the shipped one. The substitution is one integer and it is named here
//! so no reader has to discover it.
//!
//! ## Why it is not run by `cargo test --workspace`
//!
//! The whole file is `cfg`'d on `tailcat`, the daemon's mirror of
//! `farcooler-tailcat/linked`, because with no archive linked
//! `farcooler_tailcat::serve` and `dial` refuse every call for every input —
//! every assertion below would pass, or fail, for reasons that have nothing to
//! do with what it is asking. Running it needs three things a default build
//! does not have: the Go archive linked into this test binary, a `derper` on
//! disk, and `TS_DEBUG_USE_DERP_HTTP=1` in the environment at exec.
//! `scripts/tunnel-e2e.sh` supplies all three and is the only supported way to
//! run this; every one of the three is a loud failure when absent and none of
//! them is a skip.
//!
//! **No CI line runs this file.** `ci.yml` type-checks it — `cargo check -p
//! farcooler-daemon --features tailcat --all-targets`, so it cannot rot — and
//! runs nothing, because a job would need a Go toolchain, a built relay and an
//! Apple silicon runner. That is written out at the `cargo check` line itself
//! and in the script's header, including the one question that has to be
//! answered before such a job can be added.

#![cfg(feature = "tailcat")]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use farcooler_client::ssh::{Destination, HostKeyPolicy, Reach, Session, SshError};
use farcooler_daemon::allowlist;
use farcooler_fence::{self as fence, Grant, Placement};
use farcooler_protocol::v1::{ErrorCode, Scope};
use farcooler_tailcat::NodeKeyPair;
use farcooler_transport::{request, Client, ClientError};

mod common;
#[path = "support/derper.rs"]
mod derper;

/// One tunnel server per PROCESS, so one test at a time.
///
/// `farcooler_tailcat::serve` replaces whatever is running — that is what makes
/// it a revoke as well as a start — and `set_derp_map_url` is process-wide for
/// the same reason. Two of these tests running concurrently would take turns
/// tearing down each other's runner, and the symptom would be a dial that times
/// out, which is also what a genuinely broken allowlist looks like.
static SERIAL: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

/// A read-scoped device reaches the daemon through the tunnel, and the scope
/// that arrives is the file's.
///
/// The chain, and every link of it can break independently: `fence::render`
/// wrote a line carrying a node key and a scope; `allowlist::tunnel_plan` read
/// that node key back out and admitted it; tailcat registered the runner with a
/// relay and minted a token naming it; the device dialed that token under its
/// own node key and was recognized; the runner's `OnTCP` forwarded to the sshd
/// on the port it was told; sshd authenticated the device's SSH key against the
/// same file; the forced command in that line ran `farcoolerd --stdio` with the
/// scope in its argv; and the daemon enforced it.
///
/// The host key is PINNED rather than accepted, and that is not decoration: it
/// is what says the bytes ended up at this test's own sshd. A tunnel that
/// terminated anywhere else, or a forward that reached some other listener,
/// presents a different key and this refuses to authenticate at all.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_device_reaching_through_the_tunnel_gets_the_scope_in_the_file() {
    let _serial = SERIAL.lock().await;
    let relay = derper::start().await;
    let phone = Device::mint("phone-7", Scope::Read);
    let runner = Runner::start(&relay, &[&phone]).await;

    let mut session = reach(&runner, &phone).await;

    let mut client = speak(&mut session).await;
    assert_eq!(
        client.server_hello().granted_scope,
        Scope::Read as i32,
        "the tunnel changed the scope the file granted"
    );

    // The scope is the file's, not the connection's. A read device must be
    // refused a host path through the tunnel exactly as it is over a direct
    // connection — a transport that quietly widened what a session may do
    // would pass every assertion above.
    match client.call(request("worktree.list")).await {
        Err(ClientError::Daemon { code, .. }) => {
            assert_eq!(
                code,
                ErrorCode::ScopeDenied as i32,
                "a read device reached host paths through the tunnel"
            );
        }
        other => panic!("a read device wrote through the tunnel: {other:?}"),
    }
}

/// A device whose node key left `authorized_keys` cannot reach sshd at all,
/// while the device still in the file can.
///
/// **The second half is the whole point.** Tailcat ignores an unrecognized
/// client silently, so a revoked device gets a timeout rather than a refusal —
/// and a timeout is also what a dead relay, a broken token, a crashed runner
/// and a test that never started anything produce. Asserting only that the
/// revoked phone times out would be a check that cannot fail: it would go green
/// on a machine where the tunnel never worked at all. So a laptop that is still
/// in the file dials the same token through the same relay in the same test,
/// and has to get through.
///
/// The revoke here is what `enrollment::revoke` does — rewrite the fence, then
/// make the running server match the file — because tailcat can only replace an
/// allowlist, never subtract from one. See `allowlist::start_tunnel`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_revoked_device_does_not_reach_sshd_through_the_tunnel() {
    let _serial = SERIAL.lock().await;
    let relay = derper::start().await;
    let phone = Device::mint("phone-7", Scope::Read);
    let laptop = Device::mint("laptop-2", Scope::Read);
    let mut runner = Runner::start(&relay, &[&phone, &laptop]).await;

    // Both devices reach the runner before anything is revoked, so what the
    // revoke changes is observed rather than assumed.
    reach(&runner, &phone).await;

    runner.revoke(&phone, &[&laptop]).await;

    // **The control runs first, deliberately.** The revoked device's dial can
    // only prove something once the replacement server is up and registered
    // with the relay, and the laptop getting through is what says it is. Run
    // the other way round, a phone that failed because nothing was serving yet
    // would read exactly like a phone that was correctly refused.
    reach(&runner, &laptop).await;

    let began = Instant::now();
    // `Session` carries a live russh handle and no `Debug`, so the success arm
    // is named rather than unwrapped. One attempt and no patience: a revoked
    // device must fail against a runner that has already been shown to be
    // answering.
    let Err(error) = open(&runner, &phone).await else {
        panic!("a revoked device reached sshd through the tunnel");
    };
    derper::note(&format!("the revoked dial failed in {:?}: {error}", began.elapsed()));
    assert!(
        matches!(error, SshError::Tunnel { code: "no_answer" }),
        "a revoked device's dial failed for the wrong reason: {error:?}"
    );
}

/// Dial until the runner answers, or give up loudly.
///
/// **A runner is not reachable the moment `serve` returns, and this is not the
/// test being lenient about it.** `Start` brings up the WireGuard engine and
/// returns; the DERP connection that makes the runner findable is established
/// in a goroutine afterwards, and magicsock picks a home relay on its first
/// netcheck. Measured here: the server logs `home is now derp-900` **3.0
/// seconds** after `serve` returned, and a dial issued before that gets
/// `derp-1 does not know about peer [...], removing route` and then spends its
/// whole ten-second budget without re-adding the route — so it fails even
/// though the runner came up while it was waiting.
///
/// That is a real property of the transport and a phone that dials a runner
/// which has just booted meets it. What this loop must not become is a way for
/// the test to pass by trying until something works: the budget is bounded, a
/// failure that is not `no_answer` is reported at once rather than retried, and
/// every attempt is printed with what it cost.
async fn reach(runner: &Runner, device: &Device) -> Session {
    // The measured 3.0 s plus margin, so the ordinary run pays one dial rather
    // than one failed dial (ten seconds) and then a good one. The loop below
    // is what makes this an optimization instead of a fixed sleep to be flaky
    // about.
    tokio::time::sleep(Duration::from_millis(3_500)).await;

    let began = Instant::now();
    let deadline = began + Duration::from_secs(90);
    let mut attempt = 0;
    loop {
        attempt += 1;
        let tried = Instant::now();
        match open(runner, device).await {
            Ok(session) => {
                derper::note(&format!(
                    "{} reached the runner on attempt {attempt}, {:?} after serving",
                    device.client_id,
                    began.elapsed()
                ));
                return session;
            }
            Err(SshError::Tunnel { code: "no_answer" }) if Instant::now() < deadline => {
                derper::note(&format!(
                    "{} got no answer after {:?}; the runner may not have registered yet",
                    device.client_id,
                    tried.elapsed()
                ));
            }
            Err(other) => panic!(
                "{} could not reach the runner on attempt {attempt} after {:?}: {other}",
                device.client_id,
                began.elapsed()
            ),
        }
    }
}

/// Dial this runner as this device, exactly as a phone would.
///
/// `Session::open` and not a hand-rolled dial: the branch under test is the
/// shipped one, including the port-22-as-a-name convention and the error
/// mapping. A test that called `farcooler_tailcat::dial` itself would prove
/// the Go shim works and say nothing about whether the client reaches it.
async fn open(runner: &Runner, device: &Device) -> Result<Session, SshError> {
    Session::open(&Destination {
        reach: Reach::Tailcat {
            token: runner.token.clone(),
            client_key: device.node.private_key.clone(),
        },
        user: unix_user(),
        private_key: device.ssh_private.clone(),
        passphrase: None,
        host_key: HostKeyPolicy::Pinned(runner.host_fingerprint.clone()),
        // The relay this whole test agrees on, supplied the way a phone
        // supplies it: in the destination, so `Session::open`'s caller sets
        // the rendezvous on the way in rather than the test setting it out of
        // band. `Runner::start` has already set the same URL process-wide
        // because the runner's own `serve` needed it before any device
        // existed — this is the DEVICE half of the same setting, and it is
        // the one a phone actually exercises.
        derp_map: runner.derp_map.clone(),
    })
    .await
}

/// Run the shipped forced command's program over the session and handshake.
///
/// What is asked for is what `crates/cli/src/remote.rs` asks for. sshd
/// overrides it with the line's forced command, which is the property
/// `a_real_sshd_forces_the_scope.rs` is about; here it matters only that
/// nothing in this test is choosing the daemon's argv.
async fn speak(
    session: &mut Session,
) -> Client<farcooler_client::ssh::ChannelReader, std::pin::Pin<Box<dyn tokio::io::AsyncWrite + Send>>>
{
    let streams = session.exec("farcoolerd --stdio").await.expect("exec through the tunnel");
    Client::over(streams.reader, streams.writer, "test-client", "0.0.0")
        .await
        .expect("handshake over a tunnel, a relay and a real sshd")
}

/// One device: an SSH key pair for sshd and a tailcat node key pair for the
/// tunnel. Two identities, and the test is partly about them staying attached
/// to each other.
struct Device {
    client_id: String,
    scope: Scope,
    ssh_private: String,
    ssh_public: String,
    node: NodeKeyPair,
}

impl Device {
    /// Both halves of both identities, from the shipped minting call.
    ///
    /// `farcooler_tailcat::mint_node_key` rather than a constant, because a
    /// constant would be this file's idea of a node key and what is on trial
    /// includes whether Go's encoder and `fence::usable_node_key` agree about
    /// the spelling.
    fn mint(client_id: &str, scope: Scope) -> Self {
        let node = farcooler_tailcat::mint_node_key()
            .expect("a linked build mints a node key for a device");
        let dir = tempfile::tempdir().expect("a scratch directory for a device key");
        let path = dir.path().join("id");
        keygen(&path, client_id);
        Self {
            client_id: client_id.to_string(),
            scope,
            ssh_private: std::fs::read_to_string(&path).expect("the device's private key"),
            ssh_public: std::fs::read_to_string(path.with_extension("pub"))
                .expect("the device's public key"),
            node,
        }
    }

    /// The `authorized_keys` line this device is admitted by, from the shipped
    /// renderer. Never a string in this file: the format is what is on trial.
    fn line(&self) -> String {
        fence::render(
            self.ssh_public.trim(),
            "Test Device",
            &self.client_id,
            self.scope,
            Grant::FarCooler,
            Some(&self.node.public_key),
        )
        .expect("render a Key A line carrying a node key")
    }
}

/// One runner: a real sshd, a real fence, a real tunnel identity, and the
/// tailcat server that this process is holding open on its behalf.
///
/// **Field order is drop order.** `_dir` deletes the directory the tmux reaper
/// reads an install id out of, so it is declared last and dropped last — the
/// exact leak `common::DaemonChild` was written about.
struct Runner {
    _sshd: Sshd,
    /// The token a device dials. Minted by `conn_blob` from the running
    /// server, so it names the relay this runner actually registered with.
    token: String,
    /// The scratch sshd's host key, in the form `Destination::host_key` pins.
    host_fingerprint: String,
    /// The DERP map naming this test's own relay, so a device can be handed it
    /// in its `Destination` the way a phone hands over its own setting.
    derp_map: String,
    ssh_port: u16,
    tailcat_key: PathBuf,
    authorized_keys: PathBuf,
    _tmux: common::TmuxReaper,
    _dir: tempfile::TempDir,
}

impl Runner {
    /// Enroll these devices, put a real sshd in front, and serve a tunnel.
    async fn start(relay: &derper::Derper, devices: &[&Device]) -> Self {
        let sshd_binary = Path::new("/usr/sbin/sshd");
        assert!(
            sshd_binary.exists(),
            "no {} on this machine, so nothing here was tested",
            sshd_binary.display()
        );

        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().to_path_buf();
        let home = root.join("home");
        std::fs::create_dir(&home).expect("the account home");
        let runtime = root.join("rt");
        std::fs::create_dir(&runtime).expect("the runtime directory");
        let bin = root.join("bin");
        std::fs::create_dir(&bin).expect("the bin directory");

        keygen(&root.join("host_ed25519"), "host");
        let host_fingerprint = fingerprint(&root.join("host_ed25519.pub"));

        let authorized_keys = home.join(".ssh").join("authorized_keys");
        write_fence(&authorized_keys, devices);
        scaffold(&authorized_keys, &home, &runtime, &bin, devices);

        let ssh_port = free_port();
        let sshd = start_sshd(sshd_binary, &root, &authorized_keys, ssh_port).await;

        // Every process in this test agrees on one relay, which is what makes
        // it a LOCAL test: the runner registers there and the device is told
        // about it inside the token. Process-wide because it is deployment
        // configuration rather than a property of one connection — see
        // `farcooler_tailcat::set_derp_map_url`.
        farcooler_tailcat::set_derp_map_url(&relay.map_url);

        // The runner's own identity, through the shipped call rather than by
        // letting `serve` create one as a side effect. This is the first
        // execution of `ensure_identity` in any build: it has been
        // type-checked and never run.
        let tailcat_key = runtime.join("tailcat.key");
        farcooler_tailcat::ensure_identity(&tailcat_key).expect("mint this runner's identity");
        assert!(tailcat_key.exists(), "ensure_identity reported success and wrote no key");
        assert_eq!(
            mode_of(&tailcat_key),
            0o600,
            "a runner's tunnel private key was left readable by other accounts"
        );

        let mut runner = Self {
            _sshd: sshd,
            token: String::new(),
            host_fingerprint,
            derp_map: relay.map_url.clone(),
            ssh_port,
            tailcat_key,
            authorized_keys,
            _tmux: common::TmuxReaper::new(&runtime),
            _dir: dir,
        };
        runner.token = runner.serve(devices).await;
        runner
    }

    /// Make the running tunnel match `authorized_keys`, and answer with the
    /// token it is now serving.
    ///
    /// `allowlist::start_tunnel`'s three steps, in its order, with its
    /// decision: read the file with `fence::read`, ask `tunnel_plan` who is
    /// admitted, hand that to `serve`. Only the port differs, and this file's
    /// header says why.
    async fn serve(&self, expected: &[&Device]) -> String {
        let entries = fence::read(&self.authorized_keys, fence::AUTHORIZED_KEYS)
            .expect("the fence sshd authenticates against is readable");

        // The node key survived the file, which is not free: it goes in
        // through `render`'s forced command and comes back out through
        // `entry_from_line`'s `--node-key` flag, past a `usable_node_key`
        // filter, in a line this test then prefixed with `environment=`
        // options of its own.
        for device in expected {
            let entry = entries
                .iter()
                .find(|e| e.client_id == device.client_id)
                .unwrap_or_else(|| panic!("{} is not in the fence", device.client_id));
            assert_eq!(
                entry.node_key, device.node.public_key,
                "the node key {} offered did not survive authorized_keys",
                device.client_id
            );
        }

        let allowed = allowlist::tunnel_plan(&self.tailcat_key, &entries)
            .expect("this runner has an identity and admits somebody");
        let expected_keys: Vec<String> =
            expected.iter().map(|d| d.node.public_key.clone()).collect();
        assert_eq!(
            allowed.keys(),
            expected_keys.as_slice(),
            "the shipped projection admits a different set than the file names"
        );

        let key_path = self.tailcat_key.clone();
        let ssh_port = self.ssh_port;
        let keys = allowed.keys().to_vec();
        let began = Instant::now();
        // On the blocking pool because `serve` is synchronous and can hold
        // Go's package mutex for tens of seconds — a region pick, then up to
        // two `Start` attempts. Blocking a worker here would also park this
        // test's own DERP-map HTTP server, which `serve` is about to fetch
        // from.
        tokio::task::spawn_blocking(move || farcooler_tailcat::serve(&key_path, ssh_port, &keys))
            .await
            .expect("the serving task finished")
            .expect("this runner's tunnel came up against the local relay");
        let token = tokio::task::spawn_blocking(farcooler_tailcat::conn_blob)
            .await
            .expect("the token task finished")
            .expect("a serving runner has a token");
        derper::note(&format!(
            "the runner served in {:?} and minted a {}-byte token",
            began.elapsed(),
            token.len()
        ));
        token
    }

    /// Take a device out of `authorized_keys` and make the running tunnel
    /// match, exactly as `enrollment::revoke` does.
    async fn revoke(&mut self, gone: &Device, remaining: &[&Device]) {
        let before = fence::read(&self.authorized_keys, fence::AUTHORIZED_KEYS)
            .expect("read the fence before revoking");
        assert!(
            before.iter().any(|e| e.client_id == gone.client_id),
            "nothing to revoke: {} is not in the fence",
            gone.client_id
        );

        write_fence(&self.authorized_keys, remaining);
        scaffold(&self.authorized_keys, &self.home(), &self.runtime(), &self.bin(), remaining);
        self.token = self.serve(remaining).await;
    }

    fn home(&self) -> PathBuf {
        self._dir.path().join("home")
    }
    fn runtime(&self) -> PathBuf {
        self._dir.path().join("rt")
    }
    fn bin(&self) -> PathBuf {
        self._dir.path().join("bin")
    }
}

/// Write these devices' lines through the shipped fence writer.
fn write_fence(path: &Path, devices: &[&Device]) {
    let lines: Vec<String> = devices.iter().map(|d| d.line()).collect();
    fence::write(path, fence::AUTHORIZED_KEYS, &lines, &[], Placement::Last)
        .expect("write the fence");
    // Asserted rather than imposed: sshd under `StrictModes` refuses a file
    // anybody else can write, so this is a property of the shipped writer that
    // a real sshd is entitled to check.
    assert_eq!(mode_of(path), 0o600, "the fence writer left the file too open");
}

/// Prefix each of our lines with the environment a forced command needs.
///
/// The same scaffolding, for the same reasons, as
/// `a_real_sshd_forces_the_scope.rs`: the shipped forced command names a
/// program that can only be found on PATH, and the daemon reads
/// `$HOME/.ssh/authorized_keys` with no override — so a forced command that
/// inherited the real HOME would have this test's session reading the
/// developer's own fence.
///
/// **Everything after the `environment=` options is `fence::render`'s output
/// verbatim.** `PermitUserEnvironment` is off by default and no runner turns it
/// on, so these options carry no privilege; they are scaffolding and nothing
/// else.
fn scaffold(path: &Path, home: &Path, runtime: &Path, bin: &Path, devices: &[&Device]) {
    let mut file = std::fs::read_to_string(path).expect("read back the fence");
    for device in devices {
        let line = device.line();
        let prefix = format!(
            "environment=\"HOME={}\",environment=\"FARCOOLER_HOME={}\",environment=\"PATH={}\"",
            home.display(),
            runtime.display(),
            path_for_forced_command(home, bin, &line)
        );
        let scaffolded = file.replacen(&line, &format!("{prefix},{line}"), 1);
        assert_ne!(
            scaffolded, file,
            "the rendered line for {} was not in the file just written",
            device.client_id
        );
        file = scaffolded;
    }
    std::fs::write(path, &file).expect("prefix the test scaffolding");
    assert_eq!(mode_of(path), 0o600, "the scaffolding rewrite widened the mode");
}

/// A real sshd, and the rule for ending it.
struct Sshd {
    /// Read from `PidFile` at startup, while the directory still exists. Drop
    /// must not depend on reading a temporary directory that may already be
    /// gone.
    pid: i32,
}

impl Drop for Sshd {
    fn drop(&mut self) {
        // By pid, never by pattern. This machine runs the developer's own
        // sshd, and a pattern kill here would take their remote access with
        // it. Only the listener is signalled; each connection's `sshd-session`
        // child ends when its tunnel does.
        unsafe { libc::kill(self.pid, libc::SIGTERM) };
    }
}

/// Configure, validate and start a loopback sshd, and wait until it answers.
async fn start_sshd(binary: &Path, root: &Path, authorized_keys: &Path, port: u16) -> Sshd {
    let config = root.join("sshd_config");
    let log = root.join("sshd.log");
    let pid_file = root.join("sshd.pid");
    std::fs::write(
        &config,
        format!(
            "Port {port}\n\
             ListenAddress 127.0.0.1\n\
             HostKey {root}/host_ed25519\n\
             AuthorizedKeysFile {authorized_keys}\n\
             PidFile {pid_file}\n\
             StrictModes no\n\
             UsePAM no\n\
             PermitUserEnvironment yes\n\
             PasswordAuthentication no\n\
             KbdInteractiveAuthentication no\n\
             LogLevel DEBUG1\n",
            root = root.display(),
            authorized_keys = authorized_keys.display(),
            pid_file = pid_file.display(),
        ),
    )
    .expect("write sshd_config");

    // Validated before it is started, so a config this test got wrong reads as
    // sshd's own complaint rather than as a tunnel that goes nowhere.
    let checked = std::process::Command::new(binary)
        .arg("-t")
        .arg("-f")
        .arg(&config)
        .output()
        .expect("run sshd -t");
    assert!(
        checked.status.success(),
        "sshd refused the config: {}",
        String::from_utf8_lossy(&checked.stderr)
    );

    let started = std::process::Command::new(binary)
        .arg("-f")
        .arg(&config)
        .arg("-E")
        .arg(&log)
        .status()
        .expect("run sshd");
    assert!(started.success(), "sshd did not start; see {}", log.display());

    // Both facts, because they are different ones: `sshd -f` forks and returns
    // as soon as the parent forked, so its status says nothing about whether
    // anything is listening.
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if let Some(pid) =
            std::fs::read_to_string(&pid_file).ok().and_then(|t| t.trim().parse::<i32>().ok())
            && std::net::TcpStream::connect(("127.0.0.1", port)).is_ok()
        {
            return Sshd { pid };
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no sshd ever listened on port {port}; see {}", log.display());
}

/// Put the built daemon where the rendered forced command will look for it.
///
/// The name is read out of the RENDERED line rather than written down, so this
/// follows the product wherever the spelling goes. A `~/`-anchored name is
/// resolved against the OVERRIDDEN home: this machine has a live Far Cooler
/// install, and a link placed in the real `~/.local/bin` would clobber it.
fn path_for_forced_command(home: &Path, bin: &Path, line: &str) -> String {
    let program = forced_program(line);
    let target = match program.strip_prefix("~/") {
        Some(rest) => {
            let path = home.join(rest);
            let parent = path.parent().expect("a path-bearing program name has a parent");
            std::fs::create_dir_all(parent).expect("the directory the forced command names");
            path
        }
        None if program.starts_with('/') => {
            panic!("the forced command names the absolute path {program}, which this test will not create")
        }
        None => bin.join(program),
    };
    // Idempotent: `scaffold` runs again after a revoke, over the same home.
    if !target.exists() {
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_farcoolerd"), &target)
            .expect("link the daemon under the name the forced command asks for");
    }
    // The real directories after ours, because sshd runs a forced command
    // through the account's login shell and a shell that cannot find `tty` or
    // `mktemp` writes pages of its own errors down the session.
    format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", bin.display())
}

/// The program a rendered line asks sshd to run.
fn forced_program(line: &str) -> &str {
    let (_, after) = line.split_once("command=\"").expect("a Key A line carries a forced command");
    let (command, _) = after.split_once('"').expect("a forced command is quoted");
    command.split_whitespace().next().expect("a forced command names a program")
}

/// The account sshd will authenticate. A non-root sshd can only ever
/// authenticate the user that started it, which is this test's own.
fn unix_user() -> String {
    std::env::var("USER").expect("a test runs as somebody")
}

/// An ed25519 keypair, from OpenSSH's own tool.
///
/// `ssh-keygen` rather than a Rust keypair, because these keys go into a file
/// OpenSSH parses: a key generated another way and then refused would be
/// indistinguishable from the fence writing a line sshd refuses.
fn keygen(path: &Path, comment: &str) {
    let status = std::process::Command::new("/usr/bin/ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", comment, "-f"])
        .arg(path)
        .status()
        .expect("run ssh-keygen");
    assert!(status.success(), "ssh-keygen did not write {}", path.display());
}

/// The `SHA256:…` a client pins, computed the way the client computes it.
fn fingerprint(public_key: &Path) -> String {
    let text = std::fs::read_to_string(public_key).expect("read a public key");
    ssh_key::PublicKey::from_openssh(text.trim())
        .expect("ssh-keygen wrote a key ssh-key can read")
        .fingerprint(ssh_key::HashAlg::Sha256)
        .to_string()
}

/// A port nothing was listening on a moment ago.
fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind a scratch port");
    listener.local_addr().expect("the scratch port's address").port()
}

fn mode_of(path: &Path) -> u32 {
    std::fs::metadata(path).expect("stat").permissions().mode() & 0o777
}
