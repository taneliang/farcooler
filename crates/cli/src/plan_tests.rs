use clap::Parser;
use farcooler_transport::ClientError;
use serde_json::json;
use uuid::Uuid;

use super::*;
use crate::id_bytes;

const REPO: Uuid = Uuid::from_u128(0x0101);
const WORKSPACE: Uuid = Uuid::from_u128(0x0202);
const OTHER_BOARD: Uuid = Uuid::from_u128(0x0203);
const NOW: i64 = 1_800_000_000_000;
const HOUR: i64 = 3_600_000;

/// The orchestrator's pane, and the lander on Landing 9.
const ORCHESTRATOR: u128 = 0xa001;
const LANDER: u128 = 0xa002;
const BUILDER: u128 = 0xa003;

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

fn tid(n: u128) -> bytes::Bytes {
    id_bytes(Uuid::from_u128(n))
}

/// The ticket for card `n` on the line, in `stage`.
fn line(n: u128, stage: pb::TicketStage) -> pb::TicketLine {
    pb::TicketLine {
        task_id: tid(0x1000 + n),
        stage: stage as i32,
        stage_since: NOW - 10 * 60_000,
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
        budget_tokens: None,
        trend_tokens: vec![],
        last_moved_at: None,
    }
}

/// Two rulings (ov-304): R-2 standing on ov-1 and the theme, R-1 confirmed.
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

/// Landing 9 (ov-309, ov-872): red, carrying ov-1, its lander working, and
/// the runner's CI read of its SHA.
fn landings() -> (Vec<pb::Landing>, Vec<pb::BoardCiRead>) {
    let job = |name: &str, state: &str| pb::BoardCiJob { name: name.into(), state: state.into(), url: String::new() };
    let landing = pb::Landing {
        id: tid(0x6001),
        workspace_id: tid(0x0202),
        number: 9,
        route: pb::LandingRoute::Push as i32,
        base: "origin/main".into(),
        pushed_sha: Some("c85bf83d".into()),
        state: pb::LandingState::Red as i32,
        state_since: NOW - 20 * 60_000,
        lander_id: Some(tid(LANDER)),
        actor: "manager".into(),
        created_at: NOW - 2 * HOUR,
        task_ids: vec![tid(0x1001)],
        ci_subject: "sha:c85bf83d".into(),
        resource_version: 3,
        title: "Mac interface polish".into(),
        summary: "Title of ov-1".into(),
        phase: pb::LandingPhase::CiFailed as i32,
        phase_since: NOW - 20 * 60_000,
        spend: Some(pb::Spend { input_tokens: 90_000, output_tokens: 30_000, cost_micros: Some(4_000_000), runs: 1, ..Default::default() }),
        ..Default::default()
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
    (vec![landing], vec![read])
}

/// The orchestrator, and Landing 9's lander working ov-1.
fn agents() -> Vec<pb::Agent> {
    vec![
        pb::Agent {
            id: tid(LANDER),
            role: pb::AgentRole::Lander as i32,
            harness: "claude".into(),
            model: "sonnet".into(),
            title: "Land the Mac polish".into(),
            started_at: NOW - HOUR,
            tickets: vec![pb::AgentTicket { task_id: tid(0x1001), role: pb::WorkRole::Land as i32, started_at: NOW - HOUR }],
            working: true,
            doing_now: "Running the gate".into(),
            last_at: NOW,
            spend: Some(pb::Spend { input_tokens: 90_000, output_tokens: 30_000, cost_micros: Some(4_000_000), runs: 1, ..Default::default() }),
            ..Default::default()
        },
        pb::Agent {
            id: tid(ORCHESTRATOR),
            role: pb::AgentRole::Orchestrator as i32,
            harness: "claude".into(),
            model: "opus".into(),
            terminal_id: Some(tid(0xd00d)),
            started_at: NOW - 4 * HOUR,
            working: false,
            last_at: NOW - 5 * 60_000,
            spend: Some(pb::Spend { input_tokens: 2_000_000, output_tokens: 100_000, cost_micros: Some(14_200_000), runs: 4, ..Default::default() }),
            ..Default::default()
        },
    ]
}

/// What the runner holds: a plan, and the board's cards. ov-2 waits next up,
/// ov-1 is landing on Landing 9, built and now landed by agents, and ov-3
/// landed an hour ago.
fn the_plan() -> pb::Plan {
    let mut waiting = line(2, pb::TicketStage::Waiting);
    waiting.plan_rank = Some(1);
    waiting.reason = "Frees the Mac slot".into();
    let mut landing = line(1, pb::TicketStage::Landing);
    landing.branch = "mac-ux".into();
    landing.pr_number = Some(31);
    landing.landing = Some(9);
    landing.spend = Some(pb::Spend { input_tokens: 300_000, output_tokens: 170_000, runs: 2, ..Default::default() });
    landing.budget_tokens = Some(500_000);
    landing.work = vec![
        pb::TicketWork {
            agent_id: tid(BUILDER),
            task_id: tid(0x1001),
            role: pb::WorkRole::Build as i32,
            started_at: NOW - 3 * HOUR,
            ended_at: Some(NOW - 2 * HOUR),
            verdict: "finished".into(),
            ..Default::default()
        },
        pb::TicketWork {
            agent_id: tid(LANDER),
            task_id: tid(0x1001),
            role: pb::WorkRole::Land as i32,
            started_at: NOW - HOUR,
            ..Default::default()
        },
    ];
    let mut landed = line(3, pb::TicketStage::Landed);
    landed.stage_since = NOW - HOUR;
    landed.landed_sha = Some("4d3c8cb1e2".into());
    let (landings, ci) = landings();
    pb::Plan {
        now_ms: NOW,
        themes: vec![pb::BoardThemeView {
            spend: Some(theme_spend()),
            budget_tokens: Some(250_000),
            trend_tokens: TREND.to_vec(),
            last_moved_at: Some(NOW - HOUR),
            ..theme(1, "Visual language", &[1, 2, 3], 1)
        }],
        cards: items()
            .iter()
            .map(|t| pb::PlanCard { task_id: t.id.clone(), key: t.key.clone(), title: t.title.clone(), status: t.status })
            .collect(),
        rulings: rulings(),
        ci,
        board_counts: Some(board_counts()),
        cost: Some(plan_cost()),
        review_waits: vec![],
        lines: vec![waiting, landing, landed],
        landings,
        agents: agents(),
        agents_spend: Some(pb::Spend {
            input_tokens: 2_900_000, output_tokens: 100_000, cost_micros: Some(20_000_000), runs: 9, ..Default::default()
        }),
        suggested_reviewers: vec![],
    }
}

/// The theme's share of its tickets' spend (ov-306).
fn theme_spend() -> pb::Spend {
    pb::Spend { input_tokens: 235_000, output_tokens: 85_000, cost_micros: Some(15_500_000), runs: 2, ..Default::default() }
}

/// The week's tokens and the harness and model comparison (ov-307).
fn plan_cost() -> pb::PlanCost {
    pb::PlanCost {
        week_tokens: 34_200_000,
        compare: vec![
            pb::HarnessModelCost {
                harness: "claude".into(), model: "opus".into(), card_share_milli: 5_000, tokens: 7_500_000, cost_micros: Some(12_500_000),
            },
            pb::HarnessModelCost { harness: "codex".into(), model: "gpt-5.6".into(), card_share_milli: 3_400, tokens: 1_020_000, cost_micros: None },
        ],
        compare_held_back: 2,
        in_flight_tokens: 1_200_000,
        in_flight_cost_micros: None,
        week: vec![
            pb::WeekSpend { harness: "claude".into(), model: "opus".into(), tokens: 28_000_000, cost_micros: Some(41_200_000) },
            pb::WeekSpend { harness: "codex".into(), model: "gpt-5.6".into(), tokens: 6_200_000, cost_micros: Some(3_800_000) },
        ],
        week_cost_micros: Some(45_000_000),
        week_unsplit_cache_write_tokens: 0,
    }
}

/// The theme's seven days, oldest first: 320,000 tokens in all, which is past
/// its budget.
const TREND: [u64; 7] = [0, 0, 0, 40_000, 120_000, 0, 160_000];

fn board_counts() -> pb::PlanStatusCounts {
    pb::PlanStatusCounts { backlog: 4, in_progress: 2, in_review: 3, done: 11, ..Default::default() }
}

/// A runner that answers the reads from fixed state, echoes each write as the
/// layer would, and records what it was sent.
struct Runner {
    capabilities: Vec<String>,
    sent: Vec<pb::Request>,
    refuse: Option<&'static str>,
    /// What `plan.events` answers, for a theme, a ticket or a landing alike.
    events: Vec<pb::PlanEvent>,
    /// A plan to answer in place of `the_plan()`.
    plan: Option<pb::Plan>,
}

fn runner() -> Runner {
    Runner {
        capabilities: [
            "workstreams",
            "tasks",
            capability::BOARD_PLAN,
            capability::BOARD_LINE,
            capability::BOARD_RULINGS,
            capability::BOARD_COST,
            capability::BOARD_RULING_ACTIONS,
            capability::BOARD_THEME_MOVE,
            capability::AGENT_STOP,
        ]
        .map(String::from)
        .to_vec(),
        sent: vec![],
        refuse: None,
        events: vec![],
        plan: None,
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
        let plan = self.plan.clone().unwrap_or_else(the_plan);
        Ok(pb::Result {
            value: Some(match (method.as_str(), payload) {
                ("plan.get", _) => result::Value::Plan(plan),
                ("task.list", _) => result::Value::TaskList(pb::TaskList { items: items(), reads: None }),
                ("plan.events", _) => result::Value::PlanEventList(pb::PlanEventList { events: self.events.clone() }),
                ("line.order", Some(request::Payload::LineOrder(p))) => {
                    let mut plan = plan;
                    for l in &mut plan.lines {
                        l.plan_rank = p.task_ids.iter().position(|id| *id == l.task_id).map(|i| i as u32 + 1);
                    }
                    result::Value::Plan(plan)
                }
                ("line.set", Some(request::Payload::LineSet(p))) => {
                    let mut set = plan.lines.into_iter().find(|l| l.task_id == p.task_id).unwrap_or_else(|| pb::TicketLine {
                        task_id: p.task_id.clone(),
                        stage: pb::TicketStage::Waiting as i32,
                        ..Default::default()
                    });
                    if let Some(stage) = p.stage {
                        set.stage = stage;
                        set.landing = (stage == pb::TicketStage::Landing as i32).then_some(10);
                    }
                    result::Value::TicketLine(set)
                }
                ("board_theme.create" | "board_theme.update" | "board_theme.cards" | "board_theme.move", _) => {
                    result::Value::BoardThemeView(plan.themes[0].clone())
                }
                ("agent.record", Some(request::Payload::AgentRecord(p))) => result::Value::Agent(pb::Agent {
                    id: tid(0xa100),
                    role: p.role,
                    harness: p.harness,
                    model: p.model.unwrap_or_default(),
                    title: p.title.unwrap_or_default(),
                    subagent_id: p.subagent_id,
                    parent_terminal_id: p.parent_terminal_id,
                    tickets: p.task_ids.into_iter().map(|t| pb::AgentTicket { task_id: t, ..Default::default() }).collect(),
                    working: true,
                    ..Default::default()
                }),
                ("agent.work", Some(request::Payload::AgentWork(p))) => {
                    result::Value::Agent(plan.agents.into_iter().find(|a| a.id == p.agent_id).unwrap_or_default())
                }
                ("agent.end", Some(request::Payload::AgentEnd(p))) => {
                    result::Value::Agent(plan.agents.into_iter().find(|a| a.id == p.agent_id).unwrap_or_default())
                }
                ("agent.stop", Some(request::Payload::AgentStop(p))) => {
                    let agent = plan.agents.into_iter().find(|a| a.id == p.agent_id).unwrap_or_default();
                    result::Value::Agent(pb::Agent { stop_asked_at: Some(NOW), ..agent })
                }
                ("landing.start", Some(request::Payload::LandingStart(p))) => result::Value::Landing(pb::Landing {
                    id: tid(0x6002),
                    number: 10,
                    route: p.route,
                    base: p.base,
                    title: p.title,
                    lander_id: p.lander_id,
                    task_ids: p.task_ids,
                    state: pb::LandingState::Open as i32,
                    state_since: NOW,
                    ..Default::default()
                }),
                ("landing.set", Some(request::Payload::LandingSet(p))) => {
                    let mut set = plan.landings.into_iter().find(|l| l.id == p.landing_id).unwrap();
                    if let Some(sha) = p.sha {
                        set.ci_subject = format!("sha:{sha}");
                        set.pushed_sha = Some(sha);
                        set.state = pb::LandingState::Pushed as i32;
                    }
                    if p.state != 0 {
                        set.state = p.state;
                    }
                    set.task_ids.retain(|t| !p.remove_task_ids.contains(t));
                    set.task_ids.extend(p.add_task_ids);
                    result::Value::Landing(set)
                }
                ("terminal.list", _) => result::Value::TerminalList(pb::TerminalList {
                    items: vec![pb::Terminal { id: tid(0xd00d), ..Default::default() }],
                    ..Default::default()
                }),
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
                    set.reversed_sha = p.sha;
                    set.settled_by = Some(p.actor);
                    result::Value::BoardRuling(set)
                }
                ("ruling.keep_all", Some(request::Payload::RulingKeepAll(_))) => {
                    let mut open: Vec<_> = plan.rulings.into_iter().filter(|r| r.state == pb::BoardRulingState::Standing as i32).collect();
                    open.iter_mut().for_each(|r| r.state = pb::BoardRulingState::Confirmed as i32);
                    result::Value::RulingsKept(pb::RulingsKept { rulings: open })
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

async fn say_json(link: &mut Runner, args: &str) -> Value {
    let out = run_on(link, &the_board(), parsed(args), "manager", true, NOW).await.unwrap_or_else(|e| panic!("{args}: {e}"));
    serde_json::from_str(&out).unwrap()
}

fn last(link: &Runner) -> &pb::Request {
    link.sent.last().expect("something was sent")
}

/// A runner without the layer is told so before anything is sent, with the
/// sentence the design gives; one with themes and no line hears that it
/// needs the line, and still keeps its themes.
#[tokio::test]
async fn an_old_runner_is_told_before_anything_is_sent() {
    let mut link = runner();
    link.capabilities.retain(|c| c != capability::BOARD_PLAN);
    for args in ["", "set ov-2", "theme list", "ticket show ov-1", "ticket set ov-2"] {
        let err = say(&mut link, args).await.unwrap_err();
        assert_eq!(err.to_string(), "This runner needs an update to keep a plan.", "{args:?}");
    }
    assert!(link.sent.is_empty(), "sent {:?}", link.sent.iter().map(|r| &r.method).collect::<Vec<_>>());
    assert_eq!(needs_the_layer(&link.capabilities()).unwrap_err().to_string(), NEEDS_UPDATE);

    let mut link = runner();
    link.capabilities.retain(|c| c != capability::BOARD_LINE);
    for args in ["", "set ov-2", "ticket show ov-1", "ticket set ov-2 --stage building"] {
        let err = say(&mut link, args).await.unwrap_err();
        assert_eq!(err.to_string(), NEEDS_LINE, "{args:?}");
    }
    assert!(link.sent.is_empty(), "sent {:?}", link.sent.iter().map(|r| &r.method).collect::<Vec<_>>());
    say(&mut link, "theme list").await.expect("themes were kept before the line");
}

/// Every layer request names the capability it needs, so an older runner
/// refuses rather than dropping what it can't read, and who it's from.
#[tokio::test]
async fn every_write_names_the_capability_and_the_actor() {
    let cases = [
        ("set ov-2", capability::BOARD_LINE),
        ("theme create Reliability --outcome Done. --card ov-2", capability::BOARD_PLAN),
        ("theme set Visual --next Ship", capability::BOARD_PLAN),
        ("theme cards Visual --add ov-2", capability::BOARD_PLAN),
        ("ticket set ov-2 --stage building", capability::BOARD_LINE),
    ];
    for (args, needs) in cases {
        let mut link = runner();
        say(&mut link, args).await.unwrap_or_else(|e| panic!("{args}: {e}"));
        let r = last(&link);
        assert_eq!(r.required_capabilities, [needs.to_string()], "{args}");
        let actor = match r.payload.as_ref().unwrap() {
            request::Payload::LineOrder(p) => &p.actor,
            request::Payload::BoardThemeCreate(p) => &p.actor,
            request::Payload::BoardThemeUpdate(p) => &p.actor,
            request::Payload::BoardThemeCards(p) => &p.actor,
            request::Payload::LineSet(p) => &p.actor,
            other => panic!("{other:?}"),
        };
        assert_eq!(actor, "manager", "{args}: a write names who it's from, even `user`");
    }
}

/// The overview is what the Mac draws: next up first with each reason, then
/// the tickets being worked and who works them, the landings, the agents,
/// then the themes, rulings, cost and what landed today.
#[tokio::test]
async fn the_overview_reads_next_up_now_landings_agents_themes_and_landed() {
    let mut link = runner();
    let text = say(&mut link, "").await.unwrap();
    let expected = [
        "Next up",
        "  1  ov-2  Title of ov-2",
        "     Frees the Mac slot",
        "Now",
        "  ov-1  Title of ov-1 · Landing · on Landing 9 · PR 31 · Lander sonnet (0000a002) · 470K tokens · 470K of 500K tokens budgeted",
        "Landings",
        "  Mac interface polish (Landing 9) · Red · c85bf83d · CI Failed · 1 of 3 jobs failed · ov-1 · lander Lander sonnet (0000a002) · 120K tokens · $4.00 API-equivalent",
        "Agents",
        "  \u{25cf} Lander sonnet (0000a002) \u{201c}Land the Mac polish\u{201d}  ov-1  working · Running the gate  120K tokens · about $4.00 API-equivalent",
        "  \u{25cb} Orchestrator opus (0000a001)  The plan  idle · active 5m ago  2.1M tokens · about $14.20 API-equivalent",
        "Themes",
        "  Visual language  1 of 3 done · active · Over budget: 320K of 250K tokens",
        "Decided for you",
        "  R-2    The inbox is amber.",
        "Cost",
        "  Last 7 days  34M tokens · about $45.00 API-equivalent in this project (its weekly limit isn't known)",
        "    Claude Code opus · 28M tokens · about $41.20 API-equivalent",
        "    Codex gpt-5.6 · 6.2M tokens · about $3.80 API-equivalent",
        "  Claude Code opus · 5 finished cards · 1.5M tokens a card · about $2.50 a card API-equivalent",
        "  Codex gpt-5.6 · 3.4 finished cards · 300K tokens a card · No price listed for gpt-5.6",
        "  2 other harness and model pairs held back until three cards have landed",
        "  In flight  1.2M tokens on cards that haven't landed · API-equivalent dollars: Not reported",
        "Landed today",
        "  ov-3  Title of ov-3",
    ];
    assert_eq!(text.lines().collect::<Vec<_>>(), expected, "{text}");
    assert_eq!(link.sent.len(), 1, "one read draws it");
    assert_eq!(link.sent[0].method, "plan.get");
}

/// A card whose ticket landed while it isn't done, and one in progress that
/// isn't on the line, are flagged in words and in `--json`.
#[test]
fn landed_not_closed_and_off_the_line_are_derived() {
    let mut plan = the_plan();
    plan.cards[2].status = pb::TaskStatus::InProgress as i32;
    plan.lines.retain(|l| l.task_id != tid(0x1001));
    let flagged = checks(&plan);
    assert_eq!(flagged, ["ov-1  In Progress, and it isn't on the line.", "ov-3  Landed, and it's still In Progress."]);
    assert!(overview(&plan, NOW).ends_with("Worth a look\n  ov-1  In Progress, and it isn't on the line.\n  ov-3  Landed, and it's still In Progress."));
    let json = plan_json(&plan);
    assert_eq!(json["landed_not_closed"][0]["key"], "ov-3");
    assert_eq!(json["off_the_line"][0]["key"], "ov-1");
    plan.cards[2].status = pb::TaskStatus::Done as i32;
    assert!(plan_json(&plan)["landed_not_closed"].as_array().unwrap().is_empty());
}

/// `plan --json` is the client's `plan_json`, byte for byte: the shape the
/// apps decode and `test/fixtures/plan.json` holds (the client's test keeps
/// the fixture).
#[tokio::test]
async fn plan_json_is_the_apps_shape() {
    let mut link = runner();
    let out = run_on(&mut link, &the_board(), None, "manager", true, NOW).await.unwrap();
    assert_eq!(out, farcooler_client::plan_json::plan_json(&the_plan()).to_string());
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["lines"].as_array().unwrap().len(), 3);
    assert_eq!(v["lines"][1]["key"], "ov-1");
    assert_eq!(v["lines"][1]["stage"], "landing");
    assert_eq!(v["landings"][0]["number"], 9);
    assert_eq!(v["agents"][0]["short"], "0000a002");
    assert!(v.get("lanes").is_none() && v.get("trains").is_none(), "no lanes or trains in the shape");
}

/// `plan set` takes waiting tickets by key and sends their ids in order; a
/// ticket that isn't waiting, isn't on the line, or is named twice is refused
/// before anything is sent.
#[tokio::test]
async fn the_plan_takes_waiting_tickets_by_key() {
    let mut plan = the_plan();
    let mut also = line(4, pb::TicketStage::Waiting);
    also.task_id = tid(0x1004);
    plan.lines.push(also);
    plan.cards.push(pb::PlanCard { task_id: tid(0x1004), key: "ov-4".into(), title: "Four".into(), ..Default::default() });
    let mut link = Runner { plan: Some(plan), ..runner() };
    let out = say(&mut link, "set OV-4 ov-2").await.unwrap();
    assert!(out.starts_with("Next up\n  1  ov-4  Four\n  2  ov-2  Title of ov-2"), "{out}");
    let Some(request::Payload::LineOrder(p)) = &last(&link).payload else { panic!() };
    assert_eq!(p.task_ids, [tid(0x1004), tid(0x1002)]);
    assert_eq!(p.workspace_id, id_bytes(WORKSPACE));

    let mut link = runner();
    let err = say(&mut link, "set ov-1").await.unwrap_err();
    assert_eq!(err.to_string(), "Only a waiting ticket can be in the plan: ov-1 is landing.");
    let err = say(&mut link, "set ov-2 ov-2").await.unwrap_err();
    assert_eq!(err.to_string(), "ov-2 is named twice. A ticket can only be in the plan once.");
    let err = say(&mut link, "set ov-9").await.unwrap_err();
    assert_eq!(err.to_string(), "ov-9 isn't on the line. Put it there with `farcooler plan ticket set ov-9`.");
    assert!(link.sent.iter().all(|r| r.method != "line.order"), "refused before the wire");
    say(&mut link, "set").await.expect("no tickets empties the plan");
    let Some(request::Payload::LineOrder(p)) = &last(&link).payload else { panic!() };
    assert!(p.task_ids.is_empty());
}

/// `ticket set` puts a card on the line, waiting, and says so; every flag
/// reaches the wire, a stack parent by its id and `--no-stack` as empty.
#[tokio::test]
async fn ticket_set_puts_a_card_on_the_line_and_changes_it() {
    let mut plan = the_plan();
    plan.lines.retain(|l| l.task_id != tid(0x1002));
    let mut link = Runner { plan: Some(plan), ..runner() };
    let out = say(&mut link, "ticket set ov-2 --reason Next --branch el/two --stack-on ov-1 --pr 40 --budget 1M").await.unwrap();
    assert_eq!(out, "ov-2 is on the line, waiting.");
    let Some(request::Payload::LineSet(p)) = &last(&link).payload else { panic!() };
    assert_eq!(p.task_id, tid(0x1002));
    assert_eq!(p.stage, None, "a card goes on the line waiting");
    assert_eq!((p.reason.as_deref(), p.branch.as_deref(), p.pr_number), (Some("Next"), Some("el/two"), Some(40)));
    assert_eq!((p.stack_parent.clone(), p.budget_tokens), (Some(tid(0x1001)), Some(1_000_000)));

    let mut link = runner();
    assert_eq!(say(&mut link, "ticket set ov-2 --stage building --no-stack").await.unwrap(), "ov-2 is building.");
    let Some(request::Payload::LineSet(p)) = &last(&link).payload else { panic!() };
    assert_eq!((p.stage, p.stack_parent.clone()), (Some(pb::TicketStage::Building as i32), Some(bytes::Bytes::new())));
    assert_eq!(say(&mut link, "ticket set ov-1 --stage landed --sha abc1234").await.unwrap(), "ov-1 is landed.");
    let Some(request::Payload::LineSet(p)) = &last(&link).payload else { panic!() };
    assert_eq!(p.landed_sha.as_deref(), Some("abc1234"));
    assert_eq!(say(&mut link, "ticket set ov-2 --stage needs_you").await.unwrap(), "ov-2 is waiting on you.");

    let err = say(&mut link, "ticket set ov-2").await.unwrap_err();
    assert!(err.to_string().starts_with("ov-2 is already on the line. Say what to change"), "{err}");
    let err = say(&mut link, "ticket set ov-40 --stage building").await.unwrap_err();
    assert_eq!(err.to_string(), "ov-40 is on another board.");
}

/// A ticket that reaches landing says which landing it joined.
#[tokio::test]
async fn a_ticket_that_reaches_landing_names_its_landing() {
    let mut plan = the_plan();
    plan.lines[0].stage = pb::TicketStage::Reviewing as i32;
    plan.lines[0].plan_rank = None;
    let mut link = Runner { plan: Some(plan), ..runner() };
    assert_eq!(say(&mut link, "ticket set ov-2 --stage landing").await.unwrap(), "ov-2 is landing. It's on Landing 10.");
}

/// A refused move says where the ticket is and where it can go, before
/// anything is sent, in the line's own arrows.
#[tokio::test]
async fn a_refused_move_says_where_the_ticket_can_go() {
    let mut link = runner();
    let err = say(&mut link, "ticket set ov-1 --stage waiting").await.unwrap_err();
    assert_eq!(err.to_string(), "ov-1 is landing. It can move to building, needs_you, landed or dropped.");
    let err = say(&mut link, "ticket set ov-2 --stage landing").await.unwrap_err();
    assert_eq!(err.to_string(), "ov-2 is waiting. It can move to building, needs_you or dropped.");
    let err = say(&mut link, "ticket set ov-3 --stage building").await.unwrap_err();
    assert_eq!(err.to_string(), "ov-3 is landed, so it takes no more moves.");
    assert!(link.sent.iter().all(|r| r.method != "line.set"), "refused before the wire");
}

/// `ticket show` reads the ticket's own timeline and says who worked it.
#[tokio::test]
async fn ticket_show_reads_the_timeline_and_who_worked_it() {
    let mut link = runner();
    link.events = vec![plan_event(NOW - HOUR, "stage", "Landing.", json!({ "from": "reviewing", "to": "landing" }))];
    let text = say(&mut link, "ticket show ov-1").await.unwrap();
    let expected = [
        "ov-1  Title of ov-1 · Landing · on Landing 9 · PR 31 · Lander sonnet (0000a002) · 470K tokens · 470K of 500K tokens budgeted",
        "Branch mac-ux",
        "",
        "Worked by",
        "  agent 0000a003 · building, 1h · finished",
        "  Lander sonnet (0000a002) · landing, 1h · working",
        "",
        "Spend  470K tokens",
        "Budget  470K of 500K tokens budgeted",
        "",
        "Timeline",
        "  1h ago  Landing.",
    ];
    assert_eq!(text.lines().collect::<Vec<_>>(), expected, "{text}");
    let Some(request::Payload::PlanEvents(p)) = &link.sent.iter().find(|r| r.method == "plan.events").unwrap().payload else { panic!() };
    assert_eq!(p.subject, Some(pb::plan_events_request::Subject::TaskId(tid(0x1001))));

    let json = say_json(&mut link, "ticket show ov-1").await;
    assert_eq!(json, {
        let mut want = farcooler_client::plan_json::plan_json(&the_plan())["lines"][1].clone();
        want["events"] = json!([{ "at": NOW - HOUR, "actor": "manager", "kind": "stage", "body": "Landing.", "extra": { "from": "reviewing", "to": "landing" } }]);
        want
    });
    let err = say(&mut link, "ticket show ov-9").await.unwrap_err();
    assert_eq!(err.to_string(), "ov-9 isn't on the line. Put it there with `farcooler plan ticket set ov-9`.");
}

/// A ticket's pull request stage (ov-312) reaches the overview and `ticket
/// show`; a building one isn't said twice.
#[test]
fn a_ticket_s_pr_stage_is_printed() {
    let mut plan = the_plan();
    plan.lines[1].pr_stage = Some(pb::PrStage {
        kind: pb::PrStageKind::WaitingOnReviewer as i32,
        label: "Waiting on alice".into(),
        pr_number: 31,
        unresolved_threads: Some(2),
        ..Default::default()
    });
    assert!(overview(&plan, NOW).contains("Landing · on Landing 9 · PR 31 · Waiting on alice · Lander"), "{}", overview(&plan, NOW));
    let shown = ticket::ticket_text(&plan, &plan.lines[1], &[], NOW);
    assert!(shown.contains("Pull request  Waiting on alice · PR 31 · 2 threads open"), "{shown}");
    assert_eq!(plan_json(&plan)["lines"][1]["pr_stage"]["kind"], "waiting_on_reviewer");
    plan.lines[1].stage = pb::TicketStage::Building as i32;
    plan.lines[1].pr_stage = Some(pb::PrStage { kind: pb::PrStageKind::Building as i32, label: "Building".into(), ..Default::default() });
    assert!(!overview(&plan, NOW).contains("Building · Building"));
}

/// A card on another board, or nowhere, is named as such before the write.
#[tokio::test]
async fn a_card_must_be_on_this_board() {
    let mut link = runner();
    let err = say(&mut link, "theme cards Visual --add ov-40").await.unwrap_err();
    assert_eq!(err.to_string(), "ov-40 is on another board.");
    assert!(link.sent.iter().all(|r| r.method != "board_theme.cards"));
}

/// A refusal from the runner is said in this module's words, and still carries
/// the runner's word for a script. Every word the line refuses with has one.
#[tokio::test]
async fn a_refusal_is_said_in_words_a_person_reads() {
    let mut link = runner();
    link.refuse = Some("parent_not_landed");
    let err = say(&mut link, "ticket set ov-1 --stage landed").await.unwrap_err();
    assert_eq!(err.to_string(), "Its parent ticket hasn't landed yet, so it lands after that one does.");
    let refusal = err.downcast_ref::<Refused>().expect("keeps its code");
    assert_eq!(refusal.what(), Some("parent_not_landed"));
    for word in [
        "stage", "ticket_closed", "on_the_line", "not_on_the_line", "not_waiting", "duplicate", "stack_parent", "stack_cycle",
        "parent_not_landed", "same_branch", "not_landed", "role", "agent_ended", "place", "verdict", "ticket_pr", "name",
        "name_taken", "other_board", "harness", "agent_id", "state", "actor", "outcome", "story", "subject", "title", "sha",
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

/// `theme show` reads the timeline, and lists the theme's tickets on the line.
#[tokio::test]
async fn theme_show_reads_the_timeline_and_its_tickets() {
    let mut link = runner();
    let text = say(&mut link, "theme show Visual").await.unwrap();
    assert!(text.starts_with("Visual language · active"), "{text}");
    assert!(link.sent.iter().any(|r| r.method == "plan.events"));
    assert!(text.contains("On the line\n  ov-2  Title of ov-2 · Waiting · next up\n  ov-1  Title of ov-1 · Landing"), "{text}");
}

/// Spend says "Not reported" rather than zero for a ticket nothing measured,
/// tokens first, and a share of an agent's spend says so.
#[test]
fn spend_puts_tokens_first_and_says_what_isn_t_reported() {
    assert_eq!(spend_words(&pb::Spend::default()), "Not reported");
    let some = pb::Spend { input_tokens: 1_500, runs: 1, unmeasured_agents: 1, ..Default::default() };
    assert_eq!(spend_words(&some), "1.5K tokens · 1 agent not reported");
    let s = pb::Spend { input_tokens: 3_000, output_tokens: 1_000, cost_micros: Some(40_000), runs: 1, ..Default::default() };
    assert_eq!(spend_words(&s), "4K tokens · $0.04 API-equivalent");
    let one = pb::Spend { input_tokens: 2_000, runs: 1, shared_agents: 1, ..Default::default() };
    assert_eq!(spend_words(&one), "2K tokens · 1 agent\u{2019}s spend split with other tickets");
    let two = pb::Spend { input_tokens: 2_000, runs: 1, unmeasured_agents: 1, shared_agents: 2, ..Default::default() };
    assert_eq!(spend_words(&two), "2K tokens · 1 agent not reported · 2 agents\u{2019} spend split with other tickets");
}

/// A ticket stuck for over an hour says how long, and a fix round its number.
#[test]
fn a_stale_ticket_says_how_long_and_a_fix_its_round() {
    let plan = the_plan();
    let mut l = line(1, pb::TicketStage::Building);
    l.stage_since = NOW - 3 * HOUR;
    l.stale = true;
    l.fix_rounds = 2;
    let said = overview_text::ticket_status(&plan, &l, NOW);
    assert!(said.starts_with("Fixing · round 2") && said.ends_with("stuck for 3h"), "{said}");
}

/// An empty board says how to start rather than printing nothing.
#[test]
fn an_empty_board_says_how_to_start() {
    assert_eq!(overview(&pb::Plan::default(), NOW), NOTHING_PLANNED);
}

/// The argv the orchestrator writes parses, flags after the subcommand
/// included, and the retired verbs don't.
#[test]
fn the_argv_parses() {
    let cli = crate::Cli::try_parse_from([
        "farcooler", "plan", "ticket", "set", "ov-1", "--stage", "needs_you", "--workspace", "main", "--actor", "manager", "--json",
    ])
    .expect("parses");
    assert!(cli.json);
    let crate::Command::Plan(args) = cli.command else { panic!() };
    assert_eq!(args.common.workspace.as_deref(), Some("main"));
    assert_eq!(args.common.actor.as_deref(), Some("manager"));
    assert!(matches!(args.cmd, Some(PlanCmd::Ticket(ticket::TicketCmd::Set { stage: Some(ticket::StageArg::NeedsYou), .. }))));
    assert!(crate::Cli::try_parse_from(["farcooler", "plan"]).is_ok(), "bare `plan` is the overview");
    for retired in [&["plan", "lane", "list"][..], &["plan", "train", "list"], &["plan", "agents"], &["lane", "list"]] {
        let argv = std::iter::once("farcooler").chain(retired.iter().copied());
        assert!(crate::Cli::try_parse_from(argv).is_err(), "{retired:?} is gone");
    }
    for stage in ["waiting", "building", "reviewing", "needs_you", "needs-you", "landing", "landed", "dropped"] {
        assert!(crate::Cli::try_parse_from(["farcooler", "plan", "ticket", "set", "ov-1", "--stage", stage]).is_ok(), "{stage}");
    }
    assert!(crate::Cli::try_parse_from(["farcooler", "plan", "ticket", "set", "ov-1", "--stack-on", "ov-2", "--no-stack"]).is_err());
}

/// `farcooler theme` is still the terminal color schemes: the plan's themes
/// live under `plan`.
#[test]
fn farcooler_theme_is_still_the_color_schemes() {
    let cli = crate::Cli::try_parse_from(["farcooler", "theme", "list"]).expect("parses");
    assert!(matches!(cli.command, crate::Command::Theme(_)));
}

/// Ages say "just now" and "5m ago", never "just now ago".
#[test]
fn ages_read_naturally() {
    assert_eq!(ago(10_000), "just now");
    assert_eq!(ago(5 * 60_000), "5m ago");
    assert_eq!(ago(3 * HOUR), "3h ago");
    assert_eq!(age(49 * HOUR), "2d");
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

/// **`plan theme show --json` is the record the Mac decodes** (ov-273):
/// `test/fixtures/plan-theme-show.json` is this command's output,
/// pretty-printed, and AgentKit's `PlanRecordFixtureTests` decode the same
/// file. A key renamed here fails this test until the fixture is rewritten
/// (`FARCOOLER_WRITE_FIXTURES=1`), which then fails the Mac's.
#[tokio::test]
async fn theme_show_json_is_the_record_the_mac_reads() {
    let mut link = runner();
    link.events = vec![
        plan_event(NOW - 5 * HOUR, "state", "Created.", json!({})),
        plan_event(NOW - 4 * HOUR, "story", "", json!({ "to": "Tokens are in review." })),
        plan_event(NOW - 3 * HOUR, "cards", "Added ov-1.", json!({})),
        plan_event(NOW - 3 * HOUR, "cards", "Added ov-2.", json!({})),
        plan_event(NOW - HOUR, "story", "Tokens are in review.", json!({ "to": "Tokens are on main." })),
        plan_event(NOW - HOUR + 1, "state", "Paused.", json!({ "from": "active" })),
    ];
    let value = say_json(&mut link, "theme show Visual").await;
    assert_eq!(value["events"].as_array().map(Vec::len), Some(link.events.len()));
    let out = serde_json::to_string_pretty(&value).unwrap() + "\n";
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/plan-theme-show.json");
    if std::env::var_os("FARCOOLER_WRITE_FIXTURES").is_some() {
        std::fs::write(&path, &out).unwrap();
    }
    let fixture = std::fs::read_to_string(&path).expect("test/fixtures/plan-theme-show.json is missing");
    assert_eq!(out, fixture, "plan theme show --json no longer matches test/fixtures/plan-theme-show.json, which the Mac decodes");
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

/// `plan ruling ...` as the owner (the Mac's `--actor user`).
async fn rule_as_owner(link: &mut Runner, args: &[&str], json: bool) -> Result<String, Failed> {
    let argv = ["farcooler", "plan", "ruling"].iter().chain(args).chain(&["--workspace", "main"]).map(|s| s.to_string());
    let cmd = match crate::Cli::try_parse_from(argv.collect::<Vec<_>>()).unwrap_or_else(|e| panic!("{args:?}: {e}")).command {
        crate::Command::Plan(a) => a.cmd,
        _ => panic!("not plan"),
    };
    run_on(link, &the_board(), cmd, "user", json, NOW).await
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
    let err = rule(&mut link, &["set", "R-9", "--state", "reversed"], false).await.unwrap_err();
    assert_eq!(err.to_string(), "There's no ruling R-9 on this board. `plan ruling list` shows them.");
    assert!(link.sent.iter().all(|r| r.method == "plan.get"), "nothing written");
}

/// A card a ruling names, in progress and off the line, isn't flagged: a
/// ruling isn't a ticket, and the runner leaves its cards out of the plan's
/// cards (review 1005a F2). Its key still reads, from the ruling.
#[tokio::test]
async fn a_ruling_s_card_is_not_flagged_as_off_the_line() {
    let mut plan = the_plan();
    plan.themes.clear();
    plan.lines.clear();
    plan.landings.clear();
    plan.cards.clear();
    plan.rulings[0].task_keys = vec!["ov-1".into()];
    assert!(checks(&plan).is_empty());
    let json = plan_json(&plan);
    assert!(json["off_the_line"].as_array().unwrap().is_empty());
    assert_eq!(json["rulings"][0]["cards"][0]["key"], "ov-1", "the key is the ruling's own");
}

/// Keep is one `ruling.set` to confirmed, as the owner, with no sha; a ruling
/// already kept is said so and nothing is sent; a typo names no ruling.
#[tokio::test]
async fn keep_marks_one_ruling_as_the_owner() {
    let mut link = runner();
    assert_eq!(rule_as_owner(&mut link, &["keep", "R-2"], false).await.unwrap(), "Kept R-2.");
    let Some(request::Payload::RulingSet(p)) = &last(&link).payload else { panic!("{:?}", last(&link)) };
    assert_eq!(p.ruling_id, id_bytes(Uuid::from_u128(0x5002)));
    assert_eq!((p.state, p.actor.as_str(), p.sha.as_deref()), (pb::BoardRulingState::Confirmed as i32, "user", None));
    assert_eq!(last(&link).required_capabilities, [capability::BOARD_RULINGS.to_string()]);

    let mut link = runner();
    assert_eq!(rule_as_owner(&mut link, &["keep", "R-1"], false).await.unwrap(), "R-1 is already kept.");
    assert!(link.sent.iter().all(|r| r.method == "plan.get"), "nothing written for a kept ruling");
    let err = rule_as_owner(&mut link, &["keep", "R-9"], false).await.unwrap_err();
    assert_eq!(err.to_string(), "There's no ruling R-9 on this board. `plan ruling list` shows them.");
}

/// Keep All is one `ruling.keep_all` for the board, naming the owner, and says
/// which it kept; `keep` needs an id or `--all`, not both.
#[tokio::test]
async fn keep_all_is_one_request_for_the_board() {
    let mut link = runner();
    assert_eq!(rule_as_owner(&mut link, &["keep", "--all"], false).await.unwrap(), "Kept R-2.");
    let r = last(&link);
    assert_eq!(r.method, "ruling.keep_all");
    assert_eq!(r.required_capabilities, [capability::BOARD_RULING_ACTIONS.to_string()]);
    let Some(request::Payload::RulingKeepAll(p)) = &r.payload else { panic!() };
    assert_eq!(p.actor, "user");
    assert_eq!(link.sent.iter().filter(|r| r.method.starts_with("ruling.")).count(), 1);
    for argv in ["farcooler plan ruling keep", "farcooler plan ruling keep R-2 --all"] {
        assert!(crate::Cli::try_parse_from(argv.split_whitespace()).is_err(), "{argv}");
    }
}

/// `set --state confirmed` is Keep by another name: refused for anyone but a
/// person, with Keep's sentence, and nothing is sent; a person's own works.
#[tokio::test]
async fn set_confirmed_is_the_owner_s_alone() {
    let mut link = runner();
    let err = rule(&mut link, &["set", "R-2", "--state", "confirmed"], false).await.unwrap_err();
    assert!(err.to_string().starts_with("Keeping a ruling is a person's call."), "{err}");
    assert!(link.sent.is_empty(), "{:?}", link.sent.iter().map(|r| &r.method).collect::<Vec<_>>());
    assert_eq!(rule_as_owner(&mut link, &["set", "R-2", "--state", "confirmed"], false).await.unwrap(), "R-2 is confirmed.");
}

/// The orchestrator never keeps a ruling for a person, and nothing is sent.
#[tokio::test]
async fn the_orchestrator_cannot_keep() {
    for args in [&["keep", "R-2"][..], &["keep", "--all"]] {
        let mut link = runner();
        let err = rule(&mut link, args, false).await.unwrap_err();
        assert!(err.to_string().starts_with("Keeping a ruling is a person's call."), "{args:?}: {err}");
        assert!(link.sent.is_empty(), "{args:?}: {:?}", link.sent.iter().map(|r| &r.method).collect::<Vec<_>>());
    }
}

/// Reverse is the orchestrator's mark: it sends the commit and says it.
#[tokio::test]
async fn reverse_marks_a_ruling_with_its_commit() {
    let mut link = runner();
    let said = rule(&mut link, &["reverse", "R-2", "--sha", "6e7e5618", "--note", "Blue again."], false).await.unwrap();
    assert_eq!(said, "R-2 is reversed in 6e7e5618.");
    let Some(request::Payload::RulingSet(p)) = &last(&link).payload else { panic!() };
    assert_eq!((p.state, p.sha.as_deref(), p.note.as_deref()), (pb::BoardRulingState::Reversed as i32, Some("6e7e5618"), Some("Blue again.")));
    // A reversal with no commit (a setting, a process call) is marked without one.
    let mut link = runner();
    assert_eq!(rule(&mut link, &["reverse", "R-2"], false).await.unwrap(), "R-2 is reversed.");
    let Some(request::Payload::RulingSet(p)) = &last(&link).payload else { panic!() };
    assert_eq!((p.state, p.sha.as_deref()), (pb::BoardRulingState::Reversed as i32, None));
    let mut link = runner();
    link.refuse = Some("reversed_sha");
    let err = rule(&mut link, &["reverse", "R-2", "--sha", "zz"], false).await.unwrap_err();
    assert!(err.to_string().starts_with("Give the commit that reversed it"), "{err}");
}

/// A runner with `board_rulings` and none of the owner's actions is told
/// before anything is sent; the older verbs still work.
#[tokio::test]
async fn a_runner_without_the_owner_s_actions_is_told() {
    let mut link = runner();
    link.capabilities.retain(|c| c != capability::BOARD_RULING_ACTIONS);
    for args in [&["keep", "R-2"][..], &["keep", "--all"], &["reverse", "R-2", "--sha", "abcd"]] {
        let err = rule_as_owner(&mut link, args, false).await.unwrap_err();
        assert_eq!(err.to_string(), "This runner needs an update to keep or reverse rulings.", "{args:?}");
    }
    assert!(link.sent.is_empty());
    assert!(rule(&mut link, &["set", "R-2", "--state", "reversed"], false).await.is_ok(), "set still works");
}

/// `list --state` filters in the owner's words; JSON carries the filter and
/// `reversed_sha`.
#[tokio::test]
async fn list_filters_by_state() {
    let mut link = runner();
    let open = rule(&mut link, &["list", "--state", "open"], false).await.unwrap();
    assert!(open.contains("R-2") && !open.contains("R-1") && !open.contains("Past decisions"), "{open}");
    let kept = rule(&mut link, &["list", "--state", "kept"], false).await.unwrap();
    assert!(kept.contains("R-1    Kept") && !kept.contains("R-2"), "{kept}");
    assert_eq!(rule(&mut link, &["list", "--state", "reversed"], false).await.unwrap(), "No reversed rulings.");
    let json: Value = serde_json::from_str(&rule(&mut link, &["list", "--state", "kept"], true).await.unwrap()).unwrap();
    assert_eq!(json["rulings"].as_array().unwrap().len(), 1);
    assert_eq!(json["rulings"][0]["short"], "R-1");
    assert!(json["rulings"][0].get("reversed_sha").is_some());
}

/// A refused move reads as this module's sentence, not the runner's word.
#[tokio::test]
async fn a_refused_move_says_how_rulings_move() {
    let mut link = runner();
    link.refuse = Some("ruling_state");
    let err = rule_as_owner(&mut link, &["set", "R-1", "--state", "confirmed"], false).await.unwrap_err();
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
        "Past decisions",
        "  R-1    Kept · Unread stays on the phones.",
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

// ---- cost (ov-307) ----

/// `--budget` takes tokens as a number or with K, M or B, and refuses what
/// isn't a whole number of tokens.
#[test]
fn a_budget_reads_as_tokens() {
    use super::cost::parse_budget;
    assert_eq!(parse_budget("5000000"), Ok(5_000_000));
    assert_eq!(parse_budget("800k"), Ok(800_000));
    assert_eq!(parse_budget("5M"), Ok(5_000_000));
    assert_eq!(parse_budget("1.5m"), Ok(1_500_000));
    assert_eq!(parse_budget("1,200,000"), Ok(1_200_000));
    // Every decimal a person writes is exact: these were refused when it went
    // through a float (4.1 times a million is 4099999.9999999995).
    assert_eq!(parse_budget("4.1M"), Ok(4_100_000));
    assert_eq!(parse_budget("400k"), Ok(400_000));
    assert_eq!(parse_budget("8.2m"), Ok(8_200_000));
    assert_eq!(parse_budget("16.1k"), Ok(16_100));
    assert_eq!(parse_budget("32.2M"), Ok(32_200_000));
    assert_eq!(parse_budget("0.5M"), Ok(500_000));
    assert_eq!(parse_budget(".5M"), Ok(500_000));
    assert_eq!(parse_budget("2.50M"), Ok(2_500_000));
    assert_eq!(parse_budget("1.000001B"), Ok(1_000_001_000));
    assert_eq!(parse_budget("007"), Ok(7));
    // Whole over the whole range of 1.1 to 999.9 in every unit.
    for tenths in 11..=9999u64 {
        for (suffix, scale) in [("k", 1_000u64), ("M", 1_000_000), ("B", 1_000_000_000)] {
            let text = format!("{}.{}{suffix}", tenths / 10, tenths % 10);
            assert_eq!(parse_budget(&text), Ok(tenths * scale / 10), "{text}");
        }
    }
    for bad in ["", "0", "-5", "lots", "1.0000001k", "5T", "1.5", "1.5.5M", "M", "0M", "99999999999999999999"] {
        assert!(parse_budget(bad).is_err(), "{bad:?} is not a budget");
    }
}

/// A budget goes to the theme and the ticket as tokens, `--no-budget` as
/// zero, and a runner without `board_cost` is told before anything is changed.
#[tokio::test]
async fn a_budget_is_sent_and_a_runner_without_budgets_is_told() {
    let mut link = runner();
    say(&mut link, "theme set Visual --budget 5M").await.unwrap();
    let Some(request::Payload::BoardThemeUpdate(p)) = &last(&link).payload else { panic!() };
    assert_eq!(p.budget_tokens, Some(5_000_000));
    say(&mut link, "ticket set ov-1 --no-budget").await.unwrap();
    let Some(request::Payload::LineSet(p)) = &last(&link).payload else { panic!() };
    assert_eq!(p.budget_tokens, Some(0));
    say(&mut link, "theme set Visual --next Ship").await.unwrap();
    let Some(request::Payload::BoardThemeUpdate(p)) = &last(&link).payload else { panic!() };
    assert_eq!(p.budget_tokens, None, "a set that says nothing of budgets leaves it");

    let mut old = runner();
    old.capabilities.retain(|c| c != capability::BOARD_COST);
    let err = say(&mut old, "ticket set ov-1 --budget 1M").await.unwrap_err();
    assert_eq!(err.to_string(), "This runner needs an update to keep budgets.");
    assert!(old.sent.iter().all(|r| r.method != "line.set"), "nothing was changed");
}

/// The overview flags a theme over its budget, shows a ticket inside its
/// own, and says the week's tokens with no limit, and what is held back.
#[tokio::test]
async fn the_plan_flags_what_is_over_budget_and_never_invents_a_limit() {
    let mut link = runner();
    let text = say(&mut link, "").await.unwrap();
    assert!(text.contains("Visual language  1 of 3 done · active · Over budget: 320K of 250K tokens"), "{text}");
    assert!(text.contains("470K of 500K tokens budgeted"), "{text}");
    assert!(!text.contains('%'), "no share of a limit nobody can read: {text}");
    let theme = say(&mut link, "theme show Visual").await.unwrap();
    assert!(theme.contains("Last 7 days  0, 0, 0, 40K, 120K, 0, 160K tokens a day (UTC days), oldest first, today last"), "{theme}");
}

/// A week with older cache writes says its costs may read low, under Cost;
/// a week without them says nothing (ov-465).
#[test]
fn the_overview_notes_old_cache_writes_only_when_the_week_has_them() {
    let note = "Costs before Oct 10 may read low: older cache writes weren't split.";
    let mut plan = the_plan();
    assert!(!super::overview(&plan, NOW).contains("may read low"), "no unsplit rows, no note");
    plan.cost.as_mut().unwrap().week_unsplit_cache_write_tokens = 1;
    let text = super::overview(&plan, NOW);
    assert_eq!(text.matches(note).count(), 1, "{text}");
    let (before, after) = text.split_once("Cost\n").expect("a Cost section");
    assert!(!before.contains(note) && after.contains(note), "the note sits under Cost: {text}");
}

/// Without cost from the runner, nothing about cost is drawn.
#[test]
fn a_runner_that_sends_no_cost_draws_none() {
    let mut bare = pb::Plan { cost: None, ..the_plan() };
    bare.themes[0].budget_tokens = None;
    bare.themes[0].trend_tokens.clear();
    bare.lines[1].budget_tokens = None;
    let text = super::overview(&bare, NOW);
    assert!(!text.contains("Cost") && !text.contains("budget"), "{text}");
}

#[path = "plan_move_tests.rs"]
mod move_tests;

#[path = "plan_agent_tests.rs"]
mod agent_tests;

#[path = "plan_landing_tests.rs"]
mod landing_tests;

/// Every call `farcooler plan` makes, as text and as `--json`, is one a
/// rep's pane may make (`people::rep_may`, ov-527): the overview is a read.
#[tokio::test]
async fn the_plan_a_rep_reads_calls_only_what_a_rep_may() {
    for json in [false, true] {
        let mut link = runner();
        run_on(&mut link, &the_board(), parsed(""), "manager", json, NOW).await.expect("the overview");
        assert!(!link.sent.is_empty());
        for req in &link.sent {
            assert!(farcooler_daemon::people::rep_may(req), "{} isn't a rep's to call", req.method);
        }
    }
}
