//! Differential test (ov-1273): the old reducer and the pipeline core with `coding/v1`, side by side.
//!
//! Each scenario is one an existing reducer test drives (named in it): a template `Plan`, the tickets, and a
//! *world* that says what every step answers: which builds fail a gate or conflict at the fold, how often the
//! remote refuses a push, what CI and the reviewers say, and when the orchestrator steps in. The old reducer
//! runs it on `templates::build(plan)` and records its event log, which must replay to the same digest; the
//! pipeline runs it on `coding_templates::layout(plan)`. Both drivers take turns the same way (tickets in
//! order first, then the oldest landing group or batch), so what they're asked differs only where the rules
//! do. At the end each ticket's outcome (done, cancelled, held at a station, or still at one) and the landings
//! (which tickets each push carried) are compared. A difference must be one the scenario lists, and each listed
//! one is explained in reports/ov-1273/README.md as intended by the design.
#[path = "differential_support/mod.rs"]
mod differential;
use differential::*;

#[test]
fn old_and_new_reach_the_same_outcomes() {
    let all = scenarios();
    assert!(all.len() >= 20, "at least twenty scenarios");
    let mut unexpected = Vec::new();
    for s in &all {
        let (o, n) = run(s);
        let found = diff(&o, &n);
        let intended: Vec<String> = s.intended.iter().map(|d| d.to_string()).collect();
        if found != intended { unexpected.push(format!("{}:\n  found    {found:?}\n  intended {intended:?}\n  old {o:?}\n  new {n:?}", s.name)); }
    }
    assert!(unexpected.is_empty(), "{}", unexpected.join("\n"));
}
