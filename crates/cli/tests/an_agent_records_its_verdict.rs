//! ov-969: a reviewing agent records its verdict with `farcooler workflow verdict`, and the bytes it writes are the
//! receipt the engine's review gate reads.
use farcooler_daemon::workflow_runtime::{CodingReviewReceipt, AGENT_REVIEWER};

fn verdict(env: &[(&str, &str)], passed: &str) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_farcooler"))
        .args(["workflow", "verdict", "--passed", passed])
        .env_clear().envs(env.iter().copied())
        .output().expect("run farcooler")
}

#[test]
fn the_verdict_is_written_where_the_step_reads_it() {
    let dir = tempfile::tempdir().unwrap();
    let receipt = dir.path().join("effect.provider.json");
    let candidate = "a".repeat(40);
    let env = [("FARCOOLER_RECEIPT", receipt.to_str().unwrap()), ("FARCOOLER_OPERATION_BINDING", "wf1:binding"),
        ("FARCOOLER_CANDIDATE", candidate.as_str())];
    for (word, passed) in [("true", true), ("false", false)] {
        let out = verdict(&env, word);
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let read: CodingReviewReceipt = serde_json::from_slice(&std::fs::read(&receipt).unwrap()).unwrap();
        assert_eq!((read.binding.as_str(), read.identity.as_str(), read.candidate.as_str(), read.passed),
            ("wf1:binding", AGENT_REVIEWER, candidate.as_str(), passed));
    }
    // Only the receipt is left: no draft beside it.
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn outside_a_review_it_says_so_and_writes_nothing() {
    let out = verdict(&[], "true");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("only works inside an agent the engine started to review a ticket"), "{stderr}");
}

#[test]
fn a_verdict_must_be_given() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_farcooler")).args(["workflow", "verdict"]).output().unwrap();
    assert!(!out.status.success());
}
