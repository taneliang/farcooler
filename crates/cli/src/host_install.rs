//! Installing Overnight on a Linux host, over ssh.
//!
//! Everything here runs through one `ssh` session and touches only the user's
//! own home directory. No root, no package manager, no system unit, nothing
//! written outside `~`. A user who can ssh in can install Overnight, and a user
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
//!   out of it — which is precisely the situation Overnight exists for.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;

type Fallible = Result<(), Box<dyn std::error::Error>>;

/// The systemd user unit. Written to ~/.config/systemd/user/overnight.service.
const UNIT: &str = "\
[Unit]
Description=Overnight daemon
Documentation=https://github.com/overnight
After=default.target

[Service]
Type=simple
ExecStart=%h/.local/bin/overnightd
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
    let arch = remote_capture(target, "uname -m").await?;
    let os = remote_capture(target, "uname -s").await?;
    let os = os.trim().to_string();
    if os != "Linux" {
        return Err(format!(
            "{target} reports {os}. `host install` is Linux only.\n\
             A macOS host installs by running the Overnight app there once, because \
             registering its LaunchAgent needs a GUI session."
        )
        .into());
    }

    let arch = arch.trim();
    let dir = match from {
        Some(p) => p.to_path_buf(),
        None => default_dist(arch),
    };
    let daemon = dir.join("overnightd");
    let cli = dir.join("overnight");
    for path in [&daemon, &cli] {
        if !path.is_file() {
            return Err(format!(
                "No Linux binary at {}.\n\
                 Build them first:\n\
                 \n    ./scripts/build-linux.sh {arch}\n\n\
                 or pass --from <dir> with `overnight` and `overnightd` inside.",
                path.display()
            )
            .into());
        }
    }

    // tmux is what actually keeps the agents alive; without it the daemon runs
    // and every terminal derives lost. Better to say so now than to install
    // successfully and be mysteriously broken.
    let tmux = remote_capture(target, "command -v tmux || true").await?;
    if tmux.trim().is_empty() {
        return Err(format!(
            "{target} has no tmux, and Overnight keeps every terminal inside one.\n\
             Install it first, e.g.  ssh {target} 'sudo apt-get install -y tmux'"
        )
        .into());
    }

    println!("    {os} {arch}, tmux at {}", tmux.trim());

    remote_run(target, "mkdir -p ~/.local/bin ~/.config/systemd/user").await?;

    for (path, name) in [(&daemon, "overnightd"), (&cli, "overnight")] {
        println!("==> Installing {name}");
        upload_verified(target, path, name).await?;
    }

    println!("==> Registering the user service");
    remote_write(target, "~/.config/systemd/user/overnight.service", UNIT).await?;

    // Lingering needs no privilege for one's own account on most distributions,
    // and without it the daemon dies at logout. If it is refused, say so rather
    // than reporting a success that will not survive the night.
    let linger = remote_capture(
        target,
        "loginctl enable-linger \"$USER\" 2>&1 || echo OVERNIGHT_LINGER_FAILED",
    )
    .await?;
    let lingering = !linger.contains("OVERNIGHT_LINGER_FAILED");

    remote_run(
        target,
        "systemctl --user daemon-reload && systemctl --user enable --now overnight.service",
    )
    .await?;

    println!();
    println!("Installed on {target}.");
    if lingering {
        println!("  Lingering is on, so the daemon survives logout and reboot.");
    } else {
        println!("  WARNING: could not enable lingering.");
        println!("  The daemon will stop when you log out of {target}. To fix:");
        println!("      ssh {target} 'sudo loginctl enable-linger $USER'");
    }
    println!();
    println!("Check it:   overnight --host {target} status");
    Ok(())
}

/// What is installed and running on a host.
pub async fn status(target: &str) -> Fallible {
    let report = remote_capture(
        target,
        "echo \"os=$(uname -s)\"; echo \"arch=$(uname -m)\"; \
         echo \"tmux=$(command -v tmux || echo none)\"; \
         echo \"daemon=$(~/.local/bin/overnightd --version 2>/dev/null || echo none)\"; \
         echo \"cli=$(~/.local/bin/overnight --version 2>/dev/null || echo none)\"; \
         echo \"service=$(systemctl --user is-active overnight.service 2>/dev/null || echo inactive)\"; \
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

fn default_dist(arch: &str) -> PathBuf {
    // The names `uname -m` uses, not the ones Rust uses.
    let slug = match arch {
        "aarch64" | "arm64" => "aarch64",
        "x86_64" | "amd64" => "x86_64",
        other => other,
    };
    PathBuf::from("dist").join(format!("{slug}-linux"))
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

async fn remote_run(target: &str, command: &str) -> Fallible {
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

async fn remote_capture(target: &str, command: &str) -> Result<String, Box<dyn std::error::Error>> {
    let out = Command::new("ssh")
        .args(ssh_options())
        .arg(target)
        .arg(command)
        .stderr(Stdio::inherit())
        .output()
        .await?;
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
        assert!(UNIT.contains("ExecStart=%h/.local/bin/overnightd"));
        assert!(UNIT.contains("WantedBy=default.target"));
        // A crash loop with systemd's 100ms default would hammer the machine.
        assert!(UNIT.contains("RestartSec=5s"));
    }

    #[test]
    fn the_default_dist_directory_follows_uname() {
        assert_eq!(default_dist("aarch64"), PathBuf::from("dist/aarch64-linux"));
        assert_eq!(default_dist("arm64"), PathBuf::from("dist/aarch64-linux"));
        assert_eq!(default_dist("x86_64"), PathBuf::from("dist/x86_64-linux"));
    }
}
