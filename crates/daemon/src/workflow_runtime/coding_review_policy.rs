//! Prove review coverage over the admitted graph, rather than trusting a route's label (ov-1194).
use super::*;
use super::coding::CodingConfig;

pub(super) fn validate(config: &CodingConfig, definition: &Definition) -> Result<()> {
    let light_route = |stage: &Stage| stage.kind == StageKind::Route
        && stage.operation.arguments.get(1).is_some_and(|text| crate::review_agent::ReviewRoute::read(text).is_some());
    let policy = &config.security_reviews;
    if policy.is_empty() {
        return if definition.stages.iter().any(light_route) {
            Err(Error::Refused("light review requires a trusted full review policy"))
        } else { Ok(()) };
    }
    if policy.len() > 16 { return Err(Error::Refused("oversized security review policy")); }
    let mut required = Vec::new();
    for review in policy {
        let index = definition.stages.iter().position(|s| s.id == review.step)
            .ok_or(Error::Refused("missing required security review"))?;
        let stage = &definition.stages[index];
        if required.contains(&index) || stage.kind != StageKind::Review(Reviewer::Agent)
            || review.models.is_empty() || definition.models.get(&stage.id) != Some(&review.models)
            || stage.operation.arguments.get(2) != review.options.as_ref()
        { return Err(Error::Refused("required security review changed")); }
        required.push(index);
    }
    // Candidate-specific abstract execution. Only ReviewRoute's light edge certifies a non-sensitive
    // diff. All other labels (including PathRules) preserve the obligation. A new build resets it.
    // Track the ordered successful reviews, so a missing, rejected or reversed sequence cannot qualify.
    let mut pending = vec![(0usize, 0usize, false)];
    let mut seen = BTreeSet::new();
    while let Some((index, mut completed, mut light)) = pending.pop() {
        if !seen.insert((index, completed, light)) { continue; }
        let stage = &definition.stages[index];
        if stage.kind == StageKind::Build { completed = 0; light = false; }
        let covered = |count: usize, light: bool| light || count == required.len();
        if matches!(stage.kind, StageKind::Join { .. } | StageKind::Land | StageKind::PushBranch)
            && !covered(completed, light)
        { return Err(Error::Refused("a candidate can bypass required security reviews")); }
        let labels: BTreeSet<&str> = stage.routes.keys().map(String::as_str)
            .chain(["passed", "rejected"]).collect();
        for label in labels {
            let mut count = completed;
            if required.contains(&index) {
                if label == "passed" && required.get(count) == Some(&index) { count += 1; }
                else if label != "passed" { count = 0; }
            }
            let certified = if light_route(stage) { label == "light" } else { light };
            // A route has no implicit success: the executor only returns its classified labels.
            // Ignore impossible labels rather than requiring policy on fictitious route fallthroughs.
            if stage.kind == StageKind::Route && !stage.routes.contains_key(label) { continue; }
            let default = if label == "passed" {
                definition.stages.get(index + 1).map(|s| Target::Step(s.id.clone())).unwrap_or(Target::Done)
            } else { Target::Fail };
            match stage.routes.get(label).unwrap_or(&default) {
                Target::Step(id) => {
                    let next = definition.stages.iter().position(|s| &s.id == id).ok_or(Error::Refused("unknown security review target"))?;
                    pending.push((next, count, certified));
                }
                Target::Done if !covered(count, certified) => return Err(Error::Refused("completion bypasses required security reviews")),
                Target::Split { .. } => return Err(Error::Refused("unexpected ticket split")),
                _ => {}
            }
        }
    }
    Ok(())
}
