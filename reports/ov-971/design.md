# ov-971: the orchestrator edits the workflow, a person approves

## Checked against the brief

- Drafting already exists (ov-959): `workspace landing-template` builds a definition from a template and the runner's adapter. What's missing is everything after: nothing stores a draft, nothing shows a person a summary, and publishing (`workspace.set_workflow`) is person-only with no way for a person to say yes in one tap.
- `Publish` is refused to an agent seat in `Admission::permits` (`person_only`). That stays. The orchestrator never gets a publish path; it gets a *proposal* path.
- The brief says "make approval reachable through the existing Needs You or decision surface". A new Needs You kind needs new protocol enum, Mac, iOS and Android work. A **decision card** needs none: a task in Needs Decision with a question and options is already a row, with buttons, on all three. So the approval is a card.

## Flow

1. `farcooler workspace workflow-draft propose` (orchestrator or person) sends a template choice plus the requirement in the person's words.
2. The daemon builds the definition from the runner's real adapter (checks, CI, landing target), writes it to `workflow_drafts` with the pin it was drafted against, and renders the **summary itself from the definition**. Prose from the orchestrator is only the "why", shown separately, so a summary can't claim something the definition doesn't do.
3. The daemon opens a card "Workflow: <what>" in Needs Decision. Its question is the summary; options are **Approve** and **Decline**.
4. A person answers. Only an answer by `Actor::User` not said from a pane counts (the same test `answer_actor` already applies). An orchestrator's answer is `Manager` and decides nothing.
   - Approve: the runtime publishes through the existing `Action::Publish` path, as `owner`, with `expected_pin` = the pin at draft time. If the live workflow moved meanwhile the draft is marked out of date and the card says so; nothing publishes.
   - Decline: draft marked declined, card cancelled.
5. A new proposal supersedes the open one (its card is cancelled).
6. `workflow-draft show <id>` and `status` let the orchestrator read state: pending, approved, declined, out of date, replaced.

## The summary (plain US English)

Four labeled short paragraphs: **Runs as you** (the commands the workflow starts), **Checks that must pass**, **How code lands**, **Who approves**. Plus **What changes** against the live workflow when one exists.

## Cuts

- Phones: free through the decision card. Nothing native to file unless the long-summary check shows a problem.
- A change that edits the *current* definition: v1 takes the same template inputs (gates, batch, routes) and shows the diff. Free-form edits of a published definition are filed.
