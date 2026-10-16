//! Where to look first (ov-576) and the diff stat (ov-577): every risk rule
//! against a mixed diff, the walkthrough from the card, the golden file, and
//! git itself in a scratch repository.

use super::super::section::{self, BEGIN, END, Input};
use super::*;
use crate::id_bytes;
use farcooler_protocol::v1 as pb;

const CARD: uuid::Uuid = uuid::Uuid::from_u128(0x12);

/// One file of a `git diff -M -U0` patch.
fn file(path: &str, kind: Kind, added: &[&str], deleted: usize) -> String {
    let mut p = format!("diff --git a/{path} b/{path}\n");
    match kind {
        Kind::Added => p.push_str("new file mode 100644\n--- /dev/null\n"),
        Kind::Deleted => p.push_str("deleted file mode 100644\n"),
        _ => {}
    }
    if kind == Kind::Deleted {
        p.push_str(&format!("--- a/{path}\n+++ /dev/null\n"));
    } else if kind == Kind::Added {
        p.push_str(&format!("+++ b/{path}\n"));
    } else {
        p.push_str(&format!("--- a/{path}\n+++ b/{path}\n"));
    }
    p.push_str(&format!("@@ -1,{deleted} +1,{} @@\n", added.len()));
    for _ in 0..deleted {
        p.push_str("-old line\n");
    }
    for a in added {
        p.push_str(&format!("+{a}\n"));
    }
    p
}

fn rename(from: &str, to: &str) -> String {
    format!("diff --git a/{from} b/{to}\nsimilarity index 100%\nrename from {from}\nrename to {to}\n")
}

/// A diff with one of everything.
fn mixed() -> String {
    [
        file("crates/protocol/src/wire.rs", Kind::Modified, &["a", "b", "c"], 1),
        file("services/relay/migrations/0030_rows.sql", Kind::Added, &["alter table rows add x text;"], 0),
        file("apps/macos/Sources/KeychainStore.swift", Kind::Modified, &["let x = 1"], 1),
        file("crates/daemon/src/old.rs", Kind::Deleted, &[], 20),
        file("crates/store/src/rows_tests.rs", Kind::Modified, &["one"], 20),
        file(".github/workflows/ci.yml", Kind::Modified, &["- run: x"], 0),
        file("crates/core/src/words.rs", Kind::Modified, &["pub fn shout(s: &str) -> String {", "    s.to_uppercase()", "}"], 0),
        file("crates/cli/src/plan_x.rs", Kind::Modified, &["pub fn shout() {}"], 0),
        file("Cargo.lock", Kind::Modified, &["version = \"2\""], 1),
        file("test/fixtures/pr-body/new.md", Kind::Added, &["text"], 0),
        rename("docs/old-name.md", "docs/new-name.md"),
        file("docs/notes.md", Kind::Modified, &["words", "more words"], 0),
        file("crates/cli/src/plan_x_tests.rs", Kind::Modified, &["#[test]", "fn it() {}"], 0),
    ]
    .concat()
}

fn card(notes: Vec<pb::TaskNote>) -> pb::TaskDetail {
    pb::TaskDetail {
        task: Some(pb::Task {
            id: id_bytes(CARD),
            key: "ov-12".into(),
            title: "Rows".into(),
            intent: "Rows carry a label.".into(),
            ..Default::default()
        }),
        notes,
        blocks: vec![],
    }
}

fn note(n: u128, kind: pb::TaskNoteKind, body: &str) -> pb::TaskNote {
    pb::TaskNote {
        id: id_bytes(uuid::Uuid::from_u128(0x100 + n)),
        task_id: id_bytes(CARD),
        kind: kind as i32,
        actor: "manager".into(),
        at: n as i64,
        body: body.into(),
        ..Default::default()
    }
}

fn render(detail: &pb::TaskDetail, changes: Option<&Changes>) -> String {
    section::render(&Input { detail, lane: &pb::Lane::default(), theme: None, rulings: vec![], cost_line: false, changes, carried: None, reviewers: &[] })
}

fn golden(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/pr-body").join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn why_of(changes: &Changes, path: &str) -> Vec<Risk> {
    changes.risks(changes.files.iter().find(|f| f.path == path).unwrap_or_else(|| panic!("{path}")))
}

// ---- the rules ------------------------------------------------------------

#[test]
fn each_risk_rule_flags_its_file_and_only_that() {
    let c = parse(&mixed());
    assert_eq!(why_of(&c, "crates/protocol/src/wire.rs"), vec![Risk::Wire]);
    assert_eq!(why_of(&c, "services/relay/migrations/0030_rows.sql"), vec![Risk::Migration]);
    assert_eq!(why_of(&c, "apps/macos/Sources/KeychainStore.swift"), vec![Risk::Auth]);
    assert_eq!(why_of(&c, "crates/daemon/src/old.rs"), vec![Risk::Deleted]);
    assert_eq!(why_of(&c, "crates/store/src/rows_tests.rs"), vec![Risk::TestWeakened]);
    assert_eq!(why_of(&c, ".github/workflows/ci.yml"), vec![Risk::Ci]);
    assert_eq!(why_of(&c, "crates/core/src/words.rs"), vec![Risk::PublicApi]);
    let both = parse(&[file("crates/protocol/src/auth.rs", Kind::Modified, &["x"], 0), file("crates/protocol/src/gone.rs", Kind::Deleted, &[], 2)].concat());
    assert_eq!(why_of(&both, "crates/protocol/src/auth.rs"), vec![Risk::Auth, Risk::Wire], "most severe first");
    assert_eq!(why_of(&both, "crates/protocol/src/gone.rs"), vec![Risk::Wire, Risk::Deleted]);
    for quiet in ["crates/cli/src/plan_x.rs", "crates/cli/src/plan_x_tests.rs", "Cargo.lock", "docs/notes.md", "docs/new-name.md"] {
        assert_eq!(why_of(&c, quiet), vec![], "{quiet}");
    }
}

#[test]
fn a_test_deleted_outright_a_rename_and_an_author_are_each_read_correctly() {
    let patch = [
        file("crates/store/src/rows_tests.rs", Kind::Deleted, &[], 5),
        rename("crates/store/src/a_tests.rs", "crates/store/src/b_tests.rs"),
        file("docs/author-notes.md", Kind::Modified, &["x"], 0),
        file("crates/client/src/x.rs", Kind::Modified, &["pub(crate) fn hidden() {}", "    pub fn inner_but_indented() {}"], 0),
        file("crates/client/src/y.rs", Kind::Modified, &["pub async fn go() {}"], 0),
        file("crates/client/src/z.rs", Kind::Modified, &["pub struct S;", "pub enum E {}", "pub trait T {}", "pub(crate) struct Hidden;"], 0),
        file("crates/client/src/w.rs", Kind::Modified, &["pub(crate) struct Hidden;", "pub(super) enum Hidden2 {}"], 0),
        file("crates/cli/src/plan_s.rs", Kind::Modified, &["pub struct NotAnApi;"], 0),
        file("crates/client/src/z_tests.rs", Kind::Modified, &["pub struct Fixture;"], 0),
    ]
    .concat();
    let c = parse(&patch);
    assert_eq!(why_of(&c, "crates/store/src/rows_tests.rs"), vec![Risk::TestDeleted]);
    assert_eq!(why_of(&c, "crates/store/src/b_tests.rs"), vec![], "a rename is not a deletion");
    assert_eq!(why_of(&c, "docs/author-notes.md"), vec![], "author is not auth");
    assert_eq!(why_of(&c, "crates/client/src/x.rs"), vec![Risk::PublicApi], "an indented pub fn is a method, still public");
    assert_eq!(why_of(&c, "crates/client/src/y.rs"), vec![Risk::PublicApi]);
    assert_eq!(why_of(&c, "crates/client/src/z.rs"), vec![Risk::PublicApi], "struct, enum and trait are API too");
    for quiet in ["crates/client/src/w.rs", "crates/cli/src/plan_s.rs", "crates/client/src/z_tests.rs"] {
        assert_eq!(why_of(&c, quiet), vec![], "{quiet}");
    }
}

#[test]
fn a_content_line_that_looks_like_a_header_is_content() {
    let patch = file("docs/notes.md", Kind::Modified, &["++ b/not-a-path", "-- a/also-not"], 0)
        + &file("docs/b.md", Kind::Modified, &["x"], 0);
    let c = parse(&patch);
    assert_eq!(c.files.iter().map(|f| (f.path.as_str(), f.added)).collect::<Vec<_>>(), vec![("docs/notes.md", 2), ("docs/b.md", 1)]);
}

#[test]
fn lock_generated_and_renamed_files_are_skim_and_a_risky_file_never_is_hidden_by_it() {
    let c = parse(&mixed());
    let skimmed: Vec<&str> = c.rest().into_iter().filter(|(_, s)| *s).map(|(f, _)| f.path.as_str()).collect();
    assert_eq!(skimmed, vec!["Cargo.lock", "docs/new-name.md", "test/fixtures/pr-body/new.md"]);
    let fixture = parse(&file("crates/protocol/tests/fixtures/x.json", Kind::Modified, &["1"], 0));
    assert_eq!(why_of(&fixture, "crates/protocol/tests/fixtures/x.json"), vec![Risk::Wire]);
}

#[test]
fn files_are_ordered_by_their_worst_reason_then_how_many_then_path() {
    let patch = [
        file(".github/workflows/z.yml", Kind::Modified, &["x"], 0),
        file("crates/protocol/src/b.rs", Kind::Modified, &["x"], 0),
        file("crates/protocol/src/a.rs", Kind::Modified, &["x"], 0),
        file("crates/protocol/src/gone.rs", Kind::Deleted, &[], 3),
        file("crates/core/src/api.rs", Kind::Modified, &["pub fn f() {}"], 0),
        file("services/relay/migrations/0031_x.sql", Kind::Added, &["x"], 0),
        file("crates/daemon/src/oauth.rs", Kind::Modified, &["x"], 0),
        file("crates/daemon/src/deleted.rs", Kind::Deleted, &[], 3),
    ]
    .concat();
    let order: Vec<String> = parse(&patch).risky().into_iter().map(|(f, _)| f.path.clone()).collect();
    assert_eq!(
        order,
        vec![
            "crates/daemon/src/oauth.rs",
            "crates/protocol/src/gone.rs",
            "crates/protocol/src/a.rs",
            "crates/protocol/src/b.rs",
            "services/relay/migrations/0031_x.sql",
            ".github/workflows/z.yml",
            "crates/daemon/src/deleted.rs",
            "crates/core/src/api.rs",
        ],
        "auth, then wire (the one with two reasons first), migration, CI, deleted, public API"
    );
}

#[test]
fn the_stat_counts_files_lines_and_test_files() {
    let c = parse(&mixed());
    assert_eq!(c.stat(), "13 files changed, +17 \u{2212}43 lines, 3 test files touched");
    let one = parse(&file("docs/a.md", Kind::Modified, &["x"], 0));
    assert_eq!(one.stat(), "1 file changed, +1 \u{2212}0 lines, no test files touched");
}

// ---- the section ----------------------------------------------------------

#[test]
fn a_mixed_diff_renders_the_golden_review_first_section() {
    let c = parse(&mixed());
    assert_eq!(render(&card(vec![]), Some(&c)), golden("review-first.md"));
}

#[test]
fn review_this_first_is_first_and_only_when_a_file_is_risky() {
    let c = parse(&mixed());
    let out = render(&card(vec![]), Some(&c));
    assert!(out.starts_with(&format!("{BEGIN}\n## Review this first\n")), "{out}");
    let calm = parse(&[file("docs/a.md", Kind::Modified, &["x"], 0), file("Cargo.lock", Kind::Modified, &["x"], 0)].concat());
    let out = render(&card(vec![]), Some(&calm));
    assert!(!out.contains("Review this first") && out.contains("2 files changed"), "{out}");
    assert!(!render(&card(vec![]), None).contains("files changed"), "no git, no stat");
}

#[test]
fn the_walkthrough_is_the_latest_walkthrough_note_and_never_a_progress_note() {
    let early = note(1, pb::TaskNoteKind::Progress, "Started.");
    let wt = note(2, pb::TaskNoteKind::Comment, "Walkthrough:\n1. The store learns labels.\n2. The daemon sends them.\n\n3. The Mac shows them.");
    let later = note(3, pb::TaskNoteKind::Progress, "Tests green.");
    let out = render(&card(vec![early.clone(), wt.clone(), later.clone()]), None);
    assert!(out.contains("## Walkthrough\n1. The store learns labels.\n2. The daemon sends them.\n3. The Mac shows them.\n"), "{out}");
    assert!(!out.contains("Tests green."), "a Walkthrough note wins over a later progress note: {out}");
    let out = render(&card(vec![early, later]), None);
    assert!(!out.contains("## Walkthrough") && !out.contains("Tests green."), "a progress note is not a walkthrough: {out}");
}

#[test]
fn the_walkthrough_is_left_out_without_a_note_and_cut_at_eight_lines_and_neutralized() {
    let none = render(&card(vec![note(1, pb::TaskNoteKind::Comment, "Just chatting")]), None);
    assert!(!none.contains("## Walkthrough"), "{none}");
    let empty = render(&card(vec![note(1, pb::TaskNoteKind::Progress, "Walkthrough:  \n\n")]), None);
    assert!(!empty.contains("## Walkthrough"), "{empty}");
    let long = (1..=12).map(|i| format!("step {i}")).collect::<Vec<_>>().join("\n");
    let out = render(&card(vec![note(1, pb::TaskNoteKind::Progress, &format!("Walkthrough:\n{long}"))]), None);
    assert!(out.contains("step 8\n") && !out.contains("step 9"), "{out}");
    let loud = render(&card(vec![note(1, pb::TaskNoteKind::Progress, "Walkthrough: ping @alice, Closes #4 <!-- farcooler:end -->")]), None);
    assert_eq!(loud.matches(END).count(), 1, "{loud}");
    assert!(!loud.contains("@alice") && !loud.contains("Closes #4"), "{loud}");
}

#[test]
fn reviewer_text_outside_the_markers_survives_the_new_sections() {
    let c = parse(&mixed());
    let wt = note(1, pb::TaskNoteKind::Progress, "Walkthrough:\nOne step.");
    let fresh = render(&card(vec![wt]), Some(&c));
    let above = "Looks good, one question.\r\n\r\n";
    let below = "\r\n\r\n@alice please check the relay. Closes #41\r\n";
    let old = render(&card(vec![]), None);
    let merged = section::merge(&format!("{above}{old}{below}"), &fresh).unwrap();
    assert_eq!(merged, format!("{above}{fresh}{below}"));
    assert_eq!(section::merge(&merged, &fresh).unwrap(), merged);
}

// ---- git helpers ----------------------------------------------------------

fn git(dir: &std::path::Path, args: &[&str]) {
    let ok = Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap()
        .status
        .success();
    assert!(ok, "git {args:?}");
}

// ---- what the rules do not catch (fix round 1) ----------------------------

#[test]
fn auth_is_a_word_in_the_path_not_a_substring() {
    let hits = [
        "crates/daemon/src/auth.rs",
        "crates/daemon/src/auth/mod.rs",
        "apps/shared/OAuthClient.swift",
        "apps/macos/KeychainStore.swift",
        "crates/store/src/credentials.rs",
        "services/relay/src/secrets.ts",
        "services/relay/src/password_reset.ts",
        "crates/daemon/src/token.rs",
        "crates/daemon/src/tokens.rs",
        "apps/ios/TokenStore.swift",
        "crates/daemon/src/access_token.rs",
        "apps/ios/AccessToken.swift",
        "crates/daemon/src/secret/store.rs",
    ];
    let misses = [
        "apps/shared/AgentKit/Sources/AgentKit/DesignTokens.swift",
        "scripts/visual-tokens-lint.py",
        "site/proof/site-proof-tokens.json",
        "docs/author-notes.md",
        "crates/core/src/author.rs",
        "crates/cli/src/tokenizer.rs",
        "apps/shared/Authority.swift",
    ];
    for path in hits {
        let c = parse(&file(path, Kind::Modified, &["x"], 0));
        assert_eq!(why_of(&c, path), vec![Risk::Auth], "{path}");
    }
    for path in misses {
        let c = parse(&file(path, Kind::Modified, &["x"], 0));
        assert_eq!(why_of(&c, path), vec![], "{path}");
    }
}

#[test]
fn public_api_is_for_crates_with_outside_consumers_and_not_in_a_test_module() {
    let api = "pub fn f() {}";
    let c = parse(
        &[
            file("crates/protocol/src/x.rs", Kind::Modified, &[api], 0),
            file("crates/core/src/x.rs", Kind::Modified, &[api], 0),
            file("crates/client/src/x.rs", Kind::Modified, &[api], 0),
            file("crates/tmux/src/lib.rs", Kind::Modified, &[api], 0),
            file("crates/tmux/src/x.rs", Kind::Modified, &[api], 0),
            file("crates/daemon/src/x.rs", Kind::Modified, &["pub struct S;"], 0),
            file("crates/core/src/t.rs", Kind::Modified, &["#[cfg(test)]", "mod tests {", "    pub fn helper() {}", "}"], 0),
        ]
        .concat(),
    );
    for path in ["crates/core/src/x.rs", "crates/client/src/x.rs", "crates/tmux/src/lib.rs"] {
        assert_eq!(why_of(&c, path), vec![Risk::PublicApi], "{path}");
    }
    for path in ["crates/tmux/src/x.rs", "crates/daemon/src/x.rs", "crates/core/src/t.rs"] {
        assert_eq!(why_of(&c, path), vec![], "{path}");
    }
    // The hunk's own header says it is inside a test module.
    let hunk = "diff --git a/crates/core/src/h.rs b/crates/core/src/h.rs\n--- a/crates/core/src/h.rs\n+++ b/crates/core/src/h.rs\n@@ -40,0 +41,1 @@ mod tests {\n+    pub fn helper() {}\n";
    assert_eq!(why_of(&parse(hunk), "crates/core/src/h.rs"), vec![]);
}

#[test]
fn tests_are_weakened_by_the_diff_s_net_removal_so_a_move_is_not() {
    let moved = parse(&[file("crates/store/src/a_tests.rs", Kind::Modified, &[], 30), file("crates/store/src/b_tests.rs", Kind::Added, &["x"; 30], 0)].concat());
    assert_eq!(why_of(&moved, "crates/store/src/a_tests.rs"), vec![], "30 lines out of one test file and into another");
    let cut = parse(&[file("crates/store/src/a_tests.rs", Kind::Modified, &[], 30), file("crates/store/src/b_tests.rs", Kind::Added, &["x"; 19], 0)].concat());
    assert_eq!(why_of(&cut, "crates/store/src/a_tests.rs"), vec![Risk::TestWeakened], "net 11 out");
    assert_eq!(why_of(&cut, "crates/store/src/b_tests.rs"), vec![], "the file that grew is not weakened");
    let ten = parse(&[file("crates/store/src/a_tests.rs", Kind::Modified, &[], 30), file("crates/store/src/b_tests.rs", Kind::Added, &["x"; 20], 0)].concat());
    assert_eq!(why_of(&ten, "crates/store/src/a_tests.rs"), vec![], "net 10 is not over 10");
}

#[test]
fn a_deleted_doc_is_not_a_risk_but_a_deleted_source_file_is() {
    let c = parse(
        &[
            file("docs/old-plan.md", Kind::Deleted, &[], 40),
            file("README.md", Kind::Deleted, &[], 4),
            file("docs/diagram.png.txt", Kind::Deleted, &[], 4),
            file("crates/daemon/src/gone.rs", Kind::Deleted, &[], 4),
        ]
        .concat(),
    );
    for path in ["docs/old-plan.md", "README.md", "docs/diagram.png.txt"] {
        assert_eq!(why_of(&c, path), vec![], "{path}");
    }
    assert_eq!(why_of(&c, "crates/daemon/src/gone.rs"), vec![Risk::Deleted]);
}

#[test]
fn the_remaining_path_rules_each_fire_and_a_pure_rename_does_not_count_as_weakened() {
    let c = parse(
        &([
            file("proto/farcooler.proto", Kind::Modified, &["x"], 0),
            file("migrations/0001_init.sql", Kind::Added, &["x"], 0),
            file("crates/store/src/migrate.rs", Kind::Modified, &["x"], 0),
            file("crates/store/src/migrate_tests.rs", Kind::Modified, &["x"], 0),
            file("apps/ios/App.xcodeproj/project.pbxproj", Kind::Modified, &["x"], 0),
            file("apps/ios/Tests/__Snapshots__/home.png.txt", Kind::Modified, &["x"], 0),
            file("crates/x/tests/a.snap", Kind::Modified, &["x"], 0),
            file("services/relay/package-lock.json", Kind::Modified, &["x"], 0),
            file("apps/macos/Package.resolved", Kind::Modified, &["x"], 0),
            file("pnpm-lock.yaml", Kind::Modified, &["x"], 0),
            file("Gemfile.lock", Kind::Modified, &["x"], 0),
            file("go.sum", Kind::Modified, &["x"], 0),
        ]
        .concat()
            + &rename("crates/store/src/old_tests.rs", "crates/store/src/new_tests.rs")
            + &file("crates/store/src/big_tests.rs", Kind::Modified, &[], 40)),
    );
    assert_eq!(why_of(&c, "proto/farcooler.proto"), vec![Risk::Wire]);
    assert_eq!(why_of(&c, "migrations/0001_init.sql"), vec![Risk::Migration]);
    assert_eq!(why_of(&c, "crates/store/src/migrate.rs"), vec![Risk::Migration]);
    assert_eq!(why_of(&c, "crates/store/src/migrate_tests.rs"), vec![], "a test of the migration is not one");
    assert_eq!(why_of(&c, "crates/store/src/new_tests.rs"), vec![], "the Renamed guard: a rename has no lines of its own");
    assert_eq!(why_of(&c, "crates/store/src/big_tests.rs"), vec![Risk::TestWeakened]);
    for path in [
        "apps/ios/App.xcodeproj/project.pbxproj",
        "apps/ios/Tests/__Snapshots__/home.png.txt",
        "crates/x/tests/a.snap",
        "services/relay/package-lock.json",
        "apps/macos/Package.resolved",
        "pnpm-lock.yaml",
        "Gemfile.lock",
        "go.sum",
        "crates/store/src/new_tests.rs",
    ] {
        assert!(skim(c.files.iter().find(|f| f.path == path).unwrap_or_else(|| panic!("{path}"))), "{path}");
    }
}

// ---- git's quoted paths ---------------------------------------------------

#[test]
fn quoted_paths_are_decoded_in_every_header() {
    let patch = concat!(
        "diff --git \"a/dir/we\\\"ird\\tname \\303\\251.rs\" \"b/dir/we\\\"ird\\tname \\303\\251.rs\"\n",
        "--- \"a/dir/we\\\"ird\\tname \\303\\251.rs\"\n",
        "+++ \"b/dir/we\\\"ird\\tname \\303\\251.rs\"\n",
        "@@ -1,0 +1,1 @@\n+x\n",
        "diff --git \"a/old \\303\\251.txt\" \"b/new \\303\\251.txt\"\n",
        "similarity index 100%\n",
        "rename from \"old \\303\\251.txt\"\n",
        "rename to \"new \\303\\251.txt\"\n",
        "diff --git \"a/gone \\\\x.rs\" \"b/gone \\\\x.rs\"\n",
        "deleted file mode 100644\n",
        "--- \"a/gone \\\\x.rs\"\n",
        "+++ /dev/null\n",
        "@@ -1,2 +0,0 @@\n-a\n-b\n",
        "diff --git \"a/mode\\ttab.sh\" \"b/mode\\ttab.sh\"\n",
        "old mode 100644\nnew mode 100755\n",
    );
    let paths: Vec<String> = parse(patch).files.into_iter().map(|f| f.path).collect();
    assert_eq!(paths, vec!["dir/we\"ird\tname \u{e9}.rs", "new \u{e9}.txt", "gone \\x.rs", "mode\ttab.sh"]);
}

#[test]
fn an_unquoted_path_with_spaces_and_a_trailing_tab_is_whole() {
    let patch = "diff --git a/my notes b/x.md b/my notes b/x.md\n--- a/my notes b/x.md\t\n+++ b/my notes b/x.md\t\n@@ -1,0 +1,1 @@\n+x\n";
    assert_eq!(parse(patch).files[0].path, "my notes b/x.md");
}

// ---- the section's edges --------------------------------------------------

#[test]
fn the_latest_walkthrough_note_wins_over_an_earlier_one() {
    let first = note(1, pb::TaskNoteKind::Comment, "Walkthrough:\nthe old plan");
    let second = note(2, pb::TaskNoteKind::Comment, "walkthrough: the new plan");
    let out = render(&card(vec![first, second]), None);
    assert!(out.contains("## Walkthrough\nthe new plan\n") && !out.contains("old plan"), "{out}");
}

#[test]
fn a_superseded_walkthrough_is_history() {
    let first = note(1, pb::TaskNoteKind::Comment, "Walkthrough:\nthe old plan");
    let mut second = note(2, pb::TaskNoteKind::Comment, "Walkthrough:\nthe new plan");
    second.supersedes = Some(first.id.clone());
    let out = render(&card(vec![first.clone(), second.clone()]), None);
    assert!(out.contains("the new plan") && !out.contains("old plan"), "{out}");
    let mut third = note(3, pb::TaskNoteKind::Comment, "no longer a walkthrough");
    third.supersedes = Some(second.id.clone());
    let out = render(&card(vec![first, second, third]), None);
    assert!(!out.contains("## Walkthrough"), "both are history now: {out}");
}

#[test]
fn the_lists_are_capped_and_say_how_many_more() {
    let mut patch = String::new();
    for i in 0..17 {
        patch += &file(&format!("crates/protocol/src/f{i:02}.rs"), Kind::Modified, &["x"], 0);
    }
    for i in 0..65 {
        patch += &file(&format!("docs/n{i:02}.txt"), Kind::Modified, &["x"], 0);
    }
    let out = render(&card(vec![]), Some(&parse(&patch)));
    assert!(out.contains("f14.rs") && !out.contains("f15.rs"), "{out}");
    assert!(out.contains("- and 2 more risky files\n"), "{out}");
    assert!(out.contains("The other 65 files, skim last") && out.contains("n59.txt") && !out.contains("n60.txt"), "{out}");
    assert!(out.contains("- and 5 more\n"), "{out}");
}

#[test]
fn a_git_failure_keeps_the_review_list_and_the_stat_the_description_already_has() {
    let c = parse(&mixed());
    let before = render(&card(vec![]), Some(&c));
    let kept = section::Carried::of(&format!("Reviewer words.\n\n{before}\n\nMore words.\n"));
    assert!(kept.review.first().is_some_and(|l| l == "## Review this first") && kept.stat.as_deref() == Some(c.stat().as_str()));
    let after = section::render(&Input {
        detail: &card(vec![note(1, pb::TaskNoteKind::Comment, "Walkthrough:\nNew words.")]),
        lane: &pb::Lane::default(),
        theme: None,
        rulings: vec![],
        cost_line: false,
        changes: None,
        carried: Some(&kept),
        reviewers: &[],
    });
    assert!(after.contains("New words."), "{after}");
    assert_eq!(after.replace("## Walkthrough\nNew words.\n\n", ""), before, "everything diff-derived is as it was");
    // A diff that read fine and was empty is not a failure: it clears them.
    let empty = Changes::default();
    let cleared = section::render(&Input { detail: &card(vec![]), lane: &pb::Lane::default(), theme: None, rulings: vec![], cost_line: false, changes: Some(&empty), carried: Some(&kept), reviewers: &[] });
    assert!(!cleared.contains("Review this first") && !cleared.contains("files changed"), "{cleared}");
    // And nothing to keep, nothing invented.
    assert_eq!(section::Carried::of("no markers at all"), section::Carried::default());
}

// ---- git ------------------------------------------------------------------

/// A repository with `main` and a branch `lane` that changed three files.
fn scratch_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    git(d, &["init", "-q", "-b", "main"]);
    std::fs::create_dir_all(d.join("crates/core/src")).unwrap();
    std::fs::write(d.join("crates/core/src/lib.rs"), "fn a() {}\n").unwrap();
    std::fs::write(d.join("gone.txt"), "x\ny\n").unwrap();
    std::fs::write(d.join("moved.txt"), "same\ncontent\nhere\nfor\nrename\n").unwrap();
    git(d, &["add", "."]);
    git(d, &["commit", "-q", "-m", "base"]);
    git(d, &["checkout", "-q", "-b", "lane"]);
    std::fs::write(d.join("crates/core/src/lib.rs"), "fn a() {}\npub fn b() {}\n").unwrap();
    std::fs::remove_file(d.join("gone.txt")).unwrap();
    std::fs::rename(d.join("moved.txt"), d.join("moved again.txt")).unwrap();
    std::fs::write(d.join("we\"ird\tname.txt"), "nothing like the others here\n").unwrap();
    git(d, &["add", "-A"]);
    git(d, &["commit", "-q", "-m", "lane"]);
    dir
}

#[test]
fn changes_in_a_real_repository_reads_the_branch_against_its_base() {
    let repo = scratch_repo();
    let c = changes_in(Some(repo.path()), "main", "lane").unwrap();
    let got: Vec<(&str, Kind)> = c.files.iter().map(|f| (f.path.as_str(), f.kind)).collect();
    assert_eq!(
        got,
        vec![
            ("crates/core/src/lib.rs", Kind::Modified),
            ("gone.txt", Kind::Deleted),
            ("moved again.txt", Kind::Renamed),
            ("we\"ird\tname.txt", Kind::Added)
        ]
    );
    // main...lane: work that landed on main since is not the lane's.
    git(repo.path(), &["checkout", "-q", "main"]);
    std::fs::write(repo.path().join("later.txt"), "later\n").unwrap();
    git(repo.path(), &["add", "later.txt"]);
    git(repo.path(), &["commit", "-q", "-m", "later"]);
    assert_eq!(changes_in(Some(repo.path()), "main", "lane").unwrap(), c);
    // Identical branches: a diff that read fine and was empty.
    assert_eq!(changes_in(Some(repo.path()), "main", "main").unwrap(), Changes::default());
}

#[test]
fn git_failing_is_an_error_with_git_s_first_line_and_not_an_empty_diff() {
    let repo = scratch_repo();
    let err = changes_in(Some(repo.path()), "origin/main", "lane").unwrap_err();
    assert!(err.starts_with("fatal:") && !err.contains('\n'), "{err}");
    assert!(changes_in(Some(repo.path()), "main", "no-such-branch").is_err());
    let elsewhere = tempfile::tempdir().unwrap();
    assert!(changes_in(Some(elsewhere.path()), "main", "lane").is_err(), "not a repository");
    assert_eq!(changes_in(Some(repo.path()), "main", "").unwrap_err(), "the lane has no branch recorded");
    assert!(changes_in(Some(repo.path()), "--output=/tmp/x", "lane").is_err() && changes_in(Some(repo.path()), "main", "-p").is_err());
}
