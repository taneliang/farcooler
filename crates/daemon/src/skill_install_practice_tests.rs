//! The working practice the manager skill carries (ov-217): the two landing
//! modes, the plan as the status surface, rulings, landing hygiene and the
//! check-in. Each rule here is one a pressure scenario in
//! `scripts/manager-skill-pressure` also scores from what an agent ran; these
//! hold the words in place so a trim can't drop one silently.

use super::{Harness, render};

const ALL: [Harness; 3] = [Harness::Claude, Harness::Codex, Harness::Cursor];

/// The `SKILL.md` a harness gets, read as prose: where a line wraps is not
/// what's being checked.
fn prose(h: Harness) -> String {
    let text = render(h, "farcooler")
        .into_iter()
        .find(|f| f.relative.ends_with("SKILL.md"))
        .expect("every harness gets a SKILL.md")
        .contents;
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The text under a `## ` heading that starts with `title`, up to the next one.
fn section(body: &str, title: &str) -> String {
    let start = body.find(&format!(" ## {title}")).unwrap_or_else(|| panic!("no section {title}"));
    let rest = &body[start + 1..];
    let end = rest[3..].find(" ## ").map_or(rest.len(), |i| i + 3);
    rest[..end].to_string()
}

/// A repository whose main refuses pushes can't be run in direct mode,
/// whatever its charter says, and the interview asks which mode it is.
#[test]
fn the_landing_mode_is_known_before_anything_lands() {
    for h in ALL {
        let body = prose(h);
        let first = section(&body, "1. Read the charter");
        assert!(first.contains("`## Workflow` names the landing mode"), "{h:?}: {first}");
        assert!(first.contains("**Direct**") && first.contains("**PR**"), "{h:?}: {first}");
        assert!(first.contains("means PR mode whatever the charter says"), "{h:?}: {first}");
        let interview = section(&body, "The interview");
        assert!(interview.contains("or through PRs a person approves?"), "{h:?}: {interview}");
        assert!(interview.contains("whether main is protected"), "{h:?}: {interview}");
    }
}

/// PR mode, as the owner ruled it (Oct 5): approval is a person's, one PR per
/// card under the owner's login, and review is made faster rather than
/// rationed. A back-pressure rule must not creep back.
#[test]
fn pr_mode_never_lands_without_approval() {
    for h in ALL {
        let land = section(&prose(h), "4. Land");
        for rule in [
            "Never push main and never merge without the required approvals",
            "no admin bypass",
            "never approve",
            "One PR per card, under the owner's own `gh` login",
            "The train becomes a rehearsal",
            "never pushed",
            "Never slow dispatch for busy reviewers: make review faster",
        ] {
            assert!(land.contains(rule), "{h:?} lacks {rule:?}: {land}");
        }
        assert!(!land.to_lowercase().contains("back-pressure"), "{h:?}: {land}");
    }
}

/// The plan is the owner's first read, so a lane goes on it when it is
/// dispatched and moves as things happen, not at the next check-in.
#[test]
fn a_lane_goes_on_the_plan_with_its_dispatch() {
    for h in ALL {
        let third = section(&prose(h), "3. Dispatch, answer, or report");
        assert!(third.contains("Start the lane on the plan with the dispatch"), "{h:?}: {third}");
        assert!(third.contains("move it as each thing happens"), "{h:?}: {third}");
        let dispatch = third.find("farcooler task dispatch").expect("a dispatch command");
        let start = third.find("farcooler plan lane start").expect("a plan lane start command");
        assert!(dispatch < start, "{h:?}: the lane is started before anything is dispatched");
        assert!(third.contains("farcooler plan lane set <name>"), "{h:?}: {third}");
    }
}

/// A reversible call made for the owner is a ruling, recorded where the owner
/// can reverse it, and never left as an open question it didn't need to be.
#[test]
fn a_reversible_call_is_recorded_as_a_ruling() {
    for h in ALL {
        let third = section(&prose(h), "3. Dispatch, answer, or report");
        assert!(third.contains("is a ruling: make it, keep the work moving, and record it"), "{h:?}: {third}");
        assert!(third.contains("`plan ruling add --decision"), "{h:?}: {third}");
        // Until a CLI has the verb, the ruling still gets written down.
        assert!(third.contains("a `--kind decision` note starting \"Ruling:\""), "{h:?}: {third}");
    }
}

/// The owner keeps or reverses each ruling (ov-333): a kept one is precedent
/// to cite, a reversed one is marked with its commit and becomes a lesson and
/// a question next time, and the manager never keeps one for the owner.
#[test]
fn a_kept_ruling_is_precedent_and_a_reversed_one_is_a_lesson() {
    for h in ALL {
        let third = section(&prose(h), "3. Dispatch, answer, or report");
        assert!(third.contains("cite a kept one as precedent"), "{h:?}: {third}");
        assert!(third.contains("`plan ruling reverse R-12 [--sha <commit>]`, note the lesson as a decision note on its card, ask next time"), "{h:?}: {third}");
        assert!(third.contains("Never keep one for them."), "{h:?}: {third}");
    }
}

/// Landing ends with the card closed, the lane landed, the worktree removed
/// and its build output gone; a push or a rerun is watched, so a run nobody
/// looks at doesn't sit green and unlanded; and trains are kept on the plan
/// (ov-309), with a page's figures drawn live (ov-306), never a hand-kept
/// trains page.
#[test]
fn landing_is_watched_and_cleaned_up() {
    for h in ALL {
        let land = section(&prose(h), "4. Land");
        assert!(land.contains("After every push or CI rerun, take its run from `gh run list`"), "{h:?}: {land}");
        // A local bare remote made Sonnet decide there was no CI and skip
        // the watch (ov-323, S18): the run list decides, never the URL.
        assert!(land.contains("never decide there is no CI from the remote's URL"), "{h:?}: {land}");
        assert!(land.contains("start `gh run watch <id> --exit-status`"), "{h:?}: {land}");
        assert!(land.contains("as a background command"), "{h:?}: {land}");
        assert!(land.contains("delete its build output, and `worktree remove` it"), "{h:?}: {land}");
        assert!(land.contains("tick each verified `--met` line"), "{h:?}: {land}");
        assert!(land.contains("farcooler plan train start <integ-N> --repo <repo> --lane <lane>"), "{h:?}: {land}");
        assert!(land.contains("farcooler plan train set <integ-N> --repo <repo> --sha <pushed sha>"), "{h:?}: {land}");
        assert!(land.contains("plan train set <integ-N> --state landed"), "{h:?}: {land}");
        assert!(!land.contains("page set trains"), "{h:?}: the hand-kept trains page is gone: {land}");
        assert!(land.contains(r#"`{"ci":"main"}`"#) && land.contains(r#"`{"cards":"in_review"}`"#), "{h:?}: {land}");
        assert!(land.contains("Local gates mirror CI"), "{h:?}: {land}");
        assert!(land.contains("runs every gate once"), "{h:?}: {land}");
    }
}

/// A check-in re-evaluates: the stories, the structure, the ideas the owner
/// would want, and what went wrong. Initiative is filed either way and built
/// only as far as the charter's Autonomy allows.
#[test]
fn a_check_in_re_evaluates_and_shows_initiative() {
    for h in ALL {
        let body = prose(h);
        let check = section(&body, "5. Check in");
        for rule in [
            "re-evaluate rather than replay",
            "`plan theme set <name> --story",
            "reset Next Up",
            "the structure: do the themes still fit the work?",
            "File each as a card labeled `initiative`, with its evidence",
            "Build one only if `## Autonomy` allows it",
            "otherwise suggest it",
            "The owner's requests come first, unless the idea unblocks one",
            "one line in a lessons file",
            "Name each permission prompt",
        ] {
            assert!(check.contains(rule), "{h:?} lacks {rule:?}: {check}");
        }
        let interview = section(&body, "The interview");
        assert!(interview.contains("Your initiative: suggest ideas only"), "{h:?}: {interview}");
    }
}

/// The plan and page writes the skill shows name the manager, like the task
/// writes `every_write_the_skill_shows_names_the_manager` checks.
#[test]
fn every_plan_and_page_write_names_the_manager() {
    let text = render(Harness::Claude, "farcooler")
        .into_iter()
        .find(|f| f.relative.ends_with("SKILL.md"))
        .unwrap()
        .contents;
    let writes: Vec<&str> = text
        .lines()
        .filter(|l| ["farcooler plan lane ", "farcooler plan train ", "farcooler page set "].iter().any(|w| l.starts_with(w)))
        .collect();
    assert!(writes.len() >= 3, "too few plan writes to check: {writes:?}");
    for line in writes {
        assert!(line.contains("--actor manager"), "a write that doesn't name the manager: {line}");
    }
}
