//! Before and after captures side by side, and a UI card with none (ov-578):
//! the pairing convention by file name, what is paired and what is not, and
//! how a card is known to be UI.

use super::super::diff;
use super::*;
use crate::id_bytes;

const CARD: uuid::Uuid = uuid::Uuid::from_u128(0x12);
const BASE: &str = "https://github.com/o/r/blob/farcooler/captures";

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

fn card(labels: &[&str], notes: Vec<pb::TaskNote>) -> pb::TaskDetail {
    pb::TaskDetail {
        task: Some(pb::Task {
            id: id_bytes(CARD),
            key: "ov-12".into(),
            title: "Frosted plane".into(),
            intent: "Cards sit on one surface.".into(),
            labels: labels.iter().map(|l| l.to_string()).collect(),
            ..Default::default()
        }),
        notes,
        blocks: vec![],
    }
}

fn render(detail: &pb::TaskDetail, changes: Option<&Changes>) -> String {
    super::render(&Input { detail, lane: &pb::Lane::default(), theme: None, rulings: vec![], cost_line: false, changes, carried: None, reviewers: &[] })
}

fn links(names: &[&str]) -> String {
    names.iter().map(|n| format!("{BASE}/{n}?raw=true")).collect::<Vec<_>>().join(" ")
}

fn captures_of(out: &str) -> String {
    let at = out.find("## Captures").expect("a captures section");
    out[at..].lines().take_while(|l| !l.starts_with("<!--")).collect::<Vec<_>>().join("\n")
}

fn patch(path: &str, hunk_context: &str, added: &str) -> Changes {
    diff::parse(&format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -3,0 +4,1 @@ {hunk_context}\n+{added}\n"))
}

// ---- pairs ----------------------------------------------------------------

#[test]
fn a_before_and_an_after_with_the_same_name_are_one_pair_in_either_order() {
    let want = format!(
        "## Captures\n**ov-12-dark**\n\n| Before | After |\n| --- | --- |\n\
         | ![Before]({BASE}/ov-12-before-dark.png?raw=true) | ![After]({BASE}/ov-12-after-dark.png?raw=true) |"
    );
    for order in [["ov-12-before-dark.png", "ov-12-after-dark.png"], ["ov-12-after-dark.png", "ov-12-before-dark.png"]] {
        let out = render(&card(&[], vec![note(1, pb::TaskNoteKind::Progress, &format!("Captures: {}", links(&order)))]), None);
        assert_eq!(captures_of(&out), want, "{out}");
    }
}

#[test]
fn pairs_match_by_what_they_are_of_and_a_lone_capture_is_a_bullet_after_them() {
    let body = format!(
        "![Light]({BASE}/ov-12-light.png) {} and {}",
        links(&["ov-12-before-dark.png", "ov-12-after-light.png"]),
        links(&["ov-12-after-dark.png", "ov-12-before-light.png"])
    );
    let out = render(&card(&[], vec![note(1, pb::TaskNoteKind::Progress, &body)]), None);
    let caps = captures_of(&out);
    // Two pairs, dark first because its "before" was written first, then the plain light capture.
    assert_eq!(caps.matches("| Before | After |").count(), 2, "{caps}");
    assert!(caps.find("**ov-12-dark**").unwrap() < caps.find("**ov-12-light**").unwrap(), "{caps}");
    assert!(caps.ends_with(&format!("- [Light]({BASE}/ov-12-light.png)")), "{caps}");
}

#[test]
fn a_before_without_its_after_is_an_ordinary_link() {
    let out = render(&card(&[], vec![note(1, pb::TaskNoteKind::Progress, &links(&["ov-12-before-dark.png", "ov-12-after-light.png"]))]), None);
    let caps = captures_of(&out);
    assert!(!caps.contains("| Before |"), "{caps}");
    assert_eq!(caps.matches("\n- [").count(), 2, "{caps}");
}

#[test]
fn a_name_with_both_words_or_only_the_word_pairs_with_nothing() {
    for (a, b) in [("before-after.png", "after-before.png"), ("before.png", "after.png"), ("x-before-after-y.png", "x-after-before-y.png")] {
        let out = render(&card(&[], vec![note(1, pb::TaskNoteKind::Progress, &links(&[a, b]))]), None);
        assert!(!captures_of(&out).contains("| Before |"), "{a} {b}: {out}");
    }
    // A word that only contains the word is not the word.
    let out = render(&card(&[], vec![note(1, pb::TaskNoteKind::Progress, &links(&["beforehand-dark.png", "afterglow-dark.png"]))]), None);
    assert!(!captures_of(&out).contains("| Before |"), "{out}");
}

#[test]
fn a_pair_of_videos_is_one_line_of_two_links_because_a_table_cannot_play_them() {
    let out = render(&card(&[], vec![note(1, pb::TaskNoteKind::Progress, &links(&["open-after.mp4", "open-before.mp4"]))]), None);
    assert_eq!(
        captures_of(&out),
        format!("## Captures\n**open**: [Before]({BASE}/open-before.mp4?raw=true), [After]({BASE}/open-after.mp4?raw=true)")
    );
}

#[test]
fn a_pair_s_name_is_made_safe_like_everything_the_card_says() {
    let out = render(&card(&[], vec![note(1, pb::TaskNoteKind::Progress, &links(&["@alice-before.png", "@alice-after.png"]))]), None);
    assert!(captures_of(&out).contains("**@\u{200d}alice**"), "{out}");
}

// ---- a UI card with no captures ------------------------------------------

#[test]
fn a_card_labeled_ui_with_no_captures_says_so_in_the_agent_review() {
    for label in ["ui", "UI", " ui "] {
        let out = render(&card(&[label], vec![]), None);
        assert!(out.contains("## Agent review\nNo agent review yet.\nNot checked: captures\n"), "{out}");
    }
    let reviewed = card(&["ui"], vec![note(1, pb::TaskNoteKind::Finding, "**Opus review: land.**\n\n- Not checked: Android at 1x")]);
    let out = render(&reviewed, None);
    assert!(out.contains("Opus review: land. (1 round)\nNot checked: Android at 1x\nNot checked: captures\n"), "{out}");
}

#[test]
fn it_is_said_once_even_when_the_review_said_it_first() {
    let said = card(&["ui"], vec![note(1, pb::TaskNoteKind::Finding, "**Opus review: land.**\n\n- Not checked: Captures.")]);
    assert_eq!(render(&said, None).matches("Not checked: ").count(), 1);
}

#[test]
fn a_card_with_a_capture_or_that_is_not_ui_is_not_flagged() {
    let with = card(&["ui"], vec![note(1, pb::TaskNoteKind::Progress, &links(&["a-light.png"]))]);
    assert!(!render(&with, None).contains("Not checked"));
    assert!(!render(&card(&["team-mode"], vec![]), None).contains("Not checked"));
    assert!(!render(&card(&[], vec![]), None).contains("Not checked"));
}

#[test]
fn a_diff_that_adds_a_view_under_apps_makes_a_card_ui_without_a_label() {
    let view = patch("apps/macos/Sources/FarCooler/Plan/PlaneView.swift", "", "struct PlaneView: View {");
    assert!(render(&card(&[], vec![]), Some(&view)).contains("Not checked: captures"));
    let body = patch("apps/ios/FarCooler/Row.swift", "", "    var body: some View {");
    assert!(render(&card(&[], vec![]), Some(&body)).contains("Not checked: captures"));
    let compose = patch("apps/android/app/src/main/java/com/farcooler/ui/Row.kt", "", "@Composable");
    assert!(render(&card(&[], vec![]), Some(&compose)).contains("Not checked: captures"));
    // An edit inside a view that declares nothing itself: git names the hunk by the type.
    let inside = patch("apps/macos/Sources/FarCooler/Plan/PlaneView.swift", "struct PlaneView: View {", "        .padding(12)");
    assert!(render(&card(&[], vec![]), Some(&inside)).contains("Not checked: captures"));
}

#[test]
fn the_same_code_is_not_ui_in_a_test_in_a_crate_or_as_a_comment() {
    let not_ui = [
        patch("apps/macos/Tests/PlaneViewTests.swift", "", "struct Probe: View {"),
        patch("apps/shared/AgentKit/Tests/AgentKitTests/RowTests.swift", "", "    var body: some View {"),
        patch("crates/daemon/src/plane.rs", "", "// struct PlaneView: View {"),
        patch("apps/macos/Sources/FarCooler/Plan/Store.swift", "", "// struct PlaneView: View {"),
        patch("apps/macos/Sources/FarCooler/Plan/Store.swift", "", "    var rows: [Row] = []"),
    ];
    for changes in &not_ui {
        assert!(!render(&card(&[], vec![]), Some(changes)).contains("Not checked"), "{changes:?}");
    }
    // A deleted view is not a screen to capture.
    let gone = diff::parse(
        "diff --git a/apps/macos/Sources/FarCooler/Old.swift b/apps/macos/Sources/FarCooler/Old.swift\ndeleted file mode 100644\n\
         --- a/apps/macos/Sources/FarCooler/Old.swift\n+++ /dev/null\n@@ -1,1 +0,0 @@ struct Old: View {\n-x\n",
    );
    assert!(!render(&card(&[], vec![]), Some(&gone)).contains("Not checked"));
}

#[test]
fn an_address_that_could_end_a_link_a_table_cell_or_a_tag_is_not_a_capture() {
    for bad in [
        "https://x.example/a|b.png",
        "https://x.example/a<b>.png",
        "https://x.example/a>b.png",
        "https://x.example/a)b.png",
        "https://x.example/a[b.png",
        "https://x.example/a]b.png",
        "https://x.example/a`b.png",
        "https://x.example/a\"b.png",
    ] {
        let body = format!("Captures: {bad} and ![alt]({bad})");
        let out = render(&card(&[], vec![note(1, pb::TaskNoteKind::Progress, &body)]), None);
        assert!(!out.contains("## Captures"), "{bad:?}: {out}");
    }
    let spaced = render(&card(&[], vec![note(1, pb::TaskNoteKind::Progress, "![alt](https://x.example/a b.png)")]), None);
    assert!(!spaced.contains("## Captures"), "{spaced}");
    let pair = render(&card(&[], vec![note(1, pb::TaskNoteKind::Progress, &links(&["a-before-x.png", "a-after-x.png|evil"]))]), None);
    assert!(!pair.contains("| Before | After |") && !pair.contains("evil"), "a cell can't be broken out of: {pair}");
}
