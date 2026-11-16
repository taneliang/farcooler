//! ov-1280: which red CI runs failed in setup, before the repository's own commands ran. Fed the jobs, steps and
//! failed-log lines GitHub reported for run 43745907131, the Nov 16 runner outage.
use farcooler_workflow::{setup_only, CiRun, FailedStep};

const FIXTURE: &str = include_str!("../../../test/fixtures/landing/run43745907131-jobs.json");

/// The id of the first action a failed log ends in failure: the innermost, as the CI look's hook reads it.
fn first_failed_action(lines: &[serde_json::Value]) -> Option<String> {
    lines.iter().filter_map(|l| l.as_str()).find_map(|l| {
        let rest = l.split("##[end-action id=").nth(1)?;
        let (id, tail) = rest.split_once(';')?;
        tail.starts_with("outcome=failure;").then(|| id.to_owned())
    })
}
/// The run as the CI look reports it: every failed job, the step GitHub marks failed, and the action its log names.
fn reported() -> CiRun {
    let fixture: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
    let failed: Vec<&serde_json::Value> = fixture["jobs"].as_array().unwrap().iter()
        .filter(|j| j["conclusion"] == "failure").collect();
    let failed_steps = failed.iter().map(|j| FailedStep {
        job: j["name"].as_str().unwrap().into(),
        step: j["steps"].as_array().unwrap().iter().find(|s| s["conclusion"] == "failure").unwrap()["name"].as_str().unwrap().into(),
        action: first_failed_action(j["log_failed"].as_array().unwrap()),
    }).collect();
    CiRun { id: fixture["run"]["id"].as_u64().unwrap(), attempt: 1,
        failed_jobs: failed.iter().map(|j| j["name"].as_str().unwrap().into()).collect(), failed_steps }
}
fn step(job: &str, step: &str, action: Option<&str>) -> FailedStep {
    FailedStep { job: job.into(), step: step.into(), action: action.map(String::from) }
}
fn run(steps: Vec<FailedStep>) -> CiRun {
    CiRun { id: 1, attempt: 1, failed_jobs: steps.iter().map(|s| s.job.clone()).collect(), failed_steps: steps }
}

#[test]
fn the_nov16_runner_outage_is_setup_only() {
    let run = reported();
    assert_eq!(run.failed_jobs, ["iOS", "iOS UI (smoke)"]);
    assert_eq!(run.failed_steps, [
        step("iOS", "Run ./.github/actions/ios-ui-build", Some("__self.__dtolnay_rust-toolchain.__run_4")),
        step("iOS UI (smoke)", "Run ./.github/actions/ios-ui-shard", Some("__self.__actions_download-artifact")),
    ], "the jobs API names only the composite step; the failed log names the action inside it");
    assert!(run.valid());
    assert!(setup_only(&run), "a toolchain download, and the artifact its producer never made");
}

#[test]
fn the_repositorys_own_commands_failing_is_not_setup() {
    let mut own = reported();
    // The same composite step, failing at its own `Build the Rust xcframeworks` command.
    own.failed_steps[0].action = Some("__self.__run_2".into());
    assert!(!setup_only(&own), "a build that failed is the change's");
    // A test shard that failed after its products downloaded.
    let mut tests = reported();
    tests.failed_steps[1].action = Some("__self.__run_5".into());
    assert!(!setup_only(&tests));
    // A job of the run left unread: no step on record, no rerun.
    let mut unread = reported();
    unread.failed_steps.pop();
    assert!(!setup_only(&unread));
    let mut twice = reported();
    twice.failed_steps.push(step("iOS", "Run ./scripts/generator-test.sh", None));
    assert!(!setup_only(&twice), "a job with two failures on record is ambiguous");
    let mut extra = reported();
    extra.failed_jobs.push("Rust tests (1/4)".into());
    extra.failed_steps.push(step("Rust tests (1/4)", "Run ./scripts/rust-test-shard.sh 1", None));
    assert!(!setup_only(&extra), "one real failure among setup failures still counts");
    assert!(!setup_only(&run(vec![])), "no failed jobs is not a setup failure");
    assert!(!setup_only(&CiRun { failed_steps: vec![], ..reported() }), "an older receipt with no steps never reruns");
}

#[test]
fn an_artifact_download_alone_is_not_setup_only() {
    // With no producer that failed in setup, the artifact may be missing because a producer built nothing.
    let smoke = reported().failed_steps.pop().unwrap();
    assert!(!setup_only(&run(vec![smoke])));
}

#[test]
fn setup_steps_are_read_from_the_step_or_the_action() {
    for (failed, setup) in [
        (step("Rust", "Set up job", None), true),
        (step("Rust", "Run actions/checkout@v4", None), true),
        (step("Rust", "Run dtolnay/rust-toolchain@stable", None), true),
        (step("Go", "Run actions/setup-go@v5", None), true),
        (step("Rust", "Run Swatinem/rust-cache@v2", None), true),
        (step("Rust", "Run actions/checkout@v4", Some("__actions_checkout")), true),
        (step("Web", "Run ./.github/actions/web", Some("__self.__actions_setup-node")), true),
        (step("Rust", "Run ./scripts/rust-tests.sh", None), false),
        (step("Rust", "Run actions/setup-", None), false),
        (step("Rust", "Run someone/checkout@v4", None), false),
        (step("iOS", "Run ./.github/actions/ios-ui-build", Some("__self.__run")), false),
        (step("iOS", "Run ./.github/actions/ios-ui-build", Some("__self")), false),
        (step("iOS", "Run ./.github/actions/ios-ui-build", None), false),
        (step("iOS", "Run ./.github/actions/ios-ui-build", Some("__self.__run_3.__dtolnay_rust-toolchain")), false),
    ] {
        assert_eq!(setup_only(&run(vec![failed.clone()])), setup, "{failed:?}");
    }
}
