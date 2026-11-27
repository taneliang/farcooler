//! Attribute a red landing using completed, attempt-specific test evidence.
use super::*;
use std::collections::BTreeSet;

#[derive(Deserialize)]
struct Job {
    id: u64,
    name: String,
    conclusion: String,
    steps: Vec<Step>,
}

#[derive(Deserialize)]
struct Step {
    name: String,
    conclusion: String,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Failure {
    workflow: String,
    job: String,
    step: String,
    test: String,
}

impl Exec {
    pub(super) fn can_read_failed_tests(&self) -> bool {
        self.coding.as_ref().and_then(|c| c.ci.command.environment.get("CI_GH"))
            .is_some_and(|gh| Path::new(gh).is_absolute())
    }

    pub(super) async fn red_landing_ci(&self, binding: &Binding, candidate: &str, required: &[CiCheck],
        poll: u32, cancel: &tokio::sync::watch::Receiver<bool>) -> LandingCi
    {
        // Commits read, not pushes: every commit below a push's head is one of them.
        const WALK: u32 = 64;
        let observed_ms = self.clock_ms();
        let mut origin = None;
        let mut inherited = Vec::new();
        // The oldest red commit that CI ran on: what the plan can name as breaking main.
        let mut red = candidate.to_owned();
        // How far back the walk has read, through commits with and without a run.
        let mut commit = candidate.to_owned();
        let mut compared = false;
        for depth in 0..WALK {
            if *cancel.borrow() { break; }
            let Some(parent) = self.ci_parent(binding, &commit).await else { break };
            commit = parent.clone();
            // Separate receipt names for ancestry reads and the ordinary poll.
            let numbered = 0x8000_0000 + (poll % 0x100_0000) * WALK + depth;
            let previous = match self.ci_poll(binding, &parent, numbered, cancel).await {
                Ok(Some(previous)) => previous,
                // CI runs on the head of a push, so a commit below one has no run to compare.
                Ok(None) => continue,
                Err(_) => break,
            };
            if previous.status == CiStatus::Passed {
                origin = Some(red);
                break;
            }
            if previous.status != CiStatus::Failed { break; }
            if !compared
                && let Some(child) = self.failed_tests(binding, required, cancel).await
                && let Some(parent) = self.failed_tests(binding, &previous.required, cancel).await
                && child.is_subset(&parent)
            {
                let names = child.into_iter()
                    .map(|f| format!("{} / {} / {} — {}", f.workflow, f.job, f.step, f.test))
                    .collect::<Vec<_>>();
                // The entire list goes on the ticket. Oversized evidence can't clear the failure.
                let verdict = LandingCi::Red { observed_ms, origin: None, inherited: names };
                if verdict.note().is_some_and(|note| farcooler_workflow::valid_rework_reason(&note) && !note.chars().any(char::is_control))
                    && let LandingCi::Red { inherited: names, .. } = verdict
                {
                    inherited = names;
                }
            }
            compared = true;
            if let Some(known) = self.known_red_origin(&binding.workspace, &parent) {
                origin = Some(known);
                break;
            }
            red = parent;
        }
        LandingCi::Red { observed_ms, origin, inherited }
    }

    fn known_red_origin(&self, workspace: &str, sha: &str) -> Option<String> {
        let state = self.store.workflow_state(workspace).ok()??;
        let live = state.items.values().filter(|i| i.group_of.is_some() && i.pushed.as_deref() == Some(sha))
            .filter_map(|i| i.landing_ci.as_ref());
        let archived = state.archive.values().filter_map(|a| a.group.as_ref())
            .filter(|g| g.pushed.as_deref() == Some(sha)).filter_map(|g| g.landing_ci.as_ref());
        live.chain(archived).find_map(|ci| match ci {
            LandingCi::Red { origin, .. } => origin.clone(),
            LandingCi::Green { .. } => None,
        })
    }

    async fn ci_parent(&self, binding: &Binding, candidate: &str) -> Option<String> {
        // The commit object's own headers are authoritative even if the shared
        // checkout has a replace ref or a graft.
        let raw = self.git_recorded(binding, &self.repository(), &["--no-replace-objects", "cat-file", "commit", candidate]).await.ok()?;
        raw.lines().take_while(|line| !line.is_empty()).find_map(|line| line.strip_prefix("parent "))
            .filter(|parent| sha(parent)).map(str::to_owned)
    }

    async fn failed_tests(&self, binding: &Binding, checks: &[CiCheck],
        cancel: &tokio::sync::watch::Receiver<bool>) -> Option<BTreeSet<Failure>>
    {
        let repo = &self.coding.as_ref()?.ci.repository;
        let mut failures = BTreeSet::new();
        for check in checks {
            match check.status {
                CiStatus::Passed => continue,
                CiStatus::Failed if check.failure_conclusion => {}
                _ => return None,
            }
            let run = check.run.as_ref()?;
            let path = format!("repos/{repo}/actions/runs/{}/attempts/{}/jobs?per_page=100", run.id, run.attempt);
            let (exit, raw) = self.ci_gh_cancel(binding, "test-jobs", &["api", "--paginate", "--slurp", &path,
                "--jq", "[.[].jobs[] | {id, name, conclusion, steps}]"], cancel.clone()).await.ok()?;
            if exit != Some(0) { return None; }
            let jobs: Vec<Job> = serde_json::from_slice(&raw).ok()?;
            let mut seen_jobs = BTreeSet::new();
            let mut failed_jobs = BTreeSet::new();
            for job in &jobs {
                if !seen_jobs.insert(&job.name) || job.id == 0 || job.name.chars().any(char::is_control) {
                    return None;
                }
                match job.conclusion.as_str() {
                    "success" | "skipped" | "neutral" => continue,
                    "failure" => {}
                    _ => return None,
                }
                failed_jobs.insert(&job.name);
                let mut failed_steps = Vec::new();
                for step in &job.steps {
                    match step.conclusion.as_str() {
                        "success" | "skipped" => {}
                        "failure" => failed_steps.push(step),
                        _ => return None,
                    }
                }
                if failed_steps.is_empty() { return None; }
                let only_one = failed_steps.len() == 1;
                let (exit, log) = self.ci_gh_cancel(binding, "test-log", &["run", "view", &run.id.to_string(),
                    "--attempt", &run.attempt.to_string(), "--repo", repo, "--job", &job.id.to_string(), "--log-failed"],
                    cancel.clone()).await.ok()?;
                if exit != Some(0) { return None; }
                let plain = farcooler_core::activity::strip_ansi(&std::str::from_utf8(&log).ok()?.replace("^[[", "\x1b["));
                // Use log evidence for every failed step. An ambiguous step name
                // or missing test summary leaves the job's failure unexplained.
                for step in failed_steps {
                    if step.name.is_empty() || step.name.chars().any(char::is_control)
                        || job.steps.iter().filter(|s| s.name == step.name).count() != 1
                    {
                        return None;
                    }
                    for test in nextest_failures(&plain, &job.name, &step.name, only_one)? {
                        failures.insert(Failure {
                            workflow: check.workflow.clone(), job: job.name.clone(), step: step.name.clone(), test,
                        });
                    }
                }
            }
            if failed_jobs.is_empty() || failed_jobs != run.failed_jobs.iter().collect() { return None; }
        }
        (!failures.is_empty()).then_some(failures)
    }
}

/// Use nextest's final summary so a test that recovered on retry is never
/// counted. Every failure in that summary must have an ordinary FAIL line.
///
/// `gh` doesn't always know which step printed a line: it then calls every line
/// `UNKNOWN STEP` and prints the whole job. That log speaks for a step only when
/// it's the job's `only_failed` one and the process that ran these tests is the
/// one that exited.
fn nextest_failures(log: &str, job: &str, step: &str, only_failed: bool) -> Option<BTreeSet<String>> {
    const UNKNOWN: &str = "UNKNOWN STEP";
    const EXITED: &str = "##[error]Process completed with exit code ";
    let read: Vec<_> = log.lines().filter_map(|line| {
        let mut fields = line.splitn(3, '\t');
        if fields.next()? != job { return None; }
        let column = fields.next()?;
        Some((column, fields.next()?.split_once(' ')?.1.trim()))
    }).collect();
    let unsplit = !read.is_empty() && read.iter().all(|(column, _)| *column == UNKNOWN);
    if unsplit && (!only_failed || read.iter().filter(|(_, line)| line.starts_with(EXITED)).count() != 1) {
        return None;
    }
    let lines: Vec<_> = read.into_iter().filter(|(column, _)| unsplit || *column == step).map(|(_, line)| line).collect();
    let summary = lines.iter().rposition(|line| line.starts_with("Summary ["))?;
    let (_, counts) = lines[summary].split_once(" tests run: ")
        .or_else(|| lines[summary].split_once(" test run: "))?;
    let mut lines = lines.into_iter().skip(summary + 1);
    let count = counts.split(", ").find_map(|n| n.strip_suffix(" failed"))?.parse::<usize>().ok()?;
    if count == 0 { return None; }
    let mut names = BTreeSet::new();
    for _ in 0..count {
        let (_, test) = lines.next()?.strip_prefix("FAIL ")?.split_once(']')?;
        let test = test.trim();
        let test = if test.starts_with('(') { test.split_once(')')?.1.trim() } else { test };
        if test.split_whitespace().count() != 2 || test.chars().any(char::is_control) || !names.insert(test.to_owned()) {
            return None;
        }
    }
    if lines.next()? != "error: test run failed" { return None; }
    // The next step to start or exit must be this one exiting.
    if unsplit && !lines.clone().find(|line| line.starts_with(EXITED) || line.starts_with("##[group]Run "))
        .is_some_and(|line| line.starts_with(EXITED)) { return None; }
    // The leak wrappers can fail independently of nextest.
    if lines.any(|line| line.contains("LEAKED") || (line.starts_with("test-leak-check:")
        && !line.contains("left nothing") && !line.starts_with("test-leak-check: note:"))) {
        return None;
    }
    Some(names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow_runtime::coding_verbs_tests::{at_join, batched_push, command, lander, phase_of, remote_head, settled, until};
    use std::os::unix::fs::PermissionsExt;

    // As GitHub does, only a commit that was the head of a push gets a run.
    const PROVIDER: &str = r#"import json, os, pathlib, subprocess, sys
candidate = os.environ['FARCOOLER_CANDIDATE']
registry = pathlib.Path(__file__).with_name('commits')
commits = registry.read_text().splitlines() if registry.exists() else [os.environ['RED']]
head = subprocess.check_output(['/usr/bin/git', '--git-dir', os.environ['REMOTE'], 'rev-parse', 'refs/heads/main'], text=True).strip()
if candidate not in commits:
    if candidate != head:
        sys.exit(0)
    commits.append(candidate)
    registry.write_text('\n'.join(commits))
run = 100 + commits.index(candidate)
job = os.environ['TEST_JOB']
pathlib.Path(os.environ['FARCOOLER_RECEIPT']).write_text(json.dumps(dict(
    binding=os.environ['FARCOOLER_OPERATION_BINDING'], repository='offline/repository',
    failed_jobs=[job] if run > 100 else [],
    run=dict(id=run, run_attempt=2, name=os.environ['FARCOOLER_CI_WORKFLOW'], head_sha=candidate,
        status='completed', conclusion='failure' if run > 100 else 'success',
        html_url='fixture://ci/' + str(run), created_at='2026-11-25T00:00:00Z', event='push'))))
"#;

    const GH: &str = r#"#!/usr/bin/python3
import json, os, sys
job, steps = os.environ['TEST_JOB'], json.loads(os.environ['TEST_STEPS'])
if sys.argv[1] == 'api':
    run = int(sys.argv[4].split('/')[5])
    print(json.dumps([dict(id=run, name=job, conclusion='failure',
        steps=[dict(name=step, conclusion='failure') for step, _, _ in steps])]))
else:
    run = int(sys.argv[3])
    for step, binary, test_count in steps:
        tests = ['tests::one'] if test_count == 1 else ['tests::one', 'tests::two']
        if run == 102 and step == os.environ['NEW_FAILURE_STEP']:
            tests[-1] = 'tests::regression'
        noun = 'test' if test_count == 1 else 'tests'
        lines = [f'Summary [ 1.000s] {test_count} {noun} run: {test_count - len(tests)} passed, {len(tests)} failed, 0 skipped']
        lines += [f'^[[31;1mFAIL^[[0m [ 0.113s] ({i}/{test_count}) {binary} {test}' for i, test in enumerate(tests, 1)]
        lines += ['error: test run failed']
        # With one failed step, print it as gh does when it can't split the job's log.
        column = step
        if len(steps) == 1:
            column = 'UNKNOWN STEP'
            lines.append('##[error]Process completed with exit code 100.')
        for line in lines:
            print(job + '\t' + column + '\t2026-11-25T00:11:00Z     ' + line)
"#;

    type TestStep = (&'static str, &'static str, usize);
    const SHARD: TestStep = ("Full Rust workspace shard", "farcooler-daemon", 20);
    const TAILCAT: TestStep = (
        "Run cargo nextest run --profile ci -p farcooler-tailcat --features helper", "farcooler-tailcat", 20,
    );
    const TAILCAT_INTEGRATION: TestStep = (
        "Run cargo nextest run --profile ci -p farcooler-daemon --features tailcat-helper --test an_empty_allowlist_starts_no_tunnel",
        "farcooler-daemon::an_empty_allowlist_starts_no_tunnel", 4,
    );
    const SINGLE_TEST: TestStep = ("Run targeted Rust test", "farcooler-daemon", 1);

    async fn exercise(new_failure_step: Option<&str>, job: &str, steps: &[TestStep]) {
        let (dir, runtime) = lander("pass").await;
        // Each build leaves two commits, so each push has one below its head that CI never ran on.
        let adapter = dir.path().join("adapter.py");
        let script = std::fs::read_to_string(&adapter).unwrap();
        let one = "'-m', 'build '+iteration])";
        assert!(script.contains(one));
        let two = format!("{one}\n    pathlib.Path('second-' + name).write_text('more')\n    subprocess.check_call(['/usr/bin/git', 'add', '--', 'second-' + name])\n    subprocess.check_call(['/usr/bin/git', '-c', 'commit.gpgsign=false', 'commit', '--quiet', '-m', 'second'])");
        std::fs::write(&adapter, script.replace(one, &two)).unwrap();
        let provider = dir.path().join("provider.py");
        let gh = dir.path().join("gh");
        std::fs::write(&provider, PROVIDER).unwrap();
        std::fs::write(&gh, GH).unwrap();
        std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o700)).unwrap();
        let root = runtime.root.clone();
        let mut config = runtime.coding.clone().unwrap();
        config.ci.command.argv = vec!["/usr/bin/python3".into(), provider.to_string_lossy().into_owned()];
        config.ci.command.environment.insert("CI_GH".into(), gh.to_string_lossy().into_owned());
        config.ci.command.environment.insert("NEW_FAILURE_STEP".into(), new_failure_step.unwrap_or_default().into());
        config.ci.command.environment.insert("TEST_JOB".into(), job.into());
        config.ci.command.environment.insert("TEST_STEPS".into(), serde_json::to_string(steps).unwrap());
        std::fs::write(root.join("coding.json"), serde_json::to_vec(&config).unwrap()).unwrap();
        drop(runtime);

        let mut runtime = Runtime::open(&root).unwrap();
        let definition = batched_push(&runtime, "any", 20_000);
        at_join(&mut runtime, "breaker", &definition).await;
        command(&runtime, Command::Queue { item: "breaker".into() });
        until(&mut runtime, |s| phase_of(s, "breaker") == Some(Phase::Failed)).await;
        let root_red = remote_head(&runtime).await;
        let new_failure = new_failure_step.is_some();
        let expected = if new_failure { Phase::Failed } else { Phase::Done };
        let tickets: &[&str] = if new_failure { &["later"] } else { &["later", "third"] };
        for ticket in tickets {
            at_join(&mut runtime, ticket, &definition).await;
            command(&runtime, Command::Queue { item: (*ticket).into() });
            let state = until(&mut runtime, |s| matches!(phase_of(s, ticket), Some(Phase::Done | Phase::Failed))).await;
            assert_eq!(phase_of(&state, ticket), Some(expected.clone()), "{ticket}");
            assert_ne!(remote_head(&runtime).await, root_red);
            assert_eq!(phase_of(&state, "breaker"), Some(Phase::Failed));
            let fact = settled(&runtime, StageKind::AwaitCi);
            assert_eq!(fact["detail"]["landing_ci"]["Red"]["origin"], root_red);
            if new_failure {
                assert_eq!(fact["label"], "rejected");
                assert!(fact["detail"]["landing_ci"]["Red"]["inherited"].is_null());
            } else {
                assert_eq!(fact["label"], "passed");
                let note = state.items[*ticket].landing_ci.as_ref().and_then(LandingCi::note).unwrap();
                for &(step, binary, test_count) in steps {
                    for test in ["tests::one", "tests::two"].into_iter().take(test_count) {
                        assert!(note.contains(&format!("{job} / {step} — {binary} {test}")), "{note}");
                    }
                }
                assert_eq!(state.items[*ticket].landed.as_deref(), Some(remote_head(&runtime).await.as_str()));
            }
        }
    }

    #[tokio::test]
    async fn a_red_main_only_fails_the_ticket_that_broke_it() {
        exercise(None, "Rust tests (1/4)", &[SHARD]).await;
    }

    #[tokio::test]
    async fn a_new_failure_on_an_already_red_main_still_fails_the_ticket() {
        exercise(Some(TAILCAT_INTEGRATION.0), "Rust (ubuntu-24.04)", &[TAILCAT, TAILCAT_INTEGRATION]).await;
    }

    #[tokio::test]
    async fn an_inherited_failure_in_the_linux_tailcat_step_still_lands() {
        exercise(None, "Rust (ubuntu-24.04)", &[TAILCAT]).await;
    }

    #[tokio::test]
    async fn an_inherited_single_test_failure_still_lands() {
        exercise(None, "Rust (ubuntu-24.04)", &[SINGLE_TEST]).await;
    }

    #[tokio::test]
    async fn a_different_single_test_failure_still_fails_the_ticket() {
        exercise(Some(SINGLE_TEST.0), "Rust (ubuntu-24.04)", &[SINGLE_TEST]).await;
    }

    #[tokio::test]
    async fn every_failing_test_step_in_a_job_is_named_in_the_landing_note() {
        exercise(None, "Rust (ubuntu-24.04)", &[TAILCAT, TAILCAT_INTEGRATION, SINGLE_TEST]).await;
    }

    /// Lines `gh run view --log-failed` (2.106.0) printed for job 139928161060, escapes and all: it
    /// could not tell the steps apart, so it printed the whole job under `UNKNOWN STEP`.
    #[test]
    fn a_log_gh_cannot_split_into_steps_is_read_only_when_one_step_failed() {
        let line = |text: &str| format!("Rust tests (4/4)\tUNKNOWN STEP\t2026-11-26T13:15:01.6934959Z {text}\n");
        let run = "\u{1b}[36;1m##[group]Run ./scripts/test-leak-check.py -- cargo nextest run\u{1b}[0m";
        let next = "##[group]Run actions/upload-artifact@v4";
        let summary = "\u{1b}[31;1m     Summary\u{1b}[0m [ 238.211s] \u{1b}[1m1915\u{1b}[0m tests run: \u{1b}[1m1914\u{1b}[0m \u{1b}[32;1mpassed\u{1b}[0m (\u{1b}[1m1\u{1b}[0m \u{1b}[33;1mleaky\u{1b}[0m), \u{1b}[1m1\u{1b}[0m \u{1b}[31;1mfailed\u{1b}[0m, \u{1b}[1m5645\u{1b}[0m \u{1b}[33;1mskipped\u{1b}[0m";
        let fail = "\u{1b}[31;1m        FAIL\u{1b}[0m [  92.565s] (1915/1915) \u{1b}[35;1mfarcooler-daemon::engine_flow_per_skill\u{1b}[0m \u{1b}[34;1ma_ticket_goes_from_draft_to_landed_on_the_commands_the_skill_teaches\u{1b}[0m";
        let error = "\u{1b}[31;1merror\u{1b}[0m: test run failed";
        let clean = "test-leak-check: the run left nothing in TMPDIR or the preferences folder.";
        let exit = "##[error]Process completed with exit code 100.";
        let read = |lines: &[&str], alone| nextest_failures(&lines.iter().map(|l| line(l)).collect::<String>(), "Rust tests (4/4)", "Full Rust workspace shard", alone)
            .map(|names| names.into_iter().collect::<Vec<_>>());
        let strip = |lines: &[&str]| lines.iter().map(|l| farcooler_core::activity::strip_ansi(l)).collect::<Vec<_>>();
        let whole = strip(&[run, summary, fail, error, clean, exit, next]);
        let whole: Vec<&str> = whole.iter().map(String::as_str).collect();
        assert_eq!(read(&whole, true).unwrap(), ["farcooler-daemon::engine_flow_per_skill a_ticket_goes_from_draft_to_landed_on_the_commands_the_skill_teaches"]);
        // With two failed steps, nothing says which one these tests failed in.
        assert!(read(&whole, false).is_none());
        // The step that exited isn't the one that ran these tests.
        let (summary, fail, error) = (whole[1], whole[2], whole[3]);
        assert!(read(&[summary, fail, error, next, exit], true).is_none());
        assert!(read(&[summary, fail, error, clean, exit, next, exit], true).is_none());
        assert!(read(&[summary, fail, error, clean], true).is_none());
    }

    #[test]
    fn incomplete_and_mixed_failed_logs_do_not_prove_inheritance() {
        let prefix = "rust\tshard\t2026-11-25T00:11:00Z ";
        let log = |lines: &[&str]| lines.iter().map(|line| format!("{prefix}{line}\n")).collect::<String>();
        let summary = "Summary [ 1.000s] 20 tests run: 18 passed, 2 failed, 0 skipped";
        let one = "FAIL [ 0.113s] (1/20) farcooler-daemon pause_hold::one";
        let two = "FAIL [ 0.125s] (2/20) farcooler-daemon pause_hold::two";
        let end = "error: test run failed";
        assert_eq!(nextest_failures(&log(&[summary, one, two, end]), "rust", "shard", false).unwrap().len(), 2);
        assert!(nextest_failures(&log(&[summary, one, end]), "rust", "shard", false).is_none());
        assert!(nextest_failures(&log(&[summary, one, two, end, summary, one, end]), "rust", "shard", false).is_none());
        assert!(nextest_failures(&log(&[summary, one, two, end, "tmux-leak-check: LEAKED farcoolerd"]), "rust", "shard", false).is_none());
    }
}
