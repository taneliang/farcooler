# Onboarding and add-device, reworked

Sketch for review, 2026-08-19. Written before any code, at the user's request.

## What is wrong today

Five findings from reading the current flows end to end.

**1. The empty state asks a question the user cannot answer yet.** It shows two
buttons — **Authorize This Device** (prominent) and **Add a Runner** (quiet) —
which encode an ordering constraint: a runner that has never seen this device's
key refuses the first connection. That constraint is real for the manual path
and false for the QR path, where one ceremony does both. So the loud button is
the right one half the time and the wrong one the other half, and nothing on
screen says which half you are in.

**2. Sign-in is three taps deep, invisible, and load-bearing.** It lives in
`SettingsView`'s first `Form` section, behind a gear glyph in the toolbar. The
QR ceremony is gated on it. A first-run user who taps *Authorize This Device* →
*Add This Device With a Code* lands on a screen reading "Sign in to add this
device" whose only button is **Done** — a dead end, with no route to the thing
it names.

**3. "Add" means two unrelated things, across five entry points.**

| | Direction | What it changes |
| --- | --- | --- |
| Add a **runner** | this device → a machine | a local address book entry |
| Add a **device** | a trusted device → a new one | `~/.ssh/authorized_keys` on the runners |

Both are called "add", and they are reached from `HostOnboardingView`,
`AuthorizeView`, `HostEditorView`, `SettingsView`'s Devices section, and the
fleet's runner-switcher menu. On macOS they are two different Settings *tabs*.

**4. On iOS, granting access does not grant access.** `CeremonyStore.picked()`
hardcodes `pending: true` and never calls `client.enroll`, so an iPhone adding
another device shares addresses and host-key pins and writes no key anywhere —
while the confirmation copy on that very screen promises "Far Cooler adds this
device's key to `~/.ssh/authorized_keys` on each selected runner." The comment
explaining why cites a routing gap in `crates/client` that no longer exists
(`ffi.rs` routes `client.enroll`; Android already uses it).

**5. The address a phone is handed is usually the wrong one.**
`RunnerFacts.thisMac()` uses `ProcessInfo.processInfo.hostName`, which is
normally the `.local` mDNS name. Nothing re-resolves it. A phone paired on the
LAN has a permanently broken runner the moment it leaves, and the only repair is
*Edit This Runner…* and typing something better. There is no Bonjour browse
anywhere in the codebase — this is a string, captured once.

## The model

Two verbs, named for what the person wants rather than for what the protocol
does, and never both on one screen without a label:

- **Connect this device** — *this* phone or Mac gains access to runners.
- **Add another device** — a *different* device gains access to mine.

Plus the plumbing verb, kept but demoted: **add a runner by address**.

## One hub, three entry points

Everything add-shaped goes through one screen, `AddView`. It is reached from
exactly three places, and they all land on the same thing:

| Surface | Control | Opens |
| --- | --- | --- |
| iOS empty state | **Set Up Far Cooler** (primary) | Wizard A, directly |
| iOS empty state | *I already have a runner* (quiet) | the hub |
| iOS fleet, runner switcher | **Add…** | the hub |
| iOS Settings | **Add…** | the hub |
| macOS Settings → Devices | **Add…** | the hub, as a sheet |
| macOS sidebar `+` | **Add…** | the hub, as a sheet |
| Android | same three | the hub |

The hub lists the three destinations with one line each. A destination that is
not available yet says why **and offers the fix inline** — a signed-out user
sees a working **Sign In** button on that row, not a disabled row and not a
dead-end screen.

Retired: the Devices *section* of Settings as the only route to granting; the
nested `AuthorizeView` reached from `HostEditorView` without a `runners`
argument, which silently hides the code path.

## Wizard A — Connect This Device

The first-run path. One decision per screen.

1. **How do you want to connect?**
   - *Scan a code from another device* — recommended. Needs an account.
   - *Enter a runner's address* — needs nothing.
2. **Sign in** — only on the code path, only when signed out. A real step with
   the reason on it ("Both devices sign in to the same account so each can prove
   who it is"), not a bounce into Settings.
3. **Either**
   - *code path*: show this device's QR, then **Scan Code** to read the reply.
     Unchanged ceremony underneath.
   - *address path*: **one** screen carrying the address fields *and* this
     device's public key with **Copy Public Key** and the `authorized_keys`
     line. Today these are two screens and the user must go back and forth.
4. **Done** — straight into the fleet.

## Wizard B — Add Another Device

The granting side, and where the reachability problem gets fixed.

1. **Scan** the new device's code.
2. **Confirm** — who it is (fingerprint), and which runners it may use.
3. **How will it reach them?** *(new)* For each granted runner, show the address
   the new device is about to be handed, and say plainly whether it travels:

   | What we resolved | Verdict |
   | --- | --- |
   | `elt-mbp.local`, or an RFC1918 address | **Only on this Wi-Fi** |
   | `box.tail-1234.ts.net` | **Works anywhere** |
   | a public DNS name | **Works anywhere** |

   When Tailscale is installed and the runner is on the tailnet, its MagicDNS
   name is offered and preselected. When it is not, the screen offers **Set Up
   Tailscale** with one sentence on why, **Use This Anyway**, and a field for
   typing a better address. Nothing is silently substituted.
4. **Add** — and actually call `client.enroll` per granted runner, so `pending`
   reports what happened instead of a constant.
5. **Show the code** back to the new device.

Detection, on macOS, extends `RunnerFacts`, which already shells out to
`ssh -G` and reads `known_hosts`: run `tailscale status --json` from the known
install paths and read the runner's DNS name; classify what `ssh -G` returned as
durable or LAN-only. No new dependency, no daemon change, no protocol change —
`RunnerEntry.address` already carries whatever we decide to put in it.

## What this does not change

The ceremony wire format, `crates/client/src/ceremony.rs`, and every security
invariant in it. The QR exchange is fine; this is the UI around it.
