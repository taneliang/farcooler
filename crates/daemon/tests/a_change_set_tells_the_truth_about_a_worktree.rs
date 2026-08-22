//! Change-set behaviour against a real git repository.
//!
//! Real git rather than fixtures of its output, because the things most likely
//! to be wrong here are what git actually prints for a merge, for a rename, and
//! for a branch whose base has commits of its own — and a fixture would encode
//! whatever the author believed on the day.

use std::path::Path;
use std::process::Command;

use farcooler_daemon::change_set::{
    BaseSource, change_set, commits_since, merge_base, working_tree, worktree_digest,
};
use tempfile::TempDir;

fn run(repo: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("git {args:?}: {e}"));
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn write(repo: &Path, path: &str, contents: &str) {
    let full = repo.join(path);
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).expect("mkdir");
    }
    std::fs::write(full, contents).expect("write");
}

/// A repository with `main` at one commit and nothing else.
fn repo() -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    run(p, &["init", "--initial-branch=main", "-q"]);
    run(p, &["config", "user.email", "test@example.com"]);
    run(p, &["config", "user.name", "Test"]);
    run(p, &["config", "commit.gpgsign", "false"]);
    write(p, "README.md", "start\n");
    run(p, &["add", "."]);
    run(p, &["commit", "-q", "-m", "initial"]);
    dir
}

#[tokio::test]
async fn the_range_is_merge_base_to_head_and_never_the_symmetric_difference() {
    // The trap this test exists for: `git log base...HEAD` is the SYMMETRIC
    // difference and includes commits only `main` has. A reviewer would see work
    // this branch never did, attributed to it, with nothing looking wrong.
    let dir = repo();
    let p = dir.path();

    run(p, &["checkout", "-q", "-b", "feature"]);
    write(p, "feature.txt", "mine\n");
    run(p, &["add", "."]);
    run(p, &["commit", "-q", "-m", "a commit the branch made"]);

    // main moves on independently.
    run(p, &["checkout", "-q", "main"]);
    write(p, "main.txt", "theirs\n");
    run(p, &["add", "."]);
    run(p, &["commit", "-q", "-m", "a commit ONLY main made"]);
    run(p, &["checkout", "-q", "feature"]);

    let base = merge_base(p, "main").await.expect("merge base resolves");
    let commits = commits_since(p, &base).await.expect("log");

    let subjects: Vec<&str> = commits.iter().map(|c| c.subject.as_str()).collect();
    assert_eq!(
        subjects,
        vec!["a commit the branch made"],
        "main's own commit must not appear in this branch's change set"
    );
}

#[tokio::test]
async fn commits_arrive_oldest_first_with_their_message_intact() {
    let dir = repo();
    let p = dir.path();
    run(p, &["checkout", "-q", "-b", "feature"]);

    for (i, subject) in ["first", "second", "third"].iter().enumerate() {
        write(p, &format!("f{i}.txt"), "x\n");
        run(p, &["add", "."]);
        run(p, &["commit", "-q", "-m", subject, "-m", "a body line"]);
    }

    let base = merge_base(p, "main").await.expect("merge base");
    let commits = commits_since(p, &base).await.expect("log");

    let subjects: Vec<&str> = commits.iter().map(|c| c.subject.as_str()).collect();
    assert_eq!(subjects, vec!["first", "second", "third"], "oldest first");
    assert!(commits[0].body.contains("a body line"));
    assert!(!commits[0].sha.is_empty());
    assert!(commits[0].timestamp > 0);
}

#[tokio::test]
async fn a_multi_line_commit_body_does_not_break_the_record_split() {
    let dir = repo();
    let p = dir.path();
    run(p, &["checkout", "-q", "-b", "feature"]);
    write(p, "x.txt", "x\n");
    run(p, &["add", "."]);
    run(p, &["commit", "-q", "-m", "subject", "-m", "one\n\ntwo\n\nthree"]);

    let base = merge_base(p, "main").await.expect("merge base");
    let commits = commits_since(p, &base).await.expect("log");
    assert_eq!(commits.len(), 1, "blank lines in a body are not record boundaries");
    assert!(commits[0].body.contains("three"));
}

#[tokio::test]
async fn a_commit_carries_its_own_counts_and_not_the_branch_total() {
    // The bug this exists for: `files_changed`, `insertions` and `deletions`
    // were hardcoded zeroes, so a history row could say nothing about its own
    // size and both clients summed the file list of whichever commit was open.
    // A branch of two commits is the smallest shape that catches a fix which
    // fills the fields with the BRANCH total instead of each commit's own.
    let dir = repo();
    let p = dir.path();
    run(p, &["checkout", "-q", "-b", "feature"]);

    write(p, "a.txt", "one\ntwo\nthree\n");
    run(p, &["add", "."]);
    run(p, &["commit", "-q", "-m", "adds three lines"]);

    write(p, "README.md", "rewritten\n");
    write(p, "b.txt", "only line\n");
    run(p, &["add", "."]);
    run(p, &["commit", "-q", "-m", "two files, one of them a rewrite"]);

    let base = merge_base(p, "main").await.expect("merge base");
    let commits = commits_since(p, &base).await.expect("log");
    assert_eq!(commits.len(), 2);

    assert_eq!(
        (commits[0].files_changed, commits[0].insertions, commits[0].deletions),
        (1, 3, 0)
    );
    assert_eq!(
        (commits[1].files_changed, commits[1].insertions, commits[1].deletions),
        (2, 2, 1),
        "the README rewrite is one line in and one line out, plus the new file"
    );
}

#[tokio::test]
async fn a_merge_is_counted_against_its_first_parent_and_never_as_a_combined_diff() {
    // A merge prints NO stat under a plain `git log`, which is how these counts
    // would silently stay zero for exactly the commits a reviewer most wants
    // sized. First parent is also what `DiffSelector` documents and what the
    // file list for this commit is computed against, so the row and the list it
    // opens have to agree.
    let dir = repo();
    let p = dir.path();

    run(p, &["checkout", "-q", "-b", "feature"]);
    write(p, "feature.txt", "mine\n");
    run(p, &["add", "."]);
    run(p, &["commit", "-q", "-m", "a commit the branch made"]);

    run(p, &["checkout", "-q", "main"]);
    write(p, "theirs.txt", "one\ntwo\n");
    run(p, &["add", "."]);
    run(p, &["commit", "-q", "-m", "a commit only main made"]);

    run(p, &["checkout", "-q", "feature"]);
    run(p, &["merge", "-q", "--no-ff", "-m", "merge main", "main"]);

    let base = merge_base(p, "main").await.expect("merge base");
    let commits = commits_since(p, &base).await.expect("log");
    let merge = commits.last().expect("the merge is the newest commit");
    assert_eq!(merge.subject, "merge main");
    assert_eq!(
        (merge.files_changed, merge.insertions, merge.deletions),
        (1, 2, 0),
        "what the merge brought in against its first parent: main's two lines"
    );
}

#[tokio::test]
async fn a_root_commit_reports_everything_it_added_rather_than_nothing() {
    // A parentless commit has nothing to diff against, and elsewhere in the
    // daemon that is handled by naming the empty tree explicitly. `git log`
    // already does it, and this test is what says so — a fix that assumed a
    // parent would report zeroes here and look right everywhere else.
    let dir = repo();
    let p = dir.path();

    run(p, &["checkout", "-q", "--orphan", "fresh-start"]);
    run(p, &["rm", "-q", "-rf", "."]);
    write(p, "first.txt", "one\ntwo\n");
    write(p, "second.txt", "three\n");
    run(p, &["add", "."]);
    run(p, &["commit", "-q", "-m", "the first commit of an orphan branch"]);

    // Unrelated histories, so `main..HEAD` is the whole orphan branch and there
    // is no merge base to resolve.
    let commits = commits_since(p, "main").await.expect("log");
    assert_eq!(commits.len(), 1);
    assert_eq!(
        (commits[0].files_changed, commits[0].insertions, commits[0].deletions),
        (2, 3, 0)
    );
}

#[tokio::test]
async fn a_commits_body_survives_the_stat_that_now_follows_it() {
    // The record separator moved to the FRONT of the format so that git's stat
    // block lands in a field of its own. Read the old way, a commit's diffstat
    // would be glued to the next commit's sha; read carelessly the new way, it
    // would be glued to this commit's body.
    let dir = repo();
    let p = dir.path();
    run(p, &["checkout", "-q", "-b", "feature"]);
    write(p, "x.txt", "x\n");
    run(p, &["add", "."]);
    run(p, &["commit", "-q", "-m", "subject", "-m", "one\n\ntwo"]);

    let base = merge_base(p, "main").await.expect("merge base");
    let commits = commits_since(p, &base).await.expect("log");
    assert_eq!(commits.len(), 1);
    assert!(commits[0].sha.chars().all(|c| c.is_ascii_hexdigit()), "a sha and nothing else");
    assert_eq!(commits[0].body, "one\n\ntwo", "no diffstat in the body");
    assert_eq!(commits[0].files_changed, 1);
}

#[tokio::test]
async fn an_unresolvable_base_is_refused_rather_than_guessed() {
    let dir = repo();
    assert!(
        merge_base(dir.path(), "no-such-branch").await.is_err(),
        "a base that does not resolve must not silently become HEAD"
    );
}

#[tokio::test]
async fn the_four_working_tree_groups_stay_apart() {
    let dir = repo();
    let p = dir.path();

    write(p, "staged.txt", "staged\n");
    run(p, &["add", "staged.txt"]);

    write(p, "README.md", "changed but not staged\n");
    write(p, "untracked.txt", "new\n");

    let wt = working_tree(p).await.expect("status");
    assert!(wt.is_dirty());
    assert!(wt.staged.iter().any(|f| f.path == "staged.txt"));
    assert!(wt.unstaged.iter().any(|f| f.path == "README.md"));
    assert!(wt.untracked.iter().any(|f| f.path == "untracked.txt"));
    assert!(wt.conflicted.is_empty());
}

#[tokio::test]
async fn every_uncommitted_group_arrives_with_its_own_counts() {
    // The defect this replaces: the Mac summed the diffs it had already drawn
    // to get an "Uncommitted" total, so the header started near zero and climbed
    // as the reader scrolled. These are the numbers it reads instead.
    let dir = repo();
    let p = dir.path();

    write(p, "staged.txt", "one\ntwo\n");
    run(p, &["add", "staged.txt"]);
    write(p, "README.md", "start\nand a second line\n");
    write(p, "new.txt", "a\nb\nc\n");

    let cs = change_set(p, "main", "main", BaseSource::Guessed).await.expect("change set");
    let wt = &cs.working_tree;

    let staged = wt.staged.iter().find(|f| f.path == "staged.txt").expect("staged");
    assert_eq!((staged.insertions, staged.deletions), (2, 0), "index against HEAD");

    let unstaged = wt.unstaged.iter().find(|f| f.path == "README.md").expect("unstaged");
    assert_eq!((unstaged.insertions, unstaged.deletions), (1, 0), "worktree against index");

    let untracked = wt.untracked.iter().find(|f| f.path == "new.txt").expect("untracked");
    assert_eq!(
        (untracked.insertions, untracked.deletions),
        (3, 0),
        "a file git has never seen still has lines in it"
    );
    assert!(!untracked.binary);

    // What a client's Uncommitted header is: the sum of every dirty path.
    let total: u32 = wt
        .staged
        .iter()
        .chain(wt.unstaged.iter())
        .chain(wt.untracked.iter())
        .map(|f| f.insertions)
        .sum();
    assert_eq!(total, 6);
}

#[tokio::test]
async fn a_file_staged_and_edited_again_reports_both_halves_apart() {
    // The one shape where the two groups are not a partition: the file is in
    // both, and each carries ITS OWN diff rather than a share of `git diff HEAD`.
    let dir = repo();
    let p = dir.path();

    write(p, "README.md", "start\nstaged line\n");
    run(p, &["add", "README.md"]);
    write(p, "README.md", "start\nstaged line\nand another\n");

    let cs = change_set(p, "main", "main", BaseSource::Guessed).await.expect("change set");
    let staged = cs.working_tree.staged.iter().find(|f| f.path == "README.md").expect("staged");
    let unstaged =
        cs.working_tree.unstaged.iter().find(|f| f.path == "README.md").expect("unstaged");
    assert_eq!((staged.insertions, staged.deletions), (1, 0));
    assert_eq!((unstaged.insertions, unstaged.deletions), (1, 0));
}

#[tokio::test]
async fn an_untracked_binary_file_is_flagged_rather_than_counted() {
    // git prints `-` for a file it will not diff, and those lines appear in no
    // total it reports. A byte count dressed up as insertions would be a lie
    // that summed.
    let dir = repo();
    let p = dir.path();
    std::fs::write(p.join("icon.png"), [0x89, b'P', b'N', b'G', 0, 0, 0, b'\n']).expect("write");

    let cs = change_set(p, "main", "main", BaseSource::Guessed).await.expect("change set");
    let icon = cs.working_tree.untracked.iter().find(|f| f.path == "icon.png").expect("untracked");
    assert!(icon.binary);
    assert_eq!(icon.insertions, 0);
}

/// The worked example the three surfaces are specified by.
///
/// Base `main`, one commit adding two lines, one further uncommitted line in
/// that same file, and one new untracked file of three lines. The sidebar and
/// the panel answer DIFFERENT questions on purpose, and this pins all of them at
/// once so that fixing one cannot quietly move another:
///
/// ```text
///   sidebar             +6   is this worth opening      all work
///   panel · Branch      +2   what lands when it merges  committed only
///   panel · Uncommitted +4   what is not committed yet  1 tracked + 3 untracked
/// ```
///
/// Uncommitted is the one of the three that moved when these counts arrived,
/// and it moved for the reason the group exists: the untracked file's three
/// lines appeared in no panel total at all, while its row sat in the Uncommitted
/// list. The other two are untouched, which is the property worth a test — the
/// sidebar and the panel are not being equalized, they are being told apart.
#[tokio::test]
async fn the_sidebar_and_the_panel_keep_answering_their_own_questions() {
    let dir = repo();
    let p = dir.path();
    run(p, &["checkout", "-q", "-b", "feature"]);
    write(p, "a.txt", "one\ntwo\n");
    run(p, &["add", "."]);
    run(p, &["commit", "-q", "-m", "add a"]);
    write(p, "a.txt", "one\ntwo\nthree\n");
    write(p, "new.txt", "x\ny\nz\n");

    let (_files, sidebar, _del) =
        farcooler_daemon::change_set::shortstat(p, "main").await.expect("shortstat");
    assert_eq!(sidebar, 6, "committed, uncommitted, and untracked together");

    let cs = change_set(p, "feature", "main", BaseSource::Guessed).await.expect("change set");
    assert_eq!(cs.insertions, 2, "the branch's own commits, and nothing else");

    let uncommitted: u32 = cs
        .working_tree
        .staged
        .iter()
        .chain(cs.working_tree.unstaged.iter())
        .chain(cs.working_tree.untracked.iter())
        .map(|f| f.insertions)
        .sum();
    assert_eq!(uncommitted, 4, "one line in a.txt and the whole of new.txt");
}

#[tokio::test]
async fn a_clean_worktree_still_reports_no_counts() {
    let dir = repo();
    let cs = change_set(dir.path(), "main", "main", BaseSource::Guessed)
        .await
        .expect("change set");
    assert!(!cs.is_dirty());
    assert!(cs.working_tree.staged.is_empty() && cs.working_tree.unstaged.is_empty());
}

#[tokio::test]
async fn a_change_set_counts_what_the_branch_changed_and_not_the_base() {
    let dir = repo();
    let p = dir.path();
    run(p, &["checkout", "-q", "-b", "feature"]);
    write(p, "a.txt", "one\ntwo\nthree\n");
    run(p, &["add", "."]);
    run(p, &["commit", "-q", "-m", "add a"]);

    let cs = change_set(p, "feature", "main", BaseSource::Guessed).await.expect("change set");
    assert_eq!(cs.branch, "feature");
    assert_eq!(cs.commits.len(), 1);
    assert_eq!(cs.insertions, 3);
    assert_eq!(cs.deletions, 0);
    assert!(cs.files.iter().any(|f| f.path == "a.txt"));
    assert!(!cs.is_dirty(), "everything was committed");
}

#[tokio::test]
async fn a_rename_is_reported_as_one_file_with_both_names() {
    let dir = repo();
    let p = dir.path();
    write(p, "before.txt", "content that is long enough to be detected as a rename\n");
    run(p, &["add", "."]);
    run(p, &["commit", "-q", "-m", "add before"]);

    run(p, &["checkout", "-q", "-b", "feature"]);
    run(p, &["mv", "before.txt", "after.txt"]);
    run(p, &["commit", "-q", "-m", "rename"]);

    let cs = change_set(p, "feature", "main", BaseSource::Guessed).await.expect("change set");
    let renamed = cs.files.iter().find(|f| f.path == "after.txt").expect("the new name");
    assert_eq!(renamed.old_path.as_deref(), Some("before.txt"));
}

#[tokio::test]
async fn the_digest_ignores_an_mtime_bump_that_changed_no_content() {
    // Rewriting a file with identical bytes must not mark a whole review stale.
    let dir = repo();
    let p = dir.path();
    write(p, "x.txt", "same\n");

    let head = "HEADSHA";
    let before = worktree_digest(p, head).await.expect("digest");
    std::thread::sleep(std::time::Duration::from_millis(20));
    write(p, "x.txt", "same\n");
    let after = worktree_digest(p, head).await.expect("digest");

    assert_eq!(before, after, "identical content must produce an identical digest");
}

#[tokio::test]
async fn the_digest_moves_when_an_already_modified_file_is_modified_again() {
    // THE case an earlier design missed. Porcelain output alone is identical
    // across both edits below — the file is "modified" before and after — so a
    // digest built from status text would never move, and every unanchored
    // comment (most of them) would sit there claiming to be current.
    let dir = repo();
    let p = dir.path();

    write(p, "README.md", "first edit\n");
    let head = "HEADSHA";
    let once = worktree_digest(p, head).await.expect("digest");

    write(p, "README.md", "second edit\n");
    let twice = worktree_digest(p, head).await.expect("digest");

    assert_ne!(once, twice, "a second edit to an already-dirty file must move the digest");
}

#[tokio::test]
async fn the_digest_moves_when_head_moves_even_with_a_clean_tree() {
    let dir = repo();
    let p = dir.path();
    let a = worktree_digest(p, "sha-one").await.expect("digest");
    let b = worktree_digest(p, "sha-two").await.expect("digest");
    assert_ne!(a, b);
}

#[tokio::test]
async fn a_new_untracked_file_moves_the_digest() {
    let dir = repo();
    let p = dir.path();
    let before = worktree_digest(p, "head").await.expect("digest");
    write(p, "brand-new.txt", "hello\n");
    let after = worktree_digest(p, "head").await.expect("digest");
    assert_ne!(before, after, "an agent creating a file is a change worth re-reading");
}

#[tokio::test]
async fn shortstat_gives_the_three_numbers_a_sidebar_row_needs() {
    let dir = repo();
    let p = dir.path();
    run(p, &["checkout", "-q", "-b", "feature"]);
    write(p, "a.txt", "one\ntwo\nthree\n");
    run(p, &["add", "."]);
    run(p, &["commit", "-q", "-m", "add a"]);
    write(p, "README.md", "changed\n");
    run(p, &["commit", "-qam", "change readme"]);

    let (files, ins, del) =
        farcooler_daemon::change_set::shortstat(p, "main").await.expect("shortstat");
    assert_eq!(files, 2);
    assert_eq!(ins, 4, "three new lines plus the readme rewrite");
    assert_eq!(del, 1, "and git says `1 deletion(-)`, singular");
}

#[tokio::test]
async fn shortstat_counts_work_that_has_not_been_committed_yet() {
    // The whole point of the sidebar's `+N -M`: it must climb as an agent edits,
    // not only when it commits. `git diff <base> HEAD` answered the second
    // question, so a row sat still through a twenty-minute turn and then jumped.
    let dir = repo();
    let p = dir.path();
    run(p, &["checkout", "-q", "-b", "feature"]);
    write(p, "a.txt", "one\ntwo\nthree\n");
    run(p, &["add", "."]);
    run(p, &["commit", "-q", "-m", "add a"]);

    // Committed only, so far.
    let committed = farcooler_daemon::change_set::shortstat(p, "main").await.expect("shortstat");
    assert_eq!(committed, (1, 3, 0));

    // An edit nobody has committed, and a file nobody has staged.
    write(p, "a.txt", "one\ntwo\nthree\nfour\n");
    let dirty = farcooler_daemon::change_set::shortstat(p, "main").await.expect("shortstat");
    assert_eq!(dirty, (1, 4, 0), "an uncommitted line counts the moment it lands");

    // And staged-but-uncommitted, which `git diff <base>` already sees because
    // the file is in the index. It must not be counted a second time.
    run(p, &["add", "a.txt"]);
    let staged = farcooler_daemon::change_set::shortstat(p, "main").await.expect("shortstat");
    assert_eq!(staged, (1, 4, 0), "staging changes nothing about the count");
}

#[tokio::test]
async fn shortstat_counts_a_file_that_has_only_just_been_created() {
    // The failure that would be worse than the committed-only number this
    // replaces: a count that climbs while an agent edits an existing file and
    // sits still while it writes a new one. It LOOKS live, so nobody checks it.
    // `git diff` cannot see an untracked file at all; `untracked_lines` is why
    // this passes.
    let dir = repo();
    let p = dir.path();
    run(p, &["checkout", "-q", "-b", "feature"]);

    write(p, "brand-new.rs", "fn main() {}\n");
    let (files, ins, del) =
        farcooler_daemon::change_set::shortstat(p, "main").await.expect("shortstat");
    assert_eq!((files, ins, del), (1, 1, 0), "a created file is one file and its lines");

    // A last line with no newline after it is still a line — git reports it as
    // an insertion and marks the hunk `\ No newline at end of file`.
    write(p, "no-eol.txt", "one\ntwo");
    let (files, ins, _) =
        farcooler_daemon::change_set::shortstat(p, "main").await.expect("shortstat");
    assert_eq!((files, ins), (2, 3));

    // Ignored files are not work in progress, and a `target/` full of build
    // output must never land in a sidebar row.
    write(p, ".gitignore", "ignored/\n");
    run(p, &["add", ".gitignore"]);
    run(p, &["commit", "-q", "-m", "ignore"]);
    write(p, "ignored/huge.txt", "a\nb\nc\nd\ne\n");
    let (files, ins, _) =
        farcooler_daemon::change_set::shortstat(p, "main").await.expect("shortstat");
    assert_eq!((files, ins), (3, 4), "the gitignore commit itself, and nothing under it");
}

#[tokio::test]
async fn shortstat_does_not_count_the_lines_of_a_new_binary_file() {
    // git prints `-` rather than a count for a file it calls binary, and those
    // lines appear in no total it reports. An agent that drops a 2MB PNG into a
    // worktree must not read as fifty thousand insertions.
    let dir = repo();
    let p = dir.path();
    run(p, &["checkout", "-q", "-b", "feature"]);
    std::fs::write(p.join("icon.png"), [0x89, b'P', b'N', b'G', 0x00, 0x0a, 0x0a, 0x00])
        .expect("write");

    let (files, ins, del) =
        farcooler_daemon::change_set::shortstat(p, "main").await.expect("shortstat");
    assert_eq!((files, ins, del), (1, 0, 0), "counted as a file, never as lines");
}

#[tokio::test]
async fn shortstat_works_in_a_linked_worktree_which_is_the_only_kind_this_product_makes() {
    // Every workspace is a `git worktree add`, never the main checkout, so the
    // main-worktree case above is the one that never happens in production.
    let dir = repo();
    let p = dir.path();
    let linked = dir.path().parent().unwrap().join("linked-wt");

    run(p, &["worktree", "add", "-b", "feature", linked.to_str().unwrap()]);
    write(&linked, "a.txt", "one\ntwo\nthree\n");
    run(&linked, &["add", "."]);
    run(&linked, &["commit", "-q", "-m", "add a"]);

    let (files, ins, del) = farcooler_daemon::change_set::shortstat(&linked, "main")
        .await
        .expect("shortstat in a linked worktree");
    assert_eq!((files, ins, del), (1, 3, 0));

    let _ = std::fs::remove_dir_all(&linked);
}

#[tokio::test]
async fn a_repository_on_master_resolves_a_base_instead_of_showing_nothing() {
    // The bug this exists for: `main` was hardcoded, so a repository on `master`
    // resolved no merge base and review showed an empty diff with no reason —
    // "nothing changed" and "I could not work out what to compare against"
    // looked identical.
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    run(p, &["init", "--initial-branch=master", "-q"]);
    run(p, &["config", "user.email", "test@example.com"]);
    run(p, &["config", "user.name", "Test"]);
    run(p, &["config", "commit.gpgsign", "false"]);
    write(p, "README.md", "start\n");
    run(p, &["add", "."]);
    run(p, &["commit", "-q", "-m", "initial"]);

    let base = farcooler_daemon::change_set::guess_base(p).await;
    assert_eq!(base.as_deref(), Some("master"));
}

#[tokio::test]
async fn a_repository_on_main_still_resolves_main() {
    let dir = repo();
    let base = farcooler_daemon::change_set::guess_base(dir.path()).await;
    assert_eq!(base.as_deref(), Some("main"));
}

#[tokio::test]
async fn origin_head_wins_over_a_local_main_because_it_is_what_the_remote_calls_default() {
    let dir = repo();
    let p = dir.path();
    // A repository whose remote default is `trunk`, with a local `main` that is
    // not it. Guessing `main` here would diff against the wrong branch and look
    // entirely plausible.
    run(p, &["branch", "trunk"]);
    run(p, &["update-ref", "refs/remotes/origin/trunk", "refs/heads/trunk"]);
    run(p, &["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/trunk"]);

    let base = farcooler_daemon::change_set::guess_base(p).await;
    assert_eq!(base.as_deref(), Some("origin/trunk"));
}

#[tokio::test]
async fn a_repository_with_no_main_no_master_and_no_remote_resolves_nothing() {
    // Nothing is the honest answer, and it is what lets the caller say "pick a
    // base" rather than silently diffing against a ref that is not there.
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    run(p, &["init", "--initial-branch=develop", "-q"]);
    run(p, &["config", "user.email", "test@example.com"]);
    run(p, &["config", "user.name", "Test"]);
    run(p, &["config", "commit.gpgsign", "false"]);
    write(p, "README.md", "start\n");
    run(p, &["add", "."]);
    run(p, &["commit", "-q", "-m", "initial"]);

    assert!(farcooler_daemon::change_set::guess_base(p).await.is_none());
}

#[tokio::test]
async fn a_change_set_against_master_counts_what_the_branch_changed() {
    // End to end on the shape that used to show nothing at all.
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    run(p, &["init", "--initial-branch=master", "-q"]);
    run(p, &["config", "user.email", "test@example.com"]);
    run(p, &["config", "user.name", "Test"]);
    run(p, &["config", "commit.gpgsign", "false"]);
    write(p, "README.md", "start\n");
    run(p, &["add", "."]);
    run(p, &["commit", "-q", "-m", "initial"]);

    run(p, &["checkout", "-q", "-b", "feature"]);
    write(p, "a.txt", "one\ntwo\n");
    run(p, &["add", "."]);
    run(p, &["commit", "-q", "-m", "add a"]);

    let base = farcooler_daemon::change_set::guess_base(p).await.expect("a base");
    let cs = change_set(p, "feature", &base, BaseSource::Guessed).await.expect("change set");
    assert_eq!(cs.insertions, 2);
    assert_eq!(cs.commits.len(), 1);
}
