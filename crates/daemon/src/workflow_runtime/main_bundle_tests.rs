//! ov-1071: Main's workflow, run by the lane rules. Checks the bundle the owner publishes, read from the file named by
//! `FARCOOLER_OV1071_BUNDLE` (`.claude/agent/reports/ov-1071/workflow-main.json`). Run it by hand on the runner:
//! `FARCOOLER_OV1071_BUNDLE=<workflow-main.json> cargo test -p farcooler-daemon --lib main_bundle -- --ignored`.
//! With `FARCOOLER_OV1071_REPIN=1` it first rewrites every operation's configuration pin to the adapter's.
use super::*;
use crate::review_agent::ReviewOptions;
use farcooler_protocol::workflow_config::{AdapterConfiguration, CodingHarness};

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct Bundle { definition: farcooler_protocol::workflow_commands::Definition, adapter: Option<AdapterConfiguration> }

const RULES: &str = "/Users/e-liang/Dev/overnight/.claude/agent/reports/lanes/rules.md";

#[test]
#[ignore = "checks a draft file the owner has yet to publish; set FARCOOLER_OV1071_BUNDLE"]
fn main_bundle_runs_the_lane_rules() {
    let path = std::env::var("FARCOOLER_OV1071_BUNDLE").expect("FARCOOLER_OV1071_BUNDLE names the bundle file");
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let mut bundle: Bundle = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(&bundle).unwrap(), value, "the CLI refuses a bundle that doesn't round-trip");
    let Some(AdapterConfiguration::Coding(live)) = bundle.adapter.clone() else { panic!("a coding adapter") };
    let pin = live.pin();
    if std::env::var_os("FARCOOLER_OV1071_REPIN").is_some() {
        let d = &mut bundle.definition;
        for op in d.stages.iter_mut().map(|s| &mut s.operation).chain(d.groups.values_mut().flat_map(|g| g.steps.iter_mut().map(|s| &mut s.operation))) {
            op.arguments[0] = pin.clone();
        }
        std::fs::write(&path, serde_json::to_vec_pretty(&bundle).unwrap()).unwrap();
    }
    let d = &bundle.definition;
    let operations: Vec<_> = d.stages.iter().map(|s| &s.operation).chain(d.groups.values().flat_map(|g| g.steps.iter().map(|s| &s.operation))).collect();
    assert!(operations.len() > 5 && operations.iter().all(|o| o.arguments[0] == pin), "every operation pins the configuration {pin}");
    d.validate().unwrap();

    // 1. Everything ready lands together, and a full group still splits down to one ticket.
    let landing = &d.groups["landing"];
    assert_eq!((landing.limit, landing.cap()), (0, 32));
    for id in ["fold", "gate-0"] { assert_eq!(landing.steps.iter().find(|s| s.id == id).unwrap().limit, 5, "{id}"); }

    // 2. The build prompt and the review prompt point at the lane rules.
    let CodingHarness::Native { prompt, .. } = &live.harness else { panic!("native harness") };
    assert!(prompt.contains(RULES) && prompt.contains("scripts/heavy") && prompt.contains("cargo fmt") && prompt.contains("stash")
        && prompt.contains("init.defaultBranch = master") && prompt.contains("captures") && prompt.contains("net hand-written app lines"), "{prompt}");
    let gate = d.stages.iter().find(|s| s.id == "gate-0").unwrap();
    assert_eq!(gate.kind, StageKind::Review(Reviewer::Agent));
    let options = ReviewOptions::read(&gate.operation.arguments[2]).expect("sound review options");
    let words = options.prompt.as_deref().unwrap();
    assert!(words.contains(RULES) && words.contains("captures") && words.contains("narrowest width"), "{words}");

    // 3. Engine and security work is reviewed by Opus, the rest by the step's default; four rounds of fixes.
    assert_eq!(d.models["gate-0"][0], "claude:sonnet");
    let paths = |list: &[&str]| list.iter().map(|p| p.to_string()).collect::<Vec<_>>();
    assert_eq!(options.model_for(&paths(&["crates/workflow/src/reducer.rs"])), Some("claude:opus"));
    assert_eq!(options.model_for(&paths(&["apps/web/src/a.ts", "crates/daemon/src/workflow_runtime/coding.rs"])), Some("claude:opus"));
    assert_eq!(options.model_for(&paths(&["crates/daemon/src/runner_key.rs"])), Some("claude:opus"));
    assert_eq!(options.model_for(&paths(&["services/relay/src/main.rs"])), Some("claude:opus"));
    assert_eq!(options.model_for(&paths(&["apps/web/src/a.ts", "docs/x.md"])), None, "the run's pick stands");
    assert_eq!(gate.routes.get("rejected"), Some(&Target::Step("build".into())));
    assert_eq!(gate.limit, 4);

    // 4. CI after the push waits on CI, Web and Doc comments; the look reads the workflow the engine names.
    let ci = landing.steps.iter().find(|s| s.kind == StageKind::AwaitCi).unwrap();
    assert_eq!((live.ci.workflow.as_str(), ci.operation.arguments[4].as_str()), ("CI", r#"["Web","Doc comments"]"#));
    assert!(live.ci.command.argv[2].contains("env.FARCOOLER_CI_WORKFLOW") && live.ci.command.argv[2].contains("|| exit 0"));

    // The longest build brief, a fix round on several tickets, fits one launch command.
    let effect: Effect = serde_json::from_value(serde_json::json!({ "id": "e", "item": "ov-1071", "run": "r", "epoch": 1, "fence": 1,
        "resource": "x", "definition_pin": "d", "policy_pin": "p", "candidate": null, "kind": { "Execute": "Build" }, "with": ["ov-1072", "ov-1073"],
        "operation": { "adapter_version": "a", "arguments": [], "environment": {} }, "created_ms": 0 })).unwrap();
    let brief = Exec::briefed(prompt, &effect, 4);
    assert!(crate::service::shell_quote(&crate::service::shell_quote(&brief)).len() < crate::service::MAX_INLINE_LAUNCH_BYTES, "{}", brief.len());

    // On this runner, with its pinned files in place, the runtime takes the definition as published.
    if live.repository.exists() {
        live.validate_definition(&d.clone()).unwrap();
    }
}
