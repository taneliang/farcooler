//! The fold skips commits whose change is already in its tree (ov-1283), so a ticket stacked on another
//! lands without a manual rebase. Known derived baselines can also be settled during a pick.
use std::path::Path;
use super::{Binding, Command, Runtime, StageKind};
use super::coding_verbs_tests::{at_join, batched_push, command, lander, outbox, until};
use super::coding_tests::{admitted, configured};
use super::tests::enqueue;

/// A scratch repository: `main` holds one file, and the returned closure runs git in it.
fn stacked_repository(dir: &Path) -> impl Fn(&[&str]) -> String + '_ {
    let g = move |args: &[&str]| -> String {
        let out = std::process::Command::new("/usr/bin/git").current_dir(dir)
            .args(["-c", "user.name=T", "-c", "user.email=t@localhost", "-c", "commit.gpgsign=false"]).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };
    g(&["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("shared.txt"), "one\ntwo\nthree\nfour\nfive\nsix\nseven\n").unwrap();
    g(&["add", "."]); g(&["commit", "-qm", "base"]);
    g
}

fn edit(dir: &Path, from: &str, to: &str) {
    let text = std::fs::read_to_string(dir.join("shared.txt")).unwrap();
    std::fs::write(dir.join("shared.txt"), text.replacen(from, to, 1)).unwrap();
}

async fn binding() -> (tempfile::TempDir, Runtime, Binding) {
    let (dir, mut runtime) = configured(false, "pass").await;
    let definition = runtime.configured_definition().unwrap();
    enqueue(&mut runtime, "w", "one", definition);
    let binding = admitted(&runtime).await;
    (dir, runtime, binding)
}

fn member(item: &str, candidate: &str) -> farcooler_workflow::Member {
    farcooler_workflow::Member { item: item.into(), candidate: candidate.into() }
}

/// B is stacked on A, whose second commit touches the line its first one did. A folds first; B's list
/// carries A's commits again, which are skipped, and the tree equals a manual rebase of B onto A.
#[tokio::test]
async fn a_stacked_ticket_folds_clean_after_the_ticket_it_is_stacked_on() {
    let (_dir, runtime, binding) = binding().await;
    let repo = tempfile::tempdir().unwrap();
    let g = stacked_repository(repo.path());
    let base = g(&["rev-parse", "HEAD"]);
    g(&["checkout", "-qb", "a"]);
    edit(repo.path(), "two", "two-a1"); g(&["commit", "-qam", "a1"]);
    edit(repo.path(), "two-a1", "two-a2"); g(&["commit", "-qam", "a2"]);
    g(&["checkout", "-qb", "b"]);
    edit(repo.path(), "six", "six-b"); g(&["commit", "-qam", "b1"]);
    let (a, b) = (g(&["rev-parse", "a"]), g(&["rev-parse", "b"]));
    g(&["checkout", "-q", "-b", "manual", &a]);
    g(&["cherry-pick", &b]);
    let expected = g(&["rev-parse", "HEAD^{tree}"]);
    g(&["checkout", "-q", "--detach", &base]);
    assert!(runtime.pick_member(Some(&binding), repo.path(), &member("a", &a)).await.unwrap().is_none());
    assert!(runtime.pick_member(Some(&binding), repo.path(), &member("b", &b)).await.unwrap().is_none(), "A's commits are skipped");
    assert_eq!(g(&["rev-parse", "HEAD^{tree}"]), expected);
}

/// A's commits already sit on the base as copies under other shas; B, stacked on A's originals, folds clean.
#[tokio::test]
async fn a_ticket_stacked_on_commits_already_on_the_base_as_copies_folds_clean() {
    let (_dir, runtime, binding) = binding().await;
    let repo = tempfile::tempdir().unwrap();
    let g = stacked_repository(repo.path());
    let start = g(&["rev-parse", "HEAD"]);
    g(&["checkout", "-qb", "a"]);
    edit(repo.path(), "two", "two-a1"); g(&["commit", "-qam", "a1"]);
    edit(repo.path(), "two-a1", "two-a2"); g(&["commit", "-qam", "a2"]);
    g(&["checkout", "-qb", "b"]);
    edit(repo.path(), "six", "six-b"); g(&["commit", "-qam", "b1"]);
    let b = g(&["rev-parse", "b"]);
    g(&["checkout", "-q", "-B", "main", &start]);
    edit(repo.path(), "four", "four-main"); g(&["commit", "-qam", "unrelated"]);
    g(&["cherry-pick", "a~1", "a"]);
    let base = g(&["rev-parse", "HEAD"]);
    g(&["checkout", "-q", "--detach", &base]);
    assert!(runtime.pick_member(Some(&binding), repo.path(), &member("b", &b)).await.unwrap().is_none());
    assert_eq!(g(&["rev-list", "--count", &format!("{base}..HEAD")]), "1", "only B's own commit is added");
    assert_eq!(std::fs::read_to_string(repo.path().join("shared.txt")).unwrap(),
        "one\ntwo-a2\nthree\nfour-main\nfive\nsix-b\nseven\n");
}

/// A commit that truly conflicts is still the member's conflict, with the file named.
#[tokio::test]
async fn a_commit_that_truly_conflicts_still_fails_the_fold_naming_the_file() {
    let (_dir, runtime, binding) = binding().await;
    let repo = tempfile::tempdir().unwrap();
    let g = stacked_repository(repo.path());
    g(&["checkout", "-qb", "t"]);
    edit(repo.path(), "two", "two-t"); g(&["commit", "-qam", "t1"]);
    let t = g(&["rev-parse", "t"]);
    g(&["checkout", "-q", "main"]);
    edit(repo.path(), "two", "two-main"); g(&["commit", "-qam", "main moved"]);
    let conflict = runtime.pick_member(Some(&binding), repo.path(), &member("t", &t)).await.unwrap().unwrap();
    assert_eq!(conflict.files, ["shared.txt"]);
    assert_eq!(g(&["status", "--porcelain"]), "", "the aborted pick leaves the tree settled");
}

/// Two reviewed floor bumps collide at the same line. The shared gate must see both tests and
/// their combined floor, without routing either member back through build.
#[tokio::test]
async fn a_shared_floor_conflict_reaches_the_group_gate_without_rebuilds() {
    let (dir, mut runtime) = lander("floor-conflict").await;
    let definition = batched_push(&runtime, "any", 5000);
    at_join(&mut runtime, "ahead", &definition).await;
    let script = dir.path().join("adapter.py");
    let text = std::fs::read_to_string(&script).unwrap();
    let edits = r#"if stage == 'build':
    item = os.environ['FARCOOLER_WORKFLOW_ITEM']
    if item in ('first', 'second'):
        count = 1 if item == 'first' else 2
        suite = pathlib.Path('apps/macos')
        tests = suite / (item + 'Tests.swift')
        tests.write_text('@Test func added() {}\n' * count)
        if item == 'second':
            subprocess.check_call(['/usr/bin/git', 'add', '--', str(tests)])
            subprocess.check_call(['/usr/bin/git', '-c', 'commit.gpgsign=false', 'commit', '-qm', 'add tests before bumping floor'])
        (suite / 'test-floor.txt').write_text(str(100 + count) + '\n')
        subprocess.check_call(['/usr/bin/git', 'add', '--', str(suite)])"#;
    let gate = r#"elif stage == 'gate':
    suite = pathlib.Path('apps/macos')
    if (suite / 'firstTests.swift').exists():
        assert (suite / 'secondTests.swift').exists()
        assert (suite / 'test-floor.txt').read_text() == '103\n'
        pathlib.Path(os.environ['MARKER']).touch()"#;
    std::fs::write(&script, text.replace("if stage == 'build':", edits).replace("elif stage == 'gate':", gate)).unwrap();
    for item in ["first", "second"] { at_join(&mut runtime, item, &definition).await; }
    for item in ["ahead", "first", "second"] {
        assert!(command(&runtime, Command::Queue { item: item.into() }).result.is_ok());
    }
    until(&mut runtime, |s| dir.path().join("marker").exists()
        || ["first", "second"].iter().any(|item| s.items.get(*item).is_some_and(|item| item.brief.is_some()))).await;
    assert_eq!(outbox(&runtime, StageKind::Build), 3, "each member built just once");
    assert!(dir.path().join("marker").exists(), "the combined floor reached the group gate");
}
