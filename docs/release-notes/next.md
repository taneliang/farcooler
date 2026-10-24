# Next stable release notes

The notes for the next stable release. Add a line here as each change lands
that people need to know about before they update. The stable release step
pastes this file into the release's notes, then resets it to this header.

- If you roll back to a build from before per-member logins (before database
  migration 0053), every member's panes and rep relaunch on the owner's Claude
  login.
- Member panes need Claude Code 1.0.8 or later. On an older one, a member's pane
  won't start and asks you to update.
- When you remove a member, Far Cooler signs them out and deletes their login
  from the runner. If it can't confirm the sign-out, it tells you, and they
  should also sign out on claude.ai.
- Only Claude panes use a member's own login. Codex and Cursor panes still use
  the owner's logins. If you require members to sign in to their own, those
  panes won't start for them yet.
