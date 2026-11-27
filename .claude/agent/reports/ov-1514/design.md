# ov-1514: reconnecting the web app without a passkey touch

Security design and threat model only. No product code. Written Nov 27, against `main` at 29c41d4de. It builds on section 4 of `reports/ov-1513/diagnosis.md`.

## The decision in one paragraph

**Recommended: design B, a quiet key.** After a passkey touch, the page makes a WebCrypto Ed25519 key it cannot export. The runner then adds a second, restricted line for that key next to the browser's passkey line, and the line expires on its own.

- The key works for **12 hours after the touch**. It also dies after **2 hours with no connection** made with it. A new key always needs a new touch.
- The key is gone when you **sign out**, **revoke the device**, or **press End** on its row in the device list. Signing in with the passkey again replaces it.
- **What you would be accepting:** for up to 12 hours after your last touch, someone who has your unlocked laptop, or a copy of your Chrome profile folder, can open Far Cooler on that runner without touching anything. Far Cooler would let them do what the browser can do, terminals included, and a terminal is a shell on your runner.
- **What they still couldn't do:** anything that needs a step-up, such as enrolling a device, adding a person or raising access. Those still ask for the passkey with your fingerprint or PIN.

Section 5 sets this out in full for you to accept or refuse.

## 1. What a reconnect costs today, and why

- **Every login is a passkey touch.** The browser's SSH key is the passkey. The runner holds it as an `sk-ecdsa-sha2-nistp256@openssh.com` line whose application is the app host (`crates/fence/src/passkey.rs`). Each SSH login asks WebAuthn to sign that connection's session id (`crates/web/src/passkey.rs`, `PasskeySigner`). WebAuthn requires user presence on every assertion, and the page checks for it (`signature_blob_for`, the `USER_PRESENT` flag). Safari shows the prompt only inside a click (`services/web/src/client/reconnect.ts:9-14`).
- **A dead SSH connection cannot be resumed.** A new connection has a new session id, so it needs a new signature, and so a new touch.
- **The tunnel itself needs no touch.** The page already reconnects the tunnel by itself (`Reconnector`, "dial ahead"). The tunnel uses the device's node key, which is 32 raw bytes kept in IndexedDB (`device.ts`, `nodePriv`). That key grants a route to the runner, never a login.
- **ov-1513 removes most of the touches.** Those were links the page wrongly gave up on. What it cannot remove are the real drops: a laptop that slept, a Wi-Fi change, a tab Chrome discarded, a reload.

What the browser holds today, and what each piece is worth to someone who has it:

| Held in the profile | Kind | Gives on its own |
|---|---|---|
| Node key (`nodePriv`) | Raw bytes in IndexedDB | A route to the runner's tunnel; no login |
| Passkey credential id and public key | IndexedDB | Nothing without the authenticator |
| The passkey's private key | In the authenticator (iCloud Keychain, Google Password Manager, a security key) | A login, but only with a touch: Touch ID, the Mac password, or a security-key tap |
| Granted runners and pinned host keys | IndexedDB | Nothing secret |
| A live SSH connection, while a tab is open | Page memory | Everything the browser's scope allows, with no further touch |

The last row matters for the comparison: **an open, connected tab is already a session that needs no touch.** The question is how long that state may last without a tab holding it.

## 2. What attacks this design has to face

1. **The unlocked laptop.** Someone sits at the owner's logged-in Mac with Chrome open, but no Far Cooler tab is connected.
2. **A copy of the profile.** Malware, a backup, or a stolen disk image copies the Chrome profile folder to another machine. Copying the folder does not need the laptop to be unlocked if the disk is not encrypted. It does if FileVault is on.
3. **Script running in the page's origin.** This could be an injected script (XSS), a compromised dependency in the bundle, or a browser extension allowed on `app.farcooler.com`. Today the CSP (`services/web/src/headers.ts`) allows only `'self'` scripts and `'wasm-unsafe-eval'`, requires Trusted Types, sets `frame-ancestors 'none'`, and limits `connect-src` to the relay and DERP. That makes injection hard. It does not stop a malicious extension.
4. **A network attacker.** Out of scope: the host key is pinned (`crates/web/src/ssh.rs`) and the tunnel is WireGuard, as today.
5. **The runner's own files.** Out of scope: anything running as the runner's Unix user can already write `authorized_keys`.

## 3. Designs compared

### A. No new credential: drop less, and keep sessions across reloads (baseline)

- **What it is:** the ov-1513 fixes, plus the SSH session moved into a SharedWorker so a reload or a second tab reuses it. A dead tunnel still needs a touch.
- **The unlocked laptop:** as today. A touch, with Touch ID or the Mac password on a platform passkey.
- **A copied profile:** as today. A route, no login.
- **Page script:** as today. It can use a live connection while one is open. To get a new one it has to bring up a passkey prompt that the person would see.
- **The window:** none.
- **Revoking:** nothing new.
- **What it fixes:** false drops and reloads. A laptop waking from sleep still asks for a touch. **This does not meet the owner's ask on its own.**

### B. A quiet key: a short-lived, non-extractable browser key on its own line (recommended)

How it works:
- **One touch mints the key.** Right after a passkey login, the page calls `crypto.subtle.generateKey({name: 'Ed25519'}, false, ['sign'])` and keeps the `CryptoKey` in IndexedDB, beside the device record. Over the connection the passkey has just opened, it calls a new `client.quiet_key.set` with the public key.
- **The runner writes one more line in the fence**, tied to the same device:

  ```
  restrict,expiry-time="20261127T2141Z",command="~/.local/bin/farcoolerd-canary --stdio --client farcooler-b3aae7d8d0ba --scope control --node-key K --quiet-until 1764279660" ssh-ed25519 AAAA… Chrome on Mac (quiet)
  ```

  - **sshd refuses the line by itself** once `expiry-time` has passed. That needs OpenSSH 8.2 or newer; this runner has 10.5. The time is the runner's clock, written in UTC with a `Z`.
  - **The forced command repeats the end time** (`--quiet-until`), so the daemon has its own copy. It refuses a login after that time, and ends any session still open at that time.
  - **The client id, scope and node key are the passkey line's**, copied from it. A quiet key never gives more than the passkey line it came from.
- **A reconnect signs with the quiet key**, through the same `russh::Signer` path, with no prompt. The page offers the quiet key first and falls back to the passkey (one click and one touch) if the runner refuses it.

The rules:
- **One quiet key per device per runner.** A new one replaces the old one under one fence lock.
- **Only a passkey login can mint one, and only just after the touch.** `client.quiet_key.set` is refused on a connection that came in on a quiet line. It is also refused more than 60 s after the passkey login of the connection asking. So the 12 hours always count from a touch, and a quiet key can never renew itself.
- **Never a step-up.** Step-up for a browser is a fresh WebAuthn assertion with user verification (`crates/daemon/src/step_up.rs`, `ALG_WEBAUTHN`). A quiet key is never registered as a step-up key, and a connection that came in on a quiet line is refused `client.set_step_up_key`.
- **Not a key `client.rekey` can move.** `holds_ed25519` would accept the line, so the quiet line has to be excluded by name. Otherwise a rekey could swap a 12-hour key for one that never expires.

Threat model:
- **The unlocked laptop:** a login without a touch until the key expires, then a touch as today.
- **A copied profile:** a login from another machine until the key expires. This is the biggest new exposure. Chrome writes a non-extractable key's private bytes into the profile's IndexedDB files. "Non-extractable" stops page script from exporting the key; it does not stop anyone reading the disk. The node key, which they also need for a route, is in the same files.
- **Page script:**
  - It can log in silently, where today it would have to show a prompt.
  - It cannot export the key. It can, though, relay signatures: copy out the node key (raw bytes), dial the runner from its own machine, and have the page sign that SSH handshake. That gives it a session the owner's tab knows nothing about.
  - The daemon ends that session at `--quiet-until`, so its reach is the same 12 hours.
  - The CSP's `connect-src` makes the round trip this needs harder for injected script. An extension is not bound by it.
- **The window:** at most 12 h from the touch. It ends sooner after 2 h with no connection, and at sign-out, revoke or End.
- **Revoking:** removing a line, which the fence already does. Section 4 has the rules.
- **Cost:** the moderate amount of work in section 7. It reuses the fence, the forced command, `mark_revoked` and the real-sshd tests.

### C. A runner CA and short SSH user certificates

How it works:
- At enrollment the runner writes one `cert-authority,restrict,principals="<client id>",command="…"` line per browser device, for a CA key the daemon holds.
- After a touch, the daemon signs the page's WebCrypto public key into a certificate with `valid_before` set 12 h ahead. sshd checks the certificate's dates itself.

Threat model:
- **The unlocked laptop, a copied profile, page script:** the same as B. The secret in the browser is the same kind of key, kept the same way.
- **The runner's files:** a CA private key on disk can mint a login for any browser device. No worse than today, because the same account can already write `authorized_keys`, but it is one more secret to guard.

Revoking:
- sshd revokes a single certificate only through `RevokedKeys` in `sshd_config`, which belongs to root and which Far Cooler does not edit.
- Without `ExposeAuthInfo` (also root) the forced command cannot tell which certificate logged in.
- **So one quiet login cannot be ended without ending them all.** The only way is to remove the CA line, which ends every certificate for that device, and then write a new CA.
- **Cost:** more than B: CA keys, certificate minting, CA rotation. The only gain is not writing `authorized_keys` on each mint, and B writes it once per touch, which is no load at all.

### D. A resume token the daemon checks after SSH

- **What it is:** a random secret, given out after a passkey login, that a later connection presents to skip the passkey. It still needs some SSH authentication to reach the daemon at all, so one more line in `authorized_keys` would admit anyone with the node key, and the token would be what protects the daemon.
- **Threat model:** worse than B on every count. The token is a bearer secret, so page script can read it out and use it anywhere without the page's help. There is no relay step to go through.
- **Not recommended.** It is B without the non-extractable key and with a new login path in the daemon.

### E. A separate origin that holds the key and runs the SSH client

- **What it is:** a key-holding page on a different registrable domain (not under the app host, which `passkey.rs` forbids), framed by the app. It runs the tunnel and SSH itself and passes only RPCs to the app page.
- **Threat model:** script in the app origin can make RPCs only while the frame cooperates. It cannot sign an outside handshake, so the relay attack in B is closed. The unlocked laptop and the copied profile are the same as B.
- **Not now.** It means moving the wasm client, the tunnel and the passkey's RP ID to a second origin. That is a re-architecture for one attack class that B already bounds at 12 hours. Worth looking at again if extensions become the main worry.

### Summary

| | Touch on wake | Unlocked laptop | Copied profile | Page script | Revoke one | Cost |
|---|---|---|---|---|---|---|
| A | Yes | Touch | Route only | Live tab only | n/a | S |
| **B** | **No, for 12 h** | **Login, 12 h** | **Login, 12 h** | **Silent login and relay, 12 h** | **Yes, a line** | **M+** |
| C | No, for 12 h | Login, 12 h | Login, 12 h | Same as B | Only all at once | L |
| D | No | Login | Login, anywhere | Token theft, no relay needed | Yes | M |
| E | No, for 12 h | Login, 12 h | Login, 12 h | No relay | Yes | XL |

## 4. Recommendation: B, with these numbers and rules

**Window length:**
- **Absolute: 12 hours from the passkey touch.** That covers a working day with lunch, meetings and Wi-Fi changes. A laptop that sleeps overnight asks for one touch in the morning.
- **Idle: 2 hours.** If no connection has been made with the quiet key, or none has been open, for 2 hours, the daemon deletes the line. A laptop left at a café over lunch is still covered; one left closed all afternoon is not. The daemon tracks this from the sessions it sees. The idle limit is the daemon's alone: sshd's `expiry-time` carries only the 12-hour end.
- **Never extended without a touch.** Each touch mints a new key with a new 12 hours. Using the key does not push back its end.

**Revocation rules:**
1. **Expiry.** sshd refuses the line after `expiry-time`. The daemon refuses after `--quiet-until`, ends open quiet sessions at that time, and deletes expired lines when it starts and every minute after.
2. **Idle.** The daemon deletes the line after 2 h with no quiet session open.
3. **Sign out on the page.** First, over the live connection, `client.quiet_key.end` for every runner connected. Then the page deletes the key with the rest of IndexedDB (Clear-Site-Data already covers that). If a runner can't be reached, the page says that runner still accepts the key until its end time and names the time. With the local copy deleted, only a copy taken earlier could still use it.
4. **Revoke the device.** `client.revoke` removes the device's quiet line with its passkey line, under the same fence lock, and `mark_revoked` ends its sessions. Removing a member covers their browser devices the same way.
5. **End, in the device list.** A control that removes only the quiet line and ends its sessions, and keeps the passkey line. On the web and Mac device lists. It needs the same rights as revoking that device.
6. **A new passkey login replaces it.** One quiet key per device per runner.
7. **The runner's clock decides.** The browser's clock is never asked.

**How it shows in the device list:** no new device. The browser's row gets a second line, worded the Apple way:

> Chrome on Mac · Signs in without a touch until 9:41 PM · **End**

`client.list` reports `quiet_until` (ms) on the device's entry, read from the quiet line. The row disappears when the line goes. In the page's own settings, the same line appears under "This browser".

**Key type:** Ed25519 in WebCrypto, which current Chrome, Safari and Firefox all ship. The fence already admits Ed25519 on a Key A line, so the only new parser work is the option set. If the build lane finds an engine the owner uses without it, P-256 (`ecdsa-sha2-nistp256`) is the fallback, and the fence would need to admit plain ECDSA on quiet lines only.

**Turned on per browser:** the first touch after the build ships shows one sheet: "Stay signed in on this Mac for up to 12 hours? Anyone using this browser in that time can open your runners without your fingerprint." Allow / Not Now, remembered per device. Not Now keeps today's touch on every reconnect.

## 5. What someone with your unlocked browser profile gains, compared with today

For you to accept or refuse:

- **Today:** someone at your unlocked laptop, with no Far Cooler tab connected, needs your fingerprint or your Mac password (or a tap on your security key) to open your runners. Someone with a copy of your Chrome profile folder gets a route to the runner and no login.
- **With this design:** for up to 12 hours after your last passkey touch, and only while the browser has connected within the last 2 hours:
  - Someone at your unlocked laptop opens Far Cooler on your runners with **no fingerprint and no password**.
  - Someone with **a copy of your Chrome profile folder** can do the same **from their own computer**, until the same end time. That copy can be taken by malware on the Mac, or from the disk if FileVault is off.
  - A **malicious browser extension** allowed on the Far Cooler site can open a session in the background, without your page open, until the same end time.
- **What they get once in:** everything the browser's access allows. On the owner's devices that is `control`: reading and steering agents, the plan and the board, **and typing into terminals, which is a shell on your runner as your user**.
- **What they still can't do:** enroll a device, add or remove people, change anyone's access, or make the key last longer. Each of those still needs your passkey with your fingerprint or PIN.
- **How you stop it:** Sign Out in that browser, End on its row in the device list on any device you're signed in on, or revoke the device. The longest it can ever last is 12 hours.

If that trade is refused, A remains: ov-1513's fixes plus a SharedWorker, which makes reloads and false drops quiet but keeps one touch after every sleep.

## 6. Open questions for the build lane, not the owner

- **Measure what Chrome writes to disk.** Generate a non-extractable key, then look for its PKCS#8 bytes in the profile's `IndexedDB/https_app.canary.farcooler.com_0.indexeddb.leveldb`. This design assumes they are there. If Chrome turns out to encrypt them with the Keychain, the copied-profile risk drops to the unlocked-laptop risk, and the owner should be told.
- **Safari's IndexedDB keeps a `CryptoKey`** across a restart. Check it on the owner's Safari version.
- **Which connection is the quiet one.** `host.get` reports `caller_key_fingerprint`, and with two lines per client id it must report the quiet line's when that is how the caller came in. The forced command's `--quiet-until` is what tells the daemon.

## 7. The build ticket

**Title:** `Web: reconnect without a touch for 12 hours` (area Web; 43 characters).

**Size: M+, about four days of lane work, in three slices that land in order.**

1. **Fence and daemon (M).**
   - A third line shape in `crates/fence`: `render_quiet`, the parser, and `key_a_options` with `expiry-time` and `--quiet-until`.
   - `client.quiet_key.set` and `.end`; refusal of quiet callers on `set`, `set_step_up_key` and `rekey`.
   - Ending sessions at the deadline, the sweep for expired and idle lines, `quiet_until` in `client.list`.
   - Revoke and member removal take the quiet line too.
2. **Web client (S+).**
   - Make and keep the WebCrypto key, offer it first in `ssh::connect`, fall back to the passkey.
   - `Reconnector` connects without a click when a quiet key is held.
   - The opt-in sheet, `.end` at sign-out, and the "until" line in settings.
3. **Device lists (S).**
   - The "Signs in without a touch until …" line and End, on the web device list and the Mac's.
   - Shared words in `crates/client` testdata, so every surface uses the same text.

**Tests that prove each property.** Each must be seen to fail with the guard removed (see the "checks that cannot fail" rule). Real-sshd tests use `crates/daemon/tests/real_sshd`.

| Property | Test | How it fails without the guard |
|---|---|---|
| sshd refuses an expired line by itself | Real sshd. Write a quiet line with `expiry-time` one minute in the past; the quiet key's login is refused and the passkey line's still succeeds | Drop `expiry-time` from `key_a_options`, and the login succeeds |
| The daemon refuses after `--quiet-until` even if sshd admitted it | Daemon. Start a session as a quiet line whose `--quiet-until` has passed on a fake clock; the handshake is refused | Remove the check, and the session opens |
| An open quiet session ends at its deadline | Daemon, fake clock. Open a quiet session, move the clock past the deadline; the connection closes within one sweep | Remove the deadline from the session loop, and it stays open |
| Idle for 2 h removes the line | Daemon, fake clock. Mint, open no session for 2 h plus one sweep; the line is gone from `authorized_keys` | Remove the idle sweep, and the line stays |
| Only a fresh passkey login can mint | Daemon. `set` over a quiet session is refused; `set` 61 s after a passkey login is refused; `set` 5 s after one succeeds | Remove each check in turn, and the matching call succeeds |
| A quiet key can never extend itself | Daemon. Use the quiet key repeatedly for 11 h on a fake clock; `--quiet-until` and `expiry-time` are unchanged | Re-mint on use, and the deadline moves |
| One quiet key per device | Fence. Mint twice; one quiet line for that client id, holding the second key | Append instead of replace, and there are two lines |
| Never more than the passkey line | Fence. A quiet line's scope, client id and node key equal its passkey line's; asking for `host_admin` is refused | Take the scope from the request, and it renders |
| Never a step-up key | Daemon. `client.set_step_up_key` from a quiet session is refused; a step-up signed by the quiet key is refused by both verifiers | Remove the refusal, and the key is proposed |
| `rekey` never moves a quiet line | Daemon. `client.rekey` naming the quiet line's fingerprint is refused and the file is unchanged | Let `holds_ed25519` decide alone, and the line is replaced without `expiry-time` |
| Revoke takes the quiet line too | Real sshd. Revoke the device; both lines are gone, and both a quiet login and a passkey login are refused | Leave the quiet shape out of the inventory: the line is read as foreign and survives the revoke, which is the dangerous default |
| End removes only the quiet line | Daemon. `client.quiet_key.end`; the quiet line is gone, the passkey line stays, and the quiet session closes | Remove both, and the passkey login fails |
| Sign-out ends it on the runner | Web unit test (`sign-out.test.ts`) with fake deps: `.end` is called for each connected runner before the store is forgotten; an unreachable runner shows its end time | Skip the call, and the test sees no `.end` |
| Page script cannot export the key | Browser test: `crypto.subtle.exportKey` on the stored key rejects | Generate with `extractable: true`, and the export succeeds |
| The line is written in the runner's UTC | Fence. Render with a fake clock in a non-UTC zone; `expiry-time` ends in `Z` and matches `--quiet-until` | Format in local time, and the two disagree |
| The device list shows it, and it goes away | `client.list` reports `quiet_until` while the line exists and not after End or expiry; a web view test renders the line and the End button | — |

**Acceptance for that ticket:**
- The owner can sleep the laptop for 30 minutes, wake it, and see the page reconnect with no click and no touch.
- After 12 hours, and after End or Sign Out, a reconnect asks for a touch again.
- Every row of the table above has a test that has been seen to fail.
