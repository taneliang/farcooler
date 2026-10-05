use clap::Parser;
use farcooler_transport::ClientError;
use uuid::Uuid;

use super::*;
use crate::id_bytes;

const REPO: Uuid = Uuid::from_u128(0x0101);
const WORKSPACE: Uuid = Uuid::from_u128(0x0202);
const OTHER_BOARD: Uuid = Uuid::from_u128(0x0203);
const NOW: i64 = 1_800_000_000_000;
const HOUR: i64 = 3_600_000;

fn task(n: u128, key: &str, status: pb::TaskStatus, board: Uuid) -> pb::Task {
    pb::Task {
        id: id_bytes(Uuid::from_u128(0x1000 + n)),
        key: key.into(),
        title: format!("Title of {key}"),
        status: status as i32,
        workspace_id: id_bytes(board),
        repository_id: id_bytes(REPO),
        ..Default::default()
    }
}

fn items() -> Vec<pb::Task> {
    vec![
        task(1, "ov-1", pb::TaskStatus::InProgress, WORKSPACE),
        task(2, "ov-2", pb::TaskStatus::Backlog, WORKSPACE),
        task(3, "ov-3", pb::TaskStatus::Done, WORKSPACE),
    ]
}

fn lane(n: u128, name: &str, state: pb::LaneState, cards: &[u128]) -> pb::Lane {
    pb::Lane {
        id: id_bytes(Uuid::from_u128(0x2000 + n)),
        workspace_id: id_bytes(WORKSPACE),
        name: name.into(),
        state: state as i32,
        state_since: NOW - 10 * 60_000,
        cards: cards.iter().map(|c| pb::LaneCard { task_id: id_bytes(Uuid::from_u128(0x1000 + c)), slice: String::new() }).collect(),
        spend: Some(Default::default()),
        ..Default::default()
    }
}

fn theme(n: u128, name: &str, cards: &[u128], done: u32) -> pb::BoardThemeView {
    pb::BoardThemeView {
        theme: Some(pb::BoardTheme {
            id: id_bytes(Uuid::from_u128(0x3000 + n)),
            workspace_id: id_bytes(WORKSPACE),
            name: name.into(),
            state: pb::BoardThemeState::Active as i32,
            ..Default::default()
        }),
        task_ids: cards.iter().map(|c| id_bytes(Uuid::from_u128(0x1000 + c))).collect(),
        counts: Some(pb::PlanStatusCounts { done, backlog: cards.len() as u32 - done, ..Default::default() }),
        spend: None,
    }
}

/// Two rulings (ov-304): R-2 standing on ov-1 and the theme, R-1 confirmed.
/// The client's `plan_json` test builds the same two, so one fixture holds both.
fn rulings() -> Vec<pb::BoardRuling> {
    vec![
        pb::BoardRuling {
            id: id_bytes(Uuid::from_u128(0x5002)),
            workspace_id: id_bytes(WORKSPACE),
            number: 2,
            decision: "The inbox is amber.".into(),
            why: "It's the one attention color, so the inbox reads as needing you.".into(),
            reversal: "One token; every surface follows.".into(),
            task_ids: vec![id_bytes(Uuid::from_u128(0x1001))],
            task_keys: vec!["ov-1".into()],
            theme_id: Some(id_bytes(Uuid::from_u128(0x3001))),
            state: pb::BoardRulingState::Standing as i32,
            actor: "manager".into(),
            created_at: NOW - HOUR,
            resource_version: 1,
            ..Default::default()
        },
        pb::BoardRuling {
            id: id_bytes(Uuid::from_u128(0x5001)),
            workspace_id: id_bytes(WORKSPACE),
            number: 1,
            decision: "Unread stays on the phones.".into(),
            why: "The owner reads there first.".into(),
            reversal: "A setting and two screens.".into(),
            state: pb::BoardRulingState::Confirmed as i32,
            note: "Keep it.".into(),
            actor: "manager".into(),
            created_at: NOW - 5 * HOUR,
            settled_by: Some("manager".into()),
            settled_at: Some(NOW - 2 * HOUR),
            resource_version: 2,
            ..Default::default()
        },
    ]
}

/// A train (ov-309): integ-9, red, carrying mac-ux, and the runner's CI read
/// of its SHA. The client's `plan_json` test builds the same, so one fixture
/// holds both.
fn trains() -> (Vec<pb::BoardTrain>, Vec<pb::BoardCiRead>) {
    let job = |name: &str, state: &str| pb::BoardCiJob { name: name.into(), state: state.into(), url: String::new() };
    let train = pb::BoardTrain {
        id: tid(0x6001),
        workspace_id: tid(0x0202),
        name: "integ-9".into(),
        base: "origin/main".into(),
        pushed_sha: Some("c85bf83d".into()),
        state: pb::BoardTrainState::Red as i32,
        state_since: NOW - 20 * 60_000,
        actor: "manager".into(),
        created_at: NOW - 2 * HOUR,
        landed_at: None,
        lane_ids: vec![tid(0x2002)],
        ci_subject: "sha:c85bf83d".into(),
        resource_version: 3,
    };
    let read = pb::BoardCiRead {
        subject: "sha:c85bf83d".into(),
        sha: "c85bf83dce46a6b71d7312afc623899ae7914658".into(),
        status: pb::BoardCiStatus::Failed as i32,
        url: "https://github.com/taneliang/farcooler/actions/runs/37275435256".into(),
        jobs: vec![job("CI / Swift (shared + macOS)", "failed"), job("CI / Android", "passed"), job("Canary", "passed")],
        fetched_at: NOW - 60_000,
        changed_at: NOW - 20 * 60_000,
        asked_at: NOW - 60_000,
    };
    (vec![train], vec![read])
}

fn tid(n: u128) -> bytes::Bytes {
    id_bytes(Uuid::from_u128(n))
}

/// What the runner holds: a plan, and the board's cards.
fn the_plan() -> pb::Plan {
    let mut queued = lane(1, "mac-fu3", pb::LaneState::Queued, &[2]);
    queued.plan_rank = Some(1);
    queued.reason = "Frees the Mac slot".into();
    let mut review = lane(2, "mac-ux", pb::LaneState::Review, &[1]);
    review.train = Some("integ-9".into());
    review.spend = Some(pb::LaneSpend { input_tokens: 300_000, output_tokens: 170_000, runs: 2, ..Default::default() });
    let mut landed = lane(3, "fix-ac84", pb::LaneState::Landed, &[3]);
    landed.state_since = NOW - HOUR;
    pb::Plan {
        now_ms: NOW,
        themes: vec![pb::BoardThemeView { spend: Some(theme_spend()), ..theme(1, "Visual language", &[1, 2, 3], 1) }],
        order: vec![queued.id.clone()],
        lanes: vec![queued, review, landed],
        cards: items()
            .iter()
            .map(|t| pb::PlanCard { task_id: t.id.clone(), key: t.key.clone(), title: t.title.clone(), status: t.status })
            .collect(),
        coverage: vec![
            pb::PlanCoverage { task_id: items()[0].id.clone(), live: 1, landed: 0 },
            pb::PlanCoverage { task_id: items()[1].id.clone(), live: 1, landed: 0 },
            pb::PlanCoverage { task_id: items()[2].id.clone(), live: 0, landed: 1 },
        ],
        rulings: rulings(),
        trains: trains().0,
        ci: trains().1,
        board_counts: Some(board_counts()),
    }
}

/// The theme's share of its lanes' spend, and the board's counts (ov-306).
/// The client's `plan_json` test builds the same.
fn theme_spend() -> pb::LaneSpend {
    pb::LaneSpend { input_tokens: 235_000, output_tokens: 85_000, cost_micros: Some(15_500_000), runs: 2, ..Default::default() }
}

fn board_counts() -> pb::PlanStatusCounts {
    pb::PlanStatusCounts { backlog: 4, in_progress: 2, in_review: 3, done: 11, ..Default::default() }
}

/// A runner that answers the reads from fixed state, echoes each write as the
/// layer would, and records what it was sent.
struct Runner {
    capabilities: Vec<String>,
    sent: Vec<pb::Request>,
    refuse: Option<&'static str>,
    /// What `plan.events` answers, for a theme or a lane alike.
    events: Vec<pb::PlanEvent>,
}

fn runner() -> Runner {
    Runner {
        capabilities: ["workstreams", "tasks", capability::BOARD_PLAN, capability::BOARD_RULINGS, capability::BOARD_TRAINS]
            .map(String::from)
            .to_vec(),
        sent: vec![],
        refuse: None,
        events: vec![],
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
        if let Some(what) = self.refuse
            && method != "plan.get"
            && method != "task.list"
        {
            return Err(ClientError::Daemon {
                code: pb::ErrorCode::InvalidArgument as i32,
                retryable: false,
                message: String::new(),
                what: what.into(),
            });
        }
        let plan = the_plan();
        Ok(pb::Result {
            value: Some(match (method.as_str(), payload) {
                ("plan.get", _) => result::Value::Plan(plan),
                ("task.list", _) => result::Value::TaskList(pb::TaskList { items: items(), reads: None }),
                ("plan.events", _) => result::Value::PlanEventList(pb::PlanEventList { events: self.events.clone() }),
                ("plan.set", Some(request::Payload::PlanSet(p))) => result::Value::Plan(pb::Plan {
                    order: p.lane_ids,
                    ..plan
                }),
                ("board_theme.create" | "board_theme.update" | "board_theme.cards", _) => {
                    result::Value::BoardThemeView(plan.themes[0].clone())
                }
                ("lane.create", Some(request::Payload::LaneCreate(p))) => {
                    let mut made = lane(9, &p.name, pb::LaneState::Queued, &[]);
                    if p.agent.is_some() {
                        made.state = pb::LaneState::Building as i32;
                    }
                    made.cards = p.cards;
                    result::Value::Lane(made)
                }
                ("lane.update", Some(request::Payload::LaneUpdate(p))) => {
                    let mut moved = plan.lanes.into_iter().find(|l| l.id == p.lane_id).unwrap();
                    if let Some(state) = p.state {
                        moved.state = state;
                    }
                    result::Value::Lane(moved)
                }
                ("lane.cards", _) => result::Value::Lane(plan.lanes[1].clone()),
                ("train.start", Some(request::Payload::TrainStart(p))) => result::Value::BoardTrain(pb::BoardTrain {
                    name: p.name,
                    base: p.base,
                    lane_ids: p.lane_ids,
                    state: pb::BoardTrainState::Integrating as i32,
                    state_since: NOW,
                    ..Default::default()
                }),
                ("train.set", Some(request::Payload::TrainSet(p))) => {
                    let mut moved = plan.trains.into_iter().find(|t| t.id == p.train_id).unwrap();
                    if let Some(sha) = p.sha {
                        moved.ci_subject = format!("sha:{sha}");
                        moved.pushed_sha = Some(sha);
                        moved.state = pb::BoardTrainState::Pushed as i32;
                    }
                    if p.state != 0 {
                        moved.state = p.state;
                    }
                    result::Value::BoardTrain(moved)
                }
                ("ruling.add", Some(request::Payload::RulingAdd(p))) => result::Value::BoardRuling(pb::BoardRuling {
                    number: 3,
                    decision: p.decision,
                    why: p.why,
                    reversal: p.reversal,
                    task_ids: p.task_ids,
                    theme_id: p.theme_id,
                    state: pb::BoardRulingState::Standing as i32,
                    actor: p.actor,
                    created_at: NOW,
                    ..Default::default()
                }),
                ("ruling.set", Some(request::Payload::RulingSet(p))) => {
                    let mut set = plan.rulings.into_iter().find(|r| r.id == p.ruling_id).unwrap();
                    set.state = p.state;
                    set.note = p.note.unwrap_or_default();
                    result::Value::BoardRuling(set)
                }
                ("task.get_by_key", _) => result::Value::TaskList(pb::TaskList {
                    items: vec![task(40, "ov-40", pb::TaskStatus::Backlog, OTHER_BOARD)],
                    reads: None,
                }),
                ("repository.list", _) => result::Value::RepositoryList(pb::RepositoryList {
                    items: vec![pb::Repository { id: id_bytes(REPO), display_name: "overnight".into(), ..Default::default() }],
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

fn parsed(args: &str) -> Option<PlanCmd> {
    let argv = format!("farcooler plan {args} --workspace main");
    crate::Cli::try_parse_from(argv.split_whitespace().map(String::from).collect::<Vec<_>>())
        .map(|cli| match cli.command {
            crate::Command::Plan(a) => a.cmd,
            _ => panic!("not plan"),
        })
        .unwrap_or_else(|e| panic!("{argv}: {e}"))
}

async fn say(link: &mut Runner, args: &str) -> Result<String, Failed> {
    run_on(link, &the_board(), parsed(args), "manager", false, NOW).await
}

fn last(link: &Runner) -> &pb::Request {
    link.sent.last().expect("something was sent")
}

/// A runner without the layer is told so before anything is sent, with the
/// sentence the design gives.
#[tokio::test]
async fn an_old_runner_is_told_before_anything_is_sent() {
    let mut link = runner();
    link.capabilities.retain(|c| c != capability::BOARD_PLAN);
    for args in ["", "set a", "theme list", "lane list", "lane start x"] {
        let err = say(&mut link, args).await.unwrap_err();
        assert_eq!(err.to_string(), "This runner needs an update to keep a plan.", "{args:?}");
    }
    assert!(link.sent.is_empty(), "sent {:?}", link.sent.iter().map(|r| &r.method).collect::<Vec<_>>());
    assert_eq!(needs_the_layer(&link.capabilities()).unwrap_err().to_string(), NEEDS_UPDATE);
}

/// Every layer request names the capability, so an older runner refuses
/// rather than dropping what it can't read.
#[tokio::test]
async fn every_write_names_the_capability_and_the_actor() {
    let cases = [
        "set mac-fu3",
        "theme create Reliability --outcome Done. --card ov-2",
        "theme set Visual --next Ship",
        "theme cards Visual --add ov-2",
        "lane start mac-new --card ov-2:Mac --agent a1",
        "lane set mac-ux --state landing",
        "lane cards mac-ux --add ov-2",
    ];
    for args in cases {
        let mut link = runner();
        say(&mut link, args).await.unwrap_or_else(|e| panic!("{args}: {e}"));
        let r = last(&link);
        assert_eq!(r.required_capabilities, [capability::BOARD_PLAN.to_string()], "{args}");
        let actor = match r.payload.as_ref().unwrap() {
            request::Payload::PlanSet(p) => &p.actor,
            request::Payload::BoardThemeCreate(p) => &p.actor,
            request::Payload::BoardThemeUpdate(p) => &p.actor,
            request::Payload::BoardThemeCards(p) => &p.actor,
            request::Payload::LaneCreate(p) => &p.actor,
            request::Payload::LaneUpdate(p) => &p.actor,
            request::Payload::LaneCards(p) => &p.actor,
            other => panic!("{other:?}"),
        };
        assert_eq!(actor, "manager", "{args}: a write names who it's from, even `user`");
    }
}

/// The overview is what the Mac draws: next up first with each reason, then
/// what's running, then the themes, then what landed today.
#[tokio::test]
async fn the_overview_reads_next_up_now_themes_and_landed() {
    let mut link = runner();
    let text = say(&mut link, "").await.unwrap();
    let expected = [
        "Next up",
        "  1  mac-fu3          ov-2",
        "     Frees the Mac slot",
        "Now",
        "  integ-9 · Red · c85bf83d · CI Failed · 1 of 3 jobs failed",
        "    mac-ux           In review · in integ-9 · 1 card · 470K tokens",
        "Themes",
        "  Visual language  1 of 3 done · active",
        "Decided for you",
        "  R-2    The inbox is amber.",
        "Landed today",
        "  fix-ac84",
    ];
    assert_eq!(text.lines().collect::<Vec<_>>(), expected, "{text}");
    assert_eq!(link.sent.len(), 1, "one read draws it");
    assert_eq!(link.sent[0].method, "plan.get");
}

/// A card with all its lanes landed shows up as such only while it isn't done.
#[test]
fn landed_not_closed_and_no_lane_are_derived() {
    let mut plan = the_plan();
    plan.cards[2].status = pb::TaskStatus::InProgress as i32;
    plan.cards[0].status = pb::TaskStatus::InProgress as i32;
    plan.coverage[0].live = 0;
    let flagged = checks(&plan, &Keys::of_plan(&plan));
    assert_eq!(flagged.len(), 2, "{flagged:?}");
    assert!(flagged[0].starts_with("ov-1  In Progress, and no lane"), "{flagged:?}");
    assert!(flagged[1].starts_with("ov-3  All its lanes have landed"), "{flagged:?}");
    let json = plan_json(&plan, &Keys::of_plan(&plan));
    assert_eq!(json["landed_not_closed"][0]["key"], "ov-3");
    assert_eq!(json["no_lane"][0]["key"], "ov-1");
    plan.cards[2].status = pb::TaskStatus::Done as i32;
    assert!(plan_json(&plan, &Keys::of_plan(&plan))["landed_not_closed"].as_array().unwrap().is_empty());
}

/// `plan --json` is the whole plan: the read an orchestrator makes instead of
/// `task list` plus a `task show` per card.
#[tokio::test]
async fn plan_json_is_the_whole_plan() {
    let mut link = runner();
    let out = run_on(&mut link, &the_board(), None, "manager", true, NOW).await.unwrap();
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["lanes"].as_array().unwrap().len(), 3);
    assert_eq!(v["themes"][0]["name"], "Visual language");
    assert_eq!(v["themes"][0]["cards"][0]["key"], "ov-1");
    assert_eq!(v["order"][0], Uuid::from_u128(0x2001).to_string());
    assert_eq!(v["lanes"][1]["spend"]["input_tokens"], 300_000);
    assert_eq!(v["lanes"][1]["train"], "integ-9");
    assert_eq!(v["cards"][0]["status"], "In Progress");
}

/// `lane start` makes the lane, links its cards with their slices, and records
/// the agent in one command.
#[tokio::test]
async fn lane_start_is_one_command_for_a_dispatch() {
    let mut link = runner();
    let out = say(
        &mut link,
        "lane start mac-new --card ov-1:Mac --card ov-2 --reason Because --path .claude/worktrees/x --branch x --model sonnet --agent a1",
    )
    .await
    .unwrap();
    assert_eq!(out, "Started lane mac-new (building, 2 cards).");
    let Some(request::Payload::LaneCreate(p)) = &last(&link).payload else { panic!() };
    assert_eq!(last(&link).method, "lane.create");
    assert_eq!(p.name, "mac-new");
    assert_eq!(p.cards[0].slice, "Mac");
    assert_eq!(p.cards[1].slice, "");
    assert_eq!(p.harness, "claude", "an agent without a harness is a Claude one");
    let agent = p.agent.as_ref().unwrap();
    assert_eq!((agent.agent_id.as_str(), agent.role), ("a1", pb::LaneAgentRole::Build as i32));
    assert_eq!(agent.model.as_deref(), Some("sonnet"));
    assert_eq!(p.model, "sonnet");

    // Without an agent it's a plan item, written before anything is dispatched.
    let mut link = runner();
    let out = say(&mut link, "lane start later --card ov-2 --reason Next").await.unwrap();
    assert_eq!(out, "Started lane later (queued, 1 card).");
    let Some(request::Payload::LaneCreate(p)) = &last(&link).payload else { panic!() };
    assert!(p.agent.is_none());
}

/// A reviewer moves the lane to review in the same write, and a fix agent to
/// fixing; naming no state and no agent is refused.
#[tokio::test]
async fn a_reviewer_moves_the_lane_in_the_same_write() {
    let mut link = runner();
    say(&mut link, "lane set mac-ux --agent r1 --role review --model opus").await.unwrap();
    let Some(request::Payload::LaneUpdate(p)) = &last(&link).payload else { panic!() };
    assert_eq!(p.state, None, "mac-ux is in review already: no move to ask for");
    assert_eq!(p.agent.as_ref().unwrap().role, pb::LaneAgentRole::Review as i32);

    let mut link = runner();
    let err = say(&mut link, "lane set mac-fu3 --agent r1 --role review").await.unwrap_err();
    assert_eq!(err.to_string(), "mac-fu3 is queued. It can go to building or dropped.", "the move rides with the reviewer, and a queued lane can't make it");
    assert!(link.sent.iter().all(|r| r.method != "lane.update"));

    let mut link = runner();
    say(&mut link, "lane set mac-ux --agent f1 --role fix").await.unwrap();
    let Some(request::Payload::LaneUpdate(p)) = &last(&link).payload else { panic!() };
    assert_eq!(p.state, Some(pb::LaneState::Fixing as i32));

    let err = say(&mut link, "lane set mac-ux").await.unwrap_err();
    assert!(err.to_string().starts_with("Say what to change"), "{err}");
}

/// Landing records the commit and the train; `--no-train` clears the train.
#[tokio::test]
async fn landing_records_the_sha_and_the_train() {
    let mut link = runner();
    say(&mut link, "lane set mac-ux --state landed --sha 1a1b3275 --train integ-9").await.unwrap();
    let Some(request::Payload::LaneUpdate(p)) = &last(&link).payload else { panic!() };
    assert_eq!(p.state, Some(pb::LaneState::Landed as i32));
    assert_eq!(p.landed_sha.as_deref(), Some("1a1b3275"));
    assert_eq!(p.train.as_deref(), Some("integ-9"));
    say(&mut link, "lane set mac-ux --no-train").await.unwrap();
    let Some(request::Payload::LaneUpdate(p)) = &last(&link).payload else { panic!() };
    assert_eq!(p.train.as_deref(), Some(""), "an empty train takes the lane out of it");
}

/// The plan is named by lane names, in order, and only queued lanes are
/// accepted, before anything is sent.
#[tokio::test]
async fn the_plan_takes_queued_lanes_by_name() {
    let mut link = runner();
    say(&mut link, "set mac-fu3").await.unwrap();
    let Some(request::Payload::PlanSet(p)) = &last(&link).payload else { panic!() };
    assert_eq!(p.lane_ids, [id_bytes(Uuid::from_u128(0x2001))]);

    let mut link = runner();
    let err = say(&mut link, "set mac-ux").await.unwrap_err();
    assert_eq!(err.to_string(), "Only a queued lane can be in the plan: mac-ux is in review.");
    let err = say(&mut link, "set mac-fu3 MAC-FU3").await.unwrap_err();
    assert!(err.to_string().contains("named twice"), "{err}");
    let err = say(&mut link, "set nope").await.unwrap_err();
    assert_eq!(err.to_string(), "No lane here is called \"nope\".");
    assert!(link.sent.iter().all(|r| r.method != "plan.set"), "a refusal here sends nothing");
}

/// A card on another board, or nowhere, is named as such before the write.
#[tokio::test]
async fn a_card_must_be_on_this_board() {
    let mut link = runner();
    let err = say(&mut link, "theme cards Visual --add ov-40").await.unwrap_err();
    assert_eq!(err.to_string(), "ov-40 is on another board.");
    let mut link = runner();
    link.refuse = None;
    let err = say(&mut link, "lane cards mac-ux --add ov-1 --remove ov-40").await.unwrap_err();
    assert_eq!(err.to_string(), "ov-40 is on another board.");
    assert!(link.sent.iter().all(|r| r.method != "lane.cards"));
}

/// A refusal from the runner is said in this module's words, and still carries
/// the runner's word for a script.
#[tokio::test]
async fn a_refusal_is_said_in_words_a_person_reads() {
    let mut link = runner();
    link.refuse = Some("lane_state");
    let err = say(&mut link, "lane set mac-ux --state landing").await.unwrap_err();
    assert!(err.to_string().starts_with("A lane can't make that move"), "{err}");
    let refusal = err.downcast_ref::<Refused>().expect("keeps its code");
    assert_eq!(refusal.what(), Some("lane_state"));
    for word in [
        "name", "name_taken", "other_board", "lane_state", "lane_closed", "lane_twice", "plan_state", "harness",
        "agent_id", "role", "state", "actor", "outcome", "story", "train", "subject",
    ] {
        assert!(said_here(word).is_some(), "no sentence for {word}");
    }
}

/// Themes: create with cards, replace the story, clear the ask.
#[tokio::test]
async fn a_theme_is_made_rewritten_and_asked_about() {
    let mut link = runner();
    let out = say(&mut link, "theme create Reliability --outcome Fixed. --card ov-1 --card ov-2").await.unwrap();
    assert!(out.starts_with("Made theme"), "{out}");
    let Some(request::Payload::BoardThemeCreate(p)) = &last(&link).payload else { panic!() };
    assert_eq!((p.name.as_str(), p.outcome.as_str(), p.task_ids.len()), ("Reliability", "Fixed.", 2));

    say(&mut link, "theme set visual --story Going. --next Frosted --ask Which").await.unwrap();
    let Some(request::Payload::BoardThemeUpdate(p)) = &last(&link).payload else { panic!() };
    assert_eq!(p.theme_id, id_bytes(Uuid::from_u128(0x3001)), "found by a prefix-free, case-free name");
    assert_eq!((p.story.as_deref(), p.next.as_deref(), p.owner_ask.as_deref()), (Some("Going."), Some("Frosted"), Some("Which")));

    say(&mut link, "theme set Visual --no-ask --state paused").await.unwrap();
    let Some(request::Payload::BoardThemeUpdate(p)) = &last(&link).payload else { panic!() };
    assert_eq!(p.owner_ask.as_deref(), Some(""));
    assert_eq!(p.state, Some(pb::BoardThemeState::Paused as i32));
    assert!(p.story.is_none(), "what isn't named isn't written");

    let err = say(&mut link, "theme set Visual").await.unwrap_err();
    assert!(err.to_string().starts_with("Say what to change"), "{err}");
}

/// `lane show` and `theme show` read the timeline too.
#[tokio::test]
async fn show_reads_the_timeline() {
    let mut link = runner();
    let text = say(&mut link, "lane show mac-ux").await.unwrap();
    assert!(text.starts_with("mac-ux · In review · in integ-9 · 1 card · 470K tokens"), "{text}");
    assert!(link.sent.iter().any(|r| r.method == "plan.events"));
    let text = say(&mut link, "theme show Visual").await.unwrap();
    assert!(text.starts_with("Visual language · active"), "{text}");
    assert!(text.contains("Lanes\n  mac-fu3") && text.contains("  mac-ux "), "{text}");
}

/// Spend says "Not reported" rather than zero for a lane nothing measured.
#[test]
fn an_unmeasured_lane_says_not_reported() {
    assert_eq!(spend_words(&pb::LaneSpend::default()), "Not reported");
    let some = pb::LaneSpend { input_tokens: 1_500, runs: 1, unmeasured_agents: 1, ..Default::default() };
    assert_eq!(spend_words(&some), "1.5K tokens · 1 agent not reported");
}

/// Dollars follow tokens, and a card's part of a lane is labeled a share.
#[test]
fn spend_puts_tokens_first_and_a_card_s_part_is_a_share() {
    let s = pb::LaneSpend { input_tokens: 3_000, output_tokens: 1_000, cost_micros: Some(40_000), runs: 1, ..Default::default() };
    assert_eq!(spend_words(&s), "4K tokens · $0.04 API-equivalent");
    assert_eq!(
        share_words(&s, 2).as_deref(),
        Some("About 2K tokens · $0.02 API-equivalent a card, the lane\u{2019}s spend split evenly across 2 cards")
    );
    assert_eq!(share_words(&s, 1), None, "one card has the lane\u{2019}s whole spend, not a share of it");
    assert_eq!(share_words(&pb::LaneSpend::default(), 3), None, "nothing measured is never a share of zero");
}

/// A lane holding part of an agent shared with other lanes says so, after
/// what wasn't reported (review 1004j P1).
#[test]
fn a_lane_with_a_shared_agent_says_its_figure_is_a_split() {
    let one = pb::LaneSpend { input_tokens: 2_000, runs: 1, shared_agents: 1, ..Default::default() };
    assert_eq!(spend_words(&one), "2K tokens · 1 agent\u{2019}s spend split with other lanes");
    let two = pb::LaneSpend { input_tokens: 2_000, runs: 1, unmeasured_agents: 1, shared_agents: 2, ..Default::default() };
    assert_eq!(spend_words(&two), "2K tokens · 1 agent not reported · 2 agents\u{2019} spend split with other lanes");
}

/// A lane stuck for over an hour says how long.
#[test]
fn a_stale_lane_says_how_long() {
    let mut l = lane(1, "a", pb::LaneState::Building, &[1]);
    l.state_since = NOW - 3 * HOUR;
    l.stale = true;
    assert!(lane_status(&l, NOW).ends_with("stuck for 3h"), "{}", lane_status(&l, NOW));
}

/// An empty board says how to start rather than printing nothing.
#[test]
fn an_empty_board_says_how_to_start() {
    assert_eq!(overview(&pb::Plan::default(), NOW), NOTHING_PLANNED);
}

/// The Mac's and the orchestrator's argv parse, flags after the subcommand
/// included.
#[test]
fn the_argv_parses() {
    let cli = crate::Cli::try_parse_from([
        "farcooler", "plan", "lane", "start", "mac-ux", "--card", "ov-1:Mac", "--workspace", "main", "--actor",
        "manager", "--json",
    ])
    .expect("parses");
    assert!(cli.json);
    let crate::Command::Plan(args) = cli.command else { panic!() };
    assert_eq!(args.common.workspace.as_deref(), Some("main"));
    assert_eq!(args.common.actor.as_deref(), Some("manager"));
    assert!(matches!(args.cmd, Some(PlanCmd::Lane(LaneCmd::Start { .. }))));
    assert!(crate::Cli::try_parse_from(["farcooler", "plan"]).is_ok(), "bare `plan` is the overview");
    assert!(crate::Cli::try_parse_from(["farcooler", "lane", "list"]).is_err(), "`lane` lives under `plan`");
}

/// `farcooler theme` is still the terminal color schemes: the plan's themes
/// live under `plan`.
#[test]
fn farcooler_theme_is_still_the_color_schemes() {
    let cli = crate::Cli::try_parse_from(["farcooler", "theme", "list"]).expect("parses");
    assert!(matches!(cli.command, crate::Command::Theme(_)));
}

#[test]
fn a_card_names_its_slice_after_a_colon() {
    assert_eq!(split_card("ov-113:s3-4 phones"), ("ov-113", "s3-4 phones".to_string()));
    assert_eq!(split_card("ov-113"), ("ov-113", String::new()));
    assert_eq!(split_card(" ov-1 : Mac "), ("ov-1", "Mac".to_string()));
}

/// Ages say "just now" and "5m ago", never "just now ago".
#[test]
fn ages_read_naturally() {
    assert_eq!(ago(10_000), "just now");
    assert_eq!(ago(5 * 60_000), "5m ago");
    assert_eq!(ago(3 * HOUR), "3h ago");
    assert_eq!(age(49 * HOUR), "2d");
}

/// A refused move says where the lane is and where it can go, before anything
/// is sent, and review lands in one command.
#[tokio::test]
async fn a_refused_move_says_where_the_lane_can_go() {
    let mut link = runner();
    let err = say(&mut link, "lane set mac-ux --state building").await.unwrap_err();
    assert_eq!(err.to_string(), "mac-ux is in review. It can go to fixing, landing, landed or dropped.");
    let err = say(&mut link, "lane set fix-ac84 --state review").await.unwrap_err();
    assert_eq!(err.to_string(), "fix-ac84 is landed, so it takes no more changes.");
    assert!(link.sent.iter().all(|r| r.method != "lane.update"), "refused before the wire");
    say(&mut link, "lane set mac-ux --state landed --sha abc").await.expect("review lands in one command");
}

/// **`plan --json` is the shape the Mac decodes** (ov-273):
/// `test/fixtures/plan.json` is this output byte for byte, and AgentKit's
/// `PlanModelTests` decode the same file. A key renamed here fails this test
/// until the fixture is rewritten (`FARCOOLER_WRITE_FIXTURES=1`), which then
/// fails the Mac's.
#[test]
fn plan_json_is_the_shape_the_mac_reads() {
    let mut plan = the_plan();
    let review = &mut plan.lanes[1];
    review.reason = "Five small Mac fixes, batched".into();
    review.worktree_path = ".claude/worktrees/mac-ux".into();
    review.branch = "mac-ux".into();
    review.harness = "claude".into();
    review.model = "opus".into();
    review.fix_rounds = 1;
    review.cards[0].slice = "Mac".into();
    review.spend = Some(pb::LaneSpend {
        input_tokens: 300_000, output_tokens: 170_000, cost_micros: Some(31_000_000), runs: 2, unmeasured_agents: 1,
        shared_agents: 1, ..Default::default()
    });
    review.agents = vec![
        pb::LaneAgent { harness: "claude".into(), agent_id: "a1".into(), role: pb::LaneAgentRole::Build as i32,
            model: "opus".into(), started_at: NOW - 3 * HOUR, ended_at: Some(NOW - HOUR) },
        pb::LaneAgent { harness: "claude".into(), agent_id: "a2".into(), role: pb::LaneAgentRole::Review as i32,
            model: "sonnet".into(), started_at: NOW - 25 * 60_000, ended_at: None },
    ];
    plan.lanes[2].landed_sha = Some("4d3c8cb1e2".into());
    let theme = plan.themes[0].theme.as_mut().unwrap();
    theme.outcome = "Every Mac surface reads as one app.".into();
    theme.story = "Tokens are on main.".into();
    theme.story_at = NOW - HOUR;
    theme.next = "Frosted window plane".into();
    theme.owner_ask = "Should the sidebar tint follow the terminal theme?".into();
    let out = serde_json::to_string_pretty(&plan_json(&plan, &Keys::of_plan(&plan))).unwrap() + "\n";
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/plan.json");
    if std::env::var_os("FARCOOLER_WRITE_FIXTURES").is_some() {
        std::fs::write(&path, &out).unwrap();
    }
    let fixture = std::fs::read_to_string(&path).expect("test/fixtures/plan.json is missing");
    assert_eq!(out, fixture, "plan --json no longer matches test/fixtures/plan.json, which the Mac decodes");
}

/// One event of a record, its extra as the store writes it.
fn plan_event(at: i64, kind: &str, body: &str, extra: Value) -> pb::PlanEvent {
    pb::PlanEvent {
        id: id_bytes(Uuid::from_u128(0x4000 + at as u128 % 1000)),
        at,
        actor: "manager".into(),
        kind: kind.into(),
        body: body.into(),
        extra_json: extra.to_string(),
    }
}

/// **`plan theme show --json` and `plan lane show --json` are the records the
/// Mac decodes** (ov-273, review 1004i P4): `test/fixtures/plan-theme-show.json`
/// and `plan-lane-show.json` are this command's output, pretty-printed, and
/// AgentKit's `PlanRecordFixtureTests` decode the same files. Each event's
/// extra is the one `crates/store/src/plan.rs` writes for its kind. A key
/// renamed here fails this test until the fixtures are rewritten
/// (`FARCOOLER_WRITE_FIXTURES=1`), which then fails the Mac's.
#[tokio::test]
async fn plan_show_json_is_the_record_the_mac_reads() {
    let theme_events = vec![
        plan_event(NOW - 5 * HOUR, "state", "Created.", json!({})),
        plan_event(NOW - 4 * HOUR, "story", "", json!({ "to": "Tokens are in review." })),
        plan_event(NOW - 3 * HOUR, "cards", "Added ov-1.", json!({})),
        plan_event(NOW - 3 * HOUR, "cards", "Added ov-2.", json!({})),
        plan_event(NOW - HOUR, "story", "Tokens are in review.", json!({ "to": "Tokens are on main." })),
        plan_event(NOW - HOUR + 1, "state", "Paused.", json!({ "from": "active" })),
    ];
    let lane_events = vec![
        plan_event(NOW - 3 * HOUR, "state", "Queued.", json!({})),
        plan_event(NOW - 3 * HOUR + 1, "plan", "Ranked 1.", json!({ "rank": 1 })),
        plan_event(NOW - 2 * HOUR, "state", "Started building.", json!({ "from": "queued", "to": "building" })),
        plan_event(NOW - HOUR, "state", "Moved to review.", json!({ "from": "building", "to": "review" })),
    ];
    for (args, events, file) in [
        ("theme show Visual", theme_events, "plan-theme-show.json"),
        ("lane show mac-ux", lane_events, "plan-lane-show.json"),
    ] {
        let mut link = runner();
        link.events = events;
        let out = run_on(&mut link, &the_board(), parsed(args), "manager", true, NOW).await.unwrap();
        let value: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(value["events"].as_array().map(Vec::len), Some(link.events.len()), "{args}");
        let out = serde_json::to_string_pretty(&value).unwrap() + "\n";
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures").join(file);
        if std::env::var_os("FARCOOLER_WRITE_FIXTURES").is_some() {
            std::fs::write(&path, &out).unwrap();
        }
        let fixture = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("test/fixtures/{file} is missing"));
        assert_eq!(out, fixture, "plan {args} --json no longer matches test/fixtures/{file}, which the Mac decodes");
    }
}

/// A theme's progress leaves its cancelled cards out, as the Mac's does
/// (`PlanWords.total`, ov-273): a card nobody will do isn't one left to do.
#[test]
fn progress_leaves_cancelled_cards_out() {
    let mut view = theme(1, "Visual language", &[1, 2, 3, 4], 1);
    view.counts = Some(pb::PlanStatusCounts { done: 1, backlog: 1, cancelled: 2, ..Default::default() });
    assert_eq!(done_of(&view), "1 of 2 done");
    assert_eq!(theme_row(&view), "Visual language  1 of 2 done · active");
    view.counts = Some(pb::PlanStatusCounts { cancelled: 3, ..Default::default() });
    assert_eq!(done_of(&view), "0 of 0 done", "only cancelled cards: nothing to do, as the Mac says");
}

// ---- rulings (ov-304) ----

/// `plan ruling ...` with its words as given, a quoted decision included.
async fn rule(link: &mut Runner, args: &[&str], json: bool) -> Result<String, Failed> {
    let argv = ["farcooler", "plan", "ruling"].iter().chain(args).chain(&["--workspace", "main"]).map(|s| s.to_string());
    let cmd = match crate::Cli::try_parse_from(argv.collect::<Vec<_>>()).unwrap_or_else(|e| panic!("{args:?}: {e}")).command {
        crate::Command::Plan(a) => a.cmd,
        _ => panic!("not plan"),
    };
    run_on(link, &the_board(), cmd, "manager", json, NOW).await
}

/// A runner with the plan and without rulings is told before anything is
/// sent; one without the plan at all hears the plan's sentence first.
#[tokio::test]
async fn a_runner_without_rulings_is_told_before_anything_is_sent() {
    let mut link = runner();
    link.capabilities.retain(|c| c != capability::BOARD_RULINGS);
    for args in [&["list"][..], &["add", "x", "--why", "y", "--reversal", "z"], &["set", "R-2", "--state", "confirmed"]] {
        let err = rule(&mut link, args, false).await.unwrap_err();
        assert_eq!(err.to_string(), "This runner needs an update to keep rulings.", "{args:?}");
    }
    assert!(link.sent.is_empty(), "sent {:?}", link.sent.iter().map(|r| &r.method).collect::<Vec<_>>());
    link.capabilities.retain(|c| c != capability::BOARD_PLAN);
    assert_eq!(rule(&mut link, &["list"], false).await.unwrap_err().to_string(), NEEDS_UPDATE);
}

/// `ruling add` sends what was decided, why, what reversing costs, the cards
/// by id and the theme by id, naming the capability and the actor, in one
/// write after one read.
#[tokio::test]
async fn ruling_add_is_one_write_with_everything_in_it() {
    let mut link = runner();
    let said = rule(
        &mut link,
        &["add", "The gutter is 12 points.", "--why", "Tiled panes use it.", "--reversal", "One constant.", "--card", "ov-1",
          "--card", "ov-2", "--theme", "visual"],
        false,
    )
    .await
    .unwrap();
    assert_eq!(said, "Recorded R-3: The gutter is 12 points.");
    let r = last(&link);
    assert_eq!(r.method, "ruling.add");
    assert_eq!(r.required_capabilities, [capability::BOARD_RULINGS.to_string()]);
    let Some(request::Payload::RulingAdd(p)) = &r.payload else { panic!("{r:?}") };
    assert_eq!((p.decision.as_str(), p.why.as_str(), p.reversal.as_str()), ("The gutter is 12 points.", "Tiled panes use it.", "One constant."));
    assert_eq!(p.task_ids, vec![items()[0].id.clone(), items()[1].id.clone()]);
    assert_eq!(p.theme_id, Some(id_bytes(Uuid::from_u128(0x3001))));
    assert_eq!(p.actor, "manager");
    assert_eq!(link.sent.iter().filter(|r| r.method.starts_with("ruling.")).count(), 1);
}

/// `--why` and `--reversal` are required: a ruling always says both.
#[test]
fn a_ruling_without_why_or_reversal_does_not_parse() {
    for argv in ["farcooler plan ruling add x --reversal z", "farcooler plan ruling add x --why y"] {
        assert!(crate::Cli::try_parse_from(argv.split_whitespace()).is_err(), "{argv}");
    }
}

/// `ruling set` takes the short id as the owner says it, and sends the
/// ruling's id with the move and the note.
#[tokio::test]
async fn ruling_set_takes_the_short_id_as_said() {
    for id in ["R-2", "r-2", "2", "R2", "#2"] {
        let mut link = runner();
        let said = rule(&mut link, &["set", id, "--state", "reversed", "--note", "Owner: blue."], false).await.unwrap();
        assert_eq!(said, "R-2 is reversed.", "{id}");
        let Some(request::Payload::RulingSet(p)) = &last(&link).payload else { panic!() };
        assert_eq!(p.ruling_id, id_bytes(Uuid::from_u128(0x5002)));
        assert_eq!((p.state, p.note.as_deref()), (pb::BoardRulingState::Reversed as i32, Some("Owner: blue.")));
    }
    let mut link = runner();
    let err = rule(&mut link, &["set", "R-9", "--state", "confirmed"], false).await.unwrap_err();
    assert_eq!(err.to_string(), "There's no ruling R-9 on this board. `plan ruling list` shows them.");
    assert!(link.sent.iter().all(|r| r.method == "plan.get"), "nothing written");
}

/// A card a ruling names, in progress with no lane, isn't flagged: a ruling
/// isn't a lane, and the runner leaves its cards out of the plan's cards
/// (review 1005a F2). Its key still reads, from the ruling.
#[tokio::test]
async fn a_ruling_s_card_is_not_flagged_as_having_no_lane() {
    let mut plan = the_plan();
    plan.themes.clear();
    plan.lanes.clear();
    plan.order.clear();
    plan.coverage.clear();
    plan.cards.clear();
    plan.rulings[0].task_keys = vec!["ov-1".into()];
    assert!(checks(&plan, &Keys::of_plan(&plan)).is_empty());
    let json = plan_json(&plan, &Keys::of_plan(&plan));
    assert!(json["no_lane"].as_array().unwrap().is_empty());
    assert_eq!(json["rulings"][0]["cards"][0]["key"], "ov-1", "the key is the ruling's own");
}

/// A refused move reads as this module's sentence, not the runner's word.
#[tokio::test]
async fn a_refused_move_says_how_rulings_move() {
    let mut link = runner();
    link.refuse = Some("ruling_state");
    let err = rule(&mut link, &["set", "R-1", "--state", "confirmed"], false).await.unwrap_err();
    assert!(err.to_string().starts_with("A ruling can't make that move."), "{err}");
}

/// `ruling list` reads standing first with why and what reversing costs,
/// then the settled ones a line each; `--json` is the plan's `rulings`.
#[tokio::test]
async fn ruling_list_reads_standing_first() {
    let mut link = runner();
    let text = rule(&mut link, &["list"], false).await.unwrap();
    let expected = [
        "Decided for you",
        "  R-2    The inbox is amber.",
        "         Why: It's the one attention color, so the inbox reads as needing you.",
        "         Reversing: One token; every surface follows.",
        "         ov-1 · Visual language · by manager 1h ago",
        "Settled",
        "  R-1    Confirmed · Unread stays on the phones.",
        "         Note: Keep it.",
    ];
    assert_eq!(text.lines().collect::<Vec<_>>(), expected, "{text}");
    let Some(request::Payload::PlanGet(p)) = &link.sent[0].payload else { panic!() };
    assert!(p.include_closed, "every ruling, not the last week's");
    let json: Value = serde_json::from_str(&rule(&mut link, &["list"], true).await.unwrap()).unwrap();
    assert_eq!(json["rulings"][0]["short"], "R-2");
    assert_eq!(json["rulings"][0]["cards"][0]["key"], "ov-1");
    assert_eq!(json["rulings"][0]["theme"], "Visual language");
    assert_eq!(json["rulings"][1]["state"], "confirmed");
}

// ---- trains (ov-309) ----

/// `plan train start` names its lanes by name and sends them as ids, behind
/// `board_trains` as well as the plan.
#[tokio::test]
async fn a_train_starts_with_its_lanes_by_name() {
    let mut link = runner();
    let said = say(&mut link, "train start integ-10 --lane mac-ux --lane mac-fu3 --base origin/main").await.unwrap();
    let r = last(&link);
    assert_eq!(r.method, "train.start");
    assert_eq!(r.required_capabilities, [capability::BOARD_TRAINS.to_string()]);
    let Some(request::Payload::TrainStart(p)) = &r.payload else { panic!("{r:?}") };
    assert_eq!((p.name.as_str(), p.base.as_str(), p.actor.as_str()), ("integ-10", "origin/main", "manager"));
    assert_eq!(p.lane_ids, vec![tid(0x2002), tid(0x2001)]);
    assert!(said.starts_with("integ-10 · Integrating"), "{said}");
    assert!(said.contains("Lanes: mac-ux, mac-fu3"), "{said}");
    assert!(say(&mut link, "train start integ-11 --lane nope").await.unwrap_err().to_string().contains("No lane here is called"));
}

/// `plan train set` with a SHA sends it; a state goes as the wire's word.
#[tokio::test]
async fn a_train_is_set_by_name() {
    let mut link = runner();
    let said = say(&mut link, "train set INTEG-9 --sha 1a1b3275").await.unwrap();
    let Some(request::Payload::TrainSet(p)) = &last(&link).payload else { panic!() };
    assert_eq!((p.train_id.clone(), p.sha.as_deref(), p.state), (tid(0x6001), Some("1a1b3275"), 0));
    assert!(said.starts_with("integ-9 · Pushed · 1a1b3275 · CI not read yet"), "{said}");
    say(&mut link, "train set integ-9 --state landed --remove-lane mac-ux").await.unwrap();
    let Some(request::Payload::TrainSet(p)) = &last(&link).payload else { panic!() };
    assert_eq!((p.state, p.remove_lane_ids.clone()), (pb::BoardTrainState::Landed as i32, vec![tid(0x2002)]));
    let err = say(&mut link, "train set integ-99 --state gating").await.unwrap_err();
    assert!(err.to_string().contains("No train here is called"), "{err}");
}

/// A runner with the plan and no trains is told so before anything is sent.
#[tokio::test]
async fn a_runner_without_trains_is_told_before_anything_is_sent() {
    let mut link = runner();
    link.capabilities.retain(|c| c != capability::BOARD_TRAINS);
    let err = say(&mut link, "train list").await.unwrap_err();
    assert_eq!(err.to_string(), "This runner needs an update to keep trains.");
    assert!(link.sent.is_empty());
}

/// `plan train list` says what the owner's Now says, and which jobs failed.
#[tokio::test]
async fn the_train_list_names_failed_jobs() {
    let mut link = runner();
    let text = say(&mut link, "train list").await.unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "integ-9 · Red · c85bf83d · CI Failed · 1 of 3 jobs failed");
    assert_eq!(lines[1], "  cut from origin/main · red for 20m");
    assert_eq!(lines[2], "  Lanes: mac-ux");
    assert_eq!(lines[3], "  Failed: CI / Swift (shared + macOS)");
    assert!(lines[4].ends_with("runs/37275435256 · read 1m ago"), "{}", lines[4]);
}

/// The runner's refusals read as sentences, a train's own before a lane's: a
/// bad SHA isn't "too long", and a bad name names a train.
#[tokio::test]
async fn a_train_refusal_reads_as_a_train_sentence() {
    let mut link = runner();
    for (what, said) in [
        ("train_settled", "That train has landed or been dropped, so it takes no more moves or SHAs."),
        ("sha", "Give the SHA it pushed with --sha: 7 to 40 hex digits. A train is pushed, green or red only once it has one."),
        ("name", "A train's name is one word with no spaces, like integ-14."),
    ] {
        link.refuse = Some(what);
        let err = say(&mut link, "train set integ-9 --state gating").await.unwrap_err();
        assert_eq!(err.to_string(), said, "{what}");
    }
    link.refuse = Some("sha");
    let lane = say(&mut link, "lane set mac-ux --sha abc").await.unwrap_err();
    assert_eq!(lane.to_string(), "That's too long.", "a lane's words are the lane's");
}

/// A read that stopped working says how old it is (review train-1005c M1).
#[test]
fn a_stale_ci_read_says_how_old_it_is() {
    let mut plan = the_plan();
    plan.ci[0].fetched_at = NOW - 3 * HOUR;
    plan.ci[0].asked_at = NOW;
    let line = train::train_line(&plan, &plan.trains[0]);
    assert_eq!(line, "integ-9 · Red · c85bf83d · CI Failed · 1 of 3 jobs failed · as of 3h ago");
    plan.ci[0].fetched_at = NOW - 60_000;
    assert!(!train::train_line(&plan, &plan.trains[0]).contains("as of"));
}
