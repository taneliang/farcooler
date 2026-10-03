use std::io::Result;

/// Asks git for a directory and makes it absolute (git prints it relative to
/// the cwd when it can). `None` when git is missing or this is not a checkout.
fn git_dir(flag: &str) -> Option<std::path::PathBuf> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", flag])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text.is_empty() {
        return None;
    }
    std::env::current_dir().ok().map(|cwd| cwd.join(text))
}

/// Declares the git state the build stamp and channel depend on.
///
/// In a linked worktree `.git` is a file, so a hard-coded `../../.git/HEAD`
/// does not exist, and cargo treats a missing `rerun-if-changed` path as
/// always stale: every build reran this script and recompiled protocol and its
/// dependents. So ask git where things really live: `HEAD` and `index` are
/// per worktree (`--git-dir`), refs and tags are shared (`--git-common-dir`).
/// Only paths that exist are emitted. With no git or no `.git` (a source
/// tarball) nothing is emitted and the stamp falls back to `unknown`.
fn watch_git() {
    let (Some(git), Some(common)) = (git_dir("--git-dir"), git_dir("--git-common-dir")) else {
        return;
    };
    let mut paths = vec![git.join("HEAD"), git.join("index"), common.join("packed-refs")];
    // A commit moves the current branch's ref, not HEAD itself. Before the
    // branch has a loose ref file, watch the heads directory instead.
    let branch = std::fs::read_to_string(git.join("HEAD"))
        .ok()
        .and_then(|h| h.trim().strip_prefix("ref: ").map(str::to_string));
    match branch.map(|r| common.join(r)) {
        Some(r) if r.exists() => paths.push(r),
        _ => paths.push(common.join("refs/heads")),
    }
    paths.push(common.join("refs/tags"));
    for p in paths.into_iter().filter(|p| p.exists()) {
        println!("cargo:rerun-if-changed={}", p.display());
    }
}

fn main() -> Result<()> {
    // Use a vendored protoc so the build has no system dependency. The proto
    // files are the canonical protocol source of truth and ship with every
    // daemon/client release.
    let protoc = protoc_bin_vendored::protoc_bin_path().expect("vendored protoc");
    unsafe {
        std::env::set_var("PROTOC", protoc);
    }

    println!("cargo:rerun-if-changed=../../proto/farcooler.proto");

    // The build stamp every component reports.
    //
    // `CARGO_PKG_VERSION` alone is useless for this: it is "0.1.0" in every
    // build ever made, so a client talking to a daemon compiled from different
    // source could not tell, and did not. Locally that is solved by building
    // rather than copying (see `apps/macos/build-app.sh`), but a phone talking
    // to a Mac, or a Mac driving a Linux host over ssh, has no such guarantee —
    // there the two really are built separately and the only honest answer is
    // to say which source each came from.
    watch_git();
    let sha = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let dirty = std::process::Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=no"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .is_some_and(|o| !o.stdout.is_empty());
    println!(
        "cargo:rustc-env=FARCOOLER_BUILD={}+{}{}",
        env!("CARGO_PKG_VERSION"),
        sha,
        if dirty { "-dirty" } else { "" }
    );

    // Which channel this build belongs to, from the one implementation of the
    // question.
    //
    // Shelling out to `scripts/version.sh` rather than re-deriving it here: a
    // second implementation of "what channel is this" is exactly the drift that
    // script exists to prevent, and this build script already shells out to git
    // twice above.
    //
    // A missing or failing script is `dev`, deliberately, and for the reason
    // version.sh gives about an unstamped bundle: defaulting the other way
    // would let a build made outside the release path call itself a release.
    println!("cargo:rerun-if-changed=../../scripts/version.sh");
    // The environment `version.sh` reads, or the answer is cached forever.
    //
    // Cargo re-runs a build script when its declared inputs change, and an
    // environment variable is only an input once it is declared one. Without
    // these, `FARCOOLER_CHANNEL=stable cargo build` on a tree whose script and
    // tags had not moved reused the channel from the FIRST build of that tree
    // and said nothing — so the flag appeared to work, the binary was stamped
    // `local`, and it quietly used a different runtime directory and a
    // different database than the one it was built to replace.
    //
    // Which is the failure this feature is least able to afford: the whole
    // point of a channel is that it decides which install a binary is, and a
    // promotion build that silently keeps a stale answer is one that ships
    // canary as stable or the reverse.
    println!("cargo:rerun-if-env-changed=FARCOOLER_CHANNEL");
    println!("cargo:rerun-if-env-changed=FARCOOLER_TAG");
    let channel = std::process::Command::new("../../scripts/version.sh")
        .arg("channel")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "dev".to_string());
    println!("cargo:rustc-env=FARCOOLER_CHANNEL={channel}");

    let mut cfg = prost_build::Config::new();
    cfg.bytes(["."]);
    // The compiled descriptor, kept so a test can read every message's field
    // numbers by name. Renaming a message is free on the wire only while its
    // numbers stay put, and that is checked against this, not by eye.
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    cfg.file_descriptor_set_path(out.join("farcooler_descriptor.bin"));
    cfg.compile_protos(&["../../proto/farcooler.proto"], &["../../proto"])?;
    Ok(())
}
