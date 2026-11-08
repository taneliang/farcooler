# ov-980: step-up keys need presence to be trusted

From the ov-974 review (B1). A process that gets past the walk (`script`, `tmux new -d`, `ssh localhost`) could trust a key for any device id it made up, connect under a preamble it wrote, and be taken for the owner. ov-974 closed this for the cutover grant with a vouch. This card closes it for every step-up-gated action.

## What changes

### 1. A key is trusted only with someone present

There are three ways a key becomes trusted, and each now needs presence:

| Route | Before | After |
|---|---|---|
| `client.set_step_up_key` on the socket, for any id but `local:app` | trusted at once, on the walk | a **proposal**, which an approval must trust like any other |
| `client.approve_step_up` on the socket | trusted on the walk (the CLI's `people approve`) | only the Mac app, by its code signature, with its own `local:app` step-up over this approval: Touch ID or the login password. Anything else on the socket is refused `presence_needed` |
| `client.enroll` carrying a key, from the socket | trusted | a **proposal**. Still trusted when a device stepped up for that enroll with its own trusted key |

These are unchanged:

- A device line's approval: another trusted device of the same person, stepped up for it.
- The Mac app's own `local:app` key: only the app writes it, by code signature, and the key needs Touch ID each time it signs.
- A device proposing its own key on its own line.

So every key trusted from now on traces back to one of two things: the Mac app with Touch ID, or a device already trusted.

### 2. An unknown device gets no session

A session's preamble names its device. sshd's forced command writes it for a phone; anything else on the runner can write one too. A preamble naming a device id the runner has no record of is now refused at connect (`sessions::admit_preamble`), on the socket and in `farcoolerd --stdio` when no daemon is running. So a made-up id is never treated as anyone, the owner least of all.

A device has a record when it has one of these:

- **A line naming it in `authorized_keys`.** That is `--client <id>` in a Key A line's forced command, or `farcooler-shell-<name>.<id>` in a Key B line's comment (`farcooler_fence::names_client`). The match is loose on purpose: a line with options a person added still names its device.
- **A revoked mark.** Its line is gone, but it must still connect to hear its farewell (ov-820).
- **A release.** It must still connect to learn it was released (ov-758).

If the file can't be read for any reason but its absence, nobody is refused.

#### Why not `owner_devices`

The first attempt made `people::person_of_device` require an `owner_devices` row, and that broke an integration test. The access migration binds Key A lines only, and drops any row no Key A line backs. So the owner's Mac on its plain (Key B) line never has a row, and it would have been locked out. Checking for the line at connect covers both kinds of key without touching the migration. It also keeps the 34 store-only callers of `person_of` as they are.

## Existing devices (migration note)

- **Paired phones and Macs keep working.** Each one has a line in the file sshd reads, and the daemon reads that same file. Hand-edited options don't matter.
- **A daemon run with its own `FARCOOLER_HOME`** (a test, a scratch daemon) reads `<home>/ssh/authorized_keys`, not the file sshd reads. A device paired through it has its line there. A test that writes lines straight into sshd's file must point the daemon's at it too. `a_forced_command_still_streams` does that now.
- **Keys trusted before this change stay trusted.** Nothing is re-checked or dropped. A key a walk-passer trusted for a made-up id can't be used: that id gets no session.
- **Remote runners.** The Mac app's Touch ID approval needs the app on the runner's own Mac. On a runner the Mac reaches over ssh, the app still asks through `people approve`:
  - A runner from before this change trusts the key, as it always did.
  - A newer runner refuses it `presence_needed`. The app and the CLI then say to approve with Touch ID on that runner, or from a phone already allowed there.
  - A first phone with no trusted device anywhere stays keyless, and keeps the admin it has: `KEYLESS_PHONES_ALLOWED` is unchanged.
- **A Mac without a Secure Enclave or a login password** can't make the Touch ID key, so its approval on the socket is refused the same way and gets the same sentence.
- **Not covered: the recovery listener** (`rpc/recovery.rs`). It still reads a preamble with no check, because it has no service to look the device up in. It belongs to the ov-984 lane; flagged in the report.

## Tests, each seen red first

- `step_up::tests::a_walk_only_caller_trusts_no_key`:
  - the socket's key, for the owner's iPad and for a made-up id, is a proposal;
  - the socket's approval is refused `presence_needed`;
  - a socket enroll's key is a proposal;
  - the made-up id's own signature is refused `key_untrusted`.

  Three mutations each turn it red: trusting the socket's key, dropping the approval check, and trusting a socket enroll's key.
- `step_up::tests::an_unknown_device_gets_no_session`:
  - a made-up id is refused;
  - the owner's phone (Key A), a Mac on its plain line (Key B), a revoked device and a released one are admitted, and so is the local socket.

  Red with the check off, and red with each of the revoked and release allowances removed.
- `fence::tests::a_line_names_its_client_however_it_was_edited`.
- CLI `refusals_are_said_in_words` and AgentKit `PhoneApprovalTests.words`: `presence_needed` has a sentence.
