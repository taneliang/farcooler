//! The old spelling keeps working, silently.
//!
//! Renaming a flag that lives in people's shell history and scripts is a
//! breaking change wearing a vocabulary change's clothes. The alias is hidden
//! from `--help` so the docs teach one word, and accepted forever so nothing
//! anyone already wrote stops working.

#[test]
fn the_old_host_flag_is_still_accepted() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_farcooler"))
        .args(["--host", "nobody@nowhere.invalid", "status"])
        .output()
        .expect("run farcooler");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("unexpected argument") && !stderr.contains("unrecognized"),
        "--host was rejected: {stderr}"
    );
}

#[test]
fn the_new_runner_flag_is_accepted() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_farcooler"))
        .args(["--runner", "nobody@nowhere.invalid", "status"])
        .output()
        .expect("run farcooler");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("unexpected argument") && !stderr.contains("unrecognized"),
        "--runner was rejected: {stderr}"
    );
}

/// `--help` teaches one word.
#[test]
fn help_names_only_the_runner() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_farcooler"))
        .arg("--help")
        .output()
        .expect("run farcooler --help");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("--runner"), "no --runner in help");
    assert!(!text.contains("--host"), "--host is still advertised");
}

/// The subcommand renames the same way the flag does.
///
/// `farcooler host install` is what every existing runbook says, so it keeps
/// working; `runner` is the one `--help` teaches.
#[test]
fn both_spellings_of_the_subcommand_are_accepted() {
    for name in ["host", "runner"] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_farcooler"))
            .args([name, "--help"])
            .output()
            .expect("run farcooler");
        assert!(out.status.success(), "`farcooler {name} --help` failed");
    }

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_farcooler"))
        .arg("--help")
        .output()
        .expect("run farcooler --help");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("runner"), "no runner subcommand in help");
}
