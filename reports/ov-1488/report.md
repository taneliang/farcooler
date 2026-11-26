# ov-1488 Mac runner handoff

## Delivery

The standalone canary Mac job builds, Developer ID-signs, and notarizes the
runner binaries independently of the app. It uploads a content-addressed
archive before publishing the signed index. It uses the existing macOS
signing identity, notarization credentials, `CANARY_SPARKLE_KEY`, and
Cloudflare credentials. No new secret is required.

The bootstrap and update commands are in [Runners](../../docs/runners.md#standalone-mac-runner-canary).
The standalone job produces the architecture of its Mac builder, currently
Apple silicon. It follows the latest green main commit even when the app
planner skips a docs-only or web-only change. The app build retains its own
publication rules.

## Evidence

- The package verifier used an isolated home and a public fixture signing key.
  It refused a changed manifest, a signed manifest for another channel, a
  modified archive, and an older build. The intact archive reached activation.
  The test also checked the production trust anchor against the committed canary
  key and exercised the actual OpenSSH signature verifier.
- Rust tests covered refusal under an admitted landing, `--wait` reserving the
  next admission slot, the atomic CLI/daemon version switch, migration link
  ownership, and LaunchAgent configuration.
- On macOS 27.2.1, the signed fixture installed into a clean home as a
  regular user. The CLI and daemon links resolved, and the daemon answered
  as the background LaunchAgent. After unloading it, the Aqua login
  LaunchAgent restored the background job. The fixture unloaded both jobs.
- The recorded handoff used the daemon from the installed Canary app
  (build 6076), the signed fixture feed, real launchd, and real tmux.
  It moved the runner into the standalone job while preserving the
  workspace, worktree, pane PID, install ID, and managed SSH enrollment.
  The fixture used a stand-in for the app's ServiceManagement unregister
  probe to avoid changing the shared user's app registration.
- The same run invoked `farcooler-canary runner update` through the
  installed CLI. It refused while the landing lock was held. With
  `--wait`, it waited for that lock, started a different launchd PID,
  and preserved the original pane and enrollment.
- The focused Rust run passed 33 tests across standalone activation, local
  daemon connections, and SSH stdio transport. The canary planner, wire gate
  self-test, and workflow lint passed locally.
- A no-root install can start at graphical login after a reboot. Operation from
  power-on before any user logs in needs a system-level service. Logout and
  reboot were not exercised on the shared Mac. [Apple launchd documentation](https://developer.apple.com/library/archive/documentation/MacOSX/Conceptual/BPSystemStartup/Chapters/CreatingLaunchdJobs.html)

Production notarization and R2 publication will run on green main. The local
tests used a fixture signing key and a local feed.

## Mac app removal inventory

The CLI exposes many of the underlying runner operations, and the browser has
its own work and enrollment screens. The following Mac workflows still depend
on the app or a native integration.

| Area | What the Mac app provides | Relevant sources |
| --- | --- | --- |
| Local desktop and window state | A direct local runner UI with a restored multi-window/tab layout, per-window sidebar and selection memory, native navigation/history, title status, menus, key bindings and command palette. | `FarCoolerApp`, `Commands`, `WindowSession`, `WindowRestore`, `WindowFrame`, `Shortcuts`, `JumpBarView` |
| Terminal and agent desktop | Embedded native VT input/rendering, clipboard and image paste, tiled terminal groups, saved layouts, screen thumbnails and native transcript and composer interactions alongside terminal panes. | `TerminalRenderView`, `TerminalSurface`, `TerminalInput`, `ImagePaste`, `PaneCanvas`, `TerminalScreens`, `NativeAgent/` |
| Native workspace surfaces | The desktop combination of file tree, diff and LFS notices, embedded worktree web panes, project terminals, Plan and board navigation, task-history and workflow-review views, and workspace/charter forms. Browser surfaces cover parts of these workflows. | `Files/`, `ChangesPane`, `WebPaneViews`, `Plan/`, `TaskBoard`, `WorkflowChangeReviewCard`, `CharterSetupView` |
| Worktree handoff to Mac apps | Finder reveal, local and remote editor handoff including Zed, Terminal integration, configurable editor commands, and native repository/folder pickers. | `Editors`, `OpenInEditor`, `EditorSettings` (in `EditorsSettings.swift`), `RepositoryScan`, `Commands` |
| Mac client identity and SSH routing | Add-runner pairing ceremony, storing device and Tailcat node keys in Keychain, pinning host keys, prompting for direct SSH fallback, and writing managed SSH config for external tools. Standalone preserves the installed CLI link used by existing SSH entries. | `AddFlow`, `Ceremony/`, `Runners`, `SshConfig`, `StableCLI` |
| Access, enrollment and recovery UI | Native teammate and device invitations, QR handoff, runner/team and notification pairing sheets, access controls, and local Touch ID decisions including runner recovery authorization. The CLI has lower-level commands and the browser has its own enrollment flow. | `ContentView+Invite`, `DevicesSettings`, `RunnerPeopleSection`, `RunnerTeamSheet`, `NativeAgent/MacCutoverGrant` |
| Notifications and app lifecycle | macOS Notification Center and APNs registration and actions, deep links to the relevant app window, activation/reachability handling, Sparkle updates of the GUI, service and CLI switches in Settings. | `Notifications`, `Reachability`, `FarCoolerApp`, `UpdateHold`, `Updates`, `ServiceRegistration`, `CommandLineTools` |
| Bundled Linux runner files | The Mac app carries Linux CLI and daemon builds for SSH provisioning. Standalone provides the same CLI installer operation when given the Linux binaries. | `build-app.sh`, `runner_install.rs` |
| Local administration and preferences | Native runner provisioning, settings and upgrade surfaces, per-member Claude sign-in with native browser handoff, Help and log reveal, per-Mac editor/shortcut preferences, native theme editing and visual appearance. | `RunnersSettings`, `RunnerUpgradeView`, `RunnerSettings`, `MemberLoginsSection`, `ClaudeSignInSheet`, `DaemonLog`, `Preferences`, `ThemeEditor` |

Old panes and agent hook files can still name binaries inside the app bundle.
Keep that bundle on disk until those sessions finish or their references have
been checked. Canary migration is limited to the current canary bundle
identifier; stable and preview runners remain on their own channels.
