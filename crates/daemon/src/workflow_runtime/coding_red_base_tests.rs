//! ov-1291: Nov 16's group 2924, replayed from its saved receipts. It gated on group 2895's pushed head while 2895's
//! CI ran; that CI came back red, and the push step sent 2924 back to its base step (back edge push:1) for a second
//! gate of the same change on the same head. Now the gated group pushes over the red head and its own CI decides.
use std::time::Duration;
use farcooler_workflow::{digest, Command, EffectKind, Phase, StageKind, State};
use super::adapters;
use super::coding_verbs_tests::{lander, batched_push, at_join, command, until, phase_of, outbox, settled, remote_head};

const FIXTURE: &str = include_str!("../../../../test/fixtures/landing/nov16-group2924.json");
const PREVIOUS: [&str; 2] = ["ov-1266", "ov-1210"];
const GATED: [&str; 3] = ["ov-1199", "ov-1267", "ov-1277"];

#[tokio::test]
async fn nov16_group2924_pushes_over_the_red_ci_it_gated_on_and_sends_nobody_back() {
    let fixture: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
    let names = |key: &str| -> Vec<String> { fixture[key]["members"].as_array().unwrap().iter().map(|m| m.as_str().unwrap().to_owned()).collect() };
    assert_eq!((names("previous"), names("gated")), (PREVIOUS.map(String::from).to_vec(), GATED.map(String::from).to_vec()));
    let old = &fixture["old_engine"][0];
    assert_eq!((old["step"].as_str(), old["label"].as_str(), &old["back_edges"]["push"]), (Some("push"), Some("base_moved"), &1.into()),
        "the fixture records what the old engine did");
    // Group 2895's CI as the provider reported it: still running while 2924 gates, then the red conclusion.
    let polls = fixture["previous"]["ci_polls"].as_array().unwrap();
    let (running, red) = (&polls[polls.len() - 2]["run"], &polls[polls.len() - 1]["run"]);
    assert_eq!((running["status"].as_str(), red["conclusion"].as_str()), (Some("in_progress"), Some("failure")));

    let (dir, mut runtime) = lander("pass").await;
    let script = dir.path().join("adapter.py");
    let text = std::fs::read_to_string(&script).unwrap();
    std::fs::write(dir.path().join("running.json"), running.to_string()).unwrap();
    std::fs::write(dir.path().join("red.json"), red.to_string()).unwrap();
    // 2895's pushed candidate (the second one CI sees) answers from the fixture; every other candidate's CI passes.
    let replayed = r#"elif stage == 'ci':
    control = pathlib.Path(__file__).parent
    seen = control / 'ci-seen'
    order = seen.read_text().split() if seen.exists() else []
    if candidate != os.environ['RED'] and candidate not in order: order.append(candidate); seen.write_text(' '.join(order))
    if len(order) > 1 and candidate == order[1]:
        run = json.loads((control / ('red.json' if (control / 'release-ci').exists() else 'running.json')).read_text())
        run.update(head_sha=candidate, html_url='fixture://ci/' + str(run['id']))
        receipt.write_text(json.dumps(dict(binding=binding, repository='offline/repository', run=run)))
        sys.exit(0)
"#;
    std::fs::write(&script, text.replace("elif stage == 'ci':\n", replayed)).unwrap();
    let definition = batched_push(&runtime, "green", 30_000);
    // A group leaves as soon as the landing is free, so a landing ahead of 2895 holds it while 2895's two members
    // queue into one group, and 2895 holds it while 2924's three do.
    for item in ["ahead"].iter().chain(&PREVIOUS).chain(&GATED) { at_join(&mut runtime, item, &definition).await; }
    for item in ["ahead"].into_iter().chain(PREVIOUS) { command(&runtime, Command::Queue { item: item.into() }); }
    let state = runtime.state("w").unwrap().unwrap();
    let previous = state.items[PREVIOUS[0]].grouped_in.clone().unwrap();
    assert_eq!(state.items[PREVIOUS[1]].grouped_in.as_ref(), Some(&previous), "2895's two members share one group");
    let holding = |s: &State| s.items.get(&previous).is_some_and(|g| g.group_of.is_some()
        && matches!(g.definition.stages[g.stage].kind, StageKind::Fold | StageKind::Run));
    until(&mut runtime, holding).await;
    for item in GATED { command(&runtime, Command::Queue { item: item.into() }); }
    let state = runtime.state("w").unwrap().unwrap();
    let gated = state.items[GATED[0]].grouped_in.clone().unwrap();
    assert!(GATED.iter().all(|m| state.items[*m].grouped_in.as_ref() == Some(&gated)) && gated != previous, "2924 is one group of three");

    // 2924 folds on 2895's head and gates while that CI runs; at its push, the CI comes back red.
    let store = runtime.store.clone();
    let (release, watched) = (dir.path().join("release-ci"), gated.clone());
    let remote = runtime.coding.as_ref().unwrap().landing.as_ref().unwrap().repository.clone();
    let observer = tokio::spawn(async move {
        loop {
            let state = store.workflow_state("w").unwrap().unwrap();
            let group = &state.items[&watched];
            if group.definition.stages.get(group.stage).is_some_and(|s| s.kind == StageKind::Push) {
                std::fs::write(&release, "red").unwrap();
                let head = std::process::Command::new("/usr/bin/git").arg("--git-dir").arg(&remote)
                    .args(["rev-parse", "refs/heads/main"]).output().unwrap().stdout;
                return (group.candidate.clone(), String::from_utf8(head).unwrap().trim().to_owned());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });
    runtime.continuous = true;
    let state = until(&mut runtime, |s| GATED.iter().all(|m| phase_of(s, m) == Some(Phase::Done))).await;
    let (gated_on, pushed) = observer.await.unwrap();
    let gated_on = gated_on.expect("the gated group had a candidate at its push");
    assert_eq!(adapters::git(&runtime.repository(), &["merge-base", &pushed, &gated_on]).await.unwrap(), pushed,
        "2924 was folded and gated on top of 2895's pushed head");
    assert!(PREVIOUS.iter().all(|m| phase_of(&state, m) == Some(Phase::Failed)), "2895's own red CI still fails 2895");

    let bases = runtime.store.workflow_outbox("w").unwrap().iter().filter(|s|
        s.effect.item == gated && s.effect.kind == EffectKind::Execute(StageKind::AwaitBase)).count();
    assert_eq!(bases, 1, "no back edge from the push to the base step, so no second gate");
    let gates = runtime.store.workflow_outbox("w").unwrap().iter().filter(|s|
        s.effect.item == gated && s.effect.kind == EffectKind::Execute(StageKind::Run)).count();
    assert_eq!(gates, 1, "the change is gated once");
    let push = runtime.store.workflow_outbox("w").unwrap().into_iter().rev().find(|s|
        s.effect.item == gated && s.effect.kind == EffectKind::Execute(StageKind::Push)).unwrap();
    let fact: serde_json::Value = serde_json::from_slice(&std::fs::read(runtime.root.join("receipts")
        .join(format!("{}.verb.json", digest(&push.effect)))).unwrap()).unwrap();
    assert_eq!((fact["label"].as_str(), &fact["detail"]["red_base"]), (Some("passed"), &true.into()), "the push says it went over a red head");
    assert_eq!(remote_head(&runtime).await, gated_on, "2924's gated candidate is what landed");
    assert_eq!(settled(&runtime, StageKind::AwaitCi)["label"].as_str(), Some("passed"), "2924's own CI decided");
    assert_eq!(outbox(&runtime, StageKind::Build), 1 + PREVIOUS.len() + GATED.len(), "no member was sent back to build");
}
