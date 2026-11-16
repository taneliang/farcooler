//! The four landing templates as schema-3 layouts for `coding/v1` (ov-1273): each validates against the
//! pipeline core with the coding adapter linked, and the order rules moved from definition.rs refuse what the
//! old validator refused.
use std::collections::BTreeSet;
use farcooler_workflow::adapters::{coding_templates::layout, linked};
use farcooler_workflow::pipeline::*;
use farcooler_workflow::templates::{Gate, PathRoutes, Plan, Template};
use serde_json::{json, Value};

fn pools() -> BTreeSet<String> { ["agents", "work", "landing"].iter().map(|s| s.to_string()).collect() }
fn config() -> Value { json!({ "repository": ".", "target": "main", "ci": "ci.yml", "checks": ["cargo test", "cargo clippy"] }) }
fn check(l: &Layout) -> Result<(), Vec<String>> {
    Graph::validate(l, &linked(), &pools()).map(|_| ()).map_err(|p| p.into_iter()
        .map(|p| format!("{}: {}", p.station.unwrap_or_default(), p.message)).collect())
}
fn plan(t: Template) -> Plan { Plan::new(t, vec![Gate::Command(0)]) }
fn ids(stations: &[StationDef]) -> Vec<&str> { stations.iter().map(|s| s.id.as_str()).collect() }

#[test]
fn every_template_is_a_valid_layout() {
    let all = vec![Gate::Command(0), Gate::Reviewer { argument: "reviewer".into() }, Gate::Agent,
        Gate::Orchestrator { principal: "orchestrator".into() }, Gate::Person { principal: "owner".into() }, Gate::Command(1)];
    for t in Template::ALL {
        for gates in [vec![Gate::Command(0)], all.clone()] {
            for (ci, green) in [(true, true), (false, false)] {
                let mut p = Plan::new(t, gates.clone());
                p.ci = ci; p.green_base = green; p.batch = Some(3);
                let l = layout(&p, &config()).unwrap();
                assert_eq!(check(&l), Ok(()), "{} with {} gates, ci {ci}", t.name(), gates.len());
            }
        }
    }
    // Path routes with and without an approver, on both push templates.
    for t in [Template::BatchedPush, Template::TicketPush] {
        for approver in [None, Some("owner".to_string())] {
            let mut p = plan(t);
            p.paths = Some(PathRoutes { argument: "rules".into(), approver });
            assert_eq!(check(&layout(&p, &config()).unwrap()), Ok(()), "{}", t.name());
        }
    }
}

#[test]
fn each_template_has_the_shape_its_name_promises() {
    let push = layout(&plan(Template::BatchedPush), &config()).unwrap();
    assert_eq!(ids(&push.stations), ["provision", "build", "join"]);
    let join = &push.stations[2];
    // CI runs on the candidate's branch before the push (ov-1269).
    assert_eq!(ids(&join.stations), ["base", "fold", "candidate", "gate-0", "ci", "push"]);
    assert_eq!(join.stations[4].routes.get("red").map(String::as_str), Some("rejected"), "a red CI splits like a gate");
    assert_eq!(join.form.as_ref().map(|f| (f.size, f.free_on_crossing)), Some((None, true)), "no batch is no cap");
    assert_eq!((join.on_reject, join.policy.returns), (OnReject::Bisect, Some(1)));
    assert_eq!(join.routes.get("returned").map(String::as_str), Some("build"));
    assert_eq!(join.stations[5].routes.get("refused").map(String::as_str), Some("base"), "a refused push refolds");
    // Per ticket is the same layout with a batch of one.
    let single = layout(&plan(Template::TicketPush), &config()).unwrap();
    assert_eq!(single.stations[2].form.as_ref().unwrap().size, Some(1));
    let pr = layout(&plan(Template::TicketPr), &config()).unwrap();
    assert_eq!(ids(&pr.stations), ["provision", "build", "gate-0", "branch", "pr", "merged", "ci"]);
    assert_eq!(pr.stations[2].op.as_deref(), Some("coding.check:0"), "without a fold the command gate judges the ticket");
    let rehearsal = layout(&plan(Template::BatchedPr), &config()).unwrap();
    assert_eq!(ids(&rehearsal.stations), ["provision", "build", "branch", "pr", "rehearse", "ready", "merged"]);
    assert_eq!(ids(&rehearsal.stations[4].stations), ["base", "fold", "gate-0"]);
    assert_eq!(rehearsal.stations[4].policy.returns, Some(2));
    // A person's gate is a decision station that routes its verdicts.
    let mut p = plan(Template::TicketPush);
    p.gates.push(Gate::Person { principal: "owner".into() });
    let l = layout(&p, &config()).unwrap();
    let gate = l.stations.iter().find(|s| s.id == "gate-0").unwrap();
    assert_eq!((gate.kind, gate.deciders.clone()), (Kind::Decision, vec!["person:owner".to_string()]));
    assert_eq!(gate.routes.get("approved").map(String::as_str), Some("join"));
}

#[test]
fn a_push_with_no_command_gate_is_not_built() {
    for gates in [vec![], vec![Gate::Reviewer { argument: "reviewer".into() }], vec![Gate::Person { principal: "owner".into() }]] {
        assert_eq!(layout(&Plan::new(Template::BatchedPush, gates), &config()), Err(Refusal::Definition));
    }
    let mut p = plan(Template::TicketPr);
    p.paths = Some(PathRoutes { argument: "rules".into(), approver: None });
    assert_eq!(layout(&p, &config()), Err(Refusal::Definition), "path routes are for the push templates");
}

/// Edit the batched-push layout and say what the validator finds.
fn broken(edit: impl FnOnce(&mut Layout)) -> Vec<String> {
    let mut l = layout(&plan(Template::BatchedPush), &config()).unwrap();
    edit(&mut l);
    check(&l).expect_err("the edit must be refused")
}
/// A station inside the batched-push layout's `join`, by id.
fn inner<'a>(l: &'a mut Layout, id: &str) -> &'a mut StationDef { l.stations[2].stations.iter_mut().find(|s| s.id == id).unwrap() }
fn has(problems: &[String], text: &str) -> bool { problems.iter().any(|p| p.contains(text)) }

#[test]
fn the_order_rules_refuse_what_the_old_validator_refused() {
    // A push with no gate in front of it.
    let p = broken(|l| { l.stations[2].stations.retain(|s| s.id != "gate-0"); });
    assert!(has(&p, "every path into a push passes a gate"), "{p:?}");
    // A gate's failure that goes on toward the push.
    let p = broken(|l| { inner(l, "gate-0").routes.insert("red".into(), "push".into()); });
    assert!(has(&p, "failing label red goes on"), "{p:?}");
    let p = broken(|l| { inner(l, "gate-0").routes.insert("red".into(), "finish".into()); });
    assert!(has(&p, "failing label red goes on"), "{p:?}");
    // A fold with no base before it has nothing to fold onto.
    let p = broken(|l| { l.stations[2].stations.remove(0); inner(l, "push").routes.insert("refused".into(), "fold".into()); });
    assert!(has(&p, "fold: it needs a candidate"), "{p:?}");
    // A landing step outside a batch, and a ticket's step inside one.
    let p = broken(|l| { let base = l.stations[2].stations[0].clone(); l.stations.insert(2, base); });
    assert!(has(&p, "a landing step runs inside a batch"), "{p:?}");
    let p = broken(|l| { let build = l.stations[1].clone(); l.stations[2].stations.insert(1, build); });
    assert!(has(&p, "a ticket's own step can't run inside a batch"), "{p:?}");
    // A route after the push that leads back to it: the core's once-rule (5.3).
    let p = broken(|l| {
        let mut after = inner(l, "ci").clone();
        after.id = "ci-after".into();
        after.routes.insert("red".into(), "base".into());
        l.stations[2].stations.push(after);
    });
    assert!(has(&p, "leads back to boundary coding:target:main"), "{p:?}");
    // A gate that names a check the config doesn't have.
    let p = broken(|l| { inner(l, "gate-0").op = Some("coding.gate:7".into()); });
    assert!(has(&p, "not in its adapter's catalog"), "{p:?}");
}

#[test]
fn the_pull_request_order_rules() {
    let pr = || layout(&plan(Template::TicketPr), &config()).unwrap();
    let mut l = pr();
    l.stations.retain(|s| s.id != "pr");
    assert!(has(&check(&l).unwrap_err(), "opened before it is readied or awaited"));
    let mut l = pr();
    l.stations.iter_mut().find(|s| s.id == "merged").unwrap().routes.insert("changes_requested".into(), "gate-0".into());
    assert!(has(&check(&l).unwrap_err(), "goes back to a build"));
    // A merge crossed the target: nothing after it may lead back to it.
    let mut l = pr();
    l.stations.iter_mut().find(|s| s.id == "ci").unwrap().routes.insert("red".into(), "build".into());
    assert!(has(&check(&l).unwrap_err(), "leads back to boundary coding:target:main"));
}
