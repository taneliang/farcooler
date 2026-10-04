# Far Cooler

**Run a fleet of coding agents on machines you own, and keep it moving from
wherever you are.**

Far Cooler runs Claude Code, Codex, Cursor and opencode in real terminals on a
*runner*: your Mac, or a Linux box you can SSH to. Each piece of work gets its
own git worktree. An orchestrator agent keeps the task board, dispatches the
agents and asks you when it needs a decision, and everything waiting on you
reaches your Mac, your iPhone, your Apple Watch or your Android phone.

<!-- HERO SCREENSHOT (ov-203): add docs/images/hero.png, captured from a Debug
     build against a scratch daemon and a demo repository, then put this line here:
     ![Far Cooler on the Mac: the navigator with a workspace's tasks and agents, and an agent's terminal beside its changes](docs/images/hero.png) -->

> **Early software.** There's no stable release yet. The Mac app ships as a
> canary build from every push to `main`, and it breaks sometimes.

## Why Far Cooler

- **Real terminals, not a transcript.** Every agent runs its own TUI inside
  tmux on the runner, exactly as if you'd typed the command yourself, with its
  colors and its cursor. When you'd rather read it as a conversation, flip the
  pane into a chat view.
- **Your agents keep working when you leave.** The terminals belong to the
  runner, not the app. Close the laptop, lose signal, restart the app: the
  agents are still there when you come back.
- **It doesn't guess what's alive.** Far Cooler never stores "running." It asks
  tmux every time, and a terminal that died says **Lost** rather than showing a
  status that went stale.
- **One list of what needs you.** Needs You collects, from every runner, the
  agents asking permission (answer with **Allow** or **Deny**), agents blocked
  at their terminal, decisions the orchestrator asked you for, and work ready
  for review. It's the same count on the Mac, the lock screen, the widget and
  the watch, and tapping a notification opens the task or agent it's about.
- **You manage the orchestrator, not the tasks.** Tell the orchestrator what
  you want in plain words. It writes a charter for how work gets done with you,
  puts the work on the board, starts agents in fresh worktrees, and reports
  back from the board.
- **Your machines, over plain SSH.** The apps reach a runner the way you do:
  over SSH, with a key each device generates and keeps. There's no port to open
  on the runner and no second set of credentials.

## What's in the box

| | |
| --- | --- |
| **Mac** | The full workspace: the orchestrator's conversation, the board with its Unread section, every worktree's terminals, each branch's changes a commit at a time, and a read-only file viewer (Files) beside them. Terminals can be named, and a terminal that's listening on a port offers Open in Browser. Back and Forward in the toolbar retrace where you've been, Go to Anything (⌘P) finds the rest, and the Needs You tray stays in reach. It runs this Mac's own runner for you. |
| **iPhone and iPad** | Needs You first, then the boards, agents and terminals of every runner. Notifications you can answer, Home Screen widgets and a Live Activity. When the runner is slow to answer, the app says so in a sentence instead of freezing, and keeps what you typed so you can try again. |
| **Apple Watch** | What needs you, at a glance: allow or deny a permission ask, read what an agent did, and reply by dictation. A complication puts the agent that most needs you on your watch face. |
| **Android** | Needs You, boards, agents and terminals, from the same Rust core as the other apps. A task's screen can start a message to the orchestrator about it, as the iPhone's can. |
| **Command line** | `farcooler` does everything the apps do and more, on this machine or on any runner with `--runner you@box`. |

Every task card says which agent is on it and when it starts, and an
orchestrator's subagents show up as agents of their own. **Ask the
Orchestrator** (⌘K on the Mac, a row on a task's screen on the phones) drafts a
message about the task you're looking at.

A few words carry the whole product. A **workspace** is one line of work in a
repository, with its own **board** of tasks, its own charter and its own
**orchestrator**. A **worktree** is a directory and branch an agent works in.
A **runner** is one `farcoolerd` daemon, for one Unix user on one host.
[`docs/workspaces.md`](docs/workspaces.md) walks through all of it.

## Requirements

- **Mac app:** macOS 26 or later. The canary build is for Apple silicon.
- **iPhone and Apple Watch:** iOS 26 and watchOS 26 or later.
- **Android:** Android 17 (API 37) or later.
- **A Linux runner:** x86_64 or aarch64, with `tmux`, `git` and systemd user
  sessions, reachable over SSH. Everything installs into your home directory;
  no root needed. See [`docs/runners.md`](docs/runners.md).
- **The agents themselves:** install and sign in to `claude`, `codex`,
  `cursor-agent` or `opencode` on the runner. Far Cooler runs them; it doesn't
  replace them.
- **A Far Cooler account** is needed for push notifications and for pairing a
  device by scanning a code. Adding a runner by its address works without one.

## Get started

1. **Install the Mac app.** Download the newest canary from the
   [canary update feed](https://updates.farcooler.com/canary/appcast.xml) (the
   `enclosure` link), or [build it from source](CONTRIBUTING.md#the-mac-app).
   It's signed and notarized, checks for updates once a day, and asks before
   installing one.
2. **Open it.** Far Cooler starts a runner on this Mac, and offers to install
   the command-line tools.
3. **Add a repository** with File ▸ Add Repository… (⇧⌘R). It starts with one
   workspace, called Main.
4. **Start the orchestrator** from the workspace's navigator. The first time,
   it interviews you for the charter: how work gets from idea to landed, what
   done means, who reviews, and what it may decide alone. Then tell it what you
   want done.
5. **Add a Linux runner** (optional) with Add Device or Runner… ▸ Add a Runner
   by Address…, then **Install** under Settings ▸ Runners. The Mac app carries
   the Linux binaries and installs them over SSH. From a terminal,
   `farcooler runner install you@box` does the same.
6. **Add your phone** with Add Device or Runner… ▸ Scan Its Code…, with both
   devices signed in to the same account. Or, on the phone, enter the runner's
   address and add the key it shows to that runner's `~/.ssh/authorized_keys`.

The iPhone and Android apps aren't publicly distributed yet. Until they are,
build them from source; [CONTRIBUTING.md](CONTRIBUTING.md) has the steps.

Canary, preview and stable builds install side by side, each with its own app,
daemon, database and command (`farcooler-canary` for the canary), so trying a
canary never touches the fleet you depend on.
[`docs/releasing.md`](docs/releasing.md) explains the channels.

## From the command line

```sh
farcooler needs-you                          # what's waiting on you, most urgent first
farcooler task list --repo my-app            # a repository's board
farcooler report --since 7d                  # what got done, and how much of you it needed
farcooler --runner you@box worktree list     # any command, on another runner
```

`farcooler --help` lists the rest, from worktrees and terminals to device
enrollment.

## Learn more

- [`docs/workspaces.md`](docs/workspaces.md): workspaces, the orchestrator,
  the board and Needs You, on every surface.
- [`docs/runners.md`](docs/runners.md): installing and troubleshooting a Linux
  runner, and its security posture.
- [`docs/adapters.md`](docs/adapters.md): chat mode, and adding an agent of
  your own.
- [`docs/farcooler-design.md`](docs/farcooler-design.md): the design, and why
  state is derived from tmux rather than stored.

## Contributing

Bug reports and pull requests are welcome. [CONTRIBUTING.md](CONTRIBUTING.md)
covers building every part, running the tests and checks, and the repository's
conventions.

## License

Copyright © 2026 E-Liang Tan. Far Cooler is licensed under the
[MIT License](LICENSE).

The bundled Iosevka Nerd Font Mono files remain licensed under the SIL Open Font
License 1.1; see
[`apps/ios/FarCooler/Fonts/IOSEVKA-LICENSE.md`](apps/ios/FarCooler/Fonts/IOSEVKA-LICENSE.md).
The Android app ships the same files under the same license.
