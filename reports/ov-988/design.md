# ov-988: trust a first key on a runner with no Mac app

From the ov-980 review (M1). Since ov-980, a step-up key becomes trusted in only two ways:

- the Mac app on the runner's own Mac approves it with Touch ID; or
- a device whose key is already trusted approves it.

A Linux runner, or a Mac reached over ssh, has neither. So when an owner there has only a phone, no key can ever be trusted. Today that costs nothing, because `KEYLESS_PHONES_ALLOWED` is true and a keyless phone keeps its admin. Flipping that flag would lock these owners out of remote admin for good.

This note is the design only. No code yet.

## What an agent on the runner can do

The threat is an agent running as the runner's Unix user. Everything below is in scope, because each is something the daemon or the CLI offers:

- call the socket, including after getting past the pane walk (`script`, `tmux new -d`, `ssh localhost`);
- enroll a made-up device and propose a key for it (ov-980 M3), so it can hold a "phone" of its own;
- read and type into any terminal or pane, and read the daemon's log;
- run any `farcooler` command, including `runner install … you@localhost` and `runner claim --account <any>` (ov-497: the claim moves the runner to that account at once, with no notice).

Writing the runner's files directly (`authorized_keys`, the store) is out of scope here. The ov-980 review put that in the same class as B3: same-user writes, which no daemon check can stop.

**What follows:** anything that happens only on the runner can be done by the agent too. A code shown in a runner terminal, a code typed into one, or an ssh session that looks remote are all forgeable. So are both halves of a "compare these two codes" check, because the agent can play the phone (its made-up device) and the person at the terminal at once. Presence has to rest on something that never touches the runner. With only a runner and a phone, there are two candidates: a secret that is only ever off the runner, and the owner's relay account.

## Option 1: a first-key code that is minted off the runner (recommended)

**How it works.**

- `farcooler runner install you@box` creates a random 128-bit code **on the machine that runs the installer**. It shows the code there as a QR code plus 26 base32 characters.
- Over ssh, the installer writes only the code's hash into the runner's home. It writes the hash only if none is there yet; there's no flag to replace it. To replace it, you delete the file by hand, which is the out-of-scope file-write class above.
- On the phone, the owner opens the runner and taps Trust This Phone. They scan or type the code, then confirm with Face ID. Over its own line, the phone sends `client.trust_first_key { code }`, step-up signed by the key it's proposing. The signature proves the phone holds that key.
- The daemon trusts the key and marks it vouched (the owner was there) only when all of these hold:
  - the call came in on a device's line, never the socket;
  - the signature verifies against that device's proposed key;
  - the code matches the hash;
  - no key of the owner's is trusted yet.

  It then deletes the hash, so the code works exactly once. From then on, ov-980's usual route applies: a trusted device approves the next key.

**Why an agent can't do this alone.** The code exists only on the installing machine and on the owner's screen. The runner holds a hash of 128 random bits, so reading it and guessing the code offline is hopeless. A made-up device can propose a key, but it can't produce the code.

**What it doesn't cover:**

- **Runners that already exist.** They have no hash yet. Whoever runs the installer first mints the code, and that could be an agent running `runner install you@localhost`. Two mitigations:
  - the installer refuses to mint when the target's host key is the local machine's own;
  - an owner who finds a hash already there is told, "A first-key code was already made for this runner. If you didn't make it, delete `<path>` and run the installer again."

  Neither is a proof. It stays trust on first use until the owner runs the installer from somewhere else.
- **An install run on the runner itself** (the "build on the box" path) shows the code in a runner terminal, which an agent could read later if that terminal was a tmux pane. The docs should say to install from another machine. Phones can't install (they have no `farcooler` CLI), so an owner with nothing but a phone and a runner needs some other machine just once, to install.
- The code is a bearer secret, like a recovery code: anyone who sees it can trust one key. It works once, and only before any key of the owner's is trusted.

**Cost:**

- a hash file, plus `runner install` writes it;
- one daemon RPC and its step-up gate (`step_up::METHODS`);
- one phone screen on iOS and on Android.

Acceptance 1's test is daemon-only. A made-up device, a walk-passing socket call, a wrong code and a second use are all refused. The owner's phone with the right code is trusted.

## Option 2: the relay vouches, through the owner's account

**How it works.** The runner posts its proposal (device, key code) to the relay. The relay pushes it to the devices of the account that owns the runner in the team directory. The owner approves on a phone with Face ID, signing with that phone's directory key. The relay sends back a statement it has signed, and the runner checks it against a relay key built into the daemon for each channel.

**Why an agent can't do this alone.** It would need the owner's WorkOS session, or an account of its own.

**What it doesn't cover:**

- `runner claim` lets any process running as the Unix user move the runner to any account (ov-497). So an agent with any WorkOS account and one registered device completes the route. Closing that needs the relay to refuse attestation for a while after a claim changes the owner, and to notify the previous owner. That's new relay policy.
- **The relay becomes a root of trust for admin on every headless runner.** Today its README promises that a stolen daemon token is "worth exactly one thing … the ability to notify its own owner's phone", and that the relay never sees anything that matters. This option breaks that promise: a compromised relay, or a compromised relay signing key, trusts any key on any runner.
- It depends on the relay being up, and on `DEVICE_KEYS_ENFORCED` (R-72) being on, so that the directory's device keys mean something.

**Cost:** relay routes and a signing key per environment, a pinned key in the daemon, push and approval UI on both phones, and the claim cool-off. It's the largest of the three by far.

## Option 3: a waiting period with a veto

**How it works.** On a runner with no trusted key of the owner's, a proposed key becomes trusted 24 to 72 hours after the phone confirms it with Face ID. During that time, every owner device and the relay push say "A new admin key (code 123 456) will be trusted on <runner> at <time>", with Deny.

**Why it isn't enough.** An agent completes it alone by waiting. It meets acceptance 1 only if the owner reads every notice. That's useful as a tripwire alongside option 1's trust-on-first-use case, but it isn't a presence route on its own.

## Recommendation

**Option 1.** It's the only one that adds no new root of trust: the relay stays a notifier, and the code works once and only for the first key. It also fits inside the daemon and the installer, and the acceptance test is daemon-only.

Its gap is runners installed before it. It's honest about that gap: the hash file and the "already made" message keep it visible, and the owner closes it by running the installer once from another machine. Option 3 could later cover that gap as a tripwire, as a separate card if wanted.

**Acceptance 2 (doc comment on `KEYLESS_PHONES_ALLOWED`).** Whichever option is built, the doc comment says:

- flipping the flag ends admin for a keyless phone everywhere;
- on a runner with no Mac app, a phone gets admin back only through the first-key code (option 1), or through a device that's already trusted;
- so the flag shouldn't flip before the phones that can enter a code have shipped.

The flag stays true, and flipping it is the owner's call.

## Not doing

- Trusting a key on the walk, or on any code shown or typed on the runner. Both are forgeable by an agent, as shown above.
- Treating an ssh session as remote by its `SSH_CONNECTION` address. An agent can ssh to the box's own public address.

## As built (option 1, the orchestrator's ruling)

- **The wire is unchanged.** The phone sends `client.approve_step_up` naming itself, with the first-key code, signed by the key it proposed. `step_up::admit` lets only that request (a device approving its own id) verify against a proposal. `first_key::trust` decides the rest:
  - the device has a line here, and is the owner's;
  - no key of the owner's (`local:app` included) is trusted yet;
  - the proposal is fresh;
  - the code matches the kept hash.
  
  The code is spent by a rename to `first-key-code.used` before the key is trusted. The key is vouched for (`first-key-code`).
- **The code is 25 Crockford base32 characters (125 bits), in five groups.** It's printed as text; a QR code would need a new crate in the CLI. Phones accept any case, spaces and dashes, and read O, I and L as 0, 1 and 1.
- **The runner keeps the hash with `farcooler runner keep-first-key <hash>` (hidden).** It's created with `create_new` and is never replaced, and it answers `kept`, `waiting` or `used` with the path.
- **When `runner install` makes no code:**
  - when the runner's ed25519 host key is one of this computer's ("No first-key code was made: <runner> is this computer…");
  - when either side's keys can't be read;
  - when a code is waiting already: it says "A first-key code was already made for <runner>…", naming the file to delete.
- **Phones:** "Enter First-Key Code" under the six digits (iOS and Android). It opens a field and Allow With Code, and refusals have sentences (`first_key_wrong`, `first_key_taken`).
- **Tests:**
  - daemon: `a_first_key_code_trusts_the_owners_phone_once` and `with_no_first_key_code_a_phone_is_refused`. Each of seven guards was mutated and went red;
  - CLI: `no_first_key_code_is_made_for_this_computer` (red with the check off) and `the_runners_answer_is_read_and_said_in_words`;
  - phones: the iOS UI test `testAFirstKeyCodeAllowsThisPhone`, plus the Android controller and view tests and the `people-first-key` capture.
