//! The browser's passkey login, through this crate's own SSH client and
//! signer, against a real sshd (ov-767 acceptance 1).
//!
//! The in-process russh server the other tests use can't verify a
//! `webauthn-sk` signature, and only sshd's verdict counts anyway: it rebuilds
//! the signed data and the RP ID hash on its own and logs the key type it
//! accepted. So this stands up `/usr/sbin/sshd` on a loopback port, as
//! `crates/daemon/tests/real_sshd` does, with an `authorized_keys` holding the
//! line `passkey::public_line` makes, and signs in with `ssh::connect` and
//! `passkey::PasskeySigner` over a software passkey (`soft_passkey`).
//!
//! What the line looks like once the fence writes it, forced command and all,
//! is ov-765's test (`a_real_sshd_admits_a_passkey.rs`). What this adds is the
//! browser's half: the key line and the signature this crate makes are ones
//! sshd takes, at one touch per connection, and a false one is refused.

mod soft_passkey;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use farcooler_web::error::WebError;
use farcooler_web::passkey::{self, PasskeySigner};
use farcooler_web::ssh;
use russh::keys::{HashAlg, PublicKey};
use soft_passkey::{SoftPasskey, Tamper};

/// A real sshd on a loopback port, admitting one key line, killed by pid.
struct Sshd {
    pid: i32,
    port: u16,
    host_key: String,
    log: PathBuf,
    dir: PathBuf,
}

impl Drop for Sshd {
    fn drop(&mut self) {
        // By pid, never by pattern: this machine runs other sshds.
        let _ = Command::new("/bin/kill").arg(self.pid.to_string()).status();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn user() -> String {
    let out = Command::new("/usr/bin/id").arg("-un").output().expect("id -un");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

impl Sshd {
    fn start(line: &str, name: &str) -> Sshd {
        let binary = Path::new("/usr/sbin/sshd");
        // Fails loudly rather than skipping: an "ok" from a machine where
        // nothing ran would read as a pass.
        assert!(binary.exists(), "no /usr/sbin/sshd here, so nothing was tested");
        let dir = std::env::temp_dir().join(format!("fcw-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let status = Command::new("/usr/bin/ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-C", "host", "-f"])
            .arg(dir.join("host"))
            .status()
            .unwrap();
        assert!(status.success());
        let host_line = std::fs::read_to_string(dir.join("host.pub")).unwrap();
        let host_key = PublicKey::from_openssh(host_line.trim()).unwrap().fingerprint(HashAlg::Sha256).to_string();

        let keys = dir.join("authorized_keys");
        // `restrict` and a forced command, as every Key A line has.
        std::fs::write(&keys, format!("restrict,command=\"echo passkey\" {line}\n")).unwrap();
        let port = free_port();
        let (log, pid_file, config) = (dir.join("sshd.log"), dir.join("sshd.pid"), dir.join("sshd_config"));
        std::fs::write(
            &config,
            format!(
                "Port {port}\nListenAddress 127.0.0.1\nHostKey {host}\nAuthorizedKeysFile {keys}\nPidFile {pid}\n\
                 StrictModes no\nUsePAM no\nPasswordAuthentication no\nKbdInteractiveAuthentication no\nLogLevel DEBUG1\n",
                host = dir.join("host").display(),
                keys = keys.display(),
                pid = pid_file.display(),
            ),
        )
        .unwrap();
        let checked = Command::new(binary).arg("-t").arg("-f").arg(&config).output().unwrap();
        assert!(checked.status.success(), "sshd refused the config: {}", String::from_utf8_lossy(&checked.stderr));
        let started = Command::new(binary).arg("-f").arg(&config).arg("-E").arg(&log).status().unwrap();
        assert!(started.success(), "sshd did not start");

        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let pid = std::fs::read_to_string(&pid_file).ok().and_then(|t| t.trim().parse::<i32>().ok());
            if let Some(pid) = pid {
                if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                    return Sshd { pid, port, host_key, log, dir };
                }
            }
            assert!(Instant::now() < deadline, "sshd never listened; see {}", log.display());
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// The log, once it says `want` (sshd writes it from a child process).
    async fn log_saying(&self, want: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let log = std::fs::read_to_string(&self.log).unwrap_or_default();
            if log.contains(want) || Instant::now() > deadline {
                return log;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn sign_in(&self, passkey: &SoftPasskey, line: &str) -> Result<ssh::Connection, WebError> {
        let stream = tokio::net::TcpStream::connect(("127.0.0.1", self.port)).await.unwrap();
        let mut signer = PasskeySigner(passkey.authenticator());
        ssh::connect(stream, &self.host_key, &user(), line, &mut signer).await
    }
}

/// A fresh passkey's line admits it, sshd logs it as `ECDSA-SK`, and each of
/// eight more connections costs exactly one touch.
#[tokio::test(flavor = "current_thread")]
async fn a_passkey_signs_in_once_per_connection_and_sshd_logs_ecdsa_sk() {
    let passkey = SoftPasskey::new();
    let line = passkey::public_line(&passkey.spki(), "farcooler-web").unwrap();
    assert!(line.starts_with("sk-ecdsa-sha2-nistp256@openssh.com "), "{line}");
    let sshd = Sshd::start(&line, "ok");

    let first = sshd.sign_in(&passkey, &line).await.expect("sshd admits the passkey");
    drop(first);
    let log = sshd.log_saying("ECDSA-SK").await;
    assert!(
        log.lines().any(|l| l.contains("Accepted publickey") && l.contains("ECDSA-SK")),
        "sshd did not log an ECDSA-SK login:\n{log}"
    );
    assert_eq!(passkey.touches.load(Ordering::SeqCst), 1, "the first connection cost more than one touch");

    for reconnect in 1..=8 {
        let connection = sshd.sign_in(&passkey, &line).await.expect("a reconnect");
        drop(connection);
        assert_eq!(passkey.touches.load(Ordering::SeqCst), 1 + reconnect, "reconnect {reconnect}");
    }
}

/// A signature with one bit of `s` flipped passes every check the browser
/// can make, and sshd refuses it: the success above is sshd's verdict, not a
/// line that admits anything.
#[tokio::test(flavor = "current_thread")]
async fn a_false_signature_is_refused_by_sshd() {
    let passkey = SoftPasskey::with(Tamper::FlipS);
    let line = passkey::public_line(&passkey.spki(), "farcooler-web").unwrap();
    let sshd = Sshd::start(&line, "flip");
    match sshd.sign_in(&passkey, &line).await {
        Err(WebError::KeyRejected) => {}
        other => panic!("a false signature was not refused by sshd: {:?}", other.map(|_| ())),
    }
    let log = sshd.log_saying("Failed publickey").await;
    assert!(log.contains("Failed publickey"), "sshd did not log the refusal:\n{log}");
}
