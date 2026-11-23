# ov-1430 runner freeze and Developer Tools permission

During the Nov 23 incident, eight daemon threads were in `__sysctl`, and process-argument reads hung on about twenty processes that had yet to finish exec. The process table populated foreground arguments even for caller attribution and workflow recovery, which only need ancestry, groups, and terminal IDs. The incident sample implicates Gatekeeper and trust evaluation, although it did not capture a stack from `syspolicyd`.

## Developer Tools measurement

**Result: inconclusive.** Apple documents Developer Tools as allowing an app to run software that does not meet the system security policy. The remaining question is which `syspolicyd` or `trustd` checks still run for tools launched by the bundled daemon, a login shell, or tmux. [Apple Developer Tools settings](https://support.apple.com/en-gb/guide/mac-help/mchlc5fb7f9c/27/mac/27)

On macOS 27.2.1 (26B131), `spctl --status` reported assessments enabled. A scratch `farcoolerd` booted with a SQLite backup of a freshly seeded scratch daemon's database and a private tmux directory. Its tmux launcher timed five first executions at new paths of local copies of Homebrew tmux 3.8 with `-V`. All five completed successfully. Median wall time at the timer's 10 ms precision was 0 ms, with a range of 0–20 ms. These copies shared the installed binary's code identity, so the run could have benefited from a warm security cache. The baseline used the owner’s existing settings. A paired launch trace with comparable cold-cache inputs would establish whether adding the permission to onboarding is warranted.

## Owner verification

1. Use a disposable macOS VM or a separate test Mac with Gatekeeper enabled. Install a scratch copy of the Far Cooler bundle and start its bundled `farcoolerd` with a new runtime directory and a private tmux directory. Seed the scratch store, then take a SQLite backup as the input for each trial.
2. Start with the scratch app and daemon's Developer Tools permission off. From a scratch workflow, launch a trusted, freshly downloaded and quarantined test tool. Record the tool's first output, exit status, elapsed time, and the macOS unified log events for that exact tool from `syspolicyd` and `trustd`. Exercise the same direct and tmux-mediated launch routes used by builders and checks.
3. In System Settings → Privacy & Security → Developer Tools, add the **scratch** app and its bundled daemon if macOS lists the daemon separately. Enable the permission on the test machine and relaunch the scratch app and daemon. Repeat with comparable fresh tool copies and a cold-cache VM snapshot, then compare latency and the presence of launch assessment events. Testing the app and daemon entries independently can identify which identity macOS applies to each launch. Restore the original settings after the trial. [Apple Developer Tools settings](https://support.apple.com/en-gb/guide/mac-help/mchlc5fb7f9c/27/mac/27)
