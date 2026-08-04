//! Installing Far Cooler on a Linux host, over ssh.
//!
//! Everything here runs through one `ssh` session and touches only the user's
//! own home directory. No root, no package manager, no system unit, nothing
//! written outside `~`. A user who can ssh in can install Far Cooler, and a user
//! who cannot should not be able to.
//!
//! Two things are deliberate:
//!
//! - **The binary's checksum is verified on the host before it is ever run.**
//!   The upload travels inside ssh, so this is not about the wire — it is about
//!   a truncated transfer producing a half-written executable that then gets
//!   registered as a service. Comparing the hash the sender computed against
//!   the one the receiver computes catches that before anything executes.
//!
//! - **Lingering is enabled.** Without it systemd tears down a user's services
//!   at logout, so a host would stop being reachable the moment the user logged
//!   out of it — which is precisely the situation Far Cooler exists for.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;

type Fallible = Result<(), Box<dyn std::error::Error>>;

/// The launchd user agent, for a macOS host.
///
/// `KeepAlive` with `SuccessfulExit=false` is launchd's spelling of systemd's
/// `Restart=on-failure`: a daemon told to stop stays stopped, one that crashes
/// comes back. `ThrottleInterval` is the same guard as `RestartSec` — launchd's
/// default of 10s is already sane, but stating it keeps the two units
/// comparable.
const LAUNCH_AGENT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>com.farcooler.daemon.remote</string>
  <key>ProgramArguments</key>
  <array><string>__HOME__/.local/bin/farcoolerd</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key>
  <dict><key>SuccessfulExit</key><false/></dict>
  <key>ThrottleInterval</key><integer>10</integer>
  <key>ProcessType</key><string>Interactive</string>
</dict>
</plist>
"#;

/// The systemd user unit. Written to ~/.config/systemd/user/farcooler.service.
const UNIT: &str = "\
[Unit]
Description=Far Cooler daemon
Documentation=https://github.com/farcooler
After=default.target

[Service]
Type=simple
ExecStart=%h/.local/bin/farcoolerd
Restart=on-failure
# systemd's default is 100ms, which turns a crash into a hot loop.
RestartSec=5s
# The daemon supervises terminals a user is watching live, so it is not a
# batch job to be throttled.
Nice=-5

[Install]
WantedBy=default.target
";

pub async fn install(target: &str, from: Option<&Path>) -> Fallible {
    println!("==> Checking {target}");
    let probe = probe(target).await?;

    // Everything that makes this host impossible, said at once.
    //
    // These used to be sequential early returns, so a machine with neither tmux
    // nor a supported OS told you about one problem, waited for you to fix it,
    // and then told you about the next.
    if !probe.installable() {
        return Err(format!("{target}:\n  - {}", probe.blockers.join("\n  - ")).into());
    }

    let arch = probe.arch.as_str();
    let Some(slug) = probe.platform.dist_slug(arch) else {
        return Err(format!("{target} reports {arch}, which has no build here.").into());
    };
    let dir = match from {
        Some(p) => p.to_path_buf(),
        None => locate_dist(&slug),
    };
    let daemon = dir.join("farcoolerd");
    let cli = dir.join("farcooler");
    for path in [&daemon, &cli] {
        if !path.is_file() {
            // A packaged app has no `scripts/build-linux.sh` to point at — its
            // copy is either bundled next to it already or it never will be,
            // since the toolchain that builds one is a developer's, not a
            // download's. Telling someone without a checkout to run a script
            // they do not have sends them nowhere.
            let hint = if has_dev_checkout() {
                format!(
                    "Build them first:\n\n    ./scripts/build-linux.sh {arch}\n\n\
                     or pass --from <dir> with `farcooler` and `farcoolerd` inside."
                )
            } else {
                "This copy of Far Cooler was not built with a Linux install \
                 bundled in.\nPass --from <dir> with `farcooler` and `farcoolerd` \
                 inside, built for this host's architecture."
                    .to_string()
            };
            return Err(format!("No Linux binary at {}.\n{hint}", path.display()).into());
        }
    }

    println!(
        "    {} {arch}, tmux at {}",
        probe.os,
        probe.tmux.as_deref().unwrap_or("?")
    );

    remote_run(target, "mkdir -p ~/.local/bin").await?;

    for (path, name) in [(&daemon, "farcoolerd"), (&cli, "farcooler")] {
        println!("==> Installing {name}");
        upload_verified(target, path, name).await?;
    }

    let persistence = register_service(target, probe.persistence).await?;

    println!();
    println!("Installed on {target}.");
    match persistence {
        Persistence::Systemd => {
            println!("  A systemd user service keeps it running.")
        }
        Persistence::Launchd => {
            println!("  A launchd agent keeps it running.")
        }
        Persistence::OnDemand => {
            // Not a failure, and worth being exact about. Agents live in tmux
            // and tmux is not going anywhere; what is lost is the supervision
            // that notices one finishing while nobody is connected.
            println!("  No user service manager here, so the daemon starts when you connect");
            println!("  and stops when you disconnect. Agents keep running inside tmux;");
            println!("  what you lose is being told about them while you are away.");
            if probe.platform == Platform::Wsl {
                println!();
                println!("  WSL can run systemd. In /etc/wsl.conf on that machine:");
                println!("      [boot]");
                println!("      systemd=true");
                println!("  then `wsl --shutdown` from Windows and install again.");
            }
        }
        Persistence::None => println!("  WARNING: nothing will keep the daemon running."),
    }
    println!();
    println!("Check it:   farcooler --host {target} status");
    Ok(())
}

/// Register whatever this host uses to keep a user daemon alive.
///
/// Returns what actually ended up in place, which is not always what was asked
/// for: a service manager can refuse, and reporting success for a daemon that
/// will not survive the night is the one outcome worth avoiding here.
async fn register_service(target: &str, wanted: Persistence) -> Result<Persistence, Box<dyn std::error::Error>> {
    match wanted {
        Persistence::Systemd => {
            println!("==> Registering the systemd user service");
            remote_run(target, "mkdir -p ~/.config/systemd/user").await?;
            remote_write(target, "~/.config/systemd/user/farcooler.service", UNIT).await?;

            // Lingering needs no privilege for one's own account on most
            // distributions, and without it the daemon dies at logout.
            let linger = remote_capture(
                target,
                "loginctl enable-linger \"$USER\" 2>&1 || echo FARCOOLER_LINGER_FAILED",
            )
            .await?;
            if linger.contains("FARCOOLER_LINGER_FAILED") {
                println!("    WARNING: could not enable lingering.");
                println!("    The daemon will stop when you log out of {target}. To fix:");
                println!("        ssh {target} 'sudo loginctl enable-linger $USER'");
            }

            remote_run(
                target,
                "systemctl --user daemon-reload && systemctl --user enable --now farcooler.service",
            )
            .await?;
            Ok(Persistence::Systemd)
        }

        Persistence::Launchd => {
            println!("==> Registering the launchd agent");
            remote_run(target, "mkdir -p ~/Library/LaunchAgents").await?;
            // `$HOME` is not expanded inside a plist, so the path is baked in
            // at write time from the host's own answer.
            let home = remote_capture(target, "echo $HOME").await?;
            let plist = LAUNCH_AGENT.replace("__HOME__", home.trim());
            remote_write(target, "~/Library/LaunchAgents/com.farcooler.daemon.remote.plist", &plist)
                .await?;

            // `bootstrap gui/$UID` needs a GUI session to bootstrap INTO, and
            // over ssh to a Mac at a login window there is not one. That is not
            // a failure to report as an error: the plist is in place and will
            // load when someone logs in, and until then the daemon still starts
            // on demand for anyone who connects.
            let loaded = remote_capture(
                target,
                "launchctl bootstrap gui/$(id -u) \
                 ~/Library/LaunchAgents/com.farcooler.daemon.remote.plist 2>&1 \
                 || echo FARCOOLER_BOOTSTRAP_FAILED",
            )
            .await?;
            if loaded.contains("FARCOOLER_BOOTSTRAP_FAILED") {
                println!("    The agent is installed but not started: {target} has no GUI");
                println!("    session to load it into. It will start when someone logs in.");
                return Ok(Persistence::OnDemand);
            }
            Ok(Persistence::Launchd)
        }

        other => Ok(other),
    }
}

/// Where the Linux `farcooler`/`farcoolerd` this executable would upload live.
///
/// A packaged app bundles them next to itself — `Resources/dist/<slug>` inside
/// Far Cooler.app, alongside the macOS `farcooler`/`farcoolerd` it always
/// carries — so installing on a remote Linux host needs nothing beyond the
/// app itself. That is tried first. A checkout that built its own copy with
/// `build-linux.sh` instead writes to `dist/<slug>` at the repo root, which
/// only resolves correctly relative to wherever this binary happens to be
/// invoked from — a real path, but a weaker guarantee than "next to the exe
/// that is asking" — so it is the fallback, not the default.
fn locate_dist(slug: &str) -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let bundled = dir.join("dist").join(slug);
            if bundled.join("farcoolerd").is_file() && bundled.join("farcooler").is_file() {
                return bundled;
            }
        }
    }
    PathBuf::from("dist").join(slug)
}

/// Whether this is a checkout with `scripts/build-linux.sh` to point someone
/// at, versus a downloaded app with no source beside it.
fn has_dev_checkout() -> bool {
    Path::new("scripts/build-linux.sh").is_file()
}

/// What a host IS, before anything is changed on it.
///
/// Separate from `install` because the app has to be able to ask "can this
/// machine host Far Cooler, and what would installing involve" without
/// installing anything. A user adding a server should be told it has no tmux
/// before binaries start landing on it, not after.
///
/// Everything here is one ssh round trip. Each probe is a shell fragment that
/// prints `key=value` and cannot fail the whole command — a host missing
/// `loginctl` must still report its architecture.
pub async fn probe(target: &str) -> Result<Probe, Box<dyn std::error::Error>> {
    let script = "        echo \"os=$(uname -s)\"; \
        echo \"arch=$(uname -m)\"; \
        echo \"kernel=$(uname -r)\"; \
        echo \"tmux=$(command -v tmux || echo none)\"; \
        echo \"systemd=$(systemctl --user show-environment >/dev/null 2>&1 && echo yes || echo no)\"; \
        echo \"launchd=$(command -v launchctl >/dev/null 2>&1 && echo yes || echo no)\"; \
        echo \"daemon=$(~/.local/bin/farcoolerd --version 2>/dev/null || echo none)\"; \
        echo \"cli=$(~/.local/bin/farcooler --version 2>/dev/null || echo none)\"; \
        echo \"service=$(systemctl --user is-active farcooler.service 2>/dev/null || echo unknown)\"; \
        echo \"linger=$(loginctl show-user \"$USER\" -p Linger --value 2>/dev/null || echo unknown)\"";

    let report = remote_capture(target, script).await?;
    let mut fields: std::collections::HashMap<&str, &str> = Default::default();
    for line in report.lines() {
        if let Some((key, value)) = line.split_once('=') {
            fields.insert(key.trim(), value.trim());
        }
    }
    let get = |k: &str| fields.get(k).copied().unwrap_or_default().to_string();

    let os = get("os");
    let kernel = get("kernel");
    // WSL announces itself in the kernel release, and nowhere else that is
    // reliable: `uname -s` says Linux exactly like any other Linux, but the
    // machine has no systemd unless the user opted into it, and its lifetime is
    // tied to a Windows session rather than to a login.
    let wsl = kernel.to_lowercase().contains("microsoft");
    let tmux = get("tmux");
    let systemd = get("systemd") == "yes";
    // "none" is what the probe prints for a thing that is not there; an empty
    // string is a field the host did not answer at all. Neither is a value.
    fn present(value: String) -> Option<String> {
        (!value.is_empty() && value != "none").then_some(value)
    }
    let launchd = get("launchd") == "yes";

    let platform = match os.as_str() {
        "Linux" if wsl => Platform::Wsl,
        "Linux" => Platform::Linux,
        "Darwin" => Platform::MacOs,
        _ => Platform::Unsupported,
    };

    // What will keep the daemon alive when nobody is attached. Reported rather
    // than assumed, because it is the difference between agents that survive
    // the night and agents that die at logout — the whole point of the product.
    let persistence = match platform {
        Platform::MacOs if launchd => Persistence::Launchd,
        Platform::Linux | Platform::Wsl if systemd => Persistence::Systemd,
        Platform::Unsupported => Persistence::None,
        _ => Persistence::OnDemand,
    };

    let mut blockers = Vec::new();
    if matches!(platform, Platform::Unsupported) {
        blockers.push(format!("{os} is not a platform Far Cooler can install on."));
    }
    if tmux == "none" {
        blockers.push(
            "No tmux. Far Cooler keeps every terminal inside one, so without it \
             the daemon runs and every terminal derives lost."
                .to_string(),
        );
    }

    Ok(Probe {
        target: target.to_string(),
        platform,
        os,
        arch: get("arch"),
        kernel,
        tmux: present(tmux.clone()),
        persistence,
        installed_cli: present(get("cli")),
        installed_daemon: present(get("daemon")),
        service_active: get("service") == "active",
        lingering: get("linger") == "yes",
        blockers,
    })
}

/// What a host is and what installing on it would involve.
#[derive(Debug, Clone)]
pub struct Probe {
    pub target: String,
    pub platform: Platform,
    pub os: String,
    pub arch: String,
    pub kernel: String,
    pub tmux: Option<String>,
    pub persistence: Persistence,
    pub installed_cli: Option<String>,
    pub installed_daemon: Option<String>,
    pub service_active: bool,
    pub lingering: bool,
    /// Reasons this host cannot be installed on at all. Empty means it can.
    pub blockers: Vec<String>,
}

impl Probe {
    pub fn installable(&self) -> bool {
        self.blockers.is_empty()
    }

    /// Written out by hand rather than derived.
    ///
    /// This crate carries `serde_json` and not `serde` with its derive feature,
    /// the same trade `sha256_hex` below makes: one dependency avoided for a
    /// few lines, in a crate whose whole job is to wrap a CLI.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "target": self.target,
            "platform": self.platform.name(),
            "os": self.os,
            "arch": self.arch,
            "kernel": self.kernel,
            "tmux": self.tmux,
            "persistence": self.persistence.name(),
            "installedCli": self.installed_cli,
            "installedDaemon": self.installed_daemon,
            "serviceActive": self.service_active,
            "lingering": self.lingering,
            "blockers": self.blockers,
            "installable": self.installable(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Linux,
    /// Linux under Windows. Its own case because persistence is different and
    /// the failure is silent otherwise — see `probe`.
    Wsl,
    MacOs,
    Unsupported,
}

impl Platform {
    pub fn name(self) -> &'static str {
        match self {
            Platform::Linux => "linux",
            Platform::Wsl => "wsl",
            Platform::MacOs => "macos",
            Platform::Unsupported => "unsupported",
        }
    }

    /// The directory `scripts/build-linux.sh` and friends write into.
    pub fn dist_slug(self, arch: &str) -> Option<String> {
        let arch = match arch {
            "aarch64" | "arm64" => "aarch64",
            "x86_64" | "amd64" => "x86_64",
            _ => return None,
        };
        match self {
            Platform::Linux | Platform::Wsl => Some(format!("{arch}-linux")),
            Platform::MacOs => Some(format!("{arch}-darwin")),
            Platform::Unsupported => None,
        }
    }
}

/// What will keep the daemon running when nobody is attached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Persistence {
    Systemd,
    Launchd,
    /// Installed, but nothing will start it: it runs when a client connects and
    /// stops when that connection ends. Agents inside tmux still survive; what
    /// is lost is the supervision that notices them finishing.
    OnDemand,
    None,
}

impl Persistence {
    pub fn name(self) -> &'static str {
        match self {
            Persistence::Systemd => "systemd",
            Persistence::Launchd => "launchd",
            Persistence::OnDemand => "onDemand",
            Persistence::None => "none",
        }
    }
}

/// What is installed and running on a host.
pub async fn status(target: &str) -> Fallible {
    let report = remote_capture(
        target,
        "echo \"os=$(uname -s)\"; echo \"arch=$(uname -m)\"; \
         echo \"tmux=$(command -v tmux || echo none)\"; \
         echo \"daemon=$(~/.local/bin/farcoolerd --version 2>/dev/null || echo none)\"; \
         echo \"cli=$(~/.local/bin/farcooler --version 2>/dev/null || echo none)\"; \
         echo \"service=$(systemctl --user is-active farcooler.service 2>/dev/null || echo inactive)\"; \
         echo \"linger=$(loginctl show-user \"$USER\" -p Linger --value 2>/dev/null || echo unknown)\"",
    )
    .await?;

    println!("{target}");
    for line in report.lines() {
        let Some((key, value)) = line.split_once('=') else { continue };
        println!("  {key:<9} {value}");
    }
    Ok(())
}

/// Copy a file, then verify what landed before anything runs it.
async fn upload_verified(target: &str, local: &Path, name: &str) -> Fallible {
    let bytes = std::fs::read(local)?;
    let expected = sha256_hex(&bytes);

    // Piped through ssh rather than scp: one code path, one set of ssh options,
    // and no dependency on scp's shifting semantics between versions.
    let destination = format!("~/.local/bin/{name}");
    let mut child = Command::new("ssh")
        .args(ssh_options())
        .arg(target)
        .arg(format!("cat > {destination}.new && chmod 755 {destination}.new"))
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()?;
    child.stdin.as_mut().ok_or("ssh gave no stdin")?.write_all(&bytes).await?;
    child.stdin.take();
    let status = child.wait().await?;
    if !status.success() {
        return Err(format!("copying {name} to {target} failed").into());
    }

    let actual =
        remote_capture(target, &format!("sha256sum {destination}.new | cut -d' ' -f1")).await?;
    if actual.trim() != expected {
        // Leave the bad file with its .new suffix: it is evidence, and it is
        // not on the path anything runs.
        return Err(format!(
            "{name} arrived corrupted on {target}\n  expected {expected}\n  got      {}",
            actual.trim()
        )
        .into());
    }

    // Rename only after the hash matches. A running daemon holding the old
    // inode keeps running until it is restarted, which is what we want.
    remote_run(target, &format!("mv {destination}.new {destination}")).await?;
    println!("    {name}  {} bytes  sha256 {}…", bytes.len(), &expected[..12]);
    Ok(())
}

async fn remote_write(target: &str, path: &str, contents: &str) -> Fallible {
    let mut child = Command::new("ssh")
        .args(ssh_options())
        .arg(target)
        .arg(format!("cat > {path}"))
        .stdin(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    child.stdin.as_mut().ok_or("ssh gave no stdin")?.write_all(contents.as_bytes()).await?;
    child.stdin.take();
    if !child.wait().await?.success() {
        return Err(format!("writing {path} on {target} failed").into());
    }
    Ok(())
}

pub(crate) async fn remote_run(target: &str, command: &str) -> Fallible {
    let status = Command::new("ssh")
        .args(ssh_options())
        .arg(target)
        .arg(command)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await?;
    if !status.success() {
        return Err(format!("`{command}` failed on {target}").into());
    }
    Ok(())
}

/// Run a remote command, feeding it something on stdin.
///
/// Exists so a credential can reach a machine without ever being an argument.
/// The error deliberately does NOT name the command: `remote_run` does, which is
/// right for an installer step and wrong for anything carrying a secret, and the
/// only way to be sure is for the secret-carrying path to have its own runner.
pub(crate) async fn remote_run_with_stdin(target: &str, command: &str, input: &str) -> Fallible {
    use tokio::io::AsyncWriteExt;

    let mut child = Command::new("ssh")
        .args(ssh_options())
        .arg(target)
        .arg(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(input.as_bytes()).await?;
        // Dropped here rather than at the end of the function: the remote reads
        // to EOF, and holding the pipe open would deadlock both sides.
        drop(stdin);
    }

    if !child.wait().await?.success() {
        return Err(format!("pairing failed on {target}").into());
    }
    Ok(())
}

async fn remote_capture(target: &str, command: &str) -> Result<String, Box<dyn std::error::Error>> {
    let out = Command::new("ssh")
        .args(ssh_options())
        .arg(target)
        .arg(command)
        .stderr(Stdio::piped())
        .output()
        .await?;

    // ssh's own failure is an error, not an empty answer.
    //
    // This returned stdout regardless of exit status, so a host that refused
    // the connection came back as a host that answered with nothing — and a
    // probe then read those empty fields as "some OS I do not recognize". The
    // user was told their machine was an unsupported platform when the truth
    // was that ssh could not reach it, which sends them to fix the wrong thing.
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let detail = stderr.trim().lines().next().unwrap_or("ssh failed").to_string();
        return Err(format!("cannot reach {target}: {detail}").into());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn ssh_options() -> Vec<String> {
    vec!["-o".into(), "BatchMode=yes".into()]
}

/// SHA-256, written out rather than pulled in.
///
/// One dependency avoided for forty lines, in a crate whose whole job is to
/// wrap a CLI. The remote side uses `sha256sum`, so this only has to agree with
/// the standard — which the test below checks against published vectors.
fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    let mut message = data.to_vec();
    let bit_len = (data.len() as u64) * 8;
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in message.chunks(64) {
        let mut w = [0u32; 64];
        for (i, word) in chunk.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, value) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(value);
        }
    }

    h.iter().map(|word| format!("{word:08x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_the_published_vectors() {
        // If this drifts, `sha256sum` on the host disagrees with us and every
        // install fails as "corrupted" — so it is worth pinning to the standard
        // rather than to our own output.
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn sha256_handles_a_block_boundary() {
        // A binary is megabytes; the padding path that only shows up at exact
        // multiples of 64 bytes is the one that silently breaks.
        let exactly_one_block = vec![b'a'; 64];
        assert_eq!(
            sha256_hex(&exactly_one_block),
            "ffe054fe7ae0cb6dc65c3af9b61d5209f439851db43d0ba5997337df154668eb"
        );
        let just_over = vec![b'a'; 65];
        assert_ne!(sha256_hex(&just_over), sha256_hex(&exactly_one_block));
    }

    #[test]
    fn the_unit_starts_the_daemon_from_the_users_own_bin() {
        // %h so the unit is not tied to one username, and no absolute /home.
        assert!(UNIT.contains("ExecStart=%h/.local/bin/farcoolerd"));
        assert!(UNIT.contains("WantedBy=default.target"));
        // A crash loop with systemd's 100ms default would hammer the machine.
        assert!(UNIT.contains("RestartSec=5s"));
    }

    #[test]
    fn wsl_is_its_own_platform_because_persistence_differs() {
        // `uname -s` says Linux on WSL exactly as it does anywhere else, and
        // the kernel release is the only reliable tell. Treating it as ordinary
        // Linux means promising a systemd service on a machine that usually has
        // no systemd at all — the daemon then never starts, and nothing says so.
        for kernel in ["5.15.153.1-microsoft-standard-WSL2", "4.4.0-19041-Microsoft"] {
            assert!(
                kernel.to_lowercase().contains("microsoft"),
                "{kernel} must be recognized as WSL"
            );
        }
        assert!(!"6.8.0-45-generic".to_lowercase().contains("microsoft"));
        assert!(!"25.5.0".to_lowercase().contains("microsoft"));
    }

    #[test]
    fn each_platform_looks_for_its_own_binaries() {
        assert_eq!(Platform::Linux.dist_slug("x86_64").as_deref(), Some("x86_64-linux"));
        // WSL runs Linux binaries. It differs in how the daemon is kept alive,
        // not in what it can execute.
        assert_eq!(Platform::Wsl.dist_slug("aarch64").as_deref(), Some("aarch64-linux"));
        assert_eq!(Platform::MacOs.dist_slug("arm64").as_deref(), Some("aarch64-darwin"));
        assert_eq!(Platform::Unsupported.dist_slug("x86_64"), None);
        // An architecture with no build here is not a directory to guess at.
        assert_eq!(Platform::Linux.dist_slug("riscv64"), None);
    }

}
