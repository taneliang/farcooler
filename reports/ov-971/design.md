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

## Fix round: what a person approves is what runs

The independent review (`.claude/agent/reports/ov-971/independent-review.md`) and the manager's ruling found three gaps. How each is closed:

- **The question on the card is the daemon's.** `task.note` refuses a Question or Decision on a draft's card, and any note that supersedes one of its notes. `task.update` refuses to reword the card. No caller may write the `workflow_draft` mark. An answer counts only if it follows the daemon's own latest question, recorded in the draft as `question`.
- **The summary shows everything.** A last part, "In full, exactly as it runs", gives every argv, every environment variable, the reviewer, CI, `gh`, the pull request repository and remote, and the landing mirror with its config digest. Every value is escaped, so a gate can't draw lines or close its code span. `in_full` names every adapter field, and a test walks every value in the adapter's JSON.
- **Approve is never one tap from the headline.** The card has no quick-answer options, so no row, notification or watch shows an Approve button on any client, old or new. The full question ends with ``answer `approve <code>` ``. The code is six characters of the digest of the definition, the adapter and the summary.
  - A row or notification that shows only the headline never shows the code.
  - The card on Mac and iOS shows the question in full, and so does Android's Answer dialog.
  - Any other answer publishes nothing. The daemon asks again, so the card stays in Needs You.
- **The draft is bound to what was asked.** The digest is in the question note in the database. Approving re-digests the draft file and compares, so a file edited after the person read it doesn't publish (M1).
- **Smaller fixes.**
  - Only the owner's answer decides (M4). Approve claims the draft (`publishing`) under the lock before it publishes (M3). The deciding answer is recorded (M2).

Limits:

- Approving takes typing, not one tap. A native "Review and Approve" button that sends the words from the full view is the follow-up.
- The code proves which view the answer came from, not who sent it. The seat guardrail (`caller_seat`) is unchanged, so ov-974 still needs an owner-device signature.
