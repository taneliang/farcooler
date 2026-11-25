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
        const WALK: u32 = 12;
        let observed_ms = self.clock_ms();
        let mut origin = None;
        let mut inherited = Vec::new();
        let mut commit = candidate.to_owned();
        for depth in 0..WALK {
            if *cancel.borrow() { break; }
            let Some(parent) = self.ci_parent(binding, &commit).await else { break };
            // Separate receipt names for ancestry reads and the ordinary poll.
            let numbered = 0x8000_0000 + (poll % 0x100_0000) * WALK + depth;
            let Ok(Some(previous)) = self.ci_poll(binding, &parent, numbered, cancel).await else { break };
            if previous.status == CiStatus::Passed {
                origin = Some(commit);
                break;
            }
            if previous.status != CiStatus::Failed { break; }
            if depth == 0
                && let Some(child) = self.failed_tests(binding, required, cancel).await
                && let Some(parent) = self.failed_tests(binding, &previous.required, cancel).await
                && child.is_subset(&parent)
            {
                let names = child.into_iter().map(|f| format!("{} / {} — {}", f.workflow, f.job, f.test)).collect::<Vec<_>>();
                // The entire list goes on the ticket. Oversized evidence can't clear the failure.
                let red = LandingCi::Red { observed_ms, origin: None, inherited: names };
                if red.note().is_some_and(|note| farcooler_workflow::valid_rework_reason(&note) && !note.chars().any(char::is_control))
                    && let LandingCi::Red { inherited: names, .. } = red
                {
                    inherited = names;
                }
            }
            if let Some(known) = self.known_red_origin(&binding.workspace, &parent) {
                origin = Some(known);
                break;
            }
            commit = parent;
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
                if job.steps.iter().any(|s| !matches!(s.conclusion.as_str(), "success" | "skipped" | "failure")) {
                    return None;
                }
                let mut failed = job.steps.iter().filter(|s| s.conclusion == "failure");
                let step = failed.next()?;
                if failed.next().is_some() || !matches!(step.name.as_str(), "Full Rust workspace shard" | "macOS native Rust tests") {
                    return None;
                }
                let (exit, log) = self.ci_gh_cancel(binding, "test-log", &["run", "view", &run.id.to_string(),
                    "--attempt", &run.attempt.to_string(), "--repo", repo, "--job", &job.id.to_string(), "--log-failed"],
                    cancel.clone()).await.ok()?;
                if exit != Some(0) { return None; }
                for test in nextest_failures(std::str::from_utf8(&log).ok()?, &job.name, &step.name)? {
                    failures.insert(Failure { workflow: check.workflow.clone(), job: job.name.clone(), test });
                }
            }
            if failed_jobs.is_empty() || failed_jobs != run.failed_jobs.iter().collect() { return None; }
        }
        (!failures.is_empty()).then_some(failures)
    }
}

/// Use nextest's final summary so a test that recovered on retry is never
/// counted. Every failure in that summary must have an ordinary FAIL line.
fn nextest_failures(log: &str, job: &str, step: &str) -> Option<BTreeSet<String>> {
    let plain = farcooler_core::activity::strip_ansi(&log.replace("^[[", "\x1b["));
    let lines: Vec<_> = plain.lines().filter_map(|line| {
        let mut fields = line.splitn(3, '\t');
        if fields.next()? != job || fields.next()? != step { return None; }
        Some(fields.next()?.split_once(' ')?.1.trim())
    }).collect();
    let summary = lines.iter().rposition(|line| line.starts_with("Summary ["))?;
    let (_, counts) = lines[summary].split_once(" tests run: ")?;
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

    const PROVIDER: &str = r#"import json, os, pathlib
candidate = os.environ['FARCOOLER_CANDIDATE']
registry = pathlib.Path(__file__).with_name('commits')
commits = registry.read_text().splitlines() if registry.exists() else [os.environ['RED']]
if candidate not in commits:
    commits.append(candidate)
    registry.write_text('\n'.join(commits))
run = 100 + commits.index(candidate)
job = 'Rust Linux shard 1'
pathlib.Path(os.environ['FARCOOLER_RECEIPT']).write_text(json.dumps(dict(
    binding=os.environ['FARCOOLER_OPERATION_BINDING'], repository='offline/repository',
    failed_jobs=[job] if run > 100 else [],
    run=dict(id=run, run_attempt=2, name=os.environ['FARCOOLER_CI_WORKFLOW'], head_sha=candidate,
        status='completed', conclusion='failure' if run > 100 else 'success',
        html_url='fixture://ci/' + str(run), created_at='2026-11-25T00:00:00Z', event='push'))))
"#;

    const GH: &str = r#"#!/usr/bin/python3
import json, os, sys
job, step = 'Rust Linux shard 1', 'Full Rust workspace shard'
if sys.argv[1] == 'api':
    run = int(sys.argv[4].split('/')[5])
    print(json.dumps([dict(id=run, name=job, conclusion='failure',
        steps=[dict(name=step, conclusion='failure')])]))
else:
    run = int(sys.argv[3])
    tests = ['pause_hold::one', 'pause_hold::two'] if run != 102 or os.environ['NEW_FAILURE'] == 'no' else ['pause_hold::one', 'regression']
    lines = ['Summary [ 1.000s] 20 tests run: 18 passed, 2 failed, 0 skipped']
    lines += ['FAIL [ 0.113s] (1/20) farcooler-daemon ' + test for test in tests]
    lines += ['error: test run failed', 'test-leak-check: the run left nothing in TMPDIR or the preferences folder.']
    for line in lines:
        print(job + '\t' + step + '\t2026-11-25T00:11:00Z     ' + line)
"#;

    async fn exercise(new_failure: bool) {
        let (dir, runtime) = lander("pass").await;
        let provider = dir.path().join("provider.py");
        let gh = dir.path().join("gh");
        std::fs::write(&provider, PROVIDER).unwrap();
        std::fs::write(&gh, GH).unwrap();
        std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o700)).unwrap();
        let root = runtime.root.clone();
        let mut config = runtime.coding.clone().unwrap();
        config.ci.command.argv = vec!["/usr/bin/python3".into(), provider.to_string_lossy().into_owned()];
        config.ci.command.environment.insert("CI_GH".into(), gh.to_string_lossy().into_owned());
        config.ci.command.environment.insert("NEW_FAILURE".into(), if new_failure { "yes" } else { "no" }.into());
        std::fs::write(root.join("coding.json"), serde_json::to_vec(&config).unwrap()).unwrap();
        drop(runtime);

        let mut runtime = Runtime::open(&root).unwrap();
        let definition = batched_push(&runtime, "any", 20_000);
        at_join(&mut runtime, "breaker", &definition).await;
        command(&runtime, Command::Queue { item: "breaker".into() });
        until(&mut runtime, |s| phase_of(s, "breaker") == Some(Phase::Failed)).await;
        let root_red = remote_head(&runtime).await;
        let expected = if new_failure { Phase::Failed } else { Phase::Done };
        let tickets: &[&str] = if new_failure { &["later"] } else { &["later", "third"] };
        for ticket in tickets {
            at_join(&mut runtime, ticket, &definition).await;
            command(&runtime, Command::Queue { item: (*ticket).into() });
            let state = until(&mut runtime, |s| phase_of(s, ticket) == Some(expected.clone())).await;
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
                assert!(note.contains("pause_hold::one") && note.contains("pause_hold::two"), "{note}");
                assert_eq!(state.items[*ticket].landed.as_deref(), Some(remote_head(&runtime).await.as_str()));
            }
        }
    }

    #[tokio::test]
    async fn a_red_main_only_fails_the_ticket_that_broke_it() {
        exercise(false).await;
    }

    #[tokio::test]
    async fn a_new_failure_on_an_already_red_main_still_fails_the_ticket() {
        exercise(true).await;
    }

    #[test]
    fn incomplete_and_mixed_failed_logs_do_not_prove_inheritance() {
        let prefix = "rust\tshard\t2026-11-25T00:11:00Z ";
        let log = |lines: &[&str]| lines.iter().map(|line| format!("{prefix}{line}\n")).collect::<String>();
        let summary = "Summary [ 1.000s] 20 tests run: 18 passed, 2 failed, 0 skipped";
        let one = "FAIL [ 0.113s] (1/20) farcooler-daemon pause_hold::one";
        let two = "\x1b[31mFAIL\x1b[0m [ 0.125s] (2/20) farcooler-daemon pause_hold::two";
        let end = "error: test run failed";
        assert_eq!(nextest_failures(&log(&[summary, one, two, end]), "rust", "shard").unwrap().len(), 2);
        assert!(nextest_failures(&log(&[summary, one, end]), "rust", "shard").is_none());
        assert!(nextest_failures(&log(&[summary, one, two, end, summary, one, end]), "rust", "shard").is_none());
        assert!(nextest_failures(&log(&[summary, one, two, end, "tmux-leak-check: LEAKED farcoolerd"]), "rust", "shard").is_none());
    }
}
