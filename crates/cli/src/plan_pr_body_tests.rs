//! A pull request's description from the card (ov-314): the section's every
//! line against golden files, a reviewer's words surviving a re-render, and
//! `--apply` through a `gh` of the test's own.

use clap::Parser;
use farcooler_transport::ClientError;

use super::section::{self, BEGIN, END, Input};
use super::*;
use crate::id_bytes;

const CARD: uuid::Uuid = uuid::Uuid::from_u128(0x12);
const OTHER: uuid::Uuid = uuid::Uuid::from_u128(0x13);

fn golden(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/pr-body").join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
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

/// The card: two acceptance lines, one met; a review that ran twice (and one
/// earlier finding a later one replaced); captures in a progress note.
fn detail() -> pb::TaskDetail {
    let mut replaced = note(1, pb::TaskNoteKind::Finding, "**Opus review: land.** (replaced by round 1)");
    replaced.at = 1;
    let mut first = note(2, pb::TaskNoteKind::Finding, "**Opus review: fix first.** One blocker.\n\n- **Blocker:** a card with no PR took a sibling's.\n- Not checked: Android at 1x");
    first.supersedes = Some(replaced.id.clone());
    let second = note(
        3,
        pb::TaskNoteKind::Finding,
        "**Opus re-review: land.** The blocker and the should-fixes hold.\n\n- Not checked: Android at 1x\n- **Not checked:** iPad split view",
    );
    let captures = note(
        4,
        pb::TaskNoteKind::Progress,
        "Captures: ![Light](https://github.com/o/r/blob/farcooler/captures/ov-12-light.png?raw=true), and \
         https://github.com/o/r/blob/farcooler/captures/ov-12-dark.png?raw=true. Again: \
         https://github.com/o/r/blob/farcooler/captures/ov-12-dark.png?raw=true",
    );
    let chatter = note(5, pb::TaskNoteKind::Comment, "See https://example.com/page and https://example.com/not-a-capture.html");
    pb::TaskDetail {
        task: Some(pb::Task {
            id: id_bytes(CARD),
            key: "ov-12".into(),
            title: "Frosted window plane".into(),
            intent: "The window plane uses the frosted material across the Mac app, so every card sits on one surface.".into(),
            acceptance: vec![
                pb::TaskAcceptanceItem { id: id_bytes(uuid::Uuid::from_u128(1)), text: "The window plane uses the frosted material (MainCardsTests)".into(), met: true },
                pb::TaskAcceptanceItem { id: id_bytes(uuid::Uuid::from_u128(2)), text: "Phones match (stack PR 2 of 3)".into(), met: false },
            ],
            ..Default::default()
        }),
        notes: vec![replaced, first, second, captures, chatter],
        blocks: vec![],
    }
}

fn bare_detail() -> pb::TaskDetail {
    pb::TaskDetail {
        task: Some(pb::Task { id: id_bytes(CARD), key: "ov-12".into(), title: "Fix the thing".into(), ..Default::default() }),
        notes: vec![note(1, pb::TaskNoteKind::Created, "created")],
        blocks: vec![],
    }
}

fn lane() -> pb::Lane {
    pb::Lane {
        id: id_bytes(uuid::Uuid::from_u128(0x50)),
        name: "mac-frost".into(),
        branch: "el/frost".into(),
        cards: vec![pb::LaneCard { task_id: id_bytes(CARD), ..Default::default() }],
        agents: vec![
            pb::LaneAgent { harness: "claude".into(), model: "sonnet".into(), role: pb::LaneAgentRole::Build as i32, ..Default::default() },
            pb::LaneAgent { harness: "claude".into(), model: "sonnet".into(), role: pb::LaneAgentRole::Fix as i32, ..Default::default() },
            pb::LaneAgent { harness: "claude".into(), model: "opus".into(), role: pb::LaneAgentRole::Review as i32, ..Default::default() },
        ],
        spend: Some(pb::LaneSpend { input_tokens: 300_000, output_tokens: 60_000, cache_read_tokens: 40_000, cache_write_tokens: 10_000, ..Default::default() }),
        ..Default::default()
    }
}

fn ruling(number: u32, decision: &str, reversal: &str, state: pb::BoardRulingState, on: uuid::Uuid) -> pb::BoardRuling {
    pb::BoardRuling {
        number,
        decision: decision.into(),
        reversal: reversal.into(),
        state: state as i32,
        task_ids: vec![id_bytes(on)],
        ..Default::default()
    }
}

fn rulings() -> Vec<pb::BoardRuling> {
    use pb::BoardRulingState::*;
    vec![
        ruling(12, "Inbox stays amber, not blue.", "one token.", Standing, CARD),
        ruling(13, "The window plane keeps its 12 pt corner.", "a one-line change", Confirmed, CARD),
        ruling(14, "Captures are PNG.", "a rerun", Reversed, CARD),
        ruling(15, "Another card's call.", "none", Standing, OTHER),
    ]
}

fn render(detail: &pb::TaskDetail, lane: &pb::Lane, theme: Option<&str>, rulings: &[pb::BoardRuling], cost_line: bool) -> String {
    section::render(&Input { detail, lane, theme, rulings: rulings.iter().filter(|r| r.task_ids.contains(&id_bytes(CARD))).collect(), cost_line })
}

fn full(cost_line: bool) -> String {
    render(&detail(), &lane(), Some("Visual language"), &rulings(), cost_line)
}

// ---- the section ----------------------------------------------------------

#[test]
fn the_section_is_the_golden_file_with_the_cost_toggle_on() {
    assert_eq!(full(true), golden("full.md"));
}

#[test]
fn the_same_card_with_the_cost_toggle_off_has_no_cost_section() {
    assert_eq!(full(false), golden("cost-off.md"));
    assert!(!full(false).contains("## Cost"));
}

#[test]
fn a_bare_card_says_so_and_leaves_the_empty_sections_out() {
    assert_eq!(render(&bare_detail(), &pb::Lane::default(), None, &[], false), golden("bare.md"));
}

#[test]
fn a_tick_on_the_card_is_a_tick_in_the_section() {
    let mut d = detail();
    d.task.as_mut().unwrap().acceptance[1].met = true;
    let out = render(&d, &lane(), Some("Visual language"), &rulings(), true);
    assert!(out.contains("- [x] The window plane uses"), "{out}");
    assert!(out.contains("- [x] Phones match (stack PR 2 of 3)"), "{out}");
    assert!(!out.contains("- [ ]"), "{out}");
    d.task.as_mut().unwrap().acceptance[0].met = false;
    assert!(render(&d, &lane(), None, &[], false).contains("- [ ] The window plane uses"));
}

#[test]
fn rulings_on_the_lane_s_cards_appear_and_reversed_or_other_ones_do_not() {
    let out = full(false);
    assert!(out.contains("- R-12 Inbox stays amber, not blue (reversible: one token). Reply \"reverse R-12\"."), "{out}");
    assert!(out.contains("- R-13 The window plane keeps its 12 pt corner. The owner confirmed it."), "{out}");
    assert!(!out.contains("R-14"), "a reversed call is undone: {out}");
    assert!(!out.contains("R-15"), "another card's call isn't this card's: {out}");
}

#[test]
fn a_finding_a_later_one_replaced_is_history_and_the_rounds_count_the_rest() {
    let out = full(false);
    assert!(out.contains("The blocker and the should-fixes hold. (2 rounds)"), "{out}");
    assert!(!out.contains("(replaced by round 1)") && !out.contains("(3 rounds)"), "{out}");
    let mut d = detail();
    d.notes.retain(|n| n.at != 3);
    d.notes.retain(|n| n.at != 1);
    let one = render(&d, &lane(), None, &[], false);
    assert!(one.contains("Opus review: fix first. One blocker. (1 round)\nNot checked: Android at 1x\n"), "{one}");
}

#[test]
fn what_was_not_checked_is_carried_once_however_it_is_formatted() {
    let out = full(false);
    assert_eq!(out.matches("Not checked: Android at 1x").count(), 1, "{out}");
    assert!(out.contains("Not checked: iPad split view"), "a bold label counts: {out}");
}

#[test]
fn captures_are_links_found_in_the_notes_each_once() {
    let out = full(false);
    assert_eq!(out.matches("ov-12-dark.png").count(), 2, "one link, its text and its address: {out}");
    assert!(!out.contains("example.com"), "a page isn't a capture: {out}");
}

#[test]
fn the_cost_line_says_what_was_spent_and_by_whom() {
    let one = full(true);
    assert!(one.ends_with("## Cost\n410K tokens, Sonnet build + Sonnet fix + Opus review\n<!-- farcooler:end -->"), "{one}");
    let mut two = lane();
    two.cards.push(pb::LaneCard { task_id: id_bytes(OTHER), ..Default::default() });
    let out = render(&detail(), &two, None, &[], true);
    assert!(out.contains("About 205K tokens, the lane's spend split evenly across 2 cards, Sonnet build + Sonnet fix + Opus review"), "{out}");
    let mut none = lane();
    none.spend = None;
    let out = render(&detail(), &none, None, &[], true);
    assert!(out.contains("## Cost\nNot reported\n"), "no tokens is not zero tokens: {out}");
}

// ---- merging into a description -------------------------------------------

const REVIEWER_ABOVE: &str = "Thanks! Two questions below.\r\n\r\n> The frosted plane\r\n";
const REVIEWER_BELOW: &str = "\r\n\r\n@alice: can you check the phones? Closes #41\r\n";

fn described_by_a_reviewer(section: &str) -> String {
    format!("{REVIEWER_ABOVE}{section}{REVIEWER_BELOW}")
}

#[test]
fn a_reviewer_s_edits_outside_the_markers_survive_a_re_render() {
    let stale = render(&bare_detail(), &lane(), None, &[], false);
    let existing = described_by_a_reviewer(&stale);
    let merged = section::merge(&existing, &full(true)).unwrap();
    assert_eq!(merged, described_by_a_reviewer(&full(true)), "above, below and the line endings are byte for byte");
    assert!(merged.starts_with(REVIEWER_ABOVE) && merged.ends_with(REVIEWER_BELOW));
    assert!(!merged.contains("No agent review yet."), "what was inside was replaced");
    // And again: a second re-render of the same card changes nothing.
    assert_eq!(section::merge(&merged, &full(true)).unwrap(), merged);
}

#[test]
fn a_description_with_no_markers_gets_the_section_on_the_end() {
    let s = full(false);
    assert_eq!(section::merge("", &s).unwrap(), s);
    assert_eq!(section::merge("   \n", &s).unwrap(), s, "nothing a reviewer wrote is lost, and nothing was there");
    assert_eq!(section::merge("Hello", &s).unwrap(), format!("Hello\n\n{s}"));
    assert_eq!(section::merge("Hello\n", &s).unwrap(), format!("Hello\n\n{s}"));
    assert_eq!(section::merge("Hello\n\n", &s).unwrap(), format!("Hello\n\n{s}"));
}

#[test]
fn markers_it_cannot_make_sense_of_are_refused_and_nothing_is_replaced() {
    let s = full(false);
    for existing in [
        format!("text {BEGIN} no end"),
        format!("no begin {END} text"),
        format!("{END} backwards {BEGIN}"),
        format!("{BEGIN}a{END} and again {BEGIN}b{END}"),
        format!("{BEGIN}{BEGIN}{END}"),
    ] {
        let refused = section::merge(&existing, &s).expect_err(&existing);
        assert!(refused.0.starts_with("This description has "), "{refused:?}");
    }
}

// ---- the command ----------------------------------------------------------

struct Runner {
    details: Vec<pb::TaskDetail>,
    sent: Vec<String>,
}

impl DispatchLink for Runner {
    fn capabilities(&self) -> Vec<String> {
        vec![capability::TASKS.into(), capability::BOARD_PLAN.into()]
    }
    async fn pause(&mut self, _wait: std::time::Duration) {}
    async fn call(&mut self, req: pb::Request) -> Result<pb::Result, ClientError> {
        self.sent.push(req.method.clone());
        assert_eq!(req.method, "task.get", "pr-body only reads the card");
        let Some(request::Payload::TaskGet(p)) = req.payload else { panic!("payload") };
        let found = self.details.iter().find(|d| d.task.as_ref().is_some_and(|t| t.id == p.task_id)).cloned();
        Ok(pb::Result { value: Some(result::Value::TaskDetail(found.expect("a card"))) })
    }
}

fn the_plan(two_cards: bool) -> (pb::Plan, Keys) {
    let mut lane = lane();
    if two_cards {
        lane.cards.push(pb::LaneCard { task_id: id_bytes(OTHER), ..Default::default() });
    }
    let plan = pb::Plan {
        themes: vec![pb::BoardThemeView {
            theme: Some(pb::BoardTheme { name: "Visual language".into(), ..Default::default() }),
            task_ids: vec![id_bytes(CARD)],
            ..Default::default()
        }],
        lanes: vec![lane],
        cards: vec![
            pb::PlanCard { task_id: id_bytes(CARD), key: "ov-12".into(), title: "Frosted window plane".into(), ..Default::default() },
            pb::PlanCard { task_id: id_bytes(OTHER), key: "ov-13".into(), title: "Phones".into(), ..Default::default() },
        ],
        rulings: rulings(),
        ..Default::default()
    };
    let keys = Keys::of_plan(&plan);
    (plan, keys)
}

fn board(cost: Option<bool>) -> Board {
    Board {
        workspace: Some(pb::Workspace { name: "Main".into(), is_main: true, pr_cost_line: cost, ..Default::default() }),
        repository: uuid::Uuid::from_u128(1),
        has_workspaces: true,
    }
}

fn args(line: &str) -> PrBodyArgs {
    match crate::Cli::try_parse_from(line.split_whitespace()).expect(line).command {
        crate::Command::Plan(plan) => match plan.cmd {
            Some(super::super::PlanCmd::Lane(super::super::LaneCmd::PrBody(a))) => a,
            _ => panic!("{line}"),
        },
        _ => panic!("{line}"),
    }
}

struct Described {
    body: String,
    /// The branch the pull request is from.
    head: String,
    cross_repository: bool,
    writes: Vec<String>,
    refuse: bool,
    /// What a reviewer saves after the first read.
    reviewer_saves: Option<String>,
    reads: usize,
}

impl Description for Described {
    fn read(&mut self, _pr: &str) -> Result<Pr, String> {
        if self.refuse {
            return Err("GitHub wouldn't do that.".into());
        }
        self.reads += 1;
        let now = Pr { body: self.body.clone(), head: self.head.clone(), cross_repository: self.cross_repository };
        if let Some(saved) = self.reviewer_saves.take() {
            self.body = saved;
        }
        Ok(now)
    }
    fn write(&mut self, _pr: &str, body: &str) -> Result<(), String> {
        self.writes.push(body.to_string());
        self.body = body.to_string();
        Ok(())
    }
}

async fn run(line: &str, two_cards: bool, cost: Option<bool>, gh: &mut Described) -> Result<String, String> {
    let (plan, keys) = the_plan(two_cards);
    let mut link = Runner { details: vec![detail()], sent: vec![] };
    pr_body_with(&mut link, &board(cost), &plan, &keys, args(line), false, gh).await.map_err(|e| e.to_string())
}

fn gh(body: &str) -> Described {
    Described { body: body.into(), head: "el/frost".into(), cross_repository: false, writes: vec![], refuse: false, reviewer_saves: None, reads: 0 }
}

#[tokio::test]
async fn it_prints_the_section_for_the_lane_s_card_and_the_workspace_s_cost_choice() {
    let out = run("farcooler plan lane pr-body mac-frost", false, Some(true), &mut gh("")).await.unwrap();
    assert_eq!(out, golden("full.md"), "theme from the plan, rulings from the plan, cost on because the workspace said so");
    let off = run("farcooler plan lane pr-body mac-frost", false, None, &mut gh("")).await.unwrap();
    assert_eq!(off, golden("cost-off.md"), "a workspace that never opted in has no cost line");
    let off = run("farcooler plan lane pr-body mac-frost", false, Some(false), &mut gh("")).await.unwrap();
    assert_eq!(off, golden("cost-off.md"));
}

#[tokio::test]
async fn a_lane_of_several_cards_is_asked_which_because_a_pull_request_is_one_cards() {
    let said = run("farcooler plan lane pr-body mac-frost", true, None, &mut gh("")).await.unwrap_err();
    assert_eq!(said, "mac-frost works 2 cards, and a pull request is one card's. Name one with --card: ov-12, ov-13.");
    let named = run("farcooler plan lane pr-body mac-frost --card OV-12", true, None, &mut gh("")).await.unwrap();
    assert!(named.contains("## What and why\nThe window plane"), "{named}");
    let wrong = run("farcooler plan lane pr-body mac-frost --card ov-99", true, None, &mut gh("")).await.unwrap_err();
    assert_eq!(wrong, "mac-frost doesn't work ov-99. Its cards: ov-12, ov-13.");
    let nobody = run("farcooler plan lane pr-body nobody", false, None, &mut gh("")).await.unwrap_err();
    assert!(nobody.contains("No lane here is called"), "{nobody}");
}

#[tokio::test]
async fn apply_replaces_only_the_section_and_a_second_apply_writes_nothing() {
    let stale = render(&bare_detail(), &lane(), None, &[], false);
    let mut d = gh(&described_by_a_reviewer(&stale));
    let said = run("farcooler plan lane pr-body mac-frost --apply 31", false, Some(true), &mut d).await.unwrap();
    assert_eq!(said, "Updated the Far Cooler section of 31. Everything outside it was left alone.");
    assert_eq!(d.writes, [described_by_a_reviewer(&golden("full.md"))]);

    let again = run("farcooler plan lane pr-body mac-frost --apply 31", false, Some(true), &mut d).await.unwrap();
    assert_eq!(again, "The description of 31 already says this.");
    assert_eq!(d.writes.len(), 1, "nothing to change is nothing written");
}

#[tokio::test]
async fn apply_appends_when_the_markers_are_missing_and_says_so() {
    let mut d = gh("What a reviewer typed.");
    let said = run("farcooler plan lane pr-body mac-frost --apply 31", false, None, &mut d).await.unwrap();
    assert_eq!(said, "Added the Far Cooler section to the end of 31. Everything else was left alone.");
    assert_eq!(d.writes, [format!("What a reviewer typed.\n\n{}", golden("cost-off.md"))]);
}

#[tokio::test]
async fn apply_refuses_damaged_markers_and_a_gh_that_cannot_answer_without_writing() {
    let mut d = gh(&format!("mine\n{BEGIN}\nhalf a section"));
    let said = run("farcooler plan lane pr-body mac-frost --apply 31", false, None, &mut d).await.unwrap_err();
    assert!(said.starts_with("This description has 1 begin and 0 end markers"), "{said}");
    assert!(d.writes.is_empty());

    let mut refused = Described { refuse: true, ..gh("") };
    let said = run("farcooler plan lane pr-body mac-frost --apply 31", false, None, &mut refused).await.unwrap_err();
    assert_eq!(said, "GitHub wouldn't do that.");
    assert!(refused.writes.is_empty());
}

// ---- the real gh ----------------------------------------------------------

/// A `gh` that records its arguments and the body file it was given, and
/// answers `pr view` with a stored description.
fn planted_gh(dir: &std::path::Path, body_json: &str) -> std::ffi::OsString {
    let gh = dir.join("gh");
    std::fs::write(
        &gh,
        format!(
            "#!/bin/sh\necho \"$@\" >> '{dir}/args'\ncase \"$2\" in\n  view) printf '%s' '{body_json}' ;;\n  edit) cat \"$5\" > '{dir}/written' ;;\nesac\n",
            dir = dir.display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
    gh.into_os_string()
}

#[test]
fn the_real_gh_is_asked_for_the_body_exactly_and_given_the_new_one_in_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let program = planted_gh(dir.path(), r#"{"body":"line one\r\nline two\n","headRefName":"el/frost","isCrossRepository":true}"#);
    let mut real = Gh { program };
    let read = real.read("31").unwrap();
    assert_eq!(read, Pr { body: "line one\r\nline two\n".into(), head: "el/frost".into(), cross_repository: true }, "no jq, no added newline: the body as GitHub has it");
    real.write("31", "new body\r\nwith\n").unwrap();
    assert_eq!(std::fs::read_to_string(dir.path().join("written")).unwrap(), "new body\r\nwith\n");
    let args = std::fs::read_to_string(dir.path().join("args")).unwrap();
    let lines: Vec<&str> = args.lines().collect();
    assert_eq!(lines[0], "pr view 31 --json body,headRefName,isCrossRepository");
    let file = lines[1].strip_prefix("pr edit 31 --body-file ").unwrap_or_else(|| panic!("{args}"));
    assert!(file.contains("farcooler-pr-body-"), "{args}");
    assert!(!std::path::Path::new(file).exists(), "the temporary file is removed: {file}");
}

#[test]
fn a_gh_that_is_missing_or_refuses_is_said_in_words_and_a_flag_is_never_a_pull_request() {
    let mut missing = Gh { program: "/nonexistent/gh".into() };
    assert_eq!(missing.read("31").unwrap_err(), "gh isn't installed here, so Far Cooler can't reach GitHub.");
    let dir = tempfile::tempdir().unwrap();
    let failing = dir.path().join("gh");
    std::fs::write(&failing, "#!/bin/sh\necho 'HTTP 404: boom' >&2\nexit 1\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&failing, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut refusing = Gh { program: failing.into_os_string() };
    let said = refusing.read("31").unwrap_err();
    assert!(said.starts_with("GitHub wouldn't do that."), "{said}");
    assert!(!said.contains("boom"), "gh's own error isn't shown raw: {said}");
    for bad in ["--repo=x", "-1", "  "] {
        assert_eq!(refusing.read(bad).unwrap_err(), "Name the pull request by its number, its branch or its URL.");
    }
}

// ---- --apply only writes to the lane's own pull request -------------------

#[tokio::test]
async fn apply_refuses_a_pull_request_from_another_branch_and_force_overrides() {
    let mut other = Described { head: "someone/else".into(), ..gh("Someone else's description.") };
    let said = run("farcooler plan lane pr-body mac-frost --apply 42", false, None, &mut other).await.unwrap_err();
    assert_eq!(said, "42 is from the branch someone/else, and mac-frost works el/frost. Check the number, or add --force if it is the right pull request.");
    assert!(other.writes.is_empty(), "nothing is appended to somebody else's pull request");

    let forced = run("farcooler plan lane pr-body mac-frost --apply 42 --force", false, None, &mut other).await.unwrap();
    assert!(forced.starts_with("Added the Far Cooler section"), "{forced}");
    assert_eq!(other.writes.len(), 1);
}

#[tokio::test]
async fn apply_refuses_when_the_lane_has_no_branch_to_check_against() {
    let (mut plan, keys) = the_plan(false);
    plan.lanes[0].branch.clear();
    let mut link = Runner { details: vec![detail()], sent: vec![] };
    let mut d = gh("");
    let said = pr_body_with(&mut link, &board(None), &plan, &keys, args("farcooler plan lane pr-body mac-frost --apply 31"), false, &mut d)
        .await
        .map_err(|e| e.to_string())
        .unwrap_err();
    assert_eq!(said, "mac-frost has no branch recorded, so Far Cooler can't tell that 31 is its pull request. Check the number, then add --force.");
    assert!(d.writes.is_empty());
}

#[test]
fn force_is_only_for_apply() {
    assert!(crate::Cli::try_parse_from(["farcooler", "plan", "lane", "pr-body", "mac-frost", "--force"]).is_err());
}

#[tokio::test]
async fn a_description_that_moved_since_it_was_read_is_not_overwritten() {
    let mut d = Described { reviewer_saves: Some("A reviewer's edit, saved a moment ago.".into()), ..gh("Before.") };
    let said = run("farcooler plan lane pr-body mac-frost --apply 31", false, None, &mut d).await.unwrap_err();
    assert_eq!(said, "The description changed while Far Cooler was reading it, so nothing was written. Run this again.");
    assert!(d.writes.is_empty());
    assert_eq!(d.body, "A reviewer's edit, saved a moment ago.");
    // Run again: it reads the new description and writes once.
    run("farcooler plan lane pr-body mac-frost --apply 31", false, None, &mut d).await.unwrap();
    assert_eq!(d.writes.len(), 1);
    assert!(d.writes[0].starts_with("A reviewer's edit, saved a moment ago.\n\n<!-- farcooler:begin -->"));
}

#[tokio::test]
async fn one_card_s_pull_request_lists_that_card_s_rulings_only() {
    let (plan, keys) = the_plan(true);
    let mut link = Runner { details: vec![detail(), pb::TaskDetail { task: Some(pb::Task { id: id_bytes(OTHER), key: "ov-13".into(), title: "Phones".into(), ..Default::default() }), ..Default::default() }], sent: vec![] };
    let one = pr_body_with(&mut link, &board(None), &plan, &keys, args("farcooler plan lane pr-body mac-frost --card ov-12"), false, &mut gh("")).await.unwrap();
    assert!(one.contains("R-12") && !one.contains("R-15"), "{one}");
    let two = pr_body_with(&mut link, &board(None), &plan, &keys, args("farcooler plan lane pr-body mac-frost --card ov-13"), false, &mut gh("")).await.unwrap();
    assert!(two.contains("R-15") && !two.contains("R-12"), "{two}");
}

#[test]
fn a_card_listed_twice_on_a_lane_is_one_card_in_the_cost_share() {
    let mut twice = lane();
    twice.cards.push(twice.cards[0].clone());
    let out = render(&detail(), &twice, None, &[], true);
    assert!(out.contains("## Cost\n410K tokens, Sonnet build"), "{out}");
}

// ---- text from the card cannot act on the page ---------------------------

#[test]
fn mentions_closing_keywords_and_markers_in_card_text_are_neutralized() {
    use super::section::neutralize;
    let z = '\u{200d}';
    assert_eq!(neutralize("ping @alice and (@bob), mail a@b.com"), format!("ping @{z}alice and (@{z}bob), mail a@b.com"));
    assert_eq!(neutralize("Closes #12"), format!("Closes{z} #12"));
    assert_eq!(neutralize("fixes: owner/repo#7, resolved https://github.com/o/r/issues/3"), format!("fixes{z}: owner/repo#7, resolved{z} https://github.com/o/r/issues/3"));
    assert_eq!(neutralize("This fixes the bug and closes nothing"), "This fixes the bug and closes nothing", "no issue named: nothing to close");
    assert_eq!(neutralize("<!-- farcooler:end -->"), format!("<!{z}-- farcooler:end -->"));
    assert!(!neutralize("<!-- farcooler:begin -->").contains(BEGIN));
}

#[test]
fn a_card_that_quotes_the_markers_still_renders_one_pair() {
    let mut d = detail();
    let task = d.task.as_mut().unwrap();
    task.intent = format!("Quote {END} and {BEGIN} in the intent. Closes #41, thanks @alice.");
    task.acceptance[0].text = format!("Never print {END}");
    let out = render(&d, &lane(), None, &[], false);
    assert_eq!(out.matches(BEGIN).count(), 1, "{out}");
    assert_eq!(out.matches(END).count(), 1, "{out}");
    assert!(!out.contains("Closes #41") && !out.contains("@alice"), "{out}");
    // And so a merge of it, and a second merge, both work.
    let merged = section::merge("mine", &out).unwrap();
    assert_eq!(section::merge(&merged, &out).unwrap(), merged);
}

#[test]
fn text_everywhere_in_the_section_is_neutralized() {
    let mut d = detail();
    d.notes.push(note(9, pb::TaskNoteKind::Finding, "**Re-review: land, thanks @bob.** Closes #5\n- Not checked: @carol's phone"));
    let mut rs = rulings();
    rs[0].decision = "Ping @dave. Fixes #8.".into();
    let out = render(&d, &lane(), None, &rs, false);
    for bad in ["@bob", "@carol", "@dave", "Closes #5", "Fixes #8"] {
        assert!(!out.contains(bad), "{bad}: {out}");
    }
}

// ---- markers inside code are quotations ----------------------------------

#[test]
fn markers_inside_a_fenced_code_block_are_not_the_section() {
    let s = full(false);
    let quoted = format!("How to use it:\n\n```\n{BEGIN}\nsomething\n{END}\n```\n\nThanks.\n");
    let refused = section::merge(&quoted, &s).expect_err("only quotations");
    assert!(refused.0.starts_with("The only Far Cooler markers in this description are inside a code block"), "{refused:?}");
    // With a real pair after the quotation, only the real pair is replaced.
    let both = format!("{quoted}\n{BEGIN}\nold\n{END}\nafter\n");
    let merged = section::merge(&both, &s).unwrap();
    assert!(merged.starts_with(&quoted) && merged.ends_with("\nafter\n"), "{merged}");
    assert!(!merged.contains("\nold\n"));
    // ~~~ fences too.
    let tilde = format!("~~~\n{BEGIN}\n~~~\n");
    assert!(section::merge(&tilde, &s).is_err());
}

#[test]
fn an_unclosed_fence_above_the_section_refuses_instead_of_growing_the_description() {
    let s = full(false);
    let unclosed = format!("Notes:\n```\nsome code\n\n{s}\nafter\n");
    let refused = section::merge(&unclosed, &s).expect_err("both markers are inside the open fence");
    assert!(refused.0.contains("inside a code block, or after one that never closes"), "{refused:?}");
}

#[test]
fn code_spans_and_links_come_through_unchanged_and_bare_text_is_still_neutralized() {
    use super::section::neutralize;
    let z = '\u{200d}';
    assert_eq!(neutralize("Use `@MainActor` and `@Observable`."), "Use `@MainActor` and `@Observable`.");
    assert_eq!(neutralize("See https://medium.com/@user/post and (http://x.io/@a)."), "See https://medium.com/@user/post and (http://x.io/@a).");
    assert_eq!(neutralize("https://x.io/?by=@user#@frag"), "https://x.io/?by=@user#@frag", "an @ after = or # inside a link");
    assert_eq!(
        neutralize("`@MainActor` thanks @alice. Closes #12"),
        format!("`@MainActor` thanks @{z}alice. Closes{z} #12")
    );
    assert_eq!(neutralize("a lone ` tick then @bob"), format!("a lone ` tick then @{z}bob"), "an unpaired backtick opens no span");
    assert_eq!(neutralize("`code` then `@x` then @y"), format!("`code` then `@x` then @{z}y"));
    assert_eq!(neutralize("Closes `#12`"), "Closes `#12`", "inside a span it names nothing");
}

#[test]
fn a_fence_in_card_text_cannot_open_a_block_that_hides_the_markers() {
    use super::section::neutralize;
    let z = '\u{200d}';
    assert_eq!(neutralize("```swift\nlet x = 1\n```"), format!("`{z}``swift\nlet x = 1\n`{z}``"));
    assert_eq!(neutralize("  ~~~"), format!("  ~{z}~~"));
    let mut d = detail();
    d.task.as_mut().unwrap().intent = "Look:\n```\nunclosed".into();
    let out = render(&d, &lane(), None, &[], false);
    let merged = section::merge("mine", &out).unwrap();
    assert_eq!(section::merge(&merged, &out).unwrap(), merged, "the section is still found and replaced, not appended again");
}

#[tokio::test]
async fn apply_refuses_a_fork_s_pull_request_without_force() {
    let mut fork = Described { cross_repository: true, ..gh("A fork's description.") };
    let said = run("farcooler plan lane pr-body mac-frost --apply 42", false, None, &mut fork).await.unwrap_err();
    assert_eq!(said, "42 comes from a fork, not from a branch of this repository. Add --force if it is the right pull request.");
    assert!(fork.writes.is_empty());
    run("farcooler plan lane pr-body mac-frost --apply 42 --force", false, None, &mut fork).await.unwrap();
    assert_eq!(fork.writes.len(), 1);
}
