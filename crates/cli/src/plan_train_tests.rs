//! Which lanes board a train (ov-503), said before anything is sent. A child
//! of `plan_tests.rs`, whose runner it drives.

use super::*;

fn methods(link: &Runner) -> Vec<&str> {
    link.sent.iter().map(|r| r.method.as_str()).collect()
}

/// A lane still in review is refused with where it is and how to board it, and
/// nothing is sent but the read.
#[tokio::test]
async fn a_lane_in_review_does_not_board_without_force() {
    let mut link = runner();
    let err = say(&mut link, "train start integ-10 --lane mac-ux").await.unwrap_err();
    assert_eq!(err.to_string(), "mac-ux is in review. Move it to landing when its review is clean, or add --force to put it on a train now.");
    assert_eq!(methods(&link), ["plan.get"]);
}

/// With --force, a lane in review is moved to landing by a lane write of its
/// own, and then the train starts with it.
#[tokio::test]
async fn force_moves_a_lane_in_review_to_landing_first() {
    let mut link = runner();
    say(&mut link, "train start integ-10 --lane mac-ux --force").await.unwrap();
    assert_eq!(methods(&link), ["plan.get", "lane.update", "train.start"]);
    let Some(request::Payload::LaneUpdate(p)) = &link.sent[1].payload else { panic!() };
    assert_eq!((p.lane_id.clone(), p.state, p.reason.as_deref()), (tid(0x2002), Some(pb::LaneState::Landing as i32), Some("Put on a train with --force.")));
}

/// A queued lane never boards, with or without --force, and neither does a
/// landed one. The refusal names the lane and nothing is moved.
#[tokio::test]
async fn a_queued_or_landed_lane_never_boards() {
    let mut link = runner();
    for cmd in ["train start integ-10 --lane mac-fu3", "train start integ-10 --lane mac-ux --lane mac-fu3 --force"] {
        let err = say(&mut link, cmd).await.unwrap_err();
        assert_eq!(err.to_string(), "mac-fu3 is queued. A train takes only lanes that are ready to land, so it waits for the next one.");
    }
    let err = say(&mut link, "train set integ-9 --add-lane fix-ac84 --force").await.unwrap_err();
    assert_eq!(err.to_string(), "fix-ac84 has already landed, so it can't go on a train.");
    assert!(methods(&link).iter().all(|m| *m == "plan.get"), "{:?}", methods(&link));
}

/// --add-lane follows the rule too; a lane already on the train is not asked
/// again.
#[tokio::test]
async fn adding_a_lane_follows_the_same_rule() {
    let mut link = runner();
    let err = say(&mut link, "train set integ-9 --add-lane mac-fu3").await.unwrap_err();
    assert!(err.to_string().starts_with("mac-fu3 is queued."), "{err}");
    say(&mut link, "train set integ-9 --add-lane mac-ux").await.unwrap();
    assert_eq!(methods(&link).last(), Some(&"train.set"), "mac-ux is on integ-9 already");
    assert!(!methods(&link).contains(&"lane.update"));
}

/// A name in use, or a train that has landed, is refused before a forced lane
/// is moved to landing.
#[tokio::test]
async fn nothing_is_moved_for_a_train_that_cannot_take_it() {
    let mut link = runner();
    let err = say(&mut link, "train start INTEG-9 --lane mac-ux --force").await.unwrap_err();
    assert!(err.to_string().starts_with("That train name is already used"), "{err}");
    assert_eq!(methods(&link), ["plan.get"]);
}

/// Two trains still integrating, `integ-1` first. On `integ-1`: `go-one` and
/// `ready-one` ready to land, `slow-one` sent back for a fix. The first train
/// carries `held_by` as a runner before Train 119 sent it.
fn held_plan() -> pb::Plan {
    let mut plan = the_plan();
    let on = |mut l: pb::Lane, train: &str| {
        l.train = Some(train.into());
        l
    };
    plan.lanes = vec![
        on(lane(10, "go-one", pb::LaneState::Landing, &[1]), "integ-1"),
        on(lane(11, "ready-one", pb::LaneState::Landing, &[2]), "integ-1"),
        on(lane(12, "slow-one", pb::LaneState::Fixing, &[3]), "integ-1"),
        on(lane(13, "last-one", pb::LaneState::Landing, &[1]), "integ-2"),
    ];
    let train = |n: u128, name: &str, created: i64, lanes: &[u128]| pb::BoardTrain {
        id: tid(0x6000 + n),
        workspace_id: tid(0x0202),
        name: name.into(),
        title: format!("Train {n}"),
        state: pb::BoardTrainState::Integrating as i32,
        created_at: created,
        state_since: created,
        lane_ids: lanes.iter().map(|l| tid(0x2000 + l)).collect(),
        ..Default::default()
    };
    let mut first = train(1, "integ-1", NOW - 4 * HOUR, &[10, 11, 12]);
    first.phase = pb::BoardTrainPhase::WaitingForLanes as i32;
    first.phase_since = NOW - 2 * HOUR;
    first.held_by = vec![tid(0x2000 + 12)];
    plan.trains = vec![first, train(2, "integ-2", NOW - 3 * HOUR, &[13])];
    plan
}

fn sent_set(link: &Runner) -> &pb::TrainSet {
    let Some(request::Payload::TrainSet(p)) = &link.sent.last().expect("sent").payload else { panic!("not a train.set") };
    p
}

/// A runner before Train 119 still sends `held_by`: the list says where the
/// train stands and nothing of a hold, and `--json` carries no `held_by`.
#[tokio::test]
async fn a_train_from_an_old_runner_says_nothing_of_a_hold() {
    let mut link = runner();
    link.plan = Some(held_plan());
    let out = say(&mut link, "train list").await.unwrap();
    assert!(out.contains("Train 1 (integ-1) · Waiting for lanes"), "{out}");
    assert!(out.contains("waiting for lanes for 2h"), "{out}");
    assert!(!out.contains("Waiting for slow-one"), "{out}");
    assert!(!out.contains("takes the lane off"), "{out}");
    assert!(!out.contains("--actor manager"), "{out}");
    assert!(!out.contains("Train 2 (integ-2) · Waiting"), "the other train is waiting for nothing");
    let json = run_on(&mut link, &the_board(), parsed("train list"), "manager", true, NOW).await.unwrap();
    assert!(!json.contains("held_by"), "{json}");
}

/// A ready lane moves to the next train in one write on it.
#[tokio::test]
async fn a_ready_lane_moves_to_the_next_train() {
    let mut link = runner();
    link.plan = Some(held_plan());
    let out = say(&mut link, "train move go-one").await.unwrap();
    assert_eq!(out, "Moved go-one from Train 1 to Train 2.");
    assert_eq!(methods(&link), ["plan.get", "train.set"]);
    let p = sent_set(&link);
    assert_eq!((p.train_id.clone(), p.add_lane_ids.clone(), p.remove_lane_ids.len()), (tid(0x6002), vec![tid(0x2000 + 10)], 0));
}

/// A lane that is not ready can't board another train: it comes off, and
/// waits.
#[tokio::test]
async fn a_lane_that_is_not_ready_comes_off_and_waits() {
    let mut link = runner();
    link.plan = Some(held_plan());
    let out = say(&mut link, "train move slow-one").await.unwrap();
    assert_eq!(out, "Took slow-one off Train 1. It waits until it's ready to land, then joins a train.");
    let p = sent_set(&link);
    assert_eq!((p.train_id.clone(), p.add_lane_ids.len(), p.remove_lane_ids.clone()), (tid(0x6001), 0, vec![tid(0x2000 + 12)]));
}

/// With no later train, even a ready lane just comes off.
#[tokio::test]
async fn with_no_next_train_a_lane_goes_back_to_waiting() {
    let mut link = runner();
    link.plan = Some(held_plan());
    let out = say(&mut link, "train move last-one").await.unwrap();
    assert_eq!(out, "Took last-one off Train 2. It's ready to land and joins the next train that starts.");
    assert_eq!(sent_set(&link).remove_lane_ids, vec![tid(0x2000 + 13)]);
}

/// Refusals send nothing: a train named outright takes only a ready lane, a
/// pushed train keeps its lanes, and a lane on no train has nothing to leave.
#[tokio::test]
async fn a_move_that_cannot_happen_is_refused_before_anything_is_sent() {
    let mut link = runner();
    link.plan = Some(held_plan());
    let err = say(&mut link, "train move slow-one --to integ-2").await.unwrap_err();
    assert!(err.to_string().starts_with("slow-one is fixing. A train takes only lanes that are ready to land"), "{err}");
    let err = say(&mut link, "train move go-one --to integ-1").await.unwrap_err();
    assert!(err.to_string().contains("isn't integrating"), "{err}");

    let mut pushed = held_plan();
    pushed.trains[0].state = pb::BoardTrainState::Pushed as i32;
    pushed.trains[0].pushed_sha = Some("1a1b3275".into());
    link.plan = Some(pushed);
    let err = say(&mut link, "train move go-one").await.unwrap_err();
    assert!(err.to_string().contains("has already gone to gating or CI"), "{err}");

    link.plan = Some(the_plan());
    let err = say(&mut link, "train move mac-fu3").await.unwrap_err();
    assert_eq!(err.to_string(), "mac-fu3 isn't on a train.");
    assert!(methods(&link).iter().all(|m| *m == "plan.get"), "{:?}", methods(&link));
}

// ---- the open train (ov-520) ----

/// The open train, waiting behind a train that is checking.
fn open_train_plan() -> pb::Plan {
    let mut plan = the_plan();
    plan.lanes = vec![lane(10, "ahead-one", pb::LaneState::Landing, &[1]), lane(11, "next-one", pb::LaneState::Landing, &[2])];
    plan.lanes[0].train = Some("train-1".into());
    plan.lanes[1].train = Some("train-2".into());
    let train = |n: u128, name: &str, state: pb::BoardTrainState, phase: pb::BoardTrainPhase, lane: u128| pb::BoardTrain {
        id: tid(0x6000 + n),
        workspace_id: tid(0x0202),
        name: name.into(),
        title: format!("Train {n}"),
        state: state as i32,
        phase: phase as i32,
        created_at: NOW - (3 - n as i64) * HOUR,
        state_since: NOW - HOUR,
        phase_since: NOW - HOUR,
        lane_ids: vec![tid(0x2000 + lane)],
        ..Default::default()
    };
    plan.trains = vec![
        train(1, "train-1", pb::BoardTrainState::Gating, pb::BoardTrainPhase::GatingLocally, 10),
        train(2, "train-2", pb::BoardTrainState::Integrating, pb::BoardTrainPhase::WaitingForPreviousTrain, 11),
    ];
    plan
}

/// The open train reads "Waiting for the earlier landing" in the list and in
/// `--json`, and the word the owner struck never appears.
#[tokio::test]
async fn the_open_train_says_it_is_waiting_for_the_previous_train() {
    let mut link = runner();
    link.plan = Some(open_train_plan());
    let out = say(&mut link, "train list").await.unwrap();
    assert!(out.contains("Train 2 (train-2) · Waiting for the earlier landing"), "{out}");
    assert!(out.contains("waiting for the earlier landing for 1h"), "{out}");
    assert!(out.contains("Train 1 (train-1) · Checking here"), "{out}");
    assert!(!out.to_lowercase().contains("boarding"), "{out}");
    let json = run_on(&mut link, &the_board(), parsed("train list"), "manager", true, NOW).await.unwrap();
    assert!(json.contains("\"phase\":\"waiting_for_previous_train\""), "{json}");
    assert!(!json.to_lowercase().contains("boarding"), "{json}");
}

/// No word a person reads says "boarding": not a phase, not a refusal, not the
/// help of a `plan train` command, and not a string literal in the CLI's own
/// source (comments and test names may use the old word). Owner, Oct 14: let's
/// not use train terms too much.
#[test]
fn nothing_a_person_reads_says_boarding() {
    use clap::CommandFactory;
    let bad = |text: &str| text.to_lowercase().contains("boarding");
    for phase in 0..=12 {
        if let Some(word) = crate::ci_words::train_phase_word(phase) {
            assert!(!bad(word), "phase {phase}: {word}");
        }
    }
    assert_eq!(crate::ci_words::train_phase_word(pb::BoardTrainPhase::WaitingForPreviousTrain as i32), Some("Waiting for the earlier landing"));
    assert_eq!(crate::ci_words::train_phase_word(pb::BoardTrainPhase::WaitingForMain as i32), Some("Waiting for main's checks to pass"));
    for what in ["name", "name_taken", "base", "sha", "train_settled", "lane_not_ready", "lane_in_review", "train_unknown", "train_moving", "train_changed", "lane_closed"] {
        if let Some(said) = said_here(what) {
            assert!(!bad(said), "{what}: {said}");
        }
    }
    let mut help = String::new();
    fn walk(cmd: &clap::Command, out: &mut String) {
        out.push_str(&cmd.clone().render_long_help().to_string());
        for sub in cmd.get_subcommands() {
            walk(sub, out);
        }
    }
    walk(&crate::Cli::command(), &mut help);
    assert!(help.contains("train"), "the walk found the train commands");
    assert!(!bad(&help), "the help says boarding");
    // The source's string literals: a quoted run of text with the word in it.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap().flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if !name.ends_with(".rs") || name.ends_with("_tests.rs") || name == "plan_train_tests.rs" {
            continue;
        }
        for (n, line) in std::fs::read_to_string(&path).unwrap().lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            if code.split('"').skip(1).step_by(2).any(bad) {
                found.push(format!("{name}:{}: {code}", n + 1));
            }
        }
    }
    assert!(found.is_empty(), "boarding in a string: {found:#?}");
}

/// Whatever the CLI prints, in text and in `--json`, for success and for
/// refusal, never says "boarding" (the whole output, not only the literals the
/// source scan above reads).
#[tokio::test]
async fn no_output_the_cli_prints_says_boarding() {
    let plans = [the_plan(), held_plan(), open_train_plan()];
    let commands = [
        "",
        "train list",
        "train start integ-10 --lane mac-ux --force",
        "train start integ-10 --lane mac-fu3",
        "train start INTEG-9 --lane mac-ux",
        "train set integ-9 --add-lane mac-ux",
        "train set integ-9 --add-lane mac-fu3",
        "train set integ-9 --state gating",
        "train move go-one",
        "train move slow-one",
        "train move last-one --to integ-1",
        "train move ahead-one",
        "train move next-one --to train-1",
        "lane list",
    ];
    let mut seen = 0;
    for plan in &plans {
        for json in [false, true] {
            for cmd in commands {
                let mut link = runner();
                link.plan = Some(plan.clone());
                let Some(parsed) = crate::Cli::try_parse_from(format!("farcooler plan {cmd} --workspace main").split_whitespace()).ok().and_then(|cli| match cli.command {
                    crate::Command::Plan(a) => Some(a.cmd),
                    _ => None,
                }) else {
                    continue;
                };
                let said = match run_on(&mut link, &the_board(), parsed, "manager", json, NOW).await {
                    Ok(out) => out,
                    Err(err) => err.to_string(),
                };
                seen += 1;
                assert!(!said.to_lowercase().contains("boarding"), "`{cmd}` (json {json}): {said}");
            }
        }
    }
    assert!(seen > 30, "the commands ran: {seen}");
}
