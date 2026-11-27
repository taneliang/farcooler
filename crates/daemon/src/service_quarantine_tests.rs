//! The native launch preflight against real quarantine attributes (ov-1511).
use std::os::unix::fs::PermissionsExt as _;

/// Run the preflight on a script carrying `mark`. `Ok` if the agent ran, else the path it refused.
fn launch(mark: Option<&str>) -> Result<(), String> {
    let dir = tempfile::tempdir().unwrap();
    let agent = dir.path().join("agent");
    let ran = dir.path().join("ran");
    let receipt = dir.path().join("receipt");
    std::fs::write(&agent, "#!/bin/sh\n: > \"$1\"\n").unwrap();
    std::fs::set_permissions(&agent, std::fs::Permissions::from_mode(0o700)).unwrap();
    if let Some(mark) = mark {
        assert!(std::process::Command::new("/usr/bin/xattr")
            .args(["-w", "com.apple.quarantine", mark]).arg(&agent).status().unwrap().success());
    }
    let status = std::process::Command::new("/bin/sh")
        .args(["-c", super::NATIVE_AGENT_PREFLIGHT, "farcooler-agent"]).arg(&agent).arg(&ran)
        .env("FARCOOLER_NATIVE_QUARANTINE_RECEIPT", &receipt)
        .status().unwrap();
    match std::fs::read_to_string(&receipt) {
        Ok(refused) => {
            assert_eq!(status.code(), Some(126));
            assert!(!ran.exists(), "the refused agent ran");
            Err(refused.trim_end().to_owned())
        }
        Err(_) => {
            assert!(status.success() && ran.exists(), "the agent did not run: {status}");
            Ok(())
        }
    }
}

#[test]
fn an_unapproved_quarantine_mark_refuses_the_launch() {
    let refused = launch(Some("0081;6928f1a0;Homebrew Cask;")).unwrap_err();
    assert!(refused.ends_with("/agent"), "{refused}");
}

#[test]
fn an_approved_quarantine_mark_launches() {
    for mark in ["01c1;6928f1a0;Homebrew Cask;", "00c1;6928f1a0;Homebrew Cask;"] {
        assert_eq!(launch(Some(mark)), Ok(()), "{mark}");
    }
}

#[test]
fn a_mark_with_unreadable_flags_refuses_the_launch() {
    for mark in ["", "junk;6928f1a0;test;", ";6928f1a0;test;"] {
        assert!(launch(Some(mark)).is_err(), "{mark:?}");
    }
}

#[test]
fn an_unmarked_agent_launches() {
    assert_eq!(launch(None), Ok(()));
}
