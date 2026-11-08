//! The working practice the manager skill carries (ov-217): the two landing
//! modes, the plan as the status surface, rulings, landing hygiene and the
//! check-in. Each rule here is one a pressure scenario in
//! `scripts/manager-skill-pressure` also scores from what an agent ran; these
//! hold the words in place so a trim can't drop one silently.

use super::{Harness, render};

const ALL: [Harness; 3] = [Harness::Claude, Harness::Codex, Harness::Cursor];

/// The actual rendered `SKILL.md`, retaining heading and fence boundaries.
fn skill(h: Harness) -> String {
    render(h, "farcooler")
        .into_iter()
        .find(|f| f.relative.ends_with("SKILL.md"))
        .expect("every harness gets a SKILL.md")
        .contents
}

fn prose(h: Harness) -> String {
    skill(h).split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn only_unfenced_heading_lines_end_a_section() {
    let body = "## First\nRead `## Inline` too.\n```markdown\n## Example\n```\n\
        Keep this\nwrapped rule.\n## Next\nLeave this out.\n";
    assert_eq!(section(body, "First"), "## First Read `## Inline` too. ```markdown ## Example ``` Keep this wrapped rule.");
    assert_eq!(section(body, "Next"), "## Next Leave this out.");
}

/// The text under a `## ` heading that starts with `title`, up to the next one.
fn section(body: &str, title: &str) -> String {
    let mut fenced = false;
    let mut found = false;
    let mut lines = Vec::new();
    for line in body.lines() {
        if line.starts_with("```") {
            fenced = !fenced;
        }
        if !fenced && let Some(heading) = line.strip_prefix("## ") {
            if found {
                break;
            }
            found = heading.starts_with(title);
        }
        if found {
            lines.push(line);
        }
    }
    assert!(found, "no section {title}");
    lines.join("\n").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The real asset includes a charter example with its own `## Landing`.
/// It must not hide the manual landing and PR rules that follow it.
#[test]
fn a_charter_example_does_not_end_the_landing_section() {
    for h in ALL {
        let land = section(&skill(h), "4. Land");
        assert!(land.contains("- Route: push"), "{h:?}: {land}");
        assert!(land.contains("Never push main and never merge without the required approvals"), "{h:?}: {land}");
        assert!(!land.contains("At each check-in"), "{h:?}: the next section leaked in");
    }
}

/// A repository whose main refuses pushes can't be run in direct mode,
/// whatever its charter says, and the interview asks which mode it is.
#[test]
fn the_landing_mode_is_known_before_anything_lands() {
    for h in ALL {
        let body = skill(h);
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
        let land = section(&skill(h), "4. Land");
        for rule in [
            "Never push main and never merge without the required approvals",
            "no admin bypass",
            "never approve",
            "One PR per card, under the owner's own `gh` login",
            "The landing becomes a rehearsal",
            "never pushed",
            "Never slow dispatch for busy reviewers: make review faster",
        ] {
            assert!(land.contains(rule), "{h:?} lacks {rule:?}: {land}");
        }
        assert!(!land.to_lowercase().contains("back-pressure"), "{h:?}: {land}");
    }
}

/// The plan is the owner's first read, so a ticket goes on the line when its
/// agent is dispatched and moves as things happen, not at the next check-in.
#[test]
fn a_ticket_goes_on_the_line_with_its_dispatch() {
    for h in ALL {
        let third = section(&skill(h), "3. Dispatch, answer, or report");
        // An agent dispatch puts its tickets on the line and records the agent
        // by itself (ov-457, ov-872); a subagent is recorded by hand beside
        // its launch.
        assert!(third.contains("puts them on the line and records the agent itself"), "{h:?}: {third}");
        assert!(third.contains("Move each ticket on the line as each thing happens"), "{h:?}: {third}");
        assert!(third.contains("farcooler agent dispatch --role builder --ticket <key>"), "{h:?}: {third}");
        assert!(third.contains("farcooler agent record --role reviewer --subagent <agentId> --ticket <key>"), "{h:?}: {third}");
        // Agents report to the orchestrator (ov-455), and it steers them.
        assert!(third.contains("An agent reports to you in this pane"), "{h:?}: {third}");
        assert!(third.contains("farcooler message <key>"), "{h:?}: {third}");
        assert!(third.contains("farcooler plan ticket set <key>"), "{h:?}: {third}");
    }
}

/// A dispatched agent is confirmed to have started (ov-493): one that
/// didn't goes to the owner, and a folder-trust prompt is never answered
/// for them.
#[test]
fn a_dispatched_agent_is_confirmed_started() {
    for h in ALL {
        let third = section(&skill(h), "3. Dispatch, answer, or report");
        assert!(third.contains("Confirm each dispatched agent started"), "{h:?}: {third}");
        assert!(third.contains("goes to the owner at once with the reason"), "{h:?}: {third}");
        assert!(third.contains("in Needs You: never answer it for them"), "{h:?}: {third}");
    }
}

/// A reversible call made for the owner is a ruling, recorded where the owner
/// can reverse it, and never left as an open question it didn't need to be.
#[test]
fn a_reversible_call_is_recorded_as_a_ruling() {
    for h in ALL {
        let third = section(&skill(h), "3. Dispatch, answer, or report");
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
        let third = section(&skill(h), "3. Dispatch, answer, or report");
        assert!(third.contains("cite a kept one as precedent"), "{h:?}: {third}");
        assert!(third.contains("`plan ruling reverse R-12 [--sha <commit>]`, note the lesson on its card as a decision, ask next time"), "{h:?}: {third}");
        assert!(third.contains("Never keep one for them."), "{h:?}: {third}");
    }
}

/// Landing ends with the card closed, the ticket landed, the worktree removed
/// and its build output gone; a push or a rerun is watched, so a run nobody
/// looks at doesn't sit green and unlanded; and landings are kept on the plan
/// (ov-309, ov-872), with a page's figures drawn live (ov-306), never a
/// hand-kept page.
#[test]
fn landing_is_watched_and_cleaned_up() {
    for h in ALL {
        let land = section(&skill(h), "4. Land");
        assert!(land.contains("After every push or CI rerun, take its run from `gh run list`"), "{h:?}: {land}");
        // A local bare remote made Sonnet decide there was no CI and skip
        // the watch (ov-323, S18): the run list decides, never the URL.
        assert!(land.contains("never decide there is no CI from the remote's URL"), "{h:?}: {land}");
        assert!(land.contains("start `gh run watch <id> --exit-status`"), "{h:?}: {land}");
        assert!(land.contains("as a background command"), "{h:?}: {land}");
        assert!(land.contains("delete its build output, and `worktree remove` it"), "{h:?}: {land}");
        assert!(land.contains("tick each verified `--met` line"), "{h:?}: {land}");
        // The engine lands; the old landing job and its writes are gone (ov-960).
        assert!(land.contains("**The workflow engine lands.**"), "{h:?}: {land}");
        assert!(land.contains("`workflow queue` puts a ticket waiting at its landing step into the group"), "{h:?}: {land}");
        assert!(land.contains("Landings from before the engine are history"), "{h:?}: {land}");
        for gone in ["landing start", "landing set", "landing move", "--record-only", "collecting landing", "--land`"] {
            assert!(!land.contains(gone), "{h:?} still shows {gone:?}: {land}");
        }
        assert!(land.contains("- Route `services/site/**`: ask"), "{h:?}: {land}");
        assert!(land.contains("`plan ticket set <key> --stage landed --sha <sha>`"), "{h:?}: {land}");
        assert!(!land.contains("page set trains"), "{h:?}: the hand-kept page is gone: {land}");
        assert!(land.contains(r#"`{"ci":"main"}`"#) && land.contains(r#"`{"cards":"in_review"}`"#), "{h:?}: {land}");
        assert!(land.contains("Local gates mirror CI"), "{h:?}: {land}");
        assert!(land.contains("runs every gate once"), "{h:?}: {land}");
    }
}

/// A check-in re-evaluates: the stories, the structure, the ideas the owner
/// would want, and what went wrong. Initiative runs unless the charter
/// turns it off (ov-491).
#[test]
fn a_check_in_re_evaluates_and_shows_initiative() {
    for h in ALL {
        let body = skill(h);
        let check = section(&body, "5. Check in");
        for rule in [
            "re-evaluate rather than replay",
            "`plan theme set <name> --story",
            "reset Next Up",
            "the structure: do the themes still fit the work?",
            "initiative: unless the charter says `Initiative: off`",
            "one line in a lessons file",
            "Name each permission prompt",
        ] {
            assert!(check.contains(rule), "{h:?} lacks {rule:?}: {check}");
        }
        let interview = section(&body, "The interview");
        assert!(interview.contains("Initiative: on (the default)"), "{h:?}: {interview}");
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
    let writes = plan_and_page_writes(&text);
    // Five since the landing writes left the skill with the old landing job (ov-960).
    assert!(writes.len() >= 5, "too few plan writes to check: {writes:?}");
    assert!(!text.contains("plan lane") && !text.contains("plan train"), "a retired verb is still shown");
    for line in writes {
        assert!(line.contains("--actor manager"), "a write that doesn't name the manager: {line}");
    }
}

fn plan_and_page_writes(text: &str) -> Vec<&str> {
    text
        .lines()
        .filter(|l| {
            // `landing` only reads now (ov-960), so no landing command is a write.
            ["farcooler plan ", "farcooler agent ", "farcooler message ", "farcooler page set "]
                .iter()
                .any(|w| l.starts_with(w))
                // `plan --repo <repo>` reads the plan; it writes nothing.
                && !l.starts_with("farcooler plan --")
        })
        .collect()
}

#[test]
fn landing_reads_are_exempt_but_untagged_writes_are_still_checked() {
    let commands = "farcooler landing show 1 --repo overnight\n\
        farcooler landing list --repo overnight\n\
        farcooler plan --repo overnight\n\
        farcooler plan ticket set ov-1 --stage landing\n\
        farcooler agent stop ov-1\n\
        farcooler message ov-1 hello\n\
        farcooler page set status --body hello";
    let writes = plan_and_page_writes(commands);
    assert_eq!(writes.len(), 4, "{writes:?}");
    assert!(writes.iter().all(|line| !line.contains("--actor manager")));
    assert!(writes.iter().all(|line| !line.starts_with("farcooler landing ")));
}

/// The owner's conventions ship as defaults a charter overrides (ov-463):
/// landings are numbered "Landing N", agents and landings carry titles for
/// people, and a ticket that isn't ready leaves its landing group.
/// S21 to S23 in `scripts/manager-skill-pressure` score the first three from
/// what an agent ran; this holds the words.
#[test]
fn the_owners_conventions_ship_as_defaults() {
    for h in ALL {
        let body = skill(h);
        let first = section(&body, "1. Read the charter");
        assert!(first.contains("That includes the Defaults at the end: each holds until the charter says otherwise."), "{h:?}: {first}");
        let defaults = section(&body, "Defaults");
        for rule in [
            "a line under the heading it concerns, or under `## Anything else`, wins",
            "A landing is numbered by the runner, and \"Landing N\" when you speak of it",
            "Every agent and landing gets a title that says what it does",
            "a ticket's branch starts with its key",
            "Cards are `<Area>: <outcome>`",
            "Each `--option` of an ask is a short name",
            "Ready work never waits for a ticket still building or in a fix",
            "To let a landing go without a ticket that isn't ready, take it out of the group: `farcooler workflow leave`",
            "Chain them and the push with `&&`, never `;`",
            "judged by exit codes, never by grepping output",
            "A test nobody has seen fail proves nothing",
            "Make a reversible call yourself as a ruling",
            "On unless the charter says `Initiative: off`",
            "Rank work by the themes' order",
            "one or two high-confidence next steps as cards labeled `initiative`",
            "each a `plan ruling add` the person who asked can reverse; suggest the rest with options",
            "Never ahead of anyone's own asks, unless it unblocks them",
            "No merge commits: rebase, or rebase and squash",
            "noted waived (why) or split (to which card)",
        ] {
            assert!(defaults.contains(rule), "{h:?} lacks {rule:?}: {defaults}");
        }
        // The repository's own area list is not a default (ov-463).
        assert!(!body.contains("Phones, Watch, Daemon, Relay"), "{h:?}: Far Cooler's areas leaked into the skill");
        let third = section(&body, "3. Dispatch");
        assert!(third.contains("--branch <branch> --title \"<what it does>\" --actor manager"), "{h:?}: {third}");
        let land = section(&body, "4. Land");
        assert!(land.contains("joins that step's open group, which leaves for its gate by itself when nothing is ahead"), "{h:?}: {land}");
        assert!(!body.contains("integ-N>"), "{h:?}: a command still names integ-N");
    }
}

/// The first run (ov-519): a charter Far Cooler drafted from the repository
/// sits beside the charter as `charter-draft.md`, and the orchestrator goes
/// through it with the owner instead of interviewing from nothing. The
/// interview stays for a workspace with no draft. Either way the charter is
/// written only once the owner says yes, and the draft goes once it is.
#[test]
fn a_drafted_charter_is_gone_through_and_the_interview_stays_the_fallback() {
    let draft = crate::charter_draft::DRAFT_FILE;
    for h in ALL {
        let interview = section(&skill(h), "The interview");
        let asks = interview.find("One question at a time").unwrap_or_else(|| panic!("{h:?}: {interview}"));
        let at = interview.find(&format!("With a `{draft}` beside the charter, don't interview")).unwrap_or_else(|| panic!("{h:?}: {interview}"));
        assert!(asks < at, "{h:?}: {interview}");
        assert!(interview.contains("show the draft, its `<!-- from: … -->` sources, change what they ask"), "{h:?}: {interview}");
        assert!(interview.contains("only after they say yes"), "{h:?}");
        assert!(interview.contains(&format!("then delete `{draft}`")), "{h:?}: {interview}");
    }
}

/// Suggest Themes (ov-519) sends `SUGGEST_THEMES_PROMPT` as the owner's
/// message. The skill answers it in the check-in: 3 to 5 themes from the
/// history with their evidence, the owner keeps some, only those are created,
/// and one ticket is offered, not started.
#[test]
fn suggesting_themes_proposes_with_evidence_and_creates_only_what_is_kept() {
    let prompt = super::SUGGEST_THEMES_PROMPT;
    for words in ["Suggest themes", "3 to 5", "last 90 days", "merged PRs and open issues", "with its evidence", "Ask me which to keep", "offer to start one ticket"] {
        assert!(prompt.contains(words), "the prompt lacks {words:?}: {prompt}");
    }
    for h in ALL {
        let check = section(&skill(h), "5. Check in");
        let at = check.find("**Suggesting themes**, when asked:").unwrap_or_else(|| panic!("{h:?}: {check}"));
        let rule = &check[at..];
        for words in [
            "propose 3 to 5 from the last 90 days of commits, merged PRs and open issues (titles, not bodies)",
            "each an outcome with its evidence",
            "Ask which to keep, `plan theme create` those, offer one ticket",
        ] {
            assert!(rule.contains(words), "{h:?} lacks {words:?}: {rule}");
        }
    }
}

/// A task ask reaches a paired phone as a decision notice
/// (`watch/task_notice.rs`, `push.rs`), so the skill must not say it reaches
/// none: the reply still carries the question for an owner with no phone
/// (review 2a, M3).
#[test]
fn an_ask_reaches_a_paired_phone_and_the_reply_still_carries_it() {
    for h in ALL {
        let third = section(&skill(h), "3. Dispatch");
        assert!(!third.contains("reaches no phone"), "{h:?}: {third}");
        assert!(third.contains("put the question in your reply too: a phone hears it only when one is paired"), "{h:?}: {third}");
    }
}

/// The orchestrator keeps the record it routes by (review 2a, R4): the trim
/// had turned "log each ticket's tokens, model and fix rounds" into routing
/// by a record nothing said to keep.
#[test]
fn tickets_are_logged_and_routed_by_the_record() {
    for h in ALL {
        let third = section(&skill(h), "3. Dispatch");
        assert!(third.contains("Log each ticket's tokens, model and fix rounds; route by that record"), "{h:?}: {third}");
    }
}

/// A dispatched agent reads its brief and never the charter or the Defaults,
/// so the brief rule itself asks for each new test seen failing: the
/// repository's defining failure mode is a check that can't fail (review 2a,
/// M5, after a trim dropped it silently).
#[test]
fn every_brief_wants_each_new_test_seen_failing() {
    for h in ALL {
        let third = section(&skill(h), "3. Dispatch");
        let brief = third.find("A brief names the gates").unwrap_or_else(|| panic!("{h:?}: {third}"));
        assert!(third[brief..].contains("wants each new test seen failing"), "{h:?}: {}", &third[brief..]);
    }
}

/// The orchestrator keeps the theme order itself (ov-759): a sane default,
/// re-ranked as things change, a person's move kept and recorded naming them,
/// ties to the lead, people from `member list`, and no roster asked for. A
/// check-in reads the plan's history first.
#[test]
fn the_orchestrator_ranks_themes_and_records_overrides() {
    for h in ALL {
        let body = skill(h);
        let defaults = section(&body, "Defaults");
        for rule in [
            "**Theme order.** You keep it.",
            "who is waiting, what is blocked, what is urgent",
            "`plan theme move <name> --to <n>`",
            "A person's own move is an override: keep it, and record it with `plan ruling add`, naming them",
            "the workspace's lead decides unless the charter says otherwise",
            "People come from `farcooler member list`, not the charter",
            "A rule you learn about this team becomes a ruling",
        ] {
            assert!(defaults.contains(rule), "{h:?} lacks {rule:?}: {defaults}");
        }
        let check = section(&body, "5. Check in");
        assert!(check.contains("read the plan's history since your last turn"), "{h:?}: {check}");
        let interview = section(&body, "The interview");
        assert!(interview.contains("Ties between people go to the lead") && interview.contains("Don't ask for a roster"), "{h:?}: {interview}");
        assert!(!interview.contains("## Team"), "{h:?}: the owner rejected a roster");
        assert!(!body.contains("the owner is who you answer to"), "{h:?}");
    }
}

/// Landings fold (ov-796): ready tickets join the waiting landing rather than
/// queue behind it, with the reasons to split; a flaky red main is rerun once
/// and a folded ticket that might interact gets a combined gate.
#[test]
fn ready_tickets_fold_into_the_waiting_landing() {
    for h in ALL {
        let body = skill(h);
        let land = section(&body, "4. Land");
        for rule in [
            "Never queue landings: fold every ticket ready while one waits to land into it (one branch off main, one push, one CI watch), and split only for a conflict or a risky ticket",
            "when a known-flaky test did it, rerun the failed job once, file a flaky card, and keep landing on the rerun's result",
        ] {
            assert!(land.contains(rule), "{h:?} lacks {rule:?}: {land}");
        }
        let rebased = "Tickets already gated alone push folded without a combined gate when they can't plausibly interact (shared files, wire tags, migrations, capability lists, generated files, test floors); gate the combined tree where they might";
        assert!(section(&body, "Defaults").contains(rebased), "{h:?} lacks {rebased:?}");
    }
}

/// The plan links an agent to a ticket only when its description (or pane
/// title) starts with the key, so the skill states that as a rule of its
/// own where it tells the orchestrator to dispatch, with the reason (ov-872).
#[test]
fn every_agent_description_and_pane_title_starts_with_its_ticket_key() {
    for h in ALL {
        let body = prose(h);
        let rule = "Every subagent's description and every pane's title starts with its ticket key";
        let at = body.find(rule).unwrap_or_else(|| panic!("{h:?}: the key rule is gone"));
        let dispatch = body.find("To put an agent to work, dispatch it in a role").expect("dispatch paragraph");
        assert!(at < dispatch, "{h:?}: the rule must come before the dispatch text");
        let tail = &body[at..dispatch];
        assert!(tail.contains("(`ov-123: polish the sidebar`)"), "{h:?}: {tail}");
        assert!(tail.contains("how the plan shows which agent works on which ticket"), "{h:?}: {tail}");
    }
}

/// No screen has a Stop button: a person asks the orchestrator, and it stops
/// the agent with `agent stop` (ov-872), says what happened, and stops one
/// the runner can't reach with its own harness's tool.
#[test]
fn asked_to_stop_an_agent_it_uses_agent_stop() {
    for h in ALL {
        let body = prose(h);
        let rule = "When the user asks to stop an agent, use `agent stop <agent|key> --reason \"<why>\"`";
        assert!(body.contains(rule), "{h:?}: the stop rule is gone");
        assert!(body.contains("a subagent it can't reach, stop with your own tool"), "{h:?}");
    }
}
