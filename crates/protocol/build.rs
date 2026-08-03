use std::io::Result;

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
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/index");
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

    let mut cfg = prost_build::Config::new();
    cfg.bytes(["."]);
    cfg.compile_protos(&["../../proto/farcooler.proto"], &["../../proto"])?;
    Ok(())
}
