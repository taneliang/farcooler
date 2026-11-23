// The Mac bundle builds its native binaries in one Cargo graph (ov-1183).
// Only the named executable gets these linker arguments; libraries, build
// scripts and the daemon's LFS filter must never embed the Go archive.
fn main() {
    println!("cargo:rerun-if-env-changed=FARCOOLER_BUNDLE_TAILCAT_DIR");
    let Some(directory) = std::env::var_os("FARCOOLER_BUNDLE_TAILCAT_DIR") else {
        return;
    };
    assert_eq!(std::env::var("CARGO_CFG_TARGET_OS").as_deref(), Ok("macos"));
    assert!(
        std::env::var_os("CARGO_FEATURE_TAILCAT").is_some(),
        "bundle requires tailcat"
    );
    let directory = std::path::PathBuf::from(directory);
    assert!(
        directory.is_absolute(),
        "bundle archive directory must be absolute"
    );
    let archive = directory.join("libtailcat.a");
    assert!(archive.is_file(), "bundle tunnel archive is missing");
    println!("cargo:rerun-if-changed={}", archive.display());
    println!("cargo:rustc-link-arg-bin=farcoolerd={}", archive.display());
    println!("cargo:rustc-link-arg-bin=farcoolerd=-framework");
    println!("cargo:rustc-link-arg-bin=farcoolerd=Security");
}
