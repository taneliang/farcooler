//! Cargo's test executable depth is not fixed: before Rust 1.100 it lived in
//! `<profile>/deps`, then in `<profile>/build/<package>/<hash>`. Share this
//! lookup by `#[path]` so shipping libraries carry no test support (ov-1125).

use std::path::{Path, PathBuf};

pub fn daemon_binary() -> Option<PathBuf> {
    find_above(&std::env::current_exe().ok()?)
}

fn find_above(exe: &Path) -> Option<PathBuf> {
    exe.parent()?.ancestors().map(|dir| dir.join("farcoolerd")).find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_daemon_in_both_cargo_layouts() {
        let dir = tempfile::tempdir().unwrap();
        for profile in ["target/debug", "custom-target/release", "custom-target/aarch64-apple-darwin/ci"] {
            let profile = dir.path().join(profile);
            std::fs::create_dir_all(&profile).unwrap();
            let daemon = profile.join("farcoolerd");
            std::fs::write(&daemon, b"fixture").unwrap();
            for layout in ["deps/test-hash", "build/farcooler-client/hash/test-hash"] {
                assert_eq!(find_above(&profile.join(layout)), Some(daemon.clone()));
            }
        }
    }

    #[test]
    fn a_directory_named_farcoolerd_is_not_a_binary() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("target/debug");
        std::fs::create_dir_all(profile.join("farcoolerd")).unwrap();
        assert_eq!(find_above(&profile.join("build/package/hash/test")), None);
    }

    #[test]
    fn the_nearest_built_daemon_wins() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("target/debug");
        std::fs::create_dir_all(&profile).unwrap();
        std::fs::write(dir.path().join("farcoolerd"), b"other build").unwrap();
        std::fs::write(profile.join("farcoolerd"), b"this build").unwrap();
        assert_eq!(find_above(&profile.join("deps/test")), Some(profile.join("farcoolerd")));
    }
}
