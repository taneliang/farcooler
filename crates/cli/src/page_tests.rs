use clap::Parser;
use farcooler_transport::ClientError;
use uuid::Uuid;

use super::*;
use crate::id_bytes;

const REPO: Uuid = Uuid::from_u128(0x0101);
const WORKSPACE: Uuid = Uuid::from_u128(0x0202);
const NOW: i64 = 1_791_151_320_000 + 12 * 60_000;

fn fixtures() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures")
}

/// A fixture's text: a page the orchestrator would publish.
fn page_file(name: &str) -> String {
    std::fs::read_to_string(fixtures().join("pages").join(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// `--file` as the tests read it: a name under `test/fixtures/pages`.
fn read(file: &str) -> Result<String, Failed> {
    Ok(page_file(file))
}

/// The train page as the runner stores it: the same one `farcooler_client`'s
/// test holds to `test/fixtures/page.json`.
fn the_train_page() -> pb::BoardPage {
    let doc = std::fs::read_to_string(fixtures().join("pages/normalized/train.json")).unwrap();
    pb::BoardPage {
        id: id_bytes(Uuid::from_u128(0x3001)),
        slot: "train".into(),
        title: "Train integ-10".into(),
        summary: "In review · 3 of 4 lanes green".into(),
        anchor_kind: "theme".into(),
        anchor: Uuid::from_u128(0x1001).to_string(),
        doc_json: serde_json::to_string(&serde_json::from_str::<Value>(&doc).unwrap()).unwrap(),
        revision: 3,
        ordinal: 0,
        actor: "manager".into(),
        updated_at_ms: 1_791_151_320_000,
    }
}

fn task(n: u128, key: &str, status: pb::TaskStatus) -> pb::Task {
    pb::Task {
        id: id_bytes(Uuid::from_u128(0x1000 + n)),
        key: key.into(),
        title: format!("Title of {key}"),
        status: status as i32,
        workspace_id: id_bytes(WORKSPACE),
        repository_id: id_bytes(REPO),
        ..Default::default()
    }
}

fn lane(name: &str, state: pb::LaneState) -> pb::Lane {
    pb::Lane { name: name.into(), state: state as i32, spend: Some(Default::default()), ..Default::default() }
}

/// The plan the train page's references read: three lanes in review and one fixing.
fn the_plan() -> pb::Plan {
    let mut fixing = lane("ov-181-review", pb::LaneState::Fixing);
    fixing.fix_rounds = 1;
    pb::Plan {
        lanes: vec![
            lane("ov-274-phones", pb::LaneState::Review),
            lane("ov-199-lfs", pb::LaneState::Review),
            lane("ov-275-cost", pb::LaneState::Review),
            fixing,
        ],
        themes: vec![pb::BoardThemeView {
            theme: Some(pb::BoardTheme { id: id_bytes(Uuid::from_u128(0x1001)), name: "Visual language".into(), ..Default::default() }),
            counts: Some(pb::PlanStatusCounts { done: 3, backlog: 7, ..Default::default() }),
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// A runner that answers the reads from fixed state, echoes a write as the
/// runner would, and records what it was sent.
struct Runner {
    capabilities: Vec<String>,
    sent: Vec<pb::Request>,
    /// What every page call is refused with, when set: code, what, message.
    refuse: Option<(pb::ErrorCode, &'static str, &'static str)>,
    /// What `page.set` answers as changed.
    changed: bool,
    /// What `page.list` answers.
    pages: Vec<pb::BoardPage>,
}

fn runner() -> Runner {
    Runner {
        capabilities: ["workstreams", "tasks", capability::BOARD_PAGES, capability::BOARD_PLAN].map(String::from).to_vec(),
        sent: vec![],
        refuse: None,
        changed: true,
        pages: vec![the_train_page()],
    }
}

impl DispatchLink for Runner {
    fn capabilities(&self) -> Vec<String> {
        self.capabilities.clone()
    }
    async fn call(&mut self, req: pb::Request) -> Result<pb::Result, ClientError> {
        let method = req.method.clone();
        let payload = req.payload.clone();
        self.sent.push(req);
        if let Some((code, what, message)) = self.refuse
            && method.starts_with("page.")
        {
            return Err(ClientError::Daemon { code: code as i32, retryable: false, message: message.into(), what: what.into() });
        }
        Ok(pb::Result {
            value: Some(match (method.as_str(), payload) {
                ("page.list", Some(request::Payload::PageList(p))) => {
                    let mut pages = self.pages.clone();
                    if !p.with_docs {
                        pages.iter_mut().for_each(|p| p.doc_json.clear());
                    }
                    result::Value::BoardPageList(pb::BoardPageList { pages })
                }
                ("page.get", _) => result::Value::BoardPage(the_train_page()),
                ("page.set", Some(request::Payload::PageSet(p))) => {
                    let mut page = the_train_page();
                    page.slot = p.slot;
                    page.revision = if self.changed { 4 } else { 3 };
                    result::Value::PageSetResult(pb::PageSetResult { page: Some(page), changed: self.changed })
                }
                ("page.remove", _) => result::Value::BoardPage(the_train_page()),
                ("page.stats", _) => result::Value::PageStatsList(pb::PageStatsList {
                    slots: vec![pb::PageSlotStats {
                        slot: "train".into(),
                        sets: 36,
                        removes: 1,
                        first_at_ms: NOW - 86_400_000,
                        last_at_ms: NOW - 5 * 60_000,
                        shape: vec![
                            pb::PageShapeCount { kind: "ref-lane".into(), count: 252 },
                            pb::PageShapeCount { kind: "table".into(), count: 36 },
                        ],
                    }],
                }),
                ("plan.get", _) => result::Value::Plan(the_plan()),
                ("task.list", _) => result::Value::TaskList(pb::TaskList {
                    items: vec![
                        task(274, "ov-274", pb::TaskStatus::NeedsDecision),
                        task(199, "ov-199", pb::TaskStatus::InReview),
                        task(275, "ov-275", pb::TaskStatus::InReview),
                        task(181, "ov-181", pb::TaskStatus::InProgress),
                    ],
                    reads: None,
                }),
                (other, _) => panic!("sent {other}"),
            }),
        })
    }
    async fn pause(&mut self, _wait: std::time::Duration) {}
}

fn the_board() -> Board {
    Board {
        repository: REPO,
        workspace: Some(pb::Workspace { id: id_bytes(WORKSPACE), repository_id: id_bytes(REPO), ..Default::default() }),
        has_workspaces: true,
    }
}

fn parsed(args: &str) -> PageCmd {
    let argv = format!("farcooler page {args} --workspace main");
    crate::Cli::try_parse_from(argv.split_whitespace().map(String::from).collect::<Vec<_>>())
        .map(|cli| match cli.command {
            crate::Command::Page(a) => a.cmd,
            _ => panic!("not page"),
        })
        .unwrap_or_else(|e| panic!("{argv}: {e}"))
}

async fn say_as(link: &mut Runner, args: &str, json: bool) -> Result<String, Failed> {
    run_on(link, &the_board(), parsed(args), "manager", json, NOW, &read).await
}

async fn say(link: &mut Runner, args: &str) -> Result<String, Failed> {
    say_as(link, args, false).await
}

fn last(link: &Runner) -> &pb::Request {
    link.sent.last().expect("something was sent")
}

fn methods(link: &Runner) -> Vec<&str> {
    link.sent.iter().map(|r| r.method.as_str()).collect()
}

// ---- no runner ----

/// `check` needs no runner, and says where a document is wrong: the JSON path
/// and the limit, from a nonzero exit.
#[tokio::test]
async fn check_reads_a_document_with_no_runner() {
    let mut link = runner();
    link.capabilities.clear();
    let ok = say(&mut link, "check --file train.json").await.unwrap();
    assert_eq!(
        ok,
        "This page is valid: 10 blocks, 17 references, 2917 bytes. `page check` can't tell whether references resolve; `page set` does."
            .replace("2917", &farcooler_core::page_doc::parse(&page_file("train.json"), &Caps::default()).unwrap().to_json().len().to_string())
    );
    let err = say(&mut link, "check --file refused/table-rows.json").await.unwrap_err();
    assert_eq!(err.to_string(), "blocks[0].rows[50]: a table has at most 50 rows.");
    let err = say(&mut link, "check --file refused/unknown-block.json").await.unwrap_err();
    assert!(err.to_string().starts_with("blocks[0].type: there's no block called diagram."), "{err}");
    assert!(link.sent.is_empty(), "nothing was sent: {:?}", methods(&link));
}

#[tokio::test]
async fn schema_prints_the_blocks_and_as_json_the_schema() {
    let mut link = runner();
    let text = say(&mut link, "schema").await.unwrap();
    for block in farcooler_core::page_doc::BLOCK_TYPES {
        assert!(text.contains(&format!("  {block}: ")), "{block}");
    }
    assert!(text.lines().count() <= 60);
    let schema: Value = serde_json::from_str(&say_as(&mut link, "schema", true).await.unwrap()).unwrap();
    assert_eq!(schema["properties"]["v"]["const"], 1);
    assert!(link.sent.is_empty());
}

/// The CLI checks a file the way the runner will: through the same module.
#[test]
fn the_cli_and_the_daemon_share_one_validator() {
    let theirs = farcooler_core::page_doc::parse(&page_file("blocks.json"), &Caps::default()).unwrap();
    assert_eq!(theirs.blocks.len(), 9);
    assert!(check("not json").unwrap_err().to_string().starts_with("That isn't valid JSON"));
}

// ---- a runner without pages ----

/// A runner without pages is told so before anything is sent, with the
/// sentence the design gives.
#[tokio::test]
async fn an_old_runner_is_told_before_anything_is_sent() {
    let mut link = runner();
    link.capabilities.retain(|c| c != capability::BOARD_PAGES);
    for args in ["list", "show train", "set train --file train.json", "rm train", "stats"] {
        let err = say(&mut link, args).await.unwrap_err();
        assert_eq!(err.to_string(), "This runner needs an update to show pages.", "{args:?}");
    }
    assert!(link.sent.is_empty(), "sent {:?}", methods(&link));
    assert_eq!(needs_pages(&link.capabilities()).unwrap_err().to_string(), NEEDS_UPDATE);
}

// ---- set ----

/// A page is published with the document as written, the actor named, and the
/// capability on the request so an older runner refuses.
#[tokio::test]
async fn set_publishes_the_document_naming_the_actor_and_the_capability() {
    let mut link = runner();
    let said = say(&mut link, "set train --file train.json").await.unwrap();
    assert_eq!(said, "Published train, revision 4.");
    assert_eq!(methods(&link), ["page.set"]);
    let r = last(&link);
    assert_eq!(r.required_capabilities, [capability::BOARD_PAGES.to_string()]);
    let Some(request::Payload::PageSet(p)) = &r.payload else { panic!("{r:?}") };
    assert_eq!((p.slot.as_str(), p.actor.as_str()), ("train", "manager"));
    assert_eq!(p.doc_json, page_file("train.json"), "the runner is sent what was written, and validates it itself");
    assert_eq!((p.anchor_theme_id.is_none(), p.if_revision, p.workspace_id.as_ref()), (true, None, id_bytes(WORKSPACE).as_ref()));
}

#[tokio::test]
async fn set_says_when_nothing_changed() {
    let mut link = runner();
    link.changed = false;
    assert_eq!(
        say(&mut link, "set train --file train.json").await.unwrap(),
        "train is already up to date at revision 3. Nothing was published."
    );
    let json: Value = serde_json::from_str(&say_as(&mut link, "set train --file train.json", true).await.unwrap()).unwrap();
    assert_eq!((json["slot"].as_str(), json["changed"].as_bool(), json["page"]["revision"].as_u64()), (Some("train"), Some(false), Some(3)));
}

/// A document that can't be published is refused here, with its path, and
/// never sent.
#[tokio::test]
async fn set_refuses_a_bad_document_before_sending_it() {
    let mut link = runner();
    let err = say(&mut link, "set train --file refused/http-link.json").await.unwrap_err();
    assert_eq!(err.to_string(), "blocks[0].items[0].url: a link has to start with https://. Pages open no other kind.");
    let err = say(&mut link, "set Not-A-Slot --file train.json").await.unwrap_err();
    assert_eq!(err.to_string(), "A page's slot is lowercase letters, digits and hyphens, at most 40 characters.");
    assert!(link.sent.is_empty(), "sent {:?}", methods(&link));
}

/// `--theme` finds the theme by name (or short id) in the plan and sends its
/// id; `--no-theme` sends an empty one; neither sends none.
#[tokio::test]
async fn set_anchors_to_a_theme_by_name_and_can_let_go() {
    let mut link = runner();
    say(&mut link, "set risks --file risks.json --theme visual").await.unwrap();
    assert_eq!(methods(&link), ["plan.get", "page.set"]);
    let Some(request::Payload::PageSet(p)) = &last(&link).payload else { panic!() };
    assert_eq!(p.anchor_theme_id, Some(id_bytes(Uuid::from_u128(0x1001))), "a unique start of the name is enough");

    let mut link = runner();
    say(&mut link, "set risks --file risks.json --no-theme").await.unwrap();
    let Some(request::Payload::PageSet(p)) = &last(&link).payload else { panic!() };
    assert_eq!(p.anchor_theme_id, Some(bytes::Bytes::new()));

    let mut link = runner();
    let err = say(&mut link, "set risks --file risks.json --theme Nope").await.unwrap_err();
    assert_eq!(err.to_string(), "No theme here is called \"Nope\".");
    assert!(!methods(&link).contains(&"page.set"));

    let mut link = runner();
    link.capabilities.retain(|c| c != capability::BOARD_PLAN);
    let err = say(&mut link, "set risks --file risks.json --theme Visual").await.unwrap_err();
    assert!(err.to_string().starts_with("This runner has no plan"), "{err}");
    assert!(link.sent.is_empty());

    assert!(
        crate::Cli::try_parse_from(["farcooler", "page", "set", "x", "--file", "f", "--theme", "T", "--no-theme"]).is_err(),
        "a page is on a theme or off it"
    );
}

#[tokio::test]
async fn set_can_insist_on_the_revision_it_read() {
    let mut link = runner();
    say(&mut link, "set train --file train.json --if-revision 3").await.unwrap();
    let Some(request::Payload::PageSet(p)) = &last(&link).payload else { panic!() };
    assert_eq!(p.if_revision, Some(3));
}

// ---- the runner's refusals ----

/// A page the runner refused is shown in the runner's own sentence, which is
/// the path and the limit; the machine word survives for `--json`.
#[tokio::test]
async fn a_page_the_runner_refused_says_why_in_its_own_words() {
    let mut link = runner();
    link.refuse = Some((pb::ErrorCode::InvalidArgument, "page", "blocks[1].items[0]: there's no lane mac-ux on this board."));
    let err = say(&mut link, "set train --file train.json").await.unwrap_err();
    assert_eq!(err.to_string(), "blocks[1].items[0]: there's no lane mac-ux on this board.");
    let refusal = err.downcast_ref::<Refused>().expect("a refusal");
    assert_eq!(refusal.what(), Some("page"));
    assert_eq!(refusal.word(), Some("invalid-argument"));
}

#[tokio::test]
async fn a_stale_revision_and_a_missing_page_say_what_to_do() {
    let mut link = runner();
    link.refuse = Some((pb::ErrorCode::ResourceConflict, "", ""));
    let err = say(&mut link, "set train --file train.json --if-revision 1").await.unwrap_err();
    assert_eq!(err.to_string(), "This page changed since you read it. Read it again with `farcooler page show train`, then publish.");
    link.refuse = Some((pb::ErrorCode::NotFound, "", ""));
    let err = say(&mut link, "show nope").await.unwrap_err();
    assert_eq!(err.to_string(), "No page here is in the slot nope.");
    let err = say(&mut link, "rm nope").await.unwrap_err();
    assert_eq!(err.to_string(), "No page here is in the slot nope.");
}

// ---- show ----

/// The page as the apps draw it: the header says who and when, the references
/// are drawn live from the board, and the only attention is on what needs it.
#[tokio::test]
async fn show_draws_the_page_with_its_references_live() {
    let mut link = runner();
    link.pages.push(pb::BoardPage { slot: "spend".into(), title: "Spend".into(), ..Default::default() });
    let text = say(&mut link, "show train").await.unwrap();
    assert_eq!(methods(&link), ["page.get", "task.list", "plan.get", "page.list"]);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "Train integ-10");
    assert_eq!(lines[1], "In review · 3 of 4 lanes green");
    assert_eq!(lines[2], "Updated 12 min ago by manager · revision 3 · in theme Visual language");
    assert!(text.contains("Lanes: 4 · Green: 3 · Fixing: 1 (attention) · Build: 41 min (one Mac slot)"), "{text}");
    assert!(text.contains("Pick (Done) > Build (Done) > Review (Active) > Land (To do)"), "{text}");
    let table: Vec<&&str> = lines.iter().filter(|l| l.contains("ov-27") && l.contains("iOS UI class")).collect();
    assert_eq!(table.len(), 1, "{text}");
    assert!(table[0].starts_with("ov-274-phones  ov-274 Title of ov-274 (Needs Decision)"), "{}", table[0]);
    assert!(table[0].ends_with("In review"), "a lane's state is live: {}", table[0]);
    assert!(text.contains("Fixing · round 1 (attention)"), "{text}");
    assert!(text.contains("- [Waiting] Owner: should Plan hide Unread on phones? -> Needs you (attention) (ask ov-274)"), "{text}");
    assert!(text.contains("- [Active] CI on the pushed branch · run 812 -> github.com (https://github.com/example/overnight/actions/runs/812)"), "{text}");
    assert!(text.contains("Cards closed: 2 of 4 [#####-----]"), "{text}");
    assert!(text.contains("Build terminal [terminal integ-10/build]; Spend [page spend]; Visual language, 3 of 10 done [theme Visual language]"), "{text}");
    assert!(!text.contains("<"), "no markup reaches the terminal");
}

/// A reference the board can't resolve is plain text and never an error, and a
/// runner with no plan draws its lanes by name.
#[tokio::test]
async fn an_unresolved_reference_is_plain_text() {
    let mut link = runner();
    link.capabilities.retain(|c| c != capability::BOARD_PLAN);
    let text = say(&mut link, "show train").await.unwrap();
    assert_eq!(methods(&link), ["page.get", "task.list", "page.list"], "no plan to ask");
    assert!(text.contains("spend [page spend]"), "a page that isn't there is its slot: {text}");
    let row = text.lines().find(|l| l.contains("iOS UI class")).unwrap();
    assert!(row.starts_with("ov-274-phones"), "{row}");
    assert!(row.ends_with("ov-274-phones"), "a lane's state is its name without a plan: {row}");
    assert!(text.contains("Visual language [theme Visual language]"), "{text}");
    assert!(!text.contains("in theme"), "the anchor's name can't be read without a plan: {text}");
}

/// A page past its `stale_after_min` says so in the header.
#[tokio::test]
async fn a_stale_page_says_how_long() {
    let mut link = runner();
    let text = run_on(&mut link, &the_board(), parsed("show train"), "manager", false, NOW + 3 * 3_600_000, &read).await.unwrap();
    assert!(text.lines().nth(2).unwrap().ends_with("· not updated for 3 h"), "{text}");
}

/// What `show --json` prints is the one fixture the apps read.
#[tokio::test]
async fn show_json_is_the_fixture_the_apps_read() {
    let mut link = runner();
    let printed: Value = serde_json::from_str(&say_as(&mut link, "show train", true).await.unwrap()).unwrap();
    let fixture: Value = serde_json::from_str(&std::fs::read_to_string(fixtures().join("page.json")).unwrap()).unwrap();
    assert_eq!(printed, fixture);
    assert_eq!(methods(&link), ["page.get"], "`--json` is the record, not a rendering");
}

// ---- list, rm, stats ----

#[tokio::test]
async fn list_says_slot_title_theme_when_and_revision() {
    let mut link = runner();
    let text = say(&mut link, "list").await.unwrap();
    assert_eq!(
        text.lines().collect::<Vec<_>>(),
        ["Slot   Title           Theme            Updated     Rev", "train  Train integ-10  Visual language  12 min ago  3"]
    );
    let Some(request::Payload::PageList(p)) = &link.sent[0].payload else { panic!() };
    assert!(!p.with_docs, "the list doesn't read every document");
    link.pages.clear();
    assert_eq!(say(&mut link, "list").await.unwrap(), NOTHING_PUBLISHED);
    let json: Value = serde_json::from_str(&say_as(&mut link, "list", true).await.unwrap()).unwrap();
    assert_eq!(json["pages"], json!([]));
}

#[tokio::test]
async fn rm_removes_and_names_the_actor() {
    let mut link = runner();
    assert_eq!(say(&mut link, "rm train").await.unwrap(), "Removed train.");
    let r = last(&link);
    assert_eq!(r.required_capabilities, [capability::BOARD_PAGES.to_string()]);
    let Some(request::Payload::PageRemove(p)) = &r.payload else { panic!() };
    assert_eq!((p.slot.as_str(), p.actor.as_str()), ("train", "manager"));
}

#[tokio::test]
async fn stats_say_how_often_and_with_which_blocks() {
    let mut link = runner();
    assert_eq!(
        say(&mut link, "stats").await.unwrap(),
        "train  36 writes, 1 removal · last 5 min ago · ref-lane 252, table 36"
    );
    let Some(request::Payload::PageStats(p)) = &last(&link).payload else { panic!() };
    assert_eq!(p.since_ms, NOW - 14 * 86_400_000, "two weeks by default");
    say(&mut link, "stats --since 12h").await.unwrap();
    let Some(request::Payload::PageStats(p)) = &last(&link).payload else { panic!() };
    assert_eq!(p.since_ms, NOW - 12 * 3_600_000);
    let json: Value = serde_json::from_str(&say_as(&mut link, "stats", true).await.unwrap()).unwrap();
    assert_eq!(json["slots"][0]["shape"]["table"], 36);
    for bad in ["14", "d", "0d", "-3d", "14w", "soon"] {
        let err = duration_ms(bad).unwrap_err();
        assert_eq!(err.to_string(), "Use a number and d, h or m for --since, like 14d.", "{bad}");
    }
}

/// Every verb is under `page`, and the bare word asks for one.
#[test]
fn the_verbs_parse() {
    for args in ["list", "show s", "set s --file f", "check --file f", "rm s", "schema", "stats", "stats --since 3d"] {
        let argv: Vec<String> = format!("farcooler page {args}").split_whitespace().map(String::from).collect();
        assert!(crate::Cli::try_parse_from(argv).is_ok(), "{args}");
    }
    assert!(crate::Cli::try_parse_from(["farcooler", "page"]).is_err(), "a verb is needed");
    assert!(crate::Cli::try_parse_from(["farcooler", "page", "set", "s"]).is_err(), "--file is needed");
}
