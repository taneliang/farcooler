# Ceremony Apps Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Put a camera and a screen on the ceremony core, on iOS, Android and macOS, so onboarding a device is: show a code, scan it, pick runners, confirm, scan the code it shows back.

**Architecture:** Every rule that decides whether a scan is acceptable already lives in `crates/client/src/ceremony.rs` behind the FFI. These apps display QR codes, capture them, and render decisions — they do not make any. Each platform's work is the same four screens, plus two Mac-only pieces: Key B and `~/.ssh/config`.

**Tech Stack:** SwiftUI with AVFoundation and CoreImage (iOS, macOS), Compose with CameraX and ML Kit barcode scanning (Android), LocalAuthentication and BiometricPrompt.

**Spec:** [`docs/superpowers/specs/2026-08-16-device-onboarding-design.md`](../specs/2026-08-16-device-onboarding-design.md) — "The ceremony", "Grants are per runner", "A Mac needs two keys", "What signing out does not do", "Remote Login".

**Depends on:** all four previous plans.

## Global Constraints

- **US English throughout.**
- **Apple copy conventions:** title-case buttons, contractions allowed, "runner" not "host", and **no raw Rust error ever reaches a screen** — the FFI returns a machine-readable code and the app owns the sentence.
- **Every string in this plan is the string to ship.** They were written in the spec and reviewed there; do not paraphrase them.
- **No screen in this flow ever recommends relaxing an sshd setting.** Not `StrictModes`, not permissions, not `MaxAuthTries`. If the cause is unknown, the screen says the cause is unknown.
- **The apps make no security decisions.** Anything resembling validation belongs in `ceremony.rs`; if a rule is missing there, add it there rather than in Swift or Kotlin.
- **iOS project is generated** — run `apps/ios/generate-project.py` after adding files.
- **Android needs JDK 17.** `JAVA_HOME=$(/usr/libexec/java_home -v 17)`.

---

### Task 1: Showing and scanning a code, on iOS

**Files:**
- Create: `apps/ios/FarCooler/Ceremony/CodeImage.swift` — CoreImage QR generation
- Create: `apps/ios/FarCooler/Ceremony/CodeScanner.swift` — AVFoundation capture
- Create: `apps/ios/FarCooler/Ceremony/CeremonyStore.swift` — state machine over the FFI
- Modify: `apps/ios/FarCooler/Info.plist` — `NSCameraUsageDescription`
- Modify: `apps/ios/generate-project.py`

**Interfaces:**
- Produces:
  - `func qrImage(_ payload: String) -> UIImage?`
  - `final class CodeScanner: NSObject, ObservableObject` publishing `@Published var scanned: String?`
  - `final class CeremonyStore: ObservableObject` with `enum Phase { showingOffer, scanning, confirming([RunnerRow]), enrolling, showingManifest, done, refused(Refusal) }`

- [ ] **Step 1: Add the camera usage string**

```xml
<key>NSCameraUsageDescription</key>
<string>Far Cooler uses the camera to scan the code on a device you're adding.</string>
```

That sentence is shown in a system prompt. It says what and why in one line and names nothing else.

- [ ] **Step 2: Write the failing test**

`apps/ios` has no test target today; add one only if the project generator supports it cheaply. Otherwise the honest test is the FFI round-trip already covered in the ceremony-core plan, plus manual verification in Step 6. **Say which you did in the commit message** — a plan that claims a test it did not write is worse than one that names the gap.

- [ ] **Step 3: Generate the code image**

```swift
/// A QR code, drawn large enough to scan off a screen.
///
/// CoreImage renders at one pixel per module, which is a postage stamp on a
/// phone and unscannable across a desk. The transform is not cosmetic.
func qrImage(_ payload: String) -> UIImage? {
    guard let filter = CIFilter(name: "CIQRCodeGenerator") else { return nil }
    filter.setValue(Data(payload.utf8), forKey: "inputMessage")
    // Medium correction: the code is read off a lit screen at close range,
    // not off a printed label, so capacity is worth more than redundancy.
    filter.setValue("M", forKey: "inputCorrectionLevel")
    guard let output = filter.outputImage else { return nil }
    let scaled = output.transformed(by: CGAffineTransform(scaleX: 12, y: 12))
    let context = CIContext()
    guard let cg = context.createCGImage(scaled, from: scaled.extent) else { return nil }
    return UIImage(cgImage: cg)
}
```

- [ ] **Step 4: Capture**

`AVCaptureSession` with `AVCaptureMetadataOutput`, `metadataObjectTypes = [.qr]`. Publish the first payload and stop the session — a scanner that keeps firing is a scanner that scans the second code in the frame.

- [ ] **Step 5: The state machine**

`CeremonyStore` calls the FFI for every decision. On the scanning device: decode the offer, ask the relay whether that fingerprint is on this account, and only then show the confirmation. On the new device: show the offer, then scan for a manifest and hand it to `accept_manifest`.

- [ ] **Step 6: Verify on a real device**

Two devices, or one device plus the Mac app. Confirm a code renders large enough to scan across a desk and that scanning stops after one hit.

- [ ] **Step 7: Commit**

---

### Task 2: The two screens, on iOS

**Files:**
- Create: `apps/ios/FarCooler/Ceremony/AddDeviceView.swift`
- Create: `apps/ios/FarCooler/Ceremony/JoinView.swift`
- Modify: `apps/ios/FarCooler/FarCoolerApp.swift` — `AuthorizeView` gains a route to `JoinView` and keeps the manual path
- Modify: `apps/ios/FarCooler/Settings.swift` — a Devices section

**Interfaces:**
- Consumes: `CeremonyStore`, `qrImage`, `CodeScanner`.

- [ ] **Step 1: Build the confirmation, with the spec's copy verbatim**

```
Add "iPhone 17" to work@example.com?

iPhone 17 will be able to run agents and commands on the runners you pick, as
you. Each enrollment is recorded under work@example.com.

SHA256:t7Xq…9Vd ⌄

☑︎ MacBook Pro · this Mac
☐ build-vm
☐ box · your personal runner

Far Cooler adds this key to ~/.ssh/authorized_keys on each runner you pick, and
changes nothing else. You can add or remove runners later in Settings › Devices.

[ Add Device ]  [ Cancel ]
```

Only the runner being granted from is checked by default. A runner reached through a different account is listed and labeled, not hidden.

- [ ] **Step 2: Gate the tap with LocalAuthentication**

```swift
/// A fingerprint, at the moment of the tap.
///
/// This is what someone standing at an unlocked laptop runs into, and it is the
/// only thing between them and an enrolled device. An account check does not
/// help there: that laptop is signed in, and they would be using its session.
let context = LAContext()
context.localizedReason = "Confirm adding iPhone 17 to your runners"
```

Fall back to the device passcode, never to nothing. A cancelled or failed evaluation enrolls nothing.

- [ ] **Step 3: The account mismatch screen, before the confirmation appears**

```
That device is signed into a different account

Far Cooler can only add devices signed into o.o@elt.sg. Sign in to that account
on the new device, then show its code again.

[ Done ]
```

It must be impossible to reach the runner list with a mismatched code — otherwise the runners are on screen with only a fingerprint between a stranger and them.

- [ ] **Step 4: Keep the manual path reachable**

`AuthorizeView` stays, wording unchanged apart from "runner". It is what works with no trusted device and when every device is lost.

- [ ] **Step 5: Build, run, walk both directions**

```bash
xcodebuild -project apps/ios/FarCooler.xcodeproj -scheme FarCooler -destination 'generic/platform=iOS Simulator' build
```

- [ ] **Step 6: Commit**

---

### Task 3: Android

**Files:**
- Create: `apps/android/app/src/main/java/com/farcooler/ceremony/CodeImage.kt`, `CodeScanner.kt`, `CeremonyStore.kt`
- Create: `apps/android/app/src/main/java/com/farcooler/ui/AddDeviceScreen.kt`, `JoinScreen.kt`
- Modify: `apps/android/app/build.gradle.kts`, `gradle/libs.versions.toml`
- Modify: `apps/android/app/src/main/AndroidManifest.xml` — `android.permission.CAMERA`

**Interfaces:** mirrors Task 1 and 2, same FFI, same copy.

- [ ] **Step 1: Add the dependencies, and say they are a decision**

Android has no system QR scanner. This needs CameraX plus a barcode reader:

```kotlin
implementation(libs.androidx.camera.camera2)
implementation(libs.androidx.camera.lifecycle)
implementation(libs.androidx.camera.view)
implementation(libs.mlkit.barcode.scanning)
```

ML Kit's bundled barcode model adds several megabytes. The alternative is ZXing, which is smaller and worse at low light. **State the choice and the size cost in the commit message** — this repo treats a dependency as a decision.

Generating a QR also needs a library on Android, unlike iOS: ZXing's `core` artifact alone is enough for encoding, without its scanner.

- [ ] **Step 2: Request the permission in context**

Ask when the scan screen opens, never at launch, and explain in the same sentence: *"Far Cooler uses the camera to scan the code on a device you're adding."*

- [ ] **Step 3: The screens, same copy as iOS**

Use `BiometricPrompt` for the confirmation gate, with device credential as the fallback.

- [ ] **Step 4: Build and run**

```bash
cd apps/android && JAVA_HOME=$(/usr/libexec/java_home -v 17) ./gradlew assembleDebug
```

- [ ] **Step 5: Commit**

---

### Task 4: macOS, and Key B

The Mac is the only platform with the second key, the `~/.ssh/config` block, and local enrollment with no SSH at all.

**Files:**
- Create: `apps/macos/Sources/FarCooler/Ceremony/` — the same three files, AppKit flavored
- Create: `apps/macos/Sources/FarCooler/SshConfig.swift`
- Modify: `apps/macos/Sources/FarCooler/RunnersSettings.swift` — a Devices pane

**Interfaces:**
- Produces: `func writeRunnerBlock(_ runners: [RunnerEntry], identity: URL) throws` — the fenced block in `~/.ssh/config`.

- [ ] **Step 1: The Key B choice, defaulted**

```
Add "MacBook Air"?

Far Cooler access — run agents and terminals on the runners you pick.

Shell access — Zed, git and Terminal on that Mac reach them too.
☑︎ New key — farcooler-macbook-air ⌄
☑︎ Add to ~/.ssh/config

☑︎ MacBook Pro · this Mac
☐ box

[ Add Mac ]  [ Cancel ]
```

Generating is the default because it is **independently revocable**, not because it is safer. Choosing an existing key warns that removal takes the access that key has always had.

- [ ] **Step 2: Write the `~/.ssh/config` block, with tests**

Four rules, each its own test in a new `apps/macos/Tests/`:

- `the_block_lands_above_any_include` — `ssh_config` is **first**-match-wins, and `Include ~/.ssh/config.d/*` as a first line otherwise wins every keyword.
- `an_existing_pattern_matching_the_alias_causes_a_suffix_and_a_message` — a runner labeled `github.com` must not take over git.
- `two_runners_on_one_host_get_different_aliases` — `alice@box` and `deploy@box` both want `Host box`.
- `a_hand_written_host_block_is_left_alone`
- `a_damaged_fence_refuses_rather_than_rewrites`
- `every_byte_outside_the_fence_survives`

The block:

```
Host box
  HostName box.tail-1234.ts.net
  User you
  Port 22
  IdentityFile ~/.ssh/farcooler-macbook-air
  IdentitiesOnly yes
```

- [ ] **Step 3: Key A is never in that file**

Assert it in a test. Far Cooler passes Key A with `-i` on the command line — the prerequisites plan made that possible — so **deleting this block cannot break Far Cooler, only Zed.** That property is the reason the split exists; a test is what keeps it true.

- [ ] **Step 4: Hand editors the alias**

`Editors.swift:194` builds `ssh://{host}{path}`. Given the long address, ssh matches no `Host` entry and the block does nothing. Pass the alias.

- [ ] **Step 5: Remote Login**

```
Turn on Remote Login

Your other devices reach this Mac over SSH, and macOS keeps that off until you
allow it.

Open System Settings › General › Sharing and turn on Remote Login.

[ Open Sharing Settings ]
```

Shown when a Mac is added as a runner and on any device that cannot reach it. The button opens `x-apple.systempreferences:com.apple.Sharing-Settings.extension`.

- [ ] **Step 6: Build, run, and check Zed actually works**

```bash
cd apps/macos && swift build
```

Then, on a second Mac enrolled through the ceremony: open a remote worktree in Zed. That is the feature; a green build is not evidence of it.

- [ ] **Step 7: Commit**

---

### Task 5: Devices, sign-out, and the honest removals

**Files:**
- Modify: Settings on all three platforms
- Modify: `apps/shared/AgentKit/Sources/AgentKit/Account.swift` — sign-out gains a second action

**Interfaces:**
- Consumes: `client.list` and `client.revoke` from the enrollment plan.

- [ ] **Step 1: The Devices screen**

Per device, a checkbox per runner, from `client.list` on each reachable runner — derived on every look, never cached. Four states, and **Refused is not Not authorized**:

| State | Shown as |
| --- | --- |
| Authorized | It connected. |
| Not authorized | The runner's daemon says the fingerprint is not in its fence. |
| Refused | sshd rejected the connection, with the reason as sshd gave it — usually generic, and said to be generic. |
| Unknown | The runner did not answer. |

- [ ] **Step 2: Sign-out, as two operations**

```
Sign out of work@example.com?

Signing out doesn't remove this iPhone's access to that account's runners. Its
key stays in ~/.ssh/authorized_keys on work-mini and build-vm until it's
removed.

☐ Also remove this iPhone's access to those runners

Your other accounts on this iPhone aren't affected.

[ Sign Out ]  [ Remove This Account From This Device ]  [ Cancel ]
```

*Sign Out* keeps the key, so signing back in costs nothing. *Remove This Account From This Device* revokes and **deletes the key material** — the operation for a phone you are selling, where a dormant private key is the whole problem. One ambiguous sign-out cannot serve both.

- [ ] **Step 3: Tombstones**

A revocation blocked by an offline runner is written to disk with the old key, the runner's identity and the pending removal, and retried. It survives sign-out — otherwise signing out discards the only thing that could finish the job.

- [ ] **Step 4: Removing a device lists what it enrolled**

A `control` device can enroll keys of its own before anyone revokes it. Read the audit entries from `client.list` and show the descendants, or "removed" implies an eviction that did not happen.

- [ ] **Step 5: Account deletion walks the grants first**

Deleting the WorkOS account revokes nothing, and afterwards there is no sign-in left to try again with. So it happens before, and names what it could not reach.

- [ ] **Step 6: Build all three, walk every removal path, commit**

## Self-Review

**Spec coverage.** Implements the app half of "The ceremony" (Tasks 1–4), "Grants are per runner" (Tasks 2, 5), "A Mac needs two keys" and `~/.ssh/config` (Task 4), "Remote Login" (Task 4), "What signing out does not do" (Task 5), and the four connection states (Task 5).

**Placeholders.** One honest gap: Task 1 Step 2 says iOS has no test target and asks the implementer to say which verification they did rather than pretending to a test that does not exist. Task 3's dependency choice is stated as a decision to record, not a blank.

**Type consistency.** `CeremonyStore.Phase` in Task 1 drives the views in Task 2; `RunnerEntry` comes from the ceremony core and is what `writeRunnerBlock` consumes in Task 4; `client.list`/`client.revoke` in Task 5 are the enrollment plan's methods.
