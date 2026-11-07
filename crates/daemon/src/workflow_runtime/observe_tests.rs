//! A result the engine refuses fails its item, never the service (ov-956).
use super::*;
use super::coding_tests::admitted;
use super::tests::enqueue;

async fn running() -> (tempfile::TempDir, Runtime, Effect) {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
    enqueue(&mut runtime, "w", "one", coding_definition(false));
    let binding = admitted(&runtime).await;
    (dir, runtime, binding.effect)
}
fn evidence(policy_pin: &str) -> Evidence {
    Evidence { candidate: "a".repeat(40), manifest: "m".into(), receipt: "r".into(), policy_pin: policy_pin.into() }
}

#[tokio::test]
async fn evidence_for_another_policy_makes_the_item_uncertain_and_the_service_goes_on() {
    let (_dir, mut runtime, effect) = running().await;
    let applied = runtime.observe("w", &effect, Outcome::Passed(evidence("foreign")), false, 5)
        .expect("a refused result must not be an error that ends the service");
    assert!(applied, "the item was settled");
    assert_eq!(runtime.state("w").unwrap().unwrap().items["one"].phase, Phase::Uncertain);
    runtime.pump(6).await.unwrap();
}
#[tokio::test]
async fn a_label_the_step_does_not_declare_makes_the_item_uncertain() {
    let (_dir, runtime, effect) = running().await;
    let policy = effect.policy_pin.clone();
    assert!(runtime.observe("w", &effect, Outcome::Labeled("nonsense".into(), evidence(&policy)), false, 5).unwrap());
    assert_eq!(runtime.state("w").unwrap().unwrap().items["one"].phase, Phase::Uncertain);
}
#[tokio::test]
async fn a_quarantined_item_is_left_alone_and_the_service_goes_on() {
    let (_dir, mut runtime, effect) = running().await;
    farcooler_store::testing::exec_for_tests(&runtime.store, &format!(
        "INSERT INTO engine_cutover_quarantine VALUES('{}','test')", effect.resource));
    let before = runtime.state("w").unwrap().unwrap();
    let applied = runtime.observe("w", &effect, Outcome::Uncertain, false, 5).expect("a quarantined item is not a service error");
    assert!(!applied, "nothing could be written");
    assert_eq!(runtime.state("w").unwrap().unwrap(), before);
    runtime.pump(6).await.unwrap();
}
