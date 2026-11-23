use std::process::Command;

#[test]
fn the_cli_validates_its_runtime_root_before_using_it() {
    let dir = tempfile::Builder::new().prefix("fc-runtime-").tempdir_in("/tmp").unwrap();
    let root = dir.path().canonicalize().unwrap().join("runtime");

    let invalid = Command::new(env!("CARGO_BIN_EXE_farcooler"))
        .arg("--version")
        .env("FARCOOLER_HOME", root.join("../other"))
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(1));
    assert!(invalid.stdout.is_empty());
    assert!(String::from_utf8_lossy(&invalid.stderr).starts_with("invalid runtime root:"));

    let valid = Command::new(env!("CARGO_BIN_EXE_farcooler"))
        .arg("--version")
        .env("FARCOOLER_HOME", &root)
        .output()
        .unwrap();
    assert!(valid.status.success(), "{}", String::from_utf8_lossy(&valid.stderr));
    assert!(String::from_utf8_lossy(&valid.stdout).starts_with("farcooler "));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}
